-- Operator custom CSS on the landing_customization singleton (#232).
-- Postgres twin of migrations/0009. Optional/NULL.
ALTER TABLE landing_customization ADD COLUMN custom_css TEXT;
