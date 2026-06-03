//! Per-spec access counter (#549).
//!
//! A `(spec_id, day)` bucket bumped on each visit: an interactive app's
//! **new sticky session** (counted once per visit, so per-request traffic
//! — assets, WebSocket, polling — doesn't inflate it), or a **click on an
//! external card** (Ruscker sees it because the card routes through
//! `/app/{id}`, which 302s to the link). Cheap atomic upserts; the admin
//! shows a SUM per spec.

use super::ConfigDb;
use anyhow::{Context, Result};
use std::collections::HashMap;

/// Today's bucket key in UTC, `YYYY-MM-DD`.
fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Record one access for `spec_id` (today's bucket, +1). Atomic via
/// `ON CONFLICT`. Callers treat this as best-effort — a counter hiccup
/// must never break the request being served.
pub async fn record(db: &ConfigDb, spec_id: &str) -> Result<()> {
    let day = today();
    match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO spec_access (spec_id, day, count) VALUES (?, ?, 1)
                 ON CONFLICT(spec_id, day) DO UPDATE SET count = count + 1",
            )
            .bind(spec_id)
            .bind(&day)
            .execute(pool)
            .await
            .context("record spec access (sqlite)")?;
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO spec_access (spec_id, day, count) VALUES ($1, $2, 1)
                 ON CONFLICT (spec_id, day) DO UPDATE SET count = spec_access.count + 1",
            )
            .bind(spec_id)
            .bind(&day)
            .execute(pool)
            .await
            .context("record spec access (postgres)")?;
        }
    }
    Ok(())
}

/// Total accesses per spec (SUM over all day buckets). Used by the admin
/// Apps table; specs with no recorded access are simply absent from the
/// map (treated as 0 by the caller).
pub async fn totals(db: &ConfigDb) -> Result<HashMap<String, i64>> {
    let rows: Vec<(String, i64)> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as("SELECT spec_id, SUM(count) FROM spec_access GROUP BY spec_id")
                .fetch_all(pool)
                .await
                .context("spec access totals (sqlite)")?
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as("SELECT spec_id, SUM(count)::bigint FROM spec_access GROUP BY spec_id")
                .fetch_all(pool)
                .await
                .context("spec access totals (postgres)")?
        }
    };
    Ok(rows.into_iter().collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_db() -> ConfigDb {
        let db = crate::db::open_memory().await.expect("open in-memory");
        ConfigDb::Sqlite(db)
    }

    #[tokio::test]
    async fn record_accumulates_and_totals_sum() {
        let db = mem_db().await;
        // Three hits on `a`, one on `b`.
        for _ in 0..3 {
            record(&db, "a").await.expect("record a");
        }
        record(&db, "b").await.expect("record b");

        let totals = totals(&db).await.expect("totals");
        assert_eq!(totals.get("a"), Some(&3));
        assert_eq!(totals.get("b"), Some(&1));
        assert_eq!(totals.get("never"), None, "unseen spec absent from totals");
    }
}
