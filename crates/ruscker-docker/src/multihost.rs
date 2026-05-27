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
use ruscker_config::{Host, Placement};
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

/// A connected host plus the placement metadata from its config.
struct HostEntry {
    id: String,
    backend: LocalDockerBackend,
    max_containers: Option<u32>,
    weight: u32,
}

/// Backend that schedules containers across several Docker hosts.
pub struct MultiHostDockerBackend {
    /// Connected hosts, in config order.
    hosts: Vec<HostEntry>,
    /// replica id → (host id, spec id). The host id routes
    /// `stop`/`metrics`/`logs`; the spec id powers anti-affinity (how
    /// many of a spec already run on each host). Populated on `spawn`
    /// and refreshed on `list`.
    placement: DashMap<ReplicaId, (String, String)>,
}

/// One host's current load — the input to the pure placement decision.
#[derive(Debug, Clone)]
struct HostLoad {
    count: usize,
    max: Option<u32>,
    weight: u32,
    runs_spec: bool,
}

/// Choose the index of the host to spawn on, or `None` if every host is
/// at its `max_containers` capacity. Pure (no I/O), so it's unit-tested
/// directly:
/// - capacity caps exclude full hosts;
/// - anti-affinity prefers hosts not already running the spec, falling
///   back to all eligible hosts rather than refusing to scale;
/// - `Spread` picks the weighted least-loaded host; `BinPack` fills the
///   fullest host that still has room. Ties break to the lowest index.
fn choose_host(loads: &[HostLoad], placement: Placement, anti_affinity: bool) -> Option<usize> {
    let eligible: Vec<usize> = (0..loads.len())
        .filter(|&i| loads[i].max.is_none_or(|m| (loads[i].count as u32) < m))
        .collect();
    if eligible.is_empty() {
        return None;
    }
    let pool: Vec<usize> = if anti_affinity {
        let free: Vec<usize> = eligible
            .iter()
            .copied()
            .filter(|&i| !loads[i].runs_spec)
            .collect();
        if free.is_empty() {
            eligible
        } else {
            free
        }
    } else {
        eligible
    };
    match placement {
        Placement::Spread => pool.into_iter().min_by(|&a, &b| {
            let la = loads[a].count as f64 / loads[a].weight.max(1) as f64;
            let lb = loads[b].count as f64 / loads[b].weight.max(1) as f64;
            la.partial_cmp(&lb)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then(a.cmp(&b))
        }),
        Placement::BinPack => pool
            .into_iter()
            .max_by(|&a, &b| loads[a].count.cmp(&loads[b].count).then(b.cmp(&a))),
    }
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
            built.push(HostEntry {
                id: h.id.clone(),
                backend: connect_host(h)?,
                max_containers: h.max_containers,
                weight: h.weight.unwrap_or(1).max(1),
            });
        }
        Ok(Self {
            hosts: built,
            placement: DashMap::new(),
        })
    }

    /// Choose a host for `req` honouring its placement strategy,
    /// per-host capacity caps, and anti-affinity. Errors only when every
    /// host is full.
    fn pick_host(&self, req: &SpawnRequest) -> CoreResult<&HostEntry> {
        let idx_of: std::collections::HashMap<&str, usize> = self
            .hosts
            .iter()
            .enumerate()
            .map(|(i, h)| (h.id.as_str(), i))
            .collect();
        let mut counts = vec![0usize; self.hosts.len()];
        let mut runs = vec![false; self.hosts.len()];
        for kv in self.placement.iter() {
            let (host, spec) = kv.value();
            if let Some(&i) = idx_of.get(host.as_str()) {
                counts[i] += 1;
                if *spec == req.spec_id {
                    runs[i] = true;
                }
            }
        }
        let loads: Vec<HostLoad> = self
            .hosts
            .iter()
            .enumerate()
            .map(|(i, h)| HostLoad {
                count: counts[i],
                max: h.max_containers,
                weight: h.weight,
                runs_spec: runs[i],
            })
            .collect();
        match choose_host(&loads, req.placement, req.anti_affinity) {
            Some(i) => Ok(&self.hosts[i]),
            None => Err(CoreError::Backend("all docker hosts at capacity".into())),
        }
    }

    /// The backend a replica lives on, by recorded placement.
    fn placed(&self, id: &ReplicaId) -> Option<&LocalDockerBackend> {
        let entry = self.placement.get(id)?;
        let host = entry.value().0.clone();
        self.hosts.iter().find(|h| h.id == host).map(|h| &h.backend)
    }
}

