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

/// Classify a host's **config** (not connectivity): `Some(msg)` for a
/// fatal operator error — an unknown address scheme, or `tcp://` with no
/// TLS — that [`MultiHostDockerBackend::connect`] must reject up front;
/// `None` otherwise. A reachable-but-down host, or a bad TLS file path,
/// is *not* a config error here — those surface from `connect_host` and
/// are skipped for degraded start (#160 D4).
fn host_config_error(host: &Host) -> Option<String> {
    let addr = host.address.trim();
    if addr.starts_with("unix://") || addr.starts_with("ssh://") || addr.starts_with("http://") {
        None
    } else if addr.starts_with("tcp://") {
        host.tls
            .is_none()
            .then(|| format!("host `{}`: tcp:// needs `tls` (ca/cert/key)", host.id))
    } else {
        Some(format!(
            "host `{}`: address `{addr}` must start with ssh:// , tcp:// , http:// or unix://",
            host.id
        ))
    }
}

/// Reachable host from an `[user@]host[:port]` authority — drops the
/// `user@` and a trailing `:port`. Handles bracketed IPv6 literals
/// (`[2001:db8::1]:2376` → `2001:db8::1`); a bare unbracketed IPv6 is
/// returned as-is rather than truncated at the first colon (#160 D3).
fn host_part(authority: &str) -> String {
    let no_user = authority.rsplit('@').next().unwrap_or(authority);
    // Bracketed IPv6: take what's inside the brackets, ignore any port.
    if let Some(rest) = no_user.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return rest[..end].to_string();
        }
    }
    // `host:port` (IPv4 / hostname): strip the port only when there's a
    // single colon and the suffix is numeric. Multiple colons with no
    // brackets ⇒ a bare IPv6 literal — return it whole, don't truncate.
    match no_user.rsplit_once(':') {
        Some((h, port))
            if !h.contains(':') && !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) =>
        {
            h.to_string()
        }
        _ => no_user.to_string(),
    }
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
    /// Connect to every configured host. A **config** error (empty
    /// list, unknown scheme, `tcp://` without TLS) is always fatal —
    /// it's operator error. A **connectivity** failure (down daemon,
    /// bad TLS path, unreachable ssh) only logs+skips that host, so the
    /// cluster can start degraded from whoever is reachable; boot fails
    /// only if *no* host connects (#160 D4). This mirrors `list`'s
    /// degraded philosophy.
    pub fn connect(hosts: &[Host]) -> CoreResult<Self> {
        if hosts.is_empty() {
            return Err(CoreError::Backend("no docker hosts configured".into()));
        }
        // Phase 1 — config validation: fatal, before touching the
        // network, so a typo'd scheme can't be masked by degraded-start.
        for h in hosts {
            if let Some(msg) = host_config_error(h) {
                return Err(CoreError::Backend(msg));
            }
        }
        // Phase 2 — connect; skip the unreachable.
        let mut built = Vec::with_capacity(hosts.len());
        for h in hosts {
            match connect_host(h) {
                Ok(backend) => built.push(HostEntry {
                    id: h.id.clone(),
                    backend,
                    max_containers: h.max_containers,
                    weight: h.weight.unwrap_or(1).max(1),
                }),
                Err(e) => {
                    tracing::warn!(host = %h.id, error = %e, "connect to host failed; starting without it");
                }
            }
        }
        if built.is_empty() {
            return Err(CoreError::Backend(
                "no docker hosts reachable (all configured hosts failed to connect)".into(),
            ));
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
        // Placement miss (never listed, or its host left the config):
        // try every host.
        for h in &self.hosts {
            if h.backend.stop(replica_id).await.is_ok() {
                self.placement.remove(replica_id);
                return Ok(());
            }
        }
        // No host owned it. `stop` is idempotent — an absent container
        // satisfies the request — so drop any stale (unroutable)
        // placement entry instead of leaving a ghost that inflates
        // `pick_host` forever (#160 D1/D2), and report success. Warned
        // because it could also mean every host is unreachable.
        self.placement.remove(replica_id);
        tracing::warn!(
            replica = %replica_id,
            "stop: no host confirmed the container; treating as already gone"
        );
        Ok(())
    }

    async fn list(&self) -> CoreResult<Vec<Replica>> {
        // Fan out over every host; tag each replica's placement so
        // later stop/metrics/logs route correctly. A host that fails to
        // answer is logged and skipped (degraded, not fatal).
        let mut all = Vec::new();
        let mut live: std::collections::HashSet<ReplicaId> = std::collections::HashSet::new();
        let mut failed_hosts: std::collections::HashSet<String> = std::collections::HashSet::new();
        for h in &self.hosts {
            match h.backend.list().await {
                Ok(mut replicas) => {
                    for r in &mut replicas {
                        self.placement
                            .insert(r.id.clone(), (h.id.clone(), r.spec_id.clone()));
                        r.host = Some(h.id.clone());
                        live.insert(r.id.clone());
                    }
                    all.extend(replicas);
                }
                Err(e) => {
                    tracing::warn!(host = %h.id, error = %e, "list on host failed; skipping");
                    failed_hosts.insert(h.id.clone());
                }
            }
        }
        // Authoritative prune (#160 D1): drop placement for replicas no
        // host reported, so dead/crashed containers stop inflating
        // `pick_host`'s load counts. Keep entries whose owning host
        // *didn't answer* — we can't tell if those are gone.
        self.placement
            .retain(|id, (host, _)| live.contains(id) || failed_hosts.contains(host));
        Ok(all)
    }

    async fn metrics(&self, replica_id: &ReplicaId) -> CoreResult<ReplicaMetrics> {
        // Placement-only, like `logs` (#160 D2): a metrics fan-out would
        // fire a CPU-delta stats round-trip at every daemon that doesn't
        // own the replica, every cache refresh. `list` keeps placement
        // fresh, so a miss here means we genuinely don't know the host.
        match self.placed(replica_id) {
            Some(backend) => backend.metrics(replica_id).await,
            None => Err(CoreError::Backend(format!(
                "replica {replica_id} not placed on any known host"
            ))),
        }
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

    #[test]
    fn host_part_handles_ipv6_literals() {
        // #160 D3: bracketed IPv6, with and without a port / user.
        assert_eq!(host_part("[2001:db8::1]:2376"), "2001:db8::1");
        assert_eq!(host_part("[2001:db8::1]"), "2001:db8::1");
        assert_eq!(host_part("ops@[fe80::1]:2376"), "fe80::1");
        assert_eq!(host_part("[::1]:2376"), "::1");
        // A bare unbracketed IPv6 is returned whole, not truncated at
        // the first colon (the old `split(':').next()` bug).
        assert_eq!(host_part("2001:db8::1"), "2001:db8::1");
    }

    #[test]
    fn host_config_error_flags_only_config_mistakes() {
        // Valid schemes ⇒ no config error (connectivity is checked later).
        assert!(host_config_error(&host("a", "ssh://ops@h")).is_none());
        assert!(host_config_error(&host("b", "http://h:2375")).is_none());
        assert!(host_config_error(&host("c", "unix:///var/run/docker.sock")).is_none());
        // tcp:// without TLS ⇒ fatal config error.
        assert!(host_config_error(&host("d", "tcp://h:2376")).is_some());
        // tcp:// with TLS ⇒ ok.
        let mut tls_host = host("e", "tcp://h:2376");
        tls_host.tls = Some(HostTls {
            ca: PathBuf::from("ca.pem"),
            cert: PathBuf::from("cert.pem"),
            key: PathBuf::from("key.pem"),
        });
        assert!(host_config_error(&tls_host).is_none());
        // Unknown scheme ⇒ fatal config error.
        assert!(host_config_error(&host("f", "ftp://h")).is_some());
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

    /// Live test against TWO real Docker endpoints. Skipped unless built
    /// with `--features multihost-it` and `RUSCKER_IT_HOST1` /
    /// `RUSCKER_IT_HOST2` point at reachable daemons (`ssh://` /
    /// `http://` / `unix://`) whose published ports this host can reach.
    /// See `book/src/deploying.md` § Multi-host scheduling.
    ///
    ///   RUSCKER_IT_HOST1=ssh://ops@10.0.0.11 \
    ///   RUSCKER_IT_HOST2=ssh://ops@10.0.0.12 \
    ///   cargo test -p ruscker-docker --features multihost-it -- --nocapture
    #[cfg(feature = "multihost-it")]
    #[tokio::test]
    async fn spreads_two_replicas_across_real_hosts() {
        use ruscker_core::ContainerBackend;

        let addr1 = std::env::var("RUSCKER_IT_HOST1").expect("set RUSCKER_IT_HOST1");
        let addr2 = std::env::var("RUSCKER_IT_HOST2").expect("set RUSCKER_IT_HOST2");
        let image =
            std::env::var("RUSCKER_IT_IMAGE").unwrap_or_else(|_| "nginx:1.29-alpine".into());

        let backend = MultiHostDockerBackend::connect(&[host("h1", &addr1), host("h2", &addr2)])
            .expect("connect both hosts");

        // Two spread replicas of the same spec should land on different
        // hosts (anti-affinity off; spread = least-loaded).
        let req = SpawnRequest::new("it-spec", &image).with_port(80);
        let r1 = backend.spawn_request(&req).await.expect("spawn 1");
        let r2 = backend.spawn_request(&req).await.expect("spawn 2");
        assert!(
            r1.host.is_some() && r2.host.is_some(),
            "replicas carry a host"
        );
        assert_ne!(r1.host, r2.host, "spread placed both on the same host");

        // list() fans out over both hosts and tags each replica.
        let listed = backend.list().await.expect("list");
        assert!(listed.iter().any(|r| r.id == r1.id && r.host == r1.host));
        assert!(listed.iter().any(|r| r.id == r2.id && r.host == r2.host));

        // Routed stop reaches the owning host.
        backend.stop(&r1.id).await.expect("stop 1");
        backend.stop(&r2.id).await.expect("stop 2");
    }
}
