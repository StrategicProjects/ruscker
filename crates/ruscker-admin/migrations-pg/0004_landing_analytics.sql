-- Analytics snippet + its CSP origins on the landing_customization
-- singleton. Postgres twin of migrations/0004. Both optional/NULL.
ALTER TABLE landing_customization ADD COLUMN analytics_html    TEXT;
ALTER TABLE landing_customization ADD COLUMN analytics_origins TEXT;
