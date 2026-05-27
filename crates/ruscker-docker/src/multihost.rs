//! Multi-host Docker backend (Phase 6).
//!
//! Holds one [`LocalDockerBackend`] per configured host — each wrapping
//! a bollard connection to that daemon (`ssh://`, `tcp://`+TLS,
//! `http://`, or `unix://`) and configured with the right addressing so
//! the proxy reaches containers on the remote host. Implements the same
//! [`ContainerBackend`] trait, so it drops in wherever the single-host
//! backend went.
//!
//! 6a covers connection + spawn/list/stop/metrics/logs routing with a
//! simple least-loaded placement. Richer placement (spread/bin-pack,
//! anti-affinity, capacity caps) is 6c.

use async_trait::async_trait;
use bollard::{Docker, API_DEFAULT_VERSION};
use dashmap::DashMap;
use ruscker_config::Host;
use ruscker_core::{
    ContainerBackend, CoreError, CoreResult, LogStream, Replica, ReplicaId, ReplicaMetrics,
    SpawnRequest,
};

use crate::LocalDockerBackend;

/// Connection timeout for remote daemons, seconds (bollard default).
const CONNECT_TIMEOUT: u64 = 120;

/// Connect to a configured [`Host`], returning a [`LocalDockerBackend`]
/// wired with the right addressing:
/// - `unix://` → local daemon, publish + proxy on `127.0.0.1`;
/// - `ssh://` / `tcp://` / `http://` → remote daemon, publish on
///   `0.0.0.0` (so the proxy can reach it) and proxy to the host's
///   reachable address.
pub fn connect_host(host: &Host) -> CoreResult<LocalDockerBackend> {
    let addr = host.address.trim();
    let map_err = |e: bollard::errors::Error| {
        CoreError::Backend(format!("connect host `{}` ({addr}): {e}", host.id))
    };

    if addr.starts_with("unix://") {
        let docker = Docker::connect_with_unix(addr, CONNECT_TIMEOUT, API_DEFAULT_VERSION)
            .map_err(map_err)?;
        Ok(LocalDockerBackend::from_docker(docker))
    } else if let Some(rest) = addr.strip_prefix("ssh://") {
        let docker = Docker::connect_with_ssh(addr, CONNECT_TIMEOUT, API_DEFAULT_VERSION, None)
            .map_err(map_err)?;
        Ok(LocalDockerBackend::from_docker_addressed(
            docker,
            "0.0.0.0",
            host_part(rest),
        ))
    } else if let Some(rest) = addr.strip_prefix("tcp://") {
        let tls = host.tls.as_ref().ok_or_else(|| {
            CoreError::Backend(format!(
                "host `{}`: tcp:// needs `tls` (ca/cert/key)",
                host.id
            ))
        })?;
        let docker = Docker::connect_with_ssl(
            addr,
            &tls.key,
            &tls.cert,
            &tls.ca,
            CONNECT_TIMEOUT,
            API_DEFAULT_VERSION,
        )
        .map_err(map_err)?;
        Ok(LocalDockerBackend::from_docker_addressed(
            docker,
            "0.0.0.0",
            host_part(rest),
        ))
    } else if let Some(rest) = addr.strip_prefix("http://") {
        let docker = Docker::connect_with_http(addr, CONNECT_TIMEOUT, API_DEFAULT_VERSION)
            .map_err(map_err)?;
        Ok(LocalDockerBackend::from_docker_addressed(
            docker,
            "0.0.0.0",
            host_part(rest),
        ))
    } else {
        Err(CoreError::Backend(format!(
            "host `{}`: address `{addr}` must start with ssh:// , tcp:// , http:// or unix://",
            host.id
        )))
    }
}

/// Reachable host from an `[user@]host[:port]` authority — drops the
/// `user@` and the `:port`. (IPv6 literals aren't handled yet.)
fn host_part(authority: &str) -> String {
    let no_user = authority.rsplit('@').next().unwrap_or(authority);
    no_user.split(':').next().unwrap_or(no_user).to_string()
}

/// Backend that schedules containers across several Docker hosts.
pub struct MultiHostDockerBackend {
    /// host id → per-host backend, in config order.
    hosts: Vec<(String, LocalDockerBackend)>,
    /// replica id → host id, so `stop`/`metrics`/`logs` reach the right
    /// daemon. Populated on `spawn` and refreshed on `list`.
    placement: DashMap<ReplicaId, String>,
}

impl MultiHostDockerBackend {
    /// Connect to every configured host. Fails if the list is empty or
    /// any host can't be built (bad address / TLS).
    pub fn connect(hosts: &[Host]) -> CoreResult<Self> {
        if hosts.is_empty() {
            return Err(CoreError::Backend("no docker hosts configured".into()));
        }
        let mut built = Vec::with_capacity(hosts.len());
        for h in hosts {
            built.push((h.id.clone(), connect_host(h)?));
        }
        Ok(Self {
            hosts: built,
            placement: DashMap::new(),
        })
    }

    /// Pick the least-loaded host (fewest currently-placed replicas).
    /// Simple spread; richer placement is 6c.
    fn pick_host(&self) -> &(String, LocalDockerBackend) {
        let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
        for kv in self.placement.iter() {
            *counts.entry(kv.value().clone()).or_insert(0) += 1;
        }
        self.hosts
            .iter()
            .min_by_key(|(id, _)| counts.get(id).copied().unwrap_or(0))
            .expect("connect() guarantees >=1 host")
    }

