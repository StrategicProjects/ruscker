-- Postgres twin of migrations/0029_user_mfa.sql (#1005 slice 2).
CREATE TABLE user_mfa (
    username       TEXT PRIMARY KEY REFERENCES users(username) ON DELETE CASCADE,
    secret_enc     BYTEA NOT NULL,
    secret_nonce   BYTEA NOT NULL,
    -- Random per-enrollment ceremony token: binds the pending secret to
    -- the browser that passed the password re-auth (cookie) and makes the
    -- confirm UPDATE conditional, so a racing re-start can never get its
    -- replacement secret confirmed by a code proving the old one.
    ceremony       TEXT NOT NULL,
    confirmed_at   TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL,
    updated_at     TIMESTAMPTZ NOT NULL,
    -- Reserved for slice 3's TOTP replay prevention.
    last_used_step BIGINT,
    -- Bumped by every trust revocation (password set/change, forget-all).
    -- Grant issuance is conditional on the epoch read BEFORE the TOTP
    -- verification, so a revocation that lands mid-challenge wins: the
    -- in-flight grant INSERT sees a stale epoch and fails (#1005).
    security_epoch BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE user_mfa_recovery (
    id         TEXT PRIMARY KEY,
    username   TEXT NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    code_hash  TEXT NOT NULL,
    used_at    TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_user_mfa_recovery_username
    ON user_mfa_recovery(username, used_at);
