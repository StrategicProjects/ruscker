-- Per-spec access counter (#549). One row per (spec, day) bucket,
-- bumped atomically on each visit (an app's new sticky session, or a
-- click on an external card — which routes through /app/{id}, a 302 to
-- the link). Daily buckets stay cheap and enable trends; the admin
-- shows a SUM per spec.
CREATE TABLE IF NOT EXISTS spec_access (
    spec_id TEXT NOT NULL,
    day     TEXT NOT NULL,            -- 'YYYY-MM-DD' (UTC)
    count   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (spec_id, day)
);
