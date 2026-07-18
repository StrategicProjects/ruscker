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
    /// Epoch the factor had when this grant was issued — validated against
    /// the live user_mfa.security_epoch on every evaluate, so a grant that
    /// slipped past a racing revocation (pg MVCC) is never accepted.
    pub security_epoch: i64,
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
    i64,
    DateTime<Utc>,
    DateTime<Utc>,
    DateTime<Utc>,
);

#[allow(clippy::too_many_arguments)] // grant issuance is one atomic fact
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
                 security_epoch, mfa_verified_at, expires_at, created_at)
             SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?
               FROM user_mfa
              WHERE username = ? AND security_epoch = ?",
        )
        .bind(&id)
        .bind(&username)
        .bind(token_hash)
        .bind(session_binding)
        .bind(factor_confirmed_at)
        .bind(expected_epoch)
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
                 security_epoch, mfa_verified_at, expires_at, created_at)
             SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9
               FROM user_mfa
              WHERE username = $10 AND security_epoch = $11",
        )
        .bind(&id)
        .bind(&username)
        .bind(token_hash)
        .bind(session_binding)
        .bind(factor_confirmed_at)
        .bind(expected_epoch)
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
                        factor_confirmed_at, security_epoch, mfa_verified_at,
                        expires_at, created_at
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
                        factor_confirmed_at, security_epoch, mfa_verified_at,
                        expires_at, created_at
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
            security_epoch,
            mfa_verified_at,
            expires_at,
            created_at,
        )| GrantRow {
            id,
            username,
            token_hash,
            session_binding,
            factor_confirmed_at,
            security_epoch,
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
            // Lock ORDER: bump the epoch (locks user_mfa) BEFORE deleting
            // grants, matching issue()'s user_mfa-then-grants order — else a
            // re-challenge and a revocation grab the two rows in opposite
            // order and deadlock on Postgres (codex review, #1005). The bump
            // also stops an in-flight challenge that read the old epoch from
            // issuing.
            sqlx::query(
                "UPDATE user_mfa SET security_epoch = security_epoch + 1 WHERE username = ?",
            )
            .bind(&username)
            .execute(&mut *tx)
            .await
            .context("bump MFA security epoch")?;
            let changed = sqlx::query("DELETE FROM user_mfa_grants WHERE username = ?")
                .bind(&username)
                .execute(&mut *tx)
                .await
                .context("revoke all MFA device grants")?
                .rows_affected();
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
            // Lock ORDER (see the SQLite arm): user_mfa before grants.
            sqlx::query(
                "UPDATE user_mfa SET security_epoch = security_epoch + 1 WHERE username = $1",
            )
            .bind(&username)
            .execute(&mut *tx)
            .await
            .context("bump MFA security epoch")?;
            let changed = sqlx::query("DELETE FROM user_mfa_grants WHERE username = $1")
                .bind(&username)
                .execute(&mut *tx)
                .await
                .context("revoke all MFA device grants")?
                .rows_affected();
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


/// Why [`issue`] refused to mint a grant. Both cases roll the whole
/// transaction back — including a recovery-code consumption, so a finite
/// code is never spent without a grant to show for it (codex review).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueRefusal {
    /// A revocation moved the security epoch mid-challenge.
    StaleEpoch,
    /// The recovery code raced another consumer.
    RecoverySpent,
    /// The TOTP step was already consumed (replay or a concurrent
    /// challenge won the race).
    Replayed,
    /// The browser's previous grant (cookie rotation) was already replaced
    /// by a concurrent challenge, so issuing again would leave two live
    /// grants for one browser (codex review, #1005).
    Superseded,
}

