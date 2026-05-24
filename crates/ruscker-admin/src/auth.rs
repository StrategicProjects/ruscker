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

/// Global sliding-window rate limiter for login attempts.
///
/// Brute-forcing the admin token is the threat. This bounds how
/// many failed attempts the server will entertain in a window,
/// regardless of source — deliberately **global** (not per-IP)
/// because Ruscker runs behind a reverse proxy where the peer
/// IP is always the proxy, and a per-IP key would trust a
/// spoofable `X-Forwarded-For`. A global cap can't be evaded by
/// rotating source addresses.
///
/// Trade-off: a flood of bad attempts can lock out the
/// legitimate operator for up to `window`. Acceptable for a
/// single-operator install and documented in SECURITY.md; the
/// window is short (60s) so lockout self-heals quickly.
///
/// Successful logins clear the window so an operator who
/// fat-fingered a few times isn't blocked once they get it right.
#[derive(Debug)]
pub struct LoginRateLimiter {
    failures: std::sync::Mutex<std::collections::VecDeque<std::time::Instant>>,
    max: usize,
    window: std::time::Duration,
}

impl LoginRateLimiter {
    pub fn new(max: usize, window: std::time::Duration) -> Self {
        Self {
            failures: std::sync::Mutex::new(std::collections::VecDeque::new()),
            max,
            window,
        }
    }

    /// Default policy: 10 failed attempts per 60s. Generous for
    /// human retries, far below anything that threatens a
    /// high-entropy token.
    pub fn default_policy() -> Self {
        Self::new(10, std::time::Duration::from_secs(60))
    }

    /// Returns `true` if a login attempt is allowed right now
    /// (i.e. the failure window isn't saturated). Prunes expired
    /// failures as a side effect.
    pub fn allow(&self) -> bool {
        let now = std::time::Instant::now();
        let mut q = self.failures.lock().unwrap();
        while q.front().is_some_and(|t| now.duration_since(*t) > self.window) {
            q.pop_front();
        }
        q.len() < self.max
    }

    pub fn record_failure(&self) {
        let mut q = self.failures.lock().unwrap();
        q.push_back(std::time::Instant::now());
    }

    pub fn record_success(&self) {
        self.failures.lock().unwrap().clear();
    }
}

impl Default for LoginRateLimiter {
    fn default() -> Self {
        Self::default_policy()
    }
}

/// Whether the original client request reached us over HTTPS.
/// Ruscker terminates TLS at a reverse proxy, which signals the
/// original scheme via `X-Forwarded-Proto`. Used to decide
/// whether to set the `Secure` flag on cookies — we can't just
/// always set it because the dev server runs plain HTTP and the
/// browser would then drop the cookie.
pub fn request_is_https(headers: &axum::http::HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        // XFP can be a comma list ("https, http") when chained;
        // the leftmost is the original client scheme.
        .map(|s| {
            s.split(',')
                .next()
                .map(|p| p.trim().eq_ignore_ascii_case("https"))
                .unwrap_or(false)
        })
        .unwrap_or(false)
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use std::time::Duration;

    #[test]
    fn ct_eq_basics() {
        assert!(ct_eq(b"abc", b"abc"));
        assert!(!ct_eq(b"abc", b"abd"));
        assert!(!ct_eq(b"abc", b"ab")); // length mismatch
    }

    #[test]
    fn rate_limiter_allows_until_saturated() {
        let rl = LoginRateLimiter::new(3, Duration::from_secs(60));
        assert!(rl.allow());
        rl.record_failure();
        rl.record_failure();
        assert!(rl.allow(), "2 < 3 still allowed");
        rl.record_failure();
        assert!(!rl.allow(), "3 failures saturate the window");
    }

    #[test]
    fn rate_limiter_success_clears_window() {
        let rl = LoginRateLimiter::new(2, Duration::from_secs(60));
        rl.record_failure();
        rl.record_failure();
        assert!(!rl.allow());
        rl.record_success();
        assert!(rl.allow(), "success clears the failure window");
    }

    #[test]
    fn rate_limiter_prunes_expired_failures() {
        // Zero-length window → every failure is immediately
        // expired, so allow() always trims back to empty.
        let rl = LoginRateLimiter::new(1, Duration::from_secs(0));
        rl.record_failure();
        rl.record_failure();
        std::thread::sleep(Duration::from_millis(2));
        assert!(rl.allow(), "expired failures pruned");
    }

    #[test]
    fn request_is_https_reads_x_forwarded_proto() {
        let mut h = HeaderMap::new();
        assert!(!request_is_https(&h), "no header → not https");
        h.insert("x-forwarded-proto", "http".parse().unwrap());
        assert!(!request_is_https(&h));
        h.insert("x-forwarded-proto", "https".parse().unwrap());
        assert!(request_is_https(&h));
    }

    #[test]
    fn request_is_https_handles_chained_proto_list() {
        let mut h = HeaderMap::new();
        // Chained proxies: leftmost is the original client scheme.
        h.insert("x-forwarded-proto", "https, http".parse().unwrap());
        assert!(request_is_https(&h));
        h.insert("x-forwarded-proto", "HTTPS".parse().unwrap());
        assert!(request_is_https(&h), "case-insensitive");
    }
}
