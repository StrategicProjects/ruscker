-- Native intro styles (#1065). intro_style: 'text' | 'banner' | 'card'
-- (NULL ⇒ 'text', the pre-#1065 paragraph); intro_align: 'left' |
-- 'center' (NULL ⇒ the style's default: banner centred, otherwise left);
-- intro_image: Media-library filename for the banner background (NULL ⇒
-- none).
ALTER TABLE landing_customization ADD COLUMN intro_style TEXT;
ALTER TABLE landing_customization ADD COLUMN intro_align TEXT;
ALTER TABLE landing_customization ADD COLUMN intro_image TEXT;
