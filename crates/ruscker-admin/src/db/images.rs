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

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Utc};
use ruscker_config::{LandingCustomization, Spec};
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

/// Shape of one row from the `images` listing query — kept as a type
/// alias so the deeply-nested tuple doesn't drown the call site.
/// Order matches the `SELECT` in [`list_all`].
type ImageListRow = (
    String,         // id
    String,         // filename
    String,         // mime_type
    i64,            // size_bytes
    Option<i64>,    // width
    Option<i64>,    // height
    DateTime<Utc>,  // uploaded_at
);

/// Gallery listing — every uploaded image, newest first.
pub async fn list_all(db: &ConfigDb) -> Result<Vec<ImageMeta>> {
    let sql = "SELECT id, filename, mime_type, size_bytes, width, height, uploaded_at
               FROM images
              ORDER BY uploaded_at DESC, filename ASC";
    let rows: Vec<ImageListRow> =
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

/// The filename for an image id, without deleting — so a caller can find
/// what references it before removing it (#560).
pub async fn filename_for(db: &ConfigDb, id: &str) -> Result<Option<String>> {
    let row: Option<(String,)> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as("SELECT filename FROM images WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .context("lookup image filename (sqlite)")?,
        ConfigDb::Postgres(pool) => sqlx::query_as("SELECT filename FROM images WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .context("lookup image filename (postgres)")?,
    };
    Ok(row.map(|(f,)| f))
}

/// Cheap existence check by filename (no BLOB fetched). Used to pick a
/// non-colliding name for a manual upload.
pub async fn filename_taken(db: &ConfigDb, name: &str) -> Result<bool> {
    let row: Option<(i64,)> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as("SELECT 1 FROM images WHERE filename = ? LIMIT 1")
                .bind(name)
                .fetch_optional(pool)
                .await
                .context("image filename exists (sqlite)")?
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as("SELECT 1 FROM images WHERE filename = $1 LIMIT 1")
                .bind(name)
                .fetch_optional(pool)
                .await
                .context("image filename exists (postgres)")?
        }
    };
    Ok(row.is_some())
}

/// Resolve a free filename for a manual upload: returns `desired` if no
/// image row uses it, otherwise the first free `stem-N.ext` variant
/// (N = 2, 3, …). This lets a same-named upload keep BOTH images instead
/// of silently overwriting the existing one. The YAML-import path keeps
/// using exact names (it pre-checks and skips), so it never calls this.
pub async fn unique_filename(db: &ConfigDb, desired: &str) -> Result<String> {
    if !filename_taken(db, desired).await? {
        return Ok(desired.to_string());
    }
    let (stem, ext) = match desired.rsplit_once('.') {
        Some((s, e)) => (s.to_string(), format!(".{e}")),
        None => (desired.to_string(), String::new()),
    };
    for n in 2..=9999 {
        let candidate = format!("{stem}-{n}{ext}");
        if !filename_taken(db, &candidate).await? {
            return Ok(candidate);
        }
    }
    anyhow::bail!("no free filename for {desired} after 9999 tries")
}

/// Rename an image (filename only — the id and BLOB are untouched).
/// The caller must ensure `new_filename` is free and fix up any
/// references first. Writes an `image.rename` audit row.
pub async fn rename(db: &ConfigDb, id: &str, new_filename: &str, actor: Option<&str>) -> Result<()> {
    let now = Utc::now();
    let target = format!("image:{id}");
    let diff = serde_json::to_string(&serde_json::json!({ "filename": new_filename }))?;
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin image rename tx")?;
            sqlx::query("UPDATE images SET filename = ? WHERE id = ?")
                .bind(new_filename)
                .bind(id)
                .execute(&mut *tx)
                .await
                .context("rename image (sqlite)")?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'image.rename', ?, ?, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit image.rename (sqlite)")?;
            tx.commit().await.context("commit image rename")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin image rename tx")?;
            sqlx::query("UPDATE images SET filename = $1 WHERE id = $2")
                .bind(new_filename)
                .bind(id)
                .execute(&mut *tx)
                .await
                .context("rename image (postgres)")?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'image.rename', $2, $3, $4)",
            )
            .bind(actor)
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit image.rename (postgres)")?;
            tx.commit().await.context("commit image rename")?;
        }
    }
    Ok(())
}

