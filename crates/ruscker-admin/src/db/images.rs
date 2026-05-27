//! Image library repository — uploads and reads against the
//! `images` table.
//!
//! Storage choice: blobs live in SQLite. The expected scale is
//! tens to low hundreds of images per install at ~10-100 KB each
//! (WebP-encoded), totaling a few MB; well within SQLite's
//! comfort zone and gone-with-the-DB-file in backups.
//!
//! Switching to object storage is a Phase 2.5+ migration when an
//! install crosses ~1 GB of images or someone wants to share a
//! gallery across multiple Ruscker instances.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use uuid::Uuid;

use crate::db::ConfigDb;
use crate::images::Processed;

/// Soft limit warned about (not enforced) on encoded image size.
/// Anything above this is technically allowed but produces a
/// startup warning so operators see oversize uploads pile up.
pub const SOFT_SIZE_LIMIT_BYTES: i64 = 500 * 1024;

/// Row of `images` minus the blob — for the gallery list.
#[derive(Debug, Clone)]
pub struct ImageMeta {
    pub id: String,
    pub filename: String,
    pub mime_type: String,
    pub size_bytes: i64,
    pub width: Option<i64>,
    pub height: Option<i64>,
    pub uploaded_at: DateTime<Utc>,
}

/// Insert a processed image. Returns the freshly assigned id.
/// Idempotent on filename: re-uploading the same filename
/// **replaces** the existing row (and the prior bytes are gone —
/// we don't keep image history, only spec history).
pub async fn insert(db: &ConfigDb, processed: Processed, actor: Option<&str>) -> Result<String> {
    let now = Utc::now();
    let id = Uuid::new_v4().to_string();
    let target = format!("image:{id}");
    let diff = serde_json::to_string(&serde_json::json!({
        "filename": processed.filename,
        "mime": processed.mime_type,
        "size": processed.bytes.len(),
    }))?;
    let size = processed.bytes.len() as i64;
    let width = processed.width.map(|n| n as i64);
    let height = processed.height.map(|n| n as i64);

    // Replace any prior row with the same filename so the new upload
    // wins (we don't keep image history, only spec history).
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin image insert tx")?;
            sqlx::query("DELETE FROM images WHERE filename = ?")
                .bind(&processed.filename)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("delete prior image {}", processed.filename))?;
            sqlx::query(
                "INSERT INTO images
                   (id, filename, mime_type, size_bytes, blob, width, height, uploaded_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&processed.filename)
            .bind(&processed.mime_type)
            .bind(size)
            .bind(&processed.bytes)
            .bind(width)
            .bind(height)
            .bind(now)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("insert image {}", processed.filename))?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'image.upload', ?, ?, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit image.upload")?;
            tx.commit().await.context("commit image insert")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin image insert tx")?;
            sqlx::query("DELETE FROM images WHERE filename = $1")
                .bind(&processed.filename)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("delete prior image {}", processed.filename))?;
            sqlx::query(
                "INSERT INTO images
                   (id, filename, mime_type, size_bytes, blob, width, height, uploaded_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
            )
            .bind(&id)
            .bind(&processed.filename)
            .bind(&processed.mime_type)
            .bind(size)
            .bind(&processed.bytes)
            .bind(width)
            .bind(height)
            .bind(now)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("insert image {}", processed.filename))?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'image.upload', $2, $3, $4)",
            )
            .bind(actor)
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit image.upload")?;
            tx.commit().await.context("commit image insert")?;
        }
    }
    Ok(id)
}

/// Fetch the bytes + MIME of an image by **filename**. This is the
/// public lookup hit by `GET /assets/img/<filename>`.
pub async fn fetch_by_filename(
    db: &ConfigDb,
    filename: &str,
) -> Result<Option<(String, Vec<u8>)>> {
    let row: Option<(String, Vec<u8>)> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as("SELECT mime_type, blob FROM images WHERE filename = ?")
                .bind(filename)
                .fetch_optional(pool)
                .await
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as("SELECT mime_type, blob FROM images WHERE filename = $1")
                .bind(filename)
                .fetch_optional(pool)
                .await
        }
    }
    .with_context(|| format!("fetch image {filename}"))?;
    Ok(row)
}

