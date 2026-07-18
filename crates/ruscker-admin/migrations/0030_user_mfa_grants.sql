-- Device-bound MFA proofs (#1005 slice 3). The browser holds
-- `{id}.{token}`; only the salted token hash and a SHA-256 binding to the
-- admin-session id are persisted. Grants hard-expire after at most 30 days.
CREATE TABLE user_mfa_grants (
    id                  TEXT PRIMARY KEY,
    username            TEXT NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    token_hash          TEXT NOT NULL,
    session_binding     TEXT NOT NULL,
    factor_confirmed_at TEXT NOT NULL,
    mfa_verified_at     TEXT NOT NULL,
    expires_at          TEXT NOT NULL,
    created_at          TEXT NOT NULL
);

CREATE INDEX idx_user_mfa_grants_username
    ON user_mfa_grants(username);
