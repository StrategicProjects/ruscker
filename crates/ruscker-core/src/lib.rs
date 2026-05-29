//! # ruscker-core
//!
//! Domain logic for Ruscker. Defines the central abstractions that
//! `ruscker-proxy`, `ruscker-docker`, and `ruscker-admin` all depend on:
//!
//! - [`ContainerBackend`] — abstract interface over Docker, Kubernetes,
//!   or future runtimes
//! - [`SessionStore`] — abstract interface over in-memory, Redis, or
//!   Postgres session state
//! - [`RoutingDecision`] — how the proxy chooses a replica for a new
//!   session
//! - [`Replica`] — runtime representation of a running container
//!
//! Nothing in this crate does I/O directly — implementations live in
//! sibling crates. This keeps the domain pure and testable.
//!
//! ## What's implemented in MVP
//!
//! The MVP scope is small: types and traits only. Production-ready
//! implementations come in Phase 3 (proxy + docker + lifecycle).

#![allow(dead_code)]

pub mod replica;
pub mod routing;

pub use replica::{Replica, ReplicaId, ReplicaState};
pub use routing::{RoutingDecision, Router};

use async_trait::async_trait;
use futures_util::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use thiserror::Error;

/// A boxed, owned stream of log lines. Boxed (rather than an
/// `impl Stream` associated type) so it can travel through the
/// `dyn ContainerBackend` trait object the proxy/admin hold.
pub type LogStream = Pin<Box<dyn Stream<Item = String> + Send>>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("spec {0} not found in registry")]
    SpecNotFound(String),

    #[error("no available replicas for spec {0} (at capacity)")]
    AtCapacity(String),

    #[error("backend error: {0}")]
    Backend(String),

    #[error("session {0} not found or expired")]
    SessionExpired(String),
}

pub type CoreResult<T> = Result<T, CoreError>;

/// Backend-neutral registry credentials. Lives in `ruscker-core`
/// so the trait surface doesn't drag a bollard dependency into
/// callers. Each backend converts to its own native type at the
/// pull boundary.
///
/// `server_address` is the registry hostname (`registry.example.com`,
/// `ghcr.io`, etc.) — leave it `None` for Docker Hub. The username
/// and password are required when present; partial credentials
/// (just a username, just a password) make no sense and the
/// resolver should produce `None` instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryCredentials {
    pub username: String,
    pub password: String,
    pub server_address: Option<String>,
}

/// Backend-neutral container resource limits. The local Docker
/// backend translates these into `HostConfig.memory`,
/// `memory_reservation`, `cpu_period`, `cpu_quota`. Backends
/// that don't natively support a field (e.g. CPU requests on
/// Docker) silently ignore it.
///
/// `cpu_fraction` is fractional CPUs — `0.5` = half a core,
/// `2.0` = two cores. Matches Docker's `--cpus` semantics.
/// The Docker backend converts to a `cpu_period` of 100ms and a
/// proportional `cpu_quota`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ResourceLimits {
    /// Hard memory cap in bytes. Container is OOM-killed if it
    /// exceeds. Maps to Docker `HostConfig.memory`.
    pub memory_bytes: Option<i64>,

    /// Soft memory request (Docker `memory_reservation`). The
    /// container can briefly exceed this; OOM only when the
    /// hard limit is hit.
    pub memory_reservation_bytes: Option<i64>,

    /// CPU hard limit as a fraction of one CPU. `0.5` allows the
    /// container to use up to half a core; `4.0` allows up to
    /// four cores.
    pub cpu_fraction: Option<f64>,
}

impl ResourceLimits {
    /// Are any fields set? An all-`None` `ResourceLimits` is
    /// equivalent to "no limits" and backends should skip the
    /// translation entirely to keep the bollard `HostConfig`
    /// minimal.
    pub fn is_empty(&self) -> bool {
        self.memory_bytes.is_none()
            && self.memory_reservation_bytes.is_none()
            && self.cpu_fraction.is_none()
    }
}

