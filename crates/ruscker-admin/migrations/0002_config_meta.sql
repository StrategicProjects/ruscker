-- Catch-all for the YAML sections that don't have dedicated
-- tables yet: top-level `server`, `logging`, and the rest of the
-- `proxy` block (everything except `specs` and
-- `landing-customization`, which live in their own tables).
--
-- Stored as JSON blobs keyed by section name. When a Phase 2+ UI
-- needs to surface a specific field, we extract it to a column
-- and migrate the data out of here. Until then, the importer and
-- exporter just round-trip the JSON unchanged so YAML edits aren't
-- lost on import → export → reimport.

CREATE TABLE config_meta (
    -- 'proxy' | 'server' | 'logging'
    key         TEXT PRIMARY KEY,
    value_json  TEXT NOT NULL,
    updated_at  TIMESTAMP NOT NULL
);
