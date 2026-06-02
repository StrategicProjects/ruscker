-- Show the "Featured" carousel of highlighted apps above the filters
-- (#506). NULL → default on; the carousel still only renders when at
-- least one spec is featured.
ALTER TABLE landing_customization ADD COLUMN show_highlights INTEGER;
