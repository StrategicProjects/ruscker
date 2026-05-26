//! Admin authentication — env-var token model with three roles.
//!
//! Threat model: the operator runs Ruscker on a private network
//! (or behind a reverse proxy with mTLS). Up to three long random
//! secrets — `RUSCKER_ADMIN_TOKEN` (required), `RUSCKER_EDITOR_TOKEN`
//! and `RUSCKER_VIEWER_TOKEN` (both optional) — are entered once on
//! `/admin/login`. The server constant-time-compares the submitted
//! token against each and issues an HttpOnly + Strict-SameSite cookie
//! holding an opaque session id; the session carries the matched
//! [`Role`].
//!
//! Roles (see [`Role`]): **Viewer** (dashboard only), **Editor**
//! (apps + media + dashboard), **Admin** (everything). With only
//! `RUSCKER_ADMIN_TOKEN` set the install behaves exactly as before
//! — one token, full admin — so this is backward-compatible.
//!
//! Full multi-user auth (OIDC, SAML, LDAP, per-app ACLs) lands in
//! Phase 8; this is a lightweight role layer on the token/session
//! model.
//!
//! Cookie: the value is an opaque server-side session id (never the
//! token), bound by `HttpOnly`, `Secure` (when TLS terminated), and
//! `SameSite=Strict`. Logout and server restart both revoke it.

use anyhow::Result;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use dashmap::DashMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tower_cookies::Cookies;

/// Cookie name. Distinct prefix `ruscker_admin_` keeps it from
/// colliding with the locale/theme cookies and makes it easier to
/// audit in DevTools.
pub const COOKIE_NAME: &str = "ruscker_admin_session";

/// Admin access level. Ordered least → most privileged, so a
/// `role >= Role::Editor` check reads naturally.
///
/// The permission matrix is centralised in
/// [`Role::can_access_section`] — route guards and the nav both
/// dispatch on it, so they can't drift. Section names are the same
/// `nav_section` strings the templates already use.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    /// Read-only: the monitoring dashboard, nothing else.
    Viewer,
    /// Manages apps and media (create/edit/delete + uploads) and has
    /// the full dashboard, including the stop/restart actions.
    Editor,
    /// Everything — credentials, landing editor, custom blocks, audit
    /// log. The historical single-token behaviour.
    Admin,
}

impl Role {
    /// Whether this role may reach the given admin nav section.
    /// `section` matches the `nav_section` strings used by the
    /// templates (`dashboard`, `specs`, `images`, `credentials`,
    /// `landing`, `blocks`, `audit`).
    pub fn can_access_section(&self, section: &str) -> bool {
        match section {
            // Anyone logged in can view the dashboard.
            "dashboard" => true,
            // Editors (and admins) manage apps and media.
            "specs" | "images" => *self >= Role::Editor,
            // Everything else is admin-only.
            _ => *self == Role::Admin,
        }
    }

    /// Whether this role may perform mutating/operational actions —
    /// the dashboard's stop/restart, app CRUD, media uploads. Editor
    /// and Admin can; Viewer is strictly read-only. Used to gate the
    /// dashboard action buttons (the view itself is open to Viewers).
    pub fn can_manage(&self) -> bool {
        *self >= Role::Editor
    }

    /// The page a freshly-logged-in session of this role lands on.
    /// The dashboard is visible to every role, so it's home for all.
    pub fn home(&self) -> &'static str {
        "/admin/dashboard"
    }

    /// Fluent message key for the human-readable role label.
    pub fn label_key(&self) -> &'static str {
        match self {
            Role::Viewer => "role-viewer",
            Role::Editor => "role-editor",
            Role::Admin => "role-admin",
        }
    }
}

/// Held in `AppState`. One token per [`Role`]; `admin` is required
/// (when it's `None`, admin routes 503 with a hint to set
/// `RUSCKER_ADMIN_TOKEN`), `editor` and `viewer` are optional. Each
/// token is wrapped in `Arc<str>` so cloning [`AppState`] (per
/// request) doesn't copy the string.
#[derive(Clone, Debug, Default)]
pub struct AdminAuth {
    pub admin: Option<Arc<str>>,
    pub editor: Option<Arc<str>>,
    pub viewer: Option<Arc<str>>,
}

impl AdminAuth {
    pub fn from_env() -> Self {
        let read = |var: &str| {
            std::env::var(var)
                .ok()
                .filter(|s| !s.is_empty())
                .map(Arc::from)
        };
        Self {
            admin: read("RUSCKER_ADMIN_TOKEN"),
            editor: read("RUSCKER_EDITOR_TOKEN"),
            viewer: read("RUSCKER_VIEWER_TOKEN"),
        }
    }

    /// Configure the admin token only (used by tests / programmatic
    /// setup). Editor and viewer stay unset ⇒ single-admin behaviour.
    pub fn with_token(token: impl Into<String>) -> Self {
        Self {
            admin: Some(Arc::from(token.into())),
            editor: None,
            viewer: None,
        }
    }

