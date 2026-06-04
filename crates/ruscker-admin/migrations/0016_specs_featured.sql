-- Persist `featured` as a column (#588). It mirrors config_json's
-- `featured` flag — a derived cache refreshed on every upsert (every write
-- path goes through upsert_in_tx) — so the Apps list no longer
-- deserializes every spec's config_json just to know which cards are
-- featured. Backfill from the existing JSON.
ALTER TABLE specs ADD COLUMN featured INTEGER NOT NULL DEFAULT 0;
UPDATE specs SET featured = COALESCE(json_extract(config_json, '$.featured'), 0);
