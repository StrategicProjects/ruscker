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
    extract::State,
    response::Response,
    routing::get,
    Router,
};
use chrono::Utc;
use ruscker_core::{Replica, ReplicaState};

use crate::auth::AdminSession;
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/admin/dashboard", get(index))
}

/// One row of the replicas table — flattened for the template.
/// We project `Replica` plus the operator-facing `display_name`
/// resolved from the spec config (replicas only carry `spec_id`).
struct ReplicaRow {
    spec_id: String,
    /// `display-name` from the spec config, or the spec_id if
    /// the spec was renamed/deleted out from under the registry.
    display_name: String,
    state: ReplicaState,
    /// Pre-formatted "2h 14m" string. Built once at render time
    /// so the template stays declarative.
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
}

#[derive(Template)]
#[template(path = "admin/dashboard.html")]
struct DashboardPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    nav_section: &'static str,
    /// `None` when ruscker was started without `--docker`. The
    /// template renders a friendly "wire up Docker" banner
    /// instead of a 0-everything dashboard, which would look
    /// like a real production state.
    backend_connected: bool,
    total_containers: usize,
    total_sessions: u32,
    spec_count: usize,
    tracker_sessions: usize,
    /// Sum of `memory_bytes` across all replicas with cached
    /// metrics. `0` when no replicas have been observed yet
    /// (refresher hasn't ticked, or backend isn't connected).
    total_memory_bytes: u64,
    /// "412 MB" / "1.2 GB" pre-formatted for the top metric card.
    total_memory_display: String,
    rows: Vec<ReplicaRow>,
}

impl<'a> DashboardPage<'a> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// Per-state label key. Kept in Rust (not the template) so a
    /// new `ReplicaState` variant produces a compiler error
    /// instead of a missing translation at render time.
    fn state_label(&self, s: &ReplicaState) -> String {
        let key = match s {
            ReplicaState::Ready => "admin-dashboard-state-ready",
            ReplicaState::Starting => "admin-dashboard-state-starting",
            ReplicaState::Draining => "admin-dashboard-state-draining",
            ReplicaState::Stopped => "admin-dashboard-state-stopped",
            ReplicaState::Failed => "admin-dashboard-state-failed",
        };
        self.t(key)
    }

    /// CSS class for the status dot. Matches the mockup's
    /// `.dot-on / .dot-pulse / .dot-warm / .dot-off` palette.
    fn state_dot(&self, s: &ReplicaState) -> &'static str {
        match s {
            ReplicaState::Ready => "dot-on",
            ReplicaState::Starting => "dot-pulse",
            ReplicaState::Draining => "dot-warm",
            ReplicaState::Stopped | ReplicaState::Failed => "dot-off",
        }
    }
}

async fn index(
    _: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    let backend_connected = state.backend.is_some();

    // Snapshot the registry once. Cloning `Replica` is cheap
    // (a few strings + a SocketAddr + small ints) and keeps the
    // read-lock window tight.
    let snap: Vec<Replica> = {
        let reg = state.replicas.read().await;
        state
            .config
            .proxy
            .specs
            .iter()
            .flat_map(|s| reg.replicas_of(&s.id).iter().cloned().collect::<Vec<_>>())
            .collect()
    };

    let total_containers = snap.len();
    let total_sessions: u32 = snap.iter().map(|r| r.sessions_active).sum();
    let spec_count: usize = snap
        .iter()
        .map(|r| r.spec_id.as_str())
        .collect::<std::collections::HashSet<_>>()
        .len();
    let tracker_sessions = state.sessions.len();

    // Build display rows. `display_name` falls back to spec_id
    // when the operator deleted / renamed a spec out from under
    // a still-running replica. Each row also looks up its
    // cached metrics; rows without cached metrics show "n/a"
    // until the next refresher tick.
    let mut total_memory_bytes: u64 = 0;
    let rows: Vec<ReplicaRow> = snap
        .into_iter()
        .map(|r| {
            let display_name = state
                .config
                .proxy
                .specs
                .iter()
                .find(|s| s.id == r.spec_id)
                .and_then(|s| s.display_name.clone())
                .unwrap_or_else(|| r.spec_id.clone());
            let container_short = r.container_id.chars().take(12).collect();
            let uptime = format_uptime(Utc::now() - r.started_at);
            let cached = state.metrics.get(&r.id);
            let cpu_display = cached.as_ref().map(|c| format!("{:.0}%", c.metrics.cpu_percent));
            let memory_display = cached.as_ref().map(|c| format_bytes(c.metrics.memory_bytes));
            if let Some(c) = cached.as_ref() {
                total_memory_bytes = total_memory_bytes.saturating_add(c.metrics.memory_bytes);
            }
            ReplicaRow {
                spec_id: r.spec_id,
                display_name,
                state: r.state,
                uptime,
                sessions_active: r.sessions_active,
                sessions_max: r.sessions_max,
                container_short,
                cpu_display,
                memory_display,
            }
        })
        .collect();
    let total_memory_display = if total_memory_bytes > 0 {
        format_bytes(total_memory_bytes)
    } else {
        // i18n-friendly default: render an em-dash and let the
        // operator infer "no data yet" from context. The full
        // explanation lives below as the metrics-pending hint
        // when rows exist but none have cached metrics.
        "—".to_string()
    };

    let page = DashboardPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        nav_section: "dashboard",
        backend_connected,
        total_containers,
        total_sessions,
        spec_count,
        tracker_sessions,
        total_memory_bytes,
        total_memory_display,
        rows,
    };
    super::render(&page)
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
        ReplicaRow {
            spec_id: spec.into(),
            display_name: name.into(),
            state: st,
            uptime: "2h 14m".into(),
            sessions_active: active,
            sessions_max: max,
            container_short: "abc123def456".into(),
            cpu_display: None,
            memory_display: None,
        }
    }

    fn render_with(rows: Vec<ReplicaRow>, backend_connected: bool) -> String {
        let locales = load_locales();
        let page = DashboardPage {
            locale: Locale::Pt,
            theme: Theme::Auto,
            locales: &locales,
            locales_all: &Locale::ALL,
            nav_section: "dashboard",
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
        };
        page.render().expect("render dashboard")
    }

    #[test]
    fn renders_empty_state_when_no_replicas() {
        let html = render_with(vec![], true);
        assert!(html.contains("admin-dashboard-no-replicas") == false,
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
