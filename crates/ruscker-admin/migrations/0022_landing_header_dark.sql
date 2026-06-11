-- Per-theme header colours (#784). The single `header_bg`/`header_fg`
-- applied identically to the light and dark themes — there was no way
-- to choose a different gradient (or colour) per theme. These columns
-- hold the DARK theme's values; NULL → inherit the light ones (the
-- prior behaviour, so existing setups render unchanged).
ALTER TABLE landing_customization ADD COLUMN header_bg_dark TEXT;
ALTER TABLE landing_customization ADD COLUMN header_fg_dark TEXT;
