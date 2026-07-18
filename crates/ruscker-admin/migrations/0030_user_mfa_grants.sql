-- Device-bound MFA proofs (#1005 slice 3). The browser holds
-- `{id}.{token}`; only the salted token hash and a SHA-256 binding to the
-- admin-session id are persisted. Grants hard-expire after at most 30 days.
CREATE TABLE user_mfa_grants (
    id                  TEXT PRIMARY KEY,
    username            TEXT NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    token_hash          TEXT NOT NULL,
    session_binding     TEXT NOT NULL,
    factor_confirmed_at TEXT NOT NULL,
    -- Epoch do fator no momento da emissão: validado na leitura contra
    -- user_mfa.security_epoch, então um grant emitido sob uma época já
    -- revogada (corrida MVCC no pg) nunca é aceito (#1005).
    security_epoch  BIGINT NOT NULL,
    mfa_verified_at     TEXT NOT NULL,
    expires_at          TEXT NOT NULL,
    created_at          TEXT NOT NULL,
    -- One live grant per browser-session (#1005): a re-challenge UPSERTs
    -- the single row, so a browser can never hold two valid grants and a
    -- stale cookie after a revocation just gets a fresh grant.
    UNIQUE(username, session_binding)
);

CREATE INDEX idx_user_mfa_grants_username
    ON user_mfa_grants(username);

-- Época de revogação de confiança no fator (ver coluna acima). Vive na
-- 0030 (não na 0029) porque a 0029 pertence à fatia anterior e migrações
-- já aplicadas são imutáveis (checksum do sqlx).
ALTER TABLE user_mfa ADD COLUMN security_epoch BIGINT NOT NULL DEFAULT 0;
