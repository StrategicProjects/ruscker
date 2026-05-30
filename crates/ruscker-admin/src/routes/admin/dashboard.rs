//! Admin > Dashboard.
//!
//! Read-only monitoring surface. Renders a snapshot of:
//!
//! - Aggregate metric cards (total containers, total active
//!   sessions across the registry, count of specs that have at
//!   least one replica, total tracked sessions in the heartbeat
//!   store).
//! - Per-replica table: spec, state, uptime, sessions, short
//!   container id.
//!
//! ## What's NOT here (yet)
//!
//! - **Per-replica CPU / memory.** Needs a fan-out of
//!   `backend.metrics()` calls; landed in slice 2 with a small
//!   in-memory cache so each request doesn't re-poll Docker.
//! - **Live updates.** The page is a static render; an operator
//!   refreshes manually. Slice 3 adds an SSE endpoint and a
//!   tiny JS subscriber.

use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Redirect, Response,
    },
    routing::{get, post},
    Router,
};
use chrono::Utc;
use futures_util::stream::Stream;
use ruscker_core::{Replica, ReplicaId, ReplicaState};
use std::convert::Infallible;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use dashmap::DashMap;

use crate::auth::{AdminSession, RequireEditor, Role};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/dashboard", get(index))
        .route("/admin/dashboard/events", get(events))
        .route("/admin/dashboard/logs/{replica_id}", get(logs))
        .route("/admin/dashboard/logs/{replica_id}/stream", get(logs_stream))
        .route("/admin/dashboard/replicas/{replica_id}/stop", post(stop_replica))
        .route(
            "/admin/dashboard/replicas/{replica_id}/restart",
            post(restart_replica),
        )
}

/// How many trailing log lines the logs page requests. Enough
/// to see a crash traceback without flooding the page; the
/// backend caps harder at its own `MAX_TAIL`.
const LOGS_TAIL: usize = 500;

/// How often the SSE stream emits a snapshot. Same cadence as
/// the metrics cache refresh — emitting more often would show
/// stale CPU/memory values, less often would feel laggy on
/// container state changes.
const SSE_INTERVAL: Duration = Duration::from_secs(5);

/// How often to send a comment-only keep-alive when no real
/// event has fired. Stays under the typical 30-60s reverse-
/// proxy idle timeout. Doesn't trigger the JS `onmessage`
/// handler.
const SSE_KEEPALIVE_INTERVAL: Duration = Duration::from_secs(15);

/// Per-locale memoized dashboard snapshot (#291). Building a snapshot
/// reads the registry, fetches the effective spec catalog (a DB query),
/// and assembles the rows; with N open dashboard tabs each running its
/// own [`SSE_INTERVAL`] loop that was N× the work every tick. The whole
/// snapshot is the same for everyone in a given locale (only the state
/// labels are localized), so cache it for one interval and let every
/// tab/connection in that locale reuse it. Keyed by locale (≤4 entries);
/// a clone of the cached snapshot is cheap next to a fresh DB build.
static SNAPSHOT_CACHE: LazyLock<DashMap<Locale, (Instant, Arc<DashboardSnapshot>)>> =
    LazyLock::new(DashMap::new);

/// One row of the replicas table — flattened for the template
/// and also serialized as JSON over the SSE stream so the
/// client-side patcher can update in place.
///
/// We project `Replica` plus the operator-facing `display_name`
/// resolved from the spec config (replicas only carry `spec_id`).
/// The `replica_id` round-trips as a stringified UUID so the JS
/// can target the right `<tr data-replica-id="…">`.
#[derive(serde::Serialize, Clone)]
struct ReplicaRow {
    replica_id: String,
    spec_id: String,
    /// `display-name` from the spec config, or the spec_id if
    /// the spec was renamed/deleted out from under the registry.
    display_name: String,
    /// Lowercase state string ("ready", "starting", …).
    /// Kept stable across versions — the JS uses it to pick
    /// the dot CSS class and (via the i18n keys baked into the
    /// initial render) the state label.
    state: &'static str,
    /// CSS class for the dot — pre-resolved so the JS doesn't
    /// have to mirror our match.
    state_dot: &'static str,
    /// i18n-translated label — server-translated so the JS
    /// doesn't need its own locale bundle.
    state_label: String,
    /// Pre-formatted "2h 14m" string. Built once at render time
    /// so the template (and the JS) stays declarative.
    uptime: String,
    sessions_active: u32,
    sessions_max: u32,
    /// First 12 chars of the container ID, enough to disambiguate
    /// while staying readable.
    container_short: String,
    /// Pre-formatted "23%" / "n/a" for the CPU column. `None`
    /// when the metrics cache hasn't observed this replica yet
    /// (first 5 s after spawn, before the refresher's first
    /// tick).
    cpu_display: Option<String>,
    /// Pre-formatted "412 MB" / "n/a" for the memory column.
    memory_display: Option<String>,
    /// Recent CPU% / memory samples (oldest first) for the inline
    /// sparklines. Empty until the metrics cache has ≥1 reading.
    cpu_history: Vec<f64>,
    mem_history: Vec<u64>,
    /// Docker host the replica runs on (multi-host, Phase 6). Empty for
    /// the single local daemon — rendered as "local".
    host: String,
}

