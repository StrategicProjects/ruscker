//! Provenance of container images Ruscker has managed (#894).
//!
//! The Disk panel must distinguish images Ruscker pulled/managed from a
//! side-by-side ShinyProxy's (or anything else's) — so a "remove unused"
//! never deletes a neighbour's idle image cache, which is irrecoverable on
//! a host that can't reach the registry. Containers carry a
//! `ruscker.replica_id` label, but images carry no such mark, and an
//! orphaned image (its spec deleted/changed) is exactly the prune target,
//! so we can't lean on the current catalog alone.
//!
//! This table is the durable record: an image ref lands here when a spec
//! that references it is saved (`specs::upsert_in_tx*`, in the same tx) or
//! when it's explicitly pulled (the editor's Pull / re-pull). It survives
//! spec deletion, so a removed app's leftover image stays reclaimable.

use anyhow::{Context, Result};
use std::collections::HashSet;

use crate::db::ConfigDb;

/// Remember `refs` as Ruscker-managed images (idempotent; blank refs
/// skipped). Used by the explicit-pull paths, which aren't inside a spec
/// transaction — the spec save path records its image in-tx instead.
pub async fn remember(db: &ConfigDb, refs: &[String]) -> Result<()> {
    let now = chrono::Utc::now();
    for r in refs {
        let r = r.trim();
        if r.is_empty() {
            continue;
        }
        match db {
            ConfigDb::Sqlite(pool) => {
                sqlx::query(
                    "INSERT OR IGNORE INTO ruscker_images (image_ref, recorded_at) VALUES (?, ?)",
                )
                .bind(r)
                .bind(now)
                .execute(pool)
                .await
                .map(|_| ())
                .with_context(|| format!("record ruscker image {r}"))?;
            }
            ConfigDb::Postgres(pool) => {
                sqlx::query(
                    "INSERT INTO ruscker_images (image_ref, recorded_at) VALUES ($1, $2)
                     ON CONFLICT (image_ref) DO NOTHING",
                )
                .bind(r)
                .bind(now)
                .execute(pool)
                .await
                .map(|_| ())
                .with_context(|| format!("record ruscker image {r}"))?;
            }
        }
    }
    Ok(())
}

/// Every image ref Ruscker has ever managed. The Disk panel intersects
/// this with the live image list so removal/prune only ever targets a
/// Ruscker image.
pub async fn all(db: &ConfigDb) -> Result<HashSet<String>> {
    let rows: Vec<(String,)> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as("SELECT image_ref FROM ruscker_images")
            .fetch_all(pool)
            .await,
        ConfigDb::Postgres(pool) => sqlx::query_as("SELECT image_ref FROM ruscker_images")
            .fetch_all(pool)
            .await,
    }
    .context("load ruscker image provenance")?;
    Ok(rows.into_iter().map(|(r,)| r).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    #[tokio::test]
    async fn remember_is_idempotent_and_round_trips() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        // Blank refs are skipped.
        remember(&db, &["org/app:1".into(), "  ".into(), "".into()])
            .await
            .unwrap();
        // Re-recording the same ref is a no-op (no PK violation).
        remember(&db, &["org/app:1".into(), "org/api:2".into()])
            .await
            .unwrap();

        let all = all(&db).await.unwrap();
        assert!(all.contains("org/app:1"));
        assert!(all.contains("org/api:2"));
        assert_eq!(all.len(), 2, "blanks skipped, duplicate not double-counted");
    }
}
