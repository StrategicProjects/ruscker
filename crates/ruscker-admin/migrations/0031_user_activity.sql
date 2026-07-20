-- Per-user activity log (#1021): one row per login or interactive app
-- access, WITH identity. Distinct from `spec_access` (aggregate totals,
-- no user) and `audit_log` (administrative actions, different growth
-- rate). No foreign keys to `users`/`specs` so the history survives a
-- user or app being deleted. Written off the proxy hot path by a
-- supervised, bounded writer (`crate::activity`).
CREATE TABLE user_activity (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    -- 'login.success' | 'app.access'
    event_type   TEXT    NOT NULL,
    -- NULL for an anonymous access (no signed-in user).
    username     TEXT,
    -- Set for 'app.access'; NULL for 'login.success'.
    spec_id      TEXT,
    -- Session identifier — dedups the same event across HA instances.
    event_key    TEXT    NOT NULL,
    -- 'password' | 'token' (break-glass). NULL when not applicable.
    auth_method  TEXT,
    -- Optional; personal data — admin-only, define retention if used.
    client_ip    TEXT,
    -- RFC 3339 (UTC); TEXT in SQLite, TIMESTAMPTZ in Postgres.
    occurred_at  TEXT    NOT NULL
);

-- Dedup the same session's event (HA: two instances, one session).
CREATE UNIQUE INDEX user_activity_dedup_idx ON user_activity (event_type, event_key);
-- Filters + newest-first paging.
CREATE INDEX user_activity_occurred_idx ON user_activity (occurred_at);
CREATE INDEX user_activity_username_idx ON user_activity (username);
CREATE INDEX user_activity_spec_idx     ON user_activity (spec_id);
CREATE INDEX user_activity_type_idx     ON user_activity (event_type);
