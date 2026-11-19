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
        .route("/oauth/button.svg", get(button))
        .route("/oauth/authorize", get(authorize))
        // A token request is a few hundred bytes. The cap is enforced
        // while the body is READ, so an oversized one is rejected before
        // any of it is parsed — this endpoint is reachable without
        // credentials.
        .route("/oauth/token", post(token).layer(DefaultBodyLimit::max(MAX_QUERY_LEN)))
        .route("/oauth/userinfo", get(userinfo).post(userinfo))
        .route("/oauth/logout", get(end_session).post(end_session_confirm))
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

/// `GET /oauth/button.svg` — the "Sign in with <portal>" button (#1083).
///
/// Public, like discovery: it shows only the portal's title, which the
/// landing page already shows. `?theme=dark` for dark surfaces, `?lang=`
/// (`pt`, `en`, `es`, `fr`) for the wording; anything else falls back to
/// light and the portal's default language. Client systems embed it as
/// an `<img>` inside their own link to their own sign-in route.
async fn button(State(state): State<AppState>, RawQuery(query): RawQuery) -> Response {
    if let Err(refusal) = provider(&state) {
        return refusal.into_response();
    }
    let mut dark = false;
    let mut loc = crate::i18n::DEFAULT;
    for (k, v) in url::form_urlencoded::parse(query.as_deref().unwrap_or("").as_bytes()) {
        match &*k {
            "theme" => dark = v == "dark",
            "lang" => loc = crate::i18n::Locale::parse(&v).unwrap_or(loc),
            _ => {}
        }
    }
    let portal = super::landing::portal_brand(&state, loc).await.title;
    let mut args = fluent_bundle::FluentArgs::new();
    args.set("portal", portal);
    let label = state.locales.t(loc, "sso-button-label", Some(&args));
    // Fluent wraps placeables in direction-isolation marks; they are
    // invisible but would count as glyphs for the fitted width.
    let label: String = label.chars().filter(|c| !matches!(c, '\u{2066}'..='\u{2069}')).collect();
    (
        [
            (axum::http::header::CONTENT_TYPE, "image/svg+xml; charset=utf-8"),
            (CACHE_CONTROL, CACHE),
            // An SVG is a document: this one needs nothing at all.
            (axum::http::header::CONTENT_SECURITY_POLICY, "default-src 'none'; style-src 'unsafe-inline'"),
        ],
        crate::oidc::sign_in_button_svg(&label, dark),
    )
        .into_response()
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
    bounce_to(base, "/oauth/authorize", raw_query)
}