/// What both the HTML render and the SSE stream consume.
/// Building this once and serializing twice keeps the two
/// surfaces in lockstep — fix a counting bug here, both
/// endpoints get the fix.
#[derive(serde::Serialize, Clone)]
struct DashboardSnapshot {
    backend_connected: bool,
    total_containers: usize,
    total_sessions: u32,
    spec_count: usize,
    tracker_sessions: usize,
    total_memory_bytes: u64,
    total_memory_display: String,
    rows: Vec<ReplicaRow>,
}

#[derive(Template)]
#[template(path = "admin/dashboard.html")]
struct DashboardPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// Mount prefix for base-path-correct URLs (#294).
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    /// Current session role — drives nav gating and whether the
    /// per-replica stop/restart action buttons render.
    role: Role,
    /// Inline copy of `DashboardSnapshot` fields. Could be a
    /// nested `snapshot:` field but askama doesn't auto-flatten
    /// and rewriting all `{{ foo }}` to `{{ snapshot.foo }}`
    /// buys no clarity.
    backend_connected: bool,
    total_containers: usize,
    total_sessions: u32,
    spec_count: usize,
    tracker_sessions: usize,
    total_memory_bytes: u64,
    total_memory_display: String,
    rows: Vec<ReplicaRow>,
    /// JSON of the same snapshot, embedded in a `<script
    /// type="application/json">` tag so the SSE-driven JS
    /// patcher has a starting state without an immediate
    /// extra HTTP round-trip.
    snapshot_json: String,
}

impl<'a> DashboardPage<'a> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

#[derive(Template)]
#[template(path = "admin/logs.html")]
struct LogsPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// Mount prefix for base-path-correct URLs (#294).
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    /// Current session role — drives nav gating.
    role: Role,
    /// Display name resolved from the spec config (or spec_id
    /// fallback) for the page heading.
    display_name: String,
    spec_id: String,
    replica_id: String,
    /// The log lines, oldest-first. Empty vec renders an
    /// "no output" hint.
    lines: Vec<String>,
}

impl<'a> LogsPage<'a> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

/// Map a `ReplicaState` enum to the (lowercase id, CSS class)
/// pair used throughout the dashboard. Centralized so the SSE
/// JSON and the server-side HTML render stay consistent.
fn state_codes(s: ReplicaState) -> (&'static str, &'static str) {
    match s {
        ReplicaState::Ready => ("ready", "dot-on"),
        ReplicaState::Starting => ("starting", "dot-pulse"),
        ReplicaState::Draining => ("draining", "dot-warm"),
        ReplicaState::Stopped => ("stopped", "dot-off"),
        ReplicaState::Failed => ("failed", "dot-off"),
    }
}

/// i18n key for each replica state. Kept in Rust (not the
/// template) so a new `ReplicaState` variant produces a
/// compiler error instead of a missing translation at render
/// time.
fn state_label_key(s: ReplicaState) -> &'static str {
    match s {
        ReplicaState::Ready => "admin-dashboard-state-ready",
        ReplicaState::Starting => "admin-dashboard-state-starting",
        ReplicaState::Draining => "admin-dashboard-state-draining",
        ReplicaState::Stopped => "admin-dashboard-state-stopped",
        ReplicaState::Failed => "admin-dashboard-state-failed",
    }
}

