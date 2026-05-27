//! Prometheus `/metrics` endpoint (#109).
//!
//! Opt-in via `proxy.metrics-enabled` — when off (the default) the
//! route 404s. When on it's served **unauthenticated**, like the
//! health probes, so it must only be reachable by your scraper
//! (private network / firewall); see `docs/SECURITY.md`.
//!
//! The exposition is built from the same live state the dashboard
//! reads — the [`ReplicaRegistry`] and the per-replica
//! [`MetricsCache`] — so there's no separate collection path.

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use ruscker_core::{Replica, ReplicaState};
use std::collections::BTreeMap;
use std::fmt::Write as _;

use crate::metrics_cache::MetricsCache;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/metrics", get(metrics))
}

/// Prometheus text exposition format content type (v0.0.4).
const PROM_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

async fn metrics(State(state): State<AppState>) -> Response {
    if !state.config.proxy.metrics_enabled {
        // Off by default — don't advertise that the route exists.
        return (StatusCode::NOT_FOUND, "metrics disabled").into_response();
    }

    // Snapshot the registry under the read lock, then render lock-free.
    let replicas: Vec<Replica> = {
        let reg = state.replicas.read().await;
        reg.all().cloned().collect()
    };
    let body = encode(
        &replicas,
        &state.metrics,
        state.sessions.len(),
        state.backend.is_some(),
    );

    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, PROM_CONTENT_TYPE)],
        body,
    )
        .into_response()
}

/// Lowercase label for a replica state (stable metric label values).
fn state_label(s: ReplicaState) -> &'static str {
    match s {
        ReplicaState::Starting => "starting",
        ReplicaState::Ready => "ready",
        ReplicaState::Draining => "draining",
        ReplicaState::Stopped => "stopped",
        ReplicaState::Failed => "failed",
    }
}

/// Escape a Prometheus label *value* per the exposition format:
/// backslash, double-quote and newline.
fn esc(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    for c in v.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            _ => out.push(c),
        }
    }
    out
}

