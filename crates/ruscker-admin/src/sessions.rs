//! Per-visitor session tracking + idle-expiry sweeper.
//!
//! ## What a session is here
//!
//! A "session" is the binding between a sticky cookie's
//! `session_id` and the replica it was routed to. The lifecycle:
//!
//! 1. **Registered** — first request from a visitor; we mint a
//!    cookie, register the session, and increment the chosen
//!    replica's `sessions_active`.
//! 2. **Touched** — every subsequent request bumps `last_seen`.
//!    No counter change.
//! 3. **Expired** — the sweeper runs every
//!    [`SWEEPER_INTERVAL`] and evicts any session whose
//!    `now - last_seen > heartbeat_timeout`. Eviction
//!    decrements the replica's `sessions_active`.
//!
//! ## Why this exists
//!
//! Until this module, `Replica::sessions_active` was always zero
//! — nothing populated it. That meant `available_seats()` always
//! returned the maximum, the LeastConnections router was
//! degenerate, and the scaler couldn't observe saturation. With
//! accurate counts, both unlock.
//!
//! ## What this is NOT
//!
//! - Not a heartbeat receiver. ShinyProxy listens for explicit
//!   client pings; here, **any HTTP/WS request from the visitor
//!   counts as a heartbeat** because we touch on every routed
//!   request. Good enough for the MVP; explicit pings can be
//!   added if the apps stop generating traffic during idle UI
//!   work.
//! - Not persistent. Restart wipes the tracker. The cookie's
//!   session_id is still valid (HMAC verifies), and the next
//!   request will re-register on the same replica if the
//!   reconcile picked it up. Counter restarts from zero.

use dashmap::DashMap;
use ruscker_core::{ReplicaId, ReplicaRegistry};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::info;
use uuid::Uuid;

/// How often the sweeper checks for expired sessions. Tighter
/// than `heartbeat_timeout` so freed seats become available
/// promptly; coarse enough that the lock-write cost is trivial.
pub const SWEEPER_INTERVAL: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
struct SessionEntry {
    spec_id: String,
    replica_id: ReplicaId,
    last_seen: Instant,
}

/// In-memory tracker. Shared across handlers via `Arc`. The
/// internal map is `DashMap` so per-request `touch_or_register`
/// only locks a single shard, never the whole structure.
///
/// The registry write-lock is taken **only** on register/evict
/// (counter mutation). `touch` for an existing session goes
/// through DashMap alone — read traffic on a warm session never
/// blocks on the registry RwLock.
#[derive(Debug)]
pub struct SessionTracker {
    sessions: DashMap<Uuid, SessionEntry>,
}

/// Outcome of a `touch_or_register` call. Useful for the proxy
/// to know whether it just minted a new session (and therefore
/// should set the sticky cookie on the response) or refreshed
/// an existing one.
#[derive(Debug, PartialEq, Eq)]
pub enum TouchOutcome {
    Registered,
    Touched,
}

impl SessionTracker {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }

    /// Record activity for `session_id`. If unknown, register it
    /// against `(spec_id, replica_id)` and increment the
    /// replica's counter. If known, just refresh `last_seen`.
    ///
    /// Returns whether this was a fresh registration so the
    /// caller can decide cookie-setting policy.
    pub async fn touch_or_register(
        &self,
        registry: &RwLock<ReplicaRegistry>,
        session_id: Uuid,
        spec_id: &str,
        replica_id: &ReplicaId,
    ) -> TouchOutcome {
        // Fast path: the session already exists. Update
        // last_seen without touching the registry lock at all.
        if let Some(mut entry) = self.sessions.get_mut(&session_id) {
            entry.last_seen = Instant::now();
            return TouchOutcome::Touched;
        }
        // Cold path: register. Insert under the entry API to
        // serialize against a concurrent twin caller (the
        // DashMap shard mutex covers us).
        let was_new = match self.sessions.entry(session_id) {
            dashmap::Entry::Occupied(mut occ) => {
                occ.get_mut().last_seen = Instant::now();
                false
            }
            dashmap::Entry::Vacant(vac) => {
                vac.insert(SessionEntry {
                    spec_id: spec_id.to_string(),
                    replica_id: replica_id.clone(),
                    last_seen: Instant::now(),
                });
                true
            }
        };
        if was_new {
            registry.write().await.inc_sessions(replica_id);
            TouchOutcome::Registered
        } else {
            TouchOutcome::Touched
        }
    }

    /// One sweeper pass. Walks the map, evicts entries older
    /// than `timeout`, and decrements the replica counter for
    /// each. Returns the number of sessions evicted (for
    /// metrics / logging).
    ///
    /// `timeout < 0` semantics live one level up — the caller
    /// (loop) decides whether to call this at all. Here we
    /// require an unsigned timeout.
    pub async fn sweep(
        &self,
        registry: &RwLock<ReplicaRegistry>,
        timeout: Duration,
    ) -> usize {
        let now = Instant::now();
        // Collect victims first so we don't hold DashMap shard
        // guards across the registry await.
        let mut victims: Vec<(Uuid, ReplicaId)> = Vec::new();
        for kv in self.sessions.iter() {
            if now.duration_since(kv.value().last_seen) > timeout {
                victims.push((*kv.key(), kv.value().replica_id.clone()));
            }
        }
        if victims.is_empty() {
            return 0;
        }
        let n = victims.len();
        {
            let mut reg = registry.write().await;
            for (sid, rid) in &victims {
                reg.dec_sessions(rid);
                self.sessions.remove(sid);
            }
        }
        n
    }

    /// Number of tracked sessions. For introspection / tests.
    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}

