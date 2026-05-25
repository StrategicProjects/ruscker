-- Custom HTML blocks rendered in fixed landing slots ('top' after the
-- header, 'bottom' after the card grid). Admin-managed (DB-only for
-- now; not part of the YAML import/export round-trip yet). `position`
-- orders blocks within a slot; `csp_origins` widens the landing CSP so
-- embedded third-party content can load.
CREATE TABLE landing_blocks (
    id          TEXT    PRIMARY KEY,
    slot        TEXT    NOT NULL,
    position    INTEGER NOT NULL DEFAULT 0,
    enabled     INTEGER NOT NULL DEFAULT 1,
    title       TEXT    NOT NULL DEFAULT '',
    html        TEXT    NOT NULL DEFAULT '',
    csp_origins TEXT    NOT NULL DEFAULT '',
    created_at  TEXT    NOT NULL,
    updated_at  TEXT    NOT NULL
);
CREATE INDEX idx_landing_blocks_slot_pos ON landing_blocks (slot, position);
