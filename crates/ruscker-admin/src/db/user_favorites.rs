//! Per-user favorite apps (#858) — the landing card star toggle.
//!
//! A favorite is a `(username, spec_id)` row. The public landing reads a
//! viewer's favorites to fill the star state + order the "Featured"
//! section (favorites first, then admin-pinned `featured` cards).
//! Favorites need a real user account — a break-glass token session
//! (no username) can't favorite.

use anyhow::{Context, Result};

use crate::db::ConfigDb;

/// Toggle a favorite for `(username, spec_id)`. Returns the new state:
/// `true` ⇒ now favorited, `false` ⇒ removed. Implemented as
/// delete-then-insert so it's a single round-trip in the common cases
/// and naturally idempotent.
pub async fn toggle(db: &ConfigDb, username: &str, spec_id: &str) -> Result<bool> {
    let removed = match db {
        ConfigDb::Sqlite(p) => {
            sqlx::query("DELETE FROM user_favorites WHERE username = ? AND spec_id = ?")
                .bind(username)
                .bind(spec_id)
                .execute(p)
                .await
                .context("toggle favorite (delete, sqlite)")?
                .rows_affected()
        }
        ConfigDb::Postgres(p) => {
            sqlx::query("DELETE FROM user_favorites WHERE username = $1 AND spec_id = $2")
                .bind(username)
                .bind(spec_id)
                .execute(p)
                .await
                .context("toggle favorite (delete, pg)")?
                .rows_affected()
        }
    } > 0;
    if removed {
        return Ok(false);
    }
    match db {
        ConfigDb::Sqlite(p) => {
            sqlx::query(
                "INSERT INTO user_favorites (username, spec_id) VALUES (?, ?)
                 ON CONFLICT DO NOTHING",
            )
            .bind(username)
            .bind(spec_id)
            .execute(p)
            .await
            .context("toggle favorite (insert, sqlite)")?;
        }
        ConfigDb::Postgres(p) => {
            sqlx::query(
                "INSERT INTO user_favorites (username, spec_id) VALUES ($1, $2)
                 ON CONFLICT DO NOTHING",
            )
            .bind(username)
            .bind(spec_id)
            .execute(p)
            .await
            .context("toggle favorite (insert, pg)")?;
        }
    }
    Ok(true)
}

/// The spec ids `username` has favorited, ordered by `spec_id` for a
/// stable carousel order. Empty for an unknown user.
pub async fn list(db: &ConfigDb, username: &str) -> Result<Vec<String>> {
    let rows: Vec<(String,)> = match db {
        ConfigDb::Sqlite(p) => {
            sqlx::query_as("SELECT spec_id FROM user_favorites WHERE username = ? ORDER BY spec_id")
                .bind(username)
                .fetch_all(p)
                .await
        }
        ConfigDb::Postgres(p) => {
            sqlx::query_as("SELECT spec_id FROM user_favorites WHERE username = $1 ORDER BY spec_id")
                .bind(username)
                .fetch_all(p)
                .await
        }
    }
    .context("list favorites")?;
    Ok(rows.into_iter().map(|(s,)| s).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    #[tokio::test]
    async fn toggle_and_list_roundtrip() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        assert!(list(&db, "alice").await.unwrap().is_empty());

        // First toggle adds it.
        assert!(toggle(&db, "alice", "ips").await.unwrap());
        assert!(toggle(&db, "alice", "aurora").await.unwrap());
        // Ordered by spec_id.
        assert_eq!(list(&db, "alice").await.unwrap(), vec!["aurora", "ips"]);
        // Scoped per user.
        assert!(list(&db, "bob").await.unwrap().is_empty());

        // Second toggle removes it.
        assert!(!toggle(&db, "alice", "ips").await.unwrap());
        assert_eq!(list(&db, "alice").await.unwrap(), vec!["aurora"]);
    }
}
