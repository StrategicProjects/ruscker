//! Persistent TOTP enrollment state and one-time recovery codes.
//!
//! The repository stores only AES-GCM ciphertext/nonce for the TOTP secret.
//! Decryption belongs to the route layer, where `AppState::master_key` lives.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};

use crate::db::ConfigDb;

#[derive(Debug, Clone)]
pub struct MfaRow {
    pub username: String,
    pub secret_enc: Vec<u8>,
    pub secret_nonce: Vec<u8>,
    /// Random per-enrollment token binding the pending secret to the
    /// browser that passed the password re-auth (codex review, #1005):
    /// confirm requires it both to re-render the secret and in the
    /// conditional UPDATE, so neither a hijacked same-user session nor a
    /// racing re-start can act on a ceremony it didn't begin.
    pub ceremony: String,
    pub confirmed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Reserved for slice 3's accepted-step replay prevention.
    pub last_used_step: Option<i64>,
    /// Trust-revocation epoch: grant issuance is conditional on the value
    /// read before verification, so revocations racing a challenge win.
    pub security_epoch: i64,
}

type StoredRow = (
    String,
    Vec<u8>,
    Vec<u8>,
    String,
    Option<DateTime<Utc>>,
    DateTime<Utc>,
    DateTime<Utc>,
    Option<i64>,
    i64,
);

/// Start or replace a pending enrollment. A confirmed factor is never
/// overwritten; administrators must reset it explicitly before re-enrollment.
pub async fn begin_enrollment(
    db: &ConfigDb,
    username: &str,
    secret_enc: &[u8],
    nonce: &[u8],
    ceremony: &str,
) -> Result<()> {
    let username = crate::db::users::normalize_username(username);
    let now = Utc::now();
    let affected = match db {
        ConfigDb::Sqlite(pool) => sqlx::query(
            "INSERT INTO user_mfa
                (username, secret_enc, secret_nonce, ceremony, confirmed_at, created_at, updated_at, last_used_step)
             VALUES (?, ?, ?, ?, NULL, ?, ?, NULL)
             ON CONFLICT(username) DO UPDATE SET
                secret_enc = excluded.secret_enc,
                secret_nonce = excluded.secret_nonce,
                ceremony = excluded.ceremony,
                confirmed_at = NULL,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at,
                last_used_step = NULL
             WHERE user_mfa.confirmed_at IS NULL",
        )
        .bind(&username)
        .bind(secret_enc)
        .bind(nonce)
        .bind(ceremony)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .with_context(|| format!("begin MFA enrollment for {username}"))?
        .rows_affected(),
        ConfigDb::Postgres(pool) => sqlx::query(
            "INSERT INTO user_mfa
                (username, secret_enc, secret_nonce, ceremony, confirmed_at, created_at, updated_at, last_used_step)
             VALUES ($1, $2, $3, $4, NULL, $5, $6, NULL)
             ON CONFLICT(username) DO UPDATE SET
                secret_enc = excluded.secret_enc,
                secret_nonce = excluded.secret_nonce,
                ceremony = excluded.ceremony,
                confirmed_at = NULL,
                created_at = excluded.created_at,
                updated_at = excluded.updated_at,
                last_used_step = NULL
             WHERE user_mfa.confirmed_at IS NULL",
        )
        .bind(&username)
        .bind(secret_enc)
        .bind(nonce)
        .bind(ceremony)
        .bind(now)
        .bind(now)
        .execute(pool)
        .await
        .with_context(|| format!("begin MFA enrollment for {username}"))?
        .rows_affected(),
    };
    if affected == 0 {
        bail!("MFA is already confirmed for {username}; reset it before re-enrolling");
    }
    Ok(())
}

/// Mark a pending factor confirmed and atomically audit `mfa.enroll`.
pub async fn confirm_enrollment(
    db: &ConfigDb,
    username: &str,
    actor: &str,
    ceremony: &str,
) -> Result<()> {
    confirm_with_recovery_codes(db, username, actor, None, ceremony).await
}

