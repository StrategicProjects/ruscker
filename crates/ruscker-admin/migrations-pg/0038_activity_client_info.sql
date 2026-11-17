-- Audit detail for sign-ins (#1079) — Postgres twin of the SQLite
-- migration: browser and failure reason on `user_activity`, and the
-- user's address and browser carried by an authorization code.
ALTER TABLE user_activity ADD COLUMN IF NOT EXISTS user_agent TEXT;
ALTER TABLE user_activity ADD COLUMN IF NOT EXISTS detail TEXT;
ALTER TABLE oidc_auth_codes ADD COLUMN IF NOT EXISTS client_ip TEXT;
ALTER TABLE oidc_auth_codes ADD COLUMN IF NOT EXISTS user_agent TEXT;
