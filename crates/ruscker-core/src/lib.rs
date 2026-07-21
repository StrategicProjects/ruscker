//! # ruscker-core
//!
//! Domain logic for Ruscker. Defines the central abstractions that
//! `ruscker-proxy`, `ruscker-docker`, and `ruscker-admin` all depend on:
//!
//! - [`ContainerBackend`] — abstract interface over Docker, Kubernetes,
//!   or future runtimes
//! - [`SessionStore`] — abstract interface over in-memory, Redis, or
//!   Postgres session state
//! - [`Replica`] — runtime representation of a running container
//!
//! Nothing in this crate does I/O directly — implementations live in
//! sibling crates. This keeps the domain pure and testable.
//!
//! ## Where routing lives
//!
//! The replica-picking logic (`pick_replica` / `pick_accepting`) lives
//! in `ruscker-admin::routes::proxy`, next to the seat accounting it
//! depends on. An early `Router`/`RoutingDecision` abstraction here had
//! no callers and had already drifted from the real implementation, so
//! it was removed (#743) rather than left as a trap.

pub mod replica;

pub use replica::{Replica, ReplicaId, ReplicaState};

use async_trait::async_trait;
use futures_util::Stream;
use std::collections::HashMap;
use std::pin::Pin;
use thiserror::Error;

/// A boxed, owned stream of log lines. Boxed (rather than an
/// `impl Stream` associated type) so it can travel through the
/// `dyn ContainerBackend` trait object the proxy/admin hold.
pub type LogStream = Pin<Box<dyn Stream<Item = String> + Send>>;

/// The lifecycle transition a [`ContainerEvent`] reports. Only the
/// transitions the runtime reacts to are named; everything else is
/// [`Other`](ContainerEventKind::Other) and simply nudges a reconcile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContainerEventKind {
    /// The container started (e.g. the second half of a `docker restart`).
    Start,
    /// The container's main process exited.
    Die,
    /// The container was stopped.
    Stop,
    /// The container was removed (`docker rm`).
    Destroy,
    /// Any other container action — still worth a reconcile, but not
    /// individually interesting.
    Other,
}

/// A single Docker lifecycle event for a Ruscker-managed container. The
/// consumer (the admin events watcher, #1018 slice B) treats it mainly as a
/// "something changed, reconcile now" nudge and lets the authoritative
/// [`ContainerBackend::replica_liveness`] pass decide the actual action, so a
/// missed or duplicated event is always safe.
#[derive(Debug, Clone)]
pub struct ContainerEvent {
    pub kind: ContainerEventKind,
    pub container_id: String,
    pub spec_id: Option<String>,
    pub replica_id: Option<String>,
}

