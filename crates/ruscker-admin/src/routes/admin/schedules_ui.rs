//! Admin > Schedules — cron-triggered run-to-completion jobs (#986
//! slice C). Admin-only.
//!
//! The storage and the scheduler shipped in slices A/B
//! ([`crate::db::schedules`] + [`crate::jobs`]); this page is the CRUD
//! surface over them: create a schedule for a containerized spec,
//! enable/disable/delete it, and read the recent run history (status,
//! exit code, duration, log tail). Validation is server-side — the
//! cron expression must parse under the same `croner` grammar the
//! scheduler evaluates, and the spec must exist in the effective
//! catalog WITH a `container-image` (an External spec has nothing to
//! run).

use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use crate::auth::{RequireAdmin, Role};
use crate::db::schedules::{RunRow, Schedule};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

use super::KpiMetric;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/schedules", get(index).post(create))
        .route("/admin/schedules/{id}/toggle", post(toggle))
        .route("/admin/schedules/{id}/delete", post(delete))
}

/// One schedule row, pre-formatted for the template.
struct ScheduleRow {
    id: i64,
    spec_id: String,
    cron: String,
    /// First line of the argv override, or an em dash for "the spec's
    /// own command".
    cmd_summary: String,
    /// `YYYY-MM-DD HH:MM UTC`, or an em dash when the schedule is
    /// disabled or its cron no longer parses.
    next_run: String,
    /// `YYYY-MM-DD HH:MM UTC`, or an em dash before the first fire.
    last_run: String,
    enabled: bool,
}

/// One run-history row, pre-formatted for the template.
struct RunView {
    spec_id: String,
    started: String,
    /// `ok` | `failed` | `error` — drives the badge colour.
    status: String,
    exit_code: String,
    duration: String,
    log_tail: String,
}

#[derive(Template)]
#[template(path = "admin/schedules.html")]
struct SchedulesPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    /// Containerized specs from the effective catalog — `(id, display
    /// name)` — for the create form's select. External specs are
    /// excluded: there is nothing to run.
    specs: Vec<(String, String)>,
    schedules: Vec<ScheduleRow>,
    runs: Vec<RunView>,
    kpi_jobs: i64,
    kpi_active: i64,
    kpi_recent_failures: i64,
    flash: Option<&'static str>,
    flash_error: bool,
}

impl SchedulesPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    fn kpis(&self) -> [KpiMetric; 3] {
        [
            KpiMetric::new("ti-clock", "admin-schedules-kpi-jobs", self.kpi_jobs),
            KpiMetric::new(
                "ti-circle-check",
                "admin-schedules-kpi-active",
                self.kpi_active,
            ),
            KpiMetric::new(
                "ti-alert-triangle",
                "admin-schedules-kpi-recent-failures",
                self.kpi_recent_failures,
            ),
        ]
    }
}

/// `2026-07-13T03:00:12Z` → `2026-07-13 03:00 UTC` — minute precision
/// is all a cron UI needs.
fn fmt_utc(dt: &DateTime<Utc>) -> String {
    dt.format("%Y-%m-%d %H:%M UTC").to_string()
}

/// The schedule's next occurrence, from the same anchor the scheduler
/// uses ([`crate::jobs`]): the last fire marker, or creation for a
/// schedule that never fired (no fire-on-create). `None` when the
/// stored cron no longer parses.
fn next_run(schedule: &Schedule) -> Option<DateTime<Utc>> {
    let anchor = schedule.last_run_at.unwrap_or(schedule.created_at);
    let parsed: croner::Cron = schedule.cron.parse().ok()?;
    parsed.find_next_occurrence(&anchor, false).ok()
}

fn schedule_row(s: Schedule) -> ScheduleRow {
    let next = if s.enabled {
        next_run(&s).map(|dt| fmt_utc(&dt))
    } else {
        None // a disabled schedule has no next fire
    };
    ScheduleRow {
        cmd_summary: s
            .cmd_override()
            .and_then(|argv| argv.first().cloned())
            .unwrap_or_else(|| "—".to_string()),
        next_run: next.unwrap_or_else(|| "—".to_string()),
        last_run: s.last_run_at.map(|dt| fmt_utc(&dt)).unwrap_or_else(|| "—".to_string()),
        id: s.id,
        spec_id: s.spec_id,
        cron: s.cron,
        enabled: s.enabled,
    }
}

/// `1234` ms → `1.2 s`; sub-second stays in ms. Missing → em dash.
fn fmt_duration(ms: Option<i64>) -> String {
    match ms {
        None => "—".to_string(),
        Some(ms) if ms < 1000 => format!("{ms} ms"),
        Some(ms) => format!("{:.1} s", ms as f64 / 1000.0),
    }
}