/// Serialize `value` to JSON safe to embed in an HTML `<script>` element.
///
/// `serde_json` does not escape `<`, `>`, or `&`, so a string field
/// containing `</script>` would close the element early and the rest
/// would be parsed as live HTML — a stored-XSS hole when fields like a
/// spec's `display_name` or a container `host` label reach the embedded
/// `<script type="application/json">` snapshot (and the SSE frames).
/// Escaping those three to `\uXXXX` keeps the JSON valid (they only ever
/// occur inside string literals) while making `</script>`, `<!--`, etc.
/// inert.
fn json_for_html_script<T: serde::Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "{}".to_string())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
}

async fn index(
    session: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    let snap = build_snapshot(&state, loc).await;
    let snapshot_json = json_for_html_script(&snap);
    let page = DashboardPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "dashboard",
        role: session.role,
        backend_connected: snap.backend_connected,
        total_containers: snap.total_containers,
        total_sessions: snap.total_sessions,
        spec_count: snap.spec_count,
        tracker_sessions: snap.tracker_sessions,
        total_memory_bytes: snap.total_memory_bytes,
        total_memory_display: snap.total_memory_display.clone(),
        rows: snap.rows.clone(),
        snapshot_json,
    };
    super::render(&page)
}

/// Build the dashboard data snapshot. Pure(ish) function used
/// by both the initial HTML render and the SSE event stream
/// so they can't drift.
///
/// `locale` is needed to translate the per-state labels server-
/// side; the JS subscriber then just stamps the strings into
/// the DOM without owning a locale bundle of its own.
/// Snapshot for the dashboard + SSE, memoized per locale for one
/// [`SSE_INTERVAL`] (#291) so N concurrent tabs share one build instead
/// of each re-querying the DB every tick.
async fn build_snapshot(state: &AppState, locale: Locale) -> DashboardSnapshot {
    if let Some(entry) = SNAPSHOT_CACHE.get(&locale) {
        if entry.0.elapsed() < SSE_INTERVAL {
            return (*entry.1).clone();
        }
    }
    let snap = build_snapshot_uncached(state, locale).await;
    SNAPSHOT_CACHE.insert(locale, (Instant::now(), Arc::new(snap.clone())));
    snap
}

