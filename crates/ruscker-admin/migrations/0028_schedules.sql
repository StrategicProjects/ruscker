-- Scheduled jobs (#986 slice B): cron-triggered run-to-completion
-- executions of a spec's image (ETL, reports). `cmd_json` is an
-- optional argv override (JSON array) — NULL runs the spec's own
-- command. `last_run_at` is the scheduler's fire marker (set BEFORE
-- the job runs, so a crash mid-job never double-fires).
CREATE TABLE schedules (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    spec_id       TEXT NOT NULL,
    cron          TEXT NOT NULL,
    cmd_json      TEXT,
    enabled       INTEGER NOT NULL DEFAULT 1,
    timeout_secs  INTEGER,
    last_run_at   TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

-- Run history. `status`: 'ok' (exit 0) | 'failed' (non-zero exit) |
-- 'error' (could not run: pull/create/start/timeout).
CREATE TABLE schedule_runs (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    schedule_id  INTEGER NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    started_at   TEXT NOT NULL,
    finished_at  TEXT,
    status       TEXT NOT NULL,
    exit_code    INTEGER,
    log_tail     TEXT,
    duration_ms  INTEGER
);
CREATE INDEX idx_schedule_runs_schedule ON schedule_runs(schedule_id, started_at DESC);
