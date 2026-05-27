//! Custom landing HTML blocks (#54).
//!
//! Operator-authored HTML rendered in fixed landing slots (`top`
//! after the header, `bottom` after the card grid). Admin-managed and
//! DB-only for now (not part of the YAML import/export round-trip).
//! `position` orders blocks within a slot; `csp_origins` (space-
//! separated) widens the landing CSP so embedded third-party content
//! can load.

use crate::db::ConfigDb;
use anyhow::{Context, Result};
use chrono::Utc;

/// The two slots a block can live in. Stored as the lowercase string.
pub const SLOTS: &[&str] = &["top", "bottom"];

/// A single custom HTML block.
#[derive(Debug, Clone, serde::Serialize)]
pub struct LandingBlock {
    pub id: String,
    pub slot: String,
    pub position: i64,
    pub enabled: bool,
    pub title: String,
    pub html: String,
    pub csp_origins: String,
}

/// Values an insert/update writes — everything but the id/position
/// (which the repository owns).
#[derive(Debug, Clone)]
pub struct BlockInput {
    pub slot: String,
    pub enabled: bool,
    pub title: String,
    pub html: String,
    pub csp_origins: String,
}

// `enabled` is read as `bool`: SQLite decodes its INTEGER 0/1 into a
// bool, Postgres reads the native BOOLEAN — one Row type serves both
// dialects (Phase 7c). The write paths still bind `enabled as i64` on
// the SQLite side; those columns get ported with their modules.
type Row = (String, String, i64, bool, String, String, String);

fn row_to_block((id, slot, position, enabled, title, html, csp_origins): Row) -> LandingBlock {
    LandingBlock {
        id,
        slot,
        position,
        enabled,
        title,
        html,
        csp_origins,
    }
}

/// All blocks, ordered by slot then position — for the admin list.
pub async fn list_all(db: &ConfigDb) -> Result<Vec<LandingBlock>> {
    let sql = "SELECT id, slot, position, enabled, title, html, csp_origins
           FROM landing_blocks ORDER BY slot, position, created_at";
    let rows: Vec<Row> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    }
    .context("list landing_blocks")?;
    Ok(rows.into_iter().map(row_to_block).collect())
}

/// Enabled blocks only, ordered — for rendering the public landing.
///
/// Ported to dual-dialect (Phase 7c-3): bare `WHERE enabled` (not
/// `= 1`) is truthy on SQLite and a native BOOLEAN test on Postgres,
/// and the rest of the SELECT is placeholder-free, so one SQL string
/// serves both backends.
pub async fn list_enabled(db: &ConfigDb) -> Result<Vec<LandingBlock>> {
    let sql = "SELECT id, slot, position, enabled, title, html, csp_origins
           FROM landing_blocks WHERE enabled ORDER BY slot, position, created_at";
    let rows: Vec<Row> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    }
    .context("list enabled landing_blocks")?;
    Ok(rows.into_iter().map(row_to_block).collect())
}

/// One block by id, or `None`.
pub async fn fetch_one(db: &ConfigDb, id: &str) -> Result<Option<LandingBlock>> {
    let row: Option<Row> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(
            "SELECT id, slot, position, enabled, title, html, csp_origins
               FROM landing_blocks WHERE id = ?",
        )
        .bind(id)
        .fetch_optional(pool)
        .await,
        ConfigDb::Postgres(pool) => sqlx::query_as(
            "SELECT id, slot, position, enabled, title, html, csp_origins
               FROM landing_blocks WHERE id = $1",
        )
        .bind(id)
        .fetch_optional(pool)
        .await,
    }
    .context("fetch landing_block")?;
    Ok(row.map(row_to_block))
}

