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


/// Why [`issue`] refused to mint a grant. Every case rolls the whole
/// transaction back — including a recovery-code consumption or TOTP-step
/// spend — so a finite code is never burned without a grant to show for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IssueRefusal {
    /// A revocation moved the security epoch mid-challenge.
    StaleEpoch,
    /// The recovery code raced another consumer.
    RecoverySpent,
    /// The TOTP step was already consumed (replay or a concurrent
    /// challenge won the race).
    Replayed,
}

/// Issue (or rotate) the trusted-device grant for one browser-session, one
/// transaction per dialect. UPSERT on `(username, session_binding)`: a
/// re-challenge from the same browser replaces its single row (new id +
/// token, so the old cookie dies), a stale cookie after a revocation just
/// gets a fresh grant, and two concurrent challenges from one browser can
/// never leave two live grants (#1005). Lock order is uniformly
/// user_mfa -> user_mfa_grants -> user_mfa_recovery, matching the
/// revocation/reset paths so a challenge can't deadlock them on Postgres.
/// The epoch-conditional source (`SELECT ... FROM user_mfa WHERE
/// security_epoch = ?`) makes a revocation that raced this issuance win.
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
            // Lock + validate user_mfa FIRST, on EVERY path (codex review
            // r9/r10). A no-op conditional UPDATE (not a SELECT) is used so
            // this transaction takes the WRITE lock immediately: SQLite's
            // deferred transaction would otherwise establish a read snapshot,
            // and a revocation committing before our later write would fail
            // with SQLITE_BUSY_SNAPSHOT (a 500) instead of a clean recheck.
            // rows_affected == 1 means the epoch still matched under the
            // lock; 0 means a revocation raced us → StaleEpoch rollback (the
            // recovery-code spend below never happens).
            let epoch_ok = sqlx::query(
                "UPDATE user_mfa SET security_epoch = security_epoch
                  WHERE username = ? AND security_epoch = ?",
            )
            .bind(&username)
            .bind(expected_epoch)
            .execute(&mut *tx)
            .await
            .context("lock+validate MFA epoch during issue")?
            .rows_affected();
            if epoch_ok != 1 {
                let _ = tx.rollback().await;
                return Ok(Err(IssueRefusal::StaleEpoch));
            }
            // Lock order — user_mfa (TOTP replay guard).
            if let Some(step) = consume_totp_step {
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
            // Lock order 2/3 — user_mfa_grants (UPSERT the browser's row).
            let inserted = sqlx::query(
                "INSERT INTO user_mfa_grants
                    (id, username, token_hash, session_binding, factor_confirmed_at,
                     security_epoch, mfa_verified_at, expires_at, created_at)
                 SELECT ?, ?, ?, ?, ?, ?, ?, ?, ?
                   FROM user_mfa
                  WHERE username = ? AND security_epoch = ?
                 ON CONFLICT(username, session_binding) DO UPDATE SET
                    id = excluded.id,
                    token_hash = excluded.token_hash,
                    factor_confirmed_at = excluded.factor_confirmed_at,
                    security_epoch = excluded.security_epoch,
                    mfa_verified_at = excluded.mfa_verified_at,
                    expires_at = excluded.expires_at,
                    created_at = excluded.created_at",
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
            // Lock order 3/3 — user_mfa_recovery (consume, if that was the proof).
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
            // Opportunistic retention (codex review r9): session-only MFA
            // mints a row per login, and expired rows are otherwise only
            // filtered at read time — never deleted until a revocation. Sweep
            // this user's expired grants on each issuance so the table can't
            // grow unboundedly on long-running installs.
            sqlx::query("DELETE FROM user_mfa_grants WHERE username = ? AND expires_at < ?")
                .bind(&username)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("sweep expired MFA grants")?;
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
            // Parent-before-child lock order (codex review r10): lock the
            // parent `users` row (KEY SHARE — blocks deletion, allows normal
            // updates) BEFORE user_mfa. The grant INSERT's FK check needs a
            // key-share lock on this parent anyway; user deletion locks
            // `users` then cascades to the children, so taking the parent
            // first here makes both paths lock parent→child and can't
            // deadlock.
            sqlx::query("SELECT 1 FROM users WHERE username = $1 FOR KEY SHARE")
                .bind(&username)
                .fetch_optional(&mut *tx)
                .await
                .context("lock parent user row during issue")?;
            // Lock + validate user_mfa on EVERY path (codex review r9/r10):
            // FOR UPDATE so a revocation's epoch bump can't slip between this
            // and the grant insert on the recovery path (no TOTP-step UPDATE
            // to take the lock). A raced revocation fails the epoch check
            // cleanly as StaleEpoch.
            let live_epoch: Option<(i64,)> = sqlx::query_as(
                "SELECT security_epoch FROM user_mfa WHERE username = $1 FOR UPDATE",
            )
            .bind(&username)
            .fetch_optional(&mut *tx)
            .await
            .context("lock MFA row during issue")?;
            if live_epoch.map(|(e,)| e) != Some(expected_epoch) {
                let _ = tx.rollback().await;
                return Ok(Err(IssueRefusal::StaleEpoch));
            }
            // Lock order — user_mfa (TOTP replay guard).
            if let Some(step) = consume_totp_step {
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
            // Lock order 2/3 — user_mfa_grants (UPSERT the browser's row).
            let inserted = sqlx::query(
                "INSERT INTO user_mfa_grants
                    (id, username, token_hash, session_binding, factor_confirmed_at,
                     security_epoch, mfa_verified_at, expires_at, created_at)
                 SELECT $1, $2, $3, $4, $5, $6, $7, $8, $9
                   FROM user_mfa
                  WHERE username = $10 AND security_epoch = $11
                 ON CONFLICT(username, session_binding) DO UPDATE SET
                    id = excluded.id,
                    token_hash = excluded.token_hash,
                    factor_confirmed_at = excluded.factor_confirmed_at,
                    security_epoch = excluded.security_epoch,
                    mfa_verified_at = excluded.mfa_verified_at,
                    expires_at = excluded.expires_at,
                    created_at = excluded.created_at",
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
            // Lock order 3/3 — user_mfa_recovery.
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
            // Opportunistic retention sweep (see the SQLite arm).
            sqlx::query("DELETE FROM user_mfa_grants WHERE username = $1 AND expires_at < $2")
                .bind(&username)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("sweep expired MFA grants")?;
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
