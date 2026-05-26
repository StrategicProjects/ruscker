//! Custom landing HTML blocks (#54).
//!
//! Operator-authored HTML rendered in fixed landing slots (`top`
//! after the header, `bottom` after the card grid). Admin-managed and
//! DB-only for now (not part of the YAML import/export round-trip).
//! `position` orders blocks within a slot; `csp_origins` (space-
//! separated) widens the landing CSP so embedded third-party content
//! can load.

use anyhow::{Context, Result};
use chrono::Utc;
use sqlx::SqlitePool;

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

type Row = (String, String, i64, i64, String, String, String);

fn row_to_block((id, slot, position, enabled, title, html, csp_origins): Row) -> LandingBlock {
    LandingBlock {
        id,
        slot,
        position,
        enabled: enabled != 0,
        title,
        html,
        csp_origins,
    }
}

/// All blocks, ordered by slot then position — for the admin list.
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<LandingBlock>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, slot, position, enabled, title, html, csp_origins
           FROM landing_blocks ORDER BY slot, position, created_at",
    )
    .fetch_all(pool)
    .await
    .context("list landing_blocks")?;
    Ok(rows.into_iter().map(row_to_block).collect())
}

/// Enabled blocks only, ordered — for rendering the public landing.
pub async fn list_enabled(pool: &SqlitePool) -> Result<Vec<LandingBlock>> {
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, slot, position, enabled, title, html, csp_origins
           FROM landing_blocks WHERE enabled = 1 ORDER BY slot, position, created_at",
    )
    .fetch_all(pool)
    .await
    .context("list enabled landing_blocks")?;
    Ok(rows.into_iter().map(row_to_block).collect())
}

