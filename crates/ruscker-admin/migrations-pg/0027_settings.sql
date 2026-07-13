-- Postgres twin of migrations/0027 (#930). Generic key/value store for
-- small operator settings (first user: the alert webhook URL, key
-- `alert.webhook-url`).
CREATE TABLE settings (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL
);
