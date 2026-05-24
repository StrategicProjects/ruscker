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
pub mod session;

pub use replica::{Replica, ReplicaId, ReplicaState};
pub use routing::{RoutingDecision, Router};
pub use session::{Session, SessionId, SessionStore, StickySession};

use async_trait::async_trait;
use std::collections::HashMap;
use thiserror::Error;

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
        }
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
