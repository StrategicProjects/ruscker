-- Postgres twin of migrations/0014 (#549). Idempotent.
CREATE TABLE IF NOT EXISTS spec_access (
    spec_id TEXT NOT NULL,
    day     TEXT NOT NULL,
    count   BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (spec_id, day)
);