/// [`same_site_bounce`] for any of our endpoints (`path` is a constant).
fn bounce_to(base: &str, path: &str, raw_query: &str) -> Response {
    let url = format!("{base}{path}?{raw_query}");
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
    // Granted = requested ∩ supported ∩ what this client may have. A
    // scope the client is not allowed is dropped silently, as an unknown
    // one is: the token response states what was granted.
    let scope = SCOPES
        .iter()
        .filter(|s| requested.contains(s) && client.allows_scope(s))
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

    // 7. Issue the code, bound to the account's authentication generation
    // (read now; the session it came from was confirmed against it).
    let auth_epoch = match crate::db::users::role_and_epoch(db, &user.username).await {
        Ok(Some((_, epoch))) => epoch,
        Ok(None) => return fail("access_denied"),
        Err(err) => {
            tracing::error!(error = ?err, "read auth epoch for authorization failed");
            return fail("server_error");
        }
    };
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
        // Step 6 only lets a code through for an MFA-requiring client
        // after the proof; for any other client nothing was proved.
        mfa_proved: client.require_mfa,
        auth_epoch,
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

/// `scope` (space-separated) reduced to what `client` may have today.
fn still_allowed(client: &crate::db::oidc_clients::OidcClient, scope: &str) -> String {
    scope
        .split(' ')
        .filter(|s| !s.is_empty() && client.allows_scope(s))
        .collect::<Vec<_>>()
        .join(" ")
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
    // 4b. The policies the authorization checked, as they are NOW
    // (security review 2026-11-19): an administrator who resets the
    // password with a mandatory change, or who starts requiring MFA for
    // this client, within the code's minute must not see a code issued
    // under the old rules still turn into tokens.
    // …and the account's authentication generation: any password or role
    // change since the code was issued refuses it.
    let current_epoch = match crate::db::users::role_and_epoch(db, &user.username).await {
        Ok(Some((_, epoch))) => epoch,
        Ok(None) => return invalid_grant(),
        Err(err) => {
            tracing::error!(error = ?err, "read auth epoch for token failed");
            return token_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "try again later");
        }
    };
    if current_epoch != grant.auth_epoch
        || user.must_change_password
        || (client.require_mfa && !grant.mfa_proved)
    {
        tracing::warn!(client = %client.client_id, user = %user.username,
            "authorization code refused: password change pending or MFA now required");
        return invalid_grant();
    }

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

    // Re-checked against the client as it is NOW: a scope an Admin
    // withdrew after this code was issued is not granted.
    let scope = still_allowed(&client, &grant.scope);
    let extras = match claim_extras(db, &user, &scope).await {
        Ok(extras) => extras,
        Err(err) => {
            tracing::error!(error = ?err, "claims lookup failed at the token endpoint");
            return token_error(StatusCode::INTERNAL_SERVER_ERROR, "server_error", "try again later");
        }
    };
    let mut id_claims = identity_claims(&user, &extras, &client.access_groups, &scope);
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
        // The person is named by `sub` alone. The client can read this
        // token (it is signed, not encrypted), so it carries nothing a
        // withheld scope would have hidden — no username here.
        "sub": user.id,
        "client_id": client.client_id,
        "scope": scope,
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
            "scope": scope,
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
    let (true, Some(sub), Some(client_id), Some(scope)) = (
        acceptable,
        c["sub"].as_str(),
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
    let user = match crate::db::users::fetch_by_id(db, sub).await {
        Ok(Some(u)) if client.allows_groups(&u.groups) => u,
        Ok(_) => return bearer_error(),
        Err(err) => {
            tracing::error!(error = ?err, "fetch user for userinfo failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "server error").into_response();
        }
    };
    let scope = &still_allowed(&client, scope);
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

// ───────────────────────── RP-Initiated Logout ─────────────────────────

/// Parameters of `/oauth/logout` (OpenID Connect RP-Initiated Logout 1.0),
/// on the GET from the client and on the confirmation POST.
#[derive(Debug, Default, serde::Deserialize)]
pub struct EndSessionParams {
    id_token_hint: Option<String>,
    client_id: Option<String>,
    post_logout_redirect_uri: Option<String>,
    state: Option<String>,
    /// Set by our own same-site bounce, so it happens at most once.
    bounced: Option<String>,
}

/// What a sign-out request resolved to — only registry-backed values.
struct EndSession {
    /// The client, when the request named one (by `client_id` or by the
    /// audience of a valid `id_token_hint`).
    client: Option<crate::db::oidc_clients::OidcClient>,
    /// Where to send the browser afterwards: a `post_logout_redirect_uri`
    /// registered for that client, exactly. Never anything else.
    return_to: Option<String>,
    state: Option<String>,
}

/// Validate a sign-out request. `Err` is a reason shown to the person
/// (400): like the authorization endpoint before the client is verified,
/// a bad request is never redirected anywhere.
///
/// * `id_token_hint`, when given, must be an ID token WE signed for this
///   issuer. It may be expired — the specification allows it, and a
///   system signing someone out hours later is the normal case. Its
///   audience names the client.
/// * `client_id`, when given with a hint, must agree with it.
/// * `post_logout_redirect_uri` is honoured only for an identified,
///   enabled client that registered that exact address.
async fn resolve_end_session(
    state: &AppState,
    issuer: &str,
    p: &EndSessionParams,
) -> Result<EndSession, &'static str> {
    let db = state.db.as_ref().ok_or("provider not configured")?;
    let mut client_id = p.client_id.clone().filter(|c| !c.is_empty());
    if let Some(hint) = p.id_token_hint.as_deref().filter(|h| !h.is_empty()) {
        let verified = crate::oidc::verify_jwt(db, hint)
            .await
            .map_err(|_| "server error")?
            .filter(|t| t.typ == "JWT" && t.claims["iss"] == issuer)
            .ok_or("invalid id_token_hint")?;
        let aud = verified.claims["aud"].as_str().ok_or("invalid id_token_hint")?.to_string();
        if client_id.as_deref().is_some_and(|c| c != aud) {
            return Err("client_id does not match id_token_hint");
        }
        client_id = Some(aud);
    }
    let client = match client_id {
        None => None,
        Some(id) => {
            if !crate::db::oidc_clients::is_valid_client_id(&id) {
                return Err("unknown client");
            }
            let c = crate::db::oidc_clients::fetch(db, &id)
                .await
                .map_err(|_| "server error")?
                .filter(|c| c.enabled)
                .ok_or("unknown client")?;
            Some(c)
        }
    };
    let return_to = match p.post_logout_redirect_uri.as_deref().filter(|u| !u.is_empty()) {
        None => None,
        Some(uri) => {
            let ok = client.as_ref().is_some_and(|c| c.post_logout_uri_registered(uri));
            if !ok {
                return Err("post_logout_redirect_uri is not registered for this client");
            }
            Some(uri.to_string())
        }
    };
    if p.state.as_deref().is_some_and(|s| s.len() > 512) {
        return Err("state is too long");
    }
    Ok(EndSession {
        client,
        return_to,
        state: p.state.clone().filter(|s| !s.is_empty()),
    })
}

/// The sign-out page: "also sign out of the portal?" or "you are signed
/// out". Standalone, like the sign-in page, and with the portal's brand.
#[derive(askama::Template)]
#[template(path = "admin/sso_logout.html")]
struct EndSessionPage<'a> {
    locale: crate::i18n::Locale,
    theme: crate::theme::Theme,
    locales: &'a crate::i18n::Locales,
    locales_all: &'static [crate::i18n::Locale],
    base: std::sync::Arc<str>,
    brand_title: String,
    brand_logo: Option<String>,
    theme_style: String,
    /// The client's registered name, when known.
    client_name: Option<String>,
    /// `true`: ask (a portal session exists). `false`: signed out.
    confirm: bool,
    /// Carried by the confirmation form (already validated).
    client_id: String,
    return_to: String,
    state_param: String,
    /// "Stay signed in" / "back to the system": the return address with
    /// its `state`, else the client's home page.
    back_href: Option<String>,
}

impl EndSessionPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
    fn back_label(&self) -> String {
        let mut args = fluent_bundle::FluentArgs::new();
        args.set("name", self.client_name.clone().unwrap_or_default());
        self.locales.t(self.locale, "admin-pw-back-to", Some(&args))
    }
}