async fn build_snapshot_uncached(state: &AppState, locale: Locale) -> DashboardSnapshot {
    let backend_connected = state.backend.is_some();

    // Snapshot the registry once. Cloning `Replica` is cheap
    // (a few strings + a SocketAddr + small ints) and keeps the
    // read-lock window tight.
    //
    // Iterate the registry itself, not `config.proxy.specs` — replicas
    // can belong to specs that live only in the DB (the showcase seed
    // #202 / admin-added specs), which aren't in the YAML config. The
    // old config-keyed walk silently dropped every such replica from
    // the dashboard even though it was running.
    let snap: Vec<Replica> = {
        let reg = state.replicas.read().await;
        reg.all().cloned().collect()
    };

    let total_containers = snap.len();
    let total_sessions: u32 = snap.iter().map(|r| r.sessions_active).sum();
    let spec_count: usize = snap
        .iter()
        .map(|r| r.spec_id.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let tracker_sessions = state.sessions.len();

    // Resolve display names from the effective catalog (DB ∪ YAML,
    // DB-first) fetched **once** per snapshot — not a `find_spec` DB
    // round-trip per spec, which on the 5s SSE tick (× every open tab)
    // hammered the DB (#281). Still DB-first, so a renamed-in-admin spec
    // shows its current name (#275).
    let catalog = crate::catalog::effective_specs(state.db.as_ref(), &state.config).await;
    let name_of: std::collections::HashMap<String, String> = snap
        .iter()
        .map(|r| r.spec_id.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .map(|sid| {
            let name = catalog
                .iter()
                .find(|s| s.id == sid)
                .and_then(|s| s.display_name.clone())
                .unwrap_or_else(|| sid.clone());
            (sid, name)
        })
        .collect();

    let mut total_memory_bytes: u64 = 0;
    let rows: Vec<ReplicaRow> = snap
        .into_iter()
        .map(|r| {
            let display_name = name_of
                .get(&r.spec_id)
                .cloned()
                .unwrap_or_else(|| r.spec_id.clone());
            let container_short = r.container_id.chars().take(12).collect();
            let uptime = format_uptime(Utc::now() - r.started_at);
            let cached = state.metrics.get(&r.id);
            let cpu_display = cached.as_ref().map(|c| format!("{:.0}%", c.metrics.cpu_percent));
            let memory_display = cached.as_ref().map(|c| format_bytes(c.metrics.memory_bytes));
            let cpu_history = cached.as_ref().map(|c| c.cpu_history.clone()).unwrap_or_default();
            let mem_history = cached.as_ref().map(|c| c.mem_history.clone()).unwrap_or_default();
            if let Some(c) = cached.as_ref() {
                total_memory_bytes = total_memory_bytes.saturating_add(c.metrics.memory_bytes);
            }
            let (state_code, state_dot) = state_codes(r.state);
            let state_label = state.locales.t(locale, state_label_key(r.state), None);
            ReplicaRow {
                replica_id: r.id.0.to_string(),
                spec_id: r.spec_id,
                display_name,
                state: state_code,
                state_dot,
                state_label,
                uptime,
                sessions_active: r.sessions_active,
                sessions_max: r.sessions_max,
                container_short,
                cpu_display,
                memory_display,
                cpu_history,
                mem_history,
                host: r.host.unwrap_or_default(),
            }
        })
        .collect();
    let total_memory_display = if total_memory_bytes > 0 {
        format_bytes(total_memory_bytes)
    } else {
        "—".to_string()
    };

    DashboardSnapshot {
        backend_connected,
        total_containers,
        total_sessions,
        spec_count,
        tracker_sessions,
        total_memory_bytes,
        total_memory_display,
        rows,
    }
}

/// Render a byte count as a short human string with binary
/// units (KiB / MiB / GiB) and one decimal place at GB. Used
/// for the dashboard memory column and the aggregate memory
/// metric card.
///
/// "412 MB" reads better than "432211456 bytes". One-decimal
/// at GB keeps small fluctuations legible without going to
/// "412.345678 MB".
fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    if bytes < KIB {
        format!("{bytes} B")
    } else if bytes < MIB {
        format!("{} KB", bytes / KIB)
    } else if bytes < GIB {
        format!("{} MB", bytes / MIB)
    } else {
        format!("{:.1} GB", bytes as f64 / GIB as f64)
    }
}

/// Server-Sent Events stream of dashboard snapshots. The
/// client connects with `EventSource('/admin/dashboard/events')`
/// and receives a JSON-encoded [`DashboardSnapshot`] every
/// [`SSE_INTERVAL`], plus a comment keep-alive every
/// [`SSE_KEEPALIVE_INTERVAL`] to defeat proxy idle timeouts.
///
/// Each emission is independent; reconnecting after a network
/// blip just resumes the cadence — there's no event-id /
/// last-event-id handshake to manage, and replaying a snapshot
/// is idempotent on the client side anyway.
async fn events(
    _: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    use async_stream::stream;
    let stream = stream! {
        // Emit the first snapshot immediately so the client
        // patches on connect instead of waiting one full
        // interval. Avoids a visible "stale" gap right after
        // page load.
        let first = build_snapshot(&state, loc).await;
        let body = json_for_html_script(&first);
        yield Ok::<_, Infallible>(Event::default().data(body));

        let mut ticker = tokio::time::interval(SSE_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Burn the immediate first tick — we already emitted.
        ticker.tick().await;
        loop {
            ticker.tick().await;
            let snap = build_snapshot(&state, loc).await;
            let body = json_for_html_script(&snap);
            yield Ok::<_, Infallible>(Event::default().data(body));
        }
    };
    Sse::new(stream).keep_alive(KeepAlive::new().interval(SSE_KEEPALIVE_INTERVAL))
}

/// Per-replica logs page. Fetches the last [`LOGS_TAIL`] lines
/// of combined stdout+stderr via the backend and renders them
/// in a `<pre>`. One-shot — no live follow yet (that's a
/// future slice that would reuse the SSE machinery).
///
/// `replica_id` is the stringified UUID from the dashboard
/// row's `data-replica-id`. A malformed id → 400; a valid id
/// that no longer maps to a container → the backend returns an
/// error which we surface as 404.
async fn logs(
    // Container logs can carry sensitive data — gate behind Editor+
    // (Viewers can see the dashboard, not the logs). #261
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Path(replica_id): Path<String>,
) -> Response {
    let Ok(uuid) = uuid::Uuid::parse_str(&replica_id) else {
        return (StatusCode::BAD_REQUEST, "invalid replica id").into_response();
    };
    let rid = ReplicaId(uuid);

    let Some(backend) = state.backend.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no container backend — start with --docker",
        )
            .into_response();
    };

    // Resolve display_name + spec_id for the heading. Read the spec id
    // from the registry under the lock, then drop it before the DB-first
    // name lookup (#275) so we never hold the registry lock across I/O.
    let spec_id_of_replica = {
        let reg = state.replicas.read().await;
        let found = reg.all().find(|r| r.id == rid).map(|r| r.spec_id.clone());
        found
    };
    let (display_name, spec_id) = match spec_id_of_replica {
        Some(sid) => {
            let dn = crate::routes::proxy::find_spec(&state, &sid)
                .await
                .and_then(|s| s.display_name.clone())
                .unwrap_or_else(|| sid.clone());
            (dn, sid)
        }
        // Replica not in registry — still try to fetch logs (it may have
        // just been dropped from the registry but the container lingers).
        // Heading falls back to the raw id.
        None => (replica_id.clone(), String::new()),
    };

    let lines = match backend.logs(&rid, LOGS_TAIL).await {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!(replica = %replica_id, error = ?e, "fetch logs failed");
            return (
                StatusCode::NOT_FOUND,
                format!("could not fetch logs for replica {replica_id}: {e}"),
            )
                .into_response();
        }
    };

    let page = LogsPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "dashboard",
        role: editor.role,
        display_name,
        spec_id,
        replica_id,
        lines,
    };
    super::render(&page)
}

