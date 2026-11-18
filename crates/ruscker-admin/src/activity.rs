//! User-activity capture (#1021).
//!
//! Records who logged in and who opened which app — one durable row per
//! event, with identity. This is distinct from [`crate::access_counter`]
//! (aggregate per-spec totals, no user) and the `audit_log` (administrative
//! actions). It powers the "Atividades dos usuários" table.
//!
//! Design mirrors [`crate::alerts`]: emitting is a cheap synchronous
//! [`ActivitySink::record`] on the hot path — a `try_send` into a bounded
//! channel that never blocks and never fails the caller. One supervised task
//! per process ([`spawn`]) drains the channel in batches and does the slow
//! part (a single transaction per batch). If the channel is full (the DB is
//! down and the drain has stalled), the overflow is counted in
//! [`ActivitySink::dropped`] rather than applying backpressure to the proxy —
//! activity logging is best-effort observability, never a request dependency.
//!
//! Persistence + querying live in [`crate::db::user_activity`]. This slice
//! (#1021 fatia 1) delivers the schema, the store, and this writer; the
//! capture sites (login, app access) and the admin UI are later slices.

use crate::db::{self, ConfigDb};
use chrono::{DateTime, Utc};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;
use tokio::sync::mpsc;

/// Queue depth between emit sites and the drain task. App accesses can
/// burst (many users opening apps at once), so this is deeper than the
/// alert queue; a full queue means the DB is down and the drain has
/// stalled — dropping the overflow (counted) is the right call.
const QUEUE_DEPTH: usize = 1024;

/// Rows drained and written per transaction. Bounds the batch a single
/// commit holds while still collapsing a burst into few writes.
const BATCH_MAX: usize = 256;

/// Attempts to persist one batch before giving up on it (first try +
/// retries), with the pause growing per attempt.
const MAX_ATTEMPTS: u32 = 3;
const RETRY_BASE: Duration = Duration::from_millis(200);

/// The kind of activity recorded. The string form is what lands in
/// `user_activity.event_type` and what the UI filters on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityEventType {
    /// A user authenticated successfully (a session was created).
    LoginSuccess,
    /// A new interactive app session was established.
    AppAccess,
    /// A user signed in to an EXTERNAL system through the OIDC provider
    /// (a token was issued to that system's client).
    SsoLogin,
    /// A password sign-in that was refused.
    LoginFailed,
    /// An SSO sign-in the provider refused after identifying the client
    /// (the user is not in its groups, or the session has no account).
    SsoDenied,
    /// A marker, not an action: the budget for a class of failure
    /// events was reached, so further ones in that minute were not
    /// recorded. Without it a gap in the audit would look like silence.
    AuditSuppressed,
    /// The same marker for SSO refusals. Its own type, in the `sso.`
    /// family, so it shows up on the SSO "Access" tab — the place an
    /// Admin looks at refusals.
    SsoSuppressed,
}

impl ActivityEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            ActivityEventType::LoginSuccess => "login.success",
            ActivityEventType::AppAccess => "app.access",
            ActivityEventType::SsoLogin => "sso.login",
            ActivityEventType::LoginFailed => "login.failed",
            ActivityEventType::SsoDenied => "sso.denied",
            ActivityEventType::AuditSuppressed => "audit.suppressed",
            ActivityEventType::SsoSuppressed => "sso.suppressed",
        }
    }
}

/// How the actor authenticated. `None` when not applicable (e.g. an
/// anonymous app access).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthMethod {
    /// Username + password.
    Password,
    /// `RUSCKER_ADMIN_TOKEN` break-glass session.
    Token,
}

impl AuthMethod {
    pub fn as_str(self) -> &'static str {
        match self {
            AuthMethod::Password => "password",
            AuthMethod::Token => "token",
        }
    }
}

