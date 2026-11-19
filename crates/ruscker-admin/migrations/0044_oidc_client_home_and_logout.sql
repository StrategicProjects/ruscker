-- Two addresses per SSO client (#1068 follow-up, first real client).
-- `client_uri`: the system's own home page. The portal links people back
-- to it (e.g. after changing their password) — taken from the registry,
-- never from a query string, so it cannot be turned into an open redirect.
-- `post_logout_redirect_uris`: where the portal may send a browser after
-- signing it out on the system's request (OpenID Connect RP-Initiated
-- Logout), one per line, matched exactly like `redirect_uris`.
ALTER TABLE oidc_clients ADD COLUMN client_uri TEXT;
ALTER TABLE oidc_clients ADD COLUMN post_logout_redirect_uris TEXT NOT NULL DEFAULT '';
