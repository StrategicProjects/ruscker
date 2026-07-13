//! The scheduled-jobs runner (#986 slice B).
//!
//! One task per process, LEADER-ONLY in HA (same posture as the
//! scaler): every [`TICK`] it loads the enabled schedules and, for each
//! one that is DUE (its cron has an occurrence between the last fire
//! marker and now), atomically claims the fire (`mark_fired` compares
//! the previous marker, so a split-brain second scheduler loses the
//! race instead of double-firing), then runs the spec's image to
//! completion via [`ruscker_core::ContainerBackend::run_job`] on a
//! DETACHED task — a long ETL never blocks the tick. The outcome lands
//! in `schedule_runs`, and a non-`Ok` outcome emits a `job-failed`
//! alert through the webhook sink (#930).
//!
//! Missed windows collapse: if the server was down over three nightly
//! occurrences, the schedule fires ONCE on the next tick (the marker
//! then moves past all three) — ETL semantics, not a message queue.

use crate::db::schedules::{RunStatus, Schedule};
use crate::AppState;
use chrono::{DateTime, Utc};
use std::time::Duration;
use tokio::task::JoinHandle;

/// Scheduler cadence. Cron's resolution is the minute, so half-minute
/// ticks keep worst-case lateness ~30 s without busy-waiting.
pub const TICK: Duration = Duration::from_secs(30);

/// The next occurrence of `cron` strictly after `after`, or `None`
/// for an unparseable expression (surfaced by the caller once — a bad
/// expression must not wedge the tick).
fn next_occurrence(cron: &str, after: DateTime<Utc>) -> Option<DateTime<Utc>> {
    let parsed: croner::Cron = cron.parse().ok()?;
    parsed.find_next_occurrence(&after, false).ok()
}

/// Whether `schedule` is due at `now`: its cron has an occurrence in
/// `(anchor, now]`, where the anchor is the last fire marker (or the
/// schedule's creation, so a brand-new "every day at 03:00" waits for
/// 03:00 instead of firing on creation).
fn is_due(schedule: &Schedule, now: DateTime<Utc>) -> bool {
    let anchor = schedule.last_run_at.unwrap_or(schedule.created_at);
    match next_occurrence(&schedule.cron, anchor) {
        Some(next) => next <= now,
        None => false,
    }
}

/// Build the run-to-completion request for a schedule: the SPEC's
/// image/env/volumes/limits/creds (same resolution the spawn path
/// uses — `${VAR}` interpolated now, failing loudly), with the
/// schedule's argv override when set. No public-path injection: a job
/// serves no HTTP.
async fn job_request(
    state: &AppState,
    spec: &ruscker_config::Spec,
    schedule: &Schedule,
) -> anyhow::Result<ruscker_core::SpawnRequest> {
    let image = spec
        .container_image
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("spec {} has no container-image", spec.id))?;
    let creds = crate::routes::proxy::resolve_creds(state, spec).await?;
    let limits = crate::routes::proxy::limits_from_spec(spec);
    let env = spec
        .resolved_env_pairs()
        .map_err(|e| anyhow::anyhow!("spec {} container-env: {e}", spec.id))?;
    let cmd = schedule.cmd_override().or_else(|| spec.container_cmd.clone());

    let mut req = ruscker_core::SpawnRequest::new(&spec.id, image)
        .with_limits(limits)
        .with_volumes(spec.volumes.clone().unwrap_or_default())
        .with_env(env);
    if let Some(platform) = spec.platform.as_deref() {
        req = req.with_platform(platform);
    }
    if let Some(cmd) = cmd {
        req = req.with_cmd(cmd);
    }
    if let Some(net) = spec.effective_container_network() {
        req = req.with_network(net);
    }
    let labels = spec.effective_labels();
    if !labels.is_empty() {
        req = req.with_labels(labels);
    }
    if let Some(c) = creds {
        req = req.with_creds(c);
    }
    // Per-schedule wall-clock cap (#986 slice C). Zero/negative values
    // (nothing can produce them through the UI, but the column is plain
    // i64) fall back to the backend's default rather than a 0s timeout
    // that would kill every job instantly.
    if let Some(t) = schedule.timeout_secs {
        if t > 0 {
            req = req.with_job_timeout(t as u64);
        }
    }
    Ok(req)
}

