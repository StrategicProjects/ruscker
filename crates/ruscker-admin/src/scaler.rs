//! Periodic auto-scaler.
//!
//! Walks every spec on each tick and reconciles its replica
//! count toward `[min, max]`. Three reasons to act:
//!
//! 1. **Below min** — bring count up to `effective_min_replicas`
//!    in one tick (no hysteresis; baseline capacity must exist).
//! 2. **All replicas saturated, count < max** — spawn one extra,
//!    but only after [`DEFAULT_SATURATION_GRACE_TICKS`] consecutive
//!    saturated observations. A single tick of saturation can be
//!    a slow request; sustained saturation is a real load signal.
//! 3. **Idle replicas above min** — stop them, but only after
//!    they've been observed idle for [`DEFAULT_IDLE_GRACE_TICKS`]
//!    consecutive ticks.
//!
//! ## Two-sided hysteresis
//!
//! Both scale-up (saturation) and scale-down (idle) wait a few
//! ticks before acting. This buys two things:
//!
//! - **Noise tolerance**: a brief saturation spike or an idle
//!   blip doesn't ping-pong the replica count.
//! - **Flap dampening**: the worst-case flap (one user, one
//!   seat, long sticky session) is stretched by both the
//!   hysteresis windows AND a post-drop cooldown
//!   ([`DEFAULT_SCALE_DOWN_COOLDOWN_TICKS`]) that suppresses
//!   saturation scale-up of a spec right after it was scaled
//!   down. A spec we just dropped a replica from is almost
//!   certainly still saturated (the session that pinned it
//!   didn't leave), so without the cooldown we'd respawn
//!   immediately. With it, the cycle is at least
//!   `idle_grace + cooldown + saturation_grace` ticks.
//!
//! Spawns are funneled through the same per-spec coalescer as
//! on-demand spawns, so a cold-start request that arrives at
//! the same time as a scaler tick never races into a duplicate
//! container.

use crate::AppState;
use ruscker_config::{Spec, SpecKind};
use ruscker_core::ReplicaId;
use std::collections::HashMap;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// Default cadence between ticks. Operators can override per-
/// deployment later once we add a config knob.
pub const DEFAULT_INTERVAL: Duration = Duration::from_secs(10);

/// Default number of consecutive idle ticks a replica must be
/// observed before scale-down stops it. With the default 10s
/// interval that's ~30s of sustained idleness — long enough to
/// dampen the spawn↔drop flap, short enough that an operator
/// won't be wondering where their idle container went.
///
/// Setting this to 1 (used in unit tests) restores the
/// "drop on first observation" behavior.
pub const DEFAULT_IDLE_GRACE_TICKS: u32 = 3;

/// Default number of consecutive saturated ticks a spec must
/// be observed before scale-up spawns an extra replica. With
/// the default 10s interval that's a ~10–20s wait before
/// reacting to load. Shorter than [`DEFAULT_IDLE_GRACE_TICKS`]
/// because under-provisioning is worse for users than over-
/// provisioning is for the operator.
///
/// Setting this to 1 (used in unit tests) restores the
/// "spawn on first saturated observation" behavior.
pub const DEFAULT_SATURATION_GRACE_TICKS: u32 = 2;

/// After scale-down stops a replica for a spec, suppress
/// saturation-driven scale-up of that spec for this many ticks.
/// With the default 10s interval that's ~60s. This breaks the
/// residual spawn↔drop flap that the two-counter hysteresis
/// only dampens: with `seats-per-container: 1` and one
/// long-lived sticky session, replica A is permanently
/// saturated, so right after we drop the idle replica B the
/// saturation branch would want to spawn B again. The cooldown
/// holds that off, so the flap (if load truly stays this shape)
/// happens at most once per `idle_grace + cooldown + saturation_grace`
/// window instead of once per `idle_grace + saturation_grace`.
///
/// Below-min and genuinely-growing load are unaffected — the
/// cooldown only gates the *saturation* scale-up path.
pub const DEFAULT_SCALE_DOWN_COOLDOWN_TICKS: u32 = 6;

