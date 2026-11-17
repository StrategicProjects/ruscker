-- Postgres twin of migrations/0037 (#1065): intro style / alignment /
-- banner background filename. NULL ⇒ defaults. Idempotent.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS intro_style TEXT;
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS intro_align TEXT;
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS intro_image TEXT;
