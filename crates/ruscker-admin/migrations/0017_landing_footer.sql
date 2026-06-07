-- Editable public-portal footer text (appearance editor). NULL → the
-- built-in version + "ruscker" lockup renders unchanged.
ALTER TABLE landing_customization ADD COLUMN footer TEXT;
