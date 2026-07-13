-- Generic key/value store for small operator settings that don't
-- deserve their own table (#930 — first user: the alert webhook URL,
-- key `alert.webhook-url`). One row per key; values are plain text
-- (callers own any encoding). Distinct from `landing_customization`,
-- which is one wide row for the landing editor.
CREATE TABLE settings (
    key         TEXT PRIMARY KEY,
    value       TEXT NOT NULL,
    updated_at  TEXT NOT NULL
);