/// Atomically: consume the matched recovery code (when the proof came from
/// one), retire the browser's PREVIOUS grant (cookie rotation — the old
/// cookie value must die with its replacement, else a copied cookie stays
/// valid forever), insert the new epoch-conditional grant, and audit — one
/// transaction per dialect. Any refusal rolls everything back.
#[allow(clippy::too_many_arguments)] // grant issuance is one atomic fact
pub async fn issue(
    db: &ConfigDb,
    username: &str,
    token_hash: &str,
    session_binding: &str,
    factor_confirmed_at: DateTime<Utc>,
    verified_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    expected_epoch: i64,
    replace_grant_id: Option<&str>,
    consume_recovery_id: Option<&str>,
    consume_totp_step: Option<i64>,
    audit_action: &str,
    actor: &str,
) -> Result<std::result::Result<String, IssueRefusal>> {
    let username = crate::db::users::normalize_username(username);
    let id = uuid::Uuid::new_v4().to_string();
    let target = format!("user:{username}");
    let now = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin MFA grant issue")?;
            if let Some(rid) = consume_recovery_id {
                let spent = sqlx::query(
                    "UPDATE user_mfa_recovery SET used_at = ?
                      WHERE id = ? AND username = ? AND used_at IS NULL",
                )
                .bind(now)
                .bind(rid)
                .bind(&username)
                .execute(&mut *tx)
                .await
                .context("consume recovery code during issue")?
                .rows_affected();
                if spent != 1 {
                    let _ = tx.rollback().await;
                    return Ok(Err(IssueRefusal::RecoverySpent));
                }
            }
            if let Some(step) = consume_totp_step {
                // Replay guard INSIDE the issuance transaction (codex
                // review r5): a refused issuance must not burn the
                // current 30s code — the step consumption rolls back
                // with everything else.
                let fresh = sqlx::query(
                    "UPDATE user_mfa SET last_used_step = ?
                      WHERE username = ?
                        AND (last_used_step IS NULL OR last_used_step < ?)",
                )
                .bind(step)
                .bind(&username)
                .bind(step)
                .execute(&mut *tx)
                .await
                .context("consume TOTP step during issue")?
                .rows_affected();
                if fresh != 1 {
                    let _ = tx.rollback().await;
                    return Ok(Err(IssueRefusal::Replayed));
                }
            }
            if let Some(gid) = replace_grant_id {
                // Rotation must be exclusive (codex review, #1005): if the
                // browser's old grant is already gone, a concurrent
                // challenge from the same browser already rotated it —
                // issuing again would leave TWO live grants for one cookie.
                // Refuse instead; the user simply re-challenges.
                let retired = sqlx::query("DELETE FROM user_mfa_grants WHERE id = ? AND username = ?")
                    .bind(gid)
                    .bind(&username)
                    .execute(&mut *tx)
                    .await
                    .context("retire previous device grant")?
                    .rows_affected();
                if retired != 1 {
                    let _ = tx.rollback().await;
                    return Ok(Err(IssueRefusal::Superseded));
                }
            }
            let inserted = sqlx::query(
                "INSERT INTO user_mfa_grants
                    (id, username, token_hash, session_binding, factor_confirmed_at,
                     security_epoch, mfa_verified_at, expires_at, created_at)
                 SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?
                   FROM user_mfa
                  WHERE username = ? AND security_epoch = ?",
            )
            .bind(&id)
            .bind(&username)
            .bind(token_hash)
            .bind(session_binding)
            .bind(factor_confirmed_at)
            .bind(expected_epoch)
            .bind(verified_at)
            .bind(expires_at)
            .bind(verified_at)
            .bind(&username)
            .bind(expected_epoch)
            .execute(&mut *tx)
            .await
            .context("issue MFA device grant")?
            .rows_affected();
            if inserted != 1 {
                let _ = tx.rollback().await;
                return Ok(Err(IssueRefusal::StaleEpoch));
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, ?, ?, NULL, ?)",
            )
            .bind(actor)
            .bind(audit_action)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit MFA proof")?;
            tx.commit().await.context("commit MFA grant issue")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin MFA grant issue")?;
            if let Some(rid) = consume_recovery_id {
                let spent = sqlx::query(
                    "UPDATE user_mfa_recovery SET used_at = $1
                      WHERE id = $2 AND username = $3 AND used_at IS NULL",
                )
                .bind(now)
                .bind(rid)
                .bind(&username)
                .execute(&mut *tx)
                .await
                .context("consume recovery code during issue")?
                .rows_affected();
                if spent != 1 {
                    let _ = tx.rollback().await;
                    return Ok(Err(IssueRefusal::RecoverySpent));
                }
            }
            if let Some(step) = consume_totp_step {
                // Replay guard INSIDE the issuance transaction (codex
                // review r5): a refused issuance must not burn the
                // current 30s code — the step consumption rolls back
                // with everything else.
                let fresh = sqlx::query(
                    "UPDATE user_mfa SET last_used_step = $1
                      WHERE username = $2
                        AND (last_used_step IS NULL OR last_used_step < $3)",
                )
                .bind(step)
                .bind(&username)
                .bind(step)
                .execute(&mut *tx)
                .await
                .context("consume TOTP step during issue")?
                .rows_affected();
                if fresh != 1 {
                    let _ = tx.rollback().await;
                    return Ok(Err(IssueRefusal::Replayed));
                }
            }
            if let Some(gid) = replace_grant_id {
                // Rotation must be exclusive (see the SQLite arm).
                let retired = sqlx::query("DELETE FROM user_mfa_grants WHERE id = $1 AND username = $2")
                    .bind(gid)
                    .bind(&username)
                    .execute(&mut *tx)
                    .await
                    .context("retire previous device grant")?
                    .rows_affected();
                if retired != 1 {
                    let _ = tx.rollback().await;
                    return Ok(Err(IssueRefusal::Superseded));
                }
            }
            let inserted = sqlx::query(
                "INSERT INTO user_mfa_grants
                    (id, username, token_hash, session_binding, factor_confirmed_at,
                     security_epoch, mfa_verified_at, expires_at, created_at)
                 SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9
                   FROM user_mfa
                  WHERE username = $10 AND security_epoch = $11",
            )
            .bind(&id)
            .bind(&username)
            .bind(token_hash)
            .bind(session_binding)
            .bind(factor_confirmed_at)
            .bind(expected_epoch)
            .bind(verified_at)
            .bind(expires_at)
            .bind(verified_at)
            .bind(&username)
            .bind(expected_epoch)
            .execute(&mut *tx)
            .await
            .context("issue MFA device grant")?
            .rows_affected();
            if inserted != 1 {
                let _ = tx.rollback().await;
                return Ok(Err(IssueRefusal::StaleEpoch));
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, $2, $3, NULL, $4)",
            )
            .bind(actor)
            .bind(audit_action)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit MFA proof")?;
            tx.commit().await.context("commit MFA grant issue")?;
        }
    }
    Ok(Ok(id))
}

