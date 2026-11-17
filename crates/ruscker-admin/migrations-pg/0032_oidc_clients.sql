-- OIDC provider, slice 1 — Postgres twin of the SQLite migration. The
-- registry of client applications allowed to sign users in through
-- Ruscker. See docs/adr/0005-oidc-provider.md.
CREATE TABLE oidc_clients (
    client_id     TEXT        PRIMARY KEY NOT NULL,
    name          TEXT        NOT NULL,
    secret_hash   TEXT        NOT NULL,
    redirect_uris TEXT        NOT NULL,
    access_groups TEXT        NOT NULL DEFAULT '',
    require_mfa   BOOLEAN     NOT NULL DEFAULT FALSE,
    enabled       BOOLEAN     NOT NULL DEFAULT TRUE,
    created_at    TIMESTAMPTZ NOT NULL,
    updated_at    TIMESTAMPTZ NOT NULL
);