/// Create a block, appended at the end of its slot. Returns the new id.
///
/// `enabled` binds as `bool` on Postgres (native BOOLEAN) but `i64` on
/// SQLite (INTEGER 0/1) — see the migration note in `migrations-pg`.
pub async fn insert(db: &ConfigDb, input: &BlockInput, actor: Option<&str>) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin block insert tx")?;
            let (next_pos,): (i64,) = sqlx::query_as(
                "SELECT COALESCE(MAX(position) + 1, 0) FROM landing_blocks WHERE slot = ?",
            )
            .bind(&input.slot)
            .fetch_one(&mut *tx)
            .await
            .context("compute block position")?;
            sqlx::query(
                "INSERT INTO landing_blocks
                   (id, slot, position, enabled, title, html, csp_origins, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&input.slot)
            .bind(next_pos)
            .bind(input.enabled as i64)
            .bind(&input.title)
            .bind(&input.html)
            .bind(&input.csp_origins)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("insert landing_block")?;
            audit(&mut tx, actor, "landing_block.create", &id, now).await?;
            tx.commit().await.context("commit block insert")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin block insert tx")?;
            let (next_pos,): (i64,) = sqlx::query_as(
                "SELECT COALESCE(MAX(position) + 1, 0) FROM landing_blocks WHERE slot = $1",
            )
            .bind(&input.slot)
            .fetch_one(&mut *tx)
            .await
            .context("compute block position")?;
            sqlx::query(
                "INSERT INTO landing_blocks
                   (id, slot, position, enabled, title, html, csp_origins, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(&id)
            .bind(&input.slot)
            .bind(next_pos)
            .bind(input.enabled)
            .bind(&input.title)
            .bind(&input.html)
            .bind(&input.csp_origins)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("insert landing_block")?;
            audit_pg(&mut tx, actor, "landing_block.create", &id, now).await?;
            tx.commit().await.context("commit block insert")?;
        }
    }
    Ok(id)
}

/// Update a block's content (slot/enabled/title/html/origins). Keeps
/// its position unless the slot changed, in which case it's appended
/// to the new slot.
pub async fn update(
    db: &ConfigDb,
    id: &str,
    input: &BlockInput,
    actor: Option<&str>,
) -> Result<bool> {
    let now = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin block update tx")?;
            let current: Option<(String, i64)> =
                sqlx::query_as("SELECT slot, position FROM landing_blocks WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await
                    .context("lookup block")?;
            let Some((current_slot, current_pos)) = current else {
                return Ok(false);
            };
            // Keep the position when the slot is unchanged; re-append at
            // the end of the destination slot when it moves.
            let new_pos = if current_slot == input.slot {
                current_pos
            } else {
                let (next_pos,): (i64,) = sqlx::query_as(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM landing_blocks WHERE slot = ?",
                )
                .bind(&input.slot)
                .fetch_one(&mut *tx)
                .await
                .context("compute new-slot position")?;
                next_pos
            };
            sqlx::query(
                "UPDATE landing_blocks SET slot = ?, position = ?, enabled = ?, title = ?,
                    html = ?, csp_origins = ?, updated_at = ? WHERE id = ?",
            )
            .bind(&input.slot)
            .bind(new_pos)
            .bind(input.enabled as i64)
            .bind(&input.title)
            .bind(&input.html)
            .bind(&input.csp_origins)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await
            .context("update landing_block")?;
            audit(&mut tx, actor, "landing_block.update", id, now).await?;
            tx.commit().await.context("commit block update")?;
            Ok(true)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin block update tx")?;
            let current: Option<(String, i64)> =
                sqlx::query_as("SELECT slot, position FROM landing_blocks WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await
                    .context("lookup block")?;
            let Some((current_slot, current_pos)) = current else {
                return Ok(false);
            };
            let new_pos = if current_slot == input.slot {
                current_pos
            } else {
                let (next_pos,): (i64,) = sqlx::query_as(
                    "SELECT COALESCE(MAX(position) + 1, 0) FROM landing_blocks WHERE slot = $1",
                )
                .bind(&input.slot)
                .fetch_one(&mut *tx)
                .await
                .context("compute new-slot position")?;
                next_pos
            };
            sqlx::query(
                "UPDATE landing_blocks SET slot = $1, position = $2, enabled = $3, title = $4,
                    html = $5, csp_origins = $6, updated_at = $7 WHERE id = $8",
            )
            .bind(&input.slot)
            .bind(new_pos)
            .bind(input.enabled)
            .bind(&input.title)
            .bind(&input.html)
            .bind(&input.csp_origins)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await
            .context("update landing_block")?;
            audit_pg(&mut tx, actor, "landing_block.update", id, now).await?;
            tx.commit().await.context("commit block update")?;
            Ok(true)
        }
    }
}

