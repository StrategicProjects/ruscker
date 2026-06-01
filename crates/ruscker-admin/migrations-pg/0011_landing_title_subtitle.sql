-- Postgres twin of migrations/0011 (#468). Idempotent.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS title TEXT;
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS subtitle TEXT;