    /// The backend a replica lives on, by recorded placement.
    fn placed(&self, id: &ReplicaId) -> Option<&LocalDockerBackend> {
        let host = self.placement.get(id)?;
        self.hosts
            .iter()
            .find(|(hid, _)| *hid == *host.value())
            .map(|(_, b)| b)
    }
}

#[async_trait]
impl ContainerBackend for MultiHostDockerBackend {
    async fn spawn(&self, spec_id: &str, image: &str) -> CoreResult<Replica> {
        self.spawn_request(&SpawnRequest::new(spec_id, image)).await
    }

    async fn spawn_request(&self, req: &SpawnRequest) -> CoreResult<Replica> {
        let (host_id, backend) = self.pick_host();
        let mut replica = backend.spawn_request(req).await?;
        replica.host = Some(host_id.clone());
        self.placement.insert(replica.id.clone(), host_id.clone());
        tracing::info!(host = %host_id, replica = %replica.id, spec = %req.spec_id, "spawned on host");
        Ok(replica)
    }

    async fn stop(&self, replica_id: &ReplicaId) -> CoreResult<()> {
        if let Some(backend) = self.placed(replica_id) {
            let r = backend.stop(replica_id).await;
            self.placement.remove(replica_id);
            return r;
        }
        // Placement miss (e.g. never listed): best-effort across hosts.
        for (_, backend) in &self.hosts {
            if backend.stop(replica_id).await.is_ok() {
                self.placement.remove(replica_id);
                return Ok(());
            }
        }
        Err(CoreError::Backend(format!(
            "replica {replica_id} not found on any host"
        )))
    }

    async fn list(&self) -> CoreResult<Vec<Replica>> {
        // Fan out over every host; tag each replica's placement so
        // later stop/metrics/logs route correctly. A host that fails to
        // answer is logged and skipped (degraded, not fatal).
        let mut all = Vec::new();
        for (host_id, backend) in &self.hosts {
            match backend.list().await {
                Ok(mut replicas) => {
                    for r in &mut replicas {
                        self.placement.insert(r.id.clone(), host_id.clone());
                        r.host = Some(host_id.clone());
                    }
                    all.extend(replicas);
                }
                Err(e) => {
                    tracing::warn!(host = %host_id, error = %e, "list on host failed; skipping");
                }
            }
        }
        Ok(all)
    }

    async fn metrics(&self, replica_id: &ReplicaId) -> CoreResult<ReplicaMetrics> {
        if let Some(backend) = self.placed(replica_id) {
            return backend.metrics(replica_id).await;
        }
        for (_, backend) in &self.hosts {
            if let Ok(m) = backend.metrics(replica_id).await {
                return Ok(m);
            }
        }
        Err(CoreError::Backend(format!(
            "replica {replica_id} not found on any host"
        )))
    }

    async fn logs(&self, replica_id: &ReplicaId, tail: usize) -> CoreResult<Vec<String>> {
        if let Some(backend) = self.placed(replica_id) {
            return backend.logs(replica_id, tail).await;
        }
        Err(CoreError::Backend(format!(
            "replica {replica_id} not found on any host"
        )))
    }

    async fn logs_follow(&self, replica_id: &ReplicaId, tail: usize) -> CoreResult<LogStream> {
        if let Some(backend) = self.placed(replica_id) {
            return backend.logs_follow(replica_id, tail).await;
        }
        Err(CoreError::Backend(format!(
            "replica {replica_id} not found on any host"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruscker_config::{Host, HostTls};
    use std::path::PathBuf;

    fn host(id: &str, address: &str) -> Host {
        Host {
            id: id.into(),
            address: address.into(),
            tls: None,
            max_containers: None,
            weight: None,
        }
    }

    #[test]
    fn host_part_strips_user_and_port() {
        assert_eq!(host_part("ops@10.0.0.11"), "10.0.0.11");
        assert_eq!(host_part("10.0.0.12:2376"), "10.0.0.12");
        assert_eq!(host_part("ops@host.example:2376"), "host.example");
        assert_eq!(host_part("plainhost"), "plainhost");
    }

    // `LocalDockerBackend` isn't `Debug`, so extract the error message
    // by hand rather than `unwrap_err()`.
    fn err_of(r: CoreResult<LocalDockerBackend>) -> String {
        match r {
            Ok(_) => panic!("expected an error"),
            Err(e) => format!("{e}"),
        }
    }

    #[test]
    fn connect_rejects_unknown_scheme() {
        assert!(err_of(connect_host(&host("x", "rdp://nope"))).contains("must start with"));
    }

    #[test]
    fn connect_tcp_without_tls_errors() {
        assert!(err_of(connect_host(&host("x", "tcp://h:2376"))).contains("needs `tls`"));
    }

    #[test]
    fn connect_empty_hosts_errors() {
        assert!(MultiHostDockerBackend::connect(&[]).is_err());
    }

    #[test]
    fn tls_struct_builds() {
        // Sanity: a tcp host with tls parses far enough to attempt a
        // connection (which we don't exercise here — no daemon).
        let h = Host {
            id: "tcp".into(),
            address: "tcp://10.0.0.1:2376".into(),
            tls: Some(HostTls {
                ca: PathBuf::from("/ca"),
                cert: PathBuf::from("/cert"),
                key: PathBuf::from("/key"),
            }),
            max_containers: None,
            weight: None,
        };
        // connect_with_ssl is lazy about the socket but reads the cert
        // files eagerly; with bogus paths it errors — that's fine, we're
        // only asserting we reached the tcp branch (not the scheme error).
        assert!(!err_of(connect_host(&h)).contains("must start with"));
    }
}
