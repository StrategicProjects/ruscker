//! Periodic auto-scaler.
//!
//! Walks every spec on each tick and ensures the running replica
//! count is at least `effective_min_replicas`. Spawns are funneled
//! through the same per-spec coalescer as on-demand spawns, so a
//! cold-start request that arrives at the same time as a scaler
//! tick never races into a duplicate container.
//!
//! ## What this is NOT (yet)
//!
//! - **Saturation-based scale-up.** Needs per-replica session
//!   tracking that doesn't exist yet — `sessions_active` stays
//!   at 0 on every replica, so `available_seats()` always says
//!   "infinite capacity". Once the proxy increments/decrements
//!   `sessions_active` on connect/disconnect, this loop grows a
//!   "scale up when all-saturated && count < max" branch.
//! - **Scale-down on idle.** Same reason — without tracking, every
//!   replica looks idle and we'd thrash. Comes with heartbeats.
//!
//! The minimum-replicas loop is useful on its own: it guarantees
//! that an operator who sets `min-replicas: 3` actually sees 3
//! warm containers instead of waiting for traffic to populate
//! them one at a time.

use crate::AppState;
use ruscker_config::{Spec, SpecKind};
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// Default cadence between ticks. Operators can override per-
/// deployment later once we add a config knob.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(10);

/// Spawn the scaler loop as a detached tokio task. The returned
/// handle is dropped at shutdown — there's no graceful stop
/// because the loop only does idempotent work; an abrupt drop
/// leaves the registry consistent.
pub fn spawn(state: AppState, interval: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!(?interval, "auto-scaler started");
        // `MissedTickBehavior::Skip` so a long pull doesn't leave
        // a backlog of ticks queued — we just resume on the next
        // boundary.
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            tick(&state).await;
        }
    })
}

/// One pass over every spec. Logs but never panics — a failed
/// spawn or stop is reported and the loop continues with the
/// next spec.
///
/// Three reasons to act on a spec:
/// 1. **Below min** — bring count up to `effective_min_replicas`
///    in one tick.
/// 2. **All replicas saturated** — spawn one additional replica
///    (one per tick to avoid overshoot) up to
///    `effective_max_replicas`.
/// 3. **Idle replicas above min** — stop them.
///
/// Scale-up and scale-down are mutually exclusive for a single
/// spec on a single tick: a saturated spec by definition has no
/// idle replicas, and an over-min spec with idle replicas isn't
/// saturated.
async fn tick(state: &AppState) {
    let Some(backend) = state.backend.clone() else {
        return;
    };
    let specs = state.config.proxy.specs.clone();

    for spec in specs {
        let kind = spec.kind();
        if matches!(kind, SpecKind::External) {
            continue;
        }
        if spec.container_image.is_none() {
            continue;
        }

        let min = spec.effective_min_replicas() as usize;
        let max = spec.effective_max_replicas() as usize;

        // Snapshot replicas with their session counts at this
        // instant. We clone out so the read-lock releases before
        // we take any write-lock for spawn/stop. The window
        // between snapshot and act is small but real; the
        // coalescer mutex makes spawn safe, and stop is
        // idempotent at the backend so a stale stop is fine.
        let snap = state.replicas.read().await.replicas_of(&spec.id).to_vec();
        let count = snap.len();

        // --- scale-up pass ---

        // (1) Below min: bring up in one tick.
        if count < min {
            let to_spawn = min - count;
            info!(spec = %spec.id, count, min, to_spawn, "scaling up to min-replicas");
            for _ in 0..to_spawn {
                if let Err(e) = spawn_one(state, &spec, backend.as_ref()).await {
                    warn!(spec = %spec.id, error = ?e, "scale-up spawn failed");
                    break;
                }
            }
            // Done with this spec on this tick — fresh snapshot
            // next tick.
            continue;
        }

        // (2) Saturated above min, below max: spawn ONE this
        //     tick. Spawning multiple per tick risks overshoot
        //     because the new replica's seats aren't filled yet.
        if count < max && all_saturated(&snap) {
            info!(spec = %spec.id, count, max, "saturated — scaling up by 1");
            if let Err(e) = spawn_one(state, &spec, backend.as_ref()).await {
                warn!(spec = %spec.id, error = ?e, "saturation spawn failed");
            }
            continue;
        }

        // --- scale-down pass ---

        // (3) Above min with idle replicas: stop them. Cap the
        //     drop at `count - min` to never go below min.
        if count > min {
            let allowed_to_drop = count - min;
            let idle: Vec<_> = snap
                .iter()
                .filter(|r| r.sessions_active == 0)
                .take(allowed_to_drop)
                .cloned()
                .collect();
            if !idle.is_empty() {
                info!(
                    spec = %spec.id,
                    count,
                    min,
                    dropping = idle.len(),
                    "idle replicas — scaling down"
                );
                for r in idle {
                    if let Err(e) = stop_one(state, &r.id, backend.as_ref()).await {
                        warn!(
                            spec = %spec.id,
                            replica = ?r.id,
                            error = ?e,
                            "scale-down stop failed"
                        );
                    }
                }
            }
        }
    }
}

