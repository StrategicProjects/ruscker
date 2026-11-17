-- OIDC provider, slice 2: the key(s) that sign ID tokens (ES256).
-- See docs/adr/0005-oidc-provider.md.
--
-- Stored in the database, not on disk, so every node of an HA
-- deployment signs with — and publishes — the same keys.
CREATE TABLE oidc_keys (
    -- JWK thumbprint (RFC 7638) of the public key; goes in every
    -- token's header so a client picks the right key from the JWKS.
    kid        TEXT PRIMARY KEY NOT NULL,
    -- Signature algorithm. Only 'ES256' today.
    alg        TEXT NOT NULL,
    -- The PKCS#8 private key, AES-256-GCM under RUSCKER_MASTER_KEY.
    pkcs8_enc  BLOB NOT NULL,
    nonce      BLOB NOT NULL,
    -- The public half as a JWK (JSON). Public by definition; kept in
    -- the clear so the JWKS endpoint never has to decrypt anything.
    public_jwk TEXT NOT NULL,
    -- RFC 3339 (UTC); TEXT in SQLite, TIMESTAMPTZ in Postgres.
    created_at TEXT NOT NULL
);
