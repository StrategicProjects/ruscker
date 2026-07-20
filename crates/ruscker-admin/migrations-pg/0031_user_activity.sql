-- Per-user activity log (#1021) — Postgres twin of the SQLite migration.
-- One row per login or interactive app access, WITH identity. Distinct
-- from `spec_access` (aggregate totals) and `audit_log` (admin actions).
-- No foreign keys to `users`/`specs` so history survives deletion.
CREATE TABLE user_activity (
    id           BIGINT      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    event_type   TEXT        NOT NULL,
    username     TEXT,
    spec_id      TEXT,
    event_key    TEXT        NOT NULL,
    auth_method  TEXT,
    client_ip    TEXT,
    occurred_at  TIMESTAMPTZ NOT NULL
);

CREATE UNIQUE INDEX user_activity_dedup_idx ON user_activity (event_type, event_key);
CREATE INDEX user_activity_occurred_idx ON user_activity (occurred_at);
CREATE INDEX user_activity_username_idx ON user_activity (username);
CREATE INDEX user_activity_spec_idx     ON user_activity (spec_id);
CREATE INDEX user_activity_type_idx     ON user_activity (event_type);