/// SSE stream of live log lines for a replica. The logs page's
/// "Live" toggle opens an `EventSource` against this; each new
/// line from the container becomes one SSE `data:` event. The
/// stream ends when the container stops (bollard's follow
/// stream completes), which the browser sees as the source
/// closing.
///
/// Seeded with the last 100 lines so turning Live on shows
/// recent context, not just lines that arrive after connect.
async fn logs_stream(
    _: RequireEditor,
    State(state): State<AppState>,
    Path(replica_id): Path<String>,
) -> Response {
    let rid = match parse_replica_id(&replica_id) {
        Ok(r) => r,
        Err(resp) => return *resp,
    };
    let Some(backend) = state.backend.clone() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no backend").into_response();
    };

    let line_stream = match backend.logs_follow(&rid, 100).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(replica = %replica_id, error = ?e, "logs_follow failed");
            return (StatusCode::NOT_FOUND, format!("could not follow logs: {e}")).into_response();
        }
    };

    use futures_util::StreamExt;
    let event_stream = line_stream.map(|line| Ok::<_, Infallible>(Event::default().data(line)));
    Sse::new(event_stream)
        .keep_alive(KeepAlive::new().interval(SSE_KEEPALIVE_INTERVAL))
        .into_response()
}

/// Resolve a `{replica_id}` path param to a `ReplicaId`,
/// 400ing on a malformed UUID. Shared by the action handlers.
///
/// The `Err` is `Box<Response>` so the happy-path `Result` stays a
/// thin pointer-sized value — axum's `Response` is ~heavy and clippy
/// flagged the `Err` variant size otherwise. Callers `return *boxed`.
fn parse_replica_id(s: &str) -> Result<ReplicaId, Box<Response>> {
    uuid::Uuid::parse_str(s)
        .map(ReplicaId)
        .map_err(|_| Box::new((StatusCode::BAD_REQUEST, "invalid replica id").into_response()))
}

