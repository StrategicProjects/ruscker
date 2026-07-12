//! First-install seed: copy the built-in logo SVGs into the Media library
//! (#433) so they sit in the one image group, are pickable like uploads,
//! and are deletable. The bundled `/assets/showcase|brand/*` routes still
//! serve the originals for the showcase cards; these are independent DB
//! copies served at `/assets/img/<filename>`.
//!
//! Idempotent: a `config_meta` marker (`builtin_logos.seeded`) records the
//! seed so it runs once. Built-ins an operator deleted stay deleted.

use anyhow::{Context, Result};
use chrono::Utc;

use crate::db::ConfigDb;

const SEEDED_KEY: &str = "builtin_logos.seeded";

/// Seed the built-in logos into `images` if never done on this DB. Called
/// from [`crate::db::open`] / `open_pg` right after migrations.
pub async fn seed_if_unseeded(db: &ConfigDb) -> Result<()> {
    if already_seeded(db).await? {
        return Ok(());
    }
    for (filename, bytes) in crate::routes::assets::BUILTIN_LOGO_ASSETS {
        // Don't clobber an operator's upload of the same name.
        if super::images::fetch_by_filename(db, filename)
            .await?
            .is_some()
        {
            continue;
        }
        let processed = match crate::images::process_upload_async(
            filename.to_string(),
            Some("image/svg+xml".to_string()),
            bytes.to_vec(),
        )
        .await
        {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!(error = ?err, filename, "skip built-in logo seed");
                continue;
            }
        };
        super::images::insert(db, processed, None)
            .await
            .with_context(|| format!("seed built-in logo {filename}"))?;
    }
    mark_seeded(db).await?;
    Ok(())
}

async fn already_seeded(db: &ConfigDb) -> Result<bool> {
    let row: Option<(String,)> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as("SELECT value_json FROM config_meta WHERE key = ?")
                .bind(SEEDED_KEY)
                .fetch_optional(pool)
                .await
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as("SELECT value_json FROM config_meta WHERE key = $1")
                .bind(SEEDED_KEY)
                .fetch_optional(pool)
                .await
        }
    }
    .context("check builtin_logos seed marker")?;
    Ok(row.is_some())
}

async fn mark_seeded(db: &ConfigDb) -> Result<()> {
    let now = Utc::now();
    let value_json = serde_json::json!({ "seeded_at": now }).to_string();
    match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query(
                "INSERT OR REPLACE INTO config_meta (key, value_json, updated_at)
                 VALUES (?, ?, ?)",
            )
            .bind(SEEDED_KEY)
            .bind(&value_json)
            .bind(now)
            .execute(pool)
            .await
            .context("write builtin_logos seed marker (sqlite)")?;
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO config_meta (key, value_json, updated_at)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (key) DO UPDATE SET value_json = $2, updated_at = $3",
            )
            .bind(SEEDED_KEY)
            .bind(&value_json)
            .bind(now)
            .execute(pool)
            .await
            .context("write builtin_logos seed marker (postgres)")?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn seeds_builtin_logos_once() {
        let pool = crate::db::open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool);
        seed_if_unseeded(&db).await.unwrap();
        let n = super::super::images::list_all(&db).await.unwrap().len();
        assert_eq!(
            n,
            crate::routes::assets::BUILTIN_LOGO_ASSETS.len(),
            "all built-ins seeded into the library"
        );
        // Idempotent: a second run adds nothing.
        seed_if_unseeded(&db).await.unwrap();
        assert_eq!(super::super::images::list_all(&db).await.unwrap().len(), n);
    }
}