impl Default for SessionTracker {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn the sweeper as a detached tokio task. Returns a handle
/// that callers can drop on shutdown — the loop only does
/// idempotent work, no graceful-stop protocol needed.
///
/// `heartbeat_timeout_ms < 0` (the `-1` ShinyProxy idiom for
/// "never expire") makes this a no-op task that ticks but
/// evicts nothing. Useful so the wiring stays uniform.
pub fn spawn(
    tracker: Arc<SessionTracker>,
    registry: Arc<RwLock<ReplicaRegistry>>,
    heartbeat_timeout_ms: i64,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if heartbeat_timeout_ms < 0 {
            info!("session sweeper started in never-expire mode (heartbeat-timeout=-1)");
            // We still hold the task open so the JoinHandle stays
            // valid for callers; this branch deliberately spins
            // forever doing nothing.
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
            }
        }
        let timeout = Duration::from_millis(heartbeat_timeout_ms as u64);
        info!(?timeout, interval = ?SWEEPER_INTERVAL, "session sweeper started");
        let mut ticker = tokio::time::interval(SWEEPER_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let evicted = tracker.sweep(&registry, timeout).await;
            if evicted > 0 {
                info!(evicted, "session sweeper evicted idle sessions");
            }
            // Quiet on no-op — would spam logs every 5s.
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruscker_core::{Replica, ReplicaId, ReplicaState};
    use std::net::SocketAddr;

    fn fake_replica(spec: &str) -> Replica {
        Replica {
            id: ReplicaId(Uuid::new_v4()),
            spec_id: spec.to_string(),
            container_id: "x".into(),
            upstream: "127.0.0.1:1".parse::<SocketAddr>().unwrap(),
            state: ReplicaState::Ready,
            started_at: chrono::Utc::now(),
            sessions_active: 0,
            sessions_max: 5,
        }
    }

    #[tokio::test]
    async fn first_touch_registers_and_increments() {
        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let r = fake_replica("alpha");
        let rid = r.id.clone();
        reg.write().await.add(r);
        let tracker = SessionTracker::new();

        let sid = Uuid::new_v4();
        let outcome = tracker
            .touch_or_register(&reg, sid, "alpha", &rid)
            .await;
        assert_eq!(outcome, TouchOutcome::Registered);
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 1);
    }

    #[tokio::test]
    async fn second_touch_only_refreshes() {
        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let r = fake_replica("alpha");
        let rid = r.id.clone();
        reg.write().await.add(r);
        let tracker = SessionTracker::new();

        let sid = Uuid::new_v4();
        let _ = tracker.touch_or_register(&reg, sid, "alpha", &rid).await;
        let outcome = tracker.touch_or_register(&reg, sid, "alpha", &rid).await;
        assert_eq!(outcome, TouchOutcome::Touched);
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 1);
        assert_eq!(tracker.len(), 1);
    }

    #[tokio::test]
    async fn sweep_evicts_idle_sessions_and_decrements() {
        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let r = fake_replica("alpha");
        let rid = r.id.clone();
        reg.write().await.add(r);
        let tracker = SessionTracker::new();

        let sid = Uuid::new_v4();
        tracker.touch_or_register(&reg, sid, "alpha", &rid).await;
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 1);

        // Force the entry's last_seen back in time so it expires
        // under a small timeout.
        if let Some(mut e) = tracker.sessions.get_mut(&sid) {
            e.last_seen = Instant::now() - Duration::from_secs(60);
        }

        let evicted = tracker.sweep(&reg, Duration::from_secs(10)).await;
        assert_eq!(evicted, 1);
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 0);
        assert!(tracker.is_empty());
    }

    #[tokio::test]
    async fn sweep_keeps_fresh_sessions() {
        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let r = fake_replica("alpha");
        let rid = r.id.clone();
        reg.write().await.add(r);
        let tracker = SessionTracker::new();

        let sid = Uuid::new_v4();
        tracker.touch_or_register(&reg, sid, "alpha", &rid).await;
        // last_seen is now, sweep with 10s timeout — nothing
        // should evict.
        let evicted = tracker.sweep(&reg, Duration::from_secs(10)).await;
        assert_eq!(evicted, 0);
        assert_eq!(tracker.len(), 1);
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 1);
    }
}
