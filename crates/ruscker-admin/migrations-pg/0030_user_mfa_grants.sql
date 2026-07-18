-- Postgres twin of migrations/0030_user_mfa_grants.sql (#1005 slice 3).
CREATE TABLE user_mfa_grants (
    id                  TEXT PRIMARY KEY,
    username            TEXT NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    token_hash          TEXT NOT NULL,
    session_binding     TEXT NOT NULL,
    factor_confirmed_at TIMESTAMPTZ NOT NULL,
    mfa_verified_at     TIMESTAMPTZ NOT NULL,
    expires_at          TIMESTAMPTZ NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL
);

CREATE INDEX idx_user_mfa_grants_username
    ON user_mfa_grants(username);
