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
/// spawn is reported and the loop continues with the next spec.
async fn tick(state: &AppState) {
    // No backend ⇒ landing-only mode; nothing to scale.
    let Some(backend) = state.backend.clone() else {
        return;
    };

    // Snapshot the spec list once — config is `Arc<Config>` so
    // this is a single Arc clone, not a deep walk.
    let specs = state.config.proxy.specs.clone();

    for spec in specs {
        let kind = spec.kind();
        // External specs route to a URL; nothing to spawn.
        if matches!(kind, SpecKind::External) {
            continue;
        }
        // No image ⇒ misconfigured spec. The validator already
        // warned at config-load time; we silently skip here.
        if spec.container_image.is_none() {
            continue;
        }

        let want = spec.effective_min_replicas() as usize;
        if want == 0 {
            continue;
        }

        let have = state.replicas.read().await.replicas_of(&spec.id).len();
        if have >= want {
            continue;
        }

        let to_spawn = want - have;
        info!(
            spec = %spec.id,
            have, want, to_spawn,
            "scaling spec up to min-replicas"
        );
        for _ in 0..to_spawn {
            if let Err(e) = spawn_one(state, &spec, backend.as_ref()).await {
                warn!(spec = %spec.id, error = ?e, "auto-scaler spawn failed");
                // Stop trying for THIS spec on THIS tick — if the
                // first spawn failed, the second probably will too
                // (image pull error, daemon down, etc.). Wait for
                // the next tick to retry.
                break;
            }
        }
    }
}

/// Spawn one additional replica for `spec`. Mirrors the spawn
/// branch of `pick_or_spawn` but does NOT short-circuit when
/// replicas already exist (the scaler is the one path that
/// deliberately wants "spawn more"). Still goes through the
/// per-spec mutex so a concurrent on-demand spawn coalesces with
/// us.
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

    // Re-check under the mutex. A first-request spawn that landed
    // between our snapshot read and the mutex acquire may have
    // already added the replica we were going to spawn; in that
    // case we yield to it and skip.
    let have = state.replicas.read().await.replicas_of(&spec.id).len();
    if have >= spec.effective_min_replicas() as usize {
        return Ok(());
    }

    let image = spec
        .container_image
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("spec {} has no container-image", spec.id))?;

    let inner_port = spec
        .api
        .as_ref()
        .and_then(|a| a.port)
        .or_else(|| infer_inner_port(spec));

    let replica = match inner_port {
        Some(port) => backend.spawn_with_port(&spec.id, image, port).await,
        None => backend.spawn(&spec.id, image).await,
    }
    .map_err(|e| anyhow::anyhow!("backend spawn: {e}"))?;

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
}