fn run_view(r: RunRow) -> RunView {
    RunView {
        spec_id: r.spec_id,
        started: fmt_utc(&r.started_at),
        status: r.status,
        exit_code: r.exit_code.map(|c| c.to_string()).unwrap_or_else(|| "—".to_string()),
        duration: fmt_duration(r.duration_ms),
        log_tail: r.log_tail.unwrap_or_default(),
    }
}

#[derive(Debug, Deserialize)]
struct FlashQuery {
    flash: Option<String>,
}

async fn index(
    admin: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(q): Query<FlashQuery>,
) -> Response {
    // Whitelist the flash codes — the query string is user input.
    let (flash, flash_error) = match q.flash.as_deref() {
        Some("created") => (Some("admin-schedules-flash-created"), false),
        Some("deleted") => (Some("admin-schedules-flash-deleted"), false),
        Some("toggled") => (Some("admin-schedules-flash-toggled"), false),
        Some("bad-cron") => (Some("admin-schedules-flash-bad-cron"), true),
        Some("bad-spec") => (Some("admin-schedules-flash-bad-spec"), true),
        Some("error") => (Some("admin-schedules-flash-error"), true),
        _ => (None, false),
    };

    // Only containerized specs can be scheduled — an External spec is a
    // link, there is nothing to run.
    let specs: Vec<(String, String)> = crate::catalog::effective_specs_cached(&state)
        .await
        .iter()
        .filter(|s| s.container_image.is_some())
        .map(|s| {
            (
                s.id.clone(),
                s.display_name.clone().unwrap_or_else(|| s.id.clone()),
            )
        })
        .collect();

    let (schedules, runs) = match state.db.as_ref() {
        Some(db) => {
            let schedules = crate::db::schedules::list_all(db).await.unwrap_or_else(|e| {
                tracing::warn!(error = ?e, "schedules: list failed");
                Vec::new()
            });
            let runs = crate::db::schedules::recent_runs(db, 20).await.unwrap_or_else(|e| {
                tracing::warn!(error = ?e, "schedules: recent runs failed");
                Vec::new()
            });
            (schedules, runs)
        }
        None => (Vec::new(), Vec::new()),
    };

    let kpi_jobs = schedules.len() as i64;
    let kpi_active = schedules.iter().filter(|schedule| schedule.enabled).count() as i64;
    let kpi_recent_failures = runs.iter().filter(|run| run.status != "ok").count() as i64;

    super::render(&SchedulesPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "schedules",
        role: admin.role,
        specs,
        schedules: schedules.into_iter().map(schedule_row).collect(),
        runs: runs.into_iter().map(run_view).collect(),
        kpi_jobs,
        kpi_active,
        kpi_recent_failures,
        flash,
        flash_error,
    })
}

#[derive(Debug, Deserialize)]
struct CreateForm {
    spec_id: String,
    cron: String,
    /// Multiline: one argv element per line. Empty ⇒ the spec's own
    /// command (its `container-cmd`, else the image's baked CMD).
    #[serde(default)]
    cmd: String,
    /// Minutes; empty ⇒ the backend's default cap (1 h).
    #[serde(default)]
    timeout_mins: String,
}

/// Textarea lines → the argv override: trimmed, empties dropped,
/// serialized as a JSON array for `schedules.cmd_json`. All-empty ⇒
/// `None` (run the spec's own command).
fn cmd_json_from_lines(cmd: &str) -> Option<String> {
    let argv: Vec<&str> = cmd.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if argv.is_empty() {
        None
    } else {
        // Serializing a Vec<&str> can't fail.
        Some(serde_json::to_string(&argv).expect("serialize argv"))
    }
}

/// Empty ⇒ `Ok(None)` (backend default); a positive integer number of
/// minutes ⇒ seconds; anything else ⇒ `Err` (rejected as bad input).
fn timeout_secs_from_mins(timeout_mins: &str) -> Result<Option<i64>, ()> {
    let t = timeout_mins.trim();
    if t.is_empty() {
        return Ok(None);
    }
    match t.parse::<u64>() {
        // Cap at something absurd-but-safe so `* 60` can't overflow i64.
        Ok(mins) if mins > 0 && mins <= 60 * 24 * 365 => Ok(Some((mins * 60) as i64)),
        _ => Err(()),
    }
}