/// Render the Prometheus exposition text from a registry snapshot +
/// the metrics cache. Pure (no I/O), so it's unit-testable.
fn encode(
    replicas: &[Replica],
    metrics: &MetricsCache,
    tracked_sessions: usize,
    backend_connected: bool,
) -> String {
    let mut out = String::new();

    // backend up/down
    let _ = writeln!(
        out,
        "# HELP ruscker_up Container backend connected (1) or absent (0)."
    );
    let _ = writeln!(out, "# TYPE ruscker_up gauge");
    let _ = writeln!(out, "ruscker_up {}", backend_connected as u8);

    // total replicas
    let _ = writeln!(
        out,
        "# HELP ruscker_replicas_total Running replicas across all specs."
    );
    let _ = writeln!(out, "# TYPE ruscker_replicas_total gauge");
    let _ = writeln!(out, "ruscker_replicas_total {}", replicas.len());

    // tracked sticky sessions
    let _ = writeln!(
        out,
        "# HELP ruscker_sessions_tracked Sticky sessions currently tracked."
    );
    let _ = writeln!(out, "# TYPE ruscker_sessions_tracked gauge");
    let _ = writeln!(out, "ruscker_sessions_tracked {tracked_sessions}");

    // replicas by (spec, state) — BTreeMap keeps the output stable.
    let mut by_spec_state: BTreeMap<(&str, &str), u32> = BTreeMap::new();
    let mut sessions_by_spec: BTreeMap<&str, u32> = BTreeMap::new();
    for r in replicas {
        *by_spec_state
            .entry((r.spec_id.as_str(), state_label(r.state)))
            .or_insert(0) += 1;
        *sessions_by_spec.entry(r.spec_id.as_str()).or_insert(0) += r.sessions_active;
    }

    let _ = writeln!(
        out,
        "# HELP ruscker_replicas Running replicas by spec and state."
    );
    let _ = writeln!(out, "# TYPE ruscker_replicas gauge");
    for ((spec, st), n) in &by_spec_state {
        let _ = writeln!(
            out,
            "ruscker_replicas{{spec=\"{}\",state=\"{}\"}} {}",
            esc(spec),
            st,
            n
        );
    }

    let _ = writeln!(
        out,
        "# HELP ruscker_sessions_active Active sessions by spec."
    );
    let _ = writeln!(out, "# TYPE ruscker_sessions_active gauge");
    for (spec, n) in &sessions_by_spec {
        let _ = writeln!(
            out,
            "ruscker_sessions_active{{spec=\"{}\"}} {}",
            esc(spec),
            n
        );
    }

    // per-replica CPU + memory from the cache (skip replicas with no
    // reading yet, e.g. just-spawned ones).
    let _ = writeln!(
        out,
        "# HELP ruscker_replica_cpu_percent Per-replica CPU usage (percent)."
    );
    let _ = writeln!(out, "# TYPE ruscker_replica_cpu_percent gauge");
    let mut mem_lines = String::new();
    for r in replicas {
        if let Some(c) = metrics.get(&r.id) {
            let _ = writeln!(
                out,
                "ruscker_replica_cpu_percent{{spec=\"{}\",replica=\"{}\"}} {}",
                esc(&r.spec_id),
                r.id,
                c.metrics.cpu_percent
            );
            let _ = writeln!(
                mem_lines,
                "ruscker_replica_memory_bytes{{spec=\"{}\",replica=\"{}\"}} {}",
                esc(&r.spec_id),
                r.id,
                c.metrics.memory_bytes
            );
        }
    }
    let _ = writeln!(
        out,
        "# HELP ruscker_replica_memory_bytes Per-replica memory usage (bytes)."
    );
    let _ = writeln!(out, "# TYPE ruscker_replica_memory_bytes gauge");
    out.push_str(&mem_lines);

    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruscker_core::{ReplicaId, ReplicaMetrics};
    use std::net::SocketAddr;

    fn replica(spec: &str, state: ReplicaState, active: u32) -> Replica {
        Replica {
            id: ReplicaId::new(),
            spec_id: spec.to_string(),
            container_id: "c".into(),
            upstream: "127.0.0.1:8000".parse::<SocketAddr>().unwrap(),
            state,
            started_at: chrono::Utc::now(),
            sessions_active: active,
            sessions_max: 5,
            host: None,
        }
    }

    #[test]
    fn encodes_gauges_and_labels() {
        let rs = vec![
            replica("shiny-a", ReplicaState::Ready, 2),
            replica("shiny-a", ReplicaState::Ready, 1),
            replica("api-b", ReplicaState::Starting, 0),
        ];
        let cache = MetricsCache::new();
        cache.replace(vec![(
            rs[0].id.clone(),
            ReplicaMetrics {
                cpu_percent: 12.5,
                memory_bytes: 1024,
                network_rx_bytes: 0,
                network_tx_bytes: 0,
            },
        )]);

        let out = encode(&rs, &cache, 3, true);

        assert!(out.contains("ruscker_up 1"));
        assert!(out.contains("ruscker_replicas_total 3"));
        assert!(out.contains("ruscker_sessions_tracked 3"));
        // Two ready replicas of shiny-a aggregate.
        assert!(out.contains("ruscker_replicas{spec=\"shiny-a\",state=\"ready\"} 2"));
        assert!(out.contains("ruscker_replicas{spec=\"api-b\",state=\"starting\"} 1"));
        // Sessions summed per spec.
        assert!(out.contains("ruscker_sessions_active{spec=\"shiny-a\"} 3"));
        // Per-replica metrics only for the replica with a cached reading.
        assert!(out.contains("ruscker_replica_cpu_percent{spec=\"shiny-a\""));
        assert!(out.contains(" 12.5"));
        assert!(out.contains("ruscker_replica_memory_bytes{spec=\"shiny-a\""));
        assert!(out.contains(" 1024"));
        // Each TYPE line appears once.
        assert_eq!(out.matches("# TYPE ruscker_up gauge").count(), 1);
    }

    #[test]
    fn backend_absent_is_zero() {
        let out = encode(&[], &MetricsCache::new(), 0, false);
        assert!(out.contains("ruscker_up 0"));
        assert!(out.contains("ruscker_replicas_total 0"));
    }
}