/// One block by id, or `None`.
pub async fn fetch_one(pool: &SqlitePool, id: &str) -> Result<Option<LandingBlock>> {
    let row: Option<Row> = sqlx::query_as(
        "SELECT id, slot, position, enabled, title, html, csp_origins
           FROM landing_blocks WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("fetch landing_block")?;
    Ok(row.map(row_to_block))
}

/// Create a block, appended at the end of its slot. Returns the new id.
pub async fn insert(pool: &SqlitePool, input: &BlockInput, actor: Option<&str>) -> Result<String> {
    let id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let mut tx = pool.begin().await.context("begin block insert tx")?;

    // Append: one past the current max position in the slot.
    let (next_pos,): (i64,) =
        sqlx::query_as("SELECT COALESCE(MAX(position) + 1, 0) FROM landing_blocks WHERE slot = ?")
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
    Ok(id)
}

/// Update a block's content (slot/enabled/title/html/origins). Keeps
/// its position unless the slot changed, in which case it's appended
/// to the new slot.
pub async fn update(
    pool: &SqlitePool,
    id: &str,
    input: &BlockInput,
    actor: Option<&str>,
) -> Result<bool> {
    let now = Utc::now();
    let mut tx = pool.begin().await.context("begin block update tx")?;

    let current: Option<(String,)> = sqlx::query_as("SELECT slot FROM landing_blocks WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .context("lookup block slot")?;
    let Some((current_slot,)) = current else {
        return Ok(false);
    };

    // Moving slots re-appends at the end of the destination slot.
    let position_sql = if current_slot == input.slot {
        None
    } else {
        let (next_pos,): (i64,) = sqlx::query_as(
            "SELECT COALESCE(MAX(position) + 1, 0) FROM landing_blocks WHERE slot = ?",
        )
        .bind(&input.slot)
        .fetch_one(&mut *tx)
        .await
        .context("compute new-slot position")?;
        Some(next_pos)
    };

    match position_sql {
        Some(pos) => {
            sqlx::query(
                "UPDATE landing_blocks SET slot = ?, position = ?, enabled = ?, title = ?,
                    html = ?, csp_origins = ?, updated_at = ? WHERE id = ?",
            )
            .bind(&input.slot)
            .bind(pos)
            .bind(input.enabled as i64)
            .bind(&input.title)
            .bind(&input.html)
            .bind(&input.csp_origins)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await
        }
        None => {
            sqlx::query(
                "UPDATE landing_blocks SET enabled = ?, title = ?, html = ?,
                    csp_origins = ?, updated_at = ? WHERE id = ?",
            )
            .bind(input.enabled as i64)
            .bind(&input.title)
            .bind(&input.html)
            .bind(&input.csp_origins)
            .bind(now)
            .bind(id)
            .execute(&mut *tx)
            .await
        }
    }
    .context("update landing_block")?;

    audit(&mut tx, actor, "landing_block.update", id, now).await?;
    tx.commit().await.context("commit block update")?;
    Ok(true)
}

/// Delete a block. Returns whether a row was removed.
pub async fn delete(pool: &SqlitePool, id: &str, actor: Option<&str>) -> Result<bool> {
    let now = Utc::now();
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

/// Move a block one step within its slot by swapping `position` with
/// the adjacent block (`up` = toward the front). No-op (returns
/// `false`) when the block doesn't exist or is already at the slot
/// edge. Gap-safe: it picks the nearest neighbour by position, not by
/// `position ± 1`.
pub async fn move_block(
    pool: &SqlitePool,
    id: &str,
    up: bool,
    actor: Option<&str>,
) -> Result<bool> {
    let now = Utc::now();
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
        let pool = open_memory().await.unwrap();
        insert(&pool, &input("top", "a"), None).await.unwrap();
        insert(&pool, &input("top", "b"), None).await.unwrap();
        insert(&pool, &input("bottom", "c"), None).await.unwrap();
        let all = list_all(&pool).await.unwrap();
        // bottom sorts before top alphabetically; within top, a then b.
        let top: Vec<_> = all.iter().filter(|b| b.slot == "top").collect();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].position, 0);
        assert_eq!(top[1].position, 1);
        assert_eq!(all.iter().filter(|b| b.slot == "bottom").count(), 1);
    }

    #[tokio::test]
    async fn move_block_swaps_within_slot() {
        let pool = open_memory().await.unwrap();
        let a = insert(&pool, &input("top", "a"), None).await.unwrap();
        let _b = insert(&pool, &input("top", "b"), None).await.unwrap();
        let c = insert(&pool, &input("top", "c"), None).await.unwrap();
        let order = |v: &[LandingBlock]| {
            v.iter()
                .filter(|b| b.slot == "top")
                .map(|b| b.title.clone())
                .collect::<Vec<_>>()
        };

        // Move c (last) up → swaps with b → a, c, b.
        assert!(move_block(&pool, &c, true, None).await.unwrap());
        assert_eq!(order(&list_all(&pool).await.unwrap()), ["a", "c", "b"]);

        // Moving a (first) up is a no-op at the slot edge.
        assert!(!move_block(&pool, &a, true, None).await.unwrap());
        assert_eq!(order(&list_all(&pool).await.unwrap()), ["a", "c", "b"]);

        // Move a down → swaps with c → c, a, b.
        assert!(move_block(&pool, &a, false, None).await.unwrap());
        assert_eq!(order(&list_all(&pool).await.unwrap()), ["c", "a", "b"]);
    }

    #[tokio::test]
    async fn list_enabled_excludes_disabled() {
        let pool = open_memory().await.unwrap();
        let id = insert(&pool, &input("top", "x"), None).await.unwrap();
        let mut off = input("top", "x");
        off.enabled = false;
        update(&pool, &id, &off, None).await.unwrap();
        assert!(list_enabled(&pool).await.unwrap().is_empty());
        assert_eq!(list_all(&pool).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn delete_removes() {
        let pool = open_memory().await.unwrap();
        let id = insert(&pool, &input("bottom", "z"), None).await.unwrap();
        assert!(delete(&pool, &id, None).await.unwrap());
        assert!(fetch_one(&pool, &id).await.unwrap().is_none());
        assert!(!delete(&pool, &id, None).await.unwrap());
    }
}
