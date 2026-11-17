-- A logo for an SSO client (#1080): shown on the sign-in page when the
-- user arrives from that system, so they can see where they are going.
-- A Media-library filename, or NULL.
ALTER TABLE oidc_clients ADD COLUMN logo TEXT;