/// Delete a block. Returns whether a row was removed.
pub async fn delete(db: &ConfigDb, id: &str, actor: Option<&str>) -> Result<bool> {
    let now = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin block delete tx")?;
            let rows = sqlx::query("DELETE FROM landing_blocks WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await
                .context("delete landing_block")?;
            let removed = rows.rows_affected() > 0;
            if removed {
                audit(&mut tx, actor, "landing_block.delete", id, now).await?;
            }
            tx.commit().await.context("commit block delete")?;
            Ok(removed)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin block delete tx")?;
            let rows = sqlx::query("DELETE FROM landing_blocks WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await
                .context("delete landing_block")?;
            let removed = rows.rows_affected() > 0;
            if removed {
                audit_pg(&mut tx, actor, "landing_block.delete", id, now).await?;
            }
            tx.commit().await.context("commit block delete")?;
            Ok(removed)
        }
    }
}

impl LandingBlock {
    /// Map to the config-crate representation for YAML export.
    pub fn to_config(&self) -> ruscker_config::LandingBlock {
        ruscker_config::LandingBlock {
            slot: self.slot.clone(),
            title: self.title.clone(),
            html: self.html.clone(),
            csp_origins: self.csp_origins.clone(),
            enabled: self.enabled,
        }
    }
}

