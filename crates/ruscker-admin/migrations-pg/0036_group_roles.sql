-- Group roles (#1067) — Postgres twin of the SQLite migration. A free list
-- of role names per group and their assignment to members. Unrelated to the
-- ACCOUNT role in `users.role`. See the SQLite file for the rationale.
CREATE TABLE group_roles (
    group_name TEXT        NOT NULL,
    role       TEXT        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    created_by TEXT,
    PRIMARY KEY (group_name, role)
);

CREATE TABLE user_group_roles (
    username   TEXT        NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    group_name TEXT        NOT NULL,
    role       TEXT        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (username, group_name, role),
    FOREIGN KEY (group_name, role)
        REFERENCES group_roles(group_name, role) ON DELETE CASCADE
);

CREATE INDEX idx_user_group_roles_group
    ON user_group_roles(group_name, role);
