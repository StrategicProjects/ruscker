-- Group roles (#1067): a free list of role names PER GROUP, and the
-- assignment of those roles to members of that group.
--
-- Not to be confused with the ACCOUNT role (`users.role`: admin / editor /
-- viewer). A group role is a label an app reads (through the opt-in
-- `identity-claims: [roles]` header) to decide what a user may do INSIDE
-- that app; it grants nothing in Ruscker itself.
--
-- Groups have no table of their own (they are derived from the CSV in
-- `users.groups` and from specs' `access-groups`), so `group_name` cannot
-- reference anything. Membership consistency ("a user holds a role only in
-- a group they belong to") is enforced in `db::group_roles` and in every
-- writer of `users.groups`. What CAN be a real foreign key is:
--   * an assignment needs its role to be defined in that group
--     (removing the definition removes the assignments);
--   * an assignment needs its user (deleting the account removes them).

CREATE TABLE group_roles (
    -- Exact, case-sensitive group name, same matching as `users.groups`.
    group_name TEXT NOT NULL,
    -- `[A-Za-z0-9_.-]{1,40}` — validated in `db::group_roles::clean_role`
    -- because it travels in an HTTP header.
    role       TEXT NOT NULL,
    -- RFC 3339 (UTC); TEXT in SQLite, TIMESTAMPTZ in Postgres.
    created_at TEXT NOT NULL,
    created_by TEXT,
    PRIMARY KEY (group_name, role)
);

CREATE TABLE user_group_roles (
    username   TEXT NOT NULL REFERENCES users(username) ON DELETE CASCADE,
    group_name TEXT NOT NULL,
    role       TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (username, group_name, role),
    FOREIGN KEY (group_name, role)
        REFERENCES group_roles(group_name, role) ON DELETE CASCADE
);

-- The cascade from `group_roles` and the per-group reads look up by group.
CREATE INDEX idx_user_group_roles_group
    ON user_group_roles(group_name, role);