/// Confirm enrollment, replace the recovery-code hashes, and write the audit
/// row in one transaction. The conditional pending-row update is the HA-safe
/// winner election when two confirm requests race: exactly one can commit and
/// show codes that match the stored hashes.
pub async fn confirm_with_recovery_codes(
    db: &ConfigDb,
    username: &str,
    actor: &str,
    recovery_hashes: Option<&[String]>,
    ceremony: &str,
) -> Result<()> {
    let username = crate::db::users::normalize_username(username);
    let target = format!("user:{username}");
    let now = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin MFA confirm")?;
            let result = sqlx::query(
                "UPDATE user_mfa
                    SET confirmed_at = ?, updated_at = ?
                  WHERE username = ? AND confirmed_at IS NULL AND ceremony = ?",
            )
            .bind(now)
            .bind(now)
            .bind(&username)
            .bind(ceremony)
            .execute(&mut *tx)
            .await
            .context("confirm MFA enrollment")?;
            if result.rows_affected() == 0 {
                bail!("pending MFA enrollment not found for {username}");
            }
            if let Some(hashes) = recovery_hashes {
                sqlx::query("DELETE FROM user_mfa_recovery WHERE username = ?")
                    .bind(&username)
                    .execute(&mut *tx)
                    .await
                    .context("delete old recovery codes during confirm")?;
                for hash in hashes {
                    sqlx::query(
                        "INSERT INTO user_mfa_recovery
                            (id, username, code_hash, used_at, created_at)
                         VALUES (?, ?, ?, NULL, ?)",
                    )
                    .bind(uuid::Uuid::new_v4().to_string())
                    .bind(&username)
                    .bind(hash)
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .context("insert recovery code during confirm")?;
                }
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'mfa.enroll', ?, NULL, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit MFA enrollment")?;
            tx.commit().await.context("commit MFA confirm")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin MFA confirm")?;
            let result = sqlx::query(
                "UPDATE user_mfa
                    SET confirmed_at = $1, updated_at = $2
                  WHERE username = $3 AND confirmed_at IS NULL AND ceremony = $4",
            )
            .bind(now)
            .bind(now)
            .bind(&username)
            .bind(ceremony)
            .execute(&mut *tx)
            .await
            .context("confirm MFA enrollment")?;
            if result.rows_affected() == 0 {
                bail!("pending MFA enrollment not found for {username}");
            }
            if let Some(hashes) = recovery_hashes {
                sqlx::query("DELETE FROM user_mfa_recovery WHERE username = $1")
                    .bind(&username)
                    .execute(&mut *tx)
                    .await
                    .context("delete old recovery codes during confirm")?;
                for hash in hashes {
                    sqlx::query(
                        "INSERT INTO user_mfa_recovery
                            (id, username, code_hash, used_at, created_at)
                         VALUES ($1, $2, $3, NULL, $4)",
                    )
                    .bind(uuid::Uuid::new_v4().to_string())
                    .bind(&username)
                    .bind(hash)
                    .bind(now)
                    .execute(&mut *tx)
                    .await
                    .context("insert recovery code during confirm")?;
                }
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'mfa.enroll', $2, NULL, $3)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit MFA enrollment")?;
            tx.commit().await.context("commit MFA confirm")?;
        }
    }
    Ok(())
}

pub async fn fetch(db: &ConfigDb, username: &str) -> Result<Option<MfaRow>> {
    let username = crate::db::users::normalize_username(username);
    let row: Option<StoredRow> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as(
                "SELECT username, secret_enc, secret_nonce, ceremony, confirmed_at,
                    created_at, updated_at, last_used_step, security_epoch
               FROM user_mfa WHERE username = ?",
            )
            .bind(&username)
            .fetch_optional(pool)
            .await
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as(
                "SELECT username, secret_enc, secret_nonce, ceremony, confirmed_at,
                    created_at, updated_at, last_used_step, security_epoch
               FROM user_mfa WHERE username = $1",
            )
            .bind(&username)
            .fetch_optional(pool)
            .await
        }
    }
    .with_context(|| format!("fetch MFA state for {username}"))?;
    Ok(row.map(
        |(
            username,
            secret_enc,
            secret_nonce,
            ceremony,
            confirmed_at,
            created_at,
            updated_at,
            last_used_step,
            security_epoch,
        )| {
            MfaRow {
                username,
                secret_enc,
                secret_nonce,
                ceremony,
                confirmed_at,
                created_at,
                updated_at,
                last_used_step,
                security_epoch,
            }
        },
    ))
}

