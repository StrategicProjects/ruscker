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
}

impl ActivityEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            ActivityEventType::LoginSuccess => "login.success",
            ActivityEventType::AppAccess => "app.access",
            ActivityEventType::SsoLogin => "sso.login",
            ActivityEventType::LoginFailed => "login.failed",
            ActivityEventType::SsoDenied => "sso.denied",
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
    /// window, and at most [`FAILURE_BUDGET_PER_MINUTE`] across all keys.
    /// The rest are counted ([`ActivitySink::suppressed`]) and dropped:
    /// an audit needs to show THAT an address hammered the login, not
    /// one row per attempt.
    pub fn record_limited(&self, key: &str, per_minute: u32, event: ActivityEvent) {
        let now = std::time::Instant::now();
        let window = Duration::from_secs(60);
        let admitted = {
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
            // Consume ONLY when both budgets have room: an event already
            // refused by its own key must not use up the global budget
            // (one noisy address would otherwise silence everyone else).
            let ok = room(&budgets, GLOBAL_BUDGET_KEY, FAILURE_BUDGET_PER_MINUTE)
                && room(&budgets, key, per_minute);
            if ok {
                for k in [GLOBAL_BUDGET_KEY, key] {
                    let slot = budgets.entry(k.to_string()).or_insert((now, 0));
                    if now.duration_since(slot.0) >= window {
                        *slot = (now, 0);
                    }
                    slot.1 += 1;
                }
            }
            ok
        };
        if admitted {
            self.record(event);
        } else {
            self.suppressed.fetch_add(1, Ordering::Relaxed);
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

/// Ceiling on failure-class events (refused sign-ins, refused SSO
/// requests) recorded per minute across the whole process. Far above
/// anything honest, far below the queue depth.
pub const FAILURE_BUDGET_PER_MINUTE: u32 = 240;
const GLOBAL_BUDGET_KEY: &str = "\u{0}all";
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

    /// Whatever an outsider can trigger at will is budgeted: per key and
    /// overall. Ordinary events are never subject to it.
    #[test]
    fn failure_events_are_budgeted_per_key_and_overall() {
        let sink = ActivitySink::default();
        let mut rx = sink.take_receiver().unwrap();
        let fail = || ActivityEvent::login_failed(None, "unknown-account");
        // One address hammering: only its per-minute budget gets through.
        for _ in 0..500 {
            sink.record_limited("login.failed|203.0.113.9", 10, fail());
        }
        let mut kept = 0;
        while rx.try_recv().is_ok() {
            kept += 1;
        }
        assert_eq!(kept, 10);
        assert_eq!(sink.suppressed(), 490);
        // Many addresses: the overall ceiling holds (10 were already used).
        for i in 0..1000 {
            sink.record_limited(&format!("login.failed|198.51.100.{i}"), 10, fail());
        }
        let mut kept = 0;
        while rx.try_recv().is_ok() {
            kept += 1;
        }
        assert_eq!(kept as u32, FAILURE_BUDGET_PER_MINUTE - 10);
        // A real sign-in is not a failure-class event and still goes in.
        sink.record(ActivityEvent::login_success("alice", AuthMethod::Password, "k", None));
        assert!(rx.try_recv().is_ok());
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
