-- Operator-editable portal title + subtitle on the landing_customization
-- singleton (#468). Optional/NULL → fall back to proxy.title / i18n.
ALTER TABLE landing_customization ADD COLUMN title TEXT;
ALTER TABLE landing_customization ADD COLUMN subtitle TEXT;