/// Delete an image by id. Returns the deleted row's filename (so the
/// caller can invalidate caches keyed by it, #301), or `None` if no
/// such image existed. Audit row is written when a row was removed.
pub async fn delete_one(db: &ConfigDb, id: &str, actor: Option<&str>) -> Result<Option<String>> {
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
            Ok(if removed { filename.map(|(f,)| f) } else { None })
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
            Ok(if removed { filename.map(|(f,)| f) } else { None })
        }
    }
}

/// Rename an image AND rewrite every reference to it (the given specs +
/// landing) in **one transaction** (#720 audit P2). Before, the route
/// rewrote specs/landing in separate transactions and renamed the image
/// last — a failure there left cards pointing at a filename that no
/// longer existed. Here all writes commit together or not at all.
///
/// `specs` are the already-rewritten specs to upsert; `landing` is the
/// rewritten customization, or `None` when it held no reference. The
/// target filename is re-checked *inside* the tx (authoritative over the
/// route's advisory pre-check), so a colliding name rolls the batch back.
pub(crate) async fn rename_with_refs(
    db: &ConfigDb,
    id: &str,
    new_filename: &str,
    specs: &[Spec],
    landing: Option<&LandingCustomization>,
    actor: Option<&str>,
) -> Result<()> {
    let now = Utc::now();
    let target = format!("image:{id}");
    let diff = serde_json::to_string(&serde_json::json!({ "filename": new_filename }))?;
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin image rename tx")?;
            let taken: (i64,) =
                sqlx::query_as("SELECT count(*) FROM images WHERE filename = ? AND id != ?")
                    .bind(new_filename)
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await
                    .context("rename collision check (sqlite)")?;
            if taken.0 > 0 {
                bail!("filename '{new_filename}' is already taken");
            }
            sqlx::query("UPDATE images SET filename = ? WHERE id = ?")
                .bind(new_filename)
                .bind(id)
                .execute(&mut *tx)
                .await
                .context("rename image (sqlite)")?;
            audit_sqlite(&mut tx, actor, "image.rename", &target, Some(&diff), now).await?;
            for spec in specs {
                crate::db::specs::upsert_in_tx(&mut tx, spec, now).await?;
                let t = format!("spec:{}", spec.id);
                audit_sqlite(&mut tx, actor, "spec.update", &t, None, now).await?;
            }
            if let Some(lc) = landing {
                crate::db::landing::update_in_tx(&mut tx, lc, now).await?;
                audit_sqlite(
                    &mut tx,
                    actor,
                    "landing.update",
                    "landing:customization",
                    None,
                    now,
                )
                .await?;
            }
            tx.commit().await.context("commit image rename")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin image rename tx")?;
            let taken: (i64,) =
                sqlx::query_as("SELECT count(*) FROM images WHERE filename = $1 AND id != $2")
                    .bind(new_filename)
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await
                    .context("rename collision check (postgres)")?;
            if taken.0 > 0 {
                bail!("filename '{new_filename}' is already taken");
            }
            sqlx::query("UPDATE images SET filename = $1 WHERE id = $2")
                .bind(new_filename)
                .bind(id)
                .execute(&mut *tx)
                .await
                .context("rename image (postgres)")?;
            audit_pg(&mut tx, actor, "image.rename", &target, Some(&diff), now).await?;
            for spec in specs {
                crate::db::specs::upsert_in_tx_pg(&mut tx, spec, now).await?;
                let t = format!("spec:{}", spec.id);
                audit_pg(&mut tx, actor, "spec.update", &t, None, now).await?;
            }
            if let Some(lc) = landing {
                crate::db::landing::update_in_tx_pg(&mut tx, lc, now).await?;
                audit_pg(
                    &mut tx,
                    actor,
                    "landing.update",
                    "landing:customization",
                    None,
                    now,
                )
                .await?;
            }
            tx.commit().await.context("commit image rename")?;
        }
    }
    Ok(())
}

