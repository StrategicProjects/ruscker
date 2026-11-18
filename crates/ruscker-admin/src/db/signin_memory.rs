//! What the sign-in throttle remembers across restarts (#1094).
//!
//! The throttle (`auth::LoginRateLimiter`) exempts people it knows from
//! its collective ceilings. It rebuilds part of that knowledge from the
//! audit log, but the audit log is the wrong place to depend on: its
//! retention is the operator's privacy choice (it may be days), and the
//! break-glass token is deliberately not in it. This table is the
//! throttle's own memory, and it holds as little as it can: a **keyed
//! hash** of each (account, address) pair and when it last signed in.
//! The key is derived from `RUSCKER_COOKIE_KEY`, which is not in the
//! database: someone holding only the database (a backup, SQL access)
//! cannot turn a hash back into an address. Someone who also holds the
//! key can, by trying addresses — this is protection against a reader
//! of the database, not against a compromise of the host.
//!
//! Without that key there is nothing stable to hash with, and the table
//! is simply not used.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use super::ConfigDb;

/// A remembered sign-in: `(kind, username, pair_hash, last_at)`.
pub type Remembered = (String, String, String, DateTime<Utc>);

/// Most rows read at once — a backstop.
const READ_LIMIT: i64 = 50_000;

/// Record (or refresh) a correct sign-in.
pub async fn upsert(
    db: &ConfigDb,
    kind: &str,
    username: &str,
    pair_hash: &str,
    at: DateTime<Utc>,
) -> Result<()> {
    match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO signin_memory (kind, username, pair_hash, last_at) VALUES (?, ?, ?, ?)
                 ON CONFLICT(kind, username, pair_hash) DO UPDATE SET last_at = excluded.last_at",
            )
            .bind(kind)
            .bind(username)
            .bind(pair_hash)
            .bind(at)
            .execute(pool)
            .await
        }
        .map(|_| ()),
        ConfigDb::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO signin_memory (kind, username, pair_hash, last_at) VALUES ($1, $2, $3, $4)
                 ON CONFLICT (kind, username, pair_hash) DO UPDATE SET last_at = EXCLUDED.last_at",
            )
            .bind(kind)
            .bind(username)
            .bind(pair_hash)
            .bind(at)
            .execute(pool)
            .await
        }
        .map(|_| ()),
    }
    .context("remember sign-in")
}

/// Sign-ins remembered since `since`, newest first, and whether the
/// backstop limit cut the list.
///
/// Password rows are returned only while they still say something about
/// the account as it is today: the account exists, the sign-in is not
/// older than it (a username can be reused), and no password change or
/// reset happened after it. Token rows have no account to check.
pub async fn since(db: &ConfigDb, since: DateTime<Utc>) -> Result<(Vec<Remembered>, bool)> {
    let rows: Vec<Remembered> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(
            "SELECT m.kind, m.username, m.pair_hash, m.last_at
               FROM signin_memory m
              WHERE m.last_at >= ?
                AND (m.kind = 'token' OR (
                      EXISTS (SELECT 1 FROM users u
                               WHERE u.username = m.username AND m.last_at >= u.created_at)
                  AND NOT EXISTS (SELECT 1 FROM audit_log l
                               WHERE l.action = 'user.password'
                                 AND l.target = 'user:' || m.username
                                 AND l.occurred_at > m.last_at)))
              ORDER BY m.last_at DESC LIMIT ?",
        )
        .bind(since)
        .bind(READ_LIMIT)
        .fetch_all(pool)
        .await,
        ConfigDb::Postgres(pool) => sqlx::query_as(
            "SELECT m.kind, m.username, m.pair_hash, m.last_at
               FROM signin_memory m
              WHERE m.last_at >= $1
                AND (m.kind = 'token' OR (
                      EXISTS (SELECT 1 FROM users u
                               WHERE u.username = m.username AND m.last_at >= u.created_at)
                  AND NOT EXISTS (SELECT 1 FROM audit_log l
                               WHERE l.action = 'user.password'
                                 AND l.target = 'user:' || m.username
                                 AND l.occurred_at > m.last_at)))
              ORDER BY m.last_at DESC LIMIT $2",
        )
        .bind(since)
        .bind(READ_LIMIT)
        .fetch_all(pool)
        .await,
    }
    .context("read remembered sign-ins")?;
    let cut = rows.len() as i64 >= READ_LIMIT;
    Ok((rows, cut))
}

/// Forget everything remembered about `username` (password change or
/// reset, account deleted). Returns how many rows went.
pub async fn forget_account(db: &ConfigDb, username: &str) -> Result<u64> {
    match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query("DELETE FROM signin_memory WHERE kind = 'password' AND username = ?")
                .bind(username)
                .execute(pool)
                .await
                .map(|r| r.rows_affected())
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query("DELETE FROM signin_memory WHERE kind = 'password' AND username = $1")
                .bind(username)
                .execute(pool)
                .await
                .map(|r| r.rows_affected())
        }
    }
    .context("forget remembered sign-ins")
}

