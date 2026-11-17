-- Which optional scopes an SSO client may be granted (#1068). A
-- space-separated subset of: profile email groups roles setor.
-- NULL = unrestricted (every client registered before this column).
ALTER TABLE oidc_clients ADD COLUMN allowed_scopes TEXT;
