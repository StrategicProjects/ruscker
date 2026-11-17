//! OIDC provider — the public metadata endpoints (slice 2 of
//! `docs/adr/0005-oidc-provider.md`).
//!
//! * `GET /.well-known/openid-configuration` — where the endpoints are
//!   and what the provider supports.
//! * `GET /oauth/jwks` — the public keys that verify its tokens.
//! * `GET /oauth/authorize` — the authorization endpoint (slice 3): the
//!   user's browser arrives here from a client, proves who the user is
//!   (existing session or login), passes the client's gates, and leaves
//!   with a one-use code. See [`authorize`].
//! * `POST /oauth/token` — the client's back-channel call (slice 4): it
//!   proves who IT is, redeems the code and receives the ID token. See
//!   [`token`].
//! * `GET|POST /oauth/userinfo` — the same identity claims, fetched
//!   with the access token. See [`userinfo`].
//!
//! The first two are public by definition and carry no secret. All answer **404
//! unless the provider is enabled with a usable issuer**
//! (`server.oidc-provider`), so a deployment that never opted in exposes
//! nothing new.
//!
//! Paths are relative to the portal's mount point, and OIDC requires
//! the discovery document to live at
//! `{issuer}/.well-known/openid-configuration` — so the configured
//! issuer must be the portal's public base URL *including* any base
//! path. That is enforced: an issuer whose path is not the mount point
//! leaves the provider off (`oidc::active_issuer`).

use axum::{
    extract::{DefaultBodyLimit, RawQuery, State},
    http::{
        header::{
            AUTHORIZATION, CACHE_CONTROL, CONTENT_TYPE, LOCATION, PRAGMA, REFERRER_POLICY,
            WWW_AUTHENTICATE,
        },
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use base64::Engine;
use serde_json::{json, Map, Value};
use tower_cookies::Cookies;

use crate::auth::MaybeSession;
use crate::db::oidc_codes::Grant;
use crate::db::users::UserRow;
use crate::mfa::MfaDecision;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/oauth/jwks", get(jwks))
        .route("/oauth/authorize", get(authorize))
        // A token request is a few hundred bytes. The cap is enforced
        // while the body is READ, so an oversized one is rejected before
        // any of it is parsed — this endpoint is reachable without
        // credentials.
        .route("/oauth/token", post(token).layer(DefaultBodyLimit::max(MAX_QUERY_LEN)))
        .route("/oauth/userinfo", get(userinfo).post(userinfo))
}

/// Clients cache these documents; five minutes keeps a key rotation
/// visible quickly without a request per login.
const CACHE: &str = "public, max-age=300";

fn json(body: serde_json::Value) -> Response {
    ([(CACHE_CONTROL, CACHE)], Json(body)).into_response()
}

/// The issuer when the provider can serve, or the refusal to answer
/// with (status + body) when it cannot.
///
/// * off, or issuer/base-path mismatch ⇒ 404 (the endpoints do not
///   exist);
/// * enabled but no master key ⇒ 503 on BOTH documents. Discovery
///   needs no key itself, but advertising a `jwks_uri` that can only
///   ever answer 503 is a provider that half-works.
fn provider(state: &AppState) -> Result<String, (StatusCode, &'static str)> {
    let Some(issuer) = crate::oidc::active_issuer(&state.config, &state.base_path) else {
        return Err((StatusCode::NOT_FOUND, ""));
    };
    if !state.master_key.is_configured() {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "provider not configured"));
    }
    Ok(issuer)
}

async fn discovery(State(state): State<AppState>) -> Response {
    match provider(&state) {
        Ok(issuer) => json(crate::oidc::discovery(&issuer)),
        Err(refusal) => refusal.into_response(),
    }
}

async fn jwks(State(state): State<AppState>) -> Response {
    if let Err(refusal) = provider(&state) {
        return refusal.into_response();
    }
    let Some(db) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    // The common path reads the public keys and decrypts nothing. Only
    // when there is no key at all do we create the first one — so a
    // client that reads the JWKS before anything else never finds it
    // empty. Once a key exists this endpoint cannot be made to write.
    match crate::oidc::published_jwks(db).await {
        Ok(Some(set)) => return json(set),
        Ok(None) => {}
        Err(err) => {
            tracing::error!(error = ?err, "build JWKS failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "jwks error").into_response();
        }
    }
    if let Err(err) = crate::oidc::ensure_signing_key(db, &state.master_key).await {
        tracing::error!(error = ?err, "could not create the OIDC signing key");
        return (StatusCode::SERVICE_UNAVAILABLE, "signing key unavailable").into_response();
    }
    match crate::oidc::published_jwks(db).await {
        Ok(Some(set)) => json(set),
        Ok(None) => (StatusCode::SERVICE_UNAVAILABLE, "signing key unavailable").into_response(),
        Err(err) => {
            tracing::error!(error = ?err, "build JWKS failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "jwks error").into_response()
        }
    }
}

// ── Authorization endpoint ───────────────────────────────────────

/// Scopes this provider understands. `openid` is mandatory; the others
/// select which claims the client will later receive. Anything else a
/// client asks for is silently dropped (OAuth 2.0 §3.3 allows granting
/// less than requested).
const SCOPES: [&str; 6] = ["openid", "profile", "email", "groups", "roles", "setor"];

/// Upper bounds on values we store or echo back. Generous for any real
/// client, small enough that a hostile one cannot use us as storage.
const MAX_STATE_LEN: usize = 1024;
const MAX_NONCE_LEN: usize = 255;
/// The whole query. It is carried, percent-encoded (up to three times
/// its size), in the `next` of the login redirect — past this it would
/// not fit a reverse proxy's request line and the return trip would be
/// lost. Real requests are a few hundred bytes.
const MAX_QUERY_LEN: usize = 4096;

/// The query of an authorization request, parsed once.
///
/// OAuth 2.0 (RFC 6749 §3.1) forbids repeating a parameter. That is not
/// pedantry: with `redirect_uri` given twice, "which one was validated"
/// and "which one is used" can end up being different values. Parsing
/// refuses duplicates up front, so every later `get` is unambiguous.
struct AuthParams(Vec<(String, String)>);

/// No legitimate request comes near this many parameters. The bound
/// also keeps the duplicate check below (a scan per parameter) cheap
/// whatever the caller sends.
const MAX_PARAMS: usize = 32;

impl AuthParams {
    fn parse(query: &str) -> Option<Self> {
        let mut out: Vec<(String, String)> = Vec::new();
        for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
            if out.len() == MAX_PARAMS {
                return None;
            }
            if out.iter().any(|(seen, _)| seen == &*k) {
                return None;
            }
            out.push((k.into_owned(), v.into_owned()));
        }
        Some(Self(out))
    }

    /// The value of `key`, treating an empty value as absent.
    fn get(&self, key: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.as_str())
            .filter(|v| !v.is_empty())
    }
}