/// Gallery listing — every uploaded image, newest first.
pub async fn list_all(db: &ConfigDb) -> Result<Vec<ImageMeta>> {
    let sql = "SELECT id, filename, mime_type, size_bytes, width, height, uploaded_at
               FROM images
              ORDER BY uploaded_at DESC, filename ASC";
    let rows: Vec<(String, String, String, i64, Option<i64>, Option<i64>, DateTime<Utc>)> =
        match db {
            ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
            ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        }
        .context("list images")?;
    Ok(rows
        .into_iter()
        .map(|(id, filename, mime_type, size_bytes, width, height, uploaded_at)| {
            ImageMeta {
                id,
                filename,
                mime_type,
                size_bytes,
                width,
                height,
                uploaded_at,
            }
        })
        .collect())
}

/// Delete by id. Returns true if a row was removed. Audit row is
/// written on either branch (an attempted delete is an event).
pub async fn delete_one(db: &ConfigDb, id: &str, actor: Option<&str>) -> Result<bool> {
    let now = Utc::now();
    let target = format!("image:{id}");
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin image delete tx")?;
            // Capture filename for the audit diff before deletion.
            let filename: Option<(String,)> =
                sqlx::query_as("SELECT filename FROM images WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await
                    .context("lookup image for delete")?;
            let rows = sqlx::query("DELETE FROM images WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("delete image {id}"))?;
            let removed = rows.rows_affected() > 0;
            if removed {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES (?, 'image.delete', ?, ?, ?)",
                )
                .bind(actor)
                .bind(&target)
                .bind(serde_json::to_string(&delete_diff(filename.as_ref()))?)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit image.delete")?;
            }
            tx.commit().await.context("commit image delete")?;
            Ok(removed)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin image delete tx")?;
            let filename: Option<(String,)> =
                sqlx::query_as("SELECT filename FROM images WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await
                    .context("lookup image for delete")?;
            let rows = sqlx::query("DELETE FROM images WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("delete image {id}"))?;
            let removed = rows.rows_affected() > 0;
            if removed {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES ($1, 'image.delete', $2, $3, $4)",
                )
                .bind(actor)
                .bind(&target)
                .bind(serde_json::to_string(&delete_diff(filename.as_ref()))?)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit image.delete")?;
            }
            tx.commit().await.context("commit image delete")?;
            Ok(removed)
        }
    }
}

/// Audit diff for a delete — the filename if we captured one, else
/// an empty object.
fn delete_diff(filename: Option<&(String,)>) -> serde_json::Value {
    match filename {
        Some((f,)) => serde_json::json!({ "filename": f }),
        None => serde_json::json!({}),
    }
}

#[cfg(all(test, feature = "postgres-it"))]
mod pg_tests {
    use super::*;

    // insert (BYTEA blob) → fetch_by_filename → list_all → delete,
    // through the `ConfigDb::Postgres` arm against a real daemon.
    #[tokio::test]
    async fn images_against_real_postgres() {
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pool = crate::db::open_pg(&url).await.unwrap();
        sqlx::query("DELETE FROM images")
            .execute(&pool)
            .await
            .unwrap();
        let db = ConfigDb::Postgres(pool);

        let processed = Processed {
            filename: "logo.webp".into(),
            mime_type: "image/webp".into(),
            bytes: vec![0xDE, 0xAD, 0xBE, 0xEF],
            width: Some(48),
            height: Some(48),
        };
        let id = insert(&db, processed, Some("admin")).await.unwrap();
        assert!(!id.is_empty());

        let (mime, bytes) = fetch_by_filename(&db, "logo.webp").await.unwrap().unwrap();
        assert_eq!(mime, "image/webp");
        assert_eq!(bytes, vec![0xDE, 0xAD, 0xBE, 0xEF]);

        let metas = list_all(&db).await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].filename, "logo.webp");
        assert_eq!(metas[0].size_bytes, 4);
        assert_eq!(metas[0].width, Some(48));

        assert!(delete_one(&db, &id, Some("admin")).await.unwrap());
        assert!(fetch_by_filename(&db, "logo.webp").await.unwrap().is_none());
    }
}