/// A boxed, owned stream of [`ContainerEvent`]s. Boxed for the same reason as
/// [`LogStream`] — it travels through the `dyn ContainerBackend` trait object.
pub type ContainerEventStream = Pin<Box<dyn Stream<Item = ContainerEvent> + Send>>;

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
/// resolver should produce `None` instead. `credential_name` is optional
/// non-secret provenance for diagnostics: stored credentials carry their
/// admin-library name, while inline YAML credentials leave it unset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryCredentials {
    pub username: String,
    pub password: String,
    pub server_address: Option<String>,
    pub credential_name: Option<String>,
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

    /// Docker network to attach the container to (the spec's
    /// `container-network`). `None` ⇒ the daemon's default bridge. The
    /// local backend sets `HostConfig.network_mode` and creates the
    /// network (a user-defined bridge) if it's missing.
    pub network: Option<String>,

    /// Extra Docker labels to stamp on the container (the spec's
    /// `labels`), as `(key, value)` pairs. The backend merges these onto
    /// the container's labels; its own `ruscker.*` labels win on a key
    /// collision. Empty ⇒ only the internal labels.
    pub labels: Vec<(String, String)>,

    /// Per-spawn readiness budget in milliseconds. `None` (and zero via
    /// [`Self::with_readiness_timeout_ms`]) means the backend's configured
    /// default, normally `proxy.container-wait-time`. Regular replica
    /// spawns consume it; run-to-completion jobs ignore it.
    pub readiness_timeout_ms: Option<u64>,

    /// Wall-clock cap, in seconds, for a run-to-completion job (#986
    /// slice C). Only [`ContainerBackend::run_job`] consumes it — a
    /// regular replica spawn ignores the field entirely. `None` ⇒ the
    /// backend's default cap (1 h on the local Docker backend).
    pub job_timeout_secs: Option<u64>,
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
            network: None,
            labels: Vec::new(),
            readiness_timeout_ms: None,
            job_timeout_secs: None,
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

    pub fn with_network(mut self, network: impl Into<String>) -> Self {
        self.network = Some(network.into());
        self
    }

    pub fn with_labels(mut self, labels: Vec<(String, String)>) -> Self {
        self.labels = labels;
        self
    }

    /// Override the readiness budget for this spawn. Zero is normalized to
    /// `None`, preserving the configuration contract that `0` inherits the
    /// backend/global default rather than failing the spawn immediately.
    pub fn with_readiness_timeout_ms(mut self, millis: u64) -> Self {
        self.readiness_timeout_ms = (millis > 0).then_some(millis);
        self
    }

    /// Cap a run-to-completion job at `secs` seconds (see
    /// [`SpawnRequest::job_timeout_secs`]). No-op for replica spawns.
    pub fn with_job_timeout(mut self, secs: u64) -> Self {
        self.job_timeout_secs = Some(secs);
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

    /// Like [`metrics`](Self::metrics) but the caller passes the
    /// `container_id` it already holds (from the registry), so the
    /// backend can skip resolving the replica → container mapping.
    /// The default ignores the hint and delegates to `metrics`; the
    /// local Docker backend overrides it to avoid a per-call
    /// `list_containers` on every metrics refresh (#282).
    async fn metrics_for(
        &self,
        replica_id: &ReplicaId,
        _container_id: &str,
    ) -> CoreResult<ReplicaMetrics> {
        self.metrics(replica_id).await
    }

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

    /// A live stream of Docker lifecycle events (start/die/stop/destroy) for
    /// Ruscker-managed containers, so the runtime can reconcile within ~1 s of
    /// an external `docker rm -f` / `docker restart` instead of waiting out the
    /// periodic scaler tick (#1018 slice B).
    ///
    /// Default returns an empty stream: a backend without event support (mocks,
    /// future backends) simply relies on the periodic reconcile — events are a
    /// latency optimization, never the source of truth. The stream ends when
    /// the daemon connection drops; the consumer reopens it, and the periodic
    /// reconcile remains the fallback for anything missed during a gap.
    async fn container_events(&self) -> CoreResult<ContainerEventStream> {
        Ok(Box::pin(futures_util::stream::empty()))
    }

    /// Creation/publish timestamp of an image as an RFC3339 string, if
    /// the backend can read it (Docker: `inspect_image().created`).
    /// Used to stamp a card's "updated" date from the image it runs
    /// (#375). Default `None` so non-Docker backends don't break the
    /// caller — it just leaves the date unset.
    async fn image_created(&self, _image: &str) -> Option<String> {
        None
    }

    // ── Disk management (#453 part B) ───────────────────────────────
    // The admin "Disk" panel reclaims space left behind by stopped
    // replicas and unused images. All default to "nothing" so non-Docker
    // backends (and mocks) keep compiling.

    /// Every container Ruscker manages, **including stopped/exited**
    /// ones — scoped to the `ruscker.replica_id` label, so it never
    /// reports a non-Ruscker container. Unlike [`Self::list`] this does
    /// not require a live port binding, so crashed/exited replicas show
    /// up (they're exactly what the disk panel reclaims).
    async fn list_managed_containers(&self) -> CoreResult<Vec<ManagedContainer>> {
        Ok(Vec::new())
    }

    /// Reconcile a caller's in-memory replicas against one authoritative
    /// backend inventory. Besides classifying every known replica, Docker
    /// backends return the running, Ruscker-labelled replicas they found so
    /// the runtime can re-adopt a container that came back after an external
    /// restart.
    ///
    /// The default is deliberately fail-safe: a backend that has not opted
    /// into authoritative inventory reports every replica as [`Unknown`] and
    /// discovers nothing. An empty/default backend must never make the scaler
    /// forget a live container.
    async fn replica_liveness(
        &self,
        known: &[ReplicaLivenessQuery],
    ) -> CoreResult<ReplicaLivenessReport> {
        Ok(ReplicaLivenessReport::unknown(known))
    }

    /// Wait until a running replica's upstream is actually serving HTTP.
    /// Re-adoption uses the same backend-specific readiness check as spawn;
    /// the default fails closed so a backend cannot accidentally publish a
    /// just-restarted container as Ready without checking it.
    async fn wait_until_ready(&self, _replica: &Replica) -> CoreResult<()> {
        Err(CoreError::Backend(
            "readiness checks are not supported by this backend".into(),
        ))
    }

    /// Wait for a running replica using an optional per-spec readiness
    /// budget. The default delegates to [`Self::wait_until_ready`] so
    /// existing/mock backends remain source-compatible and may ignore the
    /// override. Backends that support per-spawn readiness should override
    /// this method.
    async fn wait_until_ready_with_timeout(
        &self,
        replica: &Replica,
        _readiness_timeout_ms: Option<u64>,
    ) -> CoreResult<()> {
        self.wait_until_ready(replica).await
    }

    /// Image refs (name/tag AND sha id) of **every** container on the
    /// host — not just Ruscker-managed ones (#871). The disk panel uses
    /// this so an image backing a non-Ruscker container (e.g. ShinyProxy)
    /// is never flagged "unused" or removed.
    async fn all_container_image_refs(&self) -> CoreResult<Vec<String>> {
        Ok(Vec::new())
    }

    /// Force-remove a single container by id (stops it first if running).
    /// MUST refuse a container that isn't Ruscker-managed (#871) — the
    /// caller passes an operator-supplied id, so the label check is the
    /// backstop against removing a non-Ruscker container on a shared host.
    async fn remove_container(&self, _container_id: &str) -> CoreResult<()> {
        Ok(())
    }

    /// Remove every **stopped** Ruscker-managed container (label-scoped,
    /// never touches running replicas or non-Ruscker containers). Returns
    /// how many were removed.
    async fn prune_stopped(&self) -> CoreResult<usize> {
        Ok(0)
    }

    /// The backend's daemon version string for the admin **System** tab
    /// (#766) — e.g. Docker `"28.5.2"`. `None` ⇒ not a container backend
    /// or the version couldn't be read (a diagnostic nicety, never fatal).
    async fn backend_version(&self) -> CoreResult<Option<String>> {
        Ok(None)
    }

    /// "Reclaim space" for the disk panel (#766 follow-up): prune
    /// **dangling** images (untagged orphan layers) + the build cache.
    /// Deliberately host-SAFE — it never removes a tagged image or any
    /// container (Ruscker or not), unlike a full `docker system prune`.
    /// Returns the number of bytes reclaimed.
    async fn reclaim_space(&self) -> CoreResult<u64> {
        Ok(0)
    }

    /// Local images, for the disk panel's "what's eating space" view.
    /// Each carries its size and how many containers reference it (so the
    /// UI can flag "in use" and only offer to remove unused ones).
    async fn list_images(&self) -> CoreResult<Vec<ImageInfo>> {
        Ok(Vec::new())
    }

    /// Remove an image by id or `repo:tag`. Best left to images no
    /// container uses — the Docker daemon refuses an in-use image unless
    /// forced, and this never forces.
    async fn remove_image(&self, _image: &str) -> CoreResult<()> {
        Ok(())
    }

    /// Run a container to COMPLETION (#986 slice A) — the primitive
    /// behind scheduled jobs (ETL, reports): same image/env/volumes/
    /// creds model as a spawn (the request type is [`SpawnRequest`];
    /// `inner_port`/`placement` are ignored — a job publishes nothing),
    /// but the container runs, exits, has its exit code + log tail
    /// captured, and is removed. A NON-ZERO exit is a valid
    /// [`JobOutcome`], not an `Err` — errors mean the job could not be
    /// run at all (pull/create/start failure, timeout). Defaults to an
    /// error so backends without job support fail closed.
    async fn run_job(&self, _req: &SpawnRequest) -> CoreResult<JobOutcome> {
        Err(CoreError::Backend(
            "run-to-completion jobs are not supported by this backend".into(),
        ))
    }

    /// Named Docker volumes on the host, with how many containers (ANY
    /// container, not just Ruscker's) reference each — the disk panel
    /// only offers to remove unreferenced ones (#987). Defaults to an
    /// error, NOT an empty list: a backend that doesn't implement
    /// volumes must render as "unavailable", never as "no volumes"
    /// (the fail-closed rule from #889).
    async fn list_volumes(&self) -> CoreResult<Vec<VolumeInfo>> {
        Err(CoreError::Backend(
            "volume listing is not supported by this backend".into(),
        ))
    }

    /// Create a named volume (labelled as Ruscker-created).
    async fn create_volume(&self, _name: &str) -> CoreResult<()> {
        Err(CoreError::Backend(
            "volume creation is not supported by this backend".into(),
        ))
    }

    /// Remove a named volume. Never forced — the daemon refuses an
    /// in-use volume, which is the final backstop under the panel's
    /// own zero-references check.
    async fn remove_volume(&self, _name: &str) -> CoreResult<()> {
        Err(CoreError::Backend(
            "volume removal is not supported by this backend".into(),
        ))
    }

    /// Whether `image` is already present on this backend's host — a fast,
    /// pull-free presence check for the spec editor's "image on server"
    /// indicator (#498). `Ok(false)` means "not found" (not an error).
    ///
    /// The default matches by tag over [`list_images`](Self::list_images)
    /// — handling the bare-`repo` → `repo:latest` default — so mocks and
    /// the multihost router still answer. The Docker backend overrides it
    /// with a direct `inspect`, which resolves tags natively.
    async fn image_present(&self, image: &str) -> CoreResult<bool> {
        Ok(self
            .list_images()
            .await?
            .iter()
            .any(|i| image_tag_matches(&i.tags, image)))
    }

    /// Pull `image` (optionally with registry `creds` + a `platform`),
    /// streaming human-readable progress lines for the editor's Pull
    /// button (#498, slice B). Default: unsupported — the editor only
    /// offers Pull when the backend provides it. The Docker backend
    /// streams `create_image` events.
    async fn pull_image(
        &self,
        _image: &str,
        _creds: Option<&RegistryCredentials>,
        _platform: Option<&str>,
    ) -> CoreResult<LogStream> {
        Err(CoreError::Backend(
            "image pull not supported by this backend".into(),
        ))
    }
}

