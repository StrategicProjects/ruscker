-- Postgres twin of migrations/0013 (#506). Idempotent.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS show_highlights BOOLEAN;
