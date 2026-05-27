//! Admin user accounts — per-user password login + roles.
//!
//! Passwords are stored only as argon2id PHC hashes; plaintext never
//! touches the disk. The [`crate::auth::Role`] of each user drives the
//! same route guards as the env-token model it replaces. The
//! `RUSCKER_ADMIN_TOKEN` env var stays a break-glass bootstrap (see
//! [`crate::auth::AdminAuth`]) — it always grants an Admin session and
//! seeds the first account.
//!
//! All write paths attach an `audit_log` row in the same transaction
//! (mirroring the rest of `db::*`), tagged with the acting username.

use anyhow::{Context, Result};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use chrono::{DateTime, Utc};

use crate::auth::Role;
use crate::db::ConfigDb;

/// A user row without the password hash — safe to render.
#[derive(Debug, Clone)]
pub struct UserRow {
    pub id: String,
    pub username: String,
    pub role: Role,
    pub must_change_password: bool,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<String>,
}

/// Normalize a username for storage/lookup: trimmed + lowercased.
/// Usernames are case-insensitive and unique.
pub fn normalize_username(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// Hash a plaintext password with argon2id and a fresh random salt,
/// returning the PHC string to store.
pub fn hash_password(plain: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("hash password: {e}"))
}

/// Verify a plaintext password against a stored PHC hash. Returns
/// `false` on any parse/verify failure — argon2's verify is the
/// constant-time comparison.
pub fn verify_password(plain: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default()
            .verify_password(plain.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Number of users with the Admin role. Used to block removing or
/// demoting the last admin (lockout protection).
pub async fn count_admins(db: &ConfigDb) -> Result<i64> {
    let sql = "SELECT COUNT(*) FROM users WHERE role = 'admin'";
    let (n,): (i64,) = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_one(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_one(pool).await,
    }
    .context("count admins")?;
    Ok(n)
}

/// Whether any admin account exists yet. Drives the bootstrap flow:
/// with no admin user, login falls back to the break-glass token and
/// the setup wizard.
pub async fn any_admin_exists(db: &ConfigDb) -> Result<bool> {
    Ok(count_admins(db).await? > 0)
}

/// Whether any user at all exists (used by the login page to decide
/// between the bootstrap and the normal form).
pub async fn any_user_exists(db: &ConfigDb) -> Result<bool> {
    let sql = "SELECT COUNT(*) FROM users";
    let (n,): (i64,) = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_one(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_one(pool).await,
    }
    .context("count users")?;
    Ok(n > 0)
}

/// Map a `(username, role, must_change, created_at, created_by)` tuple
/// to a [`UserRow`]. An unknown role string falls back to Viewer (the
/// least-privileged) so a hand-tampered DB can't escalate.
fn row_from(
    id: String,
    username: String,
    role: String,
    must_change: bool,
    created_at: DateTime<Utc>,
    created_by: Option<String>,
) -> UserRow {
    UserRow {
        id,
        username,
        role: Role::parse(&role).unwrap_or(Role::Viewer),
        // `must_change_password` reads as `bool` on both backends: sqlx
        // decodes SQLite's INTEGER 0/1 and Postgres' native BOOLEAN.
        must_change_password: must_change,
        created_at,
        created_by,
    }
}

/// List all users (no hashes), most-recent first.
pub async fn list_all(db: &ConfigDb) -> Result<Vec<UserRow>> {
    type Row = (String, String, String, bool, DateTime<Utc>, Option<String>);
    let sql = "SELECT id, username, role, must_change_password, created_at, created_by
           FROM users
          ORDER BY created_at DESC, username ASC";
    let rows: Vec<Row> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    }
    .context("list users")?;
    Ok(rows
        .into_iter()
        .map(|(id, u, r, m, c, by)| row_from(id, u, r, m, c, by))
        .collect())
}

/// Fetch one user by (normalized) username, without the hash.
pub async fn fetch(db: &ConfigDb, username: &str) -> Result<Option<UserRow>> {
    type Row = (String, String, String, bool, DateTime<Utc>, Option<String>);
    let username = normalize_username(username);
    let row: Option<Row> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(
            "SELECT id, username, role, must_change_password, created_at, created_by
               FROM users WHERE username = ?",
        )
        .bind(&username)
        .fetch_optional(pool)
        .await,
        ConfigDb::Postgres(pool) => sqlx::query_as(
            "SELECT id, username, role, must_change_password, created_at, created_by
               FROM users WHERE username = $1",
        )
        .bind(&username)
        .fetch_optional(pool)
        .await,
    }
    .with_context(|| format!("fetch user {username}"))?;
    Ok(row.map(|(id, u, r, m, c, by)| row_from(id, u, r, m, c, by)))
}

