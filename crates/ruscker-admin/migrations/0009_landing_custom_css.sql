-- Operator custom CSS on the landing_customization singleton (#232).
-- Injected as a <style> in the landing <head>. Optional/NULL.
ALTER TABLE landing_customization ADD COLUMN custom_css TEXT;