/// Does any of `tags` (an image's `repo:tag` refs) satisfy a request for
/// `image`? A bare `repo` matches `repo:latest`, mirroring Docker's
/// default-tag rule. Pure helper behind the default
/// [`ContainerBackend::image_present`]; the Docker backend resolves tags
/// via the daemon instead.
fn image_tag_matches(tags: &[String], image: &str) -> bool {
    let with_tag = if image.contains(':') {
        image.to_string()
    } else {
        format!("{image}:latest")
    };
    tags.iter()
        .any(|t| t.as_str() == image || t.as_str() == with_tag)
}

/// A container Ruscker manages, in any lifecycle state — the row model
/// for the admin disk panel (#453). Lighter than [`Replica`]: no
/// upstream/seat accounting, but it carries the human status string and
/// whether it's still running, which the panel needs to decide what's
/// reclaimable.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ManagedContainer {
    /// Full container id.
    pub id: String,
    /// Display name (without Docker's leading `/`).
    pub name: String,
    /// Image reference the container runs.
    pub image: String,
    /// The `ruscker.spec_id` label, if present.
    pub spec_id: Option<String>,
    /// Human status, e.g. `Up 3 minutes` / `Exited (0) 1 hour ago`.
    pub status: String,
    /// Whether the container is currently running.
    pub running: bool,
    /// When Docker last observed the container exit. `None` for running
    /// containers or when the backend cannot determine it safely.
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// What an authoritative backend inventory says about one known replica.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplicaLiveness {
    Running,
    Stopped,
    Missing,
    Unknown,
}

