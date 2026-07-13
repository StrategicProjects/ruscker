//! Alert notification webhooks (#930).
//!
//! Phase 4 shipped the monitoring dashboard but deferred the delivery
//! channel — an operator could *see* an app down, but nothing told
//! them. This module is that channel: interesting events (a spawn
//! failing, a replica's container dying, a spec saturated at
//! `max-replicas`) become a JSON `POST` to one operator-configured
//! webhook URL (admin **System** tab, stored in the `settings` table
//! under [`crate::db::settings::ALERT_WEBHOOK_URL`]).
//!
//! Design mirrors [`crate::access_counter`]: emitting is a cheap
//! synchronous call on the hot path (`try_send` into a bounded
//! channel — never blocks, never breaks the caller), and one
//! supervised task per process does the slow part (HTTP with timeout
//! and retries). A per-`(kind, spec)` cooldown suppresses repeats so a
//! saturated spec alerts once, not once per scaler tick. No webhook
//! configured ⇒ events are read and dropped — emit sites don't check.
//!
//! Payload contract (documented in the book, § admin/System):
//!
//! ```json
//! {
//!   "event": "spawn-failed" | "replica-down" | "saturated" | "test",
//!   "spec": "app-id",
//!   "replica": "replica-id or null",
//!   "message": "human-readable summary",
//!   "occurred_at": "RFC 3339 timestamp",
//!   "ruscker": { "version": "x.y.z" }
//! }
//! ```

use crate::db::{self, ConfigDb};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// Suppression window for repeats of the same `(kind, spec)` alert.
/// A stuck condition (saturation, crash-looping spawn) re-alerts at
/// most this often.
pub const ALERT_COOLDOWN: Duration = Duration::from_secs(15 * 60);

/// Queue depth between emit sites and the sender task. Alerts are
/// rare; a full queue means the webhook endpoint is down and slow —
/// dropping the overflow (logged) is the right call.
const QUEUE_DEPTH: usize = 64;

/// Per-attempt HTTP timeout for the webhook POST.
const POST_TIMEOUT: Duration = Duration::from_secs(5);

/// Delivery attempts per event (first try + retries), with the pause
/// doubling between them (2 s, 4 s).
const POST_ATTEMPTS: u32 = 3;
const RETRY_BASE: Duration = Duration::from_secs(2);

/// How long the sender task trusts a cached webhook URL before
/// re-reading it from the DB — an admin edit takes effect within this.
const URL_CACHE_TTL: Duration = Duration::from_secs(30);

/// What happened. `Test` comes from the admin "send test" button and
/// bypasses the cooldown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AlertKind {
    /// A container failed to start for a spec.
    SpawnFailed,
    /// A replica's container died outside Ruscker's control (the
    /// scaler's liveness pass pruned it).
    ReplicaDown,
    /// A spec is saturated with every replica at `max-replicas` —
    /// visitors are being turned away and Ruscker can't scale further.
    Saturated,
    /// A scheduled job (#986) exited non-zero or could not run.
    JobFailed,
    /// Operator-triggered delivery check.
    Test,
}

impl AlertKind {
    pub fn as_str(self) -> &'static str {
        match self {
            AlertKind::SpawnFailed => "spawn-failed",
            AlertKind::ReplicaDown => "replica-down",
            AlertKind::Saturated => "saturated",
            AlertKind::JobFailed => "job-failed",
            AlertKind::Test => "test",
        }
    }
}

/// One alert on its way to the webhook.
#[derive(Debug, Clone)]
pub struct AlertEvent {
    pub kind: AlertKind,
    /// Spec the event is about (`"-"` for `Test`).
    pub spec: String,
    pub replica: Option<String>,
    pub message: String,
}

/// Cheap-to-clone handle held in [`crate::AppState`]. Emitting never
/// blocks and never fails the caller; the receiving half is taken once
/// by `AdminServer::run` to start the sender task (tests take it to
/// assert on emitted events instead).
#[derive(Debug, Clone)]
pub struct AlertSink {
    tx: mpsc::Sender<AlertEvent>,
    rx: Arc<Mutex<Option<mpsc::Receiver<AlertEvent>>>>,
    /// Last-sent instant per `(kind, spec)` for the cooldown.
    recent: Arc<Mutex<HashMap<(AlertKind, String), Instant>>>,
}

