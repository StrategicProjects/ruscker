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

/// The session-tracking seam. The proxy and the idle sweeper depend on
/// `Arc<dyn SessionStore>`, not a concrete type, so a multi-instance
/// (Postgres-backed) store can drop in for HA later (Phase 7) without
/// touching proxy code. The single-node default is
/// [`InMemorySessionStore`].
///
/// `touch_or_register` and `sweep` take the [`ReplicaRegistry`] because
/// the in-memory store keeps `Replica::sessions_active` as a live
/// counter it mutates here. A future Postgres store would instead
/// derive that count from shared rows — the registry argument stays in
/// the signature so both shapes fit one trait.
#[async_trait::async_trait]
pub trait SessionStore: Send + Sync {
    /// Record activity for `session_id`, registering it against
    /// `(spec_id, replica_id)` (and incrementing the replica counter)
    /// on first sight, else refreshing `last_seen`.
    ///
    /// `seat_reserved` is `true` when the caller already reserved the
    /// replica's seat at pick time (the atomic pick+reserve that closes
    /// the seat over-admission race, #582 part 1). When set, the store
    /// registers the session but must **not** increment the replica
    /// counter again.
    async fn touch_or_register(
        &self,
        registry: &RwLock<ReplicaRegistry>,
        session_id: Uuid,
        spec_id: &str,
        replica_id: &ReplicaId,
        seat_reserved: bool,
    ) -> TouchOutcome;

    /// One idle-expiry pass: evict sessions past their (per-spec)
    /// timeout and decrement the replica counter. Returns how many
    /// were evicted.
    ///
    /// `is_leader` gates the **destructive** part in shared-store (HA)
    /// implementations: only the leader issues the idle-eviction
    /// DELETEs, so M instances don't race identical deletes on the same
    /// rows (#159 C3). The read-only reconcile (cluster-wide counts +
    /// this node's `len()`) always runs, on every node. Single-node
    /// stores ignore the flag — they're always the sole writer.
    async fn sweep(
        &self,
        registry: &RwLock<ReplicaRegistry>,
        global_ms: i64,
        overrides: &std::collections::HashMap<String, i64>,
        is_leader: bool,
    ) -> usize;

    /// Purge every tracked session pointing at `replica_id`. Called when
    /// a replica leaves the registry (operator stop/restart, scaler
    /// scale-down). Without it the session entries linger until the idle
    /// sweep expires them — inflating `len()` for up to one
    /// `heartbeat-timeout`, and **forever** when a spec sets
    /// `heartbeat-timeout: -1` (the never-expire Shiny idiom), which
    /// stalls graceful shutdown until the watchdog `process::exit`.
    /// Returns how many entries were removed.
    ///
    /// The registry counter is intentionally not touched here: the caller
    /// removes the replica (and its `sessions_active`) from the registry
    /// around this call, so there is nothing left to decrement.
    async fn drop_replica(&self, replica_id: &ReplicaId) -> usize;

    /// End one tracked session by id — remove it and decrement its
    /// replica's `sessions_active` (the same bookkeeping the idle sweep
    /// does, but triggered on demand). Used by `stop-on-logout` (#337) to
    /// free a user's session the moment they sign out. Returns `true` if
    /// a session was actually removed (a stale/unknown id is a no-op).
    async fn end_session(
        &self,
        registry: &RwLock<ReplicaRegistry>,
        session_id: Uuid,
    ) -> bool;