/// Replace every recovery code for a user. Callers pass salted hashes only;
/// plaintext codes are never accepted by this repository.
pub async fn replace_recovery_codes(
    db: &ConfigDb,
    username: &str,
    hashes: &[String],
) -> Result<()> {
    let username = crate::db::users::normalize_username(username);
    let now = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin recovery replacement")?;
            sqlx::query("DELETE FROM user_mfa_recovery WHERE username = ?")
                .bind(&username)
                .execute(&mut *tx)
                .await
                .context("delete old recovery codes")?;
            for hash in hashes {
                sqlx::query(
                    "INSERT INTO user_mfa_recovery
                        (id, username, code_hash, used_at, created_at)
                     VALUES (?, ?, ?, NULL, ?)",
                )
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&username)
                .bind(hash)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("insert recovery code")?;
            }
            tx.commit().await.context("commit recovery replacement")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin recovery replacement")?;
            sqlx::query("DELETE FROM user_mfa_recovery WHERE username = $1")
                .bind(&username)
                .execute(&mut *tx)
                .await
                .context("delete old recovery codes")?;
            for hash in hashes {
                sqlx::query(
                    "INSERT INTO user_mfa_recovery
                        (id, username, code_hash, used_at, created_at)
                     VALUES ($1, $2, $3, NULL, $4)",
                )
                .bind(uuid::Uuid::new_v4().to_string())
                .bind(&username)
                .bind(hash)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("insert recovery code")?;
            }
            tx.commit().await.context("commit recovery replacement")?;
        }
    }
    Ok(())
}

/// Find the id of the unused recovery code matching `code`, WITHOUT
/// consuming it. Consumption happens inside [`crate::db::mfa_grants::issue`]
/// so a failed grant issuance rolls the spend back — a finite code must
/// never be burned with nothing to show for it (codex review, #1005).
pub async fn find_recovery_candidate(
    db: &ConfigDb,
    username: &str,
    code: &str,
) -> Result<Option<String>> {
    let username = crate::db::users::normalize_username(username);
    let rows: Vec<(String, String)> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as(
                "SELECT id, code_hash FROM user_mfa_recovery
                  WHERE username = ? AND used_at IS NULL",
            )
            .bind(&username)
            .fetch_all(pool)
            .await
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as(
                "SELECT id, code_hash FROM user_mfa_recovery
                  WHERE username = $1 AND used_at IS NULL",
            )
            .bind(&username)
            .fetch_all(pool)
            .await
        }
    }
    .context("fetch recovery candidates")?;
    Ok(rows
        .into_iter()
        .find(|(_, hash)| crate::mfa::verify_recovery_code(code, hash))
        .map(|(id, _)| id))
}

/// Consume a matching unused recovery code exactly once. At most ten hashes
/// are checked; every digest comparison is constant-time in `crate::mfa`.
pub async fn consume_recovery_code(db: &ConfigDb, username: &str, code: &str) -> Result<bool> {
    let username = crate::db::users::normalize_username(username);
    let now = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin recovery consumption")?;
            let rows: Vec<(String, String)> = sqlx::query_as(
                "SELECT id, code_hash FROM user_mfa_recovery
                  WHERE username = ? AND used_at IS NULL",
            )
            .bind(&username)
            .fetch_all(&mut *tx)
            .await
            .context("fetch recovery hashes")?;
            let Some(id) = rows
                .iter()
                .find(|(_, hash)| crate::mfa::verify_recovery_code(code, hash))
                .map(|(id, _)| id)
            else {
                let _ = tx.rollback().await;
                return Ok(false);
            };
            let changed = sqlx::query(
                "UPDATE user_mfa_recovery SET used_at = ?
                  WHERE id = ? AND used_at IS NULL",
            )
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await
            .context("consume recovery code")?
            .rows_affected();
            tx.commit().await.context("commit recovery consumption")?;
            Ok(changed == 1)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin recovery consumption")?;
            let rows: Vec<(String, String)> = sqlx::query_as(
                "SELECT id, code_hash FROM user_mfa_recovery
                  WHERE username = $1 AND used_at IS NULL",
            )
            .bind(&username)
            .fetch_all(&mut *tx)
            .await
            .context("fetch recovery hashes")?;
            let Some(id) = rows
                .iter()
                .find(|(_, hash)| crate::mfa::verify_recovery_code(code, hash))
                .map(|(id, _)| id)
            else {
                let _ = tx.rollback().await;
                return Ok(false);
            };
            let changed = sqlx::query(
                "UPDATE user_mfa_recovery SET used_at = $1
                  WHERE id = $2 AND used_at IS NULL",
            )
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await
            .context("consume recovery code")?
            .rows_affected();
            tx.commit().await.context("commit recovery consumption")?;
            Ok(changed == 1)
        }
    }
}