/// All the parameters needed to spawn one replica. Bundled so
/// the trait surface doesn't keep growing one method per new
/// optional knob. New backend features go in here; the trait
/// method stays one.
#[derive(Debug, Clone)]
pub struct SpawnRequest {
    pub spec_id: String,
    pub image: String,
    /// `None` → backend uses its default port (e.g. 3838 for
    /// Shiny on the local Docker backend).
    pub inner_port: Option<u16>,
    /// `None` → anonymous pull.
    pub creds: Option<RegistryCredentials>,
    /// All-`None` (or omitted via `Default`) → no limits applied.
    pub limits: ResourceLimits,
    /// Bind-mount specs in Docker syntax (`"/host:/container[:ro]"`).
    /// Empty → no binds. The local backend maps these to
    /// `HostConfig.binds`.
    pub volumes: Vec<String>,
    /// Multi-host placement strategy for this spec (Phase 6). Ignored
    /// by the single-host backend; the `MultiHostDockerBackend` uses it
    /// to choose which host to spawn on.
    pub placement: ruscker_config::Placement,
    /// Prefer distinct hosts for this spec's replicas (anti-affinity).
    pub anti_affinity: bool,
    /// Docker `--platform` target, e.g. `"linux/amd64"`. Forwarded to
    /// the daemon on both pull and create so an operator on arm64 can
    /// run an amd64-only image via the daemon's emulation
    /// (QEMU / Rosetta). `None` ⇒ the daemon picks per the manifest.
    pub platform: Option<String>,

    /// Environment variables for the container as Docker `NAME=value`
    /// strings (from the spec's `container-env`). Empty ⇒ none injected;
    /// the local backend maps these to `Config.Env`.
    pub env: Vec<String>,

    /// Command override (the spec's `container-cmd`), as an argv list.
    /// `None` ⇒ the image's baked `CMD` is used; the local backend maps
    /// this to `Config.Cmd`.
    pub cmd: Option<Vec<String>>,
}

impl SpawnRequest {
    pub fn new(spec_id: impl Into<String>, image: impl Into<String>) -> Self {
        Self {
            spec_id: spec_id.into(),
            image: image.into(),
            inner_port: None,
            creds: None,
            limits: ResourceLimits::default(),
            volumes: Vec::new(),
            placement: ruscker_config::Placement::default(),
            anti_affinity: false,
            platform: None,
            env: Vec::new(),
            cmd: None,
        }
    }

    pub fn with_platform(mut self, platform: impl Into<String>) -> Self {
        self.platform = Some(platform.into());
        self
    }

    pub fn with_placement(mut self, placement: ruscker_config::Placement) -> Self {
        self.placement = placement;
        self
    }
    pub fn with_anti_affinity(mut self, anti_affinity: bool) -> Self {
        self.anti_affinity = anti_affinity;
        self
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.inner_port = Some(port);
        self
    }
    pub fn with_creds(mut self, creds: RegistryCredentials) -> Self {
        self.creds = Some(creds);
        self
    }
    pub fn with_limits(mut self, limits: ResourceLimits) -> Self {
        self.limits = limits;
        self
    }
    pub fn with_volumes(mut self, volumes: Vec<String>) -> Self {
        self.volumes = volumes;
        self
    }
    pub fn with_env(mut self, env: Vec<String>) -> Self {
        self.env = env;
        self
    }
    pub fn with_cmd(mut self, cmd: Vec<String>) -> Self {
        self.cmd = Some(cmd);
        self
    }
}

/// Abstract container backend. The default implementation is Docker
/// (via bollard) in `ruscker-docker`. Future backends could include
/// Kubernetes, Docker Swarm, or a multi-host scheduler.
///
/// Implementations must be `Send + Sync` so they can be shared across
/// async tasks. The `async_trait` macro is used because Rust stable
/// doesn't yet have native async fn in traits in all positions.
#[async_trait]
pub trait ContainerBackend: Send + Sync {
    /// Start a new container instance for the given spec, with a
    /// best-guess inner port. Implementations default to 3838
    /// (Shiny Server). Callers that know the spec's inner port
    /// (e.g. an API with `api.port`) should prefer
    /// [`Self::spawn_with_port`].
    async fn spawn(&self, spec_id: &str, image: &str) -> CoreResult<Replica>;

    /// Start a container with an explicit inner port. Default impl
    /// falls back to `spawn` — backends without per-port support
    /// (none today) won't break, but Phase 3 features like API
    /// routing depend on the override.
    async fn spawn_with_port(
        &self,
        spec_id: &str,
        image: &str,
        _inner_port: u16,
    ) -> CoreResult<Replica> {
        self.spawn(spec_id, image).await
    }

    /// Start a container with an explicit inner port AND optional
    /// registry credentials for pulling private images. Default
    /// impl drops the credentials and falls back to
    /// [`Self::spawn_with_port`] so backends that don't support
    /// private registries (or haven't been updated yet) keep
    /// working for public images.
    async fn spawn_with_port_and_creds(
        &self,
        spec_id: &str,
        image: &str,
        inner_port: u16,
        _creds: Option<&RegistryCredentials>,
    ) -> CoreResult<Replica> {
        self.spawn_with_port(spec_id, image, inner_port).await
    }

