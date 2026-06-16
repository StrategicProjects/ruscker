//! Admin authentication — per-user accounts + a break-glass token.
//!
//! Threat model: the operator runs Ruscker on a private network (or
//! behind a reverse proxy with mTLS). Each person signs in with a
//! **username + password** backed by the `users` table (see
//! [`crate::db::users`]; passwords are argon2id-hashed). The session
//! carries the user's [`Role`] and username.
//!
//! `RUSCKER_ADMIN_TOKEN` is the **break-glass** path: it always grants
//! an Admin session and, on a fresh install with no accounts, drives
//! the first-admin setup. It's the recovery route so an operator can
//! never be locked out — treat it as a sensitive secret. When it isn't
//! set, admin routes 503.
//!
//! Roles (see [`Role`]): **Viewer** (dashboard only), **Editor**
//! (apps + media + dashboard), and **Admin** (everything, incl. user
//! management). External IdPs (OIDC/SAML/LDAP) and per-app ACLs land
//! in Phase 8.
//!
//! Cookie: the value is an opaque server-side session id (never the
//! token or password), bound by `HttpOnly`, `Secure` (when TLS
//! terminated), and `SameSite=Strict`. Logout and server restart both
//! revoke it.

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
    /// Editor/Admin land on the **Apps list** — the main management
    /// surface — not the dashboard, whose live-metrics SSE starves the
    /// browser's connection pool on an HTTP/1.1 front end (#852). The
    /// dashboard stays one click away. A Viewer can only reach the
    /// dashboard (until #857 moves it to the portal), so it lands there.
    pub fn home(&self) -> &'static str {
        match self {
            Role::Viewer => "/admin/dashboard",
            _ => "/admin/specs",
        }
    }

    /// Fluent message key for the human-readable role label.
    pub fn label_key(&self) -> &'static str {
        match self {
            Role::Viewer => "role-viewer",
            Role::Editor => "role-editor",
            Role::Admin => "role-admin",
        }
    }

    /// Lowercase wire/DB form (`users.role`, form values).
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Editor => "editor",
            Role::Admin => "admin",
        }
    }

    /// Parse the lowercase form back to a [`Role`]. Unknown ⇒ `None`.
    pub fn parse(s: &str) -> Option<Role> {
        match s {
            "viewer" => Some(Role::Viewer),
            "editor" => Some(Role::Editor),
            "admin" => Some(Role::Admin),
            _ => None,
        }
    }
}

/// The break-glass admin token, held in `AppState`. `RUSCKER_ADMIN_TOKEN`
/// always grants an emergency Admin session (and bootstraps the first
/// account when no admin user exists yet — see [`crate::db::users`]).
/// `None` means admin routes are disabled (503). The token is wrapped
/// in `Arc<str>` so cloning [`AppState`] (per request) doesn't copy it.
///
/// Per-user Editor/Viewer access lives in the `users` table, not here —
/// this is purely the bootstrap / break-glass path.
#[derive(Clone, Debug, Default)]
pub struct AdminAuth {
    pub admin: Option<Arc<str>>,
}

impl AdminAuth {
    pub fn from_env() -> Self {
        Self {
            admin: std::env::var("RUSCKER_ADMIN_TOKEN")
                .ok()
                .filter(|s| !s.is_empty())
                .map(Arc::from),
        }
    }

    /// Configure the admin token (used by tests / the CLI flag).
    pub fn with_token(token: impl Into<String>) -> Self {
        Self {
            admin: Some(Arc::from(token.into())),
        }
    }

    /// Admin auth is usable iff the break-glass token is set.
    pub fn is_configured(&self) -> bool {
        self.admin.is_some()
    }

    /// Constant-time compare `candidate` against the break-glass token.
    /// `true` ⇒ grant an Admin session. [`ct_eq`] short-circuits on a
    /// length mismatch (intentional, standard) and is data-independent
    /// over equal-length bytes.
    pub fn matches_token(&self, candidate: &str) -> bool {
        self.admin
            .as_deref()
            .is_some_and(|t| ct_eq(t.as_bytes(), candidate.as_bytes()))
    }
}

