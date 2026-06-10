-- Postgres twin of migrations/0018. Default portal theme for a
-- cookieless visitor: 'light' | 'dark' | 'auto'. NULL/'auto' → OS
-- prefers-color-scheme. Idempotent.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS default_theme TEXT;
