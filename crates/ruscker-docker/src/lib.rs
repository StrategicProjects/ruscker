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
    CreateContainerOptions, CreateImageOptions, ListContainersOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions,
};
use bollard::Docker;
use chrono::Utc;
use futures_util::StreamExt;
use ruscker_core::{
    ContainerBackend, CoreError, CoreResult, Replica, ReplicaId, ReplicaMetrics, ReplicaState,
};
use std::collections::HashMap;
use std::net::SocketAddr;
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
}

impl LocalDockerBackend {
    /// Connect to the local Docker daemon. On Unix this is
    /// typically `/var/run/docker.sock`; on macOS / Windows
    /// bollard's `connect_with_local_defaults` finds the right
    /// path (Docker Desktop, Colima, OrbStack, …).
    pub fn local() -> CoreResult<Self> {
        let docker = Docker::connect_with_local_defaults()
            .map_err(|e| CoreError::Backend(format!("connect to docker socket: {e}")))?;
        Ok(Self { docker })
    }

    /// Build over an existing bollard connection — used by tests
    /// to inject a stub via testcontainers-rs.
    pub fn from_docker(docker: Docker) -> Self {
        Self { docker }
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
        let replica_id = ReplicaId::new();

        // 1. Pull image (idempotent — Docker no-ops when local).
        //    Errors bubble up through the stream-of-events.
        self.ensure_image_pulled(image).await?;

        // 2. Create container with our labels + ephemeral host port.
        let port_key = format!("{inner_port}/tcp");
        let mut port_bindings = HashMap::new();
        port_bindings.insert(
            port_key.clone(),
            Some(vec![PortBinding {
                host_ip: Some("127.0.0.1".to_string()),
                host_port: Some(String::new()), // "" => ephemeral
            }]),
        );

        let mut labels = HashMap::new();
        labels.insert(LABEL_SPEC_ID.to_string(), spec_id.to_string());
        labels.insert(LABEL_REPLICA_ID.to_string(), replica_id.to_string());
        labels.insert(LABEL_INNER_PORT.to_string(), inner_port.to_string());

        let body = ContainerCreateBody {
            image: Some(image.to_string()),
            // bollard 0.21 takes a flat Vec<String> here, not a
            // HashMap — list of "<port>/<proto>" strings.
            exposed_ports: Some(vec![port_key.clone()]),
            labels: Some(labels),
            host_config: Some(HostConfig {
                port_bindings: Some(port_bindings),
                ..Default::default()
            }),
            ..Default::default()
        };

        let container_name = format!("ruscker-{spec_id}-{}", &replica_id.to_string()[..8]);
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
        let upstream: SocketAddr = format!("127.0.0.1:{bound}")
            .parse()
            .map_err(|e| CoreError::Backend(format!("parse upstream addr: {e}")))?;

        // 5. Wait for the container's process to bind that port.
        wait_for_ready(upstream).await?;

        Ok(Replica {
            id: replica_id,
            spec_id: spec_id.to_string(),
            container_id,
            upstream,
            state: ReplicaState::Ready,
            started_at: Utc::now(),
            sessions_active: 0,
            sessions_max: 0,
        })
    }

    async fn ensure_image_pulled(&self, image: &str) -> CoreResult<()> {
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
        // None for credentials — anonymous public pulls only for
        // now; private registries come through the credentials
        // store in a follow-up.
        let mut stream = self.docker.create_image(Some(opts), None, None);
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
        self.spawn_with_port(spec_id, image, DEFAULT_INNER_PORT).await
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
                    .and_then(|p| p.public_port.map(|n| n as u16))
            });
            let upstream: SocketAddr = match host_port {
                Some(p) => format!("127.0.0.1:{p}").parse().unwrap(),
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
            });
        }
        Ok(replicas)
    }

    async fn metrics(&self, _replica_id: &ReplicaId) -> CoreResult<ReplicaMetrics> {
        // TODO(phase-4): stream stats via bollard::Docker::stats().
        // The dashboard ships in phase 4; the proxy itself doesn't
        // need metrics to function.
        Err(CoreError::Backend(
            "metrics() not implemented until phase 4".into(),
        ))
    }
}

/// Poll a TCP connect against `addr` until success or
/// [`READINESS_TIMEOUT`]. Doesn't issue an HTTP health-check —
/// that's a per-spec concern (some Shiny apps take a few seconds
/// to render the first response after the TCP socket is up).
async fn wait_for_ready(addr: SocketAddr) -> CoreResult<()> {
    let deadline = std::time::Instant::now() + READINESS_TIMEOUT;
    loop {
        match TcpStream::connect(addr).await {
            Ok(_) => return Ok(()),
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
