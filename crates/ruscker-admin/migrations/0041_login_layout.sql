-- Sign-in page layout (#1083): card (default, NULL) | split | minimal,
-- and the side image of the split layout (a Media-library filename).
ALTER TABLE landing_customization ADD COLUMN login_layout TEXT;
ALTER TABLE landing_customization ADD COLUMN login_image TEXT;
