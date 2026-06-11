-- Postgres twin of migrations/0023 (#790). Dark-theme default card
-- cover; NULL → inherit the light value. Idempotent.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS card_cover_default_dark TEXT;