/// Replace ALL blocks with `blocks` (used by the YAML import). Clears
/// the table, then inserts each block with a per-slot `position` taken
/// from its order of appearance. Runs inside the caller's transaction.
pub(crate) async fn replace_all_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    blocks: &[ruscker_config::LandingBlock],
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    sqlx::query("DELETE FROM landing_blocks")
        .execute(&mut **tx)
        .await
        .context("clear landing_blocks")?;

    let mut next: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for b in blocks {
        // Guard the slot so a hand-edited YAML can't create an
        // unrenderable block.
        let slot = if SLOTS.contains(&b.slot.as_str()) {
            b.slot.as_str()
        } else {
            "top"
        };
        let pos = next.entry(slot.to_owned()).or_insert(0);
        sqlx::query(
            "INSERT INTO landing_blocks
               (id, slot, position, enabled, title, html, csp_origins, created_at, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(slot)
        .bind(*pos)
        .bind(b.enabled as i64)
        .bind(&b.title)
        .bind(&b.html)
        .bind(&b.csp_origins)
        .bind(now)
        .bind(now)
        .execute(&mut **tx)
        .await
        .context("insert imported block")?;
        *pos += 1;
    }
    Ok(())
}

/// Postgres twin of [`replace_all_in_tx`] — `$n` placeholders, and
/// `enabled` binds as the native `bool`. Used by the Postgres arm of
/// `specs::import_all`.
pub(crate) async fn replace_all_in_tx_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    blocks: &[ruscker_config::LandingBlock],
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    sqlx::query("DELETE FROM landing_blocks")
        .execute(&mut **tx)
        .await
        .context("clear landing_blocks")?;

    let mut next: std::collections::HashMap<String, i64> = std::collections::HashMap::new();
    for b in blocks {
        let slot = if SLOTS.contains(&b.slot.as_str()) {
            b.slot.as_str()
        } else {
            "top"
        };
        let pos = next.entry(slot.to_owned()).or_insert(0);
        sqlx::query(
            "INSERT INTO landing_blocks
               (id, slot, position, enabled, title, html, csp_origins, created_at, updated_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(uuid::Uuid::new_v4().to_string())
        .bind(slot)
        .bind(*pos)
        .bind(b.enabled)
        .bind(&b.title)
        .bind(&b.html)
        .bind(&b.csp_origins)
        .bind(now)
        .bind(now)
        .execute(&mut **tx)
        .await
        .context("insert imported block")?;
        *pos += 1;
    }
    Ok(())
}

/// Move a block one step within its slot by swapping `position` with
/// the adjacent block (`up` = toward the front). No-op (returns
/// `false`) when the block doesn't exist or is already at the slot
/// edge. Gap-safe: it picks the nearest neighbour by position, not by
/// `position ± 1`.
pub async fn move_block(db: &ConfigDb, id: &str, up: bool, actor: Option<&str>) -> Result<bool> {
    let now = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin block move tx")?;
            let me: Option<(String, i64)> =
                sqlx::query_as("SELECT slot, position FROM landing_blocks WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await
                    .context("lookup block for move")?;
            let Some((slot, pos)) = me else {
                return Ok(false);
            };
            let neighbour: Option<(String, i64)> = if up {
                sqlx::query_as(
                    "SELECT id, position FROM landing_blocks
                       WHERE slot = ? AND position < ? ORDER BY position DESC LIMIT 1",
                )
            } else {
                sqlx::query_as(
                    "SELECT id, position FROM landing_blocks
                       WHERE slot = ? AND position > ? ORDER BY position ASC LIMIT 1",
                )
            }
            .bind(&slot)
            .bind(pos)
            .fetch_optional(&mut *tx)
            .await
            .context("find move neighbour")?;
            let Some((nid, npos)) = neighbour else {
                return Ok(false); // already at the slot edge
            };
            for (bid, p) in [(id, npos), (nid.as_str(), pos)] {
                sqlx::query("UPDATE landing_blocks SET position = ?, updated_at = ? WHERE id = ?")
                    .bind(p)
                    .bind(now)
                    .bind(bid)
                    .execute(&mut *tx)
                    .await
                    .context("swap block position")?;
            }
            audit(&mut tx, actor, "landing_block.move", id, now).await?;
            tx.commit().await.context("commit block move")?;
            Ok(true)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin block move tx")?;
            let me: Option<(String, i64)> =
                sqlx::query_as("SELECT slot, position FROM landing_blocks WHERE id = $1")
                    .bind(id)
                    .fetch_optional(&mut *tx)
                    .await
                    .context("lookup block for move")?;
            let Some((slot, pos)) = me else {
                return Ok(false);
            };
            let neighbour: Option<(String, i64)> = if up {
                sqlx::query_as(
                    "SELECT id, position FROM landing_blocks
                       WHERE slot = $1 AND position < $2 ORDER BY position DESC LIMIT 1",
                )
            } else {
                sqlx::query_as(
                    "SELECT id, position FROM landing_blocks
                       WHERE slot = $1 AND position > $2 ORDER BY position ASC LIMIT 1",
                )
            }
            .bind(&slot)
            .bind(pos)
            .fetch_optional(&mut *tx)
            .await
            .context("find move neighbour")?;
            let Some((nid, npos)) = neighbour else {
                return Ok(false); // already at the slot edge
            };
            for (bid, p) in [(id, npos), (nid.as_str(), pos)] {
                sqlx::query("UPDATE landing_blocks SET position = $1, updated_at = $2 WHERE id = $3")
                    .bind(p)
                    .bind(now)
                    .bind(bid)
                    .execute(&mut *tx)
                    .await
                    .context("swap block position")?;
            }
            audit_pg(&mut tx, actor, "landing_block.move", id, now).await?;
            tx.commit().await.context("commit block move")?;
            Ok(true)
        }
    }
}

/// Rewrite the `position` of every block in `slot` to match the order
/// of `ids` (drag-and-drop reorder). Ids that don't belong to `slot`
/// are ignored, so a tampered payload can't move blocks across slots.
/// Returns `false` for an unknown slot.
pub async fn reorder(
    db: &ConfigDb,
    slot: &str,
    ids: &[String],
    actor: Option<&str>,
) -> Result<bool> {
    if !SLOTS.contains(&slot) {
        return Ok(false);
    }
    let now = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin block reorder tx")?;
            // Assign 0..N in submitted order; only rows actually in this
            // slot advance the counter, so positions stay dense and
            // slot-scoped.
            let mut pos: i64 = 0;
            for id in ids {
                let res = sqlx::query(
                    "UPDATE landing_blocks SET position = ?, updated_at = ?
                       WHERE id = ? AND slot = ?",
                )
                .bind(pos)
                .bind(now)
                .bind(id)
                .bind(slot)
                .execute(&mut *tx)
                .await
                .context("reorder block")?;
                if res.rows_affected() > 0 {
                    pos += 1;
                }
            }
            audit(&mut tx, actor, "landing_block.reorder", slot, now).await?;
            tx.commit().await.context("commit block reorder")?;
            Ok(true)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin block reorder tx")?;
            let mut pos: i64 = 0;
            for id in ids {
                let res = sqlx::query(
                    "UPDATE landing_blocks SET position = $1, updated_at = $2
                       WHERE id = $3 AND slot = $4",
                )
                .bind(pos)
                .bind(now)
                .bind(id)
                .bind(slot)
                .execute(&mut *tx)
                .await
                .context("reorder block")?;
                if res.rows_affected() > 0 {
                    pos += 1;
                }
            }
            audit_pg(&mut tx, actor, "landing_block.reorder", slot, now).await?;
            tx.commit().await.context("commit block reorder")?;
            Ok(true)
        }
    }
}

