-- What the sign-in throttle remembers across restarts (#1094) —
-- Postgres twin. See the SQLite file for the reasoning: `pair_hash` is
-- a keyed hash of (account, address); no address is stored.
CREATE TABLE IF NOT EXISTS signin_memory (
    kind       TEXT NOT NULL,
    username   TEXT NOT NULL,
    pair_hash  TEXT NOT NULL,
    last_at    TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (kind, username, pair_hash)
);
CREATE INDEX IF NOT EXISTS signin_memory_last_at_idx ON signin_memory (last_at);

CREATE INDEX IF NOT EXISTS audit_log_action_occurred_idx ON audit_log (action, occurred_at);
