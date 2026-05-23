//! Replica: a running container instance for a spec.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use uuid::Uuid;

/// Unique identifier for a replica. Generated when the replica starts.
#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReplicaId(pub Uuid);

impl ReplicaId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ReplicaId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for ReplicaId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplicaState {
    /// Container is being pulled / created
    Starting,
    /// Healthy and accepting sessions
    Ready,
    /// Stopped accepting new sessions, waiting for existing to drain
    Draining,
    /// Stopped or crashed
    Stopped,
    /// Failed to start
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Replica {
    pub id: ReplicaId,
    pub spec_id: String,
    pub container_id: String,
    pub upstream: SocketAddr,
    pub state: ReplicaState,
    pub started_at: DateTime<Utc>,
    pub sessions_active: u32,
    pub sessions_max: u32,
}

impl Replica {
    /// Fraction of seats currently in use (0.0 .. 1.0).
    pub fn utilization(&self) -> f64 {
        if self.sessions_max == 0 {
            return 0.0;
        }
        self.sessions_active as f64 / self.sessions_max as f64
    }

    /// Number of free seats.
    pub fn available_seats(&self) -> u32 {
        self.sessions_max.saturating_sub(self.sessions_active)
    }

    pub fn is_accepting(&self) -> bool {
        matches!(self.state, ReplicaState::Ready) && self.available_seats() > 0
    }
}
