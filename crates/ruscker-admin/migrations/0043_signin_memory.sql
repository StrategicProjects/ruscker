-- What the sign-in throttle remembers across restarts (#1094): which
-- (account, address) pairs — and which addresses, for the break-glass
-- token — signed in correctly, and when.
--
-- The address itself is NOT stored: `pair_hash` is an HMAC-SHA256 of the
-- pair under a key derived from RUSCKER_COOKIE_KEY, which is not in the
-- database. Someone holding only the database (a backup, SQL access)
-- cannot turn a hash back into an address; someone who also holds the
-- key can, by trying addresses. Rows are deleted after the trust period
-- (30 days), independently of the audit retention.
CREATE TABLE IF NOT EXISTS signin_memory (
    -- 'password' | 'token'
    kind       TEXT NOT NULL,
    -- The account; '' for the token form, which has none.
    username   TEXT NOT NULL,
    pair_hash  TEXT NOT NULL,
    -- RFC 3339 (UTC).
    last_at    TEXT NOT NULL,
    PRIMARY KEY (kind, username, pair_hash)
);
CREATE INDEX IF NOT EXISTS signin_memory_last_at_idx ON signin_memory (last_at);

-- The throttle asks the audit log, twice a minute, which accounts had
-- their password changed or were deleted lately.
CREATE INDEX IF NOT EXISTS audit_log_action_occurred_idx ON audit_log (action, occurred_at);