/// One activity event on its way to the store.
#[derive(Debug, Clone)]
pub struct ActivityEvent {
    pub event_type: ActivityEventType,
    /// Signed-in username; `None` for an anonymous access.
    pub username: Option<String>,
    /// Target spec for `AppAccess`; `None` for `LoginSuccess`.
    pub spec_id: Option<String>,
    /// Session identifier — the dedup key (unique per `event_type`), so the
    /// same session logged by two HA instances collapses to one row.
    pub event_key: String,
    pub auth_method: Option<AuthMethod>,
    /// Client IP if the deployment chooses to record it. Personal data —
    /// admin-only, retention TBD (kept optional and unset by default).
    pub client_ip: Option<String>,
    /// The browser's `User-Agent`, sanitized and truncated. Personal data,
    /// same handling as `client_ip`.
    pub user_agent: Option<String>,
    /// Machine-readable reason for a failure or refusal
    /// (`wrong-password`, `not-in-group`, …). `None` otherwise.
    pub detail: Option<String>,
    /// When the event happened (stamped at emit, not at insert, so a
    /// drained-late batch keeps accurate timestamps).
    pub occurred_at: DateTime<Utc>,
}

/// Where a request came from: the client address and browser, as far as
/// the request tells. Both are optional and both are personal data.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClientInfo {
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

/// Longest `User-Agent` kept. Real ones are 100–200 characters; the cap
/// stops a hostile client from using the audit table as storage.
const MAX_USER_AGENT_LEN: usize = 256;

impl ClientInfo {
    /// Read the client address and browser from a request.
    ///
    /// The address follows the same trust rule as rate limiting: the
    /// right-most `X-Forwarded-For` entry when the operator opted into
    /// forwarded headers (Ruscker behind its reverse proxy), the TCP
    /// peer otherwise — never a value the client can simply invent.
    pub fn from_request(
        state: &crate::AppState,
        headers: &axum::http::HeaderMap,
        peer: Option<&std::net::SocketAddr>,
    ) -> Self {
        let user_agent = headers
            .get(axum::http::header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(|ua| {
                ua.chars()
                    .filter(|c| !c.is_control())
                    .take(MAX_USER_AGENT_LEN)
                    .collect::<String>()
            })
            .filter(|ua| !ua.trim().is_empty());
        Self {
            ip: crate::routes::proxy::client_ip(state, headers, peer),
            user_agent,
        }
    }
}

impl ActivityEvent {
    /// Attach where the request came from.
    pub fn with_client(mut self, client: ClientInfo) -> Self {
        self.client_ip = client.ip;
        self.user_agent = client.user_agent;
        self
    }

    /// A refused password sign-in. `username` is set ONLY when it names
    /// an existing account: an unknown name is not stored, because people
    /// do type passwords into the wrong field, and an audit table must
    /// not end up holding them.
    pub fn login_failed(username: Option<String>, reason: &str) -> Self {
        Self {
            event_type: ActivityEventType::LoginFailed,
            username,
            spec_id: None,
            // Every attempt is its own event; nothing to deduplicate.
            event_key: uuid::Uuid::new_v4().to_string(),
            auth_method: Some(AuthMethod::Password),
            client_ip: None,
            user_agent: None,
            detail: Some(reason.to_string()),
            occurred_at: Utc::now(),
        }
    }

    /// An SSO sign-in refused for `client_id`, with the reason.
    pub fn sso_denied(username: Option<String>, client_id: impl Into<String>, reason: &str) -> Self {
        Self {
            event_type: ActivityEventType::SsoDenied,
            username,
            spec_id: Some(client_id.into()),
            event_key: uuid::Uuid::new_v4().to_string(),
            auth_method: None,
            client_ip: None,
            user_agent: None,
            detail: Some(reason.to_string()),
            occurred_at: Utc::now(),
        }
    }

    /// "The budget for `class` was reached" — see
    /// [`ActivityEventType::AuditSuppressed`].
    fn audit_suppressed(class: &str) -> Self {
        Self {
            event_type: if class.starts_with("sso.") {
                ActivityEventType::SsoSuppressed
            } else {
                ActivityEventType::AuditSuppressed
            },
            username: None,
            spec_id: None,
            event_key: uuid::Uuid::new_v4().to_string(),
            auth_method: None,
            client_ip: None,
            user_agent: None,
            detail: Some(class.to_string()),
            occurred_at: Utc::now(),
        }
    }

    /// A successful login.
    pub fn login_success(
        username: impl Into<String>,
        auth_method: AuthMethod,
        event_key: impl Into<String>,
        client_ip: Option<String>,
    ) -> Self {
        Self {
            event_type: ActivityEventType::LoginSuccess,
            username: Some(username.into()),
            spec_id: None,
            event_key: event_key.into(),
            auth_method: Some(auth_method),
            client_ip,
            user_agent: None,
            detail: None,
            occurred_at: Utc::now(),
        }
    }

