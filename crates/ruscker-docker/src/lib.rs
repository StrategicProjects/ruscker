//! Docker backend — implements [`ruscker_core::ContainerBackend`]
//! against a local Docker daemon via bollard 0.21.
//!
//! Container ↔ replica binding is encoded in Docker **labels**
//! instead of held in memory:
//!
//! - `ruscker.spec_id = <id>`   — which spec owns this container
//! - `ruscker.replica_id = <uuid>` — opaque routing handle
//! - `ruscker.inner_port = <n>` — what port the app binds inside
//!
//! That lets `stop(replica_id)` look up the container without
//! the backend keeping its own state, and `list()` can rebuild
//! the registry on process restart by filtering on the label.
//! Ruscker can crash and resume without losing track of who's
//! running.

use async_trait::async_trait;
use bollard::models::{
    ContainerCreateBody, ContainerSummaryStateEnum, HostConfig, NetworkCreateRequest, PortBinding,
};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptions, ListContainersOptions, ListImagesOptions,
    ListVolumesOptions, LogsOptionsBuilder, RemoveContainerOptions, RemoveImageOptions,
    RemoveVolumeOptions, StartContainerOptions, StatsOptionsBuilder, StopContainerOptions,
};
pub mod multihost;
pub use multihost::MultiHostDockerBackend;

use bollard::Docker;
use chrono::Utc;
use dashmap::DashMap;
use futures_util::StreamExt;
use ruscker_core::{
    ContainerBackend, CoreError, CoreResult, ImageInfo, ManagedContainer, Replica, ReplicaId,
    ReplicaMetrics, ReplicaState,
};
use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::sleep;

pub const LABEL_SPEC_ID: &str = "ruscker.spec_id";
pub const LABEL_REPLICA_ID: &str = "ruscker.replica_id";
/// Stamped on run-to-completion job containers (#986) instead of
/// `LABEL_REPLICA_ID`, so the replica machinery (reconcile, registry,
/// liveness prune, disk panel's managed list) never sees a job.
pub const LABEL_JOB: &str = "ruscker.job";
pub const LABEL_INNER_PORT: &str = "ruscker.inner_port";

/// Default inner port if the spec doesn't tell us — matches the
/// Shiny Server default. APIs override via the `inner_port`
/// argument to [`LocalDockerBackend::spawn_with_port`].
pub const DEFAULT_INNER_PORT: u16 = 3838;

/// Default wait for a freshly-started container to accept a TCP
/// connection on its bound port. Overridden per backend by
/// `proxy.container-wait-time` via
/// [`LocalDockerBackend::with_readiness_timeout`] (#970); this const
/// is only the fallback (and mirrors ShinyProxy's spirit — their
/// default is 20 s, ours is a more forgiving 60 s).
pub const READINESS_TIMEOUT: Duration = Duration::from_secs(60);

/// How often to retry the TCP connect during readiness polling.
pub const READINESS_INTERVAL: Duration = Duration::from_millis(250);

/// How many trailing container log lines to attach to a readiness
/// failure so the operator can see *why* an app didn't come up (e.g. a
/// failed DB connection) without re-running the container by hand (#550).
const READINESS_LOG_TAIL: usize = 40;

/// How long we wait for a graceful SIGTERM before bollard's stop
/// API escalates to SIGKILL. ShinyProxy uses 10 s; we match.
pub const STOP_TIMEOUT_SECS: i32 = 10;

pub struct LocalDockerBackend {
    docker: Docker,
    /// Interface the container's port is published on, in the
    /// daemon's `HostConfig` port binding. `127.0.0.1` for a local
    /// daemon (keep it off the network); `0.0.0.0` for a remote
    /// daemon so the proxy can reach it across hosts.
    publish_ip: String,
    /// Host the proxy connects to for the upstream — `127.0.0.1` for
    /// local, or the remote host's reachable IP/name for a remote
    /// daemon. Combined with the bound port to form `Replica.upstream`.
    upstream_host: String,
    /// Previous-reading cache for CPU delta calculation. Docker
    /// reports CPU as a cumulative counter; converting to a
    /// percentage requires comparing two readings against each
    /// other. We hold the last reading per container here so a
    /// single `metrics()` call can produce a percent on its own
    /// (returning 0% only on the very first observation of a
    /// given container, until the next refresh fills the cache).
    prev_stats: DashMap<String, PrevReading>,
    /// How long a freshly-started container gets to become ready
    /// before the spawn fails. Defaults to [`READINESS_TIMEOUT`];
    /// the operator's `proxy.container-wait-time` overrides it via
    /// [`Self::with_readiness_timeout`] (#970).
    readiness_timeout: Duration,
}

/// Cumulative CPU counters from a previous `stats` read,
/// used to compute the delta on the next read.
#[derive(Debug, Clone, Copy)]
struct PrevReading {
    /// Cumulative container CPU time in nanoseconds, monotonically
    /// increasing for the container's lifetime.
    cpu_total: u64,
    /// Cumulative system-wide CPU time in nanoseconds at the same
    /// instant. Used as the denominator of the percent calc.
    cpu_system: u64,
}