/// The address to go back to: the validated return URI with `state`,
/// else the client's home page.
fn back_href(es: &EndSession) -> Option<String> {
    match &es.return_to {
        Some(uri) => {
            let mut q = url::form_urlencoded::Serializer::new(String::new());
            if let Some(st) = &es.state {
                q.append_pair("state", st);
            }
            let q = q.finish();
            Some(if q.is_empty() {
                uri.clone()
            } else {
                format!("{uri}{}{q}", if uri.contains('?') { '&' } else { '?' })
            })
        }
        None => es.client.as_ref().and_then(|c| c.client_uri.clone()),
    }
}

async fn render_end_session(
    state: &AppState,
    loc: crate::i18n::Locale,
    theme: crate::theme::Theme,
    es: &EndSession,
    confirm: bool,
) -> Response {
    let brand = super::landing::portal_brand(state, loc).await;
    let page = EndSessionPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &crate::i18n::Locale::ALL,
        base: state.base_path.clone(),
        brand_title: brand.title,
        brand_logo: brand.logo_src,
        theme_style: brand.theme_style,
        client_name: es.client.as_ref().map(|c| c.name.clone()),
        confirm,
        client_id: es.client.as_ref().map(|c| c.client_id.clone()).unwrap_or_default(),
        return_to: es.return_to.clone().unwrap_or_default(),
        state_param: es.state.clone().unwrap_or_default(),
        back_href: back_href(es),
    };
    let mut resp = super::admin::render(&page);
    resp.headers_mut().insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp.headers_mut().insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    // The portal's policy says `form-action 'self'`, and browsers apply it
    // to the redirect that FOLLOWS a form post too: the confirmation's
    // 302 to the system would be blocked. This one page may also post
    // towards the origin of the (registered, validated) return address.
    if confirm {
        if let Some(origin) = es.return_to.as_deref().and_then(form_action_origin) {
            let csp = crate::content_security_policy("")
                .replace("form-action 'self'", &format!("form-action 'self' {origin}"));
            if let Ok(v) = HeaderValue::from_str(&csp) {
                resp.headers_mut().insert(axum::http::header::CONTENT_SECURITY_POLICY, v);
            }
        }
    }
    resp
}

