-- User-owned TOTP factors and one-time recovery codes (#1005 slice 2).
-- `confirmed_at IS NULL` is a pending enrollment awaiting its first valid
-- authenticator code. Secrets are AES-256-GCM ciphertext + nonce; plaintext
-- never reaches the database.
CREATE TABLE user_mfa (
    username       TEXT PRIMARY KEY REFERENCES users(username) ON DELETE CASCADE,
    secret_enc     BLOB NOT NULL,
    secret_nonce   BLOB NOT NULL,
    confirmed_at   TEXT,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    -- Reserved for slice 3's TOTP replay prevention.
    last_used_step BIGINT
);

CREATE TABLE user_mfa_recovery (
    id         TEXT PRIMARY KEY,
    username   TEXT NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    code_hash  TEXT NOT NULL,
    used_at    TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_user_mfa_recovery_username
    ON user_mfa_recovery(username, used_at);