/// The `(client_id, redirect_uri)` pair of an authorization request's
/// query, or `None` when either is missing or a parameter is repeated —
/// exactly the cases in which [`authorize`] refuses before looking
/// anything up. Used by the sign-in page to decide whether it may name
/// the system a sign-in is for.
pub(crate) fn client_and_redirect(query: &str) -> Option<(String, String)> {
    if query.len() > MAX_QUERY_LEN {
        return None;
    }
    let params = AuthParams::parse(query)?;
    Some((params.get("client_id")?.to_string(), params.get("redirect_uri")?.to_string()))
}

/// A PKCE S256 challenge is base64url(SHA-256(verifier)) without
/// padding: always exactly 43 characters of the URL-safe alphabet.
fn is_s256_challenge(v: &str) -> bool {
    v.len() == 43 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

/// 400 for a request we cannot even safely redirect: the client or its
/// redirect URI did not check out, so sending the browser "back" would
/// mean sending it wherever the request said. The user sees this page.
fn refuse(reason: &'static str) -> Response {
    (StatusCode::BAD_REQUEST, [(CACHE_CONTROL, "no-store")], reason).into_response()
}

/// Send the browser back to the client's (already validated) redirect
/// URI carrying `pairs`, plus `state` when the client sent one.
///
/// `Cache-Control: no-store` and `Referrer-Policy: no-referrer` because
/// the URL may carry a code. A plain 302 with an absolute `Location`:
/// the base-path middleware only rewrites root-relative locations, so it
/// leaves this one alone.
fn back_to_client(redirect_uri: &str, state: Option<&str>, pairs: &[(&str, &str)]) -> Response {
    let mut query = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        query.append_pair(k, v);
    }
    if let Some(state) = state {
        query.append_pair("state", state);
    }
    // The registered URI may already have a query of its own.
    let sep = if redirect_uri.contains('?') { '&' } else { '?' };
    let target = format!("{redirect_uri}{sep}{}", query.finish());
    let Ok(location) = HeaderValue::from_str(&target) else {
        // Cannot happen for a URI that passed registration (printable
        // ASCII) with form-encoded values, but never panic on a header.
        return refuse("invalid redirect target");
    };
    (
        StatusCode::FOUND,
        [
            (LOCATION, location),
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (REFERRER_POLICY, HeaderValue::from_static("no-referrer")),
        ],
    )
        .into_response()
}

