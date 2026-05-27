-- Per-user group membership for app-visibility ACLs (#155).
--
-- Stored as a comma-separated list of group names (canonical form: no
-- surrounding spaces, empty string = no groups). A spec's
-- `access-groups` is matched against these at the landing filter and
-- the `/app` enforcement guard. Group names are opaque operator-chosen
-- tokens; we don't model them as their own table yet — membership is
-- low-cardinality and edited inline on the users screen.
ALTER TABLE users ADD COLUMN groups TEXT NOT NULL DEFAULT '';
