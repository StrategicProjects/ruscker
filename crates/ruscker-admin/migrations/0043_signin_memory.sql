-- What the sign-in throttle remembers across restarts (#1094): which
-- (account, address) pairs — and which addresses, for the break-glass
-- token — signed in correctly, and when.
--
-- The address itself is NOT stored: `pair_hash` is an HMAC-SHA256 of the
-- pair under a key derived from RUSCKER_COOKIE_KEY, which lives outside
-- the database. The throttle can recognise a pair; nobody can read an
-- address back from this table. Rows are deleted after the trust period
-- (30 days), independently of the audit retention.
CREATE TABLE signin_memory (
    -- 'password' | 'token'
    kind       TEXT NOT NULL,
    -- The account; '' for the token form, which has none.
    username   TEXT NOT NULL,
    pair_hash  TEXT NOT NULL,
    -- RFC 3339 (UTC).
    last_at    TEXT NOT NULL,
    PRIMARY KEY (kind, username, pair_hash)
);
CREATE INDEX signin_memory_last_at_idx ON signin_memory (last_at);
