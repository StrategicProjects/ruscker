-- Dark-theme default card cover (#790). `card_cover_default` alone
-- applied identically to both themes; this column holds the DARK
-- theme's value. NULL → inherit the light one (the prior behaviour,
-- so existing setups render unchanged).
ALTER TABLE landing_customization ADD COLUMN card_cover_default_dark TEXT;