/// The same-site bounce: a minimal page that immediately re-requests
/// the same authorization URL.
///
/// Why it exists: the session cookie is `SameSite=Strict`, so a browser
/// arriving here straight from a client on ANOTHER site does not send
/// it — the user would look signed-out and be asked for the password on
/// every single sign-in (and for MFA again, since that proof is bound to
/// the session). A navigation that starts from one of our own pages,
/// however, is same-site and does carry the cookie. So instead of
/// weakening the cookie, a cross-site arrival without a session gets
/// this page first; the browser follows it and the second request is
/// evaluated with the real session state.
///
/// No loop: the follow-up request is same-origin, and only a
/// cross-site one is ever bounced. A browser that sends no
/// `Sec-Fetch-Site` simply skips this and goes to the login page.
///
/// A `<meta refresh>` rather than script: the admin's CSP forbids
/// inline script, and this needs none.
fn same_site_bounce(base: &str, raw_query: &str) -> Response {
    let url = format!("{base}/oauth/authorize?{raw_query}");
    // The query came from the request: escape it for an HTML attribute.
    let url = url
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    let page = format!(
        "<!doctype html><meta charset=\"utf-8\">\
         <meta http-equiv=\"refresh\" content=\"0;url={url}\">\
         <title>…</title><a href=\"{url}\">Continue</a>"
    );
    (
        [
            (CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (REFERRER_POLICY, HeaderValue::from_static("no-referrer")),
        ],
        Html(page),
    )
        .into_response()
}

/// `GET /oauth/authorize` — the OIDC authorization endpoint.
///
/// The order of the checks is the security property:
///
/// 1. **Provider on?** Otherwise this route does not exist (404).
/// 2. **Client and redirect URI.** Until both are verified against the
///    registry, nothing is redirected anywhere — failures are a 400 page
///    shown to the user. This is what stops the endpoint from being an
///    open redirector or delivering a code to an attacker's URL.
/// 3. **The rest of the request** (response type, scope, PKCE…). From
///    here on, errors go back to the client as `error=…` on its
///    redirect URI, as the protocol specifies.
/// 4. **Who is the user?** No session ⇒ off to the login page, which
///    returns here afterwards (`next`). The login page only accepts
///    this exact path as a destination. A cross-site arrival is first
///    bounced through one of our own pages so an EXISTING session is
///    actually seen (see [`same_site_bounce`]).
/// 5. **May this user use this client?** The client's group gate, read
///    from the database on every request (group changes do not revoke
///    sessions, so a session's cached view could be stale).
/// 6. **MFA**, when the client requires it: the proof must have been
///    given in this very session.
/// 7. Only then is a code issued — single-use, one minute, bound to the
///    client, the redirect URI and the PKCE challenge.
///
/// There is no consent screen: every client is first-party and
/// registered by an Admin (ADR 0005).
async fn authorize(
    State(state): State<AppState>,
    session: MaybeSession,
    cookies: Cookies,
    headers: HeaderMap,
    peer: crate::routes::proxy::MaybePeer,
    RawQuery(raw_query): RawQuery,
) -> Response {
    // 1. Provider.
    let issuer = match provider(&state) {
        Ok(issuer) => issuer,
        Err(refusal) => return refusal.into_response(),
    };
    let Some(db) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let raw_query = raw_query.unwrap_or_default();
    // The raw query travels on in the `next` of our own redirects, where
    // a backslash is refused (open-redirect guard) and length is bounded
    // by the reverse proxy. Refuse both here, visibly, rather than lose
    // the user's way back after they log in.
    if raw_query.len() > MAX_QUERY_LEN || raw_query.contains('\\') {
        return refuse("malformed authorization request");
    }
    let Some(params) = AuthParams::parse(&raw_query) else {
        return refuse("a request parameter is repeated, or there are too many");
    };

    // 2. Client and redirect URI — no redirect before these pass.
    let Some(client_id) = params.get("client_id") else {
        return refuse("client_id is required");
    };
    // One message for every way this step fails — unknown id, disabled
    // client, unregistered URI — so the page tells a prober nothing.
    const NOT_REGISTERED: &str = "unknown client or redirect_uri";
    // Shape first: an id that could never have been registered does not
    // reach the database (Postgres, for one, errors on a NUL byte, which
    // would turn a bad request into a logged 500).
    if !crate::db::oidc_clients::is_valid_client_id(client_id) {
        return refuse(NOT_REGISTERED);
    }
    let client = match crate::db::oidc_clients::fetch(db, client_id).await {
        Ok(Some(c)) if c.enabled => c,
        Ok(_) => return refuse(NOT_REGISTERED),
        Err(err) => {
            tracing::error!(error = ?err, "fetch oidc client failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    // Always required, even for a client with a single registered URI:
    // one rule, and the token request must repeat the same value.
    let Some(redirect_uri) = params.get("redirect_uri") else {
        return refuse("redirect_uri is required");
    };
    if !client.redirect_uri_registered(redirect_uri) {
        return refuse(NOT_REGISTERED);
    }

    // 3. From here, errors are reported to the client.
    let client_state = params.get("state");
    if client_state.is_some_and(|s| s.len() > MAX_STATE_LEN) {
        // Not echoed: it is the oversized value itself.
        return back_to_client(redirect_uri, None, &[("error", "invalid_request"), ("iss", &issuer)]);
    }
    let fail = |error: &str| {
        back_to_client(redirect_uri, client_state, &[("error", error), ("iss", &issuer)])
    };
    if params.get("request").is_some() {
        return fail("request_not_supported");
    }
    if params.get("request_uri").is_some() {
        return fail("request_uri_not_supported");
    }
    // The code is only ever returned in the query, as discovery says.
    // Honouring the request while ignoring a different `response_mode`
    // would hand the code over in a way the client did not ask for.
    if params.get("response_mode").is_some_and(|m| m != "query") {
        return fail("invalid_request");
    }
    if params.get("response_type") != Some("code") {
        return fail("unsupported_response_type");
    }
    let requested: Vec<&str> = params.get("scope").unwrap_or("").split(' ').collect();
    if !requested.contains(&"openid") {
        return fail("invalid_scope");
    }
    let scope = SCOPES
        .iter()
        .filter(|s| requested.contains(s))
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    // PKCE is mandatory and only S256 — `plain` would let whoever sees
    // the authorization request redeem the code.
    let challenge = match (params.get("code_challenge"), params.get("code_challenge_method")) {
        (Some(c), Some("S256")) if is_s256_challenge(c) => c,
        _ => return fail("invalid_request"),
    };
    let nonce = params.get("nonce");
    // Stored and later copied into the ID token: bounded, printable.
    if nonce.is_some_and(|n| n.len() > MAX_NONCE_LEN || !n.bytes().all(|b| b.is_ascii_graphic())) {
        return fail("invalid_request");
    }
    // `prompt=none`: the client wants an answer without any UI. Every
    // step below that would show a page reports an error instead.
    // `none` together with another value is a contradiction (OIDC Core
    // §3.1.2.1) and is refused.
    let prompt: Vec<&str> = params.get("prompt").map(|p| p.split(' ').collect()).unwrap_or_default();
    let silent = prompt.contains(&"none");
    if silent && prompt.len() > 1 {
        return fail("invalid_request");
    }

    // Where the login / password / MFA pages send the browser back to:
    // this very request. Base-less, as those pages expect.
    let here = format!("/oauth/authorize?{raw_query}");
    let detour = |page: &str| {
        let path = format!("{}{page}", state.base_path);
        Redirect::to(&super::with_next_query(&path, &here)).into_response()
    };

    // From here on the request is about a known client, so refusals are
    // worth an audit row: who tried to enter which system, from where.
    let origin = crate::activity::ClientInfo::from_request(&state, &headers, peer.0.as_ref());
    // One row per (user, system, reason) per minute: any page can send a
    // signed-in user here, and a client that retries after a refusal
    // loops — neither should write a row per request.
    let denied = |username: Option<String>, reason: &str| {
        let key = format!(
            "sso.denied|{}|{}|{reason}",
            username.as_deref().unwrap_or("-"),
            client.client_id
        );
        state.activity.record_limited(
            "sso.denied",
            &key,
            1,
            crate::activity::ActivityEvent::sso_denied(username, client.client_id.clone(), reason)
                .with_client(origin.clone()),
        );
    };

    // 4. Who is the user?
    let Some(session) = session.0 else {
        // Arrived from another site: the Strict session cookie was not
        // sent, so "no session" may be false. Ask again from our own
        // origin before concluding anything (also for `prompt=none`).
        let cross_site = headers
            .get("sec-fetch-site")
            .and_then(|v| v.to_str().ok())
            .is_some_and(|site| site == "cross-site");
        if cross_site {
            return same_site_bounce(&state.base_path, &raw_query);
        }
        return if silent { fail("login_required") } else { detour("/admin/login") };
    };
    // A break-glass token session is an operator, not a person: it has
    // no account, so there is no identity to hand to a client.
    let Some(username) = session.actor.as_deref() else {
        denied(None, "no-account");
        return fail("access_denied");
    };
    let user = match crate::db::users::fetch(db, username).await {
        Ok(Some(u)) => u,
        // The session outlived its account.
        Ok(None) => {
            denied(None, "account-missing");
            return fail("access_denied");
        }
        Err(err) => {
            tracing::error!(error = ?err, "fetch user for authorization failed");
            return fail("server_error");
        }
    };
    // An admin-assigned password must be replaced before it is used to
    // sign in anywhere else.
    if user.must_change_password {
        return if silent {
            fail("interaction_required")
        } else {
            detour("/admin/account/password")
        };
    }

    // 5. The client's group gate. No bypass for Admins: a client's
    //    audience is whoever is in its groups, nothing more.
    if !client.allows_groups(&user.groups) {
        tracing::info!(client = %client.client_id, user = %user.username, "SSO refused: not in the client's groups");
        denied(Some(user.username.clone()), "not-in-group");
        return fail("access_denied");
    }

    // 6. MFA, session-bound (validity 0).
    if client.require_mfa {
        let Some(session_id) = cookies
            .get(crate::auth::COOKIE_NAME)
            .map(|c| c.value().to_string())
        else {
            return fail("access_denied");
        };
        let decision =
            crate::mfa::evaluate_with_validity(&state, &user.username, &session_id, &cookies, 0).await;
        let page = match decision {
            MfaDecision::Satisfied => None,
            MfaDecision::EnrollmentRequired => Some("/admin/account/mfa"),
            MfaDecision::ChallengeRequired => Some("/admin/account/mfa/challenge"),
        };
        if let Some(page) = page {
            return if silent { fail("interaction_required") } else { detour(page) };
        }
    }

    // 7. Issue the code.
    let grant = Grant {
        client_id: client.client_id.clone(),
        user_id: user.id.clone(),
        username: user.username.clone(),
        redirect_uri: redirect_uri.to_string(),
        code_challenge: challenge.to_string(),
        nonce: nonce.map(str::to_string),
        scope,
        // The user's side of the sign-in, for the audit row written
        // when the client's server redeems the code.
        client_ip: origin.ip.clone(),
        user_agent: origin.user_agent.clone(),
    };
    match crate::db::oidc_codes::issue(db, &grant).await {
        // `iss` (RFC 9207) lets a client that talks to several providers
        // tell which one this answer came from.
        Ok(code) => back_to_client(redirect_uri, client_state, &[("code", &code), ("iss", &issuer)]),
        Err(err) => {
            tracing::error!(error = ?err, "issue authorization code failed");
            fail("server_error")
        }
    }
}

// ── Token endpoint ───────────────────────────────────────────────

/// Lifetime of the ID token and of the access token. Clients keep their
/// own session after the exchange (ADR 0005), so these only have to
/// outlive the handful of requests a login makes.
const TOKEN_TTL_SECS: i64 = 300;

/// An OAuth error response for the token endpoint (RFC 6749 §5.2):
/// JSON `{error, error_description}`, never cached.
fn token_error(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        [(CACHE_CONTROL, "no-store"), (PRAGMA, "no-cache")],
        Json(json!({ "error": error, "error_description": description })),
    )
        .into_response()
}

/// 401 `invalid_client`. When the client tried HTTP Basic, the response
/// must carry a matching `WWW-Authenticate` challenge (RFC 6749 §5.2).
fn invalid_client(used_basic: bool) -> Response {
    let mut resp = token_error(StatusCode::UNAUTHORIZED, "invalid_client", "client authentication failed");
    if used_basic {
        resp.headers_mut()
            .insert(WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"oidc\""));
    }
    resp
}

/// The value after an `Authorization` scheme name, matched without
/// regard to case as RFC 7235 requires (`Basic`, `basic`, `BASIC`…).
fn auth_scheme_value<'a>(headers: &'a HeaderMap, scheme: &str) -> Option<&'a str> {
    let value = headers.get(AUTHORIZATION)?.to_str().ok()?;
    let (name, rest) = value.split_once(' ')?;
    name.eq_ignore_ascii_case(scheme).then_some(rest.trim())
}