/// Verify a login. Returns the [`UserRow`] on a correct password,
/// `None` on unknown username or wrong password (indistinguishable to
/// the caller). Always runs an argon2 verify — even for an unknown
/// user, against a dummy hash — so response time doesn't reveal
/// whether the username exists.
pub async fn verify_login(
    db: &ConfigDb,
    username: &str,
    password: &str,
) -> Result<Option<UserRow>> {
    type Row = (
        String,
        String,
        String,
        bool,
        DateTime<Utc>,
        Option<String>,
        String,
    );
    let username = normalize_username(username);
    let row: Option<Row> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(
            "SELECT id, username, role, must_change_password, created_at, created_by, password_hash
               FROM users WHERE username = ?",
        )
        .bind(&username)
        .fetch_optional(pool)
        .await,
        ConfigDb::Postgres(pool) => sqlx::query_as(
            "SELECT id, username, role, must_change_password, created_at, created_by, password_hash
               FROM users WHERE username = $1",
        )
        .bind(&username)
        .fetch_optional(pool)
        .await,
    }
    .with_context(|| format!("login lookup {username}"))?;

    match row {
        Some((id, u, r, m, c, by, hash)) if verify_password(password, &hash) => {
            Ok(Some(row_from(id, u, r, m, c, by)))
        }
        Some(_) => Ok(None),
        None => {
            // Spend the same argon2 verify time on an unknown username
            // so response timing doesn't reveal whether it exists.
            let _ = verify_password(password, dummy_hash());
            Ok(None)
        }
    }
}

/// A real argon2id hash of a throwaway password, computed once, used
/// only to keep unknown-username logins as slow as real ones.
fn dummy_hash() -> &'static str {
    use std::sync::OnceLock;
    static H: OnceLock<String> = OnceLock::new();
    H.get_or_init(|| hash_password("ruscker-timing-decoy").expect("hash dummy"))
}

/// Create a new user. `must_change` marks the optional first-login
/// password-change prompt. Errors if the username already exists.
pub async fn create(
    db: &ConfigDb,
    username: &str,
    password: &str,
    role: Role,
    must_change: bool,
    actor: Option<&str>,
) -> Result<()> {
    let username = normalize_username(username);
    let hash = hash_password(password)?;
    let now = Utc::now();
    let id = uuid::Uuid::new_v4().to_string();

    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin user tx")?;
            sqlx::query(
                "INSERT INTO users
                   (id, username, password_hash, role, must_change_password,
                    created_at, updated_at, created_by)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&username)
            .bind(&hash)
            .bind(role.as_str())
            .bind(must_change as i64)
            .bind(now)
            .bind(now)
            .bind(actor)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("insert user {username}"))?;
            audit(&mut tx, actor, "user.create", &username, &role, now).await?;
            tx.commit().await.context("commit user create")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin user tx")?;
            sqlx::query(
                "INSERT INTO users
                   (id, username, password_hash, role, must_change_password,
                    created_at, updated_at, created_by)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(&id)
            .bind(&username)
            .bind(&hash)
            .bind(role.as_str())
            .bind(must_change)
            .bind(now)
            .bind(now)
            .bind(actor)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("insert user {username}"))?;
            audit_pg(&mut tx, actor, "user.create", &username, &role, now).await?;
            tx.commit().await.context("commit user create")?;
        }
    }
    Ok(())
}

