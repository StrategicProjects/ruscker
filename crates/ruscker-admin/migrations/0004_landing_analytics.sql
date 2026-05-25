-- Analytics snippet + the CSP origins it needs, on the
-- landing_customization singleton (id = 1). Both optional/NULL.
ALTER TABLE landing_customization ADD COLUMN analytics_html    TEXT;
ALTER TABLE landing_customization ADD COLUMN analytics_origins TEXT;