    /// Number of tracked sessions (used by graceful-shutdown drain and
    /// the dashboard/metrics).
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// In-memory [`SessionStore`] — the single-node default. Shared across
/// handlers via `Arc`. The internal map is `DashMap` so per-request
/// `touch_or_register` only locks a single shard, never the whole
/// structure.
///
/// The registry write-lock is taken **only** on register/evict
/// (counter mutation). `touch` for an existing session goes
/// through DashMap alone — read traffic on a warm session never
/// blocks on the registry RwLock.
#[derive(Debug)]
pub struct InMemorySessionStore {
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

impl InMemorySessionStore {
    pub fn new() -> Self {
        Self {
            sessions: DashMap::new(),
        }
    }
}

#[async_trait::async_trait]
impl SessionStore for InMemorySessionStore {
    /// Record activity for `session_id`. If unknown, register it
    /// against `(spec_id, replica_id)` and increment the
    /// replica's counter. If known, just refresh `last_seen`.
    ///
    /// Returns whether this was a fresh registration so the
    /// caller can decide cookie-setting policy.
    async fn touch_or_register(
        &self,
        registry: &RwLock<ReplicaRegistry>,
        session_id: Uuid,
        spec_id: &str,
        replica_id: &ReplicaId,
        seat_reserved: bool,
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
            // Skip the increment when the caller already reserved the seat
            // atomically at pick time (#582 part 1) — else we'd double-count.
            if !seat_reserved {
                registry.write().await.inc_sessions(replica_id);
            }
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
    /// Each session's expiry is resolved **per spec**: the
    /// spec's `heartbeat-timeout` override if it set one, else
    /// `global_ms`. A negative effective timeout (the `-1`
    /// ShinyProxy idiom) means "never expire" — those sessions
    /// are skipped. This lets a long-running Shiny app pin
    /// `heartbeat-timeout: -1` while the rest of the portal
    /// reaps idle sessions normally.
    async fn sweep(
        &self,
        registry: &RwLock<ReplicaRegistry>,
        global_ms: i64,
        overrides: &std::collections::HashMap<String, i64>,
        // Single-node store: this process is the only writer, so it
        // always evicts (the flag only matters for the shared HA store).
        _is_leader: bool,
    ) -> usize {
        let now = Instant::now();
        // Collect victims first so we don't hold DashMap shard
        // guards across the registry await.
        let mut victims: Vec<(Uuid, ReplicaId)> = Vec::new();
        for kv in self.sessions.iter() {
            let eff_ms = overrides
                .get(&kv.value().spec_id)
                .copied()
                .unwrap_or(global_ms);
            if eff_ms < 0 {
                continue; // never expire
            }
            let timeout = Duration::from_millis(eff_ms as u64);
            if now.duration_since(kv.value().last_seen) > timeout {
                victims.push((*kv.key(), kv.value().replica_id.clone()));
            }
        }
        if victims.is_empty() {
            return 0;
        }
        let mut evicted = 0;
        {
            let mut reg = registry.write().await;
            for (sid, rid) in &victims {
                // Remove first; decrement only when WE removed it. A
                // concurrent `end_session` (stop-on-logout #337) may have
                // already evicted this sid and decremented — a second
                // decrement undercounts the replica's seats and re-opens
                // the over-admission class #582 closed (#744).
                if self.sessions.remove(sid).is_some() {
                    reg.dec_sessions(rid);
                    evicted += 1;
                }
            }
        }
        evicted
    }

    /// Purge every session entry pointing at `replica_id`. Collects the
    /// matching keys first (so no DashMap shard guard is held across the
    /// removals) then drops them. No registry interaction — see the trait
    /// doc.
    async fn drop_replica(&self, replica_id: &ReplicaId) -> usize {
        let victims: Vec<Uuid> = self
            .sessions
            .iter()
            .filter(|kv| &kv.value().replica_id == replica_id)
            .map(|kv| *kv.key())
            .collect();
        for sid in &victims {
            self.sessions.remove(sid);
        }
        victims.len()
    }

    /// End one session by id: remove it and decrement its replica's
    /// counter (mirrors a single sweep eviction). Returns whether it
    /// existed. The DashMap removal yields the entry's `replica_id`, so
    /// we only take the registry write lock when there's real work.
    async fn end_session(
        &self,
        registry: &RwLock<ReplicaRegistry>,
        session_id: Uuid,
    ) -> bool {
        match self.sessions.remove(&session_id) {
            Some((_, entry)) => {
                registry.write().await.dec_sessions(&entry.replica_id);
                true
            }
            None => false,
        }
    }

    /// Number of tracked sessions. For introspection / tests.
    fn len(&self) -> usize {
        self.sessions.len()
    }
}

impl Default for InMemorySessionStore {
    fn default() -> Self {
        Self::new()
    }
}

/// Spawn the sweeper as a detached tokio task. Returns a handle
/// that callers can drop on shutdown — the loop only does
/// idempotent work, no graceful-stop protocol needed.
///
/// `global_ms` is `proxy.heartbeat-timeout`; `overrides` maps a
/// spec id to its own `heartbeat-timeout` when the spec set one.
/// A negative value (global or per-spec) means "never expire"
/// for the matching sessions — the sweep simply skips them, so
/// there's no special no-op task branch anymore.
pub fn spawn(
    tracker: Arc<dyn SessionStore>,
    registry: Arc<RwLock<ReplicaRegistry>>,
    config: Arc<ruscker_config::Config>,
    leader: Arc<dyn crate::leader::LeaderElector>,
    db: Option<crate::db::ConfigDb>,
) -> JoinHandle<()> {
    let global_ms = config.proxy.heartbeat_timeout;
    tokio::spawn(async move {
        info!(
            global_ms,
            interval = ?SWEEPER_INTERVAL,
            "session sweeper started"
        );
        let mut ticker = tokio::time::interval(SWEEPER_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            // Per-spec `heartbeat-timeout` overrides, rebuilt each tick
            // from the effective (DB-first) catalog — so DB-only specs'
            // overrides apply and a runtime edit takes effect without a
            // restart (#257). One cheap `list` query per tick.
            let overrides: std::collections::HashMap<String, i64> =
                crate::catalog::effective_specs(db.as_ref(), &config)
                    .await
                    .iter()
                    .filter_map(|s| s.heartbeat_timeout.map(|t| (s.id.clone(), t)))
                    .collect();
            // Only the leader issues the idle-eviction DELETEs in a
            // shared (HA) store; the reconcile runs regardless (#159 C3).
            // `AlwaysLeader` (single-node) is always true.
            let is_leader = leader.is_leader().await;
            let evicted = tracker
                .sweep(&registry, global_ms, &overrides, is_leader)
                .await;
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
            host: None,
        }
    }

    #[tokio::test]
    async fn works_through_dyn_session_store() {
        // The whole point of 7a: the proxy/sweeper depend on the trait
        // object, not the concrete store. Exercise it through `dyn`.
        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let r = fake_replica("alpha");
        let rid = r.id.clone();
        reg.write().await.add(r);
        let store: Arc<dyn SessionStore> = Arc::new(InMemorySessionStore::new());

        let sid = Uuid::new_v4();
        assert_eq!(
            store.touch_or_register(&reg, sid, "alpha", &rid, false).await,
            TouchOutcome::Registered
        );
        assert_eq!(store.len(), 1);
        assert!(!store.is_empty());
        // Idle sweep through the trait object (is_empty is the trait default).
        let evicted = store
            .sweep(&reg, 0, &std::collections::HashMap::new(), true)
            .await;
        assert_eq!(evicted, 1);
        assert!(store.is_empty());
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 0);
    }

    #[tokio::test]
    async fn drop_replica_purges_only_that_replicas_sessions() {
        // #324: stopping/restarting a replica must purge its sessions so
        // `len()` doesn't stay inflated until the idle sweep — fatal with
        // `heartbeat-timeout: -1`, where the sweep never runs.
        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let ra = fake_replica("alpha");
        let rb = fake_replica("beta");
        let (rid_a, rid_b) = (ra.id.clone(), rb.id.clone());
        reg.write().await.add(ra);
        reg.write().await.add(rb);
        let store = InMemorySessionStore::new();

        // Two sessions on A, one on B.
        for spec_rid in [("alpha", &rid_a), ("alpha", &rid_a), ("beta", &rid_b)] {
            store
                .touch_or_register(&reg, Uuid::new_v4(), spec_rid.0, spec_rid.1, false)
                .await;
        }
        assert_eq!(store.len(), 3);

        // Drop A: its two sessions go, B's one stays.
        let removed = store.drop_replica(&rid_a).await;
        assert_eq!(removed, 2);
        assert_eq!(store.len(), 1);

        // Idempotent: dropping again removes nothing.
        assert_eq!(store.drop_replica(&rid_a).await, 0);
        assert_eq!(store.len(), 1);
    }

    #[tokio::test]
    async fn end_session_removes_one_and_decrements() {
        // #337: stop-on-logout ends a single session by id, decrementing
        // its replica's counter (like one sweep eviction).
        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let r = fake_replica("alpha");
        let rid = r.id.clone();
        reg.write().await.add(r);
        let store = InMemorySessionStore::new();

        let s1 = Uuid::new_v4();
        let s2 = Uuid::new_v4();
        store.touch_or_register(&reg, s1, "alpha", &rid, false).await;
        store.touch_or_register(&reg, s2, "alpha", &rid, false).await;
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 2);

        // End s1: removed, counter drops to 1.
        assert!(store.end_session(&reg, s1).await);
        assert_eq!(store.len(), 1);
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 1);

