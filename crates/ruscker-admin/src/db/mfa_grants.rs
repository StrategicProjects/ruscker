//! Server-side trusted-device grants for user-owned MFA proofs.
//!
//! The cookie's random token is never stored verbatim. `token_hash` carries
//! its own random salt (the same `salt:sha256` convention as recovery codes),
//! while `session_binding` is a one-way hash of the opaque admin-session id.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::db::ConfigDb;

#[derive(Debug, Clone)]
pub struct GrantRow {
    pub id: String,
    pub username: String,
    pub token_hash: String,
    pub session_binding: String,
    pub factor_confirmed_at: DateTime<Utc>,
    pub mfa_verified_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

type StoredRow = (
    String,
    String,
    String,
    String,
    DateTime<Utc>,
    DateTime<Utc>,
    DateTime<Utc>,
    DateTime<Utc>,
);

pub async fn create(
    db: &ConfigDb,
    username: &str,
    token_hash: &str,
    session_binding: &str,
    factor_confirmed_at: DateTime<Utc>,
    verified_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    expected_epoch: i64,
) -> Result<Option<String>> {
    // INSERT … SELECT conditioned on the security epoch read BEFORE the
    // TOTP verification (codex review, #1005): a revocation (password
    // set/change, forget-all) bumps the epoch in the same transaction
    // that deletes the grants, so a challenge that was in flight across
    // the revocation inserts 0 rows instead of resurrecting device
    // trust. `None` = stale epoch; the route re-challenges.
    let username = crate::db::users::normalize_username(username);
    let id = uuid::Uuid::new_v4().to_string();
    let inserted = match db {
        ConfigDb::Sqlite(pool) => sqlx::query(
            "INSERT INTO user_mfa_grants
                (id, username, token_hash, session_binding, factor_confirmed_at,
                 mfa_verified_at, expires_at, created_at)
             SELECT ?, ?, ?, ?, ?, ?, ?, ?
               FROM user_mfa
              WHERE username = ? AND security_epoch = ?",
        )
        .bind(&id)
        .bind(&username)
        .bind(token_hash)
        .bind(session_binding)
        .bind(factor_confirmed_at)
        .bind(verified_at)
        .bind(expires_at)
        .bind(verified_at)
        .bind(&username)
        .bind(expected_epoch)
        .execute(pool)
        .await
        .with_context(|| format!("create MFA device grant for {username}"))?
        .rows_affected(),
        ConfigDb::Postgres(pool) => sqlx::query(
            "INSERT INTO user_mfa_grants
                (id, username, token_hash, session_binding, factor_confirmed_at,
                 mfa_verified_at, expires_at, created_at)
             SELECT $1, $2, $3, $4, $5, $6, $7, $8
               FROM user_mfa
              WHERE username = $9 AND security_epoch = $10",
        )
        .bind(&id)
        .bind(&username)
        .bind(token_hash)
        .bind(session_binding)
        .bind(factor_confirmed_at)
        .bind(verified_at)
        .bind(expires_at)
        .bind(verified_at)
        .bind(&username)
        .bind(expected_epoch)
        .execute(pool)
        .await
        .with_context(|| format!("create MFA device grant for {username}"))?
        .rows_affected(),
    };
    Ok((inserted == 1).then_some(id))
}

/// Fetch one unexpired grant. Token, user, factor and session checks remain
/// in the decision layer so every mismatch follows the same fail-closed path.
pub async fn fetch_valid(db: &ConfigDb, id: &str) -> Result<Option<GrantRow>> {
    let now = Utc::now();
    let row: Option<StoredRow> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as(
                "SELECT id, username, token_hash, session_binding,
                        factor_confirmed_at, mfa_verified_at, expires_at, created_at
                   FROM user_mfa_grants
                  WHERE id = ? AND expires_at > ?",
            )
            .bind(id)
            .bind(now)
            .fetch_optional(pool)
            .await
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as(
                "SELECT id, username, token_hash, session_binding,
                        factor_confirmed_at, mfa_verified_at, expires_at, created_at
                   FROM user_mfa_grants
                  WHERE id = $1 AND expires_at > $2",
            )
            .bind(id)
            .bind(now)
            .fetch_optional(pool)
            .await
        }
    }
    .context("fetch valid MFA device grant")?;
    Ok(row.map(
        |(
            id,
            username,
            token_hash,
            session_binding,
            factor_confirmed_at,
            mfa_verified_at,
            expires_at,
            created_at,
        )| GrantRow {
            id,
            username,
            token_hash,
            session_binding,
            factor_confirmed_at,
            mfa_verified_at,
            expires_at,
            created_at,
        },
    ))
}

