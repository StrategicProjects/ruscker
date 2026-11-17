//! Storage for the OIDC provider's signing keys (slice 2 of
//! `docs/adr/0005-oidc-provider.md`). Pure persistence: the cryptography
//! lives in [`crate::oidc`], which is the only caller.
//!
//! The private key arrives here already encrypted; this module never
//! sees it in the clear.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::db::ConfigDb;

/// One stored key. `pkcs8_enc` + `nonce` are the AES-256-GCM envelope of
/// the private key; `public_jwk` is the public half as JSON.
#[derive(Debug, Clone)]
pub struct StoredKey {
    pub kid: String,
    pub alg: String,
    pub pkcs8_enc: Vec<u8>,
    pub nonce: Vec<u8>,
    pub public_jwk: String,
    pub created_at: DateTime<Utc>,
}

type Row = (String, String, Vec<u8>, Vec<u8>, String, DateTime<Utc>);

fn from_row(r: Row) -> StoredKey {
    StoredKey {
        kid: r.0,
        alg: r.1,
        pkcs8_enc: r.2,
        nonce: r.3,
        public_jwk: r.4,
        created_at: r.5,
    }
}

/// Advisory-lock key serializing first-key creation on Postgres. Any
/// constant works; it only has to differ from the scaler leader lock.
const PG_FIRST_KEY_LOCK: i64 = 0x5255_5343_4f49_4443; // "RUSCOIDC"

/// Store `key` as the FIRST signing key — only if the table is empty.
/// Returns whether a row was written.
///
/// "Check that none exists, then insert" must be one atomic step:
/// otherwise N concurrent first requests (or N nodes of an HA
/// deployment starting together) each create a key, and a caller that
/// lost the race could go on to sign with a key its neighbours'
/// already-served JWKS does not contain.
///
/// * SQLite has a single writer, so `INSERT … WHERE NOT EXISTS` is
///   atomic by itself.
/// * Postgres under READ COMMITTED would let two transactions both see
///   an empty table, so a transaction-scoped advisory lock makes the
///   loser wait for the winner's commit and then see its row.
pub async fn insert_first(db: &ConfigDb, key: &StoredKey) -> Result<bool> {
    let target = format!("oidc-key:{}", key.kid);
    let inserted = match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin oidc key tx")?;
            let n = sqlx::query(
                "INSERT INTO oidc_keys (kid, alg, pkcs8_enc, nonce, public_jwk, created_at)
                 SELECT ?, ?, ?, ?, ?, ?
                  WHERE NOT EXISTS (SELECT 1 FROM oidc_keys)",
            )
            .bind(&key.kid)
            .bind(&key.alg)
            .bind(&key.pkcs8_enc)
            .bind(&key.nonce)
            .bind(&key.public_jwk)
            .bind(key.created_at)
            .execute(&mut *tx)
            .await
            .context("insert oidc key")?
            .rows_affected();
            if n == 1 {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES (NULL, 'oidc-key.create', ?, NULL, ?)",
                )
                .bind(&target)
                .bind(key.created_at)
                .execute(&mut *tx)
                .await
                .context("audit oidc key create")?;
            }
            tx.commit().await.context("commit oidc key tx")?;
            n == 1
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin oidc key tx")?;
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(PG_FIRST_KEY_LOCK)
                .execute(&mut *tx)
                .await
                .context("lock oidc key creation")?;
            let n = sqlx::query(
                "INSERT INTO oidc_keys (kid, alg, pkcs8_enc, nonce, public_jwk, created_at)
                 SELECT $1, $2, $3, $4, $5, $6
                  WHERE NOT EXISTS (SELECT 1 FROM oidc_keys)",
            )
            .bind(&key.kid)
            .bind(&key.alg)
            .bind(&key.pkcs8_enc)
            .bind(&key.nonce)
            .bind(&key.public_jwk)
            .bind(key.created_at)
            .execute(&mut *tx)
            .await
            .context("insert oidc key")?
            .rows_affected();
            if n == 1 {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES (NULL, 'oidc-key.create', $1, NULL, $2)",
                )
                .bind(&target)
                .bind(key.created_at)
                .execute(&mut *tx)
                .await
                .context("audit oidc key create")?;
            }
            tx.commit().await.context("commit oidc key tx")?;
            n == 1
        }
    };
    Ok(inserted)
}

/// Every stored key, newest first. The first one is the signing key:
/// ordering by `(created_at, kid)` makes that choice the same on every
/// node, even if two of them generated a key in the same instant.
pub async fn list_newest_first(db: &ConfigDb) -> Result<Vec<StoredKey>> {
    let sql = "SELECT kid, alg, pkcs8_enc, nonce, public_jwk, created_at
                 FROM oidc_keys
                ORDER BY created_at DESC, kid DESC";
    let rows: Vec<Row> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    }
    .context("list oidc keys")?;
    Ok(rows.into_iter().map(from_row).collect())
}
