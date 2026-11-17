-- Full name of the person behind an account (#1081). Optional. Sent to
-- SSO clients as the `name` claim; the username is used when it is unset.
ALTER TABLE users ADD COLUMN full_name TEXT;
