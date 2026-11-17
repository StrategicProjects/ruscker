-- OIDC provider, slice 3: authorization codes. See
-- docs/adr/0005-oidc-provider.md.
--
-- A code is the short-lived, single-use ticket the browser carries from
-- Ruscker back to the client, which then trades it (with its secret and
-- the PKCE verifier) for the user's identity. Only a SHA-256 of the
-- code is stored: a read of this table yields nothing redeemable.
CREATE TABLE oidc_auth_codes (
    code_hash      TEXT PRIMARY KEY NOT NULL,
    -- Who the code was issued to, and for whom. No foreign keys: the
    -- token endpoint re-checks both against the live rows.
    client_id      TEXT NOT NULL,
    user_id        TEXT NOT NULL,
    username       TEXT NOT NULL,
    -- The exact redirect URI of the request; the token request must
    -- repeat it.
    redirect_uri   TEXT NOT NULL,
    -- PKCE (S256): base64url(SHA-256(verifier)).
    code_challenge TEXT NOT NULL,
    -- Echoed into the ID token when the client sent one.
    nonce          TEXT,
    -- Granted scopes, space-separated.
    scope          TEXT NOT NULL,
    -- RFC 3339 (UTC); TEXT in SQLite, TIMESTAMPTZ in Postgres.
    created_at     TEXT NOT NULL,
    expires_at     TEXT NOT NULL,
    -- Set by the one redemption that wins; NULL until then.
    consumed_at    TEXT
);

CREATE INDEX oidc_auth_codes_expires_idx ON oidc_auth_codes (expires_at);
