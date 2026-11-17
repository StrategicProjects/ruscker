-- OIDC provider, slice 2 — Postgres twin of the SQLite migration: the
-- key(s) that sign ID tokens. See docs/adr/0005-oidc-provider.md.
CREATE TABLE oidc_keys (
    kid        TEXT        PRIMARY KEY NOT NULL,
    alg        TEXT        NOT NULL,
    pkcs8_enc  BYTEA       NOT NULL,
    nonce      BYTEA       NOT NULL,
    public_jwk TEXT        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL
);
