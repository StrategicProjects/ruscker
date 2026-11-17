-- OIDC provider, slice 1: the registry of client applications allowed to
-- sign users in through Ruscker ("Sign in with Ruscker"). See
-- docs/adr/0005-oidc-provider.md.
--
-- A client is a separate system (not a hosted app): it sends the user to
-- Ruscker's login and receives an identity back. Only what is registered
-- here can ever receive one.
CREATE TABLE oidc_clients (
    -- Public identifier the client sends on every request. Slug charset,
    -- it lands in admin URLs un-encoded (same rule as credential names).
    client_id     TEXT    PRIMARY KEY NOT NULL,
    -- Human label shown to the user on the login page and to admins.
    name          TEXT    NOT NULL,
    -- Salted SHA-256 of the client secret (`salt_hex:hash_hex`). The
    -- secret is random and high-entropy, shown ONCE at creation/rotation
    -- and never stored — a slow password hash would add nothing here.
    secret_hash   TEXT    NOT NULL,
    -- Exact-match redirect URIs, one per line. No wildcards, ever.
    redirect_uris TEXT    NOT NULL,
    -- Comma-separated group names, same canonical form as `users.groups`.
    -- Empty = any signed-in user may use this client.
    access_groups TEXT    NOT NULL DEFAULT '',
    -- When 1 the user must have proved a second factor to sign in here.
    require_mfa   INTEGER NOT NULL DEFAULT 0,
    -- A disabled client is refused at the authorization endpoint.
    enabled       INTEGER NOT NULL DEFAULT 1,
    -- RFC 3339 (UTC); TEXT in SQLite, TIMESTAMPTZ in Postgres.
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL
);
