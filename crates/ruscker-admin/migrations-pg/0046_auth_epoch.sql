-- Authentication generation — Postgres twin of the SQLite migration.
ALTER TABLE users ADD COLUMN IF NOT EXISTS auth_epoch BIGINT NOT NULL DEFAULT 0;
ALTER TABLE oidc_auth_codes ADD COLUMN IF NOT EXISTS auth_epoch BIGINT NOT NULL DEFAULT 0;
