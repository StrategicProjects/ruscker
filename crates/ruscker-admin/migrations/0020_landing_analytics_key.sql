-- Site key for the appearance editor's analytics provider picker:
-- a GA4 measurement id, a Plausible domain, or a Matomo "url|siteId".
-- NULL → no provider snippet (raw analytics-html still applies).
ALTER TABLE landing_customization ADD COLUMN analytics_key TEXT;
