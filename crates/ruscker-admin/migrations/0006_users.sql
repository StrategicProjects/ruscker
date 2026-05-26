-- Admin user accounts with per-user password login (real RBAC).
--
-- Roles mirror `auth::Role` ('viewer' | 'editor' | 'admin'). Passwords
-- are stored only as argon2id PHC hashes — never plaintext. The
-- `RUSCKER_ADMIN_TOKEN` env var remains a break-glass bootstrap that
-- always grants an Admin session and seeds the first account here.
--
-- `must_change_password` drives the optional "want to change your
-- password?" prompt on a user's first login (set when an admin assigns
-- an initial password; cleared after the prompt is answered once).
-- `username` is stored lowercased and is unique.
CREATE TABLE users (
    id                   TEXT    PRIMARY KEY,
    username             TEXT    NOT NULL UNIQUE,
    password_hash        TEXT    NOT NULL,
    role                 TEXT    NOT NULL,
    must_change_password INTEGER NOT NULL DEFAULT 0,
    created_at           TEXT    NOT NULL,
    updated_at           TEXT    NOT NULL,
    created_by           TEXT
);