pub async fn is_enrolled(db: &ConfigDb, username: &str) -> Result<bool> {
    let username = crate::db::users::normalize_username(username);
    let (exists,): (bool,) = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as(
                "SELECT EXISTS(SELECT 1 FROM user_mfa
                            WHERE username = ? AND confirmed_at IS NOT NULL)",
            )
            .bind(&username)
            .fetch_one(pool)
            .await
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as(
                "SELECT EXISTS(SELECT 1 FROM user_mfa
                            WHERE username = $1 AND confirmed_at IS NOT NULL)",
            )
            .bind(&username)
            .fetch_one(pool)
            .await
        }
    }
    .with_context(|| format!("read MFA status for {username}"))?;
    Ok(exists)
}

/// Atomically accept a TOTP time-step only when it is newer than the last
/// successful one for this enrollment. This closes replay races across both
/// challenge and enrollment-confirm requests, including active-active nodes.
pub async fn record_used_step(db: &ConfigDb, username: &str, step: i64) -> Result<bool> {
    let username = crate::db::users::normalize_username(username);
    let changed = match db {
        ConfigDb::Sqlite(pool) => sqlx::query(
            "UPDATE user_mfa SET last_used_step = ?
              WHERE username = ? AND (last_used_step IS NULL OR last_used_step < ?)",
        )
        .bind(step)
        .bind(&username)
        .bind(step)
        .execute(pool)
        .await
        .with_context(|| format!("record MFA TOTP step for {username}"))?
        .rows_affected(),
        ConfigDb::Postgres(pool) => sqlx::query(
            "UPDATE user_mfa SET last_used_step = $1
              WHERE username = $2 AND (last_used_step IS NULL OR last_used_step < $1)",
        )
        .bind(step)
        .bind(&username)
        .execute(pool)
        .await
        .with_context(|| format!("record MFA TOTP step for {username}"))?
        .rows_affected(),
    };
    Ok(changed == 1)
}

