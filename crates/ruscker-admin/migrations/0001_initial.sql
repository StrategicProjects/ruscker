-- Initial schema for Ruscker admin DB.
--
-- See docs/adr/0002-sqlite-source-of-truth.md and
-- crates/ruscker-admin/CLAUDE.md for the rationale. Short version:
-- SQLite holds the live spec catalog, images, credentials and
-- audit log. YAML stays as an import/export format.
--
-- Conventions:
--   * Timestamps: TIMESTAMP (sqlx maps to chrono::DateTime).
--     All write paths set them explicitly — no DEFAULT triggers,
--     so the application is the single source of "now()".
--   * JSON columns: TEXT NOT NULL. Validated by serde at write
--     time; reads parse with `serde_json::from_str`.
--   * `state` enums: TEXT NOT NULL, value space documented per
--     column. SQLite has no native enums; values are checked in
--     the application layer.
--   * Foreign keys are enabled at connection time via
--     `PRAGMA foreign_keys = ON`.

CREATE TABLE specs (
    id              TEXT    PRIMARY KEY,
    display_name    TEXT,
    description     TEXT,
    -- 'shiny' | 'interactive' | 'api' | 'external' (DisplayType / SpecKind)
    kind            TEXT    NOT NULL,
    container_image TEXT,
    -- The full Spec serialized as JSON for round-trip fidelity.
    -- Schema-known fields live in their own columns above so we
    -- can index/query them without parsing JSON; this column is
    -- the authoritative record for everything else.
    config_json     TEXT    NOT NULL,
    -- 'active' | 'inactive'. Inactive specs are hidden from the
    -- public landing by default.
    state           TEXT    NOT NULL DEFAULT 'active',
    created_at      TIMESTAMP NOT NULL,
    updated_at      TIMESTAMP NOT NULL,
    -- Monotonic counter incremented on every UPDATE. spec_versions
    -- carries the historical JSON for each value of `version`.
    version         INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX specs_state_idx ON specs (state);
CREATE INDEX specs_kind_idx ON specs (kind);

-- History table — one row per (spec, version) capturing the JSON
-- that was current at that version. Lets the admin show "undo
-- last change" and the audit log link to actual diffs.
CREATE TABLE spec_versions (
    spec_id     TEXT    NOT NULL REFERENCES specs(id) ON DELETE CASCADE,
    version     INTEGER NOT NULL,
    config_json TEXT    NOT NULL,
    changed_at  TIMESTAMP NOT NULL,
    -- Who made the change. NULL for imports (no logged-in user).
    changed_by  TEXT,
    PRIMARY KEY (spec_id, version)
);

-- Operator-uploaded card images. Phase 2 stores blobs in SQLite
-- (~MB-sized files, hundreds of them) — simpler ops than a
-- separate filesystem path. Switching to object storage is a
-- migration when individual installs cross 1 GB.
CREATE TABLE images (
    id           TEXT    PRIMARY KEY,
    -- Human filename for the admin UI (uploads/auroraprime.png).
    filename     TEXT    NOT NULL,
    -- image/webp, image/png, image/svg+xml.
    mime_type    TEXT    NOT NULL,
    size_bytes   INTEGER NOT NULL,
    blob         BLOB    NOT NULL,
    width        INTEGER,
    height       INTEGER,
    uploaded_at  TIMESTAMP NOT NULL
);

CREATE INDEX images_filename_idx ON images (filename);

-- Registry / API credentials. Password ciphertext is AES-GCM-
-- encrypted with a master key supplied via the RUSCKER_MASTER_KEY
-- env var. The DB never sees the plaintext.
CREATE TABLE credentials (
    name         TEXT    PRIMARY KEY,
    registry     TEXT    NOT NULL,
    username     TEXT    NOT NULL,
    password_enc BLOB    NOT NULL,
    -- Nonce used during AES-GCM encryption — required for decryption.
    nonce        BLOB    NOT NULL,
    created_at   TIMESTAMP NOT NULL
);

-- Visual customization of the public landing. Matches the
-- LandingCustomization YAML block 1:1 so import/export round-trip
-- works without schema gymnastics.
CREATE TABLE landing_customization (
    id           INTEGER PRIMARY KEY CHECK (id = 1), -- singleton row
    header_bg    TEXT,
    header_fg    TEXT,
    intro        TEXT,
    -- JSON object: { "pt": "...", "en": "...", ... }
    intro_locales_json TEXT NOT NULL DEFAULT '{}',
    updated_at   TIMESTAMP NOT NULL
);

INSERT INTO landing_customization (id, intro_locales_json, updated_at)
VALUES (1, '{}', CURRENT_TIMESTAMP);

-- Append-only history of operator actions. Every admin write
-- inserts here so we can answer "who changed what when".
CREATE TABLE audit_log (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    -- NULL for system events (imports, scheduled tasks).
    actor       TEXT,
    -- 'spec.create' | 'spec.update' | 'spec.delete' | 'image.upload' | …
    action      TEXT NOT NULL,
    -- Foreign-key-ish reference to the affected entity, e.g.
    -- 'spec:auroraprime', 'image:abc-123'. Not enforced as a FK
    -- because the target may have been deleted.
    target      TEXT,
    -- JSON diff (old/new). NULL for events without a diff.
    diff_json   TEXT,
    occurred_at TIMESTAMP NOT NULL
);

CREATE INDEX audit_log_target_idx ON audit_log (target);
CREATE INDEX audit_log_action_idx ON audit_log (action);