/// Credentials from an `Authorization: Basic` header. Per RFC 6749
/// §2.3.1 both halves are form-urlencoded before being joined with `:`
/// and base64-encoded, so they are decoded the same way here.
fn basic_credentials(headers: &HeaderMap) -> Option<(String, String)> {
    let encoded = auth_scheme_value(headers, "Basic")?;
    let raw = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).ok()?;
    let raw = String::from_utf8(raw).ok()?;
    let (id, secret) = raw.split_once(':')?;
    // `+` means space in this encoding; then ordinary percent-decoding.
    let decode = |part: &str| -> Option<String> {
        urlencoding::decode(&part.replace('+', " ")).ok().map(|c| c.into_owned())
    };
    Some((decode(id)?, decode(secret)?))
}

/// What the claims need beyond the account row, read only when the
/// granted scope asks for it.
#[derive(Default)]
struct ClaimExtras {
    /// The person's full name, when set.
    full_name: Option<String>,
    /// Every `(group, role)` the user holds.
    group_roles: Vec<(String, String)>,
}

/// Load [`ClaimExtras`] for `scope`.
///
/// A failed read is an ERROR, not "nothing extra": a client that syncs a
/// person's profile and permissions at each sign-in would otherwise take
/// `{}` for "this person has no roles any more" (and the username for
/// their name) because of a passing database hiccup. The caller answers
/// `server_error` and the client retries.
async fn claim_extras(
    db: &crate::db::ConfigDb,
    user: &UserRow,
    scope: &str,
) -> anyhow::Result<ClaimExtras> {
    let granted = |s: &str| scope.split(' ').any(|g| g == s);
    let mut extras = ClaimExtras::default();
    if granted("profile") {
        extras.full_name = crate::db::users::full_name(db, &user.username).await?;
    }
    if granted("roles") {
        extras.group_roles = crate::db::group_roles::roles_of_user(db, &user.username).await?;
    }
    Ok(extras)
}