    /// A sign-in to an external system through the OIDC provider. The
    /// client id rides in `spec_id` (the "which system" column); `event_key`
    /// must be unique per sign-in.
    pub fn sso_login(
        username: impl Into<String>,
        client_id: impl Into<String>,
        event_key: impl Into<String>,
    ) -> Self {
        Self {
            event_type: ActivityEventType::SsoLogin,
            username: Some(username.into()),
            spec_id: Some(client_id.into()),
            event_key: event_key.into(),
            auth_method: None,
            client_ip: None,
            user_agent: None,
            detail: None,
            occurred_at: Utc::now(),
        }
    }

    /// A new interactive app session. `username` is `None` for an
    /// anonymous access.
    pub fn app_access(
        username: Option<String>,
        spec_id: impl Into<String>,
        event_key: impl Into<String>,
        auth_method: Option<AuthMethod>,
        client_ip: Option<String>,
    ) -> Self {
        Self {
            event_type: ActivityEventType::AppAccess,
            username,
            spec_id: Some(spec_id.into()),
            event_key: event_key.into(),
            auth_method,
            client_ip,
            user_agent: None,
            detail: None,
            occurred_at: Utc::now(),
        }
    }
}

/// Cheap-to-clone handle held in [`crate::AppState`]. Emitting never blocks
/// and never fails the caller; the receiving half is taken once by
/// [`spawn`] to start the drain task (tests take it to assert on events).
#[derive(Debug, Clone)]
pub struct ActivitySink {
    tx: mpsc::Sender<ActivityEvent>,
    rx: Arc<Mutex<Option<mpsc::Receiver<ActivityEvent>>>>,
    dropped: Arc<AtomicU64>,
    /// Per-key one-minute budgets for [`ActivitySink::record_limited`]:
    /// key → (window start, events in the window).
    budgets: Arc<Mutex<std::collections::HashMap<String, (std::time::Instant, u32)>>>,
    suppressed: Arc<AtomicU64>,
}

impl Default for ActivitySink {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel(QUEUE_DEPTH);
        Self {
            tx,
            rx: Arc::new(Mutex::new(Some(rx))),
            dropped: Arc::new(AtomicU64::new(0)),
            budgets: Arc::new(Mutex::new(std::collections::HashMap::new())),
            suppressed: Arc::new(AtomicU64::new(0)),
        }
    }
}

