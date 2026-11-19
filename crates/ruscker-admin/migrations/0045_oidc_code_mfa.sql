-- Whether the second factor was proved for this authorization code
-- (security review 2026-11-19). Redemption refuses a code issued without
-- it when the client requires MFA by then. Codes in flight at upgrade
-- time count as "not proved": at worst a 60-second code is refused.
ALTER TABLE oidc_auth_codes ADD COLUMN mfa_proved INTEGER NOT NULL DEFAULT 0;
