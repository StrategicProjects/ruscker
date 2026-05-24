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
use tracing::warn;

use crate::AppState;

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

    let mut checks = serde_json::Map::new();
    let mut ready = true;

    // SQLite: a trivial round-trip confirms the pool can hand out a
    // live connection (catches a deleted/locked DB file or an
    // exhausted pool).
    if let Some(pool) = &state.db {
        match sqlx::query("SELECT 1").fetch_one(pool).await {
            Ok(_) => {
                checks.insert("db".into(), json!("ok"));
            }
            Err(err) => {
                ready = false;
                warn!(error = %err, "readyz: database check failed");
                checks.insert("db".into(), json!("unreachable"));
            }
        }
    }

    // Container backend: `list()` is the cheapest "can I reach the
    // Docker daemon?" call on the trait — it's the same one the
    // startup reconcile uses.
    if let Some(backend) = &state.backend {
        match backend.list().await {
            Ok(_) => {
                checks.insert("docker".into(), json!("ok"));
            }
            Err(err) => {
                ready = false;
                warn!(error = %err, "readyz: container backend check failed");
                checks.insert("docker".into(), json!("unreachable"));
            }
        }
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