/// Spawn the scaler loop as a detached tokio task. The returned
/// handle is dropped at shutdown — there's no graceful stop
/// because the loop only does idempotent work; an abrupt drop
/// leaves the registry consistent.
pub fn spawn(state: AppState, interval: Duration) -> JoinHandle<()> {
    tokio::spawn(async move {
        info!(
            ?interval,
            idle_grace_ticks = DEFAULT_IDLE_GRACE_TICKS,
            saturation_grace_ticks = DEFAULT_SATURATION_GRACE_TICKS,
            scale_down_cooldown_ticks = DEFAULT_SCALE_DOWN_COOLDOWN_TICKS,
            "auto-scaler started"
        );
        // Per-replica idle-tick counter, per-spec saturated-tick
        // counter, and per-spec post-drop cooldown — all kept
        // inside the task so no locking is needed. Each resets
        // when its underlying condition stops holding.
        let mut idle_ticks: HashMap<ReplicaId, u32> = HashMap::new();
        let mut saturation_ticks: HashMap<String, u32> = HashMap::new();
        let mut cooldown: HashMap<String, u32> = HashMap::new();
        let mut ticker = tokio::time::interval(interval);
        // `MissedTickBehavior::Skip` so a long pull doesn't leave
        // a backlog of ticks queued — we just resume on the next
        // boundary.
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Tracks leadership across ticks so we log only on change.
        // `None` until the first observation so the first tick always
        // logs this instance's role — a single-node instance logs
        // "acquired leadership" once (confirming the scaler is live);
        // in HA the leader and every standby each announce themselves,
        // which the operator and the HA harness rely on to see who's
        // scaling. Subsequent ticks log only on a leadership change.
        let mut was_leader: Option<bool> = None;
        loop {
            ticker.tick().await;

            // HA: only the leader scales — otherwise every instance
            // would fight over the replica count against the shared
            // backend. Single-node uses `AlwaysLeader`, so this is a
            // free `true`. Stale per-spec/replica counters held in this
            // task are harmless: a new leader rebuilds them from the
            // live registry over the next few ticks.
            let is_leader = state.leader.is_leader().await;
            if was_leader != Some(is_leader) {
                if is_leader {
                    info!("auto-scaler: acquired leadership; scaling active");
                } else {
                    info!("auto-scaler: standing by (another instance leads)");
                }
                was_leader = Some(is_leader);
            }
            if !is_leader {
                continue;
            }

            tick(
                &state,
                &mut idle_ticks,
                &mut saturation_ticks,
                &mut cooldown,
                DEFAULT_IDLE_GRACE_TICKS,
                DEFAULT_SATURATION_GRACE_TICKS,
                DEFAULT_SCALE_DOWN_COOLDOWN_TICKS,
            )
            .await;
        }
    })
}