impl Default for AlertSink {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel(QUEUE_DEPTH);
        Self {
            tx,
            rx: Arc::new(Mutex::new(Some(rx))),
            recent: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl AlertSink {
    /// Queue an alert. Applies the per-`(kind, spec)` cooldown (except
    /// for `Test`); drops silently-but-logged when the queue is full.
    /// Returns whether the event was queued — callers normally ignore
    /// it; tests and the admin "send test" handler read it.
    pub fn notify(&self, event: AlertEvent) -> bool {
        if event.kind != AlertKind::Test {
            let key = (event.kind, event.spec.clone());
            let mut recent = self.recent.lock().unwrap_or_else(PoisonError::into_inner);
            let now = Instant::now();
            if let Some(last) = recent.get(&key) {
                if now.duration_since(*last) < ALERT_COOLDOWN {
                    return false;
                }
            }
            recent.insert(key, now);
            // Keep the map bounded: drop entries whose cooldown lapsed.
            recent.retain(|_, t| now.duration_since(*t) < ALERT_COOLDOWN);
        }
        match self.tx.try_send(event) {
            Ok(()) => true,
            Err(e) => {
                tracing::warn!(error = %e, "alert queue full or closed; event dropped");
                false
            }
        }
    }

    /// Take the receiving half (once). `None` on a second call.
    pub fn take_receiver(&self) -> Option<mpsc::Receiver<AlertEvent>> {
        self.rx
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
    }
}

/// Serialize one event to the documented JSON payload.
fn payload(event: &AlertEvent) -> String {
    serde_json::json!({
        "event": event.kind.as_str(),
        "spec": event.spec,
        "replica": event.replica,
        "message": event.message,
        "occurred_at": chrono::Utc::now().to_rfc3339(),
        "ruscker": { "version": crate::APP_VERSION },
    })
    .to_string()
}

/// Deliver one payload: POST with a per-attempt timeout, retrying with
/// a doubling pause. Any 2xx counts as delivered.
async fn deliver(url: &str, body: String) -> anyhow::Result<()> {
    use http_body_util::Full;
    use hyper::body::Bytes;

    // Provider named explicitly: the dependency tree carries both ring
    // and aws-lc-rs, so rustls has no process default to fall back on.
    let https = hyper_rustls::HttpsConnectorBuilder::new()
        .with_provider_and_native_roots(rustls::crypto::ring::default_provider())
        .map_err(|e| anyhow::anyhow!("load native TLS roots: {e}"))?
        .https_or_http()
        .enable_http1()
        .build();
    let client =
        hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
            .build::<_, Full<Bytes>>(https);

    let mut wait = RETRY_BASE;
    let mut last_err = anyhow::anyhow!("no attempt made");
    for attempt in 1..=POST_ATTEMPTS {
        let req = hyper::Request::post(url)
            .header("content-type", "application/json")
            .header("user-agent", concat!("ruscker/", env!("CARGO_PKG_VERSION")))
            .body(Full::new(Bytes::from(body.clone())))?;
        match tokio::time::timeout(POST_TIMEOUT, client.request(req)).await {
            Ok(Ok(resp)) if resp.status().is_success() => return Ok(()),
            Ok(Ok(resp)) => last_err = anyhow::anyhow!("webhook returned {}", resp.status()),
            Ok(Err(e)) => last_err = anyhow::anyhow!("webhook request failed: {e}"),
            Err(_) => last_err = anyhow::anyhow!("webhook timed out after {POST_TIMEOUT:?}"),
        }
        if attempt < POST_ATTEMPTS {
            tokio::time::sleep(wait).await;
            wait *= 2;
        }
    }
    Err(last_err)
}

/// Start the single per-process sender task. Reads the webhook URL
/// from the settings table (cached [`URL_CACHE_TTL`]); an empty or
/// missing URL drops events. Failures after all retries are logged —
/// alerts are best-effort by contract.
pub fn spawn(sink: &AlertSink, db: ConfigDb) -> Option<tokio::task::JoinHandle<()>> {
    let mut rx = sink.take_receiver()?;
    Some(tokio::spawn(async move {
        let mut cached_url: Option<String> = None;
        let mut cached_at = Instant::now() - URL_CACHE_TTL;
        while let Some(event) = rx.recv().await {
            if cached_at.elapsed() >= URL_CACHE_TTL {
                cached_url = match db::settings::get(&db, db::settings::ALERT_WEBHOOK_URL).await
                {
                    Ok(v) => v.filter(|u| !u.trim().is_empty()),
                    Err(e) => {
                        tracing::warn!(error = ?e, "alert webhook URL lookup failed");
                        None
                    }
                };
                cached_at = Instant::now();
            }
            let Some(url) = cached_url.as_deref() else {
                continue; // no webhook configured — drop quietly
            };
            let body = payload(&event);
            match deliver(url, body).await {
                Ok(()) => tracing::info!(
                    event = event.kind.as_str(),
                    spec = %event.spec,
                    "alert webhook delivered"
                ),
                Err(e) => tracing::warn!(
                    event = event.kind.as_str(),
                    spec = %event.spec,
                    error = %e,
                    attempts = POST_ATTEMPTS,
                    "alert webhook delivery failed; event lost"
                ),
            }
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(kind: AlertKind, spec: &str) -> AlertEvent {
        AlertEvent {
            kind,
            spec: spec.to_string(),
            replica: None,
            message: "m".into(),
        }
    }

    #[tokio::test]
    async fn cooldown_suppresses_repeats_but_not_other_keys() {
        let sink = AlertSink::default();
        assert!(sink.notify(ev(AlertKind::Saturated, "a")));
        assert!(!sink.notify(ev(AlertKind::Saturated, "a")), "same key suppressed");
        assert!(sink.notify(ev(AlertKind::Saturated, "b")), "other spec passes");
        assert!(sink.notify(ev(AlertKind::SpawnFailed, "a")), "other kind passes");
        // Test events always pass.
        assert!(sink.notify(ev(AlertKind::Test, "-")));
        assert!(sink.notify(ev(AlertKind::Test, "-")));
    }

    #[tokio::test]
    async fn receiver_taken_once_and_events_arrive() {
        let sink = AlertSink::default();
        let mut rx = sink.take_receiver().expect("first take");
        assert!(sink.take_receiver().is_none(), "second take is None");
        sink.notify(ev(AlertKind::ReplicaDown, "x"));
        let got = rx.recv().await.unwrap();
        assert_eq!(got.kind, AlertKind::ReplicaDown);
        assert_eq!(got.spec, "x");
    }

    #[test]
    fn payload_shape_is_the_documented_contract() {
        let body = payload(&AlertEvent {
            kind: AlertKind::SpawnFailed,
            spec: "my-app".into(),
            replica: Some("r1".into()),
            message: "boom".into(),
        });
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["event"], "spawn-failed");
        assert_eq!(v["spec"], "my-app");
        assert_eq!(v["replica"], "r1");
        assert_eq!(v["message"], "boom");
        assert!(v["occurred_at"].as_str().unwrap().contains('T'));
        assert!(v["ruscker"]["version"].as_str().is_some());
    }

    /// End-to-end: sender task reads the URL from the DB and POSTs the
    /// payload to a local HTTP server; a first-attempt 500 is retried.
    #[tokio::test]
    async fn sender_posts_to_webhook_and_retries() {
        use axum::{extract::State, routing::post, Router};
        use std::sync::atomic::{AtomicU32, Ordering};

        #[derive(Clone, Default)]
        struct Seen {
            hits: Arc<AtomicU32>,
            body: Arc<Mutex<Option<String>>>,
        }
        let seen = Seen::default();
        let app = Router::new()
            .route(
                "/hook",
                post(|State(s): State<Seen>, body: String| async move {
                    let n = s.hits.fetch_add(1, Ordering::SeqCst);
                    if n == 0 {
                        // First attempt fails — the sender must retry.
                        return axum::http::StatusCode::INTERNAL_SERVER_ERROR;
                    }
                    *s.body.lock().unwrap() = Some(body);
                    axum::http::StatusCode::OK
                }),
            )
            .with_state(seen.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let db = ConfigDb::Sqlite(crate::db::open_memory().await.unwrap());
        db::settings::set(
            &db,
            db::settings::ALERT_WEBHOOK_URL,
            &format!("http://{addr}/hook"),
            None,
        )
        .await
        .unwrap();

        let sink = AlertSink::default();
        spawn(&sink, db).expect("sender started");
        assert!(sink.notify(ev(AlertKind::Saturated, "hot-app")));

        // First attempt 500s, retry lands after RETRY_BASE (2 s).
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if seen.body.lock().unwrap().is_some() {
                break;
            }
            assert!(Instant::now() < deadline, "webhook never received the retry");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(seen.hits.load(Ordering::SeqCst) >= 2, "retried after the 500");
        let body = seen.body.lock().unwrap().clone().unwrap();
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(v["event"], "saturated");
        assert_eq!(v["spec"], "hot-app");
    }
}
