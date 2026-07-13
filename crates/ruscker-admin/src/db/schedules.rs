//! Scheduled-job storage (#986 slice B): `schedules` (a cron per spec,
//! with an optional argv override) and `schedule_runs` (the history the
//! admin UI shows in slice C). The SCHEDULER decides what's due — this
//! module only stores; `mark_fired` sets `last_run_at` **before** the
//! job runs so a crash mid-job can never double-fire.

use super::ConfigDb;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Schedule {
    pub id: i64,
    pub spec_id: String,
    pub cron: String,
    /// JSON argv array, or `None` = run the spec's own command.
    pub cmd_json: Option<String>,
    pub enabled: bool,
    pub timeout_secs: Option<i64>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl Schedule {
    /// The parsed argv override, if any. A malformed stored JSON is
    /// treated as "no override" (and logged by the caller).
    pub fn cmd_override(&self) -> Option<Vec<String>> {
        self.cmd_json
            .as_deref()
            .and_then(|j| serde_json::from_str(j).ok())
    }
}

/// One row of run history, joined with its schedule so the admin UI
/// (#986 slice C) can show which SPEC ran without a second query. A
/// run whose schedule was deleted disappears with it (`ON DELETE
/// CASCADE` on `schedule_runs.schedule_id`).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RunRow {
    pub schedule_id: i64,
    pub spec_id: String,
    pub started_at: DateTime<Utc>,
    /// `ok` | `failed` | `error` (see [`RunStatus`]).
    pub status: String,
    pub exit_code: Option<i64>,
    pub log_tail: Option<String>,
    pub duration_ms: Option<i64>,
}

/// Outcome bucket for a run row. `Error` = the job could not run at
/// all; `Failed` = ran and exited non-zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunStatus {
    Ok,
    Failed,
    Error,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Ok => "ok",
            RunStatus::Failed => "failed",
            RunStatus::Error => "error",
        }
    }
}

pub async fn list_all(db: &ConfigDb) -> Result<Vec<Schedule>> {
    let rows = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as::<_, Schedule>("SELECT * FROM schedules ORDER BY spec_id, id")
                .fetch_all(pool)
                .await
                .context("list schedules (sqlite)")?
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as::<_, Schedule>("SELECT * FROM schedules ORDER BY spec_id, id")
                .fetch_all(pool)
                .await
                .context("list schedules (postgres)")?
        }
    };
    Ok(rows)
}

/// The most recent runs across ALL schedules, newest first — the
/// "Latest runs" table on `/admin/schedules`.
pub async fn recent_runs(db: &ConfigDb, limit: i64) -> Result<Vec<RunRow>> {
    // The JOIN resolves spec_id; the index on (schedule_id, started_at)
    // doesn't help a global ORDER BY, but the table is small and the
    // LIMIT keeps the scan bounded in practice.
    let rows = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as::<_, RunRow>(
            "SELECT r.schedule_id, s.spec_id, r.started_at, r.status, r.exit_code,
                    r.log_tail, r.duration_ms
             FROM schedule_runs r
             JOIN schedules s ON s.id = r.schedule_id
             ORDER BY r.started_at DESC
             LIMIT ?",
        )
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("recent schedule runs (sqlite)")?,
        ConfigDb::Postgres(pool) => sqlx::query_as::<_, RunRow>(
            "SELECT r.schedule_id, s.spec_id, r.started_at, r.status, r.exit_code,
                    r.log_tail, r.duration_ms
             FROM schedule_runs r
             JOIN schedules s ON s.id = r.schedule_id
             ORDER BY r.started_at DESC
             LIMIT $1",
        )
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("recent schedule runs (postgres)")?,
    };
    Ok(rows)
}

pub async fn insert(
    db: &ConfigDb,
    spec_id: &str,
    cron: &str,
    cmd_json: Option<&str>,
    timeout_secs: Option<i64>,
    actor: Option<&str>,
) -> Result<()> {
    let now = Utc::now();
    let target = format!("schedule:{spec_id}");
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin schedule insert")?;
            sqlx::query(
                "INSERT INTO schedules (spec_id, cron, cmd_json, enabled, timeout_secs, created_at, updated_at)
                 VALUES (?, ?, ?, 1, ?, ?, ?)",
            )
            .bind(spec_id)
            .bind(cron)
            .bind(cmd_json)
            .bind(timeout_secs)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("insert schedule (sqlite)")?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'schedule.create', ?, ?, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(serde_json::json!({ "cron": cron }).to_string())
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit schedule insert")?;
            tx.commit().await.context("commit schedule insert")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin schedule insert")?;
            sqlx::query(
                "INSERT INTO schedules (spec_id, cron, cmd_json, enabled, timeout_secs, created_at, updated_at)
                 VALUES ($1, $2, $3, TRUE, $4, $5, $6)",
            )
            .bind(spec_id)
            .bind(cron)
            .bind(cmd_json)
            .bind(timeout_secs)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("insert schedule (postgres)")?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'schedule.create', $2, $3, $4)",
            )
            .bind(actor)
            .bind(&target)
            .bind(serde_json::json!({ "cron": cron }).to_string())
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit schedule insert")?;
            tx.commit().await.context("commit schedule insert")?;
        }
    }
    Ok(())
}

