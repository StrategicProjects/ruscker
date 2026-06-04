-- Postgres twin of migrations/0016 (#588). Derived cache of config_json's
-- `featured` flag; backfilled from the existing JSON.
ALTER TABLE specs ADD COLUMN IF NOT EXISTS featured BOOLEAN NOT NULL DEFAULT FALSE;
UPDATE specs SET featured = COALESCE((config_json::jsonb ->> 'featured')::boolean, FALSE);
