-- Postgres twin of migrations/0030_user_mfa_grants.sql (#1005 slice 3).
CREATE TABLE user_mfa_grants (
    id                  TEXT PRIMARY KEY,
    username            TEXT NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    token_hash          TEXT NOT NULL,
    session_binding     TEXT NOT NULL,
    factor_confirmed_at TIMESTAMPTZ NOT NULL,
    -- Epoch do fator no momento da emissão: validado na leitura contra
    -- user_mfa.security_epoch, então um grant emitido sob uma época já
    -- revogada (corrida MVCC no pg) nunca é aceito (#1005).
    security_epoch  BIGINT NOT NULL,
    mfa_verified_at     TIMESTAMPTZ NOT NULL,
    expires_at          TIMESTAMPTZ NOT NULL,
    created_at          TIMESTAMPTZ NOT NULL,
    -- One live grant per browser-session (#1005).
    UNIQUE(username, session_binding)
);

CREATE INDEX idx_user_mfa_grants_username
    ON user_mfa_grants(username);

-- Época de revogação de confiança no fator (ver coluna acima). Vive na
-- 0030 (não na 0029) porque a 0029 pertence à fatia anterior e migrações
-- já aplicadas são imutáveis (checksum do sqlx).
ALTER TABLE user_mfa ADD COLUMN security_epoch BIGINT NOT NULL DEFAULT 0;
