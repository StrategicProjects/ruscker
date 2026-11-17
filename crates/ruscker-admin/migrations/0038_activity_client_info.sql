-- Audit detail for sign-ins (#1079): the browser a sign-in came from and,
-- for failures and refusals, why.
--
-- `client_ip` already existed on `user_activity` but was never filled.
-- IP address and user agent are personal data: shown to Admins only and
-- purged with the rest of the row after the retention period
-- (`server.activity-retention-days`).
ALTER TABLE user_activity ADD COLUMN user_agent TEXT;
-- Machine-readable reason for 'login.failed' / 'sso.denied'
-- (e.g. 'wrong-password', 'not-in-group'). NULL otherwise.
ALTER TABLE user_activity ADD COLUMN detail TEXT;

-- An SSO sign-in completes in two steps: the USER's browser obtains a
-- code, then the client system's SERVER redeems it. The audit row is
-- written at redemption, but the address and browser that matter are
-- the user's — so they ride along with the code.
ALTER TABLE oidc_auth_codes ADD COLUMN client_ip TEXT;
ALTER TABLE oidc_auth_codes ADD COLUMN user_agent TEXT;
