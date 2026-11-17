-- What the `sections` catalog layout groups by (#1066): 'type' (one
-- section per app type, the pre-#1066 behaviour) or 'subject' (one
-- section per template-properties.subject + a final "Other"). NULL ⇒
-- 'type', so existing portals render exactly as before.
ALTER TABLE landing_customization ADD COLUMN catalog_sections_by TEXT;
