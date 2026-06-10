-- Postgres twin of migrations/0017. Editable public-portal footer text
-- (appearance editor). NULL → the built-in version + "ruscker" lockup
-- renders unchanged. Idempotent.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS footer TEXT;
