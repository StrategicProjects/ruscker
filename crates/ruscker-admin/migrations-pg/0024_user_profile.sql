-- Optional per-user profile fields (#856): setor (department), e-mail,
-- celular (mobile). All nullable — existing users migrate with NULLs and
-- the admin form leaves them blank unless filled.
ALTER TABLE users ADD COLUMN setor TEXT;
ALTER TABLE users ADD COLUMN email TEXT;
ALTER TABLE users ADD COLUMN celular TEXT;
