-- Postgres twin of migrations/0029_user_mfa.sql (#1005 slice 2).
CREATE TABLE user_mfa (
    username       TEXT PRIMARY KEY REFERENCES users(username) ON DELETE CASCADE,
    secret_enc     BYTEA NOT NULL,
    secret_nonce   BYTEA NOT NULL,
    confirmed_at   TIMESTAMPTZ,
    created_at     TIMESTAMPTZ NOT NULL,
    updated_at     TIMESTAMPTZ NOT NULL,
    -- Reserved for slice 3's TOTP replay prevention.
    last_used_step BIGINT
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
