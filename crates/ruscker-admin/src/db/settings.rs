//! Generic key/value settings store (#930).
//!
//! For small operator settings that don't deserve their own table.
//! First user: the alert webhook URL (`alert.webhook-url`). Values are
//! plain text; callers own any encoding. Writes audit under
//! `settings.update` with the key (never the value — a webhook URL can
//! embed a token).

use super::ConfigDb;
use anyhow::{Context, Result};
use chrono::Utc;

/// Key for the alert-notification webhook URL (#930). Empty/absent ⇒
/// alert delivery is off.
pub const ALERT_WEBHOOK_URL: &str = "alert.webhook-url";

/// Read one setting. `Ok(None)` when the key was never set.
pub async fn get(db: &ConfigDb, key: &str) -> Result<Option<String>> {
    let row: Option<(String,)> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as("SELECT value FROM settings WHERE key = ?")
                .bind(key)
                .fetch_optional(pool)
                .await
                .with_context(|| format!("get setting {key} (sqlite)"))?
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as("SELECT value FROM settings WHERE key = $1")
                .bind(key)
                .fetch_optional(pool)
                .await
                .with_context(|| format!("get setting {key} (postgres)"))?
        }
    };
    Ok(row.map(|(v,)| v))
}

/// Upsert one setting and write the audit row in the same transaction.
/// The audit diff records only the key — values may be sensitive.
pub async fn set(db: &ConfigDb, key: &str, value: &str, actor: Option<&str>) -> Result<()> {
    let now = Utc::now();
    let target = format!("setting:{key}");
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin setting update")?;
            sqlx::query(
                "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value,
                                                updated_at = excluded.updated_at",
            )
            .bind(key)
            .bind(value)
            .bind(now)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("set setting {key} (sqlite)"))?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'settings.update', ?, NULL, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit setting update")?;
            tx.commit().await.context("commit setting update")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin setting update")?;
            sqlx::query(
                "INSERT INTO settings (key, value, updated_at) VALUES ($1, $2, $3)
                 ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value,
                                                 updated_at = EXCLUDED.updated_at",
            )
            .bind(key)
            .bind(value)
            .bind(now)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("set setting {key} (postgres)"))?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'settings.update', $2, NULL, $3)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit setting update")?;
            tx.commit().await.context("commit setting update")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_db() -> ConfigDb {
        ConfigDb::Sqlite(crate::db::open_memory().await.expect("open in-memory"))
    }

    #[tokio::test]
    async fn get_set_roundtrip_and_overwrite() {
        let db = mem_db().await;
        assert_eq!(get(&db, ALERT_WEBHOOK_URL).await.unwrap(), None);

        set(&db, ALERT_WEBHOOK_URL, "https://hooks.example/x", Some("root"))
            .await
            .unwrap();
        assert_eq!(
            get(&db, ALERT_WEBHOOK_URL).await.unwrap().as_deref(),
            Some("https://hooks.example/x")
        );

        // Overwrite wins; audit rows accumulate but never carry the value.
        set(&db, ALERT_WEBHOOK_URL, "", Some("root")).await.unwrap();
        assert_eq!(get(&db, ALERT_WEBHOOK_URL).await.unwrap().as_deref(), Some(""));

        let audits = crate::db::audit::list(&db, &crate::db::audit::AuditFilter::new())
            .await
            .unwrap();
        let ours: Vec<_> = audits
            .iter()
            .filter(|e| e.action == "settings.update")
            .collect();
        assert_eq!(ours.len(), 2);
        assert!(ours.iter().all(|e| e.diff.is_none()));
    }

    // The dual-dialect UPSERT against a real Postgres. Gated on
    // `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn settings_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pg = crate::db::open_pg(&url).await.unwrap();
        sqlx::query("DELETE FROM settings").execute(&pg).await.unwrap();
        let db = ConfigDb::Postgres(pg);

        set(&db, ALERT_WEBHOOK_URL, "https://hooks.example/y", None)
            .await
            .unwrap();
        set(&db, ALERT_WEBHOOK_URL, "https://hooks.example/z", None)
            .await
            .unwrap();
        assert_eq!(
            get(&db, ALERT_WEBHOOK_URL).await.unwrap().as_deref(),
            Some("https://hooks.example/z")
        );
    }
}
