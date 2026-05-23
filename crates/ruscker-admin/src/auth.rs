//! Admin authentication — MVP env-var token model.
//!
//! Threat model for Phase 2: the operator runs Ruscker on a
//! private network (or behind a reverse proxy with mTLS).
//! `RUSCKER_ADMIN_TOKEN` is a long random secret the operator
//! enters once on `/admin/login`; the server matches it in
//! constant time and issues an HttpOnly + Strict-SameSite cookie.
//!
//! Real auth (OIDC, SAML, role-based ACL) lands in Phase 8. Until
//! then the model is "one token = full admin"; suitable for a
//! single-operator install, not for shared teams.
//!
//! Cookie storage choice: the cookie value IS the token literal.
//! Server-side sessions are a Phase 2.5 refinement once we have a
//! session store; today the cookie is bound by `HttpOnly`,
//! `Secure` (when TLS terminated), and `SameSite=Strict`.

use anyhow::Result;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use std::convert::Infallible;
use std::sync::Arc;
use tower_cookies::Cookies;

/// Cookie name. Distinct prefix `ruscker_admin_` keeps it from
/// colliding with the locale/theme cookies and makes it easier to
/// audit in DevTools.
pub const COOKIE_NAME: &str = "ruscker_admin_session";

/// Held in `AppState`. `None` means admin routes are disabled —
/// they will 503 with a hint to set `RUSCKER_ADMIN_TOKEN`. The
/// token is wrapped in `Arc<str>` so cloning [`AppState`] (per
/// request) doesn't copy the string.
#[derive(Clone, Debug, Default)]
pub struct AdminAuth {
    pub token: Option<Arc<str>>,
}

impl AdminAuth {
    pub fn from_env() -> Self {
        let token = std::env::var("RUSCKER_ADMIN_TOKEN").ok().filter(|s| !s.is_empty());
        Self {
            token: token.map(Arc::from),
        }
    }

    pub fn with_token(token: impl Into<String>) -> Self {
        Self {
            token: Some(Arc::from(token.into())),
        }
    }

    pub fn is_configured(&self) -> bool {
        self.token.is_some()
    }

    /// Constant-time compare. Mismatched lengths return false
    /// without iterating — the length leak is intentional and
    /// matches every well-known constant-time API. The XOR-fold
    /// over the body is loop-bounded by `a.len()`, so the time
    /// depends only on the (public) length, not on the bytes.
    pub fn matches(&self, candidate: &str) -> bool {
        let Some(expected) = self.token.as_deref() else {
            return false;
        };
        ct_eq(expected.as_bytes(), candidate.as_bytes())
    }
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// Marker extracted by admin routes to assert that the request is
/// authenticated. Absence of a valid session redirects to the
/// login form (303 See Other so re-POST isn't suggested by
/// browsers).
///
/// When the server has NO admin token configured at all, the
/// extractor returns 503 — admin routes simply cannot work
/// without a token to compare against.
pub struct AdminSession;

impl FromRequestParts<crate::AppState> for AdminSession {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &crate::AppState,
    ) -> Result<Self, Self::Rejection> {
        if !state.admin_auth.is_configured() {
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "RUSCKER_ADMIN_TOKEN is not set — admin disabled",
            )
                .into_response());
        }

        // Cookies extractor is infallible (always Some) when the
        // CookieManagerLayer is in the stack — which it is, see
        // router().
        let cookies = match Cookies::from_request_parts(parts, state).await {
            Ok(c) => c,
            Err(_) => return Err(Redirect::to("/admin/login").into_response()),
        };
        let candidate = cookies.get(COOKIE_NAME).map(|c| c.value().to_string());
        match candidate {
            Some(c) if state.admin_auth.matches(&c) => Ok(AdminSession),
            _ => Err(Redirect::to("/admin/login").into_response()),
        }
    }
}

/// Convenience extractor for routes that want to know whether the
/// caller is authenticated without rejecting — useful for the
/// `_layout.html` so the navbar can show/hide a "logout" link.
pub struct MaybeAdminSession(pub bool);

impl FromRequestParts<crate::AppState> for MaybeAdminSession {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &crate::AppState,
    ) -> Result<Self, Self::Rejection> {
        match AdminSession::from_request_parts(parts, state).await {
            Ok(_) => Ok(MaybeAdminSession(true)),
            Err(_) => Ok(MaybeAdminSession(false)),
        }
    }
}