    /// Start a container from a fully-described [`SpawnRequest`]:
    /// port, credentials, resource limits. **This is the
    /// preferred entry point** — the older `spawn*` methods are
    /// thin wrappers retained for back-compat with mock
    /// backends and the test suite.
    ///
    /// Default impl ignores `limits` and falls back to
    /// `spawn_with_port_and_creds` (or further fallbacks) so a
    /// backend that doesn't override this still works for
    /// public unlimited pulls.
    async fn spawn_request(&self, req: &SpawnRequest) -> CoreResult<Replica> {
        match req.inner_port {
            Some(port) => {
                self.spawn_with_port_and_creds(
                    &req.spec_id,
                    &req.image,
                    port,
                    req.creds.as_ref(),
                )
                .await
            }
            None => self.spawn(&req.spec_id, &req.image).await,
        }
    }

    /// Gracefully stop a replica. The implementation should:
    /// 1. Mark it as draining (stop accepting new sessions)
    /// 2. Wait up to `drain_timeout` seconds for sessions to end
    /// 3. Send SIGTERM
    /// 4. If still alive after grace period, SIGKILL
    async fn stop(&self, replica_id: &ReplicaId) -> CoreResult<()>;

    /// Snapshot current state for all known replicas. Used by the
    /// monitoring dashboard.
    async fn list(&self) -> CoreResult<Vec<Replica>>;

    /// Per-replica metrics (CPU, memory, network I/O).
    async fn metrics(&self, replica_id: &ReplicaId) -> CoreResult<ReplicaMetrics>;

    /// Fetch the last `tail` lines of a replica's combined
    /// stdout+stderr. A one-shot snapshot (no follow) — the
    /// dashboard logs page uses it for "why did this container
    /// crash" debugging. Default impl returns an empty vec so
    /// backends that don't support log retrieval don't break
    /// the caller.
    async fn logs(&self, _replica_id: &ReplicaId, _tail: usize) -> CoreResult<Vec<String>> {
        Ok(Vec::new())
    }

    /// Follow a replica's combined stdout+stderr as a live
    /// stream of lines, seeded with the last `tail` lines. The
    /// stream stays open until the container stops (then it
    /// ends) or the consumer drops it. Used by the dashboard's
    /// live-logs SSE endpoint. Default impl returns an empty
    /// stream so non-streaming backends don't break callers.
    async fn logs_follow(&self, _replica_id: &ReplicaId, _tail: usize) -> CoreResult<LogStream> {
        Ok(Box::pin(futures_util::stream::empty()))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ReplicaMetrics {
    pub cpu_percent: f64,
    pub memory_bytes: u64,
    pub network_rx_bytes: u64,
    pub network_tx_bytes: u64,
}

/// Live registry of all running replicas, keyed by spec ID.
#[derive(Debug, Default)]
pub struct ReplicaRegistry {
    by_spec: HashMap<String, Vec<Replica>>,
}

impl ReplicaRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn replicas_of(&self, spec_id: &str) -> &[Replica] {
        self.by_spec
            .get(spec_id)
            .map(|v| v.as_slice())
            .unwrap_or(&[])
    }

    /// Insert a freshly-spawned replica into the pool for its
    /// spec. Used by the proxy on every successful
    /// `ContainerBackend::spawn`.
    pub fn add(&mut self, replica: Replica) {
        self.by_spec
            .entry(replica.spec_id.clone())
            .or_default()
            .push(replica);
    }

    /// Remove a replica by id. Returns the removed value if found.
    /// Used by stop() and by reconciliation against
    /// `ContainerBackend::list()` on startup.
    pub fn remove(&mut self, replica_id: &ReplicaId) -> Option<Replica> {
        for replicas in self.by_spec.values_mut() {
            if let Some(pos) = replicas.iter().position(|r| r.id == *replica_id) {
                return Some(replicas.remove(pos));
            }
        }
        None
    }

    /// Replace the registry contents with `replicas`, grouped by
    /// spec id. Used at startup to reconcile with whatever the
    /// container backend reports as already running.
    pub fn reset(&mut self, replicas: Vec<Replica>) {
        self.by_spec.clear();
        for r in replicas {
            self.add(r);
        }
    }

    /// Count of distinct specs with at least one replica.
    pub fn spec_count(&self) -> usize {
        self.by_spec.len()
    }

    /// Total replicas across all specs.
    pub fn total(&self) -> usize {
        self.by_spec.values().map(Vec::len).sum()
    }

    /// Flat iterator over every replica regardless of spec. The
    /// dashboard and metrics cache use this for whole-registry
    /// walks; callers that need per-spec lookups should keep
    /// using `replicas_of`.
    pub fn all(&self) -> impl Iterator<Item = &Replica> {
        self.by_spec.values().flat_map(|v| v.iter())
    }

    /// Increment the active-session counter on a replica. Called
    /// by the session tracker when a new visitor lands. Silently
    /// no-ops if the replica is gone (race with stop/remove).
    pub fn inc_sessions(&mut self, replica_id: &ReplicaId) {
        if let Some(r) = self.find_mut(replica_id) {
            r.sessions_active = r.sessions_active.saturating_add(1);
        }
    }

    /// Decrement the active-session counter on a replica. Used
    /// by the sweeper on idle-expiry. Saturating-subtraction so
    /// a stray double-dec can't underflow.
    pub fn dec_sessions(&mut self, replica_id: &ReplicaId) {
        if let Some(r) = self.find_mut(replica_id) {
            r.sessions_active = r.sessions_active.saturating_sub(1);
        }
    }

    /// Set one replica's active-session count to an authoritative
    /// value (e.g. a fresh `count(*) WHERE replica_id = …` read after a
    /// session was registered). Unlike [`inc_sessions`], this writes an
    /// absolute count read from committed shared state, so it converges
    /// to the same truth the periodic [`set_session_counts`] reconcile
    /// uses — the two can interleave without a blind `+1` being lost.
    /// No-ops if the replica is gone. Used by the Postgres store on the
    /// register path; the in-memory store doesn't need it.
    pub fn set_session_count(&mut self, replica_id: &ReplicaId, n: u32) {
        if let Some(r) = self.find_mut(replica_id) {
            r.sessions_active = n;
        }
    }

    /// Overwrite every replica's active-session count from an
    /// authoritative external tally. Replicas absent from `counts`
    /// are reset to zero.
    ///
    /// This is the HA reconcile path: the Postgres session store owns
    /// the cluster-wide session table, and its sweep periodically
    /// pushes the per-replica totals here so this node's routing /
    /// scaling math reflects sessions opened by sibling nodes too. The
    /// single-node `InMemorySessionStore` never calls this — it owns
    /// the counter directly via `inc_sessions` / `dec_sessions`.
    pub fn set_session_counts(
        &mut self,
        counts: &std::collections::HashMap<ReplicaId, u32>,
    ) {
        for replicas in self.by_spec.values_mut() {
            for r in replicas.iter_mut() {
                r.sessions_active = counts.get(&r.id).copied().unwrap_or(0);
            }
        }
    }

    fn find_mut(&mut self, replica_id: &ReplicaId) -> Option<&mut Replica> {
        for replicas in self.by_spec.values_mut() {
            if let Some(r) = replicas.iter_mut().find(|r| r.id == *replica_id) {
                return Some(r);
            }
        }
        None
    }
}

#[cfg(test)]
mod registry_tests {
    use super::*;
    use crate::replica::ReplicaState;
    use std::net::SocketAddr;

