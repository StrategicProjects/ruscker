-- A logo for an SSO client (#1080) — Postgres twin: a Media-library
-- filename shown on the sign-in page, or NULL.
ALTER TABLE oidc_clients ADD COLUMN IF NOT EXISTS logo TEXT;