/// `scheme://host[:port]` of a registered return address, for the CSP.
/// `None` for anything that would not be a plain origin.
fn form_action_origin(uri: &str) -> Option<String> {
    let url = url::Url::parse(uri).ok()?;
    if !matches!(url.scheme(), "https" | "http") {
        return None;
    }
    let origin = url.origin().ascii_serialization();
    (origin != "null").then_some(origin)
}

/// `GET /oauth/logout` — a client system asks to sign the person out of
/// the portal too ("sign out everywhere").
///
/// Never signs anyone out by itself: a bare link must not end a session
/// (logout CSRF), so a person who is signed in is ASKED, and the answer
/// is a same-origin POST. Someone already signed out is sent straight
/// back to the system (or told they are signed out).
///
/// The session cookie is `SameSite=Strict`, so a browser arriving from
/// the system's site does not send it: like the authorization endpoint,
/// a cross-site arrival is first bounced through our own origin so an
/// existing session is seen.
async fn end_session(
    State(state): State<AppState>,
    session: MaybeSession,
    headers: HeaderMap,
    loc: crate::i18n::Locale,
    theme: crate::theme::Theme,
    RawQuery(raw_query): RawQuery,
    axum::extract::Query(params): axum::extract::Query<EndSessionParams>,
) -> Response {
    let issuer = match provider(&state) {
        Ok(i) => i,
        Err(refusal) => return refusal.into_response(),
    };
    let es = match resolve_end_session(&state, &issuer, &params).await {
        Ok(es) => es,
        Err(reason) => return refuse(reason),
    };
    if session.0.is_none() {
        // Bounce once when the cookie may have been withheld: a cross-site
        // arrival, or a browser that sends no Fetch Metadata at all (it
        // still withholds a Strict cookie). Unlike the authorization
        // endpoint, "no session" here sends the person away believing
        // they signed out, so err on the side of looking again. The
        // `bounced` marker stops a second round.
        let site = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok());
        let maybe_withheld = matches!(site, None | Some("cross-site"));
        if maybe_withheld && params.bounced.is_none() {
            let q = raw_query.as_deref().unwrap_or("");
            let q = if q.is_empty() { "bounced=1".to_string() } else { format!("{q}&bounced=1") };
            return bounce_to(&state.base_path, "/oauth/logout", &q);
        }
        // Nothing to end here.
        return match &es.return_to {
            Some(uri) => back_to_client(uri, es.state.as_deref(), &[]),
            None => render_end_session(&state, loc, theme, &es, false).await,
        };
    }
    render_end_session(&state, loc, theme, &es, true).await
}

/// `POST /oauth/logout` — the person confirmed. The request is checked
/// again from scratch (the form is just another client input), the
/// portal session of this browser is ended exactly like the admin's own
/// "Sign out", and the browser goes back to the system.
///
/// Same-origin only: the global CSRF guard refuses a cross-site POST.
async fn end_session_confirm(
    State(state): State<AppState>,
    cookies: Cookies,
    loc: crate::i18n::Locale,
    theme: crate::theme::Theme,
    axum::extract::Form(params): axum::extract::Form<EndSessionParams>,
) -> Response {
    let issuer = match provider(&state) {
        Ok(i) => i,
        Err(refusal) => return refusal.into_response(),
    };
    // The form carries no hint: the client id it holds was validated on
    // the GET and is validated again here.
    let params = EndSessionParams { id_token_hint: None, ..params };
    let es = match resolve_end_session(&state, &issuer, &params).await {
        Ok(es) => es,
        Err(reason) => return refuse(reason),
    };
    super::admin::end_browser_session(&state, &cookies).await;
    match &es.return_to {
        Some(uri) => back_to_client(uri, es.state.as_deref(), &[]),
        None => render_end_session(&state, loc, theme, &es, false).await,
    }
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
