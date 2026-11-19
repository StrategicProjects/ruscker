//! Authorization codes of the OIDC provider (slice 3 of
//! `docs/adr/0005-oidc-provider.md`).
//!
//! A code lives for [`CODE_TTL_SECS`] and can be redeemed **once**. Both
//! properties are enforced here, at the storage layer, so no caller can
//! forget them:
//!
//! * only `SHA-256(code)` is stored — the table never holds anything an
//!   attacker with read access could redeem;
//! * [`consume`] claims the row with a single conditional `UPDATE`, so
//!   of two simultaneous redemptions exactly one succeeds.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Duration, Utc};
use ring::rand as ring_rand;

use crate::db::ConfigDb;

/// Lifetime of an authorization code. The browser only has to follow one
/// redirect and the client make one back-channel call, so a minute is
/// generous; RFC 6749 recommends at most ten.
pub const CODE_TTL_SECS: i64 = 60;

/// Everything bound to a code when it is issued.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grant {
    pub client_id: String,
    pub user_id: String,
    pub username: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub nonce: Option<String>,
    /// Granted scopes, space-separated.
    pub scope: String,
    /// The USER's address and browser, seen when the code was issued.
    /// The redemption comes from the client system's server, so this is
    /// the only place the audit row can learn them from.
    pub client_ip: Option<String>,
    pub user_agent: Option<String>,
    /// The second factor was proved in the session that obtained this
    /// code. The token endpoint refuses a code without it when the client
    /// requires MFA by the time the code is redeemed.
    pub mfa_proved: bool,
}

/// A redeemed code: the grant plus when the code was issued. That is
/// NOT when the user authenticated — the session behind it may be many
/// hours old — so it must not be reported to clients as `auth_time`.
#[derive(Debug, Clone)]
pub struct Redeemed {
    pub grant: Grant,
    pub issued_at: DateTime<Utc>,
}

/// Lowercase-hex SHA-256 of a code. Unsalted on purpose: the code is 256
/// random bits (nothing to brute-force) and the lookup must be by value.
fn hash(code: &str) -> String {
    hex::encode(ring::digest::digest(&ring::digest::SHA256, code.as_bytes()).as_ref())
}

/// Issue a fresh code for `grant` and return it. The cleartext exists
/// only in the return value (and then in the redirect to the client).
///
/// Also deletes codes that expired more than ten minutes ago, so the
/// table stays small however often a signed-in user comes through. The
/// short grace only covers clock skew between HA nodes; a spent or
/// expired code is refused whether or not its row still exists.
pub async fn issue(db: &ConfigDb, grant: &Grant) -> Result<String> {
    let rng = ring_rand::SystemRandom::new();
    let mut raw = [0u8; 32];
    ring_rand::SecureRandom::fill(&rng, &mut raw)
        .map_err(|_| anyhow!("generate authorization code"))?;
    let code = hex::encode(raw);
    let code_hash = hash(&code);
    let now = Utc::now();
    let expires = now + Duration::seconds(CODE_TTL_SECS);
    let stale = now - Duration::minutes(10);

    match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query("DELETE FROM oidc_auth_codes WHERE expires_at < ?")
                .bind(stale)
                .execute(pool)
                .await
                .context("purge old authorization codes")?;
            sqlx::query(
                "INSERT INTO oidc_auth_codes
                   (code_hash, client_id, user_id, username, redirect_uri,
                    code_challenge, nonce, scope, client_ip, user_agent, mfa_proved, created_at, expires_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&code_hash)
            .bind(&grant.client_id)
            .bind(&grant.user_id)
            .bind(&grant.username)
            .bind(&grant.redirect_uri)
            .bind(&grant.code_challenge)
            .bind(&grant.nonce)
            .bind(&grant.scope)
            .bind(&grant.client_ip)
            .bind(&grant.user_agent)
            .bind(grant.mfa_proved)
            .bind(now)
            .bind(expires)
            .execute(pool)
            .await
            .context("store authorization code")?;
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query("DELETE FROM oidc_auth_codes WHERE expires_at < $1")
                .bind(stale)
                .execute(pool)
                .await
                .context("purge old authorization codes")?;
            sqlx::query(
                "INSERT INTO oidc_auth_codes
                   (code_hash, client_id, user_id, username, redirect_uri,
                    code_challenge, nonce, scope, client_ip, user_agent, mfa_proved, created_at, expires_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
            )
            .bind(&code_hash)
            .bind(&grant.client_id)
            .bind(&grant.user_id)
            .bind(&grant.username)
            .bind(&grant.redirect_uri)
            .bind(&grant.code_challenge)
            .bind(&grant.nonce)
            .bind(&grant.scope)
            .bind(&grant.client_ip)
            .bind(&grant.user_agent)
            .bind(grant.mfa_proved)
            .bind(now)
            .bind(expires)
            .execute(pool)
            .await
            .context("store authorization code")?;
        }
    }
    Ok(code)
}

