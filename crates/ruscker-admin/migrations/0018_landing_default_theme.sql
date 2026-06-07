-- Default portal theme for a cookieless visitor (appearance editor):
-- 'light' | 'dark' | 'auto'. NULL/'auto' → OS prefers-color-scheme.
ALTER TABLE landing_customization ADD COLUMN default_theme TEXT;