#[async_trait]
impl ContainerBackend for MultiHostDockerBackend {
    async fn spawn(&self, spec_id: &str, image: &str) -> CoreResult<Replica> {
        self.spawn_request(&SpawnRequest::new(spec_id, image)).await
    }

    async fn spawn_request(&self, req: &SpawnRequest) -> CoreResult<Replica> {
        let entry = self.pick_host(req)?;
        let mut replica = entry.backend.spawn_request(req).await?;
        replica.host = Some(entry.id.clone());
        self.placement
            .insert(replica.id.clone(), (entry.id.clone(), req.spec_id.clone()));
        tracing::info!(host = %entry.id, replica = %replica.id, spec = %req.spec_id, "spawned on host");
        Ok(replica)
    }

    async fn stop(&self, replica_id: &ReplicaId) -> CoreResult<()> {
        if let Some(backend) = self.placed(replica_id) {
            let r = backend.stop(replica_id).await;
            self.placement.remove(replica_id);
            return r;
        }
        // Placement miss (e.g. never listed): best-effort across hosts.
        for h in &self.hosts {
            if h.backend.stop(replica_id).await.is_ok() {
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
        for h in &self.hosts {
            match h.backend.list().await {
                Ok(mut replicas) => {
                    for r in &mut replicas {
                        self.placement
                            .insert(r.id.clone(), (h.id.clone(), r.spec_id.clone()));
                        r.host = Some(h.id.clone());
                    }
                    all.extend(replicas);
                }
                Err(e) => {
                    tracing::warn!(host = %h.id, error = %e, "list on host failed; skipping");
                }
            }
        }
        Ok(all)
    }

    async fn metrics(&self, replica_id: &ReplicaId) -> CoreResult<ReplicaMetrics> {
        if let Some(backend) = self.placed(replica_id) {
            return backend.metrics(replica_id).await;
        }
        for h in &self.hosts {
            if let Ok(m) = h.backend.metrics(replica_id).await {
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

    fn load(count: usize, max: Option<u32>, weight: u32, runs_spec: bool) -> HostLoad {
        HostLoad {
            count,
            max,
            weight,
            runs_spec,
        }
    }

    #[test]
    fn spread_picks_weighted_least_loaded() {
        // host0: 3 containers, host1: 1 → spread picks host1.
        let loads = [load(3, None, 1, false), load(1, None, 1, false)];
        assert_eq!(choose_host(&loads, Placement::Spread, false), Some(1));
        // Weight: host0 has 4 but weight 4 (eff 1.0); host1 has 2 weight 1
        // (eff 2.0) → host0 wins.
        let loads = [load(4, None, 4, false), load(2, None, 1, false)];
        assert_eq!(choose_host(&loads, Placement::Spread, false), Some(0));
        // Tie → lowest index.
        let loads = [load(2, None, 1, false), load(2, None, 1, false)];
        assert_eq!(choose_host(&loads, Placement::Spread, false), Some(0));
    }

    #[test]
    fn binpack_fills_fullest_with_room() {
        // host0: 4/5, host1: 1/5 → bin-pack tops up host0.
        let loads = [load(4, Some(5), 1, false), load(1, Some(5), 1, false)];
        assert_eq!(choose_host(&loads, Placement::BinPack, false), Some(0));
    }

    #[test]
    fn capacity_caps_exclude_full_hosts() {
        // host0 full (5/5) ⇒ spread must pick host1 even though host1 is
        // more loaded than an (ineligible) full host.
        let loads = [load(5, Some(5), 1, false), load(2, Some(5), 1, false)];
        assert_eq!(choose_host(&loads, Placement::Spread, false), Some(1));
        // Every host full ⇒ None.
        let loads = [load(5, Some(5), 1, false), load(3, Some(3), 1, false)];
        assert_eq!(choose_host(&loads, Placement::Spread, false), None);
    }

    #[test]
    fn anti_affinity_prefers_hosts_without_the_spec() {
        // host0 (less loaded) already runs the spec; host1 doesn't →
        // anti-affinity picks host1 despite being more loaded.
        let loads = [load(0, None, 1, true), load(2, None, 1, false)];
        assert_eq!(choose_host(&loads, Placement::Spread, true), Some(1));
        // Soft fallback: every eligible host runs the spec ⇒ behave like
        // plain spread (least-loaded).
        let loads = [load(2, None, 1, true), load(1, None, 1, true)];
        assert_eq!(choose_host(&loads, Placement::Spread, true), Some(1));
    }

    #[test]
    fn empty_loads_is_none() {
        assert_eq!(choose_host(&[], Placement::Spread, false), None);
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