/// Delete an image AND reset every reference to it (the given specs +
/// landing) in **one transaction** (#720 audit P2). `specs` are the
/// already-reset specs to upsert (logo → default mark, cover removed);
/// `landing` is the reset customization, or `None`. Returns the deleted
/// row's filename (for cache invalidation), or `None` if it was already
/// gone — in which case nothing is written.
pub(crate) async fn delete_with_refs(
    db: &ConfigDb,
    id: &str,
    specs: &[Spec],
    landing: Option<&LandingCustomization>,
    actor: Option<&str>,
) -> Result<Option<String>> {
    let now = Utc::now();
    let target = format!("image:{id}");
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin image delete tx")?;
            let filename: Option<(String,)> =
                sqlx::query_as("SELECT filename FROM images WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await
                    .context("lookup image for delete")?;
            if filename.is_none() {
                return Ok(None); // already gone — tx drops, nothing written
            }
            sqlx::query("DELETE FROM images WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("delete image {id}"))?;
            let diff = serde_json::to_string(&delete_diff(filename.as_ref()))?;
            audit_sqlite(&mut tx, actor, "image.delete", &target, Some(&diff), now).await?;
            for spec in specs {
                crate::db::specs::upsert_in_tx(&mut tx, spec, now).await?;
                let t = format!("spec:{}", spec.id);
                audit_sqlite(&mut tx, actor, "spec.update", &t, None, now).await?;
            }
            if let Some(lc) = landing {
                crate::db::landing::update_in_tx(&mut tx, lc, now).await?;
                audit_sqlite(
                    &mut tx,
                    actor,
                    "landing.update",
                    "landing:customization",
                    None,
                    now,
                )
                .await?;
            }
            tx.commit().await.context("commit image delete")?;
            Ok(filename.map(|(f,)| f))
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin image delete tx")?;
            let filename: Option<(String,)> =
                sqlx::query_as("SELECT filename FROM images WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await
                    .context("lookup image for delete")?;
            if filename.is_none() {
                return Ok(None);
            }
            sqlx::query("DELETE FROM images WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("delete image {id}"))?;
            let diff = serde_json::to_string(&delete_diff(filename.as_ref()))?;
            audit_pg(&mut tx, actor, "image.delete", &target, Some(&diff), now).await?;
            for spec in specs {
                crate::db::specs::upsert_in_tx_pg(&mut tx, spec, now).await?;
                let t = format!("spec:{}", spec.id);
                audit_pg(&mut tx, actor, "spec.update", &t, None, now).await?;
            }
            if let Some(lc) = landing {
                crate::db::landing::update_in_tx_pg(&mut tx, lc, now).await?;
                audit_pg(
                    &mut tx,
                    actor,
                    "landing.update",
                    "landing:customization",
                    None,
                    now,
                )
                .await?;
            }
            tx.commit().await.context("commit image delete")?;
            Ok(filename.map(|(f,)| f))
        }
    }
}

/// Append one audit row inside a SQLite transaction (shared by the atomic
/// image ops above). `diff` is the JSON diff, or `None` for no diff.
async fn audit_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor: Option<&str>,
    action: &str,
    target: &str,
    diff: Option<&str>,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(actor)
    .bind(action)
    .bind(target)
    .bind(diff)
    .bind(now)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("audit {action}"))?;
    Ok(())
}