/// Revoke every trusted-device grant and audit only when something changed.
pub async fn revoke_all(
    db: &ConfigDb,
    username: &str,
    actor: &str,
    cause: &str,
) -> Result<u64> {
    let username = crate::db::users::normalize_username(username);
    let now = Utc::now();
    let target = format!("user:{username}");
    let diff = serde_json::json!({ "cause": cause }).to_string();
    let changed = match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin MFA grant revocation")?;
            let changed = sqlx::query("DELETE FROM user_mfa_grants WHERE username = ?")
                .bind(&username)
                .execute(&mut *tx)
                .await
                .context("revoke all MFA device grants")?
                .rows_affected();
            // Epoch bump in the SAME transaction: an in-flight challenge
            // that read the old epoch can no longer issue a grant (#1005).
            sqlx::query(
                "UPDATE user_mfa SET security_epoch = security_epoch + 1 WHERE username = ?",
            )
            .bind(&username)
            .execute(&mut *tx)
            .await
            .context("bump MFA security epoch")?;
            if changed > 0 {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES (?, 'mfa.trusted_device.revoke', ?, ?, ?)",
                )
                .bind(actor)
                .bind(&target)
                .bind(&diff)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit MFA device revocation")?;
            }
            tx.commit().await.context("commit MFA grant revocation")?;
            changed
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin MFA grant revocation")?;
            let changed = sqlx::query("DELETE FROM user_mfa_grants WHERE username = $1")
                .bind(&username)
                .execute(&mut *tx)
                .await
                .context("revoke all MFA device grants")?
                .rows_affected();
            // Epoch bump in the SAME transaction: an in-flight challenge
            // that read the old epoch can no longer issue a grant (#1005).
            sqlx::query(
                "UPDATE user_mfa SET security_epoch = security_epoch + 1 WHERE username = $1",
            )
            .bind(&username)
            .execute(&mut *tx)
            .await
            .context("bump MFA security epoch")?;
            if changed > 0 {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES ($1, 'mfa.trusted_device.revoke', $2, $3, $4)",
                )
                .bind(actor)
                .bind(&target)
                .bind(&diff)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit MFA device revocation")?;
            }
            tx.commit().await.context("commit MFA grant revocation")?;
            changed
        }
    };
    Ok(changed)
}

/// Revoke one grant belonging to `username` and audit the explicit action.
pub async fn revoke_one(db: &ConfigDb, id: &str, username: &str, actor: &str) -> Result<bool> {
    let username = crate::db::users::normalize_username(username);
    let now = Utc::now();
    let target = format!("user:{username}");
    let diff = serde_json::json!({ "cause": "forget-device", "grant_id": id }).to_string();
    let changed = match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin MFA grant revocation")?;
            let changed = sqlx::query(
                "DELETE FROM user_mfa_grants WHERE id = ? AND username = ?",
            )
            .bind(id)
            .bind(&username)
            .execute(&mut *tx)
            .await
            .context("revoke one MFA device grant")?
            .rows_affected()
                == 1;
            if changed {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES (?, 'mfa.trusted_device.revoke', ?, ?, ?)",
                )
                .bind(actor)
                .bind(&target)
                .bind(&diff)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit one MFA device revocation")?;
            }
            tx.commit().await.context("commit MFA grant revocation")?;
            changed
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin MFA grant revocation")?;
            let changed = sqlx::query(
                "DELETE FROM user_mfa_grants WHERE id = $1 AND username = $2",
            )
            .bind(id)
            .bind(&username)
            .execute(&mut *tx)
            .await
            .context("revoke one MFA device grant")?
            .rows_affected()
                == 1;
            if changed {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES ($1, 'mfa.trusted_device.revoke', $2, $3, $4)",
                )
                .bind(actor)
                .bind(&target)
                .bind(&diff)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit one MFA device revocation")?;
            }
            tx.commit().await.context("commit MFA grant revocation")?;
            changed
        }
    };
    Ok(changed)
}

/// Silent cleanup for already-audited security events (password reset/change,
/// factor reset). Keeping this inside the caller's transaction makes the
/// security mutation and grant invalidation atomic without duplicate audit
/// noise.
pub(crate) async fn delete_all_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    username: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM user_mfa_grants WHERE username = ?")
        .bind(username)
        .execute(&mut **tx)
        .await
        .context("silently revoke MFA grants")?;
    // Epoch bump in the SAME transaction (#1005): a challenge in flight
    // across this revocation read the old epoch and can no longer issue
    // a grant (its conditional INSERT sees 0 matching rows).
    sqlx::query("UPDATE user_mfa SET security_epoch = security_epoch + 1 WHERE username = ?")
        .bind(username)
        .execute(&mut **tx)
        .await
        .context("bump MFA security epoch")?;
    Ok(())
}

pub(crate) async fn delete_all_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    username: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM user_mfa_grants WHERE username = $1")
        .bind(username)
        .execute(&mut **tx)
        .await
        .context("silently revoke MFA grants")?;
    // Epoch bump in the SAME transaction (#1005): a challenge in flight
    // across this revocation read the old epoch and can no longer issue
    // a grant (its conditional INSERT sees 0 matching rows).
    sqlx::query("UPDATE user_mfa SET security_epoch = security_epoch + 1 WHERE username = $1")
        .bind(username)
        .execute(&mut **tx)
        .await
        .context("bump MFA security epoch")?;
    Ok(())
}