pub async fn set_enabled(db: &ConfigDb, id: i64, enabled: bool, actor: Option<&str>) -> Result<()> {
    let now = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query("UPDATE schedules SET enabled = ?, updated_at = ? WHERE id = ?")
                .bind(enabled)
                .bind(now)
                .bind(id)
                .execute(pool)
                .await
                .context("set schedule enabled (sqlite)")?;
            super::audit::record(db, actor.unwrap_or("system"), "schedule.update", &format!("schedule:{id}"), None)
                .await?;
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query("UPDATE schedules SET enabled = $1, updated_at = $2 WHERE id = $3")
                .bind(enabled)
                .bind(now)
                .bind(id)
                .execute(pool)
                .await
                .context("set schedule enabled (postgres)")?;
            super::audit::record(db, actor.unwrap_or("system"), "schedule.update", &format!("schedule:{id}"), None)
                .await?;
        }
    }
    Ok(())
}

pub async fn delete(db: &ConfigDb, id: i64, actor: Option<&str>) -> Result<()> {
    match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query("DELETE FROM schedules WHERE id = ?")
                .bind(id)
                .execute(pool)
                .await
                .context("delete schedule (sqlite)")?;
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query("DELETE FROM schedules WHERE id = $1")
                .bind(id)
                .execute(pool)
                .await
                .context("delete schedule (postgres)")?;
        }
    }
    super::audit::record(db, actor.unwrap_or("system"), "schedule.delete", &format!("schedule:{id}"), None).await
}

/// Set the fire marker. Called BEFORE the job runs; the WHERE clause
/// on the previous value makes the claim atomic, so even a split-brain
/// second scheduler can't double-fire the same tick (mirrors the
/// scaler's belt-and-braces posture, #596). Returns whether this
/// caller won the claim.
pub async fn mark_fired(
    db: &ConfigDb,
    id: i64,
    previous: Option<DateTime<Utc>>,
    now: DateTime<Utc>,
) -> Result<bool> {
    let n = match db {
        ConfigDb::Sqlite(pool) => sqlx::query(
            "UPDATE schedules SET last_run_at = ? WHERE id = ? AND last_run_at IS ?",
        )
        .bind(now)
        .bind(id)
        .bind(previous)
        .execute(pool)
        .await
        .context("mark schedule fired (sqlite)")?
        .rows_affected(),
        ConfigDb::Postgres(pool) => sqlx::query(
            "UPDATE schedules SET last_run_at = $1 WHERE id = $2 AND last_run_at IS NOT DISTINCT FROM $3",
        )
        .bind(now)
        .bind(id)
        .bind(previous)
        .execute(pool)
        .await
        .context("mark schedule fired (postgres)")?
        .rows_affected(),
    };
    Ok(n == 1)
}

