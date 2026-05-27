-- Initial schema for the Ruscker admin DB — **Postgres dialect**.
--
-- Mirrors `migrations/0001_initial.sql` (SQLite) one-for-one; this
-- directory is the HA / multi-instance store (Phase 7). Keep the two
-- in lockstep: every SQLite migration gets a Postgres twin with the
-- same number so the shared admin catalog stays identical whichever
-- backend an install runs.
--
-- Dialect mapping vs. the SQLite original:
--   * INTEGER counters / sizes (read as i64 in Rust) -> BIGINT
--   * INTEGER 0/1 booleans                            -> BOOLEAN
--   * BLOB                                            -> BYTEA
--   * TIMESTAMP / TEXT timestamps (DateTime<Utc>)     -> TIMESTAMPTZ
--   * INTEGER PRIMARY KEY AUTOINCREMENT               -> BIGINT GENERATED
--                                                        ALWAYS AS IDENTITY
-- The application remains the single source of "now()": no DB-level
-- DEFAULTs on write-path timestamps (the singleton seed below is the
-- one exception, and it's overwritten on first save).

CREATE TABLE specs (
    id              TEXT        PRIMARY KEY,
    display_name    TEXT,
    description     TEXT,
    -- 'shiny' | 'interactive' | 'api' | 'external' (DisplayType / SpecKind)
    kind            TEXT        NOT NULL,
    container_image TEXT,
    -- Full Spec serialized as JSON for round-trip fidelity.
    config_json     TEXT        NOT NULL,
    -- 'active' | 'inactive'.
    state           TEXT        NOT NULL DEFAULT 'active',
    created_at      TIMESTAMPTZ NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL,
    -- Monotonic counter bumped on every UPDATE.
    version         BIGINT      NOT NULL DEFAULT 1
);

CREATE INDEX specs_state_idx ON specs (state);
CREATE INDEX specs_kind_idx ON specs (kind);

CREATE TABLE spec_versions (
    spec_id     TEXT        NOT NULL REFERENCES specs(id) ON DELETE CASCADE,
    version     BIGINT      NOT NULL,
    config_json TEXT        NOT NULL,
    changed_at  TIMESTAMPTZ NOT NULL,
    changed_by  TEXT,
    PRIMARY KEY (spec_id, version)
);

CREATE TABLE images (
    id           TEXT        PRIMARY KEY,
    filename     TEXT        NOT NULL,
    mime_type    TEXT        NOT NULL,
    size_bytes   BIGINT      NOT NULL,
    blob         BYTEA       NOT NULL,
    width        BIGINT,
    height       BIGINT,
    uploaded_at  TIMESTAMPTZ NOT NULL
);

CREATE INDEX images_filename_idx ON images (filename);

CREATE TABLE credentials (
    name         TEXT        PRIMARY KEY,
    registry     TEXT        NOT NULL,
    username     TEXT        NOT NULL,
    password_enc BYTEA       NOT NULL,
    nonce        BYTEA       NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL
);

CREATE TABLE landing_customization (
    id           INTEGER     PRIMARY KEY CHECK (id = 1), -- singleton row
    header_bg    TEXT,
    header_fg    TEXT,
    intro        TEXT,
    intro_locales_json TEXT  NOT NULL DEFAULT '{}',
    updated_at   TIMESTAMPTZ NOT NULL
);

INSERT INTO landing_customization (id, intro_locales_json, updated_at)
VALUES (1, '{}', CURRENT_TIMESTAMP);

CREATE TABLE audit_log (
    id          BIGINT      GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    actor       TEXT,
    action      TEXT        NOT NULL,
    target      TEXT,
    diff_json   TEXT,
    occurred_at TIMESTAMPTZ NOT NULL
);

CREATE INDEX audit_log_target_idx ON audit_log (target);
CREATE INDEX audit_log_action_idx ON audit_log (action);