    fn fake_replica(spec: &str) -> Replica {
        Replica {
            id: ReplicaId(uuid::Uuid::new_v4()),
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

    #[test]
    fn spawn_request_carries_env_and_cmd() {
        let req = SpawnRequest::new("nb", "jupyter")
            .with_env(vec!["JUPYTER_TOKEN=".into(), "GRANT_SUDO=yes".into()])
            .with_cmd(vec!["start-notebook.sh".into()]);
        assert_eq!(req.env, vec!["JUPYTER_TOKEN=", "GRANT_SUDO=yes"]);
        assert_eq!(req.cmd.as_deref(), Some(&["start-notebook.sh".to_string()][..]));

        // Defaults: no env, no cmd override (image's baked CMD wins).
        let bare = SpawnRequest::new("a", "img");
        assert!(bare.env.is_empty());
        assert!(bare.cmd.is_none());
    }

    #[test]
    fn inc_and_dec_round_trip() {
        let mut reg = ReplicaRegistry::new();
        let r = fake_replica("x");
        let id = r.id.clone();
        reg.add(r);
        reg.inc_sessions(&id);
        reg.inc_sessions(&id);
        assert_eq!(reg.replicas_of("x")[0].sessions_active, 2);
        reg.dec_sessions(&id);
        assert_eq!(reg.replicas_of("x")[0].sessions_active, 1);
    }

    #[test]
    fn dec_saturates_at_zero() {
        let mut reg = ReplicaRegistry::new();
        let r = fake_replica("x");
        let id = r.id.clone();
        reg.add(r);
        reg.dec_sessions(&id); // already zero, must not underflow
        reg.dec_sessions(&id);
        assert_eq!(reg.replicas_of("x")[0].sessions_active, 0);
    }

    #[test]
    fn inc_on_unknown_id_is_noop() {
        let mut reg = ReplicaRegistry::new();
        let unknown = ReplicaId(uuid::Uuid::new_v4());
        reg.inc_sessions(&unknown); // must not panic
    }
}