/// Whether `username`'s password was changed or reset at or after `since`.
pub async fn password_changed_since(db: &ConfigDb, username: &str, since: DateTime<Utc>) -> Result<bool> {
    let target = format!("user:{username}");
    let n: i64 = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_scalar(
            "SELECT count(*) FROM audit_log WHERE action = 'user.password' AND target = ? AND occurred_at >= ?",
        )
        .bind(&target)
        .bind(since)
        .fetch_one(pool)
        .await,
        ConfigDb::Postgres(pool) => sqlx::query_scalar(
            "SELECT count(*) FROM audit_log WHERE action = 'user.password' AND target = $1 AND occurred_at >= $2",
        )
        .bind(&target)
        .bind(since)
        .fetch_one(pool)
        .await,
    }
    .context("check for a password change")?;
    Ok(n > 0)
}

/// Delete rows older than `cutoff` (the end of the trust period).
pub async fn purge_older_than(db: &ConfigDb, cutoff: DateTime<Utc>) -> Result<u64> {
    match db {
        ConfigDb::Sqlite(pool) => sqlx::query("DELETE FROM signin_memory WHERE last_at < ?")
            .bind(cutoff)
            .execute(pool)
            .await
            .map(|r| r.rows_affected()),
        ConfigDb::Postgres(pool) => sqlx::query("DELETE FROM signin_memory WHERE last_at < $1")
            .bind(cutoff)
            .execute(pool)
            .await
            .map(|r| r.rows_affected()),
    }
    .context("purge remembered sign-ins")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;
    use crate::db::users;

    /// The whole life of a remembered sign-in, on either database.
    async fn check(db: &ConfigDb) {
        let now = Utc::now();
        let hour = chrono::Duration::hours(1);
        let pairs = |rows: Vec<Remembered>| {
            let mut v: Vec<(String, String, String)> = rows.into_iter().map(|(k, u, h, _)| (k, u, h)).collect();
            v.sort();
            v
        };
        let s = |k: &str, u: &str, h: &str| (k.to_string(), u.to_string(), h.to_string());

        // A sign-in by an `ana` who is then deleted: the next `ana` must
        // not inherit it. (Deleting does not touch this table by itself —
        // the read has to leave the row out.)
        users::create(db, "ana", "Correct#Pass9", Role::Viewer, false, &[], None).await.unwrap();
        upsert(db, "password", "ana", "old-ana-hash", Utc::now()).await.unwrap();
        users::delete(db, "ana", None).await.unwrap();
        assert!(since(db, now - hour).await.unwrap().0.is_empty(), "no such account any more");
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        for who in ["ana", "bia"] {
            users::create(db, who, "Correct#Pass9", Role::Viewer, false, &[], None).await.unwrap();
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        upsert(db, "password", "ana", "h1", Utc::now()).await.unwrap();
        upsert(db, "password", "ana", "h2", Utc::now()).await.unwrap();
        upsert(db, "password", "bia", "h3", Utc::now()).await.unwrap();
        upsert(db, "password", "ghost", "h4", Utc::now()).await.unwrap(); // no such account
        upsert(db, "token", "", "t1", Utc::now()).await.unwrap();
        // The same pair again only refreshes its date.
        upsert(db, "password", "ana", "h1", Utc::now()).await.unwrap();

        let (rows, cut) = since(db, now - hour).await.unwrap();
        assert!(!cut);
        assert_eq!(
            pairs(rows),
            [s("password", "ana", "h1"), s("password", "ana", "h2"), s("password", "bia", "h3"), s("token", "", "t1")],
            "the old ana's row and the unknown account are left out"
        );

        // bia's password is reset: her earlier sign-ins are not read back,
        // even before anything deletes them; a later one is.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        users::set_password(db, "bia", "Another#Pass7", false, Some("root")).await.unwrap();
        assert!(!pairs(since(db, now - hour).await.unwrap().0).contains(&s("password", "bia", "h3")));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        upsert(db, "password", "bia", "h5", Utc::now()).await.unwrap();
        assert!(pairs(since(db, now - hour).await.unwrap().0).contains(&s("password", "bia", "h5")));

        // The check the sign-in path makes after remembering a pair.
        assert!(password_changed_since(db, "bia", now - hour).await.unwrap());
        assert!(!password_changed_since(db, "ana", now - hour).await.unwrap());
        assert!(!password_changed_since(db, "bia", Utc::now() + hour).await.unwrap());

        // Forgetting an account removes its password rows only.
        assert_eq!(forget_account(db, "ana").await.unwrap(), 3, "h1, h2 and the old ana's");
        let left = pairs(since(db, now - hour).await.unwrap().0);
        assert_eq!(left, [s("password", "bia", "h5"), s("token", "", "t1")]);

        // Nothing from before the period; and the purge empties the table.
        assert!(since(db, Utc::now() + hour).await.unwrap().0.is_empty());
        assert!(purge_older_than(db, Utc::now() + hour).await.unwrap() >= 2);
        assert!(since(db, now - hour).await.unwrap().0.is_empty());
    }

    #[tokio::test]
    async fn remembered_sign_ins_round_trip() {
        check(&ConfigDb::Sqlite(crate::db::open_memory().await.unwrap())).await;
    }

    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn remembered_sign_ins_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL").expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pg = crate::db::open_pg(&url).await.unwrap();
        sqlx::query("DELETE FROM signin_memory").execute(&pg).await.unwrap();
        sqlx::query("DELETE FROM users").execute(&pg).await.unwrap();
        sqlx::query("DELETE FROM audit_log").execute(&pg).await.unwrap();
        check(&ConfigDb::Postgres(pg)).await;
    }
}
