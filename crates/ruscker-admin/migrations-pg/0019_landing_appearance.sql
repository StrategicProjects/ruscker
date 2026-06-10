-- Postgres twin of migrations/0019 (design handoff ruscker-06). All
-- nullable; NULL → the built-in default for each. BOOLEAN where the
-- sqlite twin used INTEGER-as-bool; BIGINT where the row struct reads
-- i64. Idempotent.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS show_search BOOLEAN;        -- default on
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS show_filters BOOLEAN;       -- default on
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS logo_mode TEXT;             -- mark | symbol | custom
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS logo_size BIGINT;           -- px, default 28
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS logo_margin BIGINT;         -- px, default 8
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS header_preset TEXT;         -- flat | soft | bold
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS card_cover TEXT;            -- tinted | gradient
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS catalog_layout TEXT;        -- grid | list | sections
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS catalog_density TEXT;       -- comfortable | compact
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS analytics_provider TEXT;    -- none | ga | plausible | matomo