/// Replace a user's password and set/clear the must-change prompt.
pub async fn set_password(
    db: &ConfigDb,
    username: &str,
    new_password: &str,
    must_change: bool,
    actor: Option<&str>,
) -> Result<()> {
    let username = normalize_username(username);
    let hash = hash_password(new_password)?;
    let now = Utc::now();
    let target = format!("user:{username}");

    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin password tx")?;
            let res = sqlx::query(
                "UPDATE users
                    SET password_hash = ?, must_change_password = ?, updated_at = ?
                  WHERE username = ?",
            )
            .bind(&hash)
            .bind(must_change as i64)
            .bind(now)
            .bind(&username)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("set password for {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("user {username} not found");
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'user.password', ?, NULL, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit password change")?;
            tx.commit().await.context("commit password change")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin password tx")?;
            let res = sqlx::query(
                "UPDATE users
                    SET password_hash = $1, must_change_password = $2, updated_at = $3
                  WHERE username = $4",
            )
            .bind(&hash)
            .bind(must_change)
            .bind(now)
            .bind(&username)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("set password for {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("user {username} not found");
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'user.password', $2, NULL, $3)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit password change")?;
            tx.commit().await.context("commit password change")?;
        }
    }
    Ok(())
}

/// Change a user's role.
pub async fn set_role(
    db: &ConfigDb,
    username: &str,
    role: Role,
    actor: Option<&str>,
) -> Result<()> {
    let username = normalize_username(username);
    let now = Utc::now();

    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin role tx")?;
            let res = sqlx::query("UPDATE users SET role = ?, updated_at = ? WHERE username = ?")
                .bind(role.as_str())
                .bind(now)
                .bind(&username)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("set role for {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("user {username} not found");
            }
            audit(&mut tx, actor, "user.role", &username, &role, now).await?;
            tx.commit().await.context("commit role change")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin role tx")?;
            let res = sqlx::query("UPDATE users SET role = $1, updated_at = $2 WHERE username = $3")
                .bind(role.as_str())
                .bind(now)
                .bind(&username)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("set role for {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("user {username} not found");
            }
            audit_pg(&mut tx, actor, "user.role", &username, &role, now).await?;
            tx.commit().await.context("commit role change")?;
        }
    }
    Ok(())
}

/// Clear the first-login password-change prompt once it's been
/// answered (whether or not the user actually changed it). No audit —
/// it's a benign per-user UI flag. `= FALSE` is valid on both backends.
pub async fn clear_must_change(db: &ConfigDb, username: &str) -> Result<()> {
    let username = normalize_username(username);
    let ctx = || format!("clear must-change for {username}");
    match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query("UPDATE users SET must_change_password = FALSE WHERE username = ?")
                .bind(&username)
                .execute(pool)
                .await
                .with_context(ctx)?;
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query("UPDATE users SET must_change_password = FALSE WHERE username = $1")
                .bind(&username)
                .execute(pool)
                .await
                .with_context(ctx)?;
        }
    }
    Ok(())
}

/// Delete a user.
pub async fn delete(db: &ConfigDb, username: &str, actor: Option<&str>) -> Result<()> {
    let username = normalize_username(username);
    let now = Utc::now();
    let target = format!("user:{username}");

    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin user delete")?;
            let res = sqlx::query("DELETE FROM users WHERE username = ?")
                .bind(&username)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("delete user {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("user {username} not found");
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'user.delete', ?, NULL, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit user delete")?;
            tx.commit().await.context("commit user delete")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin user delete")?;
            let res = sqlx::query("DELETE FROM users WHERE username = $1")
                .bind(&username)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("delete user {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("user {username} not found");
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'user.delete', $2, NULL, $3)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit user delete")?;
            tx.commit().await.context("commit user delete")?;
        }
    }
    Ok(())
}

/// Shared audit insert for create/role changes (records the new role
/// in `diff_json`).
async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor: Option<&str>,
    action: &str,
    username: &str,
    role: &Role,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(actor)
    .bind(action)
    .bind(format!("user:{username}"))
    .bind(serde_json::json!({ "role": role.as_str() }).to_string())
    .bind(now)
    .execute(&mut **tx)
    .await
    .context("audit user op")?;
    Ok(())
}

