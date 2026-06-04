-- Index spec_access by day (#589). `recent_series` filters `WHERE day >= ?`
-- for the trend sparkline, but the PRIMARY KEY (spec_id, day) leads with
-- spec_id and cannot serve that predicate, so the query was a full scan on
-- every Apps-list render. No retention is applied: the all-time access
-- total is a SUM over every row, so old rows must stay.
CREATE INDEX IF NOT EXISTS spec_access_day_idx ON spec_access (day);
