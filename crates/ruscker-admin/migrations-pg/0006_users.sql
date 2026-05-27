-- Admin user accounts with per-user password login (real RBAC).
-- Postgres twin of migrations/0006. Roles mirror `auth::Role`
-- ('viewer' | 'editor' | 'admin'); passwords are argon2id PHC hashes.
--
-- As with landing_blocks, `must_change_password` is a real BOOLEAN
-- here (INTEGER 0/1 on SQLite); the query port binds/reads it as
-- `bool`. `username` is stored lowercased and is unique.
CREATE TABLE users (
    id                   TEXT        PRIMARY KEY,
    username             TEXT        NOT NULL UNIQUE,
    password_hash        TEXT        NOT NULL,
    role                 TEXT        NOT NULL,
    must_change_password BOOLEAN     NOT NULL DEFAULT FALSE,
    created_at           TIMESTAMPTZ NOT NULL,
    updated_at           TIMESTAMPTZ NOT NULL,
    created_by           TEXT
);