/// Postgres twin of [`audit`] — `$n` placeholders.
async fn audit_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Option<&str>,
    action: &str,
    username: &str,
    role: &Role,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(actor)
    .bind(action)
    .bind(format!("user:{username}"))
    .bind(serde_json::json!({ "role": role.as_str() }).to_string())
    .bind(now)
    .execute(&mut **tx)
    .await
    .context("audit user op")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> ConfigDb {
        ConfigDb::Sqlite(crate::db::open_memory().await.expect("memory db"))
    }

    #[test]
    fn hash_and_verify_roundtrip() {
        let h = hash_password("s3cret-pw").unwrap();
        assert!(verify_password("s3cret-pw", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("s3cret-pw", "not-a-phc-string"));
    }

    #[tokio::test]
    async fn create_then_verify_login() {
        let p = pool().await;
        create(&p, "Alice", "pw-alice", Role::Editor, true, Some("admin"))
            .await
            .unwrap();
        // Username is case-insensitive.
        let u = verify_login(&p, "alice", "pw-alice").await.unwrap();
        let u = u.expect("login ok");
        assert_eq!(u.username, "alice");
        assert_eq!(u.role, Role::Editor);
        assert!(u.must_change_password);
        // Wrong password / unknown user ⇒ None.
        assert!(verify_login(&p, "alice", "nope").await.unwrap().is_none());
        assert!(verify_login(&p, "ghost", "pw").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn admin_count_and_roles() {
        let p = pool().await;
        assert!(!any_admin_exists(&p).await.unwrap());
        create(&p, "root", "pw", Role::Admin, false, None)
            .await
            .unwrap();
        create(&p, "ed", "pw", Role::Editor, false, None)
            .await
            .unwrap();
        assert_eq!(count_admins(&p).await.unwrap(), 1);
        assert!(any_admin_exists(&p).await.unwrap());
        // Promote editor → admin.
        set_role(&p, "ed", Role::Admin, Some("root")).await.unwrap();
        assert_eq!(count_admins(&p).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn duplicate_username_rejected() {
        let p = pool().await;
        create(&p, "dup", "pw", Role::Viewer, false, None)
            .await
            .unwrap();
        assert!(create(&p, "DUP", "pw2", Role::Viewer, false, None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn set_password_clears_must_change() {
        let p = pool().await;
        create(&p, "bob", "init-pw", Role::Viewer, true, Some("admin"))
            .await
            .unwrap();
        set_password(&p, "bob", "new-pw", false, Some("bob"))
            .await
            .unwrap();
        let u = verify_login(&p, "bob", "new-pw").await.unwrap().unwrap();
        assert!(!u.must_change_password);
        assert!(verify_login(&p, "bob", "init-pw").await.unwrap().is_none());
    }

    // Full user lifecycle through the `ConfigDb::Postgres` arm against a
    // real daemon (BOOLEAN `must_change_password`, argon2 verify, audit,
    // last-admin count). Gated on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn users_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pg = crate::db::open_pg(&url).await.unwrap();
        sqlx::query("DELETE FROM users")
            .execute(&pg)
            .await
            .unwrap();
        let db = ConfigDb::Postgres(pg);

        assert!(!any_admin_exists(&db).await.unwrap());
        create(&db, "Root", "rootpw12", Role::Admin, false, None)
            .await
            .unwrap();
        create(&db, "Ed", "edpw1234", Role::Editor, true, Some("root"))
            .await
            .unwrap();
        assert_eq!(count_admins(&db).await.unwrap(), 1);
        assert!(any_user_exists(&db).await.unwrap());
        assert_eq!(list_all(&db).await.unwrap().len(), 2);

        // Case-insensitive login + must_change BOOLEAN round-trip.
        let u = verify_login(&db, "ed", "edpw1234")
            .await
            .unwrap()
            .expect("login ok");
        assert_eq!(u.role, Role::Editor);
        assert!(u.must_change_password);
        assert!(verify_login(&db, "ed", "wrong").await.unwrap().is_none());
        assert!(verify_login(&db, "ghost", "x").await.unwrap().is_none());

        // Promote, re-set password (clears must_change), then delete.
        set_role(&db, "ed", Role::Admin, Some("root")).await.unwrap();
        assert_eq!(count_admins(&db).await.unwrap(), 2);
        set_password(&db, "ed", "newpw123", false, Some("ed"))
            .await
            .unwrap();
        assert!(!verify_login(&db, "ed", "newpw123")
            .await
            .unwrap()
            .unwrap()
            .must_change_password);
        clear_must_change(&db, "root").await.unwrap();
        delete(&db, "ed", Some("root")).await.unwrap();
        assert!(fetch(&db, "ed").await.unwrap().is_none());
        assert_eq!(count_admins(&db).await.unwrap(), 1);
    }
}
