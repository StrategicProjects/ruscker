//! Sessions and their persistence.
//!
//! A [`Session`] is the binding between a user (identified by signed
//! cookie) and a specific [`Replica`]. For Shiny apps and other
//! stateful interactive containers, sessions MUST be sticky — once
//! created, every subsequent request from the same client goes to the
//! same replica.
//!
//! The [`SessionStore`] trait abstracts over storage. The MVP uses an
//! in-memory `DashMap`. Multi-node HA setups will plug in a Postgres
//! or Redis implementation.

use crate::replica::ReplicaId;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Hash, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionId(pub Uuid);

impl SessionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for SessionId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for SessionId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub spec_id: String,
    pub replica_id: ReplicaId,
    pub created_at: DateTime<Utc>,
    pub last_heartbeat: DateTime<Utc>,
}

/// Public-facing "sticky session" snapshot — what gets encoded into the
/// cookie. Compact and serde-stable across deploys.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StickySession {
    pub session_id: SessionId,
    pub spec_id: String,
    pub replica_id: ReplicaId,
}

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn create(&self, session: Session) -> crate::CoreResult<()>;
    async fn get(&self, id: &SessionId) -> crate::CoreResult<Option<Session>>;
    async fn touch(&self, id: &SessionId, now: DateTime<Utc>) -> crate::CoreResult<()>;
    async fn remove(&self, id: &SessionId) -> crate::CoreResult<()>;
    async fn purge_expired(&self, threshold: DateTime<Utc>) -> crate::CoreResult<usize>;
}
