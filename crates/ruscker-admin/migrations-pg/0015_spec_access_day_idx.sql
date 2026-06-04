-- Postgres twin of migrations/0015 (#589). Index spec_access(day) so the
-- `WHERE day >= $1` trend query uses an index instead of a full scan.
CREATE INDEX IF NOT EXISTS spec_access_day_idx ON spec_access (day);