    /// Admin auth is usable iff the admin token is set. The optional
    /// editor/viewer tokens layer on top but never replace it.
    pub fn is_configured(&self) -> bool {
        self.admin.is_some()
    }

    /// Constant-time-compare `candidate` against each configured
    /// token and return the matched [`Role`], or `None` if it matches
    /// none. Checked most-privileged first so a token reused across
    /// levels (a misconfiguration) resolves to the higher role.
    ///
    /// Every comparison uses [`ct_eq`]: mismatched lengths short-
    /// circuit (the length leak is intentional and standard), and the
    /// XOR-fold over equal-length bytes is data-independent.
    pub fn role_for(&self, candidate: &str) -> Option<Role> {
        let matches = |tok: &Option<Arc<str>>| {
            tok.as_deref()
                .is_some_and(|t| ct_eq(t.as_bytes(), candidate.as_bytes()))
        };
        if matches(&self.admin) {
            Some(Role::Admin)
        } else if matches(&self.editor) {
            Some(Role::Editor)
        } else if matches(&self.viewer) {
            Some(Role::Viewer)
        } else {
            None
        }
    }
}

/// One live admin session: when it expires and the [`Role`] it was
/// minted with. Copy-cheap so lookups can read it out of the map by
/// value without holding the shard lock.
#[derive(Clone, Copy, Debug)]
struct SessionEntry {
    expiry: Instant,
    role: Role,
}

/// Server-side store of opaque admin session ids → (expiry, role).
///
/// The cookie holds a random session id, **not** the admin token. That
/// way a stolen cookie can be revoked (logout, or a server restart) and
/// never exposes the master token, and logout actually invalidates the
/// session instead of just clearing the browser's copy. The stored
/// [`Role`] is what route guards and the nav dispatch on.
#[derive(Debug)]
pub struct AdminSessions {
    sessions: DashMap<String, SessionEntry>,
    ttl: Duration,
}

impl AdminSessions {
    pub fn new(ttl: Duration) -> Self {
        Self {
            sessions: DashMap::new(),
            ttl,
        }
    }

    /// 24h default lifetime — matches the cookie's `Max-Age`.
    pub fn default_policy() -> Self {
        Self::new(Duration::from_secs(24 * 60 * 60))
    }

    /// Mint a new session for `role` and return its opaque id (244
    /// bits of random, unguessable). Caller stores the id in the
    /// cookie.
    pub fn create(&self, role: Role) -> String {
        let id = format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        );
        self.sessions.insert(
            id.clone(),
            SessionEntry {
                expiry: Instant::now() + self.ttl,
                role,
            },
        );
        id
    }

    /// The [`Role`] of a live (non-expired) session named by `id`, or
    /// `None` if the id is unknown or expired. Expired entries are
    /// pruned lazily on lookup.
    pub fn validate(&self, id: &str) -> Option<Role> {
        match self.sessions.get(id).map(|e| *e.value()) {
            Some(entry) if entry.expiry > Instant::now() => Some(entry.role),
            Some(_) => {
                self.sessions.remove(id);
                None
            }
            None => None,
        }
    }

    /// Invalidate a session (logout).
    pub fn remove(&self, id: &str) {
        self.sessions.remove(id);
    }
}

impl Default for AdminSessions {
    fn default() -> Self {
        Self::default_policy()
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

/// Extracted by admin routes to assert that the request carries a
/// valid session; exposes the session's [`Role`]. Absence of a valid
/// session redirects to the login form (303 See Other so re-POST
/// isn't suggested by browsers).
///
/// This guard accepts **any** role (≥ Viewer) — use it on routes
/// every logged-in user may reach (the dashboard view). For
/// privileged routes, wrap it with [`RequireEditor`] / [`RequireAdmin`].
///
/// When the server has NO admin token configured at all, the
/// extractor returns 503 — admin routes simply cannot work
/// without a token to compare against.
pub struct AdminSession {
    pub role: Role,
}

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
        // The cookie carries an opaque session id, validated against
        // the server-side store — not the token itself. A live session
        // yields its role.
        match cookies.get(COOKIE_NAME).map(|c| c.value().to_string()) {
            Some(c) => match state.admin_sessions.validate(&c) {
                Some(role) => Ok(AdminSession { role }),
                None => Err(Redirect::to("/admin/login").into_response()),
            },
            None => Err(Redirect::to("/admin/login").into_response()),
        }
    }
}

/// 403 response for an authenticated session that lacks the required
/// role. Distinct from the login redirect: the caller *is* logged in,
/// they just can't reach this section.
fn forbidden() -> Response {
    (
        StatusCode::FORBIDDEN,
        "forbidden — your role can't access this section",
    )
        .into_response()
}

/// Guard for routes that require **at least** [`Role::Editor`]
/// (apps + media + the dashboard's stop/restart actions). A valid
/// Viewer session is rejected with 403; an unauthenticated request
/// still redirects to login. Carries the role for the handler /
/// template.
pub struct RequireEditor {
    pub role: Role,
}