/// Record one finished (or failed-to-start) run.
#[allow(clippy::too_many_arguments)]
pub async fn record_run(
    db: &ConfigDb,
    schedule_id: i64,
    started_at: DateTime<Utc>,
    status: RunStatus,
    exit_code: Option<i64>,
    log_tail: &str,
    duration_ms: Option<i64>,
) -> Result<()> {
    let finished = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO schedule_runs (schedule_id, started_at, finished_at, status, exit_code, log_tail, duration_ms)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(schedule_id)
            .bind(started_at)
            .bind(finished)
            .bind(status.as_str())
            .bind(exit_code)
            .bind(log_tail)
            .bind(duration_ms)
            .execute(pool)
            .await
            .context("record schedule run (sqlite)")?;
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO schedule_runs (schedule_id, started_at, finished_at, status, exit_code, log_tail, duration_ms)
                 VALUES ($1, $2, $3, $4, $5, $6, $7)",
            )
            .bind(schedule_id)
            .bind(started_at)
            .bind(finished)
            .bind(status.as_str())
            .bind(exit_code)
            .bind(log_tail)
            .bind(duration_ms)
            .execute(pool)
            .await
            .context("record schedule run (postgres)")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_db() -> ConfigDb {
        ConfigDb::Sqlite(crate::db::open_memory().await.expect("open in-memory"))
    }

    #[tokio::test]
    async fn schedule_crud_and_run_history_roundtrip() {
        let db = mem_db().await;
        insert(&db, "etl-app", "0 3 * * *", Some(r#"["run.sh"]"#), Some(600), Some("root"))
            .await
            .unwrap();
        let all = list_all(&db).await.unwrap();
        assert_eq!(all.len(), 1);
        let s = &all[0];
        assert!(s.enabled);
        assert_eq!(s.cmd_override(), Some(vec!["run.sh".to_string()]));
        assert_eq!(s.timeout_secs, Some(600));
        assert!(s.last_run_at.is_none());

        // The fire claim is atomic on the previous value: the first
        // caller wins, a concurrent second (same previous) loses.
        let now = Utc::now();
        assert!(mark_fired(&db, s.id, None, now).await.unwrap());
        assert!(!mark_fired(&db, s.id, None, now).await.unwrap(), "stale claim loses");

        record_run(&db, s.id, now, RunStatus::Failed, Some(3), "boom", Some(1200))
            .await
            .unwrap();
        set_enabled(&db, s.id, false, Some("root")).await.unwrap();
        assert!(!list_all(&db).await.unwrap()[0].enabled);

        delete(&db, s.id, Some("root")).await.unwrap();
        assert!(list_all(&db).await.unwrap().is_empty());
    }

    /// #986 slice C: `recent_runs` joins the spec back in, orders
    /// newest-first, and honours the limit.
    #[tokio::test]
    async fn recent_runs_joins_spec_and_orders_newest_first() {
        let db = mem_db().await;
        insert(&db, "etl-a", "0 3 * * *", None, None, None).await.unwrap();
        insert(&db, "etl-b", "0 4 * * *", None, None, None).await.unwrap();
        let all = list_all(&db).await.unwrap();
        let (a, b) = (all[0].id, all[1].id);
        assert_eq!(all[0].spec_id, "etl-a");

        let t0: DateTime<Utc> = "2026-07-13T03:00:00Z".parse().unwrap();
        let t1: DateTime<Utc> = "2026-07-13T04:00:00Z".parse().unwrap();
        record_run(&db, a, t0, RunStatus::Ok, Some(0), "done", Some(1500)).await.unwrap();
        record_run(&db, b, t1, RunStatus::Failed, Some(2), "boom", Some(80)).await.unwrap();

        let runs = recent_runs(&db, 20).await.unwrap();
        assert_eq!(runs.len(), 2);
        // Newest first, each carrying its schedule's spec_id.
        assert_eq!(runs[0].spec_id, "etl-b");
        assert_eq!(runs[0].status, "failed");
        assert_eq!(runs[0].exit_code, Some(2));
        assert_eq!(runs[0].log_tail.as_deref(), Some("boom"));
        assert_eq!(runs[0].duration_ms, Some(80));
        assert_eq!(runs[1].spec_id, "etl-a");
        assert_eq!(runs[1].status, "ok");

        // The limit bounds the page.
        assert_eq!(recent_runs(&db, 1).await.unwrap().len(), 1);
    }

    // Dual-dialect check (the `IS NOT DISTINCT FROM` claim + BOOLEAN
    // columns). Gated on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn schedules_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pg = crate::db::open_pg(&url).await.unwrap();
        sqlx::query("DELETE FROM schedule_runs").execute(&pg).await.unwrap();
        sqlx::query("DELETE FROM schedules").execute(&pg).await.unwrap();
        let db = ConfigDb::Postgres(pg);

        insert(&db, "etl-app", "*/5 * * * *", None, None, None).await.unwrap();
        let s = list_all(&db).await.unwrap().remove(0);
        let now = Utc::now();
        assert!(mark_fired(&db, s.id, None, now).await.unwrap());
        assert!(!mark_fired(&db, s.id, None, now).await.unwrap());
        record_run(&db, s.id, now, RunStatus::Ok, Some(0), "", Some(10)).await.unwrap();
        // The $n-placeholder JOIN query parses under real Postgres too.
        let runs = recent_runs(&db, 5).await.unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].spec_id, "etl-app");
        delete(&db, s.id, None).await.unwrap();
    }
}