/// One live admin session: when it expires, the [`Role`] it was minted
/// with, and the acting identity. `actor` is the DB username for a
/// password login, or `None` for a break-glass token session (no user
/// account behind it).
#[derive(Clone, Debug)]
struct SessionEntry {
    expiry: Instant,
    role: Role,
    actor: Option<String>,
}

/// What [`AdminSessionStore::validate`] yields for a live session —
/// the role and acting identity carried by the extractors.
#[derive(Clone, Debug)]
pub struct SessionInfo {
    pub role: Role,
    pub actor: Option<String>,
}

/// Server-side store of opaque admin session ids → (expiry, role).
///
/// The cookie holds a random session id, **not** the admin token. That
/// way a stolen cookie can be revoked (logout, or a server restart) and
/// never exposes the master token, and logout actually invalidates the
/// session instead of just clearing the browser's copy. The stored
/// [`Role`] is what route guards and the nav dispatch on.
///
/// **HA note (#161 → #185):** the default implementation
/// ([`InMemoryAdminSessionStore`]) is process-local — in active-active
/// HA, a load-balancer hop to another instance re-prompts login (and,
/// with per-group app visibility, the proxy on the other instance sees
/// the request as anonymous on `/app`). When operators wire up
/// `--admin-session-store-url postgres://…` the trait dispatches to
/// [`crate::admin_sessions_pg::PostgresAdminSessionStore`] instead,
/// which backs the store in a shared Postgres table with a short-TTL
/// local cache so the proxy hot path stays effectively DB-free.
///
/// A signed stateless cookie was rejected on purpose — opaque
/// server-side ids keep logout/restart revocable (SECURITY.md §1).
#[async_trait::async_trait]
pub trait AdminSessionStore: Send + Sync + std::fmt::Debug {
    /// Mint a new session for `role`/`actor` and return its opaque id.
    /// `actor` is the DB username (or `None` for a break-glass token
    /// session).
    async fn create(&self, role: Role, actor: Option<String>) -> String;

    /// Resolve `id` to its [`SessionInfo`] when the session is live, or
    /// `None` if the id is unknown or expired. Implementations may
    /// prune expired entries lazily on lookup.
    async fn validate(&self, id: &str) -> Option<SessionInfo>;

    /// Invalidate a session (logout / forced revoke).
    async fn remove(&self, id: &str);

    /// Invalidate **every** live session belonging to `actor` (the DB
    /// username). Called when a user is demoted, deleted, or has their
    /// password reset, so a stale session can't keep the old (possibly
    /// elevated) role until it expires (#544). Break-glass token
    /// sessions (`actor == None`) are never matched.
    async fn revoke_by_actor(&self, actor: &str);
}

/// Single-node admin session store. The default; replaced by
/// [`crate::admin_sessions_pg::PostgresAdminSessionStore`] when the
/// operator passes `--admin-session-store-url`.
#[derive(Debug)]
pub struct InMemoryAdminSessionStore {
    sessions: DashMap<String, SessionEntry>,
    ttl: Duration,
}

impl InMemoryAdminSessionStore {
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
}

/// Mint an opaque session id. Two concatenated UUID-v4 simples →
/// 244 bits of random, unguessable. Shared between the in-memory and
/// Postgres stores so the wire format stays uniform.
pub(crate) fn mint_session_id() -> String {
    format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    )
}

#[async_trait::async_trait]
impl AdminSessionStore for InMemoryAdminSessionStore {
    async fn create(&self, role: Role, actor: Option<String>) -> String {
        let id = mint_session_id();
        self.sessions.insert(
            id.clone(),
            SessionEntry {
                expiry: Instant::now() + self.ttl,
                role,
                actor,
            },
        );
        id
    }

    async fn validate(&self, id: &str) -> Option<SessionInfo> {
        let info = self.sessions.get(id).map(|e| {
            let v = e.value();
            (v.expiry, v.role, v.actor.clone())
        });
        match info {
            Some((expiry, role, actor)) if expiry > Instant::now() => {
                Some(SessionInfo { role, actor })
            }
            Some(_) => {
                self.sessions.remove(id);
                None
            }
            None => None,
        }
    }

