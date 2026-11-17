-- Which optional scopes an SSO client may be granted — Postgres twin.
ALTER TABLE oidc_clients ADD COLUMN IF NOT EXISTS allowed_scopes TEXT;
