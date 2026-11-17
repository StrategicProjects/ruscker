-- Full name of the person behind an account (#1081) — Postgres twin.
ALTER TABLE users ADD COLUMN IF NOT EXISTS full_name TEXT;
