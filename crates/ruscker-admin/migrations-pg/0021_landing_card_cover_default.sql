-- Postgres twin of migrations/0021 (#720). Default card-cover CSS value
-- (solid colour or gradient) applied to every card without its own
-- cover/accent. NULL → per-kind tint (the editor's "Auto"). Idempotent.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS card_cover_default TEXT;