/// The identity claims a client may see, by granted scope. Used for the
/// ID token and for `/oauth/userinfo`, so the two can never disagree.
///
/// * always: `sub` — the account's immutable id (a username can be
///   reused after an account is deleted; the id cannot);
/// * `profile`: `preferred_username`, and `name` — the person's full
///   name when the account has one, the username otherwise;
/// * `email`: `email`, when the account has one;
/// * `setor`: `setor` (department), when the account has one;
/// * `groups`: `groups`;
/// * `roles`: `group_roles` — an object mapping each group to the roles
///   the user holds in it.
///
/// `groups` and `group_roles` carry only the groups that grant access to
/// THIS client when it is restricted, all of the user's groups when it
/// is open: a client learns what it needs to authorize, not the user's
/// memberships in unrelated teams. Roles of a group the user no longer
/// belongs to are never sent. Note that `group_roles` names groups (its
/// keys) even without the `groups` scope — the same set, limited to the
/// groups in which the user holds a role.
fn identity_claims(
    user: &UserRow,
    extras: &ClaimExtras,
    access_groups: &[String],
    scope: &str,
) -> Map<String, Value> {
    let granted = |s: &str| scope.split(' ').any(|g| g == s);
    // Which of the user's groups this client is allowed to learn about.
    let visible = |g: &String| {
        user.groups.contains(g) && (access_groups.is_empty() || access_groups.contains(g))
    };
    let non_blank = |v: &Option<String>| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);

    let mut claims = Map::new();
    claims.insert("sub".into(), json!(user.id));
    if granted("profile") {
        claims.insert("preferred_username".into(), json!(user.username));
        let name = non_blank(&extras.full_name).unwrap_or_else(|| user.username.clone());
        claims.insert("name".into(), json!(name));
    }
    if granted("email") {
        if let Some(email) = non_blank(&user.email) {
            claims.insert("email".into(), json!(email));
        }
    }
    if granted("setor") {
        if let Some(setor) = non_blank(&user.setor) {
            claims.insert("setor".into(), json!(setor));
        }
    }
    if granted("groups") {
        let groups: Vec<&String> = user.groups.iter().filter(|g| visible(g)).collect();
        claims.insert("groups".into(), json!(groups));
    }
    if granted("roles") {
        // { "group": ["role", …] }, groups and roles in a stable order.
        let mut by_group: std::collections::BTreeMap<&str, Vec<&str>> = std::collections::BTreeMap::new();
        for (group, role) in extras.group_roles.iter().filter(|(g, _)| visible(g)) {
            by_group.entry(group.as_str()).or_default().push(role.as_str());
        }
        for roles in by_group.values_mut() {
            roles.sort_unstable();
            roles.dedup();
        }
        claims.insert("group_roles".into(), json!(by_group));
    }
    claims
}

