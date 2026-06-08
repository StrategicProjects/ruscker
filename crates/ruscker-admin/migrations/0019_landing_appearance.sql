-- Appearance-editor fields (design handoff ruscker-06). All nullable;
-- NULL → the built-in default for each.
ALTER TABLE landing_customization ADD COLUMN show_search INTEGER;       -- bool, default on
ALTER TABLE landing_customization ADD COLUMN show_filters INTEGER;      -- bool, default on
ALTER TABLE landing_customization ADD COLUMN logo_mode TEXT;            -- mark | symbol | custom
ALTER TABLE landing_customization ADD COLUMN logo_size INTEGER;         -- px, default 28
ALTER TABLE landing_customization ADD COLUMN logo_margin INTEGER;       -- px, default 8
ALTER TABLE landing_customization ADD COLUMN header_preset TEXT;        -- flat | soft | bold
ALTER TABLE landing_customization ADD COLUMN card_cover TEXT;           -- tinted | gradient
ALTER TABLE landing_customization ADD COLUMN catalog_layout TEXT;       -- grid | list | sections
ALTER TABLE landing_customization ADD COLUMN catalog_density TEXT;      -- comfortable | compact
ALTER TABLE landing_customization ADD COLUMN analytics_provider TEXT;   -- none | ga | plausible | matomo
