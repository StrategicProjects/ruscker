-- Authentication generation (security review 2026-11-19, second pass).
-- `users.auth_epoch` moves forward in the SAME transaction as every change
-- that must end the account's sessions (password, role). Each session and
-- each authorization code records the generation it was issued under, and
-- is refused once the account's has moved on — on every node, after a
-- restart, whatever happened to the session store's own DELETE.
ALTER TABLE users ADD COLUMN auth_epoch INTEGER NOT NULL DEFAULT 0;
ALTER TABLE oidc_auth_codes ADD COLUMN auth_epoch INTEGER NOT NULL DEFAULT 0;