/// One pass over every spec. Logs but never panics — a failed
/// spawn or stop is reported and the loop continues with the
/// next spec.
///
/// - `idle_ticks` — per-replica observed-idle counter.
/// - `saturation_ticks` — per-spec observed-saturated counter.
/// - `cooldown` — per-spec remaining post-drop cooldown ticks.
/// - `drop_after_ticks` — hysteresis threshold for scale-down.
/// - `spawn_after_saturated_ticks` — hysteresis threshold for
///   scale-up on saturation. Below-min spawns never use this;
///   they're baseline capacity, not load-driven.
/// - `cooldown_ticks` — how long to suppress saturation
///   scale-up after a scale-down (per spec).
///
/// Scale-up and scale-down are mutually exclusive for a single
/// spec on a single tick: a saturated spec by definition has no
/// idle replicas, and an over-min spec with idle replicas isn't
/// saturated.
#[allow(clippy::too_many_arguments)]
async fn tick(
    state: &AppState,
    idle_ticks: &mut HashMap<ReplicaId, u32>,
    saturation_ticks: &mut HashMap<String, u32>,
    cooldown: &mut HashMap<String, u32>,
    drop_after_ticks: u32,
    spawn_after_saturated_ticks: u32,
    cooldown_ticks: u32,
) {
    let Some(backend) = state.backend.clone() else {
        return;
    };
    let specs = state.config.proxy.specs.clone();

    // Build a snapshot of every replica across every spec in one
    // read-lock acquisition. We use it for the idle-tick
    // accounting AND to GC counters for replicas that have left
    // the registry (stopped, crashed, reconciled out).
    let registry_snap: Vec<ruscker_core::Replica> = {
        let reg = state.replicas.read().await;
        state
            .config
            .proxy
            .specs
            .iter()
            .flat_map(|s| reg.replicas_of(&s.id).to_vec())
            .collect()
    };
    update_idle_ticks(idle_ticks, &registry_snap);

    // Spec ids present in this tick's config — used to GC
    // stale per-spec counters for deleted/renamed specs.
    let known_spec_ids: std::collections::HashSet<&str> =
        specs.iter().map(|s| s.id.as_str()).collect();
    saturation_ticks.retain(|id, _| known_spec_ids.contains(id.as_str()));
    cooldown.retain(|id, _| known_spec_ids.contains(id.as_str()));
    // GC the per-spec spawn coalescer locks for specs that no longer
    // exist. Removing the Arc from the map is safe even if a task still
    // holds a clone — that clone keeps the mutex alive until it's done.
    state
        .spawn_locks
        .retain(|id, _| known_spec_ids.contains(id.as_str()));

    for spec in &specs {
        let kind = spec.kind();
        if matches!(kind, SpecKind::External) {
            continue;
        }
        if spec.container_image.is_none() {
            continue;
        }

        let min = spec.effective_min_replicas() as usize;
        let max = spec.effective_max_replicas() as usize;

        // Read the post-drop cooldown for this spec, then
        // decrement it (creating no entry if absent). `cooling`
        // reflects the pre-decrement value so a cooldown of N
        // suppresses exactly N ticks.
        let cooling = cooldown.get(&spec.id).copied().unwrap_or(0) > 0;
        if let Some(c) = cooldown.get_mut(&spec.id) {
            *c = c.saturating_sub(1);
            if *c == 0 {
                cooldown.remove(&spec.id);
            }
        }

        // Per-spec snapshot — small Vec; not worth threading
        // through the registry-wide snapshot above.
        let snap = state.replicas.read().await.replicas_of(&spec.id).to_vec();
        let count = snap.len();

        // --- scale-up pass ---

        // (1) Below min: bring up in one tick. No hysteresis —
        //     baseline capacity is non-negotiable. Also reset
        //     saturation counter; we don't want a below-min
        //     interlude to count toward saturation.
        if count < min {
            let to_spawn = min - count;
            info!(spec = %spec.id, count, min, to_spawn, "scaling up to min-replicas");
            for _ in 0..to_spawn {
                if let Err(e) = spawn_one(state, spec, backend.as_ref()).await {
                    warn!(spec = %spec.id, error = ?e, "scale-up spawn failed");
                    break;
                }
            }
            saturation_ticks.remove(&spec.id);
            continue;
        }

        // (2) Saturation hysteresis: track consecutive saturated
        //     ticks. Only spawn once the counter crosses the
        //     threshold; reset on any non-saturated observation.
        if count < max && all_saturated(&snap) {
            // Post-drop cooldown: a spec we just scaled down is
            // very likely still saturated (the long-lived
            // session that pinned it never left). Holding off
            // the saturation spawn here is what actually breaks
            // the flap. Keep the counter at zero so when the
            // cooldown lifts we still require a fresh saturation
            // streak rather than spawning instantly.
            if cooling {
                saturation_ticks.remove(&spec.id);
                continue;
            }
            let n = saturation_ticks
                .entry(spec.id.clone())
                .and_modify(|n| *n += 1)
                .or_insert(1);
            if *n >= spawn_after_saturated_ticks {
                info!(
                    spec = %spec.id,
                    count,
                    max,
                    saturated_ticks = *n,
                    "saturation sustained — scaling up by 1"
                );
                if let Err(e) = spawn_one(state, spec, backend.as_ref()).await {
                    warn!(spec = %spec.id, error = ?e, "saturation spawn failed");
                }
                // Reset so the next spawn requires another full
                // grace window of sustained saturation —
                // prevents runaway spawns under brief noise.
                saturation_ticks.remove(&spec.id);
            }
            continue;
        }
        // Not saturated (or at cap) — clear the counter so a
        // future saturation streak starts from zero.
        saturation_ticks.remove(&spec.id);

        // --- scale-down pass (with hysteresis) ---

        // (3) Above min with idle replicas that have been idle
        //     for at least `drop_after_ticks` consecutive ticks:
        //     stop them. Cap the drop at `count - min` to never
        //     go below min.
        if count > min {
            let allowed_to_drop = count - min;
            let drop_candidates: Vec<_> = snap
                .iter()
                .filter(|r| {
                    r.sessions_active == 0
                        && idle_ticks.get(&r.id).copied().unwrap_or(0) >= drop_after_ticks
                })
                .take(allowed_to_drop)
                .cloned()
                .collect();
            if !drop_candidates.is_empty() {
                info!(
                    spec = %spec.id,
                    count,
                    min,
                    dropping = drop_candidates.len(),
                    grace_ticks = drop_after_ticks,
                    "idle replicas past grace — scaling down"
                );
                for r in drop_candidates {
                    if let Err(e) = stop_one(state, &r.id, backend.as_ref()).await {
                        warn!(
                            spec = %spec.id,
                            replica = ?r.id,
                            error = ?e,
                            "scale-down stop failed"
                        );
                    }
                    // Removed from registry → forget its counter
                    // so it can't haunt us if the same id
                    // somehow returns (it can't — uuids — but
                    // belt-and-braces).
                    idle_ticks.remove(&r.id);
                }
                // Arm the post-drop cooldown so the next saturation
                // spawn for this spec waits — breaks the flap.
                // Set after this tick's decrement so it lasts the
                // full window. `0` disables (tests).
                if cooldown_ticks > 0 {
                    cooldown.insert(spec.id.clone(), cooldown_ticks);
                }
            }
        }
    }
}