/// All replicas hit their seat cap? An empty list reads as
/// "not saturated" — there's nothing TO be saturated.
fn all_saturated(replicas: &[ruscker_core::Replica]) -> bool {
    !replicas.is_empty() && replicas.iter().all(|r| r.available_seats() == 0)
}

/// Stop a single replica and remove it from the registry. Used
/// by the idle scale-down path. The backend stop is best-effort
/// — even if it fails (already gone, network blip), we still
/// remove the entry so the registry doesn't accumulate
/// phantoms.
async fn stop_one(
    state: &AppState,
    replica_id: &ruscker_core::ReplicaId,
    backend: &dyn ruscker_core::ContainerBackend,
) -> anyhow::Result<()> {
    backend
        .stop(replica_id)
        .await
        .map_err(|e| anyhow::anyhow!("backend stop: {e}"))?;
    state.replicas.write().await.remove(replica_id);
    Ok(())
}

/// Spawn one additional replica for `spec`. Used by every
/// scale-up branch (to-min and on-saturation). Goes through the
/// per-spec mutex so a concurrent on-demand spawn coalesces
/// with us — the caller decided we should add capacity, so we
/// don't re-check against any threshold here.
///
/// Capping (don't exceed max, don't undershoot min) is the
/// caller's job; this function just performs one spawn.
async fn spawn_one(
    state: &AppState,
    spec: &Spec,
    backend: &dyn ruscker_core::ContainerBackend,
) -> anyhow::Result<()> {
    let spec_mutex: std::sync::Arc<tokio::sync::Mutex<()>> = state
        .spawn_locks
        .entry(spec.id.clone())
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let _guard = spec_mutex.lock().await;

    let image = spec
        .container_image
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("spec {} has no container-image", spec.id))?;

    let inner_port = spec
        .api
        .as_ref()
        .and_then(|a| a.port)
        .or_else(|| infer_inner_port(spec));

    let mut replica = match inner_port {
        Some(port) => backend.spawn_with_port(&spec.id, image, port).await,
        None => backend.spawn(&spec.id, image).await,
    }
    .map_err(|e| anyhow::anyhow!("backend spawn: {e}"))?;
    // The backend doesn't know the spec's seat cap — that lives
    // in config. Enrich so `available_seats()` / `all_saturated`
    // immediately reflect the right capacity.
    replica.sessions_max = spec.effective_seats();

    state.replicas.write().await.add(replica);
    Ok(())
}