/// POST `/admin/dashboard/replicas/{id}/stop` — stop a replica
/// and drop it from the registry. The auto-scaler will respawn
/// it to `min-replicas` on its next tick if the spec demands a
/// baseline, so for a `min-replicas: 1` spec this reads as
/// "recycle this container".
///
/// Safe against CSRF by the admin cookie being SameSite=Strict.
/// Redirects back to the dashboard so the browser lands on a
/// fresh render.
async fn stop_replica(
    _: RequireEditor,
    State(state): State<AppState>,
    Path(replica_id): Path<String>,
) -> Response {
    let rid = match parse_replica_id(&replica_id) {
        Ok(r) => r,
        Err(resp) => return *resp,
    };
    let Some(backend) = state.backend.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no backend").into_response();
    };

    if let Err(e) = backend.stop(&rid).await {
        tracing::warn!(replica = %replica_id, error = ?e, "stop replica failed");
        // Fall through to remove it from the registry anyway —
        // a stop that errored usually means the container is
        // already gone, and we don't want a phantom row.
    }
    state.replicas.write().await.remove(&rid);
    tracing::info!(replica = %replica_id, "replica stopped via dashboard");
    Redirect::to("/admin/dashboard").into_response()
}

/// POST `/admin/dashboard/replicas/{id}/restart` — stop the
/// replica, then immediately spawn a fresh one for the same
/// spec. Unlike plain stop (which leans on the scaler to
/// respawn ~one tick later), restart brings capacity back
/// right away.
async fn restart_replica(
    _: RequireEditor,
    State(state): State<AppState>,
    Path(replica_id): Path<String>,
) -> Response {
    let rid = match parse_replica_id(&replica_id) {
        Ok(r) => r,
        Err(resp) => return *resp,
    };
    let Some(backend) = state.backend.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no backend").into_response();
    };

    // Resolve the spec before stopping so a restart of a
    // since-deleted spec fails cleanly without first killing
    // the running container.
    let spec_id = {
        let reg = state.replicas.read().await;
        let found = reg.all().find(|r| r.id == rid).map(|r| r.spec_id.clone());
        found
    };
    let Some(spec_id) = spec_id else {
        return (StatusCode::NOT_FOUND, "replica not found").into_response();
    };
    // DB-first (admin edits + showcase seed), not just the YAML config —
    // otherwise a DB-only spec's replica couldn't be restarted (#257).
    let Some(spec) = crate::routes::proxy::find_spec(&state, &spec_id).await else {
        return (
            StatusCode::CONFLICT,
            format!("spec `{spec_id}` no longer exists; cannot restart"),
        )
            .into_response();
    };

    if let Err(e) = backend.stop(&rid).await {
        tracing::warn!(replica = %replica_id, error = ?e, "restart: stop phase failed");
    }
    state.replicas.write().await.remove(&rid);

    if let Err(e) = crate::scaler::spawn_replica(&state, &spec).await {
        tracing::error!(spec = %spec.id, error = ?e, "restart: spawn phase failed");
        return (
            StatusCode::BAD_GATEWAY,
            format!("stopped old replica but failed to spawn a new one: {e}"),
        )
            .into_response();
    }
    tracing::info!(replica = %replica_id, spec = %spec.id, "replica restarted via dashboard");
    Redirect::to("/admin/dashboard").into_response()
}

