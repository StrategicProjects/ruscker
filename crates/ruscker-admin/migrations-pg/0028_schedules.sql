-- Postgres twin of migrations/0028 (#986 slice B).
CREATE TABLE schedules (
    id            BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    spec_id       TEXT NOT NULL,
    cron          TEXT NOT NULL,
    cmd_json      TEXT,
    enabled       BOOLEAN NOT NULL DEFAULT TRUE,
    timeout_secs  BIGINT,
    last_run_at   TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL,
    updated_at    TIMESTAMPTZ NOT NULL
);

CREATE TABLE schedule_runs (
    id           BIGINT GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    schedule_id  BIGINT NOT NULL REFERENCES schedules(id) ON DELETE CASCADE,
    started_at   TIMESTAMPTZ NOT NULL,
    finished_at  TIMESTAMPTZ,
    status       TEXT NOT NULL,
    exit_code    BIGINT,
    log_tail     TEXT,
    duration_ms  BIGINT
);
CREATE INDEX idx_schedule_runs_schedule ON schedule_runs(schedule_id, started_at DESC);
