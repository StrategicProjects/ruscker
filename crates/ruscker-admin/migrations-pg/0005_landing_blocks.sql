-- Custom HTML blocks rendered in fixed landing slots ('top' / 'bottom').
-- Postgres twin of migrations/0005. `position` orders within a slot;
-- `csp_origins` widens the landing CSP.
--
-- Note the dialect shift from the SQLite original: `enabled` is a real
-- BOOLEAN here (the SQLite table stores it as INTEGER 0/1). The query
-- port (Phase 7c-2) binds/reads it as `bool` directly instead of the
-- SQLite-side `enabled as i64`.
CREATE TABLE landing_blocks (
    id          TEXT        PRIMARY KEY,
    slot        TEXT        NOT NULL,
    position    BIGINT      NOT NULL DEFAULT 0,
    enabled     BOOLEAN     NOT NULL DEFAULT TRUE,
    title       TEXT        NOT NULL DEFAULT '',
    html        TEXT        NOT NULL DEFAULT '',
    csp_origins TEXT        NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_landing_blocks_slot_pos ON landing_blocks (slot, position);
