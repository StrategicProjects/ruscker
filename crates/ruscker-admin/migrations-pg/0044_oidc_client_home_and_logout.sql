-- Two addresses per SSO client — Postgres twin of the SQLite migration.
ALTER TABLE oidc_clients ADD COLUMN IF NOT EXISTS client_uri TEXT;
ALTER TABLE oidc_clients ADD COLUMN IF NOT EXISTS post_logout_redirect_uris TEXT NOT NULL DEFAULT '';