        // Unknown / already-ended id ⇒ no-op false, counter unchanged.
        assert!(!store.end_session(&reg, s1).await);
        assert!(!store.end_session(&reg, Uuid::new_v4()).await);
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 1);
    }

    #[tokio::test]
    async fn first_touch_registers_and_increments() {
        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let r = fake_replica("alpha");
        let rid = r.id.clone();
        reg.write().await.add(r);
        let tracker = InMemorySessionStore::new();

        let sid = Uuid::new_v4();
        let outcome = tracker
            .touch_or_register(&reg, sid, "alpha", &rid, false)
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
        let tracker = InMemorySessionStore::new();

        let sid = Uuid::new_v4();
        let _ = tracker.touch_or_register(&reg, sid, "alpha", &rid, false).await;
        let outcome = tracker.touch_or_register(&reg, sid, "alpha", &rid, false).await;
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
        let tracker = InMemorySessionStore::new();

        let sid = Uuid::new_v4();
        tracker.touch_or_register(&reg, sid, "alpha", &rid, false).await;
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 1);

        // Force the entry's last_seen back in time so it expires
        // under a small timeout.
        if let Some(mut e) = tracker.sessions.get_mut(&sid) {
            e.last_seen = Instant::now() - Duration::from_secs(60);
        }

        let no_overrides = std::collections::HashMap::new();
        let evicted = tracker.sweep(&reg, 10_000, &no_overrides, true).await;
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
        let tracker = InMemorySessionStore::new();

