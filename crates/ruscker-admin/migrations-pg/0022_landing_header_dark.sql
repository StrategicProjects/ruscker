-- Postgres twin of migrations/0022 (#784). Dark-theme header colours;
-- NULL → inherit the light values. Idempotent.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS header_bg_dark TEXT;
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS header_fg_dark TEXT;
