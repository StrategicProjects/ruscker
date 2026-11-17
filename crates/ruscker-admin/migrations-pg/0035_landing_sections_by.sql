-- Postgres twin of migrations/0035 (#1066): grouping of the `sections`
-- catalog layout, 'type' | 'subject'. NULL ⇒ 'type'. Idempotent.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS catalog_sections_by TEXT;
