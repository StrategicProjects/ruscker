-- Per-theme color overrides (light/dark) on the landing_customization
-- singleton (#475). Stored as a JSON object {light:{bg,text,accent},
-- dark:{...}}. Optional/NULL → built-in theme defaults.
ALTER TABLE landing_customization ADD COLUMN theme_colors_json TEXT;
