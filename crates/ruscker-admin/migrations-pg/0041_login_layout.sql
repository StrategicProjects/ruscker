-- Sign-in page layout (#1083) — Postgres twin.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS login_layout TEXT;
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS login_image TEXT;