impl FromRequestParts<crate::AppState> for RequireEditor {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &crate::AppState,
    ) -> Result<Self, Self::Rejection> {
        let session = AdminSession::from_request_parts(parts, state).await?;
        if session.role >= Role::Editor {
            Ok(RequireEditor { role: session.role })
        } else {
            Err(forbidden())
        }
    }
}

/// Guard for routes that require [`Role::Admin`] (credentials,
/// landing editor, custom blocks, audit log). Anything below Admin is
/// rejected with 403. Carries the role for the handler / template.
pub struct RequireAdmin {
    pub role: Role,
}

impl FromRequestParts<crate::AppState> for RequireAdmin {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &crate::AppState,
    ) -> Result<Self, Self::Rejection> {
        let session = AdminSession::from_request_parts(parts, state).await?;
        if session.role == Role::Admin {
            Ok(RequireAdmin { role: session.role })
        } else {
            Err(forbidden())
        }
    }
}

/// Convenience extractor for routes that want to know the caller's
/// role without rejecting — used by `_layout.html` so the navbar can
/// gate links and show a "logout". `None` ⇒ not authenticated.
pub struct MaybeAdminSession(pub Option<Role>);

impl FromRequestParts<crate::AppState> for MaybeAdminSession {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &crate::AppState,
    ) -> Result<Self, Self::Rejection> {
        match AdminSession::from_request_parts(parts, state).await {
            Ok(s) => Ok(MaybeAdminSession(Some(s.role))),
            Err(_) => Ok(MaybeAdminSession(None)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use std::time::Duration;

    #[test]
    fn admin_sessions_create_validate_remove() {
        let s = AdminSessions::default_policy();
        let id = s.create(Role::Editor);
        assert_eq!(
            s.validate(&id),
            Some(Role::Editor),
            "freshly created session is valid and carries its role"
        );
        assert_eq!(s.validate("not-a-real-id"), None, "unknown id is invalid");
        s.remove(&id);
        assert_eq!(
            s.validate(&id),
            None,
            "removed (logged-out) session is invalid"
        );
    }

    #[test]
    fn admin_sessions_expire() {
        let s = AdminSessions::new(Duration::from_millis(0));
        let id = s.create(Role::Admin);
        std::thread::sleep(Duration::from_millis(2));
        assert_eq!(s.validate(&id), None, "expired session is invalid");
    }

    #[test]
    fn admin_session_ids_are_distinct_and_not_the_token() {
        let s = AdminSessions::default_policy();
        let a = s.create(Role::Viewer);
        let b = s.create(Role::Viewer);
        assert_ne!(a, b, "each session gets a fresh id");
        assert!(
            a.len() >= 32,
            "id is high-entropy, not a short/guessable value"
        );
    }

    #[test]
    fn role_for_resolves_each_token_and_rejects_others() {
        let auth = AdminAuth {
            admin: Some(Arc::from("admin-secret")),
            editor: Some(Arc::from("editor-secret")),
            viewer: Some(Arc::from("viewer-secret")),
        };
        assert_eq!(auth.role_for("admin-secret"), Some(Role::Admin));
        assert_eq!(auth.role_for("editor-secret"), Some(Role::Editor));
        assert_eq!(auth.role_for("viewer-secret"), Some(Role::Viewer));
        assert_eq!(auth.role_for("nope"), None);
        assert_eq!(auth.role_for(""), None);
    }

    #[test]
    fn role_for_admin_only_is_backward_compatible() {
        // Only RUSCKER_ADMIN_TOKEN set ⇒ single-admin behaviour.
        let auth = AdminAuth::with_token("only-admin");
        assert!(auth.is_configured());
        assert_eq!(auth.role_for("only-admin"), Some(Role::Admin));
        assert_eq!(auth.role_for("anything-else"), None);
    }

    #[test]
    fn role_for_prefers_higher_privilege_on_token_reuse() {
        // A token reused across levels resolves to the higher role.
        let auth = AdminAuth {
            admin: Some(Arc::from("shared")),
            editor: Some(Arc::from("shared")),
            viewer: None,
        };
        assert_eq!(auth.role_for("shared"), Some(Role::Admin));
    }

    #[test]
    fn role_ordering_and_section_matrix() {
        assert!(Role::Admin > Role::Editor);
        assert!(Role::Editor > Role::Viewer);

        // Dashboard: everyone.
        for r in [Role::Viewer, Role::Editor, Role::Admin] {
            assert!(r.can_access_section("dashboard"));
        }
        // Apps + media: Editor and up.
        for sec in ["specs", "images"] {
            assert!(!Role::Viewer.can_access_section(sec));
            assert!(Role::Editor.can_access_section(sec));
            assert!(Role::Admin.can_access_section(sec));
        }
        // Admin-only sections.
        for sec in ["credentials", "landing", "blocks", "audit"] {
            assert!(!Role::Viewer.can_access_section(sec));
            assert!(!Role::Editor.can_access_section(sec));
            assert!(Role::Admin.can_access_section(sec));
        }
    }

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