/// Delete the factor and every recovery code, then audit `mfa.reset` in the
/// same transaction. This is shared by the admin UI and later MFA slices.
pub async fn reset(db: &ConfigDb, username: &str, actor: &str) -> Result<()> {
    let username = crate::db::users::normalize_username(username);
    let target = format!("user:{username}");
    let now = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin MFA reset")?;
            crate::db::mfa_grants::delete_all_sqlite(&mut tx, &username).await?;
            sqlx::query("DELETE FROM user_mfa_recovery WHERE username = ?")
                .bind(&username)
                .execute(&mut *tx)
                .await
                .context("delete recovery codes")?;
            sqlx::query("DELETE FROM user_mfa WHERE username = ?")
                .bind(&username)
                .execute(&mut *tx)
                .await
                .context("delete MFA factor")?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'mfa.reset', ?, NULL, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit MFA reset")?;
            tx.commit().await.context("commit MFA reset")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin MFA reset")?;
            crate::db::mfa_grants::delete_all_postgres(&mut tx, &username).await?;
            sqlx::query("DELETE FROM user_mfa_recovery WHERE username = $1")
                .bind(&username)
                .execute(&mut *tx)
                .await
                .context("delete recovery codes")?;
            sqlx::query("DELETE FROM user_mfa WHERE username = $1")
                .bind(&username)
                .execute(&mut *tx)
                .await
                .context("delete MFA factor")?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'mfa.reset', $2, NULL, $3)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit MFA reset")?;
            tx.commit().await.context("commit MFA reset")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;

    async fn db_with_user(username: &str) -> ConfigDb {
        let pool = crate::db::open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool);
        crate::db::users::create(
            &db,
            username,
            "Correct#123",
            Role::Viewer,
            false,
            &[],
            Some("test"),
        )
        .await
        .unwrap();
        db
    }

    #[tokio::test]
    async fn confirmed_factor_cannot_be_overwritten_without_reset() {
        let db = db_with_user("alice").await;
        begin_enrollment(&db, "alice", b"cipher", b"nonce", "cer-1")
            .await
            .unwrap();
        confirm_enrollment(&db, "alice", "alice", "cer-1").await.unwrap();
        assert!(begin_enrollment(&db, "alice", b"new", b"nonce", "cer-2")
            .await
            .is_err());
    }

    #[tokio::test]
    async fn recovery_code_consumes_once() {
        let db = db_with_user("alice").await;
        let hash = crate::mfa::hash_recovery_code("ABCD234567").unwrap();
        replace_recovery_codes(&db, "alice", &[hash]).await.unwrap();
        assert!(consume_recovery_code(&db, "alice", "abcd234567")
            .await
            .unwrap());
        assert!(!consume_recovery_code(&db, "alice", "ABCD234567")
            .await
            .unwrap());
    }

    #[tokio::test]
    async fn reset_removes_factor_and_codes_and_audits() {
        let db = db_with_user("alice").await;
        begin_enrollment(&db, "alice", b"cipher", b"nonce", "cer-1")
            .await
            .unwrap();
        replace_recovery_codes(
            &db,
            "alice",
            &[crate::mfa::hash_recovery_code("ABCD234567").unwrap()],
        )
        .await
        .unwrap();
        reset(&db, "alice", "root").await.unwrap();
        assert!(fetch(&db, "alice").await.unwrap().is_none());
        let filter = crate::db::audit::AuditFilter {
            actor: Some("root".to_string()),
            ..crate::db::audit::AuditFilter::new()
        };
        let audit = crate::db::audit::list(&db, &filter).await.unwrap();
        assert!(audit.iter().any(|row| row.action == "mfa.reset"));
    }

    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn postgres_begin_confirm_reset_round_trip() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pool = crate::db::open_pg(&url).await.unwrap();
        let db = ConfigDb::Postgres(pool);
        let username = format!("mfa-pg-{}", uuid::Uuid::new_v4().simple());
        crate::db::users::create(
            &db,
            &username,
            "Correct#123",
            Role::Viewer,
            false,
            &[],
            Some("test"),
        )
        .await
        .unwrap();
        begin_enrollment(&db, &username, b"cipher", b"nonce", "cer-pg")
            .await
            .unwrap();
        assert!(fetch(&db, &username)
            .await
            .unwrap()
            .unwrap()
            .confirmed_at
            .is_none());
        confirm_enrollment(&db, &username, &username, "cer-pg").await.unwrap();
        assert!(is_enrolled(&db, &username).await.unwrap());
        let factor_confirmed_at = fetch(&db, &username)
            .await
            .unwrap()
            .unwrap()
            .confirmed_at
            .unwrap();
        assert!(record_used_step(&db, &username, 42).await.unwrap());
        assert!(!record_used_step(&db, &username, 42).await.unwrap());
        let verified_at = Utc::now();
        let grant_id = crate::db::mfa_grants::create(
            &db,
            &username,
            "salt:hash",
            "session-binding",
            factor_confirmed_at,
            verified_at,
            verified_at + chrono::Duration::days(30),
            0,
        )
        .await
        .unwrap()
        .expect("grant issued under current epoch");
        let grant = crate::db::mfa_grants::fetch_valid(&db, &grant_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(grant.username, username);
        assert_eq!(
            crate::db::mfa_grants::revoke_all(&db, &username, "root", "postgres-test")
                .await
                .unwrap(),
            1
        );
        assert!(crate::db::mfa_grants::fetch_valid(&db, &grant_id)
            .await
            .unwrap()
            .is_none());
        reset(&db, &username, "root").await.unwrap();
        assert!(fetch(&db, &username).await.unwrap().is_none());
        crate::db::users::delete(&db, &username, Some("test"))
            .await
            .unwrap();
    }
}
