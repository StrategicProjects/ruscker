-- SEO / social-share meta tags on the landing_customization singleton.
-- Postgres twin of migrations/0003. All optional; NULL means fall back.
ALTER TABLE landing_customization ADD COLUMN seo_title       TEXT;
ALTER TABLE landing_customization ADD COLUMN seo_description TEXT;
ALTER TABLE landing_customization ADD COLUMN og_image        TEXT;
