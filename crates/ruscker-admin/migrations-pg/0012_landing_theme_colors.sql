-- Postgres twin of migrations/0012 (#475). Idempotent.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS theme_colors_json TEXT;
