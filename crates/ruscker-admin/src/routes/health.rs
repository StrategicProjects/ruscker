//! Liveness and readiness probes for orchestrators and load
//! balancers (`/healthz`, `/readyz`).
//!
//! These follow the de-facto Kubernetes convention:
//!
//! - **`/healthz` (liveness)** — "is the process up and serving?".
//!   Always returns `200`. It does *not* touch the DB or Docker:
//!   a liveness probe that fails on a dependency outage would make
//!   the orchestrator kill-and-restart Ruscker, which never fixes
//!   a down database and only sheds the landing page too. Liveness
//!   means "the event loop is responsive"; readiness means "my
//!   dependencies are reachable".
//!
//! - **`/readyz` (readiness)** — "should traffic be routed to me?".
//!   Probes the dependencies Ruscker was actually configured with
//!   (the SQLite pool when `--db` is set, the container backend
//!   when `--docker` is set) and returns `503` if any is
//!   unreachable. A dependency that wasn't configured is simply
//!   absent from the report — readiness reflects the running mode,
//!   not an idealized full deployment.
//!
//! Both are unauthenticated: probes run before any operator logs
//! in, and the responses carry no sensitive data. Dependency
//! errors are logged at `warn` but reported to the caller only as
//! a terse `"unreachable"` label, so internal details (socket
//! paths, SQL text) never leak through the probe.

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde_json::json;
use std::time::Duration;
use tracing::warn;

use crate::AppState;

/// Finite budget for each readiness dependency. Multi-host Docker has its own
/// longer reconcile budget, but the public probe is deliberately tighter so a
/// dependency outage cannot pin probe tasks indefinitely. Operators should set
/// their load-balancer probe timeout above this five-second application budget.
const READINESS_CHECK_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProbeStatus {
    Ok,
    Unreachable,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
}

/// Liveness: the process is up and the async runtime is servicing
/// requests. Intentionally dependency-free — see the module docs.
async fn healthz() -> Response {
    Json(json!({
        "status": "ok",
        "service": "ruscker",
        "version": env!("CARGO_PKG_VERSION"),
    }))
    .into_response()
}

async fn probe_database(db: &crate::db::ConfigDb, budget: Duration) -> ProbeStatus {
    use crate::db::ConfigDb;

    let probe = async {
        match db {
            ConfigDb::Sqlite(pool) => sqlx::query("SELECT 1").fetch_one(pool).await.map(|_| ()),
            ConfigDb::Postgres(pool) => sqlx::query("SELECT 1").fetch_one(pool).await.map(|_| ()),
        }
    };
    match tokio::time::timeout(budget, probe).await {
        Ok(Ok(())) => ProbeStatus::Ok,
        Ok(Err(error)) => {
            warn!(error = %error, "readyz: database check failed");
            ProbeStatus::Unreachable
        }
        Err(_) => {
            warn!(timeout_ms = budget.as_millis(), "readyz: database check timed out");
            ProbeStatus::Unreachable
        }
    }
}

async fn probe_backend(
    backend: &dyn ruscker_core::ContainerBackend,
    budget: Duration,
) -> ProbeStatus {
    match tokio::time::timeout(budget, backend.list()).await {
        Ok(Ok(_)) => ProbeStatus::Ok,
        Ok(Err(error)) => {
            warn!(error = %error, "readyz: container backend check failed");
            ProbeStatus::Unreachable
        }
        Err(_) => {
            warn!(timeout_ms = budget.as_millis(), "readyz: container backend check timed out");
            ProbeStatus::Unreachable
        }
    }
}

/// Probe independent dependencies concurrently. Returning `Option` keeps the
/// distinction between "not configured" and "configured but unreachable".
async fn probe_dependencies(
    db: Option<&crate::db::ConfigDb>,
    backend: Option<&dyn ruscker_core::ContainerBackend>,
    budget: Duration,
) -> (Option<ProbeStatus>, Option<ProbeStatus>) {
    let db_probe = async {
        match db {
            Some(db) => Some(probe_database(db, budget).await),
            None => None,
        }
    };
    let backend_probe = async {
        match backend {
            Some(backend) => Some(probe_backend(backend, budget).await),
            None => None,
        }
    };
    tokio::join!(db_probe, backend_probe)
}

/// Readiness: every configured dependency is reachable. Returns
/// `200` with `{"status":"ready", ...}` when all checks pass, or
/// `503` with `{"status":"not_ready", ...}` when any fails.
async fn readyz(State(state): State<AppState>) -> Response {
    // Shutting down: report `not ready` immediately so the load
    // balancer deregisters this instance before the listener
    // closes. Skip the dependency probes — we're going away
    // regardless of their state.
    if state.draining.load(std::sync::atomic::Ordering::SeqCst) {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "status": "draining" })),
        )
            .into_response();
    }

    // The DB and Docker checks are independent. Probe them concurrently so
    // readiness latency is bounded by the slowest dependency, not their sum.
    let (db_status, backend_status) = probe_dependencies(
        state.db.as_ref(),
        state.backend.as_deref(),
        READINESS_CHECK_TIMEOUT,
    )
    .await;
    let mut checks = serde_json::Map::new();
    let mut ready = true;
    if let Some(status) = db_status {
        let value = match status {
            ProbeStatus::Ok => "ok",
            ProbeStatus::Unreachable => {
                ready = false;
                "unreachable"
            }
        };
        checks.insert("db".into(), json!(value));
    }
    if let Some(status) = backend_status {
        let value = match status {
            ProbeStatus::Ok => "ok",
            ProbeStatus::Unreachable => {
                ready = false;
                "unreachable"
            }
        };
        checks.insert("docker".into(), json!(value));
    }

    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = json!({
        "status": if ready { "ready" } else { "not_ready" },
        "checks": checks,
    });
    (status, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use ruscker_core::{
        ContainerBackend, CoreError, CoreResult, Replica, ReplicaId, ReplicaMetrics,
    };

    struct PendingBackend;

    #[async_trait]
    impl ContainerBackend for PendingBackend {
        async fn spawn(&self, _spec_id: &str, _image: &str) -> CoreResult<Replica> {
            Err(CoreError::Backend("unused".into()))
        }

        async fn stop(&self, _replica_id: &ReplicaId) -> CoreResult<()> {
            Err(CoreError::Backend("unused".into()))
        }

        async fn list(&self) -> CoreResult<Vec<Replica>> {
            std::future::pending().await
        }

        async fn metrics(&self, _replica_id: &ReplicaId) -> CoreResult<ReplicaMetrics> {
            Err(CoreError::Backend("unused".into()))
        }
    }

    #[tokio::test]
    async fn dependency_timeouts_run_concurrently() {
        // Exhaust a one-connection SQLite pool so SELECT 1 waits for a slot,
        // while the backend also stays pending forever. Both probes must time
        // out in one shared wall-clock window rather than serially (#945).
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let _held_connection = pool.acquire().await.unwrap();
        let db = crate::db::ConfigDb::Sqlite(pool);
        let backend = PendingBackend;
        let budget = Duration::from_millis(100);
        let started = std::time::Instant::now();

        let (db_status, backend_status) =
            probe_dependencies(Some(&db), Some(&backend), budget).await;

        assert_eq!(db_status, Some(ProbeStatus::Unreachable));
        assert_eq!(backend_status, Some(ProbeStatus::Unreachable));
        assert!(
            started.elapsed() < Duration::from_millis(175),
            "dependency timeouts ran serially: {:?}",
            started.elapsed()
        );
    }
}
