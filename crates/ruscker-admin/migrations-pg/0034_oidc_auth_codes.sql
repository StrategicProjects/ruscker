-- OIDC provider, slice 3 — Postgres twin of the SQLite migration:
-- single-use authorization codes, stored as a SHA-256 only. See
-- docs/adr/0005-oidc-provider.md.
CREATE TABLE oidc_auth_codes (
    code_hash      TEXT        PRIMARY KEY NOT NULL,
    client_id      TEXT        NOT NULL,
    user_id        TEXT        NOT NULL,
    username       TEXT        NOT NULL,
    redirect_uri   TEXT        NOT NULL,
    code_challenge TEXT        NOT NULL,
    nonce          TEXT,
    scope          TEXT        NOT NULL,
    created_at     TIMESTAMPTZ NOT NULL,
    expires_at     TIMESTAMPTZ NOT NULL,
    consumed_at    TIMESTAMPTZ
);

CREATE INDEX oidc_auth_codes_expires_idx ON oidc_auth_codes (expires_at);