impl ActivitySink {
    /// Record one event. Hot path: a non-blocking `try_send`. If the queue
    /// is full (drain stalled / DB down) the event is dropped and counted —
    /// never blocks the caller.
    pub fn record(&self, event: ActivityEvent) {
        if self.tx.try_send(event).is_err() {
            self.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record an event that an outsider can cause at will — a refused
    /// sign-in, a refused SSO request — under a budget, so it can neither
    /// fill the table nor crowd legitimate events out of the queue.
    ///
    /// At most `per_minute` events are kept per `key` in any one-minute
    /// window, and at most [`FAILURE_BUDGET_PER_MINUTE`] per `class`.
    /// Each class has its OWN ceiling: anonymous traffic hammering the
    /// login must not be able to silence the record of SSO refusals,
    /// which need a session to produce. When a class reaches its
    /// ceiling, ONE marker row (`audit.suppressed`) is written for that
    /// minute, so the gap is visible in the audit instead of looking
    /// like nothing happened; the count is in
    /// [`ActivitySink::suppressed`].
    pub fn record_limited(&self, class: &'static str, key: &str, per_minute: u32, event: ActivityEvent) {
        let now = std::time::Instant::now();
        let window = Duration::from_secs(60);
        let class_key = format!("\u{0}class|{class}");
        let marker_key = format!("\u{0}marker|{class}");
        let (admitted, mark) = {
            let mut budgets = self.budgets.lock().unwrap_or_else(PoisonError::into_inner);
            // Forget finished windows before the map can grow large.
            if budgets.len() > BUDGET_KEYS_MAX {
                budgets.retain(|_, (start, _)| now.duration_since(*start) < window);
                if budgets.len() > BUDGET_KEYS_MAX {
                    budgets.clear();
                }
            }
            // Is there room under `limit` for `k` in the current window?
            // (A finished window counts as empty.)
            let room = |budgets: &std::collections::HashMap<String, (std::time::Instant, u32)>,
                        k: &str,
                        limit: u32| match budgets.get(k) {
                Some((start, used)) if now.duration_since(*start) < window => *used < limit,
                _ => true,
            };
            // Decide first (reads only), then write.
            let class_has_room = room(&budgets, &class_key, FAILURE_BUDGET_PER_MINUTE);
            // Consume ONLY when both budgets have room: an event already
            // refused by its own key must not use up the class budget
            // (one noisy address would otherwise silence everyone else).
            let ok = class_has_room && room(&budgets, key, per_minute);
            // The class ceiling itself was hit: leave one marker a minute.
            let mark = !class_has_room && room(&budgets, &marker_key, 1);
            let mut consume = |k: &str| {
                let slot = budgets.entry(k.to_string()).or_insert((now, 0));
                if now.duration_since(slot.0) >= window {
                    *slot = (now, 0);
                }
                slot.1 += 1;
            };
            if ok {
                consume(&class_key);
                consume(key);
            }
            if mark {
                consume(&marker_key);
            }
            (ok, mark)
        };
        if admitted {
            self.record(event);
        } else {
            self.suppressed.fetch_add(1, Ordering::Relaxed);
            if mark {
                tracing::warn!(class, "audit budget reached; further events this minute are not recorded");
                self.record(ActivityEvent::audit_suppressed(class));
            }
        }
    }

    /// Failure-class events dropped by [`ActivitySink::record_limited`]
    /// since boot.
    pub fn suppressed(&self) -> u64 {
        self.suppressed.load(Ordering::Relaxed)
    }

    /// Take the receiving half — called once by [`spawn`]. Returns `None`
    /// on a second call (the task is already running, or a test took it).
    pub fn take_receiver(&self) -> Option<mpsc::Receiver<ActivityEvent>> {
        self.rx
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }

    /// Events dropped to the [`QUEUE_DEPTH`] backstop since boot —
    /// observability for the overflow case.
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

/// Start the single per-process drain task: batch-reads events and writes
/// each batch in one transaction, retrying a failed batch a few times
/// before dropping it (logged). Returns `None` if the receiver was already
/// taken. The task ends when every [`ActivitySink`] clone is dropped.
pub fn spawn(sink: &ActivitySink, db: ConfigDb) -> Option<tokio::task::JoinHandle<()>> {
    let mut rx = sink.take_receiver()?;
    Some(tokio::spawn(async move {
        let mut buf: Vec<ActivityEvent> = Vec::with_capacity(BATCH_MAX);
        // `recv_many` appends up to BATCH_MAX and returns 0 only when the
        // channel is closed and drained — i.e. all senders are gone.
        while rx.recv_many(&mut buf, BATCH_MAX).await > 0 {
            let mut attempt = 0u32;
            loop {
                match db::user_activity::insert_batch(&db, &buf).await {
                    Ok(_) => break,
                    Err(e) => {
                        attempt += 1;
                        if attempt >= MAX_ATTEMPTS {
                            tracing::warn!(
                                error = ?e, lost = buf.len(),
                                "activity batch insert failed; events lost"
                            );
                            break;
                        }
                        tracing::warn!(error = ?e, attempt, "activity batch insert failed; retrying");
                        tokio::time::sleep(RETRY_BASE * attempt).await;
                    }
                }
            }
            buf.clear();
        }
    }))
}

/// Ceiling on each class of failure events (refused sign-ins; refused
/// SSO requests) recorded per minute across the whole process. Far
/// above anything honest, far below the queue depth.
pub const FAILURE_BUDGET_PER_MINUTE: u32 = 240;

/// The budget bucket for a client address: the address itself for IPv4,
/// the /64 for IPv6 — one subscriber owns a whole /64 and could
/// otherwise present a fresh address per request.
pub fn address_bucket(ip: Option<&str>) -> String {
    // Canonical first: an IPv4 client seen as `::ffff:a.b.c.d` (a
    // dual-stack listener, or a proxy that reports mapped addresses) is
    // an IPv4 client — otherwise every one of them would share the single
    // bucket `0:0:0:0::/64`.
    match ip.and_then(|ip| ip.parse::<std::net::IpAddr>().ok()).map(|ip| ip.to_canonical()) {
        Some(std::net::IpAddr::V6(v6)) => {
            let s = v6.segments();
            format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
        }
        Some(std::net::IpAddr::V4(v4)) => v4.to_string(),
        None => "-".to_string(),
    }
}
/// How often the throttle's list of known sign-ins is refreshed from
/// the audit log.
const KNOWN_SIGN_INS_INTERVAL: Duration = Duration::from_secs(5 * 60);

/// Bring the sign-in throttle's known pairs in step with the database
/// for everything since `since`: first forget the accounts whose
/// password changed or that were deleted, then learn the correct
/// password sign-ins. Returns how many sign-ins were read.
///
/// The throttle exempts people it knows from its collective ceilings
/// (see `auth::LoginRateLimiter`), but it only knows what it has seen —
/// and it forgets at every restart, which is every upgrade. The audit
/// log already has each correct sign-in with the account and the
/// address, so that is where the list is rebuilt from. Forgetting comes
/// first so that a sign-in made AFTER a password change in the same
/// period is kept (the read leaves out those made before it).
pub async fn load_known_sign_ins(
    db: &ConfigDb,
    limiter: &crate::auth::LoginRateLimiter,
    since: chrono::DateTime<Utc>,
) -> anyhow::Result<usize> {
    for account in db::user_activity::accounts_to_forget_since(db, since).await? {
        limiter.forget_account(&account);
    }
    let (rows, cut) = db::user_activity::password_sign_ins_since(db, since).await?;
    if cut {
        tracing::warn!(
            rows = rows.len(),
            "sign-in throttle: more sign-ins in the period than are read; the oldest were left out"
        );
    }
    let now = Utc::now();
    for (username, ip, at) in &rows {
        // A clock that went backwards reads as "just now".
        let age = (now - *at).to_std().unwrap_or_default();
        limiter.remember_sign_in(username, &address_bucket(Some(ip)), age);
    }
    Ok(rows.len())
}

/// Start the task that keeps the throttle's known sign-ins in step with
/// the database: everything within the trust period at start-up, then
/// what is new every few minutes — which is also how one node of an HA
/// deployment learns about sign-ins, and password changes, made on
/// another (within a few minutes, and assuming the nodes' clocks agree
/// to within the overlap below).
pub fn spawn_known_sign_ins(
    db: ConfigDb,
    limiter: Arc<crate::auth::LoginRateLimiter>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let trust = chrono::Duration::from_std(limiter.trust_for()).unwrap_or(chrono::Duration::days(30));
        let mut since = Utc::now() - trust;
        let mut first = true;
        loop {
            let started = Utc::now();
            match load_known_sign_ins(&db, &limiter, since).await {
                Ok(n) => {
                    if first {
                        tracing::info!(sign_ins = n, "sign-in throttle: known sign-ins loaded from the audit log");
                    }
                    // Overlap a little: rows are written in batches, a
                    // moment after they happen.
                    since = started - chrono::Duration::minutes(2);
                    first = false;
                }
                // `since` is kept: the next pass covers this one too.
                Err(e) => tracing::warn!(error = ?e, "sign-in throttle: could not read known sign-ins"),
            }
            tokio::time::sleep(KNOWN_SIGN_INS_INTERVAL).await;
        }
    })
}

/// Budget map size at which finished windows are swept.
const BUDGET_KEYS_MAX: usize = 4096;

/// How often the retention task looks for rows to purge.
const RETENTION_INTERVAL: Duration = Duration::from_secs(6 * 60 * 60);

/// Start the retention task: delete `user_activity` rows older than
/// `retention_days`, once at startup and then every six hours. Returns
/// `None` (no task) when retention is disabled with `0`.
///
/// The rows hold personal data (who signed in, from which address and
/// browser); keeping them forever is neither needed for an audit nor
/// defensible. Every node may run this — the delete is idempotent.
pub fn spawn_retention(db: ConfigDb, retention_days: u32) -> Option<tokio::task::JoinHandle<()>> {
    if retention_days == 0 {
        return None;
    }
    Some(tokio::spawn(async move {
        loop {
            purge_expired(&db, retention_days).await;
            tokio::time::sleep(RETENTION_INTERVAL).await;
        }
    }))
}

/// One retention pass: delete what is older than `retention_days`.
/// Never panics — an absurdly large value (a date before the calendar
/// begins) simply purges nothing.
async fn purge_expired(db: &ConfigDb, retention_days: u32) {
    let Some(cutoff) = chrono::Duration::try_days(i64::from(retention_days))
        .and_then(|age| Utc::now().checked_sub_signed(age))
    else {
        tracing::warn!(retention_days, "activity retention is out of range; nothing purged");
        return;
    };
    match db::user_activity::purge_older_than(db, cutoff).await {
        Ok(0) => {}
        Ok(n) => tracing::info!(rows = n, retention_days, "purged old user activity"),
        Err(e) => tracing::warn!(error = ?e, "user activity retention purge failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Whatever an outsider can trigger at will is budgeted — per key
    /// and per class — and a class reaching its ceiling leaves a marker
    /// instead of a silent gap. One class cannot starve another.
    #[test]
    fn failure_events_are_budgeted_per_key_and_per_class() {
        let sink = ActivitySink::default();
        let mut rx = sink.take_receiver().unwrap();
        let fail = || ActivityEvent::login_failed(None, "unknown-account");
        let drain = |rx: &mut mpsc::Receiver<ActivityEvent>| {
            let mut kinds = Vec::new();
            while let Ok(ev) = rx.try_recv() {
                kinds.push(ev.event_type);
            }
            kinds
        };

        // One address hammering: only its per-minute budget gets through,
        // and that is NOT the class ceiling, so no marker.
        for _ in 0..500 {
            sink.record_limited("login.failed", "login.failed|203.0.113.9", 10, fail());
        }
        assert_eq!(drain(&mut rx), vec![ActivityEventType::LoginFailed; 10]);
        assert_eq!(sink.suppressed(), 490);

        // Many addresses: the class ceiling holds (10 already used) and
        // exactly one marker says the log has a gap.
        for i in 0..1000 {
            sink.record_limited("login.failed", &format!("login.failed|198.51.100.{i}"), 10, fail());
        }
        let kinds = drain(&mut rx);
        let failed = kinds.iter().filter(|k| **k == ActivityEventType::LoginFailed).count() as u32;
        let markers = kinds.iter().filter(|k| **k == ActivityEventType::AuditSuppressed).count();
        assert_eq!(failed, FAILURE_BUDGET_PER_MINUTE - 10);
        assert_eq!(markers, 1, "one marker per class per minute");

        // The login class is spent; the SSO class is untouched by it.
        sink.record_limited(
            "sso.denied",
            "sso.denied|bia|radar|not-in-group",
            1,
            ActivityEvent::sso_denied(Some("bia".into()), "radar", "not-in-group"),
        );
        assert_eq!(drain(&mut rx), vec![ActivityEventType::SsoDenied]);
        // When the SSO class itself overflows, its marker is an `sso.`
        // event (so the SSO tab shows it), again once.
        for i in 0..400 {
            sink.record_limited(
                "sso.denied",
                &format!("sso.denied|user{i}|radar|not-in-group"),
                1,
                ActivityEvent::sso_denied(Some(format!("user{i}")), "radar", "not-in-group"),
            );
        }
        let kinds = drain(&mut rx);
        assert_eq!(kinds.iter().filter(|k| **k == ActivityEventType::SsoSuppressed).count(), 1);
        assert_eq!(kinds.iter().filter(|k| **k == ActivityEventType::AuditSuppressed).count(), 0);

        // A real sign-in is not a failure-class event and still goes in.
        sink.record(ActivityEvent::login_success("alice", AuthMethod::Password, "k", None));
        assert_eq!(drain(&mut rx), vec![ActivityEventType::LoginSuccess]);
    }

    #[test]
    fn address_bucket_groups_ipv6_by_64() {
        assert_eq!(address_bucket(Some("203.0.113.9")), "203.0.113.9");
        assert_eq!(
            address_bucket(Some("2001:db8:1:2:aaaa:bbbb:cccc:dddd")),
            address_bucket(Some("2001:db8:1:2::1"))
        );
        assert_ne!(address_bucket(Some("2001:db8:1:2::1")), address_bucket(Some("2001:db8:1:3::1")));
        // IPv4-mapped IPv6 is the IPv4 address, not a shared /64.
        assert_eq!(address_bucket(Some("::ffff:203.0.113.9")), "203.0.113.9");
        assert_ne!(address_bucket(Some("::ffff:203.0.113.9")), address_bucket(Some("::ffff:203.0.113.10")));
        assert_eq!(address_bucket(None), "-");
        assert_eq!(address_bucket(Some("not-an-ip")), "-");
    }

    #[tokio::test]
    async fn retention_pass_purges_and_survives_absurd_values() {
        let db = mem_db().await;
        let mut old = ActivityEvent::login_success("alice", AuthMethod::Password, "old", None);
        old.occurred_at = Utc::now() - chrono::Duration::days(400);
        db::user_activity::insert_batch(
            &db,
            &[old, ActivityEvent::login_success("alice", AuthMethod::Password, "new", None)],
        )
        .await
        .unwrap();
        let count = || async {
            db::user_activity::count_filtered(&db, &db::user_activity::ActivityFilter::default())
                .await
                .unwrap()
        };
        // Out of range: no panic, nothing deleted.
        purge_expired(&db, u32::MAX).await;
        assert_eq!(count().await, 2);
        purge_expired(&db, 180).await;
        assert_eq!(count().await, 1);
        // `0` means "keep everything": no task at all.
        assert!(spawn_retention(db.clone(), 0).is_none());
    }

    async fn mem_db() -> ConfigDb {
        ConfigDb::Sqlite(crate::db::open_memory().await.expect("open in-memory"))
    }

    #[test]
    fn record_full_queue_counts_dropped_never_blocks() {
        let sink = ActivitySink::default();
        // Don't start the drain, so nothing is consumed. QUEUE_DEPTH fit,
        // the rest are dropped and counted — record never blocks.
        for i in 0..(QUEUE_DEPTH + 50) {
            sink.record(ActivityEvent::login_success(
                "alice",
                AuthMethod::Password,
                format!("sess-{i}"),
                None,
            ));
        }
        assert_eq!(sink.dropped(), 50);
    }

    #[tokio::test]
    async fn drain_persists_recorded_events() {
        let db = mem_db().await;
        let sink = ActivitySink::default();
        let handle = spawn(&sink, db.clone()).expect("drain starts");

        sink.record(ActivityEvent::login_success(
            "alice",
            AuthMethod::Password,
            "login-1",
            Some("10.0.0.1".into()),
        ));
        sink.record(ActivityEvent::app_access(
            Some("alice".into()),
            "sales",
            "app-sess-1",
            Some(AuthMethod::Password),
            None,
        ));
        // Anonymous access — username None.
        sink.record(ActivityEvent::app_access(
            None,
            "public-dash",
            "app-sess-2",
            None,
            None,
        ));

        // Dropping every sink clone closes the channel; the drain then
        // finishes its final batch and the task returns.
        drop(sink);
        handle.await.expect("drain task joins");

        let filter = db::user_activity::ActivityFilter {
            limit: 100,
            ..Default::default()
        };
        let rows = db::user_activity::list_page(&db, &filter).await.unwrap();
        assert_eq!(rows.len(), 3);
        // Newest first: the two app accesses then the login (insertion order).
        assert_eq!(
            db::user_activity::count_filtered(&db, &filter)
                .await
                .unwrap(),
            3
        );
        let logins: Vec<_> = rows
            .iter()
            .filter(|r| r.event_type == "login.success")
            .collect();
        assert_eq!(logins.len(), 1);
        assert_eq!(logins[0].username.as_deref(), Some("alice"));
        assert_eq!(logins[0].client_ip.as_deref(), Some("10.0.0.1"));
        let anon: Vec<_> = rows
            .iter()
            .filter(|r| r.spec_id.as_deref() == Some("public-dash"))
            .collect();
        assert_eq!(anon.len(), 1);
        assert_eq!(anon[0].username, None, "anonymous access has no username");
    }

    #[tokio::test]
    async fn duplicate_event_key_is_deduped() {
        let db = mem_db().await;
        // Same (event_type, event_key) inserted twice — the unique index +
        // ON CONFLICT DO NOTHING keeps exactly one (HA cross-instance case).
        let e = ActivityEvent::app_access(
            Some("bob".into()),
            "sales",
            "dup-key",
            Some(AuthMethod::Password),
            None,
        );
        assert_eq!(
            db::user_activity::insert_batch(&db, std::slice::from_ref(&e))
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db::user_activity::insert_batch(&db, &[e]).await.unwrap(),
            0,
            "second insert of the same session event is a no-op"
        );
        let filter = db::user_activity::ActivityFilter {
            limit: 100,
            ..Default::default()
        };
        assert_eq!(
            db::user_activity::count_filtered(&db, &filter)
                .await
                .unwrap(),
            1
        );
    }
}