/// Postgres twin of [`audit_sqlite`].
async fn audit_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Option<&str>,
    action: &str,
    target: &str,
    diff: Option<&str>,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(actor)
    .bind(action)
    .bind(target)
    .bind(diff)
    .bind(now)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("audit {action}"))?;
    Ok(())
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
        let _guard = crate::db::pg_test_lock().lock().await;
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

        // Returns the deleted filename now (#301).
        assert_eq!(
            delete_one(&db, &id, Some("admin")).await.unwrap().as_deref(),
            Some("logo.webp")
        );
        assert!(fetch_by_filename(&db, "logo.webp").await.unwrap().is_none());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;
    use ruscker_config::Config;

    async fn insert_image(db: &ConfigDb, id: &str, filename: &str) {
        let ConfigDb::Sqlite(pool) = db else {
            return;
        };
        sqlx::query(
            "INSERT INTO images (id, filename, mime_type, size_bytes, blob, uploaded_at)
             VALUES (?, ?, 'image/png', 3, ?, ?)",
        )
        .bind(id)
        .bind(filename)
        .bind(vec![1u8, 2, 3])
        .bind(Utc::now())
        .execute(pool)
        .await
        .unwrap();
    }

    fn spec_with_logo(id: &str, logo: &str) -> Spec {
        std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
        let yaml = format!(
            "proxy:\n  specs:\n    - id: {id}\n      container-image: img\n      template-properties:\n        logo: \"{logo}\"\n"
        );
        Config::from_yaml(&yaml)
            .unwrap()
            .proxy
            .specs
            .into_iter()
            .next()
            .unwrap()
    }

    async fn logo_of(db: &ConfigDb, id: &str) -> Option<String> {
        crate::db::specs::fetch_one(db, id)
            .await
            .unwrap()
            .unwrap()
            .template_properties
            .get_str("logo")
            .map(str::to_string)
    }

    // Happy path: the image rename and the spec rewrite commit together.
    #[tokio::test]
    async fn rename_with_refs_commits_image_and_spec_together() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        insert_image(&db, "img1", "old.png").await;
        let mut spec = spec_with_logo("app", "/assets/img/old.png");
        crate::db::specs::upsert_one(&db, &spec, None).await.unwrap();

        spec.template_properties
            .set_str("logo", "/assets/img/new.png");
        rename_with_refs(&db, "img1", "new.png", std::slice::from_ref(&spec), None, None)
            .await
            .unwrap();

        assert_eq!(filename_for(&db, "img1").await.unwrap().as_deref(), Some("new.png"));
        assert_eq!(logo_of(&db, "app").await.as_deref(), Some("/assets/img/new.png"));
    }

    // Conflict: renaming onto a name another image already holds must roll
    // the WHOLE batch back — the spec ref stays pointing at the old name,
    // and the image keeps its old name (no half-applied state).
    #[tokio::test]
    async fn rename_with_refs_rolls_back_on_name_conflict() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        insert_image(&db, "img1", "old.png").await;
        insert_image(&db, "img2", "taken.png").await;
        let spec = spec_with_logo("app", "/assets/img/old.png");
        crate::db::specs::upsert_one(&db, &spec, None).await.unwrap();

        let mut rewritten = spec.clone();
        rewritten
            .template_properties
            .set_str("logo", "/assets/img/taken.png");
        let res =
            rename_with_refs(&db, "img1", "taken.png", std::slice::from_ref(&rewritten), None, None)
                .await;

        assert!(res.is_err(), "rename onto a taken name must fail");
        assert_eq!(
            filename_for(&db, "img1").await.unwrap().as_deref(),
            Some("old.png"),
            "image name rolled back"
        );
        assert_eq!(
            logo_of(&db, "app").await.as_deref(),
            Some("/assets/img/old.png"),
            "spec ref rolled back — not left pointing at the failed rename"
        );
    }

    // Delete + reference reset commit together: image gone, spec reset.
    #[tokio::test]
    async fn delete_with_refs_removes_image_and_resets_specs_together() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        insert_image(&db, "img1", "gone.png").await;
        let mut spec = spec_with_logo("app", "/assets/img/gone.png");
        crate::db::specs::upsert_one(&db, &spec, None).await.unwrap();

        spec.template_properties
            .set_str("logo", "/assets/img/ruscker-mark.svg");
        let removed = delete_with_refs(&db, "img1", std::slice::from_ref(&spec), None, None)
            .await
            .unwrap();

        assert_eq!(removed.as_deref(), Some("gone.png"));
        assert!(filename_for(&db, "img1").await.unwrap().is_none(), "image deleted");
        assert_eq!(
            logo_of(&db, "app").await.as_deref(),
            Some("/assets/img/ruscker-mark.svg"),
            "spec logo reset to the default mark"
        );
    }
}
