-- Per-user favorite apps (#858): the landing card star toggle. The
-- "Featured" section shows a viewer's own favorites first, then the
-- admin-pinned `featured` cards. Composite PK = one favorite per
-- (user, spec); orphan rows from a deleted user are harmless.
CREATE TABLE IF NOT EXISTS user_favorites (
    username TEXT NOT NULL,
    spec_id  TEXT NOT NULL,
    PRIMARY KEY (username, spec_id)
);
