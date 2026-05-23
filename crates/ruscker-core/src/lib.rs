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
    /// Start a new container instance for the given spec. Returns a
    /// [`Replica`] handle once the container is ready (healthy + bound
    /// to a port).
    async fn spawn(&self, spec_id: &str, image: &str) -> CoreResult<Replica>;

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
}
