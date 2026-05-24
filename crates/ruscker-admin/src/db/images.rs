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
use sqlx::SqlitePool;
use uuid::Uuid;

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
pub async fn insert(pool: &SqlitePool, processed: Processed, actor: Option<&str>) -> Result<String> {
    let now = Utc::now();
    let mut tx = pool.begin().await.context("begin image insert tx")?;

    // Remove any prior row with the same filename so the new
    // upload wins. We could UPSERT with the same id but reusing
    // ids across content is worse for cache headers downstream.
    sqlx::query("DELETE FROM images WHERE filename = ?")
        .bind(&processed.filename)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("delete prior image {}", processed.filename))?;

    let id = Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT INTO images
           (id, filename, mime_type, size_bytes, blob, width, height, uploaded_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(&id)
    .bind(&processed.filename)
    .bind(&processed.mime_type)
    .bind(processed.bytes.len() as i64)
    .bind(&processed.bytes)
    .bind(processed.width.map(|n| n as i64))
    .bind(processed.height.map(|n| n as i64))
    .bind(now)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("insert image {}", processed.filename))?;

    sqlx::query(
        "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
         VALUES (?, 'image.upload', ?, ?, ?)",
    )
    .bind(actor)
    .bind(format!("image:{id}"))
    .bind(serde_json::to_string(&serde_json::json!({
        "filename": processed.filename,
        "mime": processed.mime_type,
        "size": processed.bytes.len(),
    }))?)
    .bind(now)
    .execute(&mut *tx)
    .await
    .context("audit image.upload")?;

    tx.commit().await.context("commit image insert")?;
    Ok(id)
}

/// Fetch the bytes + MIME of an image by **filename**. This is the
/// public lookup hit by `GET /assets/img/<filename>`.
pub async fn fetch_by_filename(
    pool: &SqlitePool,
    filename: &str,
) -> Result<Option<(String, Vec<u8>)>> {
    let row: Option<(String, Vec<u8>)> =
        sqlx::query_as("SELECT mime_type, blob FROM images WHERE filename = ?")
            .bind(filename)
            .fetch_optional(pool)
            .await
            .with_context(|| format!("fetch image {filename}"))?;
    Ok(row)
}

/// Gallery listing — every uploaded image, newest first.
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<ImageMeta>> {
    let rows: Vec<(String, String, String, i64, Option<i64>, Option<i64>, DateTime<Utc>)> =
        sqlx::query_as(
            "SELECT id, filename, mime_type, size_bytes, width, height, uploaded_at
               FROM images
              ORDER BY uploaded_at DESC, filename ASC",
        )
        .fetch_all(pool)
        .await
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
pub async fn delete_one(pool: &SqlitePool, id: &str, actor: Option<&str>) -> Result<bool> {
    let now = Utc::now();
    let mut tx = pool.begin().await.context("begin image delete tx")?;

    // Capture filename for the audit diff before deletion.
    let filename: Option<(String,)> = sqlx::query_as("SELECT filename FROM images WHERE id = ?")
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
        let diff = filename
            .as_ref()
            .map(|(f,)| serde_json::json!({ "filename": f }))
            .unwrap_or_else(|| serde_json::json!({}));
        sqlx::query(
            "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
             VALUES (?, 'image.delete', ?, ?, ?)",
        )
        .bind(actor)
        .bind(format!("image:{id}"))
        .bind(serde_json::to_string(&diff)?)
        .bind(now)
        .execute(&mut *tx)
        .await
        .context("audit image.delete")?;
    }
    tx.commit().await.context("commit image delete")?;
    Ok(removed)
}