/// The identity and host affinity a backend needs to classify a replica.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaLivenessQuery {
    pub replica_id: ReplicaId,
    pub container_id: String,
    pub host: Option<String>,
}

impl From<&Replica> for ReplicaLivenessQuery {
    fn from(replica: &Replica) -> Self {
        Self {
            replica_id: replica.id.clone(),
            container_id: replica.container_id.clone(),
            host: replica.host.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplicaLivenessObservation {
    pub replica_id: ReplicaId,
    pub liveness: ReplicaLiveness,
}

/// One runtime inventory pass: classifications for the caller's known
/// replicas plus running labelled containers available for re-adoption.
#[derive(Debug, Clone)]
pub struct ReplicaLivenessReport {
    pub observations: Vec<ReplicaLivenessObservation>,
    pub running: Vec<Replica>,
    /// All Ruscker-managed containers in the same authoritative inventory,
    /// including stopped ones that may no longer exist in the registry.
    pub managed: Vec<ManagedContainer>,
}

impl ReplicaLivenessReport {
    pub fn unknown(known: &[ReplicaLivenessQuery]) -> Self {
        Self {
            observations: known
                .iter()
                .map(|query| ReplicaLivenessObservation {
                    replica_id: query.replica_id.clone(),
                    liveness: ReplicaLiveness::Unknown,
                })
                .collect(),
            running: Vec::new(),
            managed: Vec::new(),
        }
    }
}

/// The result of a run-to-completion job (#986). A captured non-zero
/// exit code is a *reported failure*, distinct from the backend being
/// unable to run the job at all (which is a `CoreResult::Err`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct JobOutcome {
    /// The container's exit code (0 = success).
    pub exit_code: i64,
    /// Trailing log lines (stdout+stderr interleaved), for the run
    /// history and failure alerts.
    pub log_tail: Vec<String>,
    /// Wall-clock duration of the run, in milliseconds.
    pub duration_ms: u64,
}

/// A named Docker volume, for the disk panel's volumes card (#987).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct VolumeInfo {
    /// Volume name (the removal handle).
    pub name: String,
    /// Volume driver (usually `local`).
    pub driver: String,
    /// Creation timestamp as reported by the daemon (RFC 3339), when known.
    pub created_at: Option<String>,
    /// Containers (running or stopped, ANY owner) mounting this volume.
    pub refs: i64,
    /// Whether the volume carries the `ruscker.created` label (made
    /// from the admin panel, as opposed to a neighbour's volume).
    pub ruscker_created: bool,
}

/// A local image, for the disk panel. `containers` mirrors Docker's
/// "how many containers reference this image" (`-1` when the daemon
/// can't tell) — the panel treats `> 0` as in-use.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ImageInfo {
    /// Image id (`sha256:…`).
    pub id: String,
    /// `repo:tag` references; empty for a dangling image.
    pub tags: Vec<String>,
    /// On-disk size in bytes (includes shared layers).
    pub size_bytes: i64,
    /// Number of containers using the image (`-1` if unknown).
    pub containers: i64,
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