/// `POST /oauth/token` — redeem an authorization code.
///
/// The caller is the client's BACKEND, not a browser. Order matters
/// here too:
///
/// 1. **Authenticate the client** (secret via HTTP Basic or the form
///    body, never both). Nothing else is looked at before this passes,
///    so someone who merely intercepted a code cannot even burn it.
/// 2. **Redeem the code** — atomically, once. From this point the code
///    is spent whatever happens next: a failed attempt is not retried.
/// 3. **Check everything the code was bound to**: the same client, the
///    same redirect URI, and the PKCE verifier matching the challenge
///    stored at authorization time.
/// 4. **Re-read the user**: the account must still exist (same id, not
///    merely the same name) and still pass the client's group gate.
/// 5. Only then are the ID token and access token signed.
///
/// Every failure after step 1 is the same `invalid_grant`, so the
/// response does not reveal which binding failed.
async fn token(State(state): State<AppState>, headers: HeaderMap, body: String) -> Response {
    let issuer = match provider(&state) {
        Ok(issuer) => issuer,
        Err(refusal) => return refusal.into_response(),
    };
    let Some(db) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let bad_request = |d: &str| token_error(StatusCode::BAD_REQUEST, "invalid_request", d);
    let form_encoded = headers
        .get(CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| {
            v.split(';').next().unwrap_or("").trim().eq_ignore_ascii_case("application/x-www-form-urlencoded")
        });
    if !form_encoded {
        return bad_request("the body must be application/x-www-form-urlencoded");
    }
    let Some(params) = AuthParams::parse(&body) else {
        return bad_request("a request parameter is repeated, or there are too many");
    };

    // 1. Client authentication.
    let basic = basic_credentials(&headers);
    let used_basic = headers.contains_key(AUTHORIZATION);
    // An `Authorization` header we cannot read as Basic is a failed
    // authentication — it must not quietly fall through to credentials
    // in the body ("one method, never both").
    if used_basic && basic.is_none() {
        return invalid_client(true);
    }
    let (client_id, client_secret) = match (basic, params.get("client_secret")) {
        (Some(_), Some(_)) => return bad_request("use one client authentication method"),
        (Some((id, secret)), None) => {
            // A client_id repeated in the body must agree with the header.
            if params.get("client_id").is_some_and(|b| b != id) {
                return invalid_client(true);
            }
            (id, secret)
        }
        (None, Some(secret)) => match params.get("client_id") {
            Some(id) => (id.to_string(), secret.to_string()),
            None => return invalid_client(used_basic),
        },
        (None, None) => return invalid_client(used_basic),
    };
    // Shape first, as in `authorize`: an id that could never have been
    // registered does not reach the database (where, on Postgres, a NUL
    // byte would turn a bad request into a logged 500).
    if !crate::db::oidc_clients::is_valid_client_id(&client_id) {
        return invalid_client(used_basic);
    }
    let client = match crate::db::oidc_clients::fetch(db, &client_id).await {
        Ok(Some(c)) if c.enabled && c.verify_secret(&client_secret) => c,
        Ok(_) => return invalid_client(used_basic),
        Err(err) => {
            tracing::error!(error = ?err, "fetch oidc client failed");
            return token_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "try again later");
        }
    };

    if params.get("grant_type") != Some("authorization_code") {
        return token_error(
            StatusCode::BAD_REQUEST,
            "unsupported_grant_type",
            "only authorization_code is supported",
        );
    }
    let (Some(code), Some(redirect_uri), Some(verifier)) =
        (params.get("code"), params.get("redirect_uri"), params.get("code_verifier"))
    else {
        return bad_request("code, redirect_uri and code_verifier are required");
    };

    // 2. Redeem — once.
    let invalid_grant =
        || token_error(StatusCode::BAD_REQUEST, "invalid_grant", "the authorization code is not valid");
    let redeemed = match crate::db::oidc_codes::consume(db, code).await {
        Ok(Some(r)) => r,
        Ok(None) => return invalid_grant(),
        Err(err) => {
            tracing::error!(error = ?err, "redeem authorization code failed");
            return token_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "try again later");
        }
    };
    let grant = &redeemed.grant;

    // 3. Bindings.
    if grant.client_id != client.client_id
        || grant.redirect_uri != redirect_uri
        || !crate::oidc::pkce_matches(verifier, &grant.code_challenge)
    {
        tracing::warn!(client = %client.client_id, "authorization code presented with a wrong binding");
        return invalid_grant();
    }

    // 4. The user, as they are NOW.
    let user = match crate::db::users::fetch(db, &grant.username).await {
        Ok(Some(u)) if u.id == grant.user_id && client.allows_groups(&u.groups) => u,
        Ok(_) => {
            // The user passed the gate a moment ago and no longer does
            // (removed from the group, or the account is gone). That is
            // a refused sign-in, and the audit should show it.
            let key = format!("sso.denied|{}|{}|access-revoked", grant.username, client.client_id);
            state.activity.record_limited(
                "sso.denied",
                &key,
                1,
                crate::activity::ActivityEvent::sso_denied(
                    Some(grant.username.clone()),
                    client.client_id.clone(),
                    "access-revoked",
                )
                .with_client(crate::activity::ClientInfo {
                    ip: grant.client_ip.clone(),
                    user_agent: grant.user_agent.clone(),
                }),
            );
            return invalid_grant();
        }
        Err(err) => {
            tracing::error!(error = ?err, "fetch user for token failed");
            return token_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "try again later");
        }
    };

    // 5. Sign.
    let key = match crate::oidc::ensure_signing_key(db, &state.master_key).await {
        Ok(k) => k,
        Err(err) => {
            tracing::error!(error = ?err, "OIDC signing key unavailable");
            return token_error(StatusCode::SERVICE_UNAVAILABLE, "server_error", "signing key unavailable");
        }
    };
    let now = chrono::Utc::now().timestamp();
    let exp = now + TOKEN_TTL_SECS;

    let extras = match claim_extras(db, &user, &grant.scope).await {
        Ok(extras) => extras,
        Err(err) => {
            tracing::error!(error = ?err, "claims lookup failed at the token endpoint");
            return token_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "try again later");
        }
    };
    let mut id_claims = identity_claims(&user, &extras, &client.access_groups, &grant.scope);
    id_claims.insert("iss".into(), json!(issuer));
    id_claims.insert("aud".into(), json!(client.client_id));
    id_claims.insert("iat".into(), json!(now));
    id_claims.insert("exp".into(), json!(exp));
    // No `auth_time`: the session's creation time is not recorded, and
    // the code's issue time is not when the user authenticated — a
    // wrong value would be worse than none (ADR 0005).
    if let Some(nonce) = grant.nonce.as_deref() {
        id_claims.insert("nonce".into(), json!(nonce));
    }
    // The access token is for OUR userinfo endpoint only: audience is
    // the issuer itself, and its `typ` differs from the ID token's, so
    // neither can stand in for the other.
    let access_claims = json!({
        "iss": issuer,
        "aud": issuer,
        "sub": user.id,
        "username": user.username,
        "client_id": client.client_id,
        "scope": grant.scope,
        "iat": now,
        "exp": exp,
    });
    let (id_token, access_token) = match (
        crate::oidc::sign_jwt(&key, "JWT", &Value::Object(id_claims)),
        crate::oidc::sign_jwt(&key, ACCESS_TOKEN_TYP, &access_claims),
    ) {
        (Ok(i), Ok(a)) => (i, a),
        (Err(err), _) | (_, Err(err)) => {
            tracing::error!(error = ?err, "sign tokens failed");
            return token_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "try again later");
        }
    };

    // One activity row per sign-in to an external system.
    // Address and browser are the USER's (carried by the code), not
    // those of the client system's server making this very request.
    state.activity.record(
        crate::activity::ActivityEvent::sso_login(
            user.username.clone(),
            client.client_id.clone(),
            uuid::Uuid::new_v4().to_string(),
        )
        .with_client(crate::activity::ClientInfo {
            ip: grant.client_ip.clone(),
            user_agent: grant.user_agent.clone(),
        }),
    );

    (
        [(CACHE_CONTROL, "no-store"), (PRAGMA, "no-cache")],
        Json(json!({
            "access_token": access_token,
            "token_type": "Bearer",
            "expires_in": TOKEN_TTL_SECS,
            "id_token": id_token,
            "scope": grant.scope,
        })),
    )
        .into_response()
}