/// One scheduler pass. Public-for-tests; `spawn` loops it.
async fn tick(state: &AppState) {
    // HA: only the leader fires schedules (the DB claim in
    // `mark_fired` backstops a split brain).
    if !state.leader.is_leader().await {
        return;
    }
    let (Some(db), Some(backend)) = (state.db.as_ref(), state.backend.as_ref()) else {
        return;
    };
    let schedules = match crate::db::schedules::list_all(db).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = ?e, "scheduler: list schedules failed; skipping tick");
            return;
        }
    };
    let now = Utc::now();
    for schedule in schedules.into_iter().filter(|s| s.enabled) {
        if next_occurrence(&schedule.cron, now).is_none() {
            tracing::warn!(
                schedule = schedule.id,
                spec = %schedule.spec_id,
                cron = %schedule.cron,
                "scheduler: unparseable cron expression; schedule skipped"
            );
            continue;
        }
        if !is_due(&schedule, now) {
            continue;
        }
        // Claim BEFORE running — a crash mid-job must not double-fire,
        // and a concurrent second scheduler loses this compare-and-set.
        match crate::db::schedules::mark_fired(db, schedule.id, schedule.last_run_at, now).await {
            Ok(true) => {}
            Ok(false) => continue, // someone else claimed it
            Err(e) => {
                tracing::warn!(error = ?e, schedule = schedule.id, "scheduler: fire claim failed");
                continue;
            }
        }
        let Some(spec) = crate::catalog::effective_specs_cached(state)
            .await
            .iter()
            .find(|s| s.id == schedule.spec_id)
            .cloned()
        else {
            tracing::warn!(
                schedule = schedule.id,
                spec = %schedule.spec_id,
                "scheduler: schedule references a spec that no longer exists"
            );
            let _ = crate::db::schedules::record_run(
                db, schedule.id, now, RunStatus::Error, None,
                "spec no longer exists in the catalog", None,
            )
            .await;
            continue;
        };

        let req = match job_request(state, &spec, &schedule).await {
            Ok(r) => r,
            Err(e) => {
                report(state, &schedule, now, RunStatus::Error, None, &format!("{e}"), None).await;
                continue;
            }
        };

        // Detached: a long ETL must not block the next tick. The run
        // itself is bounded by run_job's cap (the schedule's own
        // timeout when set, else the backend default).
        let state = state.clone();
        let backend = backend.clone();
        tokio::spawn(async move {
            tracing::info!(schedule = schedule.id, spec = %schedule.spec_id, "scheduled job starting");
            match backend.run_job(&req).await {
                Ok(out) => {
                    let status = if out.exit_code == 0 { RunStatus::Ok } else { RunStatus::Failed };
                    report(
                        &state, &schedule, now, status, Some(out.exit_code),
                        &out.log_tail.join("\n"), Some(out.duration_ms as i64),
                    )
                    .await;
                }
                Err(e) => {
                    report(&state, &schedule, now, RunStatus::Error, None, &format!("{e}"), None)
                        .await;
                }
            }
        });
    }
}

/// Persist the run row and, for anything but `Ok`, raise the
/// `job-failed` alert (#930).
async fn report(
    state: &AppState,
    schedule: &Schedule,
    started: DateTime<Utc>,
    status: RunStatus,
    exit_code: Option<i64>,
    log_tail: &str,
    duration_ms: Option<i64>,
) {
    if let Some(db) = state.db.as_ref() {
        if let Err(e) = crate::db::schedules::record_run(
            db, schedule.id, started, status, exit_code, log_tail, duration_ms,
        )
        .await
        {
            tracing::warn!(error = ?e, schedule = schedule.id, "scheduler: record run failed");
        }
    }
    match status {
        RunStatus::Ok => {
            tracing::info!(schedule = schedule.id, spec = %schedule.spec_id, "scheduled job succeeded");
        }
        RunStatus::Failed | RunStatus::Error => {
            tracing::warn!(
                schedule = schedule.id,
                spec = %schedule.spec_id,
                ?exit_code,
                "scheduled job failed"
            );
            state.alerts.notify(crate::alerts::AlertEvent {
                kind: crate::alerts::AlertKind::JobFailed,
                spec: schedule.spec_id.clone(),
                replica: None,
                message: match (status, exit_code) {
                    (RunStatus::Failed, Some(code)) => format!(
                        "scheduled job for `{}` (cron `{}`) exited with code {code}",
                        schedule.spec_id, schedule.cron
                    ),
                    _ => format!(
                        "scheduled job for `{}` (cron `{}`) could not run: {log_tail}",
                        schedule.spec_id, schedule.cron
                    ),
                },
            });
        }
    }
}

/// Start the scheduler loop. Detached like the scaler — every tick is
/// idempotent (the DB claim makes fires exactly-once per occurrence).
pub fn spawn(state: AppState) -> JoinHandle<()> {
    tokio::spawn(async move {
        tracing::info!(tick = ?TICK, "job scheduler started");
        loop {
            tokio::time::sleep(TICK).await;
            tick(&state).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sched(cron: &str, last: Option<&str>, created: &str) -> Schedule {
        Schedule {
            id: 1,
            spec_id: "etl".into(),
            cron: cron.into(),
            cmd_json: None,
            enabled: true,
            timeout_secs: None,
            last_run_at: last.map(|s| s.parse().unwrap()),
            created_at: created.parse().unwrap(),
            updated_at: created.parse().unwrap(),
        }
    }

    #[test]
    fn due_when_an_occurrence_passed_since_the_anchor() {
        let now: DateTime<Utc> = "2026-07-13T03:05:00Z".parse().unwrap();
        // Daily at 03:00, last fired yesterday 03:00 → due.
        assert!(is_due(&sched("0 3 * * *", Some("2026-07-12T03:00:30Z"), "2026-07-01T00:00:00Z"), now));
        // Already fired today at 03:00 → not due again.
        assert!(!is_due(&sched("0 3 * * *", Some("2026-07-13T03:00:30Z"), "2026-07-01T00:00:00Z"), now));
        // Brand new schedule created at 02:00 today → 03:00 passed → due.
        assert!(is_due(&sched("0 3 * * *", None, "2026-07-13T02:00:00Z"), now));
        // Brand new created at 04:00 → next is tomorrow → NOT due (no
        // fire-on-create).
        let later: DateTime<Utc> = "2026-07-13T04:10:00Z".parse().unwrap();
        assert!(!is_due(&sched("0 3 * * *", None, "2026-07-13T04:00:00Z"), later));
        // Downtime over several occurrences collapses to one firing.
        assert!(is_due(&sched("0 3 * * *", Some("2026-07-09T03:00:00Z"), "2026-07-01T00:00:00Z"), now));
    }

    #[test]
    fn unparseable_cron_is_never_due() {
        let now = Utc::now();
        assert!(!is_due(&sched("not a cron", None, "2026-07-01T00:00:00Z"), now));
        assert!(next_occurrence("61 99 * * *", now).is_none());
    }
}
