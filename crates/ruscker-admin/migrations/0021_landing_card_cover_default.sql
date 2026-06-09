-- Default card-cover style for the public portal, set in the appearance
-- editor's "Background" section (#720). A CSS value — a solid colour or a
-- gradient — applied as the cover of every card that has no per-app
-- `cover`/`accent` of its own. NULL → keep the per-kind tint (the prior
-- behaviour), which is the editor's "Auto" mode.
ALTER TABLE landing_customization ADD COLUMN card_cover_default TEXT;