    async fn remove(&self, id: &str) {
        self.sessions.remove(id);
    }

    async fn revoke_by_actor(&self, actor: &str) {
        // Drop every entry whose actor matches; token sessions (None)
        // never match a username, so they're left alone.
        self.sessions
            .retain(|_, e| e.actor.as_deref() != Some(actor));
    }
}

impl Default for InMemoryAdminSessionStore {
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
pub fn request_is_https(headers: &axum::http::HeaderMap, forwarded_trusted: bool) -> bool {
    // Same trust model as every other X-Forwarded-* read (#744): only
    // honoured when the operator opted in (`server.useForwardHeaders`).
    // Before this, ANY client could flip a cookie's `Secure` flag with
    // a spoofed header. TLS-terminating deployments must set the flag
    // — ShinyProxy-migrated configs already do.
    if !forwarded_trusted {
        return false;
    }
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        // XFP can be a comma list ("https, http") when chained; take
        // the RIGHTMOST entry — the one appended by the trusted proxy
        // closest to us. The leftmost is client-controlled whenever a
        // proxy appends instead of replacing (the spoof-unsafe slot,
        // same rationale as `client_key`'s XFF handling).
        .map(|s| {
            s.rsplit(',')
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
    /// DB username for a password login, or `None` for a break-glass
    /// token session. Use [`AdminSession::actor`] for audit logging.
    pub actor: Option<String>,
}

impl AdminSession {
    /// The actor string to record in the audit log: the username, or
    /// `"token"` for a break-glass token session.
    pub fn actor(&self) -> &str {
        self.actor.as_deref().unwrap_or("token")
    }
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
        // yields its role + actor.
        match cookies.get(COOKIE_NAME).map(|c| c.value().to_string()) {
            Some(c) => match state.admin_sessions.validate(&c).await {
                Some(info) => Ok(AdminSession {
                    role: info.role,
                    actor: info.actor,
                }),
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
    pub actor: Option<String>,
}

impl RequireEditor {
    pub fn actor(&self) -> &str {
        self.actor.as_deref().unwrap_or("token")
    }
}

impl FromRequestParts<crate::AppState> for RequireEditor {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &crate::AppState,
    ) -> Result<Self, Self::Rejection> {
        let session = AdminSession::from_request_parts(parts, state).await?;
        if session.role >= Role::Editor {
            Ok(RequireEditor {
                role: session.role,
                actor: session.actor,
            })
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
    pub actor: Option<String>,
}

impl RequireAdmin {
    pub fn actor(&self) -> &str {
        self.actor.as_deref().unwrap_or("token")
    }
}

impl FromRequestParts<crate::AppState> for RequireAdmin {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &crate::AppState,
    ) -> Result<Self, Self::Rejection> {
        let session = AdminSession::from_request_parts(parts, state).await?;
        if session.role == Role::Admin {
            Ok(RequireAdmin {
                role: session.role,
                actor: session.actor,
            })
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

/// Like [`MaybeAdminSession`] but keeps the whole session (role +
/// username) instead of just the role. Used by the public landing to
/// resolve the viewer's group memberships and show a "signed in"
/// affordance. `None` ⇒ anonymous visitor.
pub struct MaybeSession(pub Option<AdminSession>);

impl FromRequestParts<crate::AppState> for MaybeSession {
    type Rejection = Infallible;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &crate::AppState,
    ) -> Result<Self, Self::Rejection> {
        match AdminSession::from_request_parts(parts, state).await {
            Ok(s) => Ok(MaybeSession(Some(s))),
            Err(_) => Ok(MaybeSession(None)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use std::time::Duration;

    #[tokio::test]
    async fn admin_sessions_create_validate_remove() {
        let s = InMemoryAdminSessionStore::default_policy();
        let id = s.create(Role::Editor, Some("alice".into())).await;
        let info = s
            .validate(&id)
            .await
            .expect("freshly created session is valid");
        assert_eq!(info.role, Role::Editor);
        assert_eq!(info.actor.as_deref(), Some("alice"));
        assert!(
            s.validate("not-a-real-id").await.is_none(),
            "unknown id is invalid"
        );
        s.remove(&id).await;
        assert!(
            s.validate(&id).await.is_none(),
            "removed (logged-out) session is invalid"
        );
    }

    #[tokio::test]
    async fn admin_sessions_revoke_by_actor() {
        let s = InMemoryAdminSessionStore::default_policy();
        // Two sessions for alice (e.g. two browsers), one for bob, one token.
        let a1 = s.create(Role::Admin, Some("alice".into())).await;
        let a2 = s.create(Role::Admin, Some("alice".into())).await;
        let bob = s.create(Role::Editor, Some("bob".into())).await;
        let token = s.create(Role::Admin, None).await;

        s.revoke_by_actor("alice").await;

        assert!(s.validate(&a1).await.is_none(), "alice session 1 revoked");
        assert!(s.validate(&a2).await.is_none(), "alice session 2 revoked");
        assert!(
            s.validate(&bob).await.is_some(),
            "another user's session is untouched"
        );
        assert!(
            s.validate(&token).await.is_some(),
            "break-glass token session (actor=None) is never matched"
        );
    }

    #[tokio::test]
    async fn admin_sessions_expire() {
        let s = InMemoryAdminSessionStore::new(Duration::from_millis(0));
        let id = s.create(Role::Admin, None).await;
        std::thread::sleep(Duration::from_millis(2));
        assert!(
            s.validate(&id).await.is_none(),
            "expired session is invalid"
        );
    }

    #[tokio::test]
    async fn admin_session_ids_are_distinct_and_not_the_token() {
        let s = InMemoryAdminSessionStore::default_policy();
        let a = s.create(Role::Viewer, None).await;
        let b = s.create(Role::Viewer, None).await;
        assert_ne!(a, b, "each session gets a fresh id");
        assert!(
            a.len() >= 32,
            "id is high-entropy, not a short/guessable value"
        );
    }

    #[test]
    fn matches_token_is_constant_time_exact() {
        let auth = AdminAuth::with_token("admin-secret");
        assert!(auth.is_configured());
        assert!(auth.matches_token("admin-secret"));
        assert!(!auth.matches_token("admin-secre")); // length mismatch
        assert!(!auth.matches_token("nope"));
        assert!(!auth.matches_token(""));
        // Unconfigured never matches.
        assert!(!AdminAuth::default().matches_token("anything"));
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
    fn request_is_https_reads_x_forwarded_proto_when_trusted() {
        let mut h = HeaderMap::new();
        assert!(!request_is_https(&h, true), "no header → not https");
        h.insert("x-forwarded-proto", "http".parse().unwrap());
        assert!(!request_is_https(&h, true));
        h.insert("x-forwarded-proto", "https".parse().unwrap());
        assert!(request_is_https(&h, true));
    }

    // #744: XFP rides the same opt-in as every other forwarded header —
    // without `server.useForwardHeaders`, a direct client could flip a
    // cookie's `Secure` flag with a spoofed header.
    #[test]
    fn request_is_https_ignores_header_when_untrusted() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-proto", "https".parse().unwrap());
        assert!(!request_is_https(&h, false), "untrusted → header ignored");
    }

    #[test]
    fn request_is_https_handles_chained_proto_list() {
        let mut h = HeaderMap::new();
        // Chained proxies: the RIGHTMOST entry is the one the trusted
        // proxy appended — the spoof-safe slot (#744). A client sending
        // "https" through an appending plain-HTTP proxy must not win.
        h.insert("x-forwarded-proto", "https, http".parse().unwrap());
        assert!(!request_is_https(&h, true), "client-spoofed leftmost loses");
        h.insert("x-forwarded-proto", "http, https".parse().unwrap());
        assert!(request_is_https(&h, true), "trusted rightmost wins");
        h.insert("x-forwarded-proto", "HTTPS".parse().unwrap());
        assert!(request_is_https(&h, true), "case-insensitive");
    }
}