async fn create(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Form(form): Form<CreateForm>,
) -> Response {
    let Some(db) = state.db.as_ref() else {
        return redirect("error");
    };

    // The spec must exist in the effective catalog and be containerized
    // (External has nothing to run) — same gate the scheduler applies
    // per tick, enforced here so a schedule is never born broken.
    let spec_ok = crate::catalog::effective_specs_cached(&state)
        .await
        .iter()
        .any(|s| s.id == form.spec_id && s.container_image.is_some());
    if !spec_ok {
        return redirect("bad-spec");
    }

    // The exact grammar the scheduler evaluates (croner, minute-level).
    let cron = form.cron.trim();
    if cron.is_empty() || cron.parse::<croner::Cron>().is_err() {
        return redirect("bad-cron");
    }

    let cmd_json = cmd_json_from_lines(&form.cmd);
    let Ok(timeout_secs) = timeout_secs_from_mins(&form.timeout_mins) else {
        return redirect("error");
    };

    match crate::db::schedules::insert(
        db,
        &form.spec_id,
        cron,
        cmd_json.as_deref(),
        timeout_secs,
        Some(admin.actor()),
    )
    .await
    {
        Ok(()) => redirect("created"),
        Err(e) => {
            tracing::warn!(error = ?e, spec = %form.spec_id, "schedules: create failed");
            redirect("error")
        }
    }
}

async fn toggle(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    let Some(db) = state.db.as_ref() else {
        return redirect("error");
    };
    // No fetch-by-id in the store (the table is small); find the current
    // enabled bit in the full listing.
    let current = match crate::db::schedules::list_all(db).await {
        Ok(all) => all.into_iter().find(|s| s.id == id),
        Err(e) => {
            tracing::warn!(error = ?e, id, "schedules: list for toggle failed");
            return redirect("error");
        }
    };
    let Some(schedule) = current else {
        return redirect("error");
    };
    match crate::db::schedules::set_enabled(db, id, !schedule.enabled, Some(admin.actor())).await {
        Ok(()) => redirect("toggled"),
        Err(e) => {
            tracing::warn!(error = ?e, id, "schedules: toggle failed");
            redirect("error")
        }
    }
}

async fn delete(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Response {
    let Some(db) = state.db.as_ref() else {
        return redirect("error");
    };
    match crate::db::schedules::delete(db, id, Some(admin.actor())).await {
        Ok(()) => redirect("deleted"),
        Err(e) => {
            tracing::warn!(error = ?e, id, "schedules: delete failed");
            redirect("error")
        }
    }
}

/// Post/redirect/get back to the page with a one-word flash code. The
/// base-path response rewriter re-prefixes the `Location` header.
fn redirect(flash: &str) -> Response {
    Redirect::to(&format!("/admin/schedules?flash={flash}")).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmd_lines_become_a_json_argv_or_none() {
        assert_eq!(cmd_json_from_lines(""), None);
        assert_eq!(cmd_json_from_lines("  \n\n  "), None);
        assert_eq!(
            cmd_json_from_lines("Rscript\n etl.R \n\n--full"),
            Some(r#"["Rscript","etl.R","--full"]"#.to_string())
        );
    }

    #[test]
    fn timeout_minutes_parse_to_seconds_or_default() {
        assert_eq!(timeout_secs_from_mins(""), Ok(None));
        assert_eq!(timeout_secs_from_mins("  "), Ok(None));
        assert_eq!(timeout_secs_from_mins("90"), Ok(Some(5400)));
        assert_eq!(timeout_secs_from_mins("0"), Err(()));
        assert_eq!(timeout_secs_from_mins("-5"), Err(()));
        assert_eq!(timeout_secs_from_mins("abc"), Err(()));
    }

    fn sched(cron: &str, enabled: bool, last: Option<&str>) -> Schedule {
        Schedule {
            id: 7,
            spec_id: "etl".into(),
            cron: cron.into(),
            cmd_json: Some(r#"["Rscript","etl.R"]"#.into()),
            enabled,
            timeout_secs: None,
            last_run_at: last.map(|s| s.parse().unwrap()),
            created_at: "2026-07-01T10:00:00Z".parse().unwrap(),
            updated_at: "2026-07-01T10:00:00Z".parse().unwrap(),
        }
    }

    #[test]
    fn schedule_row_formats_next_and_last_run() {
        // Fired 2026-07-12 03:00 → next daily 03:00 is the 13th.
        let row = schedule_row(sched("0 3 * * *", true, Some("2026-07-12T03:00:10Z")));
        assert_eq!(row.next_run, "2026-07-13 03:00 UTC");
        assert_eq!(row.last_run, "2026-07-12 03:00 UTC");
        assert_eq!(row.cmd_summary, "Rscript");

        // Disabled → no next fire; never fired → em-dash last run.
        let row = schedule_row(sched("0 3 * * *", false, None));
        assert_eq!(row.next_run, "—");
        assert_eq!(row.last_run, "—");

        // A stored cron that no longer parses can't show a next run.
        let row = schedule_row(sched("not a cron", true, None));
        assert_eq!(row.next_run, "—");
    }

    #[test]
    fn durations_format_human_readable() {
        assert_eq!(fmt_duration(None), "—");
        assert_eq!(fmt_duration(Some(80)), "80 ms");
        assert_eq!(fmt_duration(Some(1234)), "1.2 s");
        assert_eq!(fmt_duration(Some(65000)), "65.0 s");
    }
}
