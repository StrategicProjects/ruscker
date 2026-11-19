-- Whether the second factor was proved for this code — Postgres twin.
ALTER TABLE oidc_auth_codes ADD COLUMN IF NOT EXISTS mfa_proved BOOLEAN NOT NULL DEFAULT FALSE;
