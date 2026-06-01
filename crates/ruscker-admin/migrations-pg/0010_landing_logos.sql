-- Header/footer logos — Postgres twin of migrations/0010, which was
-- missing (so landing logos didn't persist on Postgres). Stored as a
-- JSON array of {url, slot, align, link, height}. Idempotent.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS logos_json TEXT;
