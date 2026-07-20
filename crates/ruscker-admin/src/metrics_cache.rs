//! In-memory cache of per-replica metrics for the dashboard.
//!
//! Each `backend.metrics(replica_id)` call hits the Docker
//! daemon's stats endpoint, which on the local socket costs a
//! few milliseconds per container. With N replicas, a dashboard
//! render that polled every replica synchronously would block
//! the request for ~N × few ms — fine at N=5, painful at N=50.
//!
//! This module solves it with a single background tokio task
//! that:
//!
//! 1. Snapshots the registry.
//! 2. Fans out `backend.metrics()` calls in parallel (via
//!    `futures_util::future::join_all`).
//! 3. Writes the results into a shared `DashMap` keyed by
//!    `ReplicaId`.
//!
//! Dashboard handlers read straight out of the DashMap — no
//! awaits on Docker — and accept that the data is up to
//! [`REFRESH_INTERVAL`] seconds stale. For a monitoring view
//! that's correct: real-time CPU bars on a 10-tab admin would
//! be visual noise anyway.

use dashmap::DashMap;
use ruscker_core::{ContainerBackend, ReplicaId, ReplicaMetrics};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// How often the background task refreshes the cache. 5 s
/// matches the dashboard mockup's polling indicator and keeps
/// CPU delta windows large enough to read sanely.
pub const REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Max concurrent `stats` round-trips to the Docker daemon per refresh
/// (#288). Bounds the burst on a host with many replicas; the calls
/// still overlap, just not all at once.
const MAX_CONCURRENT_STATS: usize = 8;

/// How many recent samples to keep per replica for the dashboard
/// sparklines. 30 × [`REFRESH_INTERVAL`] (5 s) ≈ 2.5 min of history —
/// enough to show a trend without unbounded growth.
pub const HISTORY_LEN: usize = 30;

/// What we keep per replica: the latest reading plus a short rolling
/// history of CPU% and memory for the dashboard sparklines (oldest
/// first, most recent last).
#[derive(Debug, Clone)]
pub struct CachedMetrics {
    pub metrics: ReplicaMetrics,
    pub observed_at: Instant,
    pub cpu_history: Vec<f64>,
    pub mem_history: Vec<u64>,
}

/// The cache itself. Cheap to clone (it's all `Arc`s underneath)
/// so the refresher task and the dashboard handler share the
/// same map.
#[derive(Debug, Clone, Default)]
pub struct MetricsCache {
    inner: Arc<DashMap<ReplicaId, CachedMetrics>>,
}

impl MetricsCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: &ReplicaId) -> Option<CachedMetrics> {
        self.inner.get(id).map(|r| r.clone())
    }

    pub fn len(&self) -> usize {
        self.inner.len()
    }

    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }

    /// Forget one replica immediately. The periodic `replace` pass also
    /// garbage-collects stale rows, but liveness cleanup calls this so a dead
    /// or restarting container disappears from the dashboard in the same
    /// reconcile tick.
    pub fn remove(&self, id: &ReplicaId) {
        self.inner.remove(id);
    }

    /// Replace the cache contents with the given (id, metrics)
    /// pairs and drop any entries for ids that didn't appear.
    /// Called by the refresher each tick. Each replica's rolling
    /// CPU/memory history is **carried forward** and appended to (capped
    /// at [`HISTORY_LEN`]), so the dashboard sparklines accumulate a
    /// trend instead of resetting every tick. A replica that
    /// disappears and comes back starts its history fresh.
    pub fn replace(&self, fresh: Vec<(ReplicaId, ReplicaMetrics)>) {
        use std::collections::HashSet;
        let now = Instant::now();
        let kept: HashSet<ReplicaId> = fresh.iter().map(|(id, _)| id.clone()).collect();
        self.inner.retain(|id, _| kept.contains(id));
        for (id, m) in fresh {
            // Carry the prior history forward, then append this sample.
            let (mut cpu_history, mut mem_history) = self
                .inner
                .get(&id)
                .map(|e| (e.cpu_history.clone(), e.mem_history.clone()))
                .unwrap_or_default();
            cpu_history.push(m.cpu_percent);
            mem_history.push(m.memory_bytes);
            // Keep only the most recent HISTORY_LEN samples.
            if cpu_history.len() > HISTORY_LEN {
                let drop = cpu_history.len() - HISTORY_LEN;
                cpu_history.drain(0..drop);
                mem_history.drain(0..drop);
            }
            self.inner.insert(
                id,
                CachedMetrics {
                    metrics: m,
                    observed_at: now,
                    cpu_history,
                    mem_history,
                },
            );
        }
    }
}

