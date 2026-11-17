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
    extract::{RawQuery, State},
    http::{
        header::{CACHE_CONTROL, LOCATION, REFERRER_POLICY},
        HeaderMap, HeaderValue, StatusCode,
    },
    response::{Html, IntoResponse, Redirect, Response},
    routing::get,
    Json, Router,
};
use tower_cookies::Cookies;

use crate::auth::MaybeSession;
use crate::db::oidc_codes::Grant;
use crate::mfa::MfaDecision;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/oauth/jwks", get(jwks))
        .route("/oauth/authorize", get(authorize))
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
const SCOPES: [&str; 4] = ["openid", "profile", "email", "groups"];

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

impl AuthParams {
    fn parse(query: &str) -> Option<Self> {
        let mut out: Vec<(String, String)> = Vec::new();
        for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
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
        return refuse("a request parameter is repeated");
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
        return fail("access_denied");
    };
    let user = match crate::db::users::fetch(db, username).await {
        Ok(Some(u)) => u,
        // The session outlived its account.
        Ok(None) => return fail("access_denied"),
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

#[cfg(test)]
mod tests {
    use super::*;

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
