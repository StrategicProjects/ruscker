-- SEO / social-share meta tags for the public landing.
-- Added to the landing_customization singleton (id = 1). All optional;
-- NULL means "fall back" (title -> proxy.title, description -> intro,
-- image -> omitted).
ALTER TABLE landing_customization ADD COLUMN seo_title       TEXT;
ALTER TABLE landing_customization ADD COLUMN seo_description TEXT;
ALTER TABLE landing_customization ADD COLUMN og_image        TEXT;
