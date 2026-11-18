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
//! Roles (see [`Role`]): **Viewer** (portal only), **Editor**
//! (apps + media + group-scoped user management), and **Admin**
//! (everything). External IdPs (OIDC/SAML/LDAP) and per-app ACLs land
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
    /// Portal end user: app visibility comes from group membership.
    Viewer,
    /// Manages apps, media and group-scoped users and has the full
    /// dashboard, including the stop/restart actions.
    Editor,
    /// Everything — credentials, landing editor, custom blocks, audit
    /// log. The historical single-token behaviour.
    Admin,
}

impl Role {
    /// Whether this role may reach the given admin nav section.
    /// `section` matches the `nav_section` strings used by the
    /// templates (`dashboard`, `specs`, `images`, `users`, `groups`,
    /// `credentials`, `landing`, `blocks`, `audit`).
    pub fn can_access_section(&self, section: &str) -> bool {
        match section {
            // Dashboard + apps + media + scoped users/groups: Editor and up. A
            // Viewer is NOT an admin-panel operator (#857) — it's the portal's
            // authenticated-end-user role (group-based card visibility,
            // #155), so it reaches NO admin section.
            "dashboard" | "specs" | "images" | "users" | "groups" => *self >= Role::Editor,
            // Everything else — credentials, landing, logs, disk, audit,
            // system, schedules — is admin-only.
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
    /// A **Viewer** has no panel access (#857) — it logs in to unlock
    /// its group's cards, so it lands on the public portal.
    /// **Editor/Admin** land on the **Apps list**, not the dashboard,
    /// whose live-metrics SSE starves the browser's connection pool on
    /// an HTTP/1.1 front end (#852); the dashboard is one click away.
    pub fn home(&self) -> &'static str {
        match self {
            Role::Viewer => "/",
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

    /// Parse a wire/DB role case-insensitively. Unknown ⇒ `None`.
    ///
    /// Writes stay canonical lowercase through [`Self::as_str`], but the
    /// column has no CHECK and external identity provisioners may vary casing.
    pub fn parse(s: &str) -> Option<Role> {
        if s.eq_ignore_ascii_case("viewer") {
            Some(Role::Viewer)
        } else if s.eq_ignore_ascii_case("editor") {
            Some(Role::Editor)
        } else if s.eq_ignore_ascii_case("admin") {
            Some(Role::Admin)
        } else {
            None
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

/// What the sign-in throttle allows. The defaults ([`Default`]) are the
/// production policy; tests build smaller ones.
#[derive(Debug, Clone, Copy)]
pub struct LoginPolicy {
    /// Window of the address, token and global counters.
    pub window: std::time::Duration,
    /// Attempts in `window` from everywhere before the password form
    /// pauses for people it does not know. The last line, not the
    /// working limit.
    pub global_max: usize,
    /// Attempts in `window` from one address (an IPv6 /64 counts as one).
    pub address_max: usize,
    /// Attempts in `account_window` against one account, from anywhere.
    pub account_max: usize,
    pub account_window: std::time::Duration,
    /// After this many failures in a row for one account from one
    /// address, each further attempt waits `backoff_base`, then twice
    /// that, and so on (1 s, 2 s, 4 s …) up to `backoff_cap`.
    pub backoff_after: u32,
    pub backoff_base: std::time::Duration,
    pub backoff_cap: std::time::Duration,
    /// A streak is forgotten after this long without an attempt. Longer
    /// than `backoff_cap` on purpose: forgetting right after the longest
    /// wait would hand a guesser a fresh run of fast tries every time.
    pub streak_forget: std::time::Duration,
    /// How long a successful sign-in to an account from an address makes
    /// that pair "known" (see [`LoginRateLimiter`]).
    pub trust_for: std::time::Duration,
    /// Attempts in `window` on the break-glass **token** form, from
    /// everywhere together. Low and address-blind on purpose: the token
    /// is one secret that opens everything, so spreading guesses over
    /// many addresses must not buy more of them.
    pub token_max: usize,
}

impl Default for LoginPolicy {
    fn default() -> Self {
        use std::time::Duration;
        Self {
            window: Duration::from_secs(60),
            global_max: 300,
            address_max: 30,
            account_max: 30,
            account_window: Duration::from_secs(600),
            backoff_after: 3,
            backoff_base: Duration::from_secs(1),
            backoff_cap: Duration::from_secs(300),
            streak_forget: Duration::from_secs(3600),
            trust_for: Duration::from_secs(30 * 24 * 3600),
            token_max: 10,
        }
    }
}

/// Failures in a row for one account from one address.
#[derive(Debug, Clone, Copy)]
struct Streak {
    failures: u32,
    last: std::time::Instant,
}

type Window = std::collections::VecDeque<std::time::Instant>;

#[derive(Debug, Default)]
struct LimiterState {
    global: Window,
    /// Attempts on the token form, whatever their address.
    token: Window,
    by_address: std::collections::HashMap<String, Window>,
    by_account: std::collections::HashMap<String, Window>,
    streaks: std::collections::HashMap<(String, String), Streak>,
    /// (account, address) pairs that signed in correctly, and when.
    known: std::collections::HashMap<(String, String), std::time::Instant>,
    /// Addresses the break-glass token was used correctly from, and when.
    known_token: std::collections::HashMap<String, std::time::Instant>,
    /// Token attempts from each of those addresses.
    token_by_address: std::collections::HashMap<String, Window>,
}

/// An attempt the throttle let through. It is **already counted as a
/// failure**; pass it to [`LoginRateLimiter::success`] when the
/// credentials turn out right, and simply drop it otherwise.
#[derive(Debug)]
#[must_use = "an admitted attempt is counted as failed unless passed to `success`"]
pub struct Attempt {
    at: std::time::Instant,
    account: Option<String>,
    address: String,
    token: bool,
}

/// Throttle for sign-in attempts (password and break-glass token).
///
/// Guessing a password is the threat; locking honest people out is the
/// cost to avoid. The first version was one global window: it stopped
/// guessing, but anyone could pause sign-in **for everybody** by failing
/// on purpose — tolerable for a single-operator install, not for a
/// portal that is the sign-in of several systems. Attempts are now
/// counted where they belong, in layers that each refuse on their own:
///
/// 1. **Account + address** — after a few failures in a row, each new
///    attempt waits twice as long as the last. Slows a guesser to a
///    crawl without affecting the same account from another address.
/// 2. **Account**, from anywhere — a ceiling per ten minutes, so
///    spreading guesses over many addresses does not help.
/// 3. **Address** — a ceiling per minute, against trying one password
///    on many accounts.
/// 4. **Global** — a high ceiling, the last line.
///
/// **Counted when admitted, not when it fails.** Checking a password is
/// slow on purpose (argon2). If attempts were only counted once they had
/// failed, a burst of parallel requests would all be let in before the
/// first failure was recorded, and every ceiling above would be soft.
/// So [`Self::begin`] checks and counts under one lock, and a correct
/// sign-in gives its slot back ([`Self::success`]).
///
/// **People the portal knows are not held hostage by strangers.**
/// Layers 2–4 are collective: someone failing on purpose could use them
/// to keep a chosen account — or, with a few addresses, everyone — out.
/// A pair (account, address) that has signed in correctly is therefore
/// exempt from those three for a while; it keeps its own streak (layer
/// 1), so the exemption is worth nothing to someone guessing from that
/// address. What remains, and cannot be fixed at this layer (it would
/// take something per browser, like a device cookie):
///
/// * an account can be kept out **from an address it has never signed
///   in from** by someone who keeps failing against it;
/// * someone **at the same address** as a person (an office NAT — or
///   everyone, behind a proxy without trusted forwarded headers) can
///   keep that person's streak alive from there with a wrong password
///   every few minutes;
/// * a co-located observer can tell a known pair from an unknown one by
///   which attempts the ceilings refuse.
///
/// The memory of known pairs is per process and lost on restart.
///
/// The break-glass **token** form has no account, and its secret opens
/// everything, so it keeps what the first version had: one low ceiling
/// for all addresses together ([`LoginPolicy::token_max`]). It shares
/// no counter with the password form: flooding one does not close the
/// other, from any address. Failing on purpose can pause the token
/// form for strangers — but not for an address the token was used
/// correctly from, which has a window of its own. After a restart
/// nobody is known yet; an operator locked out then still has the
/// server itself (restart, or a new token).
///
/// The address is the client's as the portal resolves it for auditing:
/// from `X-Forwarded-For` only when the operator trusts forwarded
/// headers, the TCP peer otherwise. Behind a proxy WITHOUT that trust
/// every request has the proxy's address: layer 3 becomes one shared
/// ceiling for everyone (as the first version was, at 30 instead of 10
/// per minute) and layer 1 no longer tells people apart. Set
/// `server.useForwardHeaders` behind a proxy.
///
/// In memory, per process: an HA deployment throttles per node.
#[derive(Debug, Default)]
pub struct LoginRateLimiter {
    state: std::sync::Mutex<LimiterState>,
    policy: LoginPolicy,
}

/// Most distinct keys tracked per map — a backstop. Admitted attempts
/// are bounded by the ceilings, so in practice the maps stay far below.
const LIMITER_MAX_KEYS: usize = 20_000;
/// Most addresses remembered as known for one account.
const KNOWN_PER_ACCOUNT: usize = 50;
/// Longest account key kept (a typed username can be any length).
const LIMITER_KEY_MAX_CHARS: usize = 128;

/// Drop instants older than `window`; if the window is still full,
/// return how long until its oldest entry leaves.
fn full(
    q: &mut Window,
    max: usize,
    window: std::time::Duration,
    now: std::time::Instant,
) -> Option<std::time::Duration> {
    while q.front().is_some_and(|t| now.duration_since(*t) > window) {
        q.pop_front();
    }
    if q.len() < max {
        return None;
    }
    // `max == 0` with an empty window: refuse for a whole window.
    Some(q.front().map_or(window, |oldest| window.saturating_sub(now.duration_since(*oldest))))
}

/// Remove one occurrence of `at` (the newest matching one).
fn give_back(q: &mut Window, at: std::time::Instant) {
    if let Some(i) = q.iter().rposition(|t| *t == at) {
        q.remove(i);
    }
}

impl LoginRateLimiter {
    pub fn with_policy(policy: LoginPolicy) -> Self {
        Self { state: std::sync::Mutex::new(LimiterState::default()), policy }
    }

    /// The production policy ([`LoginPolicy::default`]).
    pub fn default_policy() -> Self {
        Self::with_policy(LoginPolicy::default())
    }

    /// Admit a password attempt for `account` (the typed username) from
    /// `address`, or say how long to wait. An admitted attempt is counted
    /// at once; see [`Attempt`].
    pub fn begin(&self, account: &str, address: &str) -> Result<Attempt, std::time::Duration> {
        self.begin_at(std::time::Instant::now(), account, address)
    }

    /// Admit a break-glass token attempt from `address`.
    pub fn begin_token(&self, address: &str) -> Result<Attempt, std::time::Duration> {
        self.begin_token_at(std::time::Instant::now(), address)
    }

    /// The attempt's credentials were right: give its slot back, forget
    /// the account's failures and remember the pair as known. Other
    /// accounts are untouched — signing in to your own account must not
    /// buy guesses against someone else's.
    pub fn success(&self, attempt: Attempt) {
        self.success_at(std::time::Instant::now(), attempt)
    }

    /// The attempt could not be judged — the user store failed, say. It
    /// was nobody's wrong password, so it is uncounted; nothing else
    /// changes.
    pub fn cancel(&self, attempt: Attempt) {
        Self::uncount(&mut self.lock(), &attempt);
    }

    /// The lock, even after a panic elsewhere while holding it: a
    /// poisoned throttle must not become "no throttle", nor "nobody can
    /// sign in".
    fn lock(&self) -> std::sync::MutexGuard<'_, LimiterState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn begin_at(
        &self,
        now: std::time::Instant,
        account: &str,
        address: &str,
    ) -> Result<Attempt, std::time::Duration> {
        let p = self.policy;
        let account = account_key(account);
        let pair = account.clone().map(|a| (a, address.to_string()));
        let mut st = self.lock();
        Self::housekeeping(&mut st, &p, now);

        // Fail closed: an attempt that could not be counted is refused.
        // (Only at the backstop size of a map, after housekeeping.)
        fn room<K: std::hash::Hash + Eq, V>(map: &std::collections::HashMap<K, V>, key: &K) -> bool {
            map.len() < LIMITER_MAX_KEYS || map.contains_key(key)
        }
        let countable = room(&st.by_address, &address.to_string())
            && account.as_ref().is_none_or(|a| room(&st.by_account, a))
            && pair.as_ref().is_none_or(|k| room(&st.streaks, k));
        if !countable {
            return Err(p.window);
        }

        let known = pair
            .as_ref()
            .and_then(|k| st.known.get(k))
            .is_some_and(|since| now.duration_since(*since) <= p.trust_for);

        // The collective ceilings. Always evaluated (it prunes the
        // windows); they only REFUSE people the portal does not know.
        let global = full(&mut st.global, p.global_max, p.window, now);
        let by_address = st.by_address.get_mut(address).and_then(|q| full(q, p.address_max, p.window, now));
        let by_account = account
            .as_ref()
            .and_then(|a| st.by_account.get_mut(a))
            .and_then(|q| full(q, p.account_max, p.account_window, now));
        if !known {
            if let Some(wait) = global.or(by_address).or(by_account) {
                return Err(wait);
            }
        }
        // The pair's own streak, for everyone.
        if let Some(streak) = pair.as_ref().and_then(|k| st.streaks.get(k)) {
            let quiet = now.duration_since(streak.last);
            let wait = backoff(&p, streak.failures);
            if quiet < wait {
                return Err(wait - quiet);
            }
        }

        // Admitted: count it now (see the type's doc).
        st.global.push_back(now);
        st.by_address.entry(address.to_string()).or_default().push_back(now);
        if let Some(a) = &account {
            st.by_account.entry(a.clone()).or_default().push_back(now);
        }
        if let Some(key) = pair {
            let streak = st.streaks.entry(key).or_insert(Streak { failures: 0, last: now });
            if now.duration_since(streak.last) > p.streak_forget {
                streak.failures = 0;
            }
            streak.failures = streak.failures.saturating_add(1);
            streak.last = now;
        }
        Ok(Attempt { at: now, account, address: address.to_string(), token: false })
    }

    /// The token form shares NOTHING with the password form: its own
    /// address-blind window for strangers, and a window per address for
    /// addresses the token was used correctly from.
    fn begin_token_at(&self, now: std::time::Instant, address: &str) -> Result<Attempt, std::time::Duration> {
        let p = self.policy;
        let mut st = self.lock();
        Self::housekeeping(&mut st, &p, now);
        let known = st
            .known_token
            .get(address)
            .is_some_and(|since| now.duration_since(*since) <= p.trust_for);
        if known {
            // The operator's address: its own ceiling, so strangers
            // failing on purpose cannot close the break-glass path to it.
            let q = st.token_by_address.entry(address.to_string()).or_default();
            if let Some(wait) = full(q, p.token_max, p.window, now) {
                return Err(wait);
            }
            q.push_back(now);
        } else {
            if let Some(wait) = full(&mut st.token, p.token_max, p.window, now) {
                return Err(wait);
            }
            st.token.push_back(now);
        }
        Ok(Attempt { at: now, account: None, address: address.to_string(), token: true })
    }

    /// Give back what `begin` counted for this attempt, and step its
    /// streak back by one. Shared by [`Self::success`] and
    /// [`Self::cancel`].
    fn uncount(st: &mut LimiterState, attempt: &Attempt) {
        if attempt.token {
            give_back(&mut st.token, attempt.at);
            if let Some(q) = st.token_by_address.get_mut(&attempt.address) {
                give_back(q, attempt.at);
            }
            return;
        }
        give_back(&mut st.global, attempt.at);
        if let Some(q) = st.by_address.get_mut(&attempt.address) {
            give_back(q, attempt.at);
        }
        if let Some(account) = &attempt.account {
            if let Some(q) = st.by_account.get_mut(account) {
                give_back(q, attempt.at);
            }
            let key = (account.clone(), attempt.address.clone());
            if let Some(streak) = st.streaks.get_mut(&key) {
                streak.failures = streak.failures.saturating_sub(1);
                if streak.failures == 0 {
                    st.streaks.remove(&key);
                }
            }
        }
    }

    fn success_at(&self, now: std::time::Instant, attempt: Attempt) {
        let mut st = self.lock();
        Self::uncount(&mut st, &attempt);
        if attempt.token {
            // The operator is in: the token form starts clean for
            // everyone, and this address is remembered.
            st.token.clear();
            st.token_by_address.remove(&attempt.address);
            if st.known_token.len() < LIMITER_MAX_KEYS || st.known_token.contains_key(&attempt.address) {
                st.known_token.insert(attempt.address, now);
            }
            return;
        }
        let Some(account) = attempt.account else { return };
        let key = (account, attempt.address);
        // Earlier failures on this pair are HALVED, not erased. Erasing
        // them would let someone guessing from an address they share
        // with the account's owner start over at full speed every time
        // the owner signs in; an hour without attempts forgets the rest.
        // (The account's own window is likewise left to expire.)
        if let Some(streak) = st.streaks.get_mut(&key) {
            streak.failures /= 2;
            if streak.failures == 0 {
                st.streaks.remove(&key);
            }
        }
        // Remember the pair — at most `KNOWN_PER_ACCOUNT` addresses per
        // account, oldest out, so one account cannot fill the table.
        if !st.known.contains_key(&key) {
            let mut mine: Vec<(std::time::Instant, String)> = st
                .known
                .iter()
                .filter(|((a, _), _)| *a == key.0)
                .map(|((_, addr), since)| (*since, addr.clone()))
                .collect();
            if mine.len() >= KNOWN_PER_ACCOUNT {
                mine.sort();
                if let Some((_, oldest)) = mine.into_iter().next() {
                    st.known.remove(&(key.0.clone(), oldest));
                }
            }
        }
        if st.known.len() < LIMITER_MAX_KEYS || st.known.contains_key(&key) {
            st.known.insert(key, now);
        }
    }

    /// Drop expired entries when a map is at its backstop size.
    fn housekeeping(st: &mut LimiterState, p: &LoginPolicy, now: std::time::Instant) {
        let at_limit = [st.by_address.len(), st.by_account.len(), st.streaks.len(), st.known.len(), st.known_token.len()]
            .iter()
            .any(|n| *n >= LIMITER_MAX_KEYS);
        if !at_limit {
            return;
        }
        st.by_address.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) <= p.window));
        st.by_account.retain(|_, q| q.back().is_some_and(|t| now.duration_since(*t) <= p.account_window));
        st.streaks.retain(|_, s| now.duration_since(s.last) <= p.streak_forget);
        st.known.retain(|_, since| now.duration_since(*since) <= p.trust_for);
        st.known_token.retain(|_, since| now.duration_since(*since) <= p.trust_for);
        let known_token = &st.known_token;
        st.token_by_address.retain(|addr, _| known_token.contains_key(addr));
    }
}

/// How long to wait after `failures` in a row: nothing until
/// `backoff_after`, then `backoff_base` doubling up to the cap.
fn backoff(p: &LoginPolicy, failures: u32) -> std::time::Duration {
    if failures < p.backoff_after {
        return std::time::Duration::ZERO;
    }
    let doublings = (failures - p.backoff_after).min(20);
    p.backoff_base.saturating_mul(1u32 << doublings).min(p.backoff_cap)
}

/// The key an account is counted under: the typed username normalised
/// exactly as the user store does ([`crate::db::users::normalize_username`]),
/// then cut to a sane length. NOT looked up: an unknown username is
/// throttled exactly like a real one, and reveals nothing by behaving
/// differently.
fn account_key(account: &str) -> Option<String> {
    let key: String = crate::db::users::normalize_username(account)
        .chars()
        .take(LIMITER_KEY_MAX_CHARS)
        .collect();
    (!key.is_empty()).then_some(key)
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

/// The password policy, in ONE place for every path that sets one
/// (create user, admin reset, first-admin setup, own-password change,
/// CSV import): at least 8 characters with at least one uppercase
/// letter, one lowercase letter, one digit, and one special
/// (non-alphanumeric) character. Length counts characters, not bytes,
/// so `Sécurité#1` isn't over-credited; upper/lower use the Unicode
/// classes (an accented letter counts), digits are ASCII.
///
/// Composition rules are a deliberate trade-off: modern NIST guidance
/// prefers plain length + a blocklist, but composition is what audits
/// commonly require, and with `must_change_password` + the login rate
/// limiter the cost is low. Existing hashes are untouched — the policy
/// applies only when a password is (re)set.
pub fn password_meets_policy(password: &str) -> bool {
    password.chars().count() >= 8
        && password.chars().any(|c| c.is_uppercase())
        && password.chars().any(|c| c.is_lowercase())
        && password.chars().any(|c| c.is_ascii_digit())
        && password.chars().any(|c| !c.is_alphanumeric())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderMap;
    use std::time::Duration;

    #[test]
    fn password_policy_requires_all_four_classes_and_length() {
        // The old-world passwords the policy exists to reject.
        assert!(!password_meets_policy("teste123"), "no upper, no special");
        assert!(!password_meets_policy("12345678"), "digits only");
        assert!(!password_meets_policy("Senh!X1"), "7 chars — too short");
        assert!(!password_meets_policy("SENHA!123"), "no lowercase");
        assert!(!password_meets_policy("senha!123"), "no uppercase");
        assert!(!password_meets_policy("Senhas!!"), "no digit");
        assert!(!password_meets_policy("Senha123"), "no special");
        // Compliant.
        assert!(password_meets_policy("Senha!123"));
        assert!(password_meets_policy("Str0ng#pass"));
        // Unicode: accented letters count as letters (à = lowercase),
        // and length is chars, not bytes.
        assert!(password_meets_policy("Sécurité#1"));
        assert!(!password_meets_policy("Sé#1aB"), "6 chars even if 8+ bytes");
    }

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

        // Dashboard + apps + media + scoped users/groups: Editor and up — a
        // Viewer reaches no admin section (#857).
        for sec in ["dashboard", "specs", "images", "users", "groups"] {
            assert!(!Role::Viewer.can_access_section(sec));
            assert!(Role::Editor.can_access_section(sec));
            assert!(Role::Admin.can_access_section(sec));
        }
        // A Viewer's home is the portal (no panel); operators land on it.
        assert_eq!(Role::Viewer.home(), "/");
        assert_eq!(Role::Editor.home(), "/admin/specs");
        assert_eq!(Role::Admin.home(), "/admin/specs");
        // Admin-only sections.
        for sec in ["credentials", "landing", "blocks", "audit", "schedules"] {
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

    /// A small policy so the tests read easily.
    fn policy() -> LoginPolicy {
        LoginPolicy {
            window: Duration::from_secs(60),
            global_max: 50,
            address_max: 8,
            account_max: 6,
            account_window: Duration::from_secs(600),
            backoff_after: 3,
            backoff_base: Duration::from_secs(1),
            backoff_cap: Duration::from_secs(300),
            streak_forget: Duration::from_secs(3600),
            trust_for: Duration::from_secs(86_400),
            token_max: 4,
        }
    }

    /// A failed attempt at `at`: admitted, then dropped.
    fn fail(rl: &LoginRateLimiter, at: std::time::Instant, who: &str, from: &str) {
        let _ = rl.begin_at(at, who, from).expect("should have been admitted");
    }

    #[test]
    fn repeated_failures_on_one_account_wait_longer_each_time() {
        // Roomy collective ceilings: this test is about one pair's streak.
        let rl = LoginRateLimiter::with_policy(LoginPolicy { account_max: 100, address_max: 100, ..policy() });
        let t0 = std::time::Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        // Three mistakes go through; then the waits start: 1 s, 2 s, 4 s.
        for s in 0..3 {
            fail(&rl, at(s), "ana", "10.0.0.1");
        }
        assert_eq!(rl.begin_at(at(2), "ana", "10.0.0.1").unwrap_err(), Duration::from_secs(1));
        fail(&rl, at(3), "ana", "10.0.0.1");
        assert_eq!(rl.begin_at(at(4), "ana", "10.0.0.1").unwrap_err(), Duration::from_secs(1));
        fail(&rl, at(5), "ana", "10.0.0.1");
        assert_eq!(rl.begin_at(at(5), "ana", "10.0.0.1").unwrap_err(), Duration::from_secs(4));

        // A refused attempt is not counted: asking again does not push
        // the wait further out.
        for _ in 0..10 {
            assert!(rl.begin_at(at(6), "ana", "10.0.0.1").is_err());
        }
        assert!(rl.begin_at(at(9), "ana", "10.0.0.1").is_ok());

        // The username is matched as the user store matches it.
        assert!(rl.begin_at(at(9), "  ANA ", "10.0.0.1").is_err());
        // Nobody else is slowed: another account, or the same account
        // from another address.
        assert!(rl.begin_at(at(9), "bia", "10.0.0.1").is_ok());
        assert!(rl.begin_at(at(9), "ana", "10.0.0.2").is_ok());
    }

    #[test]
    fn the_wait_is_capped_and_a_streak_is_not_forgotten_right_after_it() {
        let p = policy();
        assert_eq!(backoff(&p, 2), Duration::ZERO);
        assert_eq!(backoff(&p, 3), Duration::from_secs(1));
        assert_eq!(backoff(&p, 8), Duration::from_secs(32));
        assert_eq!(backoff(&p, 40), p.backoff_cap, "the cap, not 2^37 seconds");
        assert_eq!(backoff(&p, u32::MAX), p.backoff_cap);

        // Reach the cap, sit out the longest wait, fail once more: the
        // next wait is the cap again — sitting it out does not buy a
        // fresh run of fast guesses.
        let rl = LoginRateLimiter::with_policy(LoginPolicy { account_max: 1000, address_max: 1000, global_max: 1000, ..p });
        let mut now = std::time::Instant::now();
        for _ in 0..20 {
            fail(&rl, now, "ana", "a");
            now += rl.begin_at(now, "ana", "a").err().unwrap_or_default();
        }
        assert_eq!(rl.begin_at(now - Duration::from_secs(1), "ana", "a").unwrap_err(), Duration::from_secs(1));
        fail(&rl, now, "ana", "a");
        assert_eq!(rl.begin_at(now, "ana", "a").unwrap_err(), p.backoff_cap);
        // Only a long quiet (an hour here) forgets it.
        let later = now + p.streak_forget + Duration::from_secs(1);
        fail(&rl, later, "ana", "a");
        assert!(rl.begin_at(later, "ana", "a").is_ok());
    }

    /// The point of counting at admission: a burst that arrives before
    /// any password has been checked is still bounded.
    #[test]
    fn a_parallel_burst_is_bounded_before_any_password_is_checked() {
        let rl = LoginRateLimiter::with_policy(policy());
        let t0 = std::time::Instant::now();
        // 100 simultaneous attempts on one account from one address.
        let admitted = (0..100).filter(|_| rl.begin_at(t0, "ana", "a").is_ok()).count();
        assert_eq!(admitted, 3, "the streak admits three, whatever the timing");
        // 100 simultaneous attempts on 100 accounts from one address.
        let admitted = (0..100).filter(|i| rl.begin_at(t0, &format!("u{i}"), "b").is_ok()).count();
        assert_eq!(admitted, 8, "the address ceiling");
        // …and from 100 addresses on 100 accounts: the global ceiling
        // (3 + 8 were already admitted above).
        let admitted = (0..100).filter(|i| rl.begin_at(t0, &format!("v{i}"), &format!("c{i}")).is_ok()).count();
        assert_eq!(admitted, 50 - 3 - 8);
    }

    #[test]
    fn a_success_gives_the_slot_back_and_clears_that_account_only() {
        let rl = LoginRateLimiter::with_policy(LoginPolicy { address_max: 100, ..policy() });
        let t0 = std::time::Instant::now();
        for who in ["ana", "bia"] {
            for _ in 0..3 {
                fail(&rl, t0, who, "a");
            }
        }
        assert!(rl.begin_at(t0, "ana", "a").is_err() && rl.begin_at(t0, "bia", "a").is_err());
        // ana gets it right after her wait.
        let ok = rl.begin_at(t0 + Duration::from_secs(1), "Ana", "a").unwrap();
        rl.success_at(t0 + Duration::from_secs(1), ok);
        // Her earlier failures are halved, not erased: 3 → 1, so her next
        // attempts go through — but a guesser at her address did not
        // get a clean slate (see the next test).
        assert!(rl.begin_at(t0 + Duration::from_secs(1), "ana", "a").is_ok());
        assert!(rl.begin_at(t0, "bia", "a").is_err(), "signing in does not buy guesses on another account");

        // Many correct sign-ins from one address never fill its window:
        // each gives its slot back.
        let rl = LoginRateLimiter::with_policy(policy());
        for i in 0..50 {
            let who = format!("user{i}");
            let ok = rl.begin_at(t0, &who, "office-nat").expect("a correct sign-in is never throttled by others");
            rl.success_at(t0, ok);
        }
        assert!(rl.begin_at(t0, "someone-new", "office-nat").is_ok());
    }

    #[test]
    fn guesses_spread_over_many_addresses_hit_the_account_ceiling() {
        let rl = LoginRateLimiter::with_policy(policy());
        let t0 = std::time::Instant::now();
        // Six addresses, one guess each: no streak anywhere…
        for i in 0..6 {
            fail(&rl, t0, "ana", &format!("10.0.0.{i}"));
        }
        // …but the account has had enough, from any address it is not
        // known at.
        assert_eq!(rl.begin_at(t0, "ana", "10.9.9.9").unwrap_err(), Duration::from_secs(600));
        assert!(rl.begin_at(t0, "bia", "10.9.9.9").is_ok());
        assert!(rl.begin_at(t0 + Duration::from_secs(601), "ana", "10.9.9.9").is_ok());
    }

    #[test]
    fn one_password_tried_on_many_accounts_hits_the_address_ceiling() {
        let rl = LoginRateLimiter::with_policy(policy());
        let t0 = std::time::Instant::now();
        for i in 0..8 {
            fail(&rl, t0, &format!("user{i}"), "10.0.0.1");
        }
        assert_eq!(rl.begin_at(t0, "user99", "10.0.0.1").unwrap_err(), Duration::from_secs(60));
        assert!(rl.begin_token_at(t0, "10.0.0.1").is_ok(), "the token form shares no counter with it");
        // Everyone else signs in normally: this is the point of the change.
        assert!(rl.begin_at(t0, "ana", "10.0.0.2").is_ok());
        assert!(rl.begin_at(t0 + Duration::from_secs(61), "user99", "10.0.0.1").is_ok());
    }

    /// Someone failing on purpose cannot keep out a person signing in
    /// from where they have signed in before — neither by filling that
    /// person's account, nor the address, nor the whole portal.
    #[test]
    fn a_known_pair_is_not_held_hostage_by_the_collective_ceilings() {
        let rl = LoginRateLimiter::with_policy(policy());
        let t0 = std::time::Instant::now();
        let ok = rl.begin_at(t0, "ana", "home").unwrap();
        rl.success_at(t0, ok);

        // An attacker fills ana's account ceiling from six addresses, the
        // global ceiling from many more, and the address ceiling of a
        // shared address ana also uses but never signed in from.
        for i in 0..6 {
            fail(&rl, t0, "ana", &format!("evil{i}"));
        }
        for i in 0..44 {
            fail(&rl, t0, &format!("x{i}"), &format!("bot{i}"));
        }
        assert!(rl.begin_at(t0, "bia", "anywhere").is_err(), "the portal is saturated for strangers");
        assert!(rl.begin_at(t0, "ana", "cafe").is_err(), "and for ana from a place she is not known at");

        // From home she still gets in — and a mistyped password there
        // still costs her only her own streak.
        for _ in 0..3 {
            fail(&rl, t0, "ana", "home");
        }
        assert_eq!(rl.begin_at(t0, "ana", "home").unwrap_err(), Duration::from_secs(1));
        // The exemption is per account: someone else at her address has none.
        assert!(rl.begin_at(t0, "bia", "home").is_err());
        // And it ends.
        let far = t0 + Duration::from_secs(86_400 + 601);
        for i in 0..6 {
            fail(&rl, far, "ana", &format!("evil{i}"));
        }
        assert!(rl.begin_at(far, "ana", "home").is_err());
    }

    #[test]
    fn the_token_form_has_one_low_ceiling_and_is_separate_from_accounts() {
        let rl = LoginRateLimiter::with_policy(policy());
        let t0 = std::time::Instant::now();
        // Four wrong tokens, each from a different address: no address
        // is anywhere near its own ceiling…
        for i in 0..4 {
            let _ = rl.begin_token_at(t0, &format!("10.0.0.{i}")).unwrap();
        }
        // …but the token has had its guesses, from a fifth address too.
        assert_eq!(rl.begin_token_at(t0, "10.0.9.9").unwrap_err(), Duration::from_secs(60));
        // Accounts are not affected by it,
        assert!(rl.begin_at(t0, "ana", "10.0.9.9").is_ok());
        // and a flood on the password form does not close the token form.
        let rl = LoginRateLimiter::with_policy(policy());
        for i in 0..50 {
            fail(&rl, t0, &format!("u{i}"), &format!("a{i}"));
        }
        assert!(rl.begin_at(t0, "ana", "x").is_err(), "password form saturated");
        let ok = rl.begin_token_at(t0, "operator").expect("break-glass still works");
        // The right token clears the token window.
        for i in 0..3 {
            let _ = rl.begin_token_at(t0, &format!("t{i}")).unwrap();
        }
        rl.success_at(t0, ok);
        assert!(rl.begin_token_at(t0, "t9").is_ok());
        assert_eq!(LoginPolicy::default().token_max, 10, "the first version's global ceiling");

        // Strangers failing on purpose close the form to strangers, not
        // to the address the operator used the token from…
        // (one attempt, from "t9", is already in the window)
        for i in 0..3 {
            let _ = rl.begin_token_at(t0, &format!("s{i}")).unwrap();
        }
        assert!(rl.begin_token_at(t0, "another-stranger").is_err());
        assert!(rl.begin_token_at(t0, "operator").is_ok());
        // …which has a window of its own (4 here), not "no limit".
        let mut admitted = 1;
        while rl.begin_token_at(t0, "operator").is_ok() {
            admitted += 1;
        }
        assert_eq!(admitted, 4);
        // Filling the password form's address counter at the operator's
        // own address does not close the token form there either.
        let rl = LoginRateLimiter::with_policy(policy());
        for i in 0..8 {
            fail(&rl, t0, &format!("u{i}"), "office");
        }
        assert!(rl.begin_at(t0, "someone", "office").is_err());
        assert!(rl.begin_token_at(t0, "office").is_ok());
    }

    /// The owner signing in must not hand a guesser at the same address
    /// a clean slate, nor a fresh account window.
    #[test]
    fn a_success_halves_the_streak_and_leaves_the_account_window() {
        let rl = LoginRateLimiter::with_policy(LoginPolicy { account_max: 100, address_max: 100, global_max: 1000, ..policy() });
        let mut now = std::time::Instant::now();
        // A guesser at ana's address runs her streak up to 9 failures.
        let mut failures = 0;
        while failures < 9 {
            match rl.begin_at(now, "ana", "office") {
                Ok(_dropped) => failures += 1,
                Err(wait) => now += wait,
            }
        }
        let before = rl.begin_at(now, "ana", "office").unwrap_err();
        assert_eq!(before, Duration::from_secs(64));
        // ana gets in (after the wait), from the same address.
        now += before;
        let ok = rl.begin_at(now, "ana", "office").unwrap();
        rl.success_at(now, ok);
        // The streak is 9 → 4, not gone: the very next attempt from that
        // address still waits (2 s), and the one after it 4 s.
        assert_eq!(rl.begin_at(now, "ana", "office").unwrap_err(), Duration::from_secs(2));
        now += Duration::from_secs(2);
        fail(&rl, now, "ana", "office");
        assert_eq!(rl.begin_at(now, "ana", "office").unwrap_err(), Duration::from_secs(4));

        // The account window keeps the failures too: with a ceiling of 6,
        // five failures then a success leave room for exactly one more.
        let rl = LoginRateLimiter::with_policy(policy());
        let t0 = std::time::Instant::now();
        for i in 0..5 {
            fail(&rl, t0, "ana", &format!("evil{i}"));
        }
        let ok = rl.begin_at(t0, "ana", "home").unwrap();
        rl.success_at(t0, ok);
        fail(&rl, t0, "ana", "evil5");
        assert!(rl.begin_at(t0, "ana", "evil6").is_err(), "her success did not reset the count");
    }

    /// An attempt that could not be judged (the user store failed) is
    /// nobody's wrong password.
    #[test]
    fn a_cancelled_attempt_is_uncounted() {
        let rl = LoginRateLimiter::with_policy(policy());
        let t0 = std::time::Instant::now();
        for _ in 0..20 {
            let attempt = rl.begin_at(t0, "ana", "a").expect("never throttled by cancelled attempts");
            rl.cancel(attempt);
        }
        // Three real failures still behave as three.
        for _ in 0..3 {
            fail(&rl, t0, "ana", "a");
        }
        assert_eq!(rl.begin_at(t0, "ana", "a").unwrap_err(), Duration::from_secs(1));
    }

    /// One account cannot fill the table of known pairs.
    #[test]
    fn known_addresses_are_capped_per_account() {
        let rl = LoginRateLimiter::with_policy(policy());
        let t0 = std::time::Instant::now();
        for i in 0..(KNOWN_PER_ACCOUNT + 25) {
            let at = t0 + Duration::from_millis(i as u64);
            let ok = rl.begin_at(at, "mallory", &format!("addr{i}")).unwrap();
            rl.success_at(at, ok);
        }
        let st = rl.lock();
        assert_eq!(st.known.len(), KNOWN_PER_ACCOUNT);
        // The oldest went out, the newest stayed.
        assert!(!st.known.contains_key(&("mallory".to_string(), "addr0".to_string())));
        assert!(st.known.contains_key(&("mallory".to_string(), format!("addr{}", KNOWN_PER_ACCOUNT + 24))));
    }

    /// Fail closed: when a map is at its backstop size and cannot take
    /// this attempt's key, the attempt is refused — never admitted
    /// uncounted. (A known pair's only limit is its streak; admitted
    /// without one it would have no limit at all.)
    #[test]
    fn an_attempt_that_cannot_be_counted_is_refused() {
        let rl = LoginRateLimiter::with_policy(LoginPolicy { account_max: 2, ..policy() });
        let t0 = std::time::Instant::now();
        let ok = rl.begin_at(t0, "ana", "home").unwrap();
        rl.success_at(t0, ok);
        {
            // Fill the streak map with other pairs, up to the backstop.
            let mut st = rl.lock();
            for i in 0..LIMITER_MAX_KEYS {
                st.streaks.insert((format!("x{i}"), "elsewhere".into()), Streak { failures: 1, last: t0 });
            }
        }
        // ana at home is known, but her streak cannot be recorded.
        let admitted = (0..10).filter(|_| rl.begin_at(t0, "ana", "home").is_ok()).count();
        assert_eq!(admitted, 0);
        // Pairs already in the map are unaffected.
        assert!(rl.begin_at(t0, "x1", "elsewhere").is_ok());
    }

    #[test]
    fn keys_are_bounded_and_a_zero_ceiling_refuses_without_panicking() {
        assert_eq!(account_key("   "), None);
        assert_eq!(account_key(&"x".repeat(5000)).unwrap().chars().count(), LIMITER_KEY_MAX_CHARS);
        let rl = LoginRateLimiter::with_policy(LoginPolicy { global_max: 0, ..policy() });
        assert_eq!(rl.begin_at(std::time::Instant::now(), "ana", "a").unwrap_err(), Duration::from_secs(60));
        // The production policy: the working limits are per account and
        // per address; the global one is far above them, and a streak
        // outlives the longest wait.
        let p = LoginPolicy::default();
        assert!(p.global_max >= 10 * p.address_max && p.backoff_after >= 3 && p.streak_forget > p.backoff_cap);
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
