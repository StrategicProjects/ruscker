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

#![allow(dead_code)]

use async_trait::async_trait;
use bollard::models::{ContainerCreateBody, HostConfig, PortBinding};
use bollard::query_parameters::{
    CreateContainerOptions, CreateImageOptions, ListContainersOptions, LogsOptionsBuilder,
    RemoveContainerOptions, StartContainerOptions, StatsOptionsBuilder, StopContainerOptions,
};
pub mod multihost;
pub use multihost::MultiHostDockerBackend;

use bollard::Docker;
use chrono::Utc;
use dashmap::DashMap;
use futures_util::StreamExt;
use ruscker_core::{
    ContainerBackend, CoreError, CoreResult, Replica, ReplicaId, ReplicaMetrics, ReplicaState,
};
use std::collections::HashMap;
use std::net::{SocketAddr, ToSocketAddrs};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio::time::sleep;

pub const LABEL_SPEC_ID: &str = "ruscker.spec_id";
pub const LABEL_REPLICA_ID: &str = "ruscker.replica_id";
pub const LABEL_INNER_PORT: &str = "ruscker.inner_port";

/// Default inner port if the spec doesn't tell us — matches the
/// Shiny Server default. APIs override via the `inner_port`
/// argument to [`LocalDockerBackend::spawn_with_port`].
pub const DEFAULT_INNER_PORT: u16 = 3838;

/// How long to wait for a freshly-started container to accept a
/// TCP connection on its bound port. Mirrors the ShinyProxy
/// `container-wait-time` default of 60 s.
pub const READINESS_TIMEOUT: Duration = Duration::from_secs(60);

/// How often to retry the TCP connect during readiness polling.
pub const READINESS_INTERVAL: Duration = Duration::from_millis(250);

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
        }
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

        // 1. Pull image (idempotent — Docker no-ops when local).
        //    Errors bubble up through the stream-of-events.
        self.ensure_image_pulled(&req.image, req.creds.as_ref()).await?;

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

        let body = ContainerCreateBody {
            image: Some(req.image.clone()),
            // bollard 0.21 takes a flat Vec<String> here, not a
            // HashMap — list of "<port>/<proto>" strings.
            exposed_ports: Some(vec![port_key.clone()]),
            labels: Some(labels),
            host_config: Some(host_config),
            ..Default::default()
        };

        let container_name = format!("ruscker-{}-{}", req.spec_id, &replica_id.to_string()[..8]);
        let opts = CreateContainerOptions {
            name: Some(container_name.clone()),
            ..Default::default()
        };
        let created = self
            .docker
            .create_container(Some(opts), body)
            .await
            .map_err(|e| backend_err("create container", e))?;
        let container_id = created.id;

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

        // 5. Wait for the container's process to bind that port.
        wait_for_ready(upstream).await?;

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
    ) -> CoreResult<()> {
        // bollard's create_image always hits the registry for the
        // manifest — even when the layers are already local. Skip
        // the round-trip when the image is present, mirroring the
        // operator's expectation of "no-op when local". A flaky
        // network shouldn't stop a container that's already on
        // the host.
        if self.docker.inspect_image(image).await.is_ok() {
            tracing::debug!(image, "image present locally; skipping pull");
            return Ok(());
        }
        let opts = CreateImageOptions {
            from_image: Some(image.to_string()),
            ..Default::default()
        };
        // Convert our backend-neutral creds to bollard's native
        // type only at the pull boundary. `None` keeps the pull
        // anonymous (Docker Hub public images).
        let bollard_creds = creds.map(|c| bollard::auth::DockerCredentials {
            username: Some(c.username.clone()),
            password: Some(c.password.clone()),
            serveraddress: c.server_address.clone(),
            ..Default::default()
        });
        tracing::info!(
            image,
            with_creds = bollard_creds.is_some(),
            registry = ?creds.and_then(|c| c.server_address.as_deref()),
            "pulling image"
        );
        let mut stream = self.docker.create_image(Some(opts), None, bollard_creds);
        while let Some(event) = stream.next().await {
            event.map_err(|e| backend_err("pull image", e))?;
        }
        Ok(())
    }

    async fn bound_host_port(&self, container_id: &str, port_key: &str) -> CoreResult<u16> {
        let info = self
            .docker
            .inspect_container(container_id, None)
            .await
            .map_err(|e| backend_err("inspect container", e))?;
        let bindings = info
            .network_settings
            .as_ref()
            .and_then(|ns| ns.ports.as_ref())
            .and_then(|p| p.get(port_key).cloned())
            .ok_or_else(|| {
                CoreError::Backend(format!("no port binding for {port_key} on {container_id}"))
            })?;
        let port_str = bindings
            .as_ref()
            .and_then(|v| v.first())
            .and_then(|pb| pb.host_port.as_ref())
            .ok_or_else(|| CoreError::Backend("port binding missing host_port".into()))?;
        port_str
            .parse::<u16>()
            .map_err(|e| CoreError::Backend(format!("parse host_port `{port_str}`: {e}")))
    }

    async fn container_id_for_replica(&self, replica_id: &ReplicaId) -> CoreResult<String> {
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
        list.first()
            .and_then(|c| c.id.clone())
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
        let container_id = self.container_id_for_replica(replica_id).await?;
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

            let state = state_from_docker(c.state.as_ref().map(|s| format!("{s:?}").to_ascii_lowercase()).as_deref());
            replicas.push(Replica {
                id: replica_id,
                spec_id,
                container_id: c.id.unwrap_or_default(),
                upstream,
                state,
                started_at: Utc::now(), // exact value would need inspect()
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
async fn wait_for_ready(addr: SocketAddr) -> CoreResult<()> {
    let deadline = std::time::Instant::now() + READINESS_TIMEOUT;

    // Phase 1: TCP connect.
    loop {
        match TcpStream::connect(addr).await {
            Ok(_) => break,
            Err(_) if std::time::Instant::now() < deadline => {
                sleep(READINESS_INTERVAL).await;
            }
            Err(e) => {
                return Err(CoreError::Backend(format!(
                    "container at {addr} never accepted TCP within {:?}: {e}",
                    READINESS_TIMEOUT
                )));
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
            _ if std::time::Instant::now() < deadline => {
                sleep(READINESS_INTERVAL).await;
            }
            _ => {
                return Err(CoreError::Backend(format!(
                    "container at {addr} accepted TCP but never answered HTTP within {:?}",
                    READINESS_TIMEOUT
                )));
            }
        }
    }
}

fn state_from_docker(s: Option<&str>) -> ReplicaState {
    match s {
        Some(s) if s.contains("running") => ReplicaState::Ready,
        Some(s) if s.contains("created") || s.contains("restarting") => ReplicaState::Starting,
        Some(s) if s.contains("paused") => ReplicaState::Draining,
        Some(s) if s.contains("exited") || s.contains("dead") || s.contains("removing") => {
            ReplicaState::Stopped
        }
        _ => ReplicaState::Starting,
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

    #[test]
    fn state_from_docker_maps_common_values() {
        assert_eq!(state_from_docker(Some("running")), ReplicaState::Ready);
        assert_eq!(state_from_docker(Some("created")), ReplicaState::Starting);
        assert_eq!(state_from_docker(Some("paused")), ReplicaState::Draining);
        assert_eq!(state_from_docker(Some("exited")), ReplicaState::Stopped);
        assert_eq!(state_from_docker(None), ReplicaState::Starting);
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

        // Clean up.
        backend.stop(&replica.id).await.expect("stop");
        let after = backend.list().await.expect("list after stop");
        assert!(!after.iter().any(|r| r.id == replica.id));
    }
}