async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor: Option<&str>,
    action: &str,
    id: &str,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
         VALUES (?, ?, ?, NULL, ?)",
    )
    .bind(actor)
    .bind(action)
    .bind(format!("landing_block:{id}"))
    .bind(now)
    .execute(&mut **tx)
    .await
    .context("audit landing_block")?;
    Ok(())
}

/// Postgres twin of [`audit`] — `$n` placeholders.
async fn audit_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Option<&str>,
    action: &str,
    id: &str,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
         VALUES ($1, $2, $3, NULL, $4)",
    )
    .bind(actor)
    .bind(action)
    .bind(format!("landing_block:{id}"))
    .bind(now)
    .execute(&mut **tx)
    .await
    .context("audit landing_block")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    fn input(slot: &str, title: &str) -> BlockInput {
        BlockInput {
            slot: slot.into(),
            enabled: true,
            title: title.into(),
            html: format!("<p>{title}</p>"),
            csp_origins: String::new(),
        }
    }

    #[tokio::test]
    async fn insert_appends_position_per_slot() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        insert(&db, &input("top", "a"), None).await.unwrap();
        insert(&db, &input("top", "b"), None).await.unwrap();
        insert(&db, &input("bottom", "c"), None).await.unwrap();
        let all = list_all(&db).await.unwrap();
        // bottom sorts before top alphabetically; within top, a then b.
        let top: Vec<_> = all.iter().filter(|b| b.slot == "top").collect();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].position, 0);
        assert_eq!(top[1].position, 1);
        assert_eq!(all.iter().filter(|b| b.slot == "bottom").count(), 1);
    }

    #[tokio::test]
    async fn move_block_swaps_within_slot() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        let a = insert(&db, &input("top", "a"), None).await.unwrap();
        let _b = insert(&db, &input("top", "b"), None).await.unwrap();
        let c = insert(&db, &input("top", "c"), None).await.unwrap();
        let order = |v: &[LandingBlock]| {
            v.iter()
                .filter(|b| b.slot == "top")
                .map(|b| b.title.clone())
                .collect::<Vec<_>>()
        };

        // Move c (last) up → swaps with b → a, c, b.
        assert!(move_block(&db, &c, true, None).await.unwrap());
        assert_eq!(order(&list_all(&db).await.unwrap()), ["a", "c", "b"]);

        // Moving a (first) up is a no-op at the slot edge.
        assert!(!move_block(&db, &a, true, None).await.unwrap());
        assert_eq!(order(&list_all(&db).await.unwrap()), ["a", "c", "b"]);

        // Move a down → swaps with c → c, a, b.
        assert!(move_block(&db, &a, false, None).await.unwrap());
        assert_eq!(order(&list_all(&db).await.unwrap()), ["c", "a", "b"]);
    }

    #[tokio::test]
    async fn list_enabled_excludes_disabled() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        let id = insert(&db, &input("top", "x"), None).await.unwrap();
        let mut off = input("top", "x");
        off.enabled = false;
        update(&db, &id, &off, None).await.unwrap();
        assert!(list_enabled(&db)
            .await
            .unwrap()
            .is_empty());
        assert_eq!(list_all(&db).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn delete_removes() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        let id = insert(&db, &input("bottom", "z"), None).await.unwrap();
        assert!(delete(&db, &id, None).await.unwrap());
        assert!(fetch_one(&db, &id).await.unwrap().is_none());
        assert!(!delete(&db, &id, None).await.unwrap());
    }

    // Proves the ported `list_enabled` reads the native BOOLEAN column
    // through the `ConfigDb::Postgres` arm and honours `WHERE enabled`.
    // Inserts are raw SQL here because the write path isn't ported yet.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn list_enabled_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pool = crate::db::open_pg(&url).await.unwrap();
        // Isolate from other runs sharing this database.
        sqlx::query("DELETE FROM landing_blocks")
            .execute(&pool)
            .await
            .unwrap();
        for (id, enabled) in [("keep", true), ("skip", false)] {
            sqlx::query(
                "INSERT INTO landing_blocks
                   (id, slot, position, enabled, title, html, csp_origins,
                    created_at, updated_at)
                 VALUES ($1, 'top', 0, $2, '', '', '', now(), now())",
            )
            .bind(id)
            .bind(enabled)
            .execute(&pool)
            .await
            .unwrap();
        }

        let enabled = list_enabled(&ConfigDb::Postgres(pool)).await.unwrap();
        let ids: Vec<&str> = enabled.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["keep"]);
        assert!(enabled[0].enabled);
    }

    // insert / update (disable) / move / reorder / delete through the
    // `ConfigDb::Postgres` arm against a real daemon — the BOOLEAN
    // `enabled` binds as `bool` here. Gated on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn blocks_crud_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pool = crate::db::open_pg(&url).await.unwrap();
        sqlx::query("DELETE FROM landing_blocks")
            .execute(&pool)
            .await
            .unwrap();
        let db = ConfigDb::Postgres(pool);

        let a = insert(&db, &input("top", "a"), Some("admin")).await.unwrap();
        let b = insert(&db, &input("top", "b"), Some("admin")).await.unwrap();
        assert_eq!(list_all(&db).await.unwrap().len(), 2);

        // Disable `a`: it drops out of list_enabled.
        let mut off = input("top", "a");
        off.enabled = false;
        assert!(update(&db, &a, &off, None).await.unwrap());
        assert!(!fetch_one(&db, &a).await.unwrap().unwrap().enabled);
        let enabled_ids: Vec<String> = list_enabled(&db)
            .await
            .unwrap()
            .into_iter()
            .map(|x| x.id)
            .collect();
        assert_eq!(enabled_ids, vec![b.clone()]);

        // Reorder top → [b, a].
        assert!(reorder(&db, "top", &[b.clone(), a.clone()], None)
            .await
            .unwrap());
        let order: Vec<String> = list_all(&db).await.unwrap().into_iter().map(|x| x.id).collect();
        assert_eq!(order, vec![b.clone(), a.clone()]);

        // move_block b down swaps it back behind a.
        assert!(move_block(&db, &b, false, None).await.unwrap());
        let order: Vec<String> = list_all(&db).await.unwrap().into_iter().map(|x| x.id).collect();
        assert_eq!(order, vec![a.clone(), b.clone()]);

        assert!(delete(&db, &a, None).await.unwrap());
        assert!(delete(&db, &b, None).await.unwrap());
        assert!(list_all(&db).await.unwrap().is_empty());
    }
}