        let sid = Uuid::new_v4();
        tracker.touch_or_register(&reg, sid, "alpha", &rid, false).await;
        // last_seen is now, sweep with 10s timeout — nothing
        // should evict.
        let no_overrides = std::collections::HashMap::new();
        let evicted = tracker.sweep(&reg, 10_000, &no_overrides, true).await;
        assert_eq!(evicted, 0);
        assert_eq!(tracker.len(), 1);
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 1);
    }

    #[tokio::test]
    async fn sweep_respects_per_spec_never_expire() {
        // Global timeout is short, but spec "pinned" overrides
        // to -1 (never expire). Its session must survive even
        // when stale; a different spec on the global timeout
        // still gets reaped.
        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let pinned = fake_replica("pinned");
        let normal = fake_replica("normal");
        let (pid, nid) = (pinned.id.clone(), normal.id.clone());
        {
            let mut w = reg.write().await;
            w.add(pinned);
            w.add(normal);
        }
        let tracker = InMemorySessionStore::new();
        let s_pinned = Uuid::new_v4();
        let s_normal = Uuid::new_v4();
        tracker.touch_or_register(&reg, s_pinned, "pinned", &pid, false).await;
        tracker.touch_or_register(&reg, s_normal, "normal", &nid, false).await;
        // Age both sessions well past the global timeout.
        for sid in [s_pinned, s_normal] {
            if let Some(mut e) = tracker.sessions.get_mut(&sid) {
                e.last_seen = Instant::now() - Duration::from_secs(600);
            }
        }
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("pinned".to_string(), -1_i64); // never expire

        let evicted = tracker.sweep(&reg, 10_000, &overrides, true).await;
        assert_eq!(evicted, 1, "only the global-timeout session is reaped");
        // pinned survives, normal is gone
        assert!(tracker.sessions.contains_key(&s_pinned));
        assert!(!tracker.sessions.contains_key(&s_normal));
        assert_eq!(reg.read().await.replicas_of("pinned")[0].sessions_active, 1);
        assert_eq!(reg.read().await.replicas_of("normal")[0].sessions_active, 0);
    }

    #[tokio::test]
    async fn sweep_per_spec_shorter_timeout_reaps_sooner() {
        // A spec with a shorter override expires while the
        // global-timeout sibling is still fresh.
        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let r = fake_replica("snappy");
        let rid = r.id.clone();
        reg.write().await.add(r);
        let tracker = InMemorySessionStore::new();
        let sid = Uuid::new_v4();
        tracker.touch_or_register(&reg, sid, "snappy", &rid, false).await;
        // 30s old.
        if let Some(mut e) = tracker.sessions.get_mut(&sid) {
            e.last_seen = Instant::now() - Duration::from_secs(30);
        }
        let mut overrides = std::collections::HashMap::new();
        overrides.insert("snappy".to_string(), 5_000_i64); // 5s override

        // Global is 1h (would keep it), but the 5s override reaps it.
        let evicted = tracker.sweep(&reg, 3_600_000, &overrides, true).await;
        assert_eq!(evicted, 1);
    }
}