    /// Replace a replica with the same id, or insert it when absent. Runtime
    /// re-adoption uses this to make `Stopped -> Ready` atomic from readers'
    /// point of view and to guarantee one registry row per Docker replica.
    pub fn upsert(&mut self, replica: Replica) {
        self.remove(&replica.id);
        self.add(replica);
    }

    /// Take a replica out of routing while preserving its slot in the pool.
    /// `observed_at` records when the external restart was first noticed so
    /// the scaler can allow a short grace window without separate AppState.
    pub fn mark_restarting(
        &mut self,
        replica_id: &ReplicaId,
        observed_at: chrono::DateTime<chrono::Utc>,
    ) -> bool {
        let Some(replica) = self.find_mut(replica_id) else {
            return false;
        };
        replica.state = ReplicaState::Stopped;
        replica.started_at = observed_at;
        replica.sessions_active = 0;
        true
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

    // ── image_tag_matches (default image_present, #498) ──────────────

    #[test]
    fn image_tag_matches_exact_and_latest() {
        let tags = vec!["nginx:1.27".to_string(), "org/app:latest".to_string()];
        // Exact ref hits.
        assert!(image_tag_matches(&tags, "nginx:1.27"));
        assert!(image_tag_matches(&tags, "org/app:latest"));
        // A bare repo matches its `:latest` (Docker's default tag).
        assert!(image_tag_matches(&tags, "org/app"));
        // A bare repo does NOT match a non-latest tag.
        assert!(!image_tag_matches(&tags, "nginx"));
        // Unknown image.
        assert!(!image_tag_matches(&tags, "redis:7"));
        // No tags (dangling image) never matches.
        assert!(!image_tag_matches(&[], "nginx:1.27"));
    }

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

    // #986 slice C: the per-schedule job timeout rides on the request.
    // Default is None (backend default cap); the builder sets it.
    #[test]
    fn spawn_request_carries_job_timeout() {
        assert!(SpawnRequest::new("a", "img").job_timeout_secs.is_none());
        let req = SpawnRequest::new("etl", "img").with_job_timeout(600);
        assert_eq!(req.job_timeout_secs, Some(600));
    }

    #[test]
    fn spawn_request_carries_readiness_timeout_and_zero_inherits() {
        assert!(SpawnRequest::new("a", "img")
            .readiness_timeout_ms
            .is_none());
        assert!(SpawnRequest::new("a", "img")
            .with_readiness_timeout_ms(0)
            .readiness_timeout_ms
            .is_none());
        let req = SpawnRequest::new("slow", "img").with_readiness_timeout_ms(120_000);
        assert_eq!(req.readiness_timeout_ms, Some(120_000));
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
