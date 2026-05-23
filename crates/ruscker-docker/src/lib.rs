//! # ruscker-docker
//!
//! Docker backend implementation of [`ruscker_core::ContainerBackend`].
//!
//! ## Status: stub
//!
//! The structure is in place; the implementation will land in Phase 3.
//! See `CLAUDE.md` in this crate for the exact sequence of work to do.

#![allow(dead_code)]

use async_trait::async_trait;
use ruscker_core::{
    ContainerBackend, CoreError, CoreResult, Replica, ReplicaId, ReplicaMetrics,
};

/// Docker backend talking to a local or remote daemon over HTTP/socket.
pub struct DockerBackend {
    // TODO(phase-3): replace with bollard::Docker
    _placeholder: (),
}

impl DockerBackend {
    pub fn local() -> CoreResult<Self> {
        // TODO(phase-3): bollard::Docker::connect_with_local_defaults()
        Ok(Self { _placeholder: () })
    }
}

#[async_trait]
impl ContainerBackend for DockerBackend {
    async fn spawn(&self, _spec_id: &str, _image: &str) -> CoreResult<Replica> {
        // TODO(phase-3):
        // 1. Pull image if not local (with registry credentials)
        // 2. Create container with resource limits + env + volumes
        // 3. Start container
        // 4. Poll healthcheck until healthy or container-wait-time elapses
        // 5. Return Replica with bound port and Ready state
        Err(CoreError::Backend(
            "DockerBackend::spawn not yet implemented (phase 3)".into(),
        ))
    }

    async fn stop(&self, _replica_id: &ReplicaId) -> CoreResult<()> {
        // TODO(phase-3): drain → stop → kill
        Err(CoreError::Backend(
            "DockerBackend::stop not yet implemented (phase 3)".into(),
        ))
    }

    async fn list(&self) -> CoreResult<Vec<Replica>> {
        Ok(Vec::new())
    }

    async fn metrics(&self, _replica_id: &ReplicaId) -> CoreResult<ReplicaMetrics> {
        Err(CoreError::Backend(
            "metrics not yet implemented (phase 4)".into(),
        ))
    }
}