/// Bump or reset each replica's idle-tick counter based on
/// current `sessions_active`, then GC entries for replicas no
/// longer in the registry. Pure function over the snapshot —
/// no I/O, no locks, easy to test.
fn update_idle_ticks(
    idle_ticks: &mut HashMap<ReplicaId, u32>,
    registry_snap: &[ruscker_core::Replica],
) {
    use std::collections::HashSet;
    let alive: HashSet<&ReplicaId> = registry_snap.iter().map(|r| &r.id).collect();
    idle_ticks.retain(|id, _| alive.contains(id));
    for r in registry_snap {
        if r.sessions_active == 0 {
            *idle_ticks.entry(r.id.clone()).or_insert(0) += 1;
        } else {
            idle_ticks.remove(&r.id);
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

/// Spawn one replica for `spec` using the configured backend.
/// Public entry point for callers outside the scaler loop
/// (e.g. the dashboard's "restart" action). Resolves the
/// backend off `state`; errors if none is wired.
///
/// Same coalescer semantics as the internal scale-up path —
/// concurrent spawns for the same spec serialize on the
/// per-spec mutex.
pub(crate) async fn spawn_replica(state: &AppState, spec: &Spec) -> anyhow::Result<()> {
    let backend = state
        .backend
        .clone()
        .ok_or_else(|| anyhow::anyhow!("no container backend wired"))?;
    spawn_one(state, spec, backend.as_ref()).await
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

    let inner_port = spec.effective_inner_port();

    let creds = crate::routes::proxy::resolve_creds(state, spec).await;
    let limits = crate::routes::proxy::limits_from_spec(spec);
    let mut req = ruscker_core::SpawnRequest::new(&spec.id, image)
        .with_limits(limits)
        .with_volumes(spec.volumes.clone().unwrap_or_default())
        .with_env(spec.env_pairs())
        .with_placement(spec.effective_placement())
        .with_anti_affinity(spec.effective_anti_affinity());
    if let Some(port) = inner_port {
        req = req.with_port(port);
    }
    if let Some(platform) = spec.platform.as_deref() {
        req = req.with_platform(platform);
    }
    if let Some(cmd) = spec.container_cmd.clone() {
        req = req.with_cmd(cmd);
    }
    if let Some(c) = creds {
        req = req.with_creds(c);
    }
    let mut replica = backend
        .spawn_request(&req)
        .await
        .map_err(|e| anyhow::anyhow!("backend spawn: {e}"))?;
    // The backend doesn't know the spec's seat cap — that lives
    // in config. Enrich so `available_seats()` / `all_saturated`
    // immediately reflect the right capacity.
    replica.sessions_max = spec.effective_seats();

    state.replicas.write().await.add(replica);
    Ok(())
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
                host: None,
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
            base_path: Arc::from(""),
            locales: Arc::new(crate::i18n::Locales::load().expect("load locales")),
            admin_auth: Default::default(),
            admin_sessions: Arc::new(crate::auth::InMemoryAdminSessionStore::default()),
            log_buffer: None,
            login_limiter: Arc::new(crate::auth::LoginRateLimiter::default_policy()),
            api_limiter: Arc::new(crate::ratelimit::ApiRateLimiter::new()),
            db: None,
            images_dir: None,
            master_key: Default::default(),
            backend: Some(backend),
            replicas: Arc::new(tokio::sync::RwLock::new(ReplicaRegistry::new())),
            cookie_key: ruscker_proxy::sticky::CookieKey::random(),
            spawn_locks: Arc::new(dashmap::DashMap::new()),
            sessions: Arc::new(crate::sessions::InMemorySessionStore::new()),
            leader: Arc::new(crate::leader::AlwaysLeader),
            metrics: crate::metrics_cache::MetricsCache::new(),
            draining: Arc::new(std::sync::atomic::AtomicBool::new(false)),
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

        tick(&state, &mut HashMap::new(), &mut HashMap::new(), &mut HashMap::new(), 1, 1, 0).await;
        assert_eq!(backend.spawns.load(Ordering::SeqCst), 3, "spawned to min");
        assert_eq!(
            state.replicas.read().await.replicas_of("warmpool").len(),
            3
        );

        // Second tick: already at min, no more spawns.
        tick(&state, &mut HashMap::new(), &mut HashMap::new(), &mut HashMap::new(), 1, 1, 0).await;
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
        tick(&state, &mut HashMap::new(), &mut HashMap::new(), &mut HashMap::new(), 1, 1, 0).await;
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
        tick(&state, &mut HashMap::new(), &mut HashMap::new(), &mut HashMap::new(), 1, 1, 0).await;
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
            host: None,
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

        tick(&state, &mut HashMap::new(), &mut HashMap::new(), &mut HashMap::new(), 1, 1, 0).await;
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
            host: None,
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

        tick(&state, &mut HashMap::new(), &mut HashMap::new(), &mut HashMap::new(), 1, 1, 0).await;
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
        tick(&state, &mut HashMap::new(), &mut HashMap::new(), &mut HashMap::new(), 1, 1, 0).await;
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
        tick(&state, &mut HashMap::new(), &mut HashMap::new(), &mut HashMap::new(), 1, 1, 0).await;
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
        tick(&state, &mut HashMap::new(), &mut HashMap::new(), &mut HashMap::new(), 1, 1, 0).await;
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
        tick(&state, &mut HashMap::new(), &mut HashMap::new(), &mut HashMap::new(), 1, 1, 0).await;
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
        tick(&state, &mut HashMap::new(), &mut HashMap::new(), &mut HashMap::new(), 1, 1, 0).await;
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

    // -------------------------------------------------------------
    // Post-drop cooldown — suppress saturation respawn after a drop
    // -------------------------------------------------------------

    #[tokio::test]
    async fn cooldown_suppresses_saturation_respawn_after_drop() {
        // The flap scenario: seats=1, one pinned session keeps
        // replica A saturated forever; replica B is idle. With a
        // cooldown armed, after B is dropped the next ticks must
        // NOT respawn despite A staying saturated — until the
        // cooldown expires.
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: flappy
    display-name: Flappy
    container-image: nginx:alpine
    min-replicas: 1
    max-replicas: 3
"#,
            backend.clone(),
        );
        // A: saturated (1/1). B: idle (0/1). count=2, min=1.
        let busy = replica_with_sessions("flappy", 1, 1);
        let idle = replica_with_sessions("flappy", 0, 1);
        let _idle_id = idle.id.clone();
        {
            let mut reg = state.replicas.write().await;
            reg.add(busy);
            reg.add(idle);
        }

        let mut idle_ticks = HashMap::new();
        let mut sat = HashMap::new();
        let mut cooldown = HashMap::new();
        // idle grace = 1 (drop immediately), saturation grace = 1
        // (would respawn immediately if not for cooldown),
        // cooldown = 3 ticks.
        let (drop_after, sat_after, cd) = (1u32, 1u32, 3u32);

        // Tick 1: B is idle past grace → drop B → count=1, cooldown armed.
        tick(&state, &mut idle_ticks, &mut sat, &mut cooldown, drop_after, sat_after, cd).await;
        assert_eq!(state.replicas.read().await.replicas_of("flappy").len(), 1);
        assert_eq!(backend.spawns.load(Ordering::SeqCst), 0, "drop only, no spawn yet");
        assert!(cooldown.contains_key("flappy"), "cooldown armed after drop");

        // Ticks 2-4: A still saturated (1/1), count=1<max, but
        // cooldown suppresses the saturation spawn.
        for t in 0..3 {
            tick(&state, &mut idle_ticks, &mut sat, &mut cooldown, drop_after, sat_after, cd).await;
            assert_eq!(
                backend.spawns.load(Ordering::SeqCst),
                0,
                "no respawn during cooldown (sub-tick {t})"
            );
        }

        // Cooldown now expired → saturation spawn allowed again.
        tick(&state, &mut idle_ticks, &mut sat, &mut cooldown, drop_after, sat_after, cd).await;
        assert_eq!(
            backend.spawns.load(Ordering::SeqCst),
            1,
            "saturation respawn resumes after cooldown lifts"
        );
    }

    #[tokio::test]
    async fn cooldown_zero_disables_suppression() {
        // With cooldown=0, the old behavior holds: saturated
        // spec respawns right after a drop.
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: nocd
    display-name: NoCooldown
    container-image: nginx:alpine
    min-replicas: 1
    max-replicas: 3
"#,
            backend.clone(),
        );
        let busy = replica_with_sessions("nocd", 1, 1);
        let idle = replica_with_sessions("nocd", 0, 1);
        {
            let mut reg = state.replicas.write().await;
            reg.add(busy);
            reg.add(idle);
        }
        let mut idle_ticks = HashMap::new();
        let mut sat = HashMap::new();
        let mut cooldown = HashMap::new();

        // Tick 1: drop the idle one (count 2→1). cooldown=0 → not armed.
        tick(&state, &mut idle_ticks, &mut sat, &mut cooldown, 1, 1, 0).await;
        assert!(!cooldown.contains_key("nocd"));
        // Tick 2: A saturated, count=1<max → respawn immediately.
        tick(&state, &mut idle_ticks, &mut sat, &mut cooldown, 1, 1, 0).await;
        assert_eq!(backend.spawns.load(Ordering::SeqCst), 1);
    }

    // -------------------------------------------------------------
    // Hysteresis tests — drop only after N consecutive idle ticks
    // -------------------------------------------------------------

    #[test]
    fn update_idle_ticks_increments_for_idle_resets_for_active() {
        let mut counts: HashMap<ReplicaId, u32> = HashMap::new();
        let idle = replica_with_sessions("x", 0, 1);
        let busy = replica_with_sessions("x", 1, 1);
        let snap = vec![idle.clone(), busy.clone()];

        update_idle_ticks(&mut counts, &snap);
        assert_eq!(counts.get(&idle.id), Some(&1));
        assert!(!counts.contains_key(&busy.id), "active replicas have no entry");

        update_idle_ticks(&mut counts, &snap);
        assert_eq!(counts.get(&idle.id), Some(&2));

        // Idle replica goes active → counter resets (removed).
        let now_active = ruscker_core::Replica {
            sessions_active: 1,
            ..idle.clone()
        };
        update_idle_ticks(&mut counts, &[now_active]);
        assert!(!counts.contains_key(&idle.id), "active replicas drop the counter");
    }

    #[test]
    fn update_idle_ticks_gcs_disappeared_replicas() {
        let mut counts: HashMap<ReplicaId, u32> = HashMap::new();
        let gone = replica_with_sessions("x", 0, 1);
        counts.insert(gone.id.clone(), 42);
        // The new snapshot doesn't include `gone` → it must be
        // garbage-collected so we don't leak counters.
        update_idle_ticks(&mut counts, &[]);
        assert!(counts.is_empty());
    }

    #[tokio::test]
    async fn tick_holds_idle_replica_through_grace_then_drops() {
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: graceful
    display-name: Graceful
    container-image: nginx:alpine
    min-replicas: 1
    max-replicas: 5
"#,
            backend.clone(),
        );
        // Two replicas: one busy, one idle. min=1 so dropping the
        // idle one is allowed, but only after 3 consecutive idle
        // ticks under the chosen threshold.
        let busy = replica_with_sessions("graceful", 1, 1);
        let idle = replica_with_sessions("graceful", 0, 1);
        let idle_id = idle.id.clone();
        {
            let mut reg = state.replicas.write().await;
            reg.add(busy);
            reg.add(idle);
        }
        let mut counts: HashMap<ReplicaId, u32> = HashMap::new();
        let threshold = 3;

        // Tick 1 + 2: idle counter is 1, then 2 — both under
        // threshold, replica stays.
        tick(&state, &mut counts, &mut HashMap::new(), &mut HashMap::new(), threshold, 1, 0).await;
        assert_eq!(state.replicas.read().await.replicas_of("graceful").len(), 2);
        assert_eq!(counts.get(&idle_id), Some(&1));

        tick(&state, &mut counts, &mut HashMap::new(), &mut HashMap::new(), threshold, 1, 0).await;
        assert_eq!(state.replicas.read().await.replicas_of("graceful").len(), 2);
        assert_eq!(counts.get(&idle_id), Some(&2));

        // Tick 3: counter reaches 3 = threshold → drop.
        tick(&state, &mut counts, &mut HashMap::new(), &mut HashMap::new(), threshold, 1, 0).await;
        assert_eq!(state.replicas.read().await.replicas_of("graceful").len(), 1);
        assert!(!counts.contains_key(&idle_id), "dropped replica's counter GC'd");
    }

    // -------------------------------------------------------------
    // Saturation hysteresis — wait N saturated ticks before spawn
    // -------------------------------------------------------------

    #[tokio::test]
    async fn saturation_hysteresis_holds_first_observation_then_spawns() {
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
        // 1 saturated replica
        state
            .replicas
            .write()
            .await
            .add(replica_with_sessions("hot", 1, 1));
        let mut idle_counts = HashMap::new();
        let mut sat_counts = HashMap::new();
        let sat_threshold = 3;

        // Tick 1: saturated, counter=1, no spawn yet.
        tick(&state, &mut idle_counts, &mut sat_counts, &mut HashMap::new(), 1, sat_threshold, 0).await;
        assert_eq!(backend.spawns.load(Ordering::SeqCst), 0);
        assert_eq!(sat_counts.get("hot"), Some(&1));

        // Tick 2: saturated, counter=2, still no spawn.
        tick(&state, &mut idle_counts, &mut sat_counts, &mut HashMap::new(), 1, sat_threshold, 0).await;
        assert_eq!(backend.spawns.load(Ordering::SeqCst), 0);
        assert_eq!(sat_counts.get("hot"), Some(&2));

        // Tick 3: counter reaches 3 → spawn → counter resets.
        tick(&state, &mut idle_counts, &mut sat_counts, &mut HashMap::new(), 1, sat_threshold, 0).await;
        assert_eq!(backend.spawns.load(Ordering::SeqCst), 1);
        assert!(!sat_counts.contains_key("hot"), "counter resets after spawn");
        assert_eq!(state.replicas.read().await.replicas_of("hot").len(), 2);
    }

    #[tokio::test]
    async fn saturation_hysteresis_resets_when_saturation_lifts() {
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: bursty
    display-name: Bursty
    container-image: nginx:alpine
    min-replicas: 1
    max-replicas: 3
"#,
            backend.clone(),
        );
        let r = replica_with_sessions("bursty", 1, 1);
        let rid = r.id.clone();
        state.replicas.write().await.add(r);
        let mut idle_counts = HashMap::new();
        let mut sat_counts = HashMap::new();

        // Two saturated ticks (counter goes to 2).
        tick(&state, &mut idle_counts, &mut sat_counts, &mut HashMap::new(), 1, 3, 0).await;
        tick(&state, &mut idle_counts, &mut sat_counts, &mut HashMap::new(), 1, 3, 0).await;
        assert_eq!(sat_counts.get("bursty"), Some(&2));

        // Session drops — replica no longer saturated.
        state.replicas.write().await.dec_sessions(&rid);

        // Next tick: not saturated → counter cleared.
        tick(&state, &mut idle_counts, &mut sat_counts, &mut HashMap::new(), 1, 3, 0).await;
        assert!(
            !sat_counts.contains_key("bursty"),
            "counter clears the instant saturation lifts"
        );
        assert_eq!(
            backend.spawns.load(Ordering::SeqCst),
            0,
            "no spawn because the streak was interrupted"
        );

        // Session comes back — must wait the full grace again.
        state.replicas.write().await.inc_sessions(&rid);
        tick(&state, &mut idle_counts, &mut sat_counts, &mut HashMap::new(), 1, 3, 0).await;
        assert_eq!(sat_counts.get("bursty"), Some(&1));
        assert_eq!(backend.spawns.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn saturation_hysteresis_threshold_one_spawns_immediately() {
        // Threshold=1 preserves the pre-hysteresis behavior.
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: eager
    display-name: Eager
    container-image: nginx:alpine
    min-replicas: 1
    max-replicas: 3
"#,
            backend.clone(),
        );
        state
            .replicas
            .write()
            .await
            .add(replica_with_sessions("eager", 1, 1));
        let mut idle_counts = HashMap::new();
        let mut sat_counts = HashMap::new();

        tick(&state, &mut idle_counts, &mut sat_counts, &mut HashMap::new(), 1, 1, 0).await;
        assert_eq!(backend.spawns.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn tick_resets_grace_counter_when_replica_becomes_active() {
        // The flap-prevention scenario in miniature: an idle
        // replica builds up grace, then a request lands on it
        // and resets it. The replica must NOT be dropped on the
        // tick where the reset happens or the one right after.
        let backend = Arc::new(CountingBackend {
            spawns: AtomicU32::new(0),
        });
        // max=2 so saturation scale-up can't fire and confound
        // the grace-reset assertion — this test is about the
        // counter behavior, not about scale-up.
        let state = state_with_yaml(
            r#"
proxy:
  specs:
  - id: bouncy
    display-name: Bouncy
    container-image: nginx:alpine
    min-replicas: 1
    max-replicas: 2
"#,
            backend.clone(),
        );
        let busy = replica_with_sessions("bouncy", 1, 1);
        let idle = replica_with_sessions("bouncy", 0, 1);
        let idle_id = idle.id.clone();
        {
            let mut reg = state.replicas.write().await;
            reg.add(busy);
            reg.add(idle);
        }
        let mut counts: HashMap<ReplicaId, u32> = HashMap::new();
        let threshold = 3;

        // Two idle ticks accumulate grace=2.
        tick(&state, &mut counts, &mut HashMap::new(), &mut HashMap::new(), threshold, 1, 0).await;
        tick(&state, &mut counts, &mut HashMap::new(), &mut HashMap::new(), threshold, 1, 0).await;
        assert_eq!(counts.get(&idle_id), Some(&2));

        // Replica gets a session — reset to active.
        {
            let mut reg = state.replicas.write().await;
            reg.inc_sessions(&idle_id);
        }
        tick(&state, &mut counts, &mut HashMap::new(), &mut HashMap::new(), threshold, 1, 0).await;
        assert!(!counts.contains_key(&idle_id), "active replica's grace resets");
        assert_eq!(
            state.replicas.read().await.replicas_of("bouncy").len(),
            2,
            "active replica must not be dropped"
        );

        // Replica goes idle again — grace starts from 1, not 3.
        {
            let mut reg = state.replicas.write().await;
            reg.dec_sessions(&idle_id);
        }
        tick(&state, &mut counts, &mut HashMap::new(), &mut HashMap::new(), threshold, 1, 0).await;
        assert_eq!(
            counts.get(&idle_id),
            Some(&1),
            "grace counter starts fresh after activity"
        );
        assert_eq!(state.replicas.read().await.replicas_of("bouncy").len(), 2);
    }
}