// ── UserInfo endpoint ────────────────────────────────────────────

/// JWT `typ` of the access tokens this provider mints (RFC 9068).
const ACCESS_TOKEN_TYP: &str = "at+jwt";

/// 401 for a missing or unusable access token (RFC 6750 §3).
fn bearer_error() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [
            (WWW_AUTHENTICATE, "Bearer error=\"invalid_token\""),
            (CACHE_CONTROL, "no-store"),
        ],
    )
        .into_response()
}

/// `GET|POST /oauth/userinfo` — the identity claims of the access
/// token's user, for the scopes that were granted.
///
/// The token is one of ours (signature), meant for this endpoint
/// (`typ`, `aud`), from this issuer and not expired. The user and the
/// client are then read from the database again: the answer reflects
/// the account as it is now, and stops when the account, the client or
/// the user's access to it is gone.
async fn userinfo(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let issuer = match provider(&state) {
        Ok(issuer) => issuer,
        Err(refusal) => return refusal.into_response(),
    };
    let Some(db) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let Some(bearer) = auth_scheme_value(&headers, "Bearer") else {
        return bearer_error();
    };
    let jwt = match crate::oidc::verify_jwt(db, bearer).await {
        Ok(Some(jwt)) => jwt,
        Ok(None) => return bearer_error(),
        Err(err) => {
            tracing::error!(error = ?err, "verify access token failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "server error").into_response();
        }
    };
    let c = &jwt.claims;
    let now = chrono::Utc::now().timestamp();
    let acceptable = jwt.typ == ACCESS_TOKEN_TYP
        && c["iss"] == issuer.as_str()
        && c["aud"] == issuer.as_str()
        && c["exp"].as_i64().is_some_and(|exp| exp > now);
    let (true, Some(sub), Some(username), Some(client_id), Some(scope)) = (
        acceptable,
        c["sub"].as_str(),
        c["username"].as_str(),
        c["client_id"].as_str(),
        c["scope"].as_str(),
    ) else {
        return bearer_error();
    };

    let client = match crate::db::oidc_clients::fetch(db, client_id).await {
        Ok(Some(client)) if client.enabled => client,
        Ok(_) => return bearer_error(),
        Err(err) => {
            tracing::error!(error = ?err, "fetch oidc client failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "server error").into_response();
        }
    };
    let user = match crate::db::users::fetch(db, username).await {
        Ok(Some(u)) if u.id == sub && client.allows_groups(&u.groups) => u,
        Ok(_) => return bearer_error(),
        Err(err) => {
            tracing::error!(error = ?err, "fetch user for userinfo failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "server error").into_response();
        }
    };
    let extras = match claim_extras(db, &user, scope).await {
        Ok(extras) => extras,
        Err(err) => {
            tracing::error!(error = ?err, "claims lookup failed at userinfo");
            return (StatusCode::INTERNAL_SERVER_ERROR, "server error").into_response();
        }
    };
    (
        [(CACHE_CONTROL, "no-store")],
        Json(Value::Object(identity_claims(&user, &extras, &client.access_groups, scope))),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The group filter of `groups` / `group_roles`, on its own: what a
    /// restricted client may learn, and that a role row for a group the
    /// user is NOT in (stale, or simply not theirs) is never sent.
    #[test]
    fn claims_show_only_groups_the_client_may_learn_and_the_user_is_in() {
        let user = UserRow {
            id: "u1".into(),
            username: "ana".into(),
            role: crate::auth::Role::Viewer,
            must_change_password: false,
            groups: vec!["fiscal".into(), "outro".into()],
            created_at: chrono::Utc::now(),
            created_by: None,
            setor: Some(" GAPE ".into()),
            email: None,
            celular: None,
        };
        let pair = |g: &str, r: &str| (g.to_string(), r.to_string());
        let extras = ClaimExtras {
            full_name: Some("  ".into()),
            group_roles: vec![
                pair("fiscal", "gestor"),
                pair("fiscal", "gestor"),
                pair("outro", "leitor"),
                pair("saiu", "chefe"),  // a group the user is not in
                pair("gestao", "chefe"), // listed by the client, user not in it
            ],
        };
        let scope = "openid profile setor groups roles";

        // Restricted to `fiscal` and `gestao`: only `fiscal` (user ∩ client).
        let restricted = ["fiscal".to_string(), "gestao".to_string()];
        let c = identity_claims(&user, &extras, &restricted, scope);
        assert_eq!(c["groups"], json!(["fiscal"]));
        assert_eq!(c["group_roles"], json!({"fiscal": ["gestor"]}));
        // A blank full name falls back to the username; setor is trimmed.
        assert_eq!(c["name"], "ana");
        assert_eq!(c["setor"], "GAPE");

        // Open client: every group the user is in — still not `saiu`.
        let c = identity_claims(&user, &extras, &[], scope);
        assert_eq!(c["groups"], json!(["fiscal", "outro"]));
        assert_eq!(c["group_roles"], json!({"fiscal": ["gestor"], "outro": ["leitor"]}));

        // Only `sub` without scopes.
        let c = identity_claims(&user, &extras, &[], "openid");
        assert_eq!(c.keys().collect::<Vec<_>>(), vec!["sub"]);
    }

    #[test]
    fn repeated_parameters_are_refused() {
        assert!(AuthParams::parse("a=1&b=2").is_some());
        assert!(AuthParams::parse("redirect_uri=x&redirect_uri=y").is_none());
        // Percent-encoded spelling of the same name is the same name.
        assert!(AuthParams::parse("client_id=x&client%5Fid=y").is_none());
        let p = AuthParams::parse("state=&scope=openid%20email").unwrap();
        assert_eq!(p.get("state"), None, "empty reads as absent");
        assert_eq!(p.get("scope"), Some("openid email"));
    }

    #[test]
    fn parameter_count_is_bounded() {
        let ok: String = (0..MAX_PARAMS).map(|i| format!("k{i}=v&")).collect();
        assert!(AuthParams::parse(&ok).is_some());
        let too_many: String = (0..=MAX_PARAMS).map(|i| format!("k{i}=v&")).collect();
        assert!(AuthParams::parse(&too_many).is_none());
    }

    #[test]
    fn basic_credentials_are_form_decoded_and_scheme_is_case_insensitive() {
        use base64::engine::general_purpose::STANDARD;
        let with = |value: String| {
            let mut h = HeaderMap::new();
            h.insert(AUTHORIZATION, value.parse().unwrap());
            h
        };
        let enc = |raw: &str| STANDARD.encode(raw);
        for scheme in ["Basic", "basic", "BASIC"] {
            let h = with(format!("{scheme} {}", enc("radar:s3cret")));
            assert_eq!(basic_credentials(&h), Some(("radar".into(), "s3cret".into())));
        }
        // RFC 6749 §2.3.1: both halves are form-urlencoded.
        let h = with(format!("Basic {}", enc("my%2Eclient:a%3Ab+c")));
        assert_eq!(basic_credentials(&h), Some(("my.client".into(), "a:b c".into())));
        // Only the FIRST colon separates id from secret.
        let h = with(format!("Basic {}", enc("radar:a:b")));
        assert_eq!(basic_credentials(&h), Some(("radar".into(), "a:b".into())));
        // Unusable headers.
        for bad in ["Basic !!!notbase64".to_string(), format!("Basic {}", enc("nocolon")), "Bearer abc".into()] {
            assert_eq!(basic_credentials(&with(bad.clone())), None, "{bad}");
        }
    }

    #[test]
    fn s256_challenge_shape() {
        assert!(is_s256_challenge("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"));
        assert!(!is_s256_challenge("short"));
        assert!(!is_s256_challenge("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-c="));
        assert!(!is_s256_challenge("E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cMx"));
    }

    #[test]
    fn back_to_client_appends_to_an_existing_query_and_encodes_values() {
        let resp = back_to_client(
            "https://c.example/cb?tenant=1",
            Some("a b&c"),
            &[("error", "access_denied")],
        );
        assert_eq!(resp.status(), StatusCode::FOUND);
        let loc = resp.headers()[LOCATION].to_str().unwrap();
        assert_eq!(loc, "https://c.example/cb?tenant=1&error=access_denied&state=a+b%26c");
        assert_eq!(resp.headers()[CACHE_CONTROL], "no-store");
        assert_eq!(resp.headers()[REFERRER_POLICY], "no-referrer");
    }
}
