-- Postgres twin of migrations/0020. Site key for the analytics provider
-- picker: a GA4 measurement id, a Plausible domain, or a Matomo
-- "url|siteId". NULL → no provider snippet. Idempotent.
ALTER TABLE landing_customization ADD COLUMN IF NOT EXISTS analytics_key TEXT;