impl LocalDockerBackend {
    /// Connect to the local Docker daemon. On Unix this is
    /// typically `/var/run/docker.sock`; on macOS / Windows
    /// bollard's `connect_with_local_defaults` finds the right
    /// path (Docker Desktop, Colima, OrbStack, …).
    pub fn local() -> CoreResult<Self> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| CoreError::Backend(format!("connect to docker socket: {e}")))?;
        Ok(Self::from_docker(docker))
    }

    /// Build over an existing bollard connection bound to the **local**
    /// daemon — publishes on `127.0.0.1` and proxies to `127.0.0.1`.
    /// Used by `local()` and tests.
    pub fn from_docker(docker: Docker) -> Self {
        Self::from_docker_addressed(docker, "127.0.0.1", "127.0.0.1")
    }

    /// Build over an existing bollard connection with explicit
    /// addressing — for a remote daemon (multi-host, Phase 6):
    /// `publish_ip` is the interface the container port binds to on the
    /// daemon's host (use `0.0.0.0` so the proxy can reach it), and
    /// `upstream_host` is the reachable host the proxy connects to.
    pub fn from_docker_addressed(
        docker: Docker,
        publish_ip: impl Into<String>,
        upstream_host: impl Into<String>,
    ) -> Self {
        Self {
            docker,
            publish_ip: publish_ip.into(),
            upstream_host: upstream_host.into(),
            prev_stats: DashMap::new(),
            readiness_timeout: READINESS_TIMEOUT,
        }
    }

    /// Override how long a spawned container may take to become ready
    /// (`proxy.container-wait-time`, #970). Zero keeps the default —
    /// a 0 ms budget would fail every spawn instantly, which no
    /// operator ever means.
    pub fn with_readiness_timeout(mut self, timeout: Duration) -> Self {
        if !timeout.is_zero() {
            self.readiness_timeout = timeout;
        }
        self
    }

    /// Spawn a container for `spec_id` with the explicit inner
    /// port to bind. Use this when the spec carries an
    /// `api.port`; otherwise [`Self::spawn`] uses
    /// [`DEFAULT_INNER_PORT`].
    pub async fn spawn_with_port(
        &self,
        spec_id: &str,
        image: &str,
        inner_port: u16,
    ) -> CoreResult<Replica> {
        self.spawn_with_port_and_creds(spec_id, image, inner_port, None).await
    }

    /// Like [`Self::spawn_with_port`] but takes optional registry
    /// credentials for pulling private images. Thin wrapper around
    /// [`Self::spawn_request`] kept for callers that pre-date the
    /// `SpawnRequest` consolidation.
    pub async fn spawn_with_port_and_creds(
        &self,
        spec_id: &str,
        image: &str,
        inner_port: u16,
        creds: Option<&ruscker_core::RegistryCredentials>,
    ) -> CoreResult<Replica> {
        let mut req = ruscker_core::SpawnRequest::new(spec_id, image).with_port(inner_port);
        if let Some(c) = creds {
            req = req.with_creds(c.clone());
        }
        self.spawn_request(&req).await
    }

    /// Spawn one replica from a fully-described
    /// [`ruscker_core::SpawnRequest`]. This is the real
    /// implementation; the older `spawn_*` shims delegate here.
    pub async fn spawn_request(
        &self,
        req: &ruscker_core::SpawnRequest,
    ) -> CoreResult<Replica> {
        let replica_id = ReplicaId::new();
        let inner_port = req.inner_port.unwrap_or(DEFAULT_INNER_PORT);

        // `${VAR}` resolution (container-env + registry password) happens
        // upstream in ruscker-admin, which now fails the spawn naming the
        // missing variable before we get here (#314). We deliberately do
        // NOT re-scan `req.env` for a residual `${` — that scan also
        // false-rejected a legitimately-resolved value that happens to
        // contain the literal `${` (e.g. a templated JSON config).

        // 1. Pull image (idempotent — Docker no-ops when local).
        //    Errors bubble up through the stream-of-events.
        self.ensure_image_pulled(&req.image, req.creds.as_ref(), req.platform.as_deref())
            .await?;

        // 2. Create container with our labels + ephemeral host port.
        let port_key = format!("{inner_port}/tcp");
        let mut port_bindings = HashMap::new();
        port_bindings.insert(
            port_key.clone(),
            Some(vec![PortBinding {
                // Local daemon ⇒ 127.0.0.1 (off the network); remote
                // daemon ⇒ 0.0.0.0 so the proxy can reach it.
                host_ip: Some(self.publish_ip.clone()),
                host_port: Some(String::new()), // "" => ephemeral
            }]),
        );

        let mut labels = HashMap::new();
        // Operator-supplied labels (`labels`, #851) go in FIRST so the
        // internal `ruscker.*` labels below overwrite them on any key
        // collision — those three are load-bearing (registry / reconcile
        // / disk panel key off them) and must not be clobbered.
        for (k, v) in &req.labels {
            labels.insert(k.clone(), v.clone());
        }
        labels.insert(LABEL_SPEC_ID.to_string(), req.spec_id.clone());
        labels.insert(LABEL_REPLICA_ID.to_string(), replica_id.to_string());
        labels.insert(LABEL_INNER_PORT.to_string(), inner_port.to_string());

        let mut host_config = HostConfig {
            port_bindings: Some(port_bindings),
            // Bind-mount volumes, in Docker's "/host:/container[:ro]"
            // syntax — passed straight through. Empty → no binds.
            binds: (!req.volumes.is_empty()).then(|| req.volumes.clone()),
            ..Default::default()
        };
        // Apply resource limits if the spec set any. Empty limits
        // leave the HostConfig minimal so we don't paint Docker
        // with zero-quotas that mean "unlimited" anyway but show
        // up in `inspect` output and confuse operators.
        apply_limits(&mut host_config, &req.limits);

        // Attach to an explicit Docker network if the spec set
        // `container-network` (phase 3.5). We create the network (a
        // plain user-defined bridge) when it's missing so the operator
        // doesn't have to pre-create it. `network_mode` names it on the
        // container; the published port still binds on `publish_ip`, so
        // the proxy reaches the app exactly as before — the network only
        // isolates app-to-app traffic on its own L2 segment.
        if let Some(network) = req.network.as_deref() {
            self.ensure_network(network).await?;
            host_config.network_mode = Some(network.to_string());
        }

        let body = ContainerCreateBody {
            image: Some(req.image.clone()),
            // bollard 0.21 takes a flat Vec<String> here, not a
            // HashMap — list of "<port>/<proto>" strings.
            exposed_ports: Some(vec![port_key.clone()]),
            labels: Some(labels),
            host_config: Some(host_config),
            // `container-env` → `Config.Env` ("NAME=value"); `None`
            // (not an empty Vec) when unset so we don't override the
            // image's own env with nothing. `container-cmd` →
            // `Config.Cmd`; `None` keeps the image's baked `CMD`.
            env: (!req.env.is_empty()).then(|| req.env.clone()),
            cmd: req.cmd.clone(),
            ..Default::default()
        };

        let container_name = format!("ruscker-{}-{}", req.spec_id, &replica_id.to_string()[..8]);
        let opts = CreateContainerOptions {
            name: Some(container_name.clone()),
            // Same `platform` as the pull so the daemon doesn't try
            // to pick a fresh manifest match. Empty string = no
            // override (daemon picks per the image's manifest).
            platform: req.platform.clone().unwrap_or_default(),
        };
        let created = self
            .docker
            .create_container(Some(opts), body)
            .await
            .map_err(|e| backend_err("create container", e))?;
        let container_id = created.id;

        // Steps 3–5 run with the container already created (and, after
        // step 3, RUNNING). Any failure here used to just bubble up,
        // leaving the container behind with full ruscker labels but no
        // registry entry (#733): each visitor retry then spawned another
        // duplicate (the coalescer can't see an unregistered container),
        // and a restart's reconcile adopted the orphan as `Ready` even
        // though it never answered HTTP. Funnel them through one
        // fallible block so any error reaps the container first.
        let ready = async {
            // 3. Start it.
            self.docker
                .start_container(&container_id, None::<StartContainerOptions>)
                .await
                .map_err(|e| backend_err("start container", e))?;

            // 4. Inspect to learn the host port Docker assigned.
            let bound = self.bound_host_port(&container_id, &port_key).await?;
            // Proxy connects to the daemon's host (127.0.0.1 local, or the
            // remote host's reachable address) on the published port.
            // `to_socket_addrs` resolves an IP literal instantly and a
            // hostname via DNS, so a remote `ssh://user@host` works too.
            let upstream: SocketAddr = format!("{}:{bound}", self.upstream_host)
                .to_socket_addrs()
                .map_err(|e| {
                    CoreError::Backend(format!("resolve upstream {}: {e}", self.upstream_host))
                })?
                .next()
                .ok_or_else(|| {
                    CoreError::Backend(format!("no address for upstream {}", self.upstream_host))
                })?;

            // 5. Wait for the container's process to bind that port. Passes
            //    the backend + container id so a startup crash fails fast and
            //    the error carries the container's log tail (#550).
            wait_for_ready(self, &container_id, upstream).await?;
            Ok::<SocketAddr, CoreError>(upstream)
        }
        .await;

        let upstream = match ready {
            Ok(u) => u,
            Err(err) => {
                // Best-effort reap; `force` covers the already-running
                // case. A removal failure is logged, not returned — the
                // spawn error is the one the caller needs to see.
                if let Err(rm) = self
                    .docker
                    .remove_container(
                        &container_id,
                        Some(RemoveContainerOptions {
                            force: true,
                            ..Default::default()
                        }),
                    )
                    .await
                {
                    tracing::warn!(
                        container = %container_id, error = %rm,
                        "failed to remove container after spawn failure — manual cleanup may be needed"
                    );
                } else {
                    tracing::info!(
                        container = %container_id,
                        "removed container left by failed spawn"
                    );
                }
                return Err(err);
            }
        };

        Ok(Replica {
            id: replica_id,
            spec_id: req.spec_id.clone(),
            container_id,
            upstream,
            state: ReplicaState::Ready,
            started_at: Utc::now(),
            sessions_active: 0,
            sessions_max: 0,
            host: None,
        })
    }

    async fn ensure_image_pulled(
        &self,
        image: &str,
        creds: Option<&ruscker_core::RegistryCredentials>,
        platform: Option<&str>,
    ) -> CoreResult<()> {
        // bollard's create_image always hits the registry for the
        // manifest — even when the layers are already local. Skip
        // the round-trip when the image is present, mirroring the
        // operator's expectation of "no-op when local". A flaky
        // network shouldn't stop a container that's already on
        // the host. A transient daemon error (not a 404) is logged but
        // still falls through to a pull — the spawn needs the image, and
        // pulling is idempotent (#586).
        match image_is_present(&self.docker, image).await {
            Ok(true) => {
                tracing::debug!(image, "image present locally; skipping pull");
                return Ok(());
            }
            Ok(false) => {} // genuinely absent → pull below
            Err(e) => {
                tracing::warn!(image, error = %e, "image presence check failed; attempting pull");
            }
        }
        let opts = CreateImageOptions {
            from_image: Some(image.to_string()),
            // `platform` defaults to empty string → bollard sends no
            // explicit platform → daemon picks per the manifest. Only
            // set it when the spec carries one (e.g. `linux/amd64`
            // on an arm64 host running an amd64-only image via
            // emulation).
            platform: platform.unwrap_or("").to_string(),
            ..Default::default()
        };
        // The registry password's `${VAR}` is resolved upstream in
        // ruscker-admin, which fails the spawn naming the missing var
        // (#314); no residual-`${` scan here (it would false-reject a
        // legitimately-resolved value containing the literal `${`).
        // Convert our backend-neutral creds to bollard's native
        // type only at the pull boundary. `None` keeps the pull
        // anonymous (Docker Hub public images).
        let bollard_creds = creds.map(|c| bollard::auth::DockerCredentials {
            username: Some(c.username.clone()),
            password: Some(c.password.clone()),
            serveraddress: canonical_server_address(c.server_address.as_deref()),
            ..Default::default()
        });
        let context = pull_log_context(image, creds);
        let auth = auth_desc(&context);
        tracing::info!(
            image,
            auth_mode = context.auth_mode,
            credential = %context.credential,
            registry = %context.registry,
            username = %context.username,
            "pulling image"
        );
        let mut stream = self.docker.create_image(Some(opts), None, bollard_creds);
        while let Some(event) = stream.next().await {
            // The auth context rides in the error: "pull access denied"
            // with `anonymous` vs `authenticated as …` is the difference
            // between a missing credential and a wrong one (#820).
            match event {
                Err(error) => {
                    log_pull_failure(image, &context, &error);
                    return Err(backend_err(&format!("pull image ({auth})"), error));
                }
                Ok(info) => {
                    if let Some(error) = info.error_detail {
                        let message = error
                            .message
                            .unwrap_or_else(|| "Docker reported an unspecified pull error".into());
                        log_pull_failure(image, &context, &message);
                        return Err(CoreError::Backend(format!(
                            "pull image ({auth}): {message}"
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    /// Ensure a user-defined Docker network exists, creating a plain
    /// bridge when it's missing. Idempotent: an existing network (any
    /// driver) is left untouched. Called before `create_container` when a
    /// spec sets `container-network` so operators don't have to
    /// pre-create the network. Tolerates the create/create race between
    /// two concurrent spawns by re-inspecting on a create error.
    async fn ensure_network(&self, name: &str) -> CoreResult<()> {
        if self.docker.inspect_network(name, None).await.is_ok() {
            return Ok(());
        }
        match self
            .docker
            .create_network(NetworkCreateRequest {
                name: name.to_string(),
                driver: Some("bridge".to_string()),
                ..Default::default()
            })
            .await
        {
            Ok(_) => Ok(()),
            // A concurrent spawn may have created it between our inspect
            // and our create — a now-present network means success.
            Err(e) => {
                if self.docker.inspect_network(name, None).await.is_ok() {
                    Ok(())
                } else {
                    Err(backend_err("create network", e))
                }
            }
        }
    }

    async fn bound_host_port(&self, container_id: &str, port_key: &str) -> CoreResult<u16> {
        let info = self
            .docker
            .inspect_container(container_id, None)
            .await
            .map_err(|e| backend_err("inspect container", e))?;
        let bindings = match info
            .network_settings
            .as_ref()
            .and_then(|ns| ns.ports.as_ref())
            .and_then(|p| p.get(port_key).cloned())
        {
            Some(b) => b,
            None => {
                // A missing binding almost always means the container
                // already DIED before publishing the port — when a
                // container stops, Docker drops its port bindings, so the
                // bare "no port binding" was a misleading symptom of a
                // boot crash (#823). Turn it into a real diagnosis: name
                // the exit code and append the container's log tail (the
                // app's own stack trace — bad config, unreachable DB,
                // missing mounted file…), the same enrichment a readiness
                // failure already gets.
                let headline = match container_exit_code(&self.docker, container_id).await {
                    Some(code) => format!(
                        "container exited (code {code}) before publishing {port_key} — \
                         likely a startup crash; see the log below"
                    ),
                    None => format!(
                        "{port_key} is not published on the container (is the app \
                         listening on that port inside the container?)"
                    ),
                };
                return Err(readiness_failure(self, container_id, headline).await);
            }
        };
        let port_str = bindings
            .as_ref()
            .and_then(|v| v.first())
            .and_then(|pb| pb.host_port.as_ref())
            .ok_or_else(|| CoreError::Backend("port binding missing host_port".into()))?;
        port_str
            .parse::<u16>()
            .map_err(|e| CoreError::Backend(format!("parse host_port `{port_str}`: {e}")))
    }

    async fn find_container_id_for_replica(
        &self,
        replica_id: &ReplicaId,
    ) -> CoreResult<Option<String>> {
        let label_filter = format!("{LABEL_REPLICA_ID}={replica_id}");
        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec![label_filter]);
        let opts = ListContainersOptions {
            all: true,
            filters: Some(filters),
            ..Default::default()
        };
        let list = self
            .docker
            .list_containers(Some(opts))
            .await
            .map_err(|e| backend_err("list containers", e))?;
        Ok(list.first().and_then(|c| c.id.clone()))
    }

    async fn container_id_for_replica(&self, replica_id: &ReplicaId) -> CoreResult<String> {
        self.find_container_id_for_replica(replica_id)
            .await?
            .ok_or_else(|| {
                CoreError::Backend(format!("no container labeled with replica_id={replica_id}"))
            })
    }
}

#[async_trait]
impl ContainerBackend for LocalDockerBackend {
    async fn spawn(&self, spec_id: &str, image: &str) -> CoreResult<Replica> {
        // Inherent method shadowed by the trait method below — call
        // the inherent one explicitly via Self::.
        Self::spawn_with_port(self, spec_id, image, DEFAULT_INNER_PORT).await
    }

    /// The image's creation/publish timestamp (RFC3339), read from a
    /// local `docker inspect`. `None` if the image isn't present or has
    /// no timestamp — the caller leaves the date unset (#375).
    async fn image_created(&self, image: &str) -> Option<String> {
        self.docker
            .inspect_image(image)
            .await
            .ok()
            .and_then(|info| info.created)
    }

    /// Trait override: routes through the inherent method that
    /// already does the bollard work.
    async fn spawn_with_port(
        &self,
        spec_id: &str,
        image: &str,
        inner_port: u16,
    ) -> CoreResult<Replica> {
        Self::spawn_with_port(self, spec_id, image, inner_port).await
    }

    /// Trait override: pass-through to the credentials-aware
    /// inherent method.
    async fn spawn_with_port_and_creds(
        &self,
        spec_id: &str,
        image: &str,
        inner_port: u16,
        creds: Option<&ruscker_core::RegistryCredentials>,
    ) -> CoreResult<Replica> {
        Self::spawn_with_port_and_creds(self, spec_id, image, inner_port, creds).await
    }

    /// Trait override: the full SpawnRequest path lands here.
    /// This is the only override that honors `req.limits` —
    /// older callers fall through to the back-compat shims.
    async fn spawn_request(
        &self,
        req: &ruscker_core::SpawnRequest,
    ) -> CoreResult<Replica> {
        Self::spawn_request(self, req).await
    }

    async fn stop(&self, replica_id: &ReplicaId) -> CoreResult<()> {
        let Some(container_id) = self.find_container_id_for_replica(replica_id).await? else {
            tracing::debug!(replica = %replica_id, "stop: container already absent");
            return Ok(());
        };
        // Graceful: SIGTERM with timeout — bollard escalates to
        // SIGKILL internally after `t` seconds.
        let _ = self
            .docker
            .stop_container(
                &container_id,
                Some(StopContainerOptions {
                    t: Some(STOP_TIMEOUT_SECS),
                    ..Default::default()
                }),
            )
            .await;
        // Always remove, even if stop hit a race. force=true
        // covers the "already exited" case bollard otherwise
        // reports as an error.
        self.docker
            .remove_container(
                &container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| backend_err("remove container", e))?;
        // Drop the container's CPU-delta baseline so `prev_stats` doesn't
        // accumulate a dead entry per recycled container over a long-
        // lived deployment (#325). The hook existed but was never called.
        self.forget_metrics(&container_id);
        Ok(())
    }

    async fn list(&self) -> CoreResult<Vec<Replica>> {
        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec![LABEL_REPLICA_ID.to_string()]);
        let opts = ListContainersOptions {
            all: true,
            filters: Some(filters),
            ..Default::default()
        };
        let list = self
            .docker
            .list_containers(Some(opts))
            .await
            .map_err(|e| backend_err("list containers", e))?;

        let mut replicas = Vec::with_capacity(list.len());
        for c in list {
            let labels = c.labels.clone().unwrap_or_default();
            let Some(spec_id) = labels.get(LABEL_SPEC_ID).cloned() else {
                continue;
            };
            let Some(replica_id_str) = labels.get(LABEL_REPLICA_ID).cloned() else {
                continue;
            };
            let Ok(replica_id) = replica_id_str.parse::<uuid::Uuid>().map(ReplicaId) else {
                continue;
            };
            let inner_port: u16 = labels
                .get(LABEL_INNER_PORT)
                .and_then(|s| s.parse().ok())
                .unwrap_or(DEFAULT_INNER_PORT);

            // Find the bound host port. ContainerSummary.ports is
            // a flat list of {private_port, public_port, typ} —
            // match the private to the inner port we set.
            let host_port: Option<u16> = c.ports.as_ref().and_then(|ports| {
                ports
                    .iter()
                    .find(|p| p.private_port == inner_port)
                    .and_then(|p| p.public_port)
            });
            let upstream: SocketAddr = match host_port {
                Some(p) => match format!("{}:{p}", self.upstream_host).to_socket_addrs() {
                    Ok(mut addrs) => match addrs.next() {
                        Some(a) => a,
                        None => continue,
                    },
                    Err(e) => {
                        tracing::warn!(error = %e, host = %self.upstream_host, "resolve upstream on list; skipping");
                        continue;
                    }
                },
                None => {
                    tracing::warn!(
                        container_id = ?c.id,
                        inner_port,
                        "no host port binding for replica; skipping from list"
                    );
                    continue;
                }
            };

            let state = state_from_enum(c.state.as_ref());
            // Use the container's real creation time so a Ruscker restart
            // doesn't reset every pre-existing replica's uptime to 0 (#586).
            // `created` is Unix seconds; fall back to now if absent.
            let started_at = c
                .created
                .and_then(|ts| chrono::DateTime::from_timestamp(ts, 0))
                .unwrap_or_else(Utc::now);
            replicas.push(Replica {
                id: replica_id,
                spec_id,
                container_id: c.id.unwrap_or_default(),
                upstream,
                state,
                started_at,
                sessions_active: 0,
                sessions_max: 0,
                host: None,
            });
        }
        Ok(replicas)
    }

    async fn metrics(&self, replica_id: &ReplicaId) -> CoreResult<ReplicaMetrics> {
        // Map replica → container via the label set on spawn.
        let container_id = self.container_id_for_replica(replica_id).await?;
        self.metrics_for_container(&container_id).await
    }

    /// Use the caller-supplied `container_id` (from the registry) and
    /// skip the `container_id_for_replica` `list_containers` lookup —
    /// the metrics cache calls this for every replica every refresh, so
    /// dropping that round-trip halves the per-refresh daemon traffic
    /// (#282). Falls back to the label lookup if the id is empty.
    async fn metrics_for(
        &self,
        replica_id: &ReplicaId,
        container_id: &str,
    ) -> CoreResult<ReplicaMetrics> {
        if container_id.is_empty() {
            return self.metrics(replica_id).await;
        }
        self.metrics_for_container(container_id).await
    }

    async fn logs(&self, replica_id: &ReplicaId, tail: usize) -> CoreResult<Vec<String>> {
        let container_id = self.container_id_for_replica(replica_id).await?;
        self.logs_for_container(&container_id, tail).await
    }

    async fn logs_follow(
        &self,
        replica_id: &ReplicaId,
        tail: usize,
    ) -> CoreResult<ruscker_core::LogStream> {
        let container_id = self.container_id_for_replica(replica_id).await?;
        // Clone the bollard handle (cheap — Arc inside) so the
        // returned stream owns everything it needs and is
        // `'static`. The `follow: true` stream stays open until
        // the container exits.
        let docker = self.docker.clone();
        let opts = LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .follow(true)
            .tail(&tail.min(5_000).to_string())
            .build();
        let stream = async_stream::stream! {
            let mut inner = docker.logs(&container_id, Some(opts));
            while let Some(item) = inner.next().await {
                match item {
                    Ok(chunk) => {
                        let text = chunk.to_string();
                        for line in text.split_inclusive('\n') {
                            yield line.trim_end_matches(['\r', '\n']).to_string();
                        }
                    }
                    // A read error ends the follow — the
                    // container likely went away. The SSE layer
                    // turns stream-end into a closed event source,
                    // which the browser may auto-reconnect.
                    Err(_) => break,
                }
            }
        };
        Ok(Box::pin(stream))
    }

    // ── Disk management (#453 part B) ───────────────────────────────

    async fn list_managed_containers(&self) -> CoreResult<Vec<ManagedContainer>> {
        // Same label scope as `list`, but `all: true` keeps stopped/exited
        // containers — and we don't require a live port binding, so a
        // crashed replica still shows up for reclamation.
        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec![LABEL_REPLICA_ID.to_string()]);
        let opts = ListContainersOptions {
            all: true,
            filters: Some(filters),
            ..Default::default()
        };
        let list = self
            .docker
            .list_containers(Some(opts))
            .await
            .map_err(|e| backend_err("list managed containers", e))?;
        let managed = list
            .into_iter()
            .map(|c| {
                let labels = c.labels.unwrap_or_default();
                let name = c
                    .names
                    .and_then(|n| n.into_iter().next())
                    .map(|s| s.trim_start_matches('/').to_string())
                    .unwrap_or_default();
                ManagedContainer {
                    id: c.id.unwrap_or_default(),
                    name,
                    image: c.image.unwrap_or_default(),
                    spec_id: labels.get(LABEL_SPEC_ID).cloned(),
                    status: c.status.unwrap_or_default(),
                    running: matches!(c.state, Some(ContainerSummaryStateEnum::RUNNING)),
                }
            })
            .collect();
        Ok(managed)
    }

    async fn all_container_image_refs(&self) -> CoreResult<Vec<String>> {
        // ALL containers (running + stopped), no label filter.
        let opts = ListContainersOptions {
            all: true,
            ..Default::default()
        };
        let list = self
            .docker
            .list_containers(Some(opts))
            .await
            .map_err(|e| backend_err("list all containers", e))?;
        let mut refs = Vec::with_capacity(list.len() * 2);
        for c in list {
            if let Some(img) = c.image {
                refs.push(img);
            }
            if let Some(id) = c.image_id {
                refs.push(id);
            }
        }
        Ok(refs)
    }

    async fn remove_container(&self, container_id: &str) -> CoreResult<()> {
        // #871: the id comes from an operator POST, and `force=true` would
        // stop+remove ANY container (incl. a running non-Ruscker one,
        // bypassing the daemon's in-use refusal). Re-inspect and refuse
        // anything without our `ruscker.replica_id` label — the listing is
        // label-scoped, but the backend is the real backstop.
        let info = self
            .docker
            .inspect_container(container_id, None)
            .await
            .map_err(|e| backend_err("inspect container", e))?;
        let is_ruscker = info
            .config
            .as_ref()
            .and_then(|c| c.labels.as_ref())
            .is_some_and(|l| l.contains_key(LABEL_REPLICA_ID));
        if !is_ruscker {
            return Err(CoreError::Backend(format!(
                "refusing to remove non-Ruscker container {container_id}"
            )));
        }
        // force=true also stops a running container first, mirroring `stop`.
        self.docker
            .remove_container(
                container_id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await
            .map_err(|e| backend_err("remove container", e))?;
        self.forget_metrics(container_id);
        Ok(())
    }

    async fn prune_stopped(&self) -> CoreResult<usize> {
        // Only Ruscker-labeled, non-running containers — never a running
        // replica, never a non-Ruscker container on the host.
        let managed = self.list_managed_containers().await?;
        let mut removed = 0;
        for c in managed.iter().filter(|c| !c.running) {
            match self
                .docker
                .remove_container(
                    &c.id,
                    Some(RemoveContainerOptions {
                        force: true,
                        ..Default::default()
                    }),
                )
                .await
            {
                Ok(()) => {
                    self.forget_metrics(&c.id);
                    removed += 1;
                }
                Err(e) => tracing::warn!(error = %e, container = %c.id, "prune: remove failed"),
            }
        }
        Ok(removed)
    }

    async fn backend_version(&self) -> CoreResult<Option<String>> {
        // Best-effort: a failed version read is a missing diagnostic, not
        // an error worth bubbling — return None so the System tab shows
        // "unknown" rather than failing the page.
        Ok(self.docker.version().await.ok().and_then(|v| v.version))
    }

    async fn reclaim_space(&self) -> CoreResult<u64> {
        let mut bytes: i64 = 0;
        // `prune_images(None)` removes ONLY dangling (untagged) images —
        // orphan layers, never a tagged image or a container's image. No
        // `dangling:false` filter, so it can't behave like `image prune -a`.
        match self
            .docker
            .prune_images(None::<bollard::query_parameters::PruneImagesOptions>)
            .await
        {
            Ok(r) => bytes += r.space_reclaimed.unwrap_or(0),
            Err(e) => tracing::warn!(error = %e, "reclaim: prune dangling images failed"),
        }
        // Build cache (unreferenced layers from past builds).
        match self
            .docker
            .prune_build(None::<bollard::query_parameters::PruneBuildOptions>)
            .await
        {
            Ok(r) => bytes += r.space_reclaimed.unwrap_or(0),
            Err(e) => tracing::warn!(error = %e, "reclaim: prune build cache failed"),
        }
        Ok(bytes.max(0) as u64)
    }

    async fn list_images(&self) -> CoreResult<Vec<ImageInfo>> {
        let images = self
            .docker
            .list_images(None::<ListImagesOptions>)
            .await
            .map_err(|e| backend_err("list images", e))?;
        Ok(images
            .into_iter()
            .map(|i| ImageInfo {
                id: i.id,
                // A dangling image reports the `<none>:<none>` placeholder;
                // drop it so the UI shows "untagged" rather than noise.
                tags: i
                    .repo_tags
                    .into_iter()
                    .filter(|t| t != "<none>:<none>")
                    .collect(),
                size_bytes: i.size,
                containers: i.containers,
            })
            .collect())
    }

    async fn remove_image(&self, image: &str) -> CoreResult<()> {
        // No force: the daemon refuses an image still used by a container,
        // which backstops the panel's "remove unused only" rule.
        self.docker
            .remove_image(image, None::<RemoveImageOptions>, None)
            .await
            .map_err(|e| backend_err("remove image", e))?;
        Ok(())
    }

    async fn run_job(&self, req: &ruscker_core::SpawnRequest) -> CoreResult<ruscker_core::JobOutcome> {
        use futures_util::StreamExt;
        let started = std::time::Instant::now();

        // Same pull path as a spawn (idempotent, creds-aware).
        self.ensure_image_pulled(&req.image, req.creds.as_ref(), req.platform.as_deref())
            .await?;

        // Create WITHOUT any port publishing: a job talks to nobody.
        // `ruscker.job` (not `ruscker.replica_id`) keeps it invisible
        // to the replica machinery; `ruscker.spec_id` still ties it to
        // its spec for the disk panel / manual cleanup.
        let mut labels = HashMap::new();
        for (k, v) in &req.labels {
            labels.insert(k.clone(), v.clone());
        }
        labels.insert(LABEL_SPEC_ID.to_string(), req.spec_id.clone());
        labels.insert(LABEL_JOB.to_string(), "true".to_string());

        let mut host_config = HostConfig {
            binds: (!req.volumes.is_empty()).then(|| req.volumes.clone()),
            ..Default::default()
        };
        apply_limits(&mut host_config, &req.limits);
        if let Some(network) = req.network.as_deref() {
            self.ensure_network(network).await?;
            host_config.network_mode = Some(network.to_string());
        }
        let body = ContainerCreateBody {
            image: Some(req.image.clone()),
            labels: Some(labels),
            host_config: Some(host_config),
            env: (!req.env.is_empty()).then(|| req.env.clone()),
            cmd: req.cmd.clone(),
            ..Default::default()
        };
        let suffix = uuid::Uuid::new_v4().to_string();
        let name = format!("ruscker-job-{}-{}", req.spec_id, &suffix[..8]);
        let opts = CreateContainerOptions {
            name: Some(name.clone()),
            platform: req.platform.clone().unwrap_or_default(),
        };
        let container_id = self
            .docker
            .create_container(Some(opts), body)
            .await
            .map_err(|e| backend_err("create job container", e))?
            .id;

        // Best-effort removal on EVERY exit path below — a finished or
        // failed job must never linger as a stopped container.
        let cleanup = |docker: Docker, id: String| async move {
            let opts = RemoveContainerOptions {
                force: true,
                ..Default::default()
            };
            let _ = docker.remove_container(&id, Some(opts)).await;
        };

        if let Err(e) = self
            .docker
            .start_container(&container_id, None::<StartContainerOptions>)
            .await
        {
            cleanup(self.docker.clone(), container_id).await;
            return Err(backend_err("start job container", e));
        }

        // Wait for the exit, bounded: a hung job must not pin the
        // scheduler forever. The cap is generous — ETL runs are long —
        // and slice B makes it per-schedule.
        const JOB_TIMEOUT: Duration = Duration::from_secs(60 * 60);
        let waited = tokio::time::timeout(JOB_TIMEOUT, async {
            self.docker
                .wait_container(&container_id, None::<bollard::query_parameters::WaitContainerOptions>)
                .next()
                .await
        })
        .await;
        let exit_code = match waited {
            Err(_) => {
                cleanup(self.docker.clone(), container_id).await;
                return Err(CoreError::Backend(format!(
                    "job for `{}` still running after {JOB_TIMEOUT:?}; killed and removed",
                    req.spec_id
                )));
            }
            // Docker reports a non-zero exit as a stream *error* carrying
            // the response — both arms below still mean "the job ran".
            Ok(Some(Ok(w))) => w.status_code,
            Ok(Some(Err(bollard::errors::Error::DockerContainerWaitError { code, .. }))) => code,
            Ok(Some(Err(e))) => {
                cleanup(self.docker.clone(), container_id).await;
                return Err(backend_err("wait for job container", e));
            }
            Ok(None) => {
                cleanup(self.docker.clone(), container_id).await;
                return Err(CoreError::Backend("job wait stream ended without a status".into()));
            }
        };

        let log_tail = self
            .logs_for_container(&container_id, READINESS_LOG_TAIL)
            .await
            .unwrap_or_default();
        cleanup(self.docker.clone(), container_id).await;
        Ok(ruscker_core::JobOutcome {
            exit_code,
            log_tail,
            duration_ms: started.elapsed().as_millis() as u64,
        })
    }

    async fn list_volumes(&self) -> CoreResult<Vec<ruscker_core::VolumeInfo>> {
        // Reference counts come from EVERY container on the host
        // (running or stopped, any owner) — the safety cross-ref that
        // keeps the panel from offering a neighbour's in-use volume.
        let opts = ListContainersOptions {
            all: true,
            ..Default::default()
        };
        let containers = self
            .docker
            .list_containers(Some(opts))
            .await
            .map_err(|e| backend_err("list containers for volume refs", e))?;
        let mut refs: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
        for c in &containers {
            for m in c.mounts.iter().flatten() {
                if let Some(name) = &m.name {
                    *refs.entry(name.clone()).or_insert(0) += 1;
                }
            }
        }
        let listed = self
            .docker
            .list_volumes(None::<ListVolumesOptions>)
            .await
            .map_err(|e| backend_err("list volumes", e))?;
        Ok(listed
            .volumes
            .unwrap_or_default()
            .into_iter()
            .map(|v| ruscker_core::VolumeInfo {
                refs: refs.get(&v.name).copied().unwrap_or(0),
                ruscker_created: v.labels.contains_key("ruscker.created"),
                created_at: v.created_at,
                driver: v.driver,
                name: v.name,
            })
            .collect())
    }

    async fn create_volume(&self, name: &str) -> CoreResult<()> {
        // Labelled so the panel can tell "made here" from a
        // neighbour's volume — informational, mirrors the image
        // provenance idea (#894).
        let req = bollard::models::VolumeCreateRequest {
            name: Some(name.to_string()),
            labels: Some(std::collections::HashMap::from([(
                "ruscker.created".to_string(),
                "true".to_string(),
            )])),
            ..Default::default()
        };
        self.docker
            .create_volume(req)
            .await
            .map_err(|e| backend_err("create volume", e))?;
        Ok(())
    }

    async fn remove_volume(&self, name: &str) -> CoreResult<()> {
        // No force: the daemon refuses an in-use volume, backstopping
        // the panel's zero-references rule. Volume removal destroys
        // data — the daemon must always have the final word.
        self.docker
            .remove_volume(name, None::<RemoveVolumeOptions>)
            .await
            .map_err(|e| backend_err("remove volume", e))?;
        Ok(())
    }

    async fn image_present(&self, image: &str) -> CoreResult<bool> {
        // `inspect_image` resolves `repo` → `repo:latest` natively. A real
        // 404 is a clean `Ok(false)` (absent); any other daemon failure
        // is an `Err`, so the editor's indicator distinguishes "absent"
        // from a real failure (#498) instead of reporting both as absent
        // (#586).
        image_is_present(&self.docker, image).await
    }

    async fn pull_image(
        &self,
        image: &str,
        creds: Option<&ruscker_core::RegistryCredentials>,
        platform: Option<&str>,
    ) -> CoreResult<ruscker_core::LogStream> {
        // Same options/creds path as `ensure_image_pulled`, but the
        // `create_image` events are surfaced as progress lines instead of
        // discarded — the editor's Pull button streams them over SSE
        // (#498, slice B). A registry/auth failure rides through as an
        // `error: …` line, not a hard error, so the caller can show it.
        let opts = CreateImageOptions {
            from_image: Some(image.to_string()),
            platform: platform.unwrap_or("").to_string(),
            ..Default::default()
        };
        let bollard_creds = creds.map(|c| bollard::auth::DockerCredentials {
            username: Some(c.username.clone()),
            password: Some(c.password.clone()),
            serveraddress: canonical_server_address(c.server_address.as_deref()),
            ..Default::default()
        });
        // Ride the auth context on the streamed error line too (#822):
        // the spawn pull already does, but the editor's Pull button
        // (this path) didn't — "denied — anonymous" vs "— authenticated
        // as …" tells a dropped credential from a rejected one.
        let context = pull_log_context(image, creds);
        let auth = auth_desc(&context);
        let log_image = image.to_string();
        let stream = self
            .docker
            .create_image(Some(opts), None, bollard_creds)
            .map(move |ev| match ev {
                Ok(info) => {
                    if let Some(err) = info.error_detail {
                        let message = err
                            .message
                            .unwrap_or_else(|| "Docker reported an unspecified pull error".into());
                        log_pull_failure(&log_image, &context, &message);
                        return format!("error: {message} — {auth}");
                    }
                    let id = info.id.unwrap_or_default();
                    let status = info.status.unwrap_or_default();
                    let mut line = String::new();
                    if !id.is_empty() {
                        line.push_str(&id);
                        line.push_str(": ");
                    }
                    line.push_str(&status);
                    // Byte progress, when the daemon reports it (layer pulls).
                    if let Some(pd) = info.progress_detail {
                        if let (Some(cur), Some(total)) = (pd.current, pd.total) {
                            if total > 0 {
                                line.push_str(&format!(" {cur}/{total}"));
                            }
                        }
                    }
                    line
                }
                Err(error) => {
                    log_pull_failure(&log_image, &context, &error);
                    format!("error: {error} — {auth}")
                }
            });
        Ok(Box::pin(stream))
    }
}

impl LocalDockerBackend {
    /// One-shot stats read for a known container id. Returns the
    /// current memory + network counters and a CPU percent
    /// computed against the previous reading stored in
    /// [`Self::prev_stats`]. First observation of a container
    /// returns `cpu_percent: 0.0` because there's no delta — the
    /// next call (typically ~5 s later from the dashboard cache
    /// refresher) starts producing real percentages.
    ///
    /// Failing to read stats for one container doesn't trigger a
    /// retry — the cache layer will try again on its next tick.
    pub async fn metrics_for_container(&self, container_id: &str) -> CoreResult<ReplicaMetrics> {
        let opts = StatsOptionsBuilder::default()
            .stream(false)
            .one_shot(true)
            .build();
        // `stream: false` returns a single event and the stream
        // completes. Use `take(1)` defensively in case bollard's
        // semantics shift.
        let mut stream = self.docker.stats(container_id, Some(opts)).take(1);
        let Some(maybe) = stream.next().await else {
            return Err(CoreError::Backend(format!(
                "no stats event for container {container_id}"
            )));
        };
        let stat = maybe.map_err(|e| backend_err("stats", e))?;

        let cpu_total = stat
            .cpu_stats
            .as_ref()
            .and_then(|c| c.cpu_usage.as_ref())
            .and_then(|u| u.total_usage)
            .unwrap_or(0);
        let cpu_system = stat
            .cpu_stats
            .as_ref()
            .and_then(|c| c.system_cpu_usage)
            .unwrap_or(0);
        // bollard reports `online_cpus` as a `u32` in 0.21;
        // promote to `u64` so the percent math doesn't overflow
        // on absurdly large numbers (and matches our delta types).
        let online_cpus = stat
            .cpu_stats
            .as_ref()
            .and_then(|c| c.online_cpus)
            .map(u64::from)
            .unwrap_or_else(|| {
                // Fall back to percpu length if `online_cpus`
                // isn't reported (older Docker engines).
                stat.cpu_stats
                    .as_ref()
                    .and_then(|c| c.cpu_usage.as_ref())
                    .and_then(|u| u.percpu_usage.as_ref())
                    .map(|v| v.len() as u64)
                    .unwrap_or(1)
            })
            .max(1);

        // Compute the percent against the previous reading if we
        // have one. Otherwise stash the current reading and
        // report 0% for this call.
        let prev = self.prev_stats.get(container_id).map(|r| *r);
        self.prev_stats.insert(
            container_id.to_string(),
            PrevReading {
                cpu_total,
                cpu_system,
            },
        );
        let cpu_percent = match prev {
            Some(p) => cpu_percent_from_delta(p, cpu_total, cpu_system, online_cpus),
            None => 0.0,
        };

        let memory_bytes = stat.memory_stats.as_ref().and_then(|m| m.usage).unwrap_or(0);

        // Sum across all network interfaces. Most containers
        // have exactly one (`eth0`); summing keeps the math
        // robust if the operator attaches extras.
        let (network_rx_bytes, network_tx_bytes) = stat
            .networks
            .as_ref()
            .map(|nets| {
                nets.values().fold((0u64, 0u64), |(rx, tx), n| {
                    (rx + n.rx_bytes.unwrap_or(0), tx + n.tx_bytes.unwrap_or(0))
                })
            })
            .unwrap_or((0, 0));

        Ok(ReplicaMetrics {
            cpu_percent,
            memory_bytes,
            network_rx_bytes,
            network_tx_bytes,
        })
    }

    /// Forget a container's previous-reading entry. Called when
    /// a replica is stopped so the map doesn't grow without
    /// bound across long-lived deployments.
    pub fn forget_metrics(&self, container_id: &str) {
        self.prev_stats.remove(container_id);
    }

    /// Fetch the last `tail` lines of a container's combined
    /// stdout + stderr. One-shot (`follow: false`). Each
    /// returned `String` is one log line with its trailing
    /// newline trimmed; the stream framing bytes that Docker
    /// prepends in multiplexed mode are stripped by bollard's
    /// `LogOutput` decoding, so we just stringify each chunk.
    ///
    /// `tail` is capped defensively — an operator-supplied
    /// huge number shouldn't let a chatty container balloon
    /// the response.
    pub async fn logs_for_container(
        &self,
        container_id: &str,
        tail: usize,
    ) -> CoreResult<Vec<String>> {
        const MAX_TAIL: usize = 5_000;
        let tail = tail.min(MAX_TAIL);
        let opts = LogsOptionsBuilder::default()
            .stdout(true)
            .stderr(true)
            .follow(false)
            .tail(&tail.to_string())
            .build();

        let mut stream = self.docker.logs(container_id, Some(opts));
        let mut lines: Vec<String> = Vec::new();
        while let Some(item) = stream.next().await {
            let chunk = item.map_err(|e| backend_err("logs", e))?;
            // `LogOutput` Display gives the decoded text for the
            // frame (stdout/stderr/console). A single frame can
            // hold multiple newline-separated lines, so split.
            let text = chunk.to_string();
            for line in text.split_inclusive('\n') {
                lines.push(line.trim_end_matches(['\r', '\n']).to_string());
            }
        }
        Ok(lines)
    }
}

/// CPU percent in the Docker convention: container CPU time over
/// system CPU time, scaled by online CPU count to give a
/// "percent of one core" reading that exceeds 100% on multi-core
/// hosts. Returns 0% when the system delta is zero (no time
/// elapsed) to avoid divide-by-zero — that's typical for two
/// readings collected too close together.
fn cpu_percent_from_delta(
    prev: PrevReading,
    cpu_total: u64,
    cpu_system: u64,
    online_cpus: u64,
) -> f64 {
    let cpu_delta = cpu_total.saturating_sub(prev.cpu_total) as f64;
    let system_delta = cpu_system.saturating_sub(prev.cpu_system) as f64;
    if system_delta <= 0.0 {
        return 0.0;
    }
    (cpu_delta / system_delta) * (online_cpus as f64) * 100.0
}

/// Two-phase readiness:
///
/// 1. **TCP connect** — wait until Docker is forwarding the port.
///    This succeeds the moment Docker creates the binding, often
///    before the containerized app has called `accept()`.
/// 2. **HTTP HEAD `/`** — wait until the app actually answers any
///    HTTP request. ANY response (200, 404, 500) means the
///    process is alive and serving; we just need the connection
///    not to be closed mid-handshake. Without this second phase,
///    proxied requests immediately after spawn race the app's
///    own readiness and get `connection closed before message
///    completed`.
///
/// Per-spec health-check overrides (e.g. `api.health_path`) are a
/// Phase 3.5 refinement. For now we settle for "any HTTP response
/// means ready".
/// `Some(exit_code)` once the container has stopped/exited, `None` while
/// it is still running (or inspect failed — treat as "keep waiting").
/// Uses the formatted status defensively because the inspect response's
/// nested status type differs from the container-summary enum (#550 A).
async fn container_exit_code(docker: &Docker, container_id: &str) -> Option<i64> {
    let info = docker.inspect_container(container_id, None).await.ok()?;
    let state = info.state?;
    let status = format!("{:?}", state.status).to_ascii_lowercase();
    if status.contains("exited") || status.contains("dead") {
        Some(state.exit_code.unwrap_or(-1))
    } else {
        None
    }
}

/// Build a readiness-failure error enriched with the tail of the
/// container's logs (#550 B). Best-effort: if logs can't be read, just
/// return the headline. The tail surfaces in the operator-facing warn
/// log and the admin Logs tab — it is never returned to an end user.
async fn readiness_failure(
    backend: &LocalDockerBackend,
    container_id: &str,
    headline: String,
) -> CoreError {
    match backend.logs_for_container(container_id, READINESS_LOG_TAIL).await {
        Ok(lines) if !lines.is_empty() => CoreError::Backend(format!(
            "{headline}\n--- last {} container log line(s) ---\n{}",
            lines.len(),
            lines.join("\n")
        )),
        _ => CoreError::Backend(headline),
    }
}

/// Two-phase readiness with crash detection. On any failure the returned
/// error carries the tail of the container's logs (#550). The
/// `container_id` and the backend handle let us inspect the container
/// state (to fail fast on a startup crash) and read its logs.
async fn wait_for_ready(
    backend: &LocalDockerBackend,
    container_id: &str,
    addr: SocketAddr,
) -> CoreResult<()> {
    let budget = backend.readiness_timeout;
    let deadline = std::time::Instant::now() + budget;

    // Phase 1: TCP connect.
    loop {
        match TcpStream::connect(addr).await {
            Ok(_) => break,
            Err(e) => {
                // (#550 A) Fail fast if the container already exited —
                // e.g. it crashed on startup (a failed DB connection) —
                // rather than burning the whole readiness budget.
                if let Some(code) = container_exit_code(&backend.docker, container_id).await {
                    return Err(readiness_failure(
                        backend,
                        container_id,
                        format!("container at {addr} exited (code {code}) during startup, before it accepted TCP"),
                    )
                    .await);
                }
                if std::time::Instant::now() < deadline {
                    sleep(READINESS_INTERVAL).await;
                } else {
                    return Err(readiness_failure(
                        backend,
                        container_id,
                        format!("container at {addr} never accepted TCP within {budget:?}: {e}"),
                    )
                    .await);
                }
            }
        }
    }

    // Phase 2: HTTP HEAD / with a tiny budget. The app might
    // still be initializing its router. Tiny manual HTTP/1.1
    // request — no need to pull a client just for this.
    let req = b"HEAD / HTTP/1.1\r\nHost: ruscker-readiness\r\nConnection: close\r\n\r\n";
    loop {
        let attempt = async {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let mut s = TcpStream::connect(addr).await?;
            s.write_all(req).await?;
            // Read up to 64 bytes — enough to know the app sent
            // *something* back (status line). We don't parse it.
            let mut buf = [0u8; 64];
            let n = s.read(&mut buf).await?;
            std::io::Result::Ok(n)
        };
        match tokio::time::timeout(Duration::from_secs(2), attempt).await {
            Ok(Ok(n)) if n > 0 => return Ok(()),
            _ => {
                // (#550 A) Same fail-fast for a crash that happens after
                // the port briefly opened (TCP accepted, then exited).
                if let Some(code) = container_exit_code(&backend.docker, container_id).await {
                    return Err(readiness_failure(
                        backend,
                        container_id,
                        format!("container at {addr} exited (code {code}) during startup, before it answered HTTP"),
                    )
                    .await);
                }
                if std::time::Instant::now() < deadline {
                    sleep(READINESS_INTERVAL).await;
                } else {
                    return Err(readiness_failure(
                        backend,
                        container_id,
                        format!("container at {addr} accepted TCP but never answered HTTP within {budget:?}"),
                    )
                    .await);
                }
            }
        }
    }
}

/// Is `image` present on `docker`? Distinguishes a true 404 (`Ok(false)`,
/// absent) from any other daemon error (`Err`), so a transient failure
/// isn't silently reported as "absent" and doesn't trigger a needless
/// pull on every spawn (#586).
async fn image_is_present(docker: &Docker, image: &str) -> CoreResult<bool> {
    match docker.inspect_image(image).await {
        Ok(_) => Ok(true),
        Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => {
            Ok(false)
        }
        Err(e) => Err(backend_err("inspect image", e)),
    }
}

/// Map a container summary's state enum to a [`ReplicaState`]. Matching the
/// enum variants directly is robust across bollard versions — the previous
/// `format!("{:?}")` string-matching happened to work but would break on a
/// bollard bump (#586). Unknown/absent ⇒ `Starting` (conservative).
fn state_from_enum(s: Option<&ContainerSummaryStateEnum>) -> ReplicaState {
    use ContainerSummaryStateEnum as E;
    match s {
        Some(E::RUNNING) => ReplicaState::Ready,
        Some(E::CREATED | E::RESTARTING) => ReplicaState::Starting,
        Some(E::PAUSED) => ReplicaState::Draining,
        Some(E::EXITED | E::DEAD | E::REMOVING) => ReplicaState::Stopped,
        _ => ReplicaState::Starting,
    }
}

const DOCKER_HUB_REGISTRY: &str = "https://index.docker.io/v1/";

/// The registry address the Docker daemon should see for a pull.
///
/// Docker Hub is special: `docker login` records it as
/// `https://index.docker.io/v1/` and daemons match `X-Registry-Auth`
/// against that exact string — a bare `docker.io` (what an operator
/// naturally types in the credential's Registry field) can be treated
/// as a *different* registry and the credential silently ignored,
/// degrading the pull to anonymous: a private image then fails with
/// "404: pull access denied" even though the credential is valid
/// (#820). Map the Hub aliases (and the empty value, which the
/// credential store documents as "Docker Hub") to the canonical
/// address; any other registry passes through verbatim.
fn canonical_server_address(addr: Option<&str>) -> Option<String> {
    let Some(a) = addr.map(str::trim).filter(|s| !s.is_empty()) else {
        return Some(DOCKER_HUB_REGISTRY.to_string());
    };
    let bare = a
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .trim_end_matches('/');
    if matches!(
        bare,
        "docker.io" | "index.docker.io" | "registry-1.docker.io"
            | "hub.docker.com" | "registry.hub.docker.com"
    ) {
        Some(DOCKER_HUB_REGISTRY.to_string())
    } else {
        Some(a.to_string())
    }
}

/// Non-secret, structured context attached to every pull failure. Keeping
/// this as its own value makes it impossible for the password to be logged by
/// accident: the secret never enters the context at all.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PullLogContext {
    auth_mode: &'static str,
    credential: String,
    registry: String,
    username: String,
}

fn pull_log_context(
    image: &str,
    creds: Option<&ruscker_core::RegistryCredentials>,
) -> PullLogContext {
    match creds {
        Some(creds) => PullLogContext {
            auth_mode: "authenticated",
            credential: creds
                .credential_name
                .clone()
                .unwrap_or_else(|| "inline".to_string()),
            registry: canonical_server_address(creds.server_address.as_deref())
                .unwrap_or_else(|| DOCKER_HUB_REGISTRY.to_string()),
            username: creds.username.clone(),
        },
        None => PullLogContext {
            auth_mode: "anonymous",
            credential: "none".to_string(),
            registry: registry_from_image(image),
            username: "none".to_string(),
        },
    }
}

/// Infer the registry from a Docker image reference when an anonymous pull
/// has no credential server address. Docker treats the first path component
/// as a registry only when it looks like a host (`.`, `:`, or `localhost`);
/// otherwise the reference belongs to Docker Hub.
fn registry_from_image(image: &str) -> String {
    let first = image.split('/').next().unwrap_or_default();
    if image.contains('/')
        && (first == "localhost" || first.contains('.') || first.contains(':'))
    {
        canonical_server_address(Some(first)).unwrap_or_else(|| first.to_string())
    } else {
        DOCKER_HUB_REGISTRY.to_string()
    }
}

fn log_pull_failure(
    image: &str,
    context: &PullLogContext,
    error: impl std::fmt::Display,
) {
    tracing::error!(
        image,
        auth_mode = context.auth_mode,
        credential = %context.credential,
        registry = %context.registry,
        username = %context.username,
        error = %error,
        "image pull failed"
    );
}

/// Operator-facing description of how a pull authenticates — rides in
/// the pull error so "credential ignored / not attached" is diagnosable
/// from the splash message alone (#820).
fn auth_desc(context: &PullLogContext) -> String {
    if context.auth_mode == "authenticated" {
        format!(
            "authenticated as {} @ {}",
            context.username, context.registry
        )
    } else {
        format!("anonymous @ {}", context.registry)
    }
}

fn backend_err(op: &str, e: bollard::errors::Error) -> CoreError {
    CoreError::Backend(format!("{op}: {e}"))
}

/// Docker's cpu_period unit. 100ms is the cgroup v2 default and
/// what `docker run --cpus=N` uses internally. Quoting one value
/// here means our `cpu_quota` math always matches what an
/// operator would see from `docker run`.
const CPU_PERIOD_US: i64 = 100_000;

/// Translate backend-neutral [`ResourceLimits`] into the bollard
/// [`HostConfig`] fields. No-op when limits are empty so the
/// `inspect` output for unlimited containers stays clean.
fn apply_limits(host_config: &mut HostConfig, limits: &ruscker_core::ResourceLimits) {
    if limits.is_empty() {
        return;
    }
    if let Some(bytes) = limits.memory_bytes {
        host_config.memory = Some(bytes);
    }
    if let Some(bytes) = limits.memory_reservation_bytes {
        host_config.memory_reservation = Some(bytes);
    }
    if let Some(cpus) = limits.cpu_fraction {
        // cpu_quota / cpu_period = fractional CPUs. Docker stores
        // both fields; we always set them as a pair so the
        // operator-visible `--cpus` math works out.
        host_config.cpu_period = Some(CPU_PERIOD_US);
        host_config.cpu_quota = Some((cpus * CPU_PERIOD_US as f64) as i64);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `proxy.container-wait-time` reaches the readiness wait (#970):
    /// a 300 ms budget against a port nobody listens on must fail in
    /// well under the 60 s default, and the error names the budget.
    /// Needs no running daemon — the connect refuses immediately and
    /// the crash-detection inspect is best-effort (`.ok()?`).
    #[tokio::test]
    async fn readiness_wait_honours_configured_timeout() {
        let Ok(backend) = LocalDockerBackend::local() else {
            return; // no way to even construct a client handle here
        };
        let backend = backend.with_readiness_timeout(Duration::from_millis(300));
        let addr: SocketAddr = "127.0.0.1:1".parse().unwrap();
        let started = std::time::Instant::now();
        let err = wait_for_ready(&backend, "no-such-container", addr)
            .await
            .expect_err("nothing listens on port 1");
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "must give up near the 300 ms budget, not the 60 s default"
        );
        assert!(
            err.to_string().contains("300ms"),
            "error names the configured budget: {err}"
        );
    }

    /// Zero keeps the default — a 0 ms budget would fail every spawn.
    #[test]
    fn zero_wait_time_keeps_the_default() {
        let Ok(backend) = LocalDockerBackend::local() else {
            return;
        };
        let backend = backend.with_readiness_timeout(Duration::ZERO);
        assert_eq!(backend.readiness_timeout, READINESS_TIMEOUT);
    }

    #[test]
    fn hub_aliases_normalize_to_the_v1_index(){
        let hub = Some("https://index.docker.io/v1/".to_string());
        for a in [None, Some(""), Some("  "), Some("docker.io"), Some("https://docker.io"),
                  Some("index.docker.io"), Some("registry-1.docker.io/"),
                  Some("hub.docker.com"), Some("https://index.docker.io/v1/")] {
            assert_eq!(canonical_server_address(a), hub, "alias {a:?}");
        }
        assert_eq!(
            canonical_server_address(Some("registry.example.com")),
            Some("registry.example.com".to_string()),
            "non-Hub registries pass through"
        );
    }

    #[test]
    fn pull_log_context_names_auth_credential_and_registry_without_password() {
        let anonymous = pull_log_context("ghcr.io/acme/public:latest", None);
        assert_eq!(anonymous.auth_mode, "anonymous");
        assert_eq!(anonymous.credential, "none");
        assert_eq!(anonymous.registry, "ghcr.io");
        assert_eq!(anonymous.username, "none");
        assert_eq!(auth_desc(&anonymous), "anonymous @ ghcr.io");

        let c = ruscker_core::RegistryCredentials {
            username: "milkway".into(),
            password: "never-log-this-password".into(),
            server_address: Some("docker.io".into()),
            credential_name: Some("private-hub".into()),
        };
        let authenticated = pull_log_context("milkway/private", Some(&c));
        assert_eq!(authenticated.auth_mode, "authenticated");
        assert_eq!(authenticated.credential, "private-hub");
        assert_eq!(authenticated.registry, DOCKER_HUB_REGISTRY);
        assert_eq!(authenticated.username, "milkway");
        assert_eq!(
            auth_desc(&authenticated),
            "authenticated as milkway @ https://index.docker.io/v1/"
        );
        assert!(!format!("{authenticated:?}").contains(&c.password));
    }

    #[test]
    fn anonymous_registry_is_inferred_from_the_image_reference() {
        assert_eq!(registry_from_image("ubuntu"), DOCKER_HUB_REGISTRY);
        assert_eq!(registry_from_image("library/nginx"), DOCKER_HUB_REGISTRY);
        assert_eq!(registry_from_image("docker.io/acme/app"), DOCKER_HUB_REGISTRY);
        assert_eq!(registry_from_image("ghcr.io/acme/app"), "ghcr.io");
        assert_eq!(registry_from_image("localhost:5000/app"), "localhost:5000");
    }

    #[test]
    fn state_from_enum_maps_variants_directly() {
        use ContainerSummaryStateEnum as E;
        assert_eq!(state_from_enum(Some(&E::RUNNING)), ReplicaState::Ready);
        assert_eq!(state_from_enum(Some(&E::CREATED)), ReplicaState::Starting);
        assert_eq!(state_from_enum(Some(&E::RESTARTING)), ReplicaState::Starting);
        assert_eq!(state_from_enum(Some(&E::PAUSED)), ReplicaState::Draining);
        assert_eq!(state_from_enum(Some(&E::EXITED)), ReplicaState::Stopped);
        assert_eq!(state_from_enum(Some(&E::DEAD)), ReplicaState::Stopped);
        assert_eq!(state_from_enum(None), ReplicaState::Starting);
    }

    #[test]
    fn apply_limits_noop_when_empty() {
        let mut hc = HostConfig::default();
        apply_limits(&mut hc, &ruscker_core::ResourceLimits::default());
        assert!(hc.memory.is_none());
        assert!(hc.memory_reservation.is_none());
        assert!(hc.cpu_period.is_none());
        assert!(hc.cpu_quota.is_none());
    }

    #[test]
    fn apply_limits_sets_memory_caps() {
        let mut hc = HostConfig::default();
        apply_limits(
            &mut hc,
            &ruscker_core::ResourceLimits {
                memory_bytes: Some(512 * 1024 * 1024),
                memory_reservation_bytes: Some(256 * 1024 * 1024),
                cpu_fraction: None,
            },
        );
        assert_eq!(hc.memory, Some(512 * 1024 * 1024));
        assert_eq!(hc.memory_reservation, Some(256 * 1024 * 1024));
        assert!(hc.cpu_period.is_none(), "CPU untouched when unset");
    }

    #[test]
    fn apply_limits_translates_cpu_fraction_to_quota() {
        let mut hc = HostConfig::default();
        apply_limits(
            &mut hc,
            &ruscker_core::ResourceLimits {
                memory_bytes: None,
                memory_reservation_bytes: None,
                cpu_fraction: Some(0.5),
            },
        );
        assert_eq!(hc.cpu_period, Some(CPU_PERIOD_US));
        assert_eq!(hc.cpu_quota, Some(CPU_PERIOD_US / 2));
    }

    #[test]
    fn cpu_percent_zero_when_no_delta() {
        let prev = PrevReading {
            cpu_total: 100,
            cpu_system: 1_000,
        };
        // Same readings → zero delta → 0%.
        assert_eq!(cpu_percent_from_delta(prev, 100, 1_000, 4), 0.0);
    }

    #[test]
    fn cpu_percent_handles_half_a_core_on_quad_host() {
        // Container used 1 unit of CPU time while the system
        // observed 4 (4-core machine, 1 second). That's 25% of
        // the total system, but Docker expresses it as
        // 25% × 4 cores = 100% of one core.
        let prev = PrevReading {
            cpu_total: 0,
            cpu_system: 0,
        };
        let pct = cpu_percent_from_delta(prev, 1, 4, 4);
        assert!((pct - 100.0).abs() < 1e-9, "got {pct}");
    }

    #[test]
    fn cpu_percent_full_core_on_quad_host() {
        // Container used as much CPU as the whole system saw —
        // it pegged one core. 100% of system × 4 = 400% Docker-
        // style.
        let prev = PrevReading {
            cpu_total: 0,
            cpu_system: 0,
        };
        let pct = cpu_percent_from_delta(prev, 4, 4, 4);
        assert!((pct - 400.0).abs() < 1e-9, "got {pct}");
    }

    #[test]
    fn cpu_percent_saturates_counter_rollback() {
        // Docker shouldn't ever go backward, but a daemon
        // restart could reset the counters. Saturating
        // subtraction means we just report 0% on that tick
        // instead of producing wild negatives.
        let prev = PrevReading {
            cpu_total: 999,
            cpu_system: 999,
        };
        let pct = cpu_percent_from_delta(prev, 100, 1000, 1);
        assert_eq!(pct, 0.0);
    }

    #[test]
    fn apply_limits_two_cpus() {
        let mut hc = HostConfig::default();
        apply_limits(
            &mut hc,
            &ruscker_core::ResourceLimits {
                memory_bytes: None,
                memory_reservation_bytes: None,
                cpu_fraction: Some(2.0),
            },
        );
        assert_eq!(hc.cpu_period, Some(CPU_PERIOD_US));
        assert_eq!(hc.cpu_quota, Some(CPU_PERIOD_US * 2));
    }

    /// Integration test gated behind `--features docker-it`. Pulls
    /// a tiny image, spawns a container, verifies readiness +
    /// list + stop. Costs ~5 MB pull + ~3 s on first run.
    /// Skipped by default so `cargo test` works on machines
    /// without a Docker daemon.
    #[cfg(feature = "docker-it")]
    #[tokio::test]
    async fn spawn_list_stop_against_real_docker() {
        // Override the image via env var so CI can pick whatever
        // is locally available. `nginx:1.29-alpine` is the default
        // because nginx listens on :80 and exits cleanly on stop.
        let image = std::env::var("RUSCKER_IT_IMAGE")
            .unwrap_or_else(|_| "nginx:1.29-alpine".into());
        let backend = LocalDockerBackend::local().expect("connect docker");
        let replica = backend
            .spawn_with_port("itest-spec", &image, 80)
            .await
            .expect("spawn nginx");
        assert_eq!(replica.spec_id, "itest-spec");
        assert_eq!(replica.state, ReplicaState::Ready);

        // list() should include our replica.
        let listed = backend.list().await.expect("list");
        assert!(listed.iter().any(|r| r.id == replica.id));

        // Record a metrics sample so the CPU-delta baseline lands in
        // `prev_stats`, then assert `stop()` clears it (#325) — otherwise
        // the map grows by one dead entry per recycled container.
        let _ = backend
            .metrics_for_container(&replica.container_id)
            .await
            .expect("metrics");
        assert!(
            backend.prev_stats.contains_key(&replica.container_id),
            "metrics_for_container should seed prev_stats"
        );

        // Clean up.
        backend.stop(&replica.id).await.expect("stop");
        let after = backend.list().await.expect("list after stop");
        assert!(!after.iter().any(|r| r.id == replica.id));
        backend
            .stop(&replica.id)
            .await
            .expect("stopping an already-absent container is idempotent");
        assert!(
            !backend.prev_stats.contains_key(&replica.container_id),
            "stop() must forget the container's metrics baseline (#325)"
        );
    }

    /// `container-env` / `container-cmd` smoke: spawn with both set and
    /// assert the real container's `Config.Env` / `Config.Cmd` carry
    /// them (via `docker inspect`). Uses nginx's own argv as the cmd so
    /// the container still listens on :80 and passes readiness.
    #[cfg(feature = "docker-it")]
    #[tokio::test]
    async fn spawn_injects_env_and_cmd() {
        let image = std::env::var("RUSCKER_IT_IMAGE")
            .unwrap_or_else(|_| "nginx:1.29-alpine".into());
        let backend = LocalDockerBackend::local().expect("connect docker");
        let cmd = vec!["nginx".to_string(), "-g".to_string(), "daemon off;".to_string()];
        let req = ruscker_core::SpawnRequest::new("itest-envcmd", &image)
            .with_port(80)
            .with_env(vec!["RUSCKER_SMOKE=hello".to_string()])
            .with_cmd(cmd.clone());
        let replica = backend.spawn_request(&req).await.expect("spawn");

        let info = backend
            .docker
            .inspect_container(&replica.container_id, None)
            .await
            .expect("inspect");
        let cfg = info.config.expect("container config");
        let env = cfg.env.unwrap_or_default();
        assert!(
            env.iter().any(|e| e == "RUSCKER_SMOKE=hello"),
            "injected env missing from inspect: {env:?}"
        );
        assert_eq!(cfg.cmd, Some(cmd), "cmd override not applied");

        backend.stop(&replica.id).await.expect("stop");
    }

    /// `container-network` (phase 3.5): the backend creates the named
    /// network when it's missing and attaches the container to it.
    /// Verifies via inspect that the container joined the network, then
    /// reaps the container + the network so the test is repeatable.
    #[cfg(feature = "docker-it")]
    #[tokio::test]
    async fn spawn_attaches_to_named_network() {
        let image =
            std::env::var("RUSCKER_IT_IMAGE").unwrap_or_else(|_| "nginx:1.29-alpine".into());
        let backend = LocalDockerBackend::local().expect("connect docker");
        let net = "ruscker-it-net";
        let req = ruscker_core::SpawnRequest::new("itest-net", &image)
            .with_port(80)
            .with_network(net);
        let replica = backend.spawn_request(&req).await.expect("spawn");

        let info = backend
            .docker
            .inspect_container(&replica.container_id, None)
            .await
            .expect("inspect");
        let networks = info
            .network_settings
            .and_then(|ns| ns.networks)
            .unwrap_or_default();
        assert!(
            networks.contains_key(net),
            "container not attached to `{net}`; joined: {:?}",
            networks.keys().collect::<Vec<_>>()
        );

        backend.stop(&replica.id).await.expect("stop");
        // Best-effort cleanup: the container is gone, so the network can
        // be removed — keeps the test idempotent across runs.
        let _ = backend.docker.remove_network(net).await;
    }

    /// `labels` (#851): operator labels are stamped on the container, and
    /// the internal `ruscker.*` labels win on a key collision (an
    /// operator can't clobber the load-bearing ones).
    #[cfg(feature = "docker-it")]
    #[tokio::test]
    async fn spawn_stamps_operator_labels_internal_wins() {
        let image =
            std::env::var("RUSCKER_IT_IMAGE").unwrap_or_else(|_| "nginx:1.29-alpine".into());
        let backend = LocalDockerBackend::local().expect("connect docker");
        let req = ruscker_core::SpawnRequest::new("itest-labels", &image)
            .with_port(80)
            .with_labels(vec![
                ("team".to_string(), "data".to_string()),
                // Attempt to override an internal label — must NOT win.
                (LABEL_SPEC_ID.to_string(), "hacked".to_string()),
            ]);
        let replica = backend.spawn_request(&req).await.expect("spawn");

        let info = backend
            .docker
            .inspect_container(&replica.container_id, None)
            .await
            .expect("inspect");
        let labels = info
            .config
            .and_then(|c| c.labels)
            .unwrap_or_default();
        assert_eq!(labels.get("team").map(String::as_str), Some("data"));
        // Internal label kept its real value, not the operator's "hacked".
        assert_eq!(
            labels.get(LABEL_SPEC_ID).map(String::as_str),
            Some("itest-labels")
        );

        backend.stop(&replica.id).await.expect("stop");
    }

    /// #871: the disk panel must never touch a non-Ruscker container or
    /// its image on a shared host. `remove_container` refuses a container
    /// without our label, and `all_container_image_refs` includes the
    /// foreign container's image so the panel won't flag it "unused".
    #[cfg(feature = "docker-it")]
    #[tokio::test]
    async fn disk_ops_spare_non_ruscker_container_and_image() {
        let image =
            std::env::var("RUSCKER_IT_IMAGE").unwrap_or_else(|_| "nginx:1.29-alpine".into());
        let backend = LocalDockerBackend::local().expect("connect docker");
        backend
            .ensure_image_pulled(&image, None, None)
            .await
            .expect("pull");
        // A plain container with NO ruscker labels (stands in for a
        // ShinyProxy / unrelated container on the host).
        let created = backend
            .docker
            .create_container(
                Some(CreateContainerOptions {
                    name: Some("ruscker-it-foreign".into()),
                    ..Default::default()
                }),
                ContainerCreateBody {
                    image: Some(image.clone()),
                    cmd: Some(vec!["sleep".into(), "30".into()]),
                    ..Default::default()
                },
            )
            .await
            .expect("create foreign container");
        let id = created.id;

        // remove_container must REFUSE it (not Ruscker-labeled).
        let err = backend
            .remove_container(&id)
            .await
            .expect_err("must refuse a non-Ruscker container");
        assert!(err.to_string().contains("non-Ruscker"), "got: {err}");
        assert!(
            backend.docker.inspect_container(&id, None).await.is_ok(),
            "the foreign container must NOT have been removed"
        );

        // Its image shows up in the all-container ref set → the disk panel
        // counts it in-use and won't offer to prune it.
        let refs = backend.all_container_image_refs().await.expect("refs");
        assert!(
            refs.iter().any(|r| r == &image || r.starts_with("sha256:")),
            "foreign image must be in the in-use set: {refs:?}"
        );

        // Cleanup directly (bypassing the guard).
        let _ = backend
            .docker
            .remove_container(
                &id,
                Some(RemoveContainerOptions {
                    force: true,
                    ..Default::default()
                }),
            )
            .await;
    }

    /// #550: a container that crashes on startup (e.g. it can't reach its
    /// DB) fails the spawn *fast* — not after the full readiness timeout —
    /// and the error carries the container's log tail so the operator sees
    /// *why* without re-running `docker run` by hand.
    #[cfg(feature = "docker-it")]
    #[tokio::test]
    async fn spawn_failure_surfaces_exit_and_logs() {
        let image =
            std::env::var("RUSCKER_IT_IMAGE").unwrap_or_else(|_| "nginx:1.29-alpine".into());
        let backend = LocalDockerBackend::local().expect("connect docker");
        // Publish :80 but never listen on it; print a diagnostic line,
        // then exit non-zero a moment later — mimicking an app that halts
        // because it couldn't connect to its database.
        let req = ruscker_core::SpawnRequest::new("itest-crash", &image)
            .with_port(80)
            .with_cmd(vec![
                "sh".into(),
                "-c".into(),
                "echo 'FATAL: could not connect to database'; sleep 1; exit 7".into(),
            ]);
        let started = std::time::Instant::now();
        let err = backend
            .spawn_request(&req)
            .await
            .expect_err("a crashing container must fail the spawn");
        let elapsed = started.elapsed();
        let msg = err.to_string();

        // (A) fails fast — well under the 60 s readiness timeout.
        assert!(
            elapsed < std::time::Duration::from_secs(30),
            "expected fast failure on crash, took {elapsed:?}"
        );
        assert!(
            msg.contains("exited") && msg.contains("code 7"),
            "exit code not reported in error: {msg}"
        );
        // (B) the container's log tail is attached.
        assert!(
            msg.contains("could not connect to database"),
            "container log tail missing from error: {msg}"
        );

        // (C) #733: the failed spawn must not leave its container behind
        // — neither running (each visitor retry would spawn another
        // duplicate, and a restart's reconcile would adopt it as `Ready`)
        // nor stopped (disk litter). The backend reaps it itself, so this
        // doubles as the teardown.
        let leftovers: Vec<_> = backend
            .list_managed_containers()
            .await
            .expect("list managed")
            .into_iter()
            .filter(|c| c.spec_id.as_deref() == Some("itest-crash"))
            .collect();
        assert!(
            leftovers.is_empty(),
            "failed spawn left containers behind: {leftovers:?}"
        );
    }

    /// #823: a container that dies INSTANTLY (no `sleep` — the crash the
    /// operator hit) loses its port binding before the inspect at step 4,
    /// so the old code reported the misleading "no port binding for
    /// 8000/tcp". The spawn error must now name the exit code AND carry
    /// the boot log instead.
    #[cfg(feature = "docker-it")]
    #[tokio::test]
    async fn instant_crash_reports_exit_code_and_log_not_no_port_binding() {
        let image =
            std::env::var("RUSCKER_IT_IMAGE").unwrap_or_else(|_| "alpine:latest".into());
        let backend = LocalDockerBackend::local().expect("connect docker");
        // Publishes :8000, never listens, prints a diagnostic and exits
        // non-zero IMMEDIATELY — like aurora_config() not finding
        // /app/data/config.yml.
        let req = ruscker_core::SpawnRequest::new("itest-instant", &image)
            .with_port(8000)
            .with_cmd(vec![
                "sh".into(),
                "-c".into(),
                "echo 'FATAL: cannot read /app/data/config.yml'; exit 3".into(),
            ]);
        let err = backend
            .spawn_request(&req)
            .await
            .expect_err("an instantly-crashing container must fail the spawn");
        let msg = err.to_string();
        assert!(
            !msg.contains("no port binding"),
            "still reporting the misleading symptom: {msg}"
        );
        assert!(
            msg.contains("exited (code 3)"),
            "exit code missing from error: {msg}"
        );
        assert!(
            msg.contains("config.yml"),
            "boot log tail missing from error: {msg}"
        );
        // Self-cleaning: the failed spawn reaps its own container (#733).
        let leftovers: Vec<_> = backend
            .list_managed_containers()
            .await
            .expect("list managed")
            .into_iter()
            .filter(|c| c.spec_id.as_deref() == Some("itest-instant"))
            .collect();
        assert!(leftovers.is_empty(), "left containers behind: {leftovers:?}");
    }

    /// #453 part B: the disk-panel methods see a live replica + its image
    /// and `remove_container` reaps it. Self-cleaning (the removal is the
    /// teardown).
    #[cfg(feature = "docker-it")]
    #[tokio::test]
    async fn disk_lists_and_removes_managed_resources() {
        let image =
            std::env::var("RUSCKER_IT_IMAGE").unwrap_or_else(|_| "nginx:1.29-alpine".into());
        let backend = LocalDockerBackend::local().expect("connect docker");
        let replica = backend
            .spawn_with_port("disk-itest", &image, 80)
            .await
            .expect("spawn");

        // list_managed_containers includes our running replica.
        let managed = backend
            .list_managed_containers()
            .await
            .expect("list managed");
        let ours = managed
            .iter()
            .find(|c| c.id == replica.container_id)
            .expect("our container listed");
        assert!(ours.running, "freshly spawned container should be running");
        assert_eq!(ours.spec_id.as_deref(), Some("disk-itest"));

        // list_images carries real sizes.
        let images = backend.list_images().await.expect("list images");
        assert!(
            images.iter().any(|i| i.size_bytes > 0),
            "images should carry sizes"
        );

        // remove_container reaps it (and serves as teardown).
        backend
            .remove_container(&replica.container_id)
            .await
            .expect("remove container");
        let after = backend
            .list_managed_containers()
            .await
            .expect("list after");
        assert!(
            !after.iter().any(|c| c.id == replica.container_id),
            "container should be gone after remove"
        );
    }

    /// #986 slice A: a run-to-completion job against the real daemon.
    /// A succeeding job returns exit 0 + its stdout in the tail; a
    /// FAILING job is a valid JobOutcome (exit code preserved), never
    /// an Err; and in both cases the container is removed afterwards.
    #[cfg(feature = "docker-it")]
    #[tokio::test]
    async fn run_job_captures_exit_and_logs_and_reaps() {
        use ruscker_core::ContainerBackend as _;
        let backend = LocalDockerBackend::local().expect("connect docker");

        let mut req = ruscker_core::SpawnRequest::new("it-job", "busybox:latest");
        req.cmd = Some(vec![
            "sh".into(),
            "-c".into(),
            "echo etl-ok; exit 0".into(),
        ]);
        let out = backend.run_job(&req).await.expect("job ran");
        assert_eq!(out.exit_code, 0);
        assert!(
            out.log_tail.iter().any(|l| l.contains("etl-ok")),
            "stdout captured: {:?}",
            out.log_tail
        );

        // Non-zero exit: reported, not an error.
        req.cmd = Some(vec!["sh".into(), "-c".into(), "echo boom >&2; exit 3".into()]);
        let out = backend.run_job(&req).await.expect("failing job still ran");
        assert_eq!(out.exit_code, 3);
        assert!(out.log_tail.iter().any(|l| l.contains("boom")));

        // No stopped job containers linger.
        let opts = ListContainersOptions {
            all: true,
            filters: Some(HashMap::from([(
                "label".to_string(),
                vec![format!("{LABEL_JOB}=true")],
            )])),
            ..Default::default()
        };
        let leftover = backend.docker.list_containers(Some(opts)).await.unwrap();
        assert!(leftover.is_empty(), "job containers must be removed");
    }

    /// #987: named-volume roundtrip against the real daemon — create
    /// (labelled `ruscker.created`) → list (refs == 0, ours) → remove →
    /// gone. Reaps a leftover from a previous crashed run first, so the
    /// test is idempotent.
    #[cfg(feature = "docker-it")]
    #[tokio::test]
    async fn volume_create_list_remove_roundtrip() {
        let backend = LocalDockerBackend::local().expect("connect docker");
        let name = "ruscker-it-vol-test";

        // Idempotence: a run that died mid-test may have left the
        // volume behind — best-effort reap before starting.
        let _ = backend.remove_volume(name).await;

        backend.create_volume(name).await.expect("create volume");
        let listed = backend.list_volumes().await.expect("list volumes");
        let v = listed
            .iter()
            .find(|v| v.name == name)
            .expect("created volume listed");
        assert_eq!(v.refs, 0, "fresh volume has no container references");
        assert!(v.ruscker_created, "create_volume must stamp ruscker.created");

        backend.remove_volume(name).await.expect("remove volume");
        let after = backend.list_volumes().await.expect("list after remove");
        assert!(
            !after.iter().any(|v| v.name == name),
            "volume should be gone after remove"
        );
    }
}
