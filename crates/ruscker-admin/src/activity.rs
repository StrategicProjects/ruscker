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
}

impl ActivityEventType {
    pub fn as_str(self) -> &'static str {
        match self {
            ActivityEventType::LoginSuccess => "login.success",
            ActivityEventType::AppAccess => "app.access",
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
    /// When the event happened (stamped at emit, not at insert, so a
    /// drained-late batch keeps accurate timestamps).
    pub occurred_at: DateTime<Utc>,
}

impl ActivityEvent {
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
}

impl Default for ActivitySink {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel(QUEUE_DEPTH);
        Self {
            tx,
            rx: Arc::new(Mutex::new(Some(rx))),
            dropped: Arc::new(AtomicU64::new(0)),
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

#[cfg(test)]
mod tests {
    use super::*;

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
