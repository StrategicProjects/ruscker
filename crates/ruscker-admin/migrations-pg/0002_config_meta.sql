-- Catch-all for YAML sections without dedicated tables (top-level
-- `server`, `logging`, and the rest of `proxy` minus `specs` and
-- `landing-customization`). Postgres twin of migrations/0002.
CREATE TABLE config_meta (
    -- 'proxy' | 'server' | 'logging'
    key         TEXT        PRIMARY KEY,
    value_json  TEXT        NOT NULL,
    updated_at  TIMESTAMPTZ NOT NULL
);