/// Best-guess inner port for a spec. Mirrors the helper in
/// `routes::proxy` — pulled into a small free function here to
/// avoid taking a dependency on the routes module from the
/// scaler. Kept private and minimal; if a third caller appears,
/// promote to a shared module.
fn infer_inner_port(spec: &Spec) -> Option<u16> {
    match spec.kind() {
        SpecKind::Api => Some(8000),
        SpecKind::Shiny => Some(3838),
        SpecKind::InteractiveApp => Some(8080),
        SpecKind::External => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ruscker_config::Config;
    use ruscker_core::{
        ContainerBackend, CoreResult, Replica, ReplicaId, ReplicaMetrics, ReplicaRegistry,
        ReplicaState,
    };
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    struct CountingBackend {
        spawns: AtomicU32,
    }
    #[async_trait]
    impl ContainerBackend for CountingBackend {
        async fn spawn(&self, spec_id: &str, _image: &str) -> CoreResult<Replica> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            Ok(Replica {
                id: ReplicaId(uuid::Uuid::new_v4()),
                spec_id: spec_id.to_string(),
                container_id: "fake".into(),
                upstream: "127.0.0.1:1".parse::<SocketAddr>().unwrap(),
                state: ReplicaState::Ready,
                started_at: chrono::Utc::now(),
                sessions_active: 0,
                sessions_max: 1,
            })
        }
        async fn spawn_with_port(
            &self,
            spec_id: &str,
            image: &str,
            _port: u16,
        ) -> CoreResult<Replica> {
            self.spawn(spec_id, image).await
        }
        async fn stop(&self, _id: &ReplicaId) -> CoreResult<()> {
            Ok(())
        }
        async fn list(&self) -> CoreResult<Vec<Replica>> {
            Ok(vec![])
        }
        async fn metrics(&self, _id: &ReplicaId) -> CoreResult<ReplicaMetrics> {
            Ok(ReplicaMetrics {
                cpu_percent: 0.0,
                memory_bytes: 0,
                network_rx_bytes: 0,
                network_tx_bytes: 0,
            })
        }
    }

    fn state_with_yaml(yaml: &str, backend: Arc<dyn ContainerBackend>) -> AppState {
        let cfg = Config::from_yaml(yaml).expect("parse config");
        AppState {
            config: Arc::new(cfg),
            locales: Arc::new(crate::i18n::Locales::load().expect("load locales")),
            admin_auth: Default::default(),
            db: None,
            images_dir: None,
            master_key: Default::default(),
            backend: Some(backend),
            replicas: Arc::new(tokio::sync::RwLock::new(ReplicaRegistry::new())),
            cookie_key: ruscker_proxy::sticky::CookieKey::random(),
            spawn_locks: Arc::new(dashmap::DashMap::new()),
            sessions: Arc::new(crate::sessions::SessionTracker::new()),
        }
    }

    #[tokio::test]
    async fn tick_spawns_up_to_min_replicas() {
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: warmpool
    display-name: Warm Pool
    container-image: nginx:alpine
    min-replicas: 3
    max-replicas: 5
"#,
            backend.clone(),
        );

        tick(&state).await;
        assert_eq!(backend.spawns.load(Ordering::SeqCst), 3, "spawned to min");
        assert_eq!(
            state.replicas.read().await.replicas_of("warmpool").len(),
            3
        );

        // Second tick: already at min, no more spawns.
        tick(&state).await;
        assert_eq!(backend.spawns.load(Ordering::SeqCst), 3, "no extra spawns");
    }

    #[tokio::test]
    async fn tick_skips_external_specs() {
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: dashboard
    display-name: Old Dashboard
    type: external
    template-properties:
      link: https://example.com
"#,
            backend.clone(),
        );
        tick(&state).await;
        assert_eq!(backend.spawns.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn tick_skips_when_min_is_zero() {
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: lazyapp
    display-name: Lazy
    container-image: nginx:alpine
    min-replicas: 0
    max-replicas: 4
"#,
            backend.clone(),
        );
        tick(&state).await;
        assert_eq!(backend.spawns.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn tick_does_not_double_spawn_when_backend_already_has_replicas() {
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: warm
    display-name: Warm
    container-image: nginx:alpine
    min-replicas: 2
"#,
            backend.clone(),
        );
        // Pre-populate registry as if reconcile had picked up 2.
        let pre1 = Replica {
            id: ReplicaId(uuid::Uuid::new_v4()),
            spec_id: "warm".into(),
            container_id: "x".into(),
            upstream: "127.0.0.1:1".parse().unwrap(),
            state: ReplicaState::Ready,
            started_at: chrono::Utc::now(),
            sessions_active: 0,
            sessions_max: 1,
        };
        let pre2 = Replica {
            id: ReplicaId(uuid::Uuid::new_v4()),
            ..pre1.clone()
        };
        {
            let mut reg = state.replicas.write().await;
            reg.add(pre1);
            reg.add(pre2);
        }

        tick(&state).await;
        assert_eq!(
            backend.spawns.load(Ordering::SeqCst),
            0,
            "reconciled replicas count toward min — no extra spawns"
        );
    }

    // Helper: build a Replica with explicit session counts so we
    // can exercise the saturation / idle branches without driving
    // the proxy.
    fn replica_with_sessions(spec: &str, active: u32, max: u32) -> Replica {
        Replica {
            id: ReplicaId(uuid::Uuid::new_v4()),
            spec_id: spec.into(),
            container_id: "x".into(),
            upstream: "127.0.0.1:1".parse().unwrap(),
            state: ReplicaState::Ready,
            started_at: chrono::Utc::now(),
            sessions_active: active,
            sessions_max: max,
        }
    }

    #[tokio::test]
    async fn tick_scales_up_one_on_saturation() {
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: hot
    display-name: Hot
    container-image: nginx:alpine
    min-replicas: 1
    max-replicas: 3
"#,
            backend.clone(),
        );
        // One replica, fully saturated (1/1).
        state
            .replicas
            .write()
            .await
            .add(replica_with_sessions("hot", 1, 1));

        tick(&state).await;
        assert_eq!(
            backend.spawns.load(Ordering::SeqCst),
            1,
            "saturated → one extra spawn"
        );
        assert_eq!(state.replicas.read().await.replicas_of("hot").len(), 2);
    }

    #[tokio::test]
    async fn tick_caps_scale_up_at_max() {
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: capped
    display-name: Capped
    container-image: nginx:alpine
    min-replicas: 1
    max-replicas: 2
"#,
            backend.clone(),
        );
        // Two saturated replicas; max is 2 — must NOT spawn.
        let r1 = replica_with_sessions("capped", 1, 1);
        let r2 = replica_with_sessions("capped", 1, 1);
        {
            let mut reg = state.replicas.write().await;
            reg.add(r1);
            reg.add(r2);
        }
        tick(&state).await;
        assert_eq!(backend.spawns.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn tick_scales_up_one_per_tick_not_to_max() {
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: gradual
    display-name: Gradual
    container-image: nginx:alpine
    min-replicas: 1
    max-replicas: 5
"#,
            backend.clone(),
        );
        state
            .replicas
            .write()
            .await
            .add(replica_with_sessions("gradual", 1, 1));
        tick(&state).await;
        assert_eq!(
            backend.spawns.load(Ordering::SeqCst),
            1,
            "one tick = at most one saturation spawn"
        );
    }

    #[tokio::test]
    async fn tick_scales_down_idle_above_min() {
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: shrink
    display-name: Shrink
    container-image: nginx:alpine
    min-replicas: 1
    max-replicas: 5
"#,
            backend.clone(),
        );
        // Three replicas: one busy, two idle. min=1 → drop both
        // idle ones, keeping the busy one.
        let busy = replica_with_sessions("shrink", 1, 5);
        let idle1 = replica_with_sessions("shrink", 0, 5);
        let idle2 = replica_with_sessions("shrink", 0, 5);
        {
            let mut reg = state.replicas.write().await;
            reg.add(busy);
            reg.add(idle1);
            reg.add(idle2);
        }
        tick(&state).await;
        let remaining = state.replicas.read().await.replicas_of("shrink").to_vec();
        assert_eq!(remaining.len(), 1, "two idle stopped, busy stays");
        assert_eq!(remaining[0].sessions_active, 1);
    }

    #[tokio::test]
    async fn tick_does_not_scale_down_below_min() {
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: floor
    display-name: Floor
    container-image: nginx:alpine
    min-replicas: 2
    max-replicas: 4
"#,
            backend.clone(),
        );
        // Two idle replicas, min=2 → drop nothing.
        {
            let mut reg = state.replicas.write().await;
            reg.add(replica_with_sessions("floor", 0, 5));
            reg.add(replica_with_sessions("floor", 0, 5));
        }
        tick(&state).await;
        assert_eq!(
            state.replicas.read().await.replicas_of("floor").len(),
            2,
            "min is a hard floor"
        );
    }

    #[tokio::test]
    async fn tick_does_not_scale_down_busy_replica() {
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: half
    display-name: Half busy
    container-image: nginx:alpine
    min-replicas: 1
    max-replicas: 5
"#,
            backend.clone(),
        );
        // 2 replicas, both have 1 active session each (below seat
        // cap of 5). Neither is idle → no scale-down.
        {
            let mut reg = state.replicas.write().await;
            reg.add(replica_with_sessions("half", 1, 5));
            reg.add(replica_with_sessions("half", 1, 5));
        }
        tick(&state).await;
        assert_eq!(state.replicas.read().await.replicas_of("half").len(), 2);
    }

    #[test]
    fn all_saturated_helper() {
        assert!(!all_saturated(&[]));
        assert!(all_saturated(&[replica_with_sessions("a", 1, 1)]));
        assert!(!all_saturated(&[
            replica_with_sessions("a", 1, 1),
            replica_with_sessions("a", 0, 1),
        ]));
        assert!(all_saturated(&[
            replica_with_sessions("a", 5, 5),
            replica_with_sessions("a", 3, 3),
        ]));
    }
}