type Row = (
    String,
    String,
    String,
    String,
    String,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    bool,
    DateTime<Utc>,
    DateTime<Utc>,
);

/// Redeem `code`: returns its grant the FIRST time it is presented
/// before expiry, and `None` in every other case (unknown, already
/// used, expired).
///
/// The claim is one statement — `UPDATE … WHERE consumed_at IS NULL …
/// RETURNING` — so two concurrent redemptions cannot both read "not
/// used yet". An expired code is still marked consumed: presenting it
/// burns it either way.
pub async fn consume(db: &ConfigDb, code: &str) -> Result<Option<Redeemed>> {
    let code_hash = hash(code);
    let now = Utc::now();
    let row: Option<Row> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as(
                "UPDATE oidc_auth_codes SET consumed_at = ?
                  WHERE code_hash = ? AND consumed_at IS NULL
              RETURNING client_id, user_id, username, redirect_uri,
                        code_challenge, nonce, scope, client_ip, user_agent,
                        mfa_proved, created_at, expires_at",
            )
            .bind(now)
            .bind(&code_hash)
            .fetch_optional(pool)
            .await
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as(
                "UPDATE oidc_auth_codes SET consumed_at = $1
                  WHERE code_hash = $2 AND consumed_at IS NULL
              RETURNING client_id, user_id, username, redirect_uri,
                        code_challenge, nonce, scope, client_ip, user_agent,
                        mfa_proved, created_at, expires_at",
            )
            .bind(now)
            .bind(&code_hash)
            .fetch_optional(pool)
            .await
        }
    }
    .context("redeem authorization code")?;

    let Some(r) = row else {
        return Ok(None);
    };
    // Compared here rather than in SQL: the two dialects store the
    // instant differently (TEXT vs TIMESTAMPTZ), chrono compares it the
    // same way on both.
    if r.11 <= now {
        return Ok(None);
    }
    Ok(Some(Redeemed {
        grant: Grant {
            client_id: r.0,
            user_id: r.1,
            username: r.2,
            redirect_uri: r.3,
            code_challenge: r.4,
            nonce: r.5,
            scope: r.6,
            client_ip: r.7,
            user_agent: r.8,
            mfa_proved: r.9,
        },
        issued_at: r.10,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_db() -> ConfigDb {
        ConfigDb::Sqlite(crate::db::open_memory().await.expect("open in-memory"))
    }

    fn grant() -> Grant {
        Grant {
            client_id: "radar".into(),
            user_id: "u-1".into(),
            username: "ana".into(),
            redirect_uri: "https://radar.example/cb".into(),
            code_challenge: "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM".into(),
            nonce: Some("n-1".into()),
            scope: "openid email".into(),
            client_ip: Some("203.0.113.9".into()),
            user_agent: Some("Mozilla/5.0 test".into()),
            mfa_proved: true,
        }
    }

    /// Count rows without going through the API under test.
    async fn rows(db: &ConfigDb) -> i64 {
        let ConfigDb::Sqlite(pool) = db else { unreachable!() };
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM oidc_auth_codes")
            .fetch_one(pool)
            .await
            .unwrap();
        n
    }

    #[tokio::test]
    async fn a_code_redeems_exactly_once() {
        let db = mem_db().await;
        let code = issue(&db, &grant()).await.unwrap();
        assert_eq!(code.len(), 64);

        let got = consume(&db, &code).await.unwrap().expect("first redemption");
        assert_eq!(got.grant, grant());
        assert!(consume(&db, &code).await.unwrap().is_none(), "replay refused");
        assert!(consume(&db, "not-a-code").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn only_the_hash_is_stored() {
        let db = mem_db().await;
        let code = issue(&db, &grant()).await.unwrap();
        let ConfigDb::Sqlite(pool) = &db else { unreachable!() };
        let (stored,): (String,) = sqlx::query_as("SELECT code_hash FROM oidc_auth_codes")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_ne!(stored, code);
        assert_eq!(stored, hash(&code));
    }

    #[tokio::test]
    async fn an_expired_code_is_refused_and_burned() {
        let db = mem_db().await;
        let code = issue(&db, &grant()).await.unwrap();
        let ConfigDb::Sqlite(pool) = &db else { unreachable!() };
        sqlx::query("UPDATE oidc_auth_codes SET expires_at = ?")
            .bind(Utc::now() - Duration::seconds(1))
            .execute(pool)
            .await
            .unwrap();
        assert!(consume(&db, &code).await.unwrap().is_none());
        // Even if the clock were rolled back, it is spent.
        sqlx::query("UPDATE oidc_auth_codes SET expires_at = ?")
            .bind(Utc::now() + Duration::seconds(60))
            .execute(pool)
            .await
            .unwrap();
        assert!(consume(&db, &code).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn concurrent_redemptions_have_one_winner() {
        let db = mem_db().await;
        let code = issue(&db, &grant()).await.unwrap();
        let mut tasks = Vec::new();
        for _ in 0..12 {
            let (db, code) = (db.clone(), code.clone());
            tasks.push(tokio::spawn(async move { consume(&db, &code).await.unwrap().is_some() }));
        }
        let mut winners = 0;
        for t in tasks {
            if t.await.unwrap() {
                winners += 1;
            }
        }
        assert_eq!(winners, 1);
    }

    #[tokio::test]
    async fn issuing_purges_long_expired_codes_only() {
        let db = mem_db().await;
        issue(&db, &grant()).await.unwrap();
        issue(&db, &grant()).await.unwrap();
        let ConfigDb::Sqlite(pool) = &db else { unreachable!() };
        // One expired an hour ago, one a minute ago.
        sqlx::query(
            "UPDATE oidc_auth_codes SET expires_at = ?
              WHERE code_hash = (SELECT MIN(code_hash) FROM oidc_auth_codes)",
        )
        .bind(Utc::now() - Duration::hours(1))
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            "UPDATE oidc_auth_codes SET expires_at = ?
              WHERE code_hash = (SELECT MAX(code_hash) FROM oidc_auth_codes)",
        )
        .bind(Utc::now() - Duration::seconds(60))
        .execute(pool)
        .await
        .unwrap();
        issue(&db, &grant()).await.unwrap();
        assert_eq!(rows(&db).await, 2, "the hour-old row is gone, the recent one stays");
    }

    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn codes_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pg = crate::db::open_pg(&url).await.unwrap();
        sqlx::query("DELETE FROM oidc_auth_codes").execute(&pg).await.unwrap();
        let db = ConfigDb::Postgres(pg);

        let code = issue(&db, &grant()).await.unwrap();
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let (db, code) = (db.clone(), code.clone());
            tasks.push(tokio::spawn(async move { consume(&db, &code).await.unwrap() }));
        }
        let mut winners = Vec::new();
        for t in tasks {
            if let Some(r) = t.await.unwrap() {
                winners.push(r);
            }
        }
        assert_eq!(winners.len(), 1);
        assert_eq!(winners[0].grant, grant());
    }
}