/// Silent cleanup for already-audited security events (password reset/change,
/// factor reset). Keeping this inside the caller's transaction makes the
/// security mutation and grant invalidation atomic without duplicate audit
/// noise.
pub(crate) async fn delete_all_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    username: &str,
) -> Result<()> {
    // Lock ORDER: user_mfa (epoch bump) before grants, matching issue() —
    // opposite order deadlocks with a concurrent re-challenge on Postgres
    // (codex review, #1005). The bump also stops an in-flight challenge
    // that read the old epoch from issuing.
    sqlx::query("UPDATE user_mfa SET security_epoch = security_epoch + 1 WHERE username = ?")
        .bind(username)
        .execute(&mut **tx)
        .await
        .context("bump MFA security epoch")?;
    sqlx::query("DELETE FROM user_mfa_grants WHERE username = ?")
        .bind(username)
        .execute(&mut **tx)
        .await
        .context("silently revoke MFA grants")?;
    Ok(())
}

pub(crate) async fn delete_all_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    username: &str,
) -> Result<()> {
    // Lock ORDER (see delete_all_sqlite): user_mfa before grants.
    sqlx::query("UPDATE user_mfa SET security_epoch = security_epoch + 1 WHERE username = $1")
        .bind(username)
        .execute(&mut **tx)
        .await
        .context("bump MFA security epoch")?;
    sqlx::query("DELETE FROM user_mfa_grants WHERE username = $1")
        .bind(username)
        .execute(&mut **tx)
        .await
        .context("silently revoke MFA grants")?;
    Ok(())
}