/// Format a chrono `Duration` as a short uptime: "45s", "12m",
/// "2h 14m", "3d 7h". One-line because that's how the table
/// renders it; longer formats land in the per-replica detail
/// view (future).
fn format_uptime(d: chrono::Duration) -> String {
    let secs = d.num_seconds().max(0);
    if secs < 60 {
        return format!("{secs}s");
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m");
    }
    let hours = mins / 60;
    let leftover_min = mins % 60;
    if hours < 24 {
        return format!("{hours}h {leftover_min:02}m");
    }
    let days = hours / 24;
    let leftover_h = hours % 24;
    format!("{days}d {leftover_h}h")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_for_html_script_neutralizes_script_breakout() {
        // A spec display_name / container host carrying `</script>` must
        // not break out of the embedded <script> island (stored XSS).
        #[derive(serde::Serialize)]
        struct Row {
            display_name: String,
        }
        let evil = Row {
            display_name: "</script><img src=x onerror=alert(1)>".into(),
        };
        let out = json_for_html_script(&evil);
        assert!(
            !out.contains("</script>"),
            "literal </script> must not survive: {out}"
        );
        assert!(!out.contains("<img"), "literal < must be escaped: {out}");
        assert!(out.contains("\\u003c/script\\u003e"), "got: {out}");
        // Still valid JSON that round-trips to the original string.
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v["display_name"], "</script><img src=x onerror=alert(1)>",
            "JSON.parse must recover the original value"
        );
    }

    #[test]
    fn format_uptime_picks_the_right_unit() {
        use chrono::Duration as D;
        assert_eq!(format_uptime(D::seconds(0)), "0s");
        assert_eq!(format_uptime(D::seconds(45)), "45s");
        assert_eq!(format_uptime(D::seconds(60)), "1m");
        assert_eq!(format_uptime(D::minutes(45)), "45m");
        assert_eq!(format_uptime(D::minutes(60)), "1h 00m");
        assert_eq!(format_uptime(D::minutes(134)), "2h 14m");
        assert_eq!(format_uptime(D::hours(24)), "1d 0h");
        assert_eq!(format_uptime(D::hours(79)), "3d 7h");
    }

    #[test]
    fn format_uptime_treats_negative_as_zero() {
        // Clock skew between the server and Docker should never
        // produce a "-5s" label.
        use chrono::Duration as D;
        assert_eq!(format_uptime(D::seconds(-30)), "0s");
    }

    #[test]
    fn format_bytes_picks_the_right_unit() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(1023), "1023 B");
        assert_eq!(format_bytes(1024), "1 KB");
        assert_eq!(format_bytes(2_000_000), "1 MB");
        assert_eq!(format_bytes(512 * 1024 * 1024), "512 MB");
        assert_eq!(format_bytes(1_500_000_000), "1.4 GB");
    }

    // -------------------------------------------------------------
    // Template rendering — fake data, no DB / backend / HTTP.
    // Catches template-side regressions (missing field, wrong
    // helper signature, malformed loop) at unit-test speed.
    // -------------------------------------------------------------

    use crate::i18n::Locales;
    use crate::theme::Theme;

    fn load_locales() -> Locales {
        Locales::load().expect("load locales")
    }

    fn fake_row(spec: &str, name: &str, st: ReplicaState, active: u32, max: u32) -> ReplicaRow {
        let (state_code, state_dot) = state_codes(st);
        ReplicaRow {
            replica_id: uuid::Uuid::new_v4().to_string(),
            spec_id: spec.into(),
            display_name: name.into(),
            state: state_code,
            state_dot,
            // Hard-coded pt labels — keeps the test independent
            // of the live locale bundle for these specific keys.
            state_label: match st {
                ReplicaState::Ready => "pronto",
                ReplicaState::Starting => "iniciando",
                ReplicaState::Draining => "drenando",
                ReplicaState::Stopped => "parado",
                ReplicaState::Failed => "falhou",
            }
            .into(),
            uptime: "2h 14m".into(),
            sessions_active: active,
            sessions_max: max,
            container_short: "abc123def456".into(),
            cpu_display: None,
            memory_display: None,
            cpu_history: Vec::new(),
            mem_history: Vec::new(),
            host: String::new(),
        }
    }

    fn render_with(rows: Vec<ReplicaRow>, backend_connected: bool) -> String {
        let locales = load_locales();
        let page = DashboardPage {
            locale: Locale::Pt,
            theme: Theme::Auto,
            locales: &locales,
            locales_all: &Locale::ALL,
            base: std::sync::Arc::from(""),
            nav_section: "dashboard",
            role: Role::Admin,
            backend_connected,
            total_containers: rows.len(),
            total_sessions: rows.iter().map(|r| r.sessions_active).sum(),
            spec_count: rows
                .iter()
                .map(|r| r.spec_id.as_str())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            tracker_sessions: 0,
            total_memory_bytes: 0,
            total_memory_display: "—".to_string(),
            rows,
            snapshot_json: "{}".to_string(),
        };
        page.render().expect("render dashboard")
    }

    fn render_logs(lines: Vec<String>) -> String {
        let locales = load_locales();
        let page = LogsPage {
            locale: Locale::Pt,
            theme: Theme::Auto,
            locales: &locales,
            locales_all: &Locale::ALL,
            base: std::sync::Arc::from(""),
            nav_section: "dashboard",
            role: Role::Admin,
            display_name: "Aurora Prime".into(),
            spec_id: "sales-dashboard".into(),
            replica_id: "11111111-2222-3333-4444-555555555555".into(),
            lines,
        };
        page.render().expect("render logs")
    }

    #[test]
    fn logs_page_renders_each_line() {
        let html = render_logs(vec![
            "2026-05-24 starting nginx".into(),
            "worker process 1 started".into(),
        ]);
        assert!(html.contains("Aurora Prime"));
        assert!(html.contains("sales-dashboard"));
        assert!(html.contains("11111111-2222-3333-4444-555555555555"));
        assert!(html.contains("starting nginx"));
        assert!(html.contains("worker process 1 started"));
        assert!(html.contains("últimas linhas"));
        // The empty hint div is always in the DOM now (the
        // follow-mode JS toggles it), but with lines present it
        // must be hidden with `display:none`.
        assert!(
            html.contains(r#"id="logs-empty" class="logs-empty" style="display:none""#),
            "empty hint should be present-but-hidden when lines exist"
        );
        // The <pre> is visible (no inline display:none on it).
        assert!(html.contains(r#"id="logs-pre">"#));
    }

    #[test]
    fn logs_page_renders_empty_hint() {
        let html = render_logs(vec![]);
        // With no lines the empty hint is visible (no display:none).
        assert!(html.contains(r#"id="logs-empty" class="logs-empty">"#));
        assert!(html.contains("Sem saída de log"));
        // ...and the <pre> + note are hidden.
        assert!(html.contains(r#"id="logs-pre" style="display:none""#));
    }

    #[test]
    fn logs_page_escapes_html_in_log_lines() {
        // A log line containing HTML must not break out of the
        // <pre>. Askama auto-escapes by default. We don't assert
        // the exact entity form (askama may emit `&lt;` or
        // `&#60;`) — only that the raw injectable form is gone
        // and the `<` got encoded to *some* entity.
        let html = render_logs(vec!["<script>alert(1)</script>".into()]);
        assert!(
            !html.contains("<script>alert(1)</script>"),
            "raw script tag must not survive: {html}"
        );
        // The literal payload's `<` must be entity-encoded.
        assert!(
            html.contains("&lt;script&gt;") || html.contains("&#60;script&#62;"),
            "escaped form not found in: {html}"
        );
    }

    #[test]
    fn renders_empty_state_when_no_replicas() {
        let html = render_with(vec![], true);
        assert!(!html.contains("admin-dashboard-no-replicas"),
            "raw key must be translated, not echoed");
        // pt-BR empty-state line is present
        assert!(html.contains("Nenhuma réplica em execução"));
        assert!(!html.contains("backend-missing"));
    }

    #[test]
    fn renders_backend_missing_banner_when_disconnected() {
        let html = render_with(vec![], false);
        assert!(html.contains("backend Docker não está conectado"));
    }

    #[test]
    fn renders_each_row_with_its_state_dot_and_label() {
        let html = render_with(
            vec![
                fake_row("alpha", "Alpha App", ReplicaState::Ready, 3, 10),
                fake_row("beta", "Beta App", ReplicaState::Starting, 0, 5),
                fake_row("gamma", "Gamma App", ReplicaState::Draining, 1, 1),
            ],
            true,
        );
        assert!(html.contains("Alpha App"));
        assert!(html.contains("Beta App"));
        assert!(html.contains("Gamma App"));
        // pt-BR state labels
        assert!(html.contains("pronto"));
        assert!(html.contains("iniciando"));
        assert!(html.contains("drenando"));
        // dot classes
        assert!(html.contains("dot-on"));
        assert!(html.contains("dot-pulse"));
        assert!(html.contains("dot-warm"));
        // sessions column
        assert!(html.contains("3</span>") || html.contains(">3<"));
    }

    #[test]
    fn metric_cards_show_aggregated_counts() {
        let html = render_with(
            vec![
                fake_row("alpha", "Alpha", ReplicaState::Ready, 2, 5),
                fake_row("beta", "Beta", ReplicaState::Ready, 4, 10),
            ],
            true,
        );
        // 2 containers
        assert!(html.contains(">2</div>"));
        // 6 total sessions
        assert!(html.contains(">6</div>"));
    }
}