/// Spawn the refresher loop as a detached tokio task. Returns a
/// handle that callers can drop on shutdown — each tick is
/// idempotent so an abrupt drop just stops the polling.
///
/// Stops gracefully on backend errors: a single failed
/// `metrics()` call logs a warning and the replica is omitted
/// from this tick's update; the next tick retries.
pub fn spawn(
    cache: MetricsCache,
    backend: Arc<dyn ContainerBackend>,
    replicas: Arc<RwLock<ruscker_core::ReplicaRegistry>>,
    interval: Duration,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!(?interval, "metrics-cache refresher started");
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            refresh_once(&cache, backend.as_ref(), &replicas).await;
        }
    })
}

async fn refresh_once(
    cache: &MetricsCache,
    backend: &dyn ContainerBackend,
    replicas: &RwLock<ruscker_core::ReplicaRegistry>,
) {
    // Snapshot (replica id, container id) under the read lock, then
    // release it before issuing any I/O. Passing the container id the
    // registry already knows lets the backend skip a per-replica
    // `list_containers` lookup on each refresh (#282).
    let targets: Vec<(ReplicaId, String)> = {
        let reg = replicas.read().await;
        reg.all().map(|r| (r.id.clone(), r.container_id.clone())).collect()
    };
    if targets.is_empty() {
        cache.replace(Vec::new());
        return;
    }

    // Fan out `backend.metrics_for()` calls, but **bounded** — at most
    // `MAX_CONCURRENT_STATS` in flight at once (#288). `join_all` would
    // fire one Docker `stats` round-trip per replica simultaneously,
    // which on a host with many replicas bursts the daemon and can drag
    // the whole admin. `buffer_unordered` caps the burst while still
    // overlapping the independent round-trips.
    use futures_util::stream::{self, StreamExt};
    let results: Vec<(ReplicaId, Result<ReplicaMetrics, _>)> =
        stream::iter(targets.into_iter().map(|(id, container_id)| {
            let id_for_err = id.clone();
            async move {
                let r = backend.metrics_for(&id, &container_id).await;
                (id_for_err, r)
            }
        }))
        .buffer_unordered(MAX_CONCURRENT_STATS)
        .collect()
        .await;

    let mut fresh = Vec::with_capacity(results.len());
    for (id, result) in results {
        match result {
            Ok(m) => fresh.push((id, m)),
            Err(e) => {
                warn!(replica = ?id, error = ?e, "metrics refresh failed");
            }
        }
    }
    cache.replace(fresh);
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ruscker_core::{CoreResult, Replica, ReplicaId, ReplicaRegistry, ReplicaState};
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicU32, Ordering};

    fn fake_replica(spec: &str) -> Replica {
        Replica {
            id: ReplicaId(uuid::Uuid::new_v4()),
            spec_id: spec.into(),
            container_id: "x".into(),
            upstream: "127.0.0.1:1".parse::<SocketAddr>().unwrap(),
            state: ReplicaState::Ready,
            started_at: chrono::Utc::now(),
            sessions_active: 0,
            sessions_max: 1,
            host: None,
        }
    }

    struct CountingMetricsBackend {
        calls: AtomicU32,
    }
    #[async_trait]
    impl ContainerBackend for CountingMetricsBackend {
        async fn spawn(&self, _spec_id: &str, _image: &str) -> CoreResult<Replica> {
            unimplemented!()
        }
        async fn stop(&self, _id: &ReplicaId) -> CoreResult<()> {
            Ok(())
        }
        async fn list(&self) -> CoreResult<Vec<Replica>> {
            Ok(vec![])
        }
        async fn metrics(&self, _id: &ReplicaId) -> CoreResult<ReplicaMetrics> {
            let n = self.calls.fetch_add(1, Ordering::SeqCst) as f64;
            Ok(ReplicaMetrics {
                cpu_percent: n,
                memory_bytes: 100,
                network_rx_bytes: 0,
                network_tx_bytes: 0,
            })
        }
    }

    #[tokio::test]
    async fn refresh_populates_cache_for_each_replica() {
        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let r1 = fake_replica("alpha");
        let r2 = fake_replica("beta");
        let (id1, id2) = (r1.id.clone(), r2.id.clone());
        {
            let mut w = reg.write().await;
            w.add(r1);
            w.add(r2);
        }
        let cache = MetricsCache::new();
        let backend = Arc::new(CountingMetricsBackend {
            calls: AtomicU32::new(0),
        });
        refresh_once(&cache, backend.as_ref(), &reg).await;

        assert_eq!(cache.len(), 2);
        assert!(cache.get(&id1).is_some());
        assert!(cache.get(&id2).is_some());
    }

    #[tokio::test]
    async fn replace_evicts_disappeared_replicas() {
        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let r1 = fake_replica("alpha");
        let id1 = r1.id.clone();
        reg.write().await.add(r1);
        let cache = MetricsCache::new();
        let backend = Arc::new(CountingMetricsBackend {
            calls: AtomicU32::new(0),
        });
        refresh_once(&cache, backend.as_ref(), &reg).await;
        assert!(cache.get(&id1).is_some());

        // Replica goes away.
        reg.write().await.remove(&id1);
        refresh_once(&cache, backend.as_ref(), &reg).await;
        assert!(cache.is_empty(), "stale entry must be evicted");
    }

    #[test]
    fn history_accumulates_and_caps() {
        let cache = MetricsCache::new();
        let id = ReplicaId(uuid::Uuid::new_v4());
        let sample = |cpu: f64, mem: u64| {
            vec![(
                id.clone(),
                ReplicaMetrics {
                    cpu_percent: cpu,
                    memory_bytes: mem,
                    network_rx_bytes: 0,
                    network_tx_bytes: 0,
                },
            )]
        };
        // Two ticks ⇒ two samples, oldest first.
        cache.replace(sample(1.0, 10));
        cache.replace(sample(2.0, 20));
        let c = cache.get(&id).unwrap();
        assert_eq!(c.cpu_history, vec![1.0, 2.0]);
        assert_eq!(c.mem_history, vec![10, 20]);
        assert_eq!(c.metrics.cpu_percent, 2.0); // latest

        // Push well past the cap; only the most recent HISTORY_LEN survive.
        for i in 0..HISTORY_LEN + 10 {
            cache.replace(sample(i as f64, i as u64));
        }
        let c = cache.get(&id).unwrap();
        assert_eq!(c.cpu_history.len(), HISTORY_LEN);
        // Last sample is the most recent push.
        assert_eq!(*c.cpu_history.last().unwrap(), (HISTORY_LEN + 9) as f64);
    }

    #[tokio::test]
    async fn empty_registry_clears_cache() {
        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let cache = MetricsCache::new();
        // Pre-poison the cache with a fake entry.
        cache.replace(vec![(
            ReplicaId(uuid::Uuid::new_v4()),
            ReplicaMetrics {
                cpu_percent: 42.0,
                memory_bytes: 1,
                network_rx_bytes: 0,
                network_tx_bytes: 0,
            },
        )]);
        let backend = Arc::new(CountingMetricsBackend {
            calls: AtomicU32::new(0),
        });
        refresh_once(&cache, backend.as_ref(), &reg).await;
        assert!(cache.is_empty());
    }
}
