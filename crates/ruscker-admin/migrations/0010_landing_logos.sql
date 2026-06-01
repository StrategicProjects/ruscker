-- Header/footer logos for the public landing (admin-managed).
-- Stored as a JSON array of {url, slot, align, link, height}; one row
-- (the singleton landing_customization). Idempotent ADD COLUMN.
ALTER TABLE landing_customization ADD COLUMN logos_json TEXT;
