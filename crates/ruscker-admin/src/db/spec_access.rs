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

/// Today's bucket key in UTC, `YYYY-MM-DD`. `pub(crate)` so the
/// in-memory aggregator ([`crate::access_counter`]) buckets increments
/// with the same key it will later flush under.
pub(crate) fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// Record one access for `spec_id` (today's bucket, +1). Callers treat
/// this as best-effort — a counter hiccup must never break the request
/// being served. The proxy hot path doesn't call this directly anymore
/// (#944) — it buffers in [`crate::access_counter`], which flushes here
/// via [`record_delta`].
pub async fn record(db: &ConfigDb, spec_id: &str) -> Result<()> {
    record_delta(db, spec_id, &today(), 1).await
}

/// Add `delta` accesses to the `(spec_id, day)` bucket in one atomic
/// UPSERT (#944). The flush task calls this once per touched bucket per
/// window, so the write rate tracks specs × flush windows — not the
/// request rate.
pub async fn record_delta(db: &ConfigDb, spec_id: &str, day: &str, delta: i64) -> Result<()> {
    match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query(
                "INSERT INTO spec_access (spec_id, day, count) VALUES (?, ?, ?)
                 ON CONFLICT(spec_id, day) DO UPDATE SET count = count + excluded.count",
            )
            .bind(spec_id)
            .bind(day)
            .bind(delta)
            .execute(pool)
            .await
            .context("record spec access delta (sqlite)")?;
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO spec_access (spec_id, day, count) VALUES ($1, $2, $3)
                 ON CONFLICT (spec_id, day) DO UPDATE
                     SET count = spec_access.count + EXCLUDED.count",
            )
            .bind(spec_id)
            .bind(day)
            .bind(delta)
            .execute(pool)
            .await
            .context("record spec access delta (postgres)")?;
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

/// Per-spec daily series for the last `days` days (oldest → newest),
/// 0-filled for days with no access. Drives the admin sparkline (#549
/// follow-up). Specs with no access in the window are absent.
pub async fn recent_series(db: &ConfigDb, days: usize) -> Result<HashMap<String, Vec<i64>>> {
    let today = chrono::Utc::now().date_naive();
    // Day keys oldest → newest, e.g. [d-13, …, d-1, d] for days=14.
    let keys: Vec<String> = (0..days)
        .rev()
        .map(|i| {
            (today - chrono::Duration::days(i as i64))
                .format("%Y-%m-%d")
                .to_string()
        })
        .collect();
    let since = keys.first().cloned().unwrap_or_default();

    let rows: Vec<(String, String, i64)> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as("SELECT spec_id, day, count FROM spec_access WHERE day >= ?")
                .bind(&since)
                .fetch_all(pool)
                .await
                .context("spec access series (sqlite)")?
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as("SELECT spec_id, day, count FROM spec_access WHERE day >= $1")
                .bind(&since)
                .fetch_all(pool)
                .await
                .context("spec access series (postgres)")?
        }
    };

    let mut grouped: HashMap<String, HashMap<String, i64>> = HashMap::new();
    for (spec_id, day, count) in rows {
        grouped.entry(spec_id).or_default().insert(day, count);
    }
    let out = grouped
        .into_iter()
        .map(|(spec, daymap)| {
            let series = keys
                .iter()
                .map(|k| daymap.get(k).copied().unwrap_or(0))
                .collect();
            (spec, series)
        })
        .collect();
    Ok(out)
}

/// Render a tiny inline-SVG line sparkline from a daily series. Returns
/// an empty string when there's nothing to show (all-zero or empty), so
/// the template can skip it. The SVG is generated here from plain numbers
/// — no user input — so it's safe to render unescaped.
pub fn sparkline_svg(values: &[i64]) -> String {
    if values.len() < 2 || values.iter().all(|&v| v == 0) {
        return String::new();
    }
    let (w, h, pad) = (64.0_f64, 18.0_f64, 1.5_f64);
    let max = (*values.iter().max().unwrap_or(&1)).max(1) as f64;
    let step = w / (values.len() - 1) as f64;
    let points = values
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = i as f64 * step;
            let y = h - pad - (v as f64 / max) * (h - 2.0 * pad);
            format!("{x:.1},{y:.1}")
        })
        .collect::<Vec<_>>()
        .join(" ");
    format!(
        "<svg viewBox=\"0 0 {w:.0} {h:.0}\" width=\"{w:.0}\" height=\"{h:.0}\" \
         class=\"sparkline\" aria-hidden=\"true\"><polyline fill=\"none\" \
         stroke=\"currentColor\" stroke-width=\"1.5\" stroke-linejoin=\"round\" \
         stroke-linecap=\"round\" points=\"{points}\"/></svg>"
    )
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

    #[tokio::test]
    async fn recent_series_aligns_to_days_and_zero_fills() {
        let db = mem_db().await;
        record(&db, "a").await.unwrap();
        record(&db, "a").await.unwrap();
        let series = recent_series(&db, 7).await.unwrap();
        let a = series.get("a").expect("a present");
        assert_eq!(a.len(), 7);
        assert_eq!(*a.last().unwrap(), 2, "today's bucket is the last value");
        assert!(a[..6].iter().all(|&v| v == 0), "earlier days zero-filled");
    }

    #[test]
    fn sparkline_empty_when_flat_else_polyline() {
        assert_eq!(sparkline_svg(&[]), "");
        assert_eq!(sparkline_svg(&[0, 0, 0]), "");
        assert!(sparkline_svg(&[0, 1, 3, 2]).contains("<polyline"));
    }

    #[tokio::test]
    async fn record_delta_upserts_and_accumulates() {
        let db = mem_db().await;
        record_delta(&db, "a", "2026-07-12", 41).await.unwrap();
        record_delta(&db, "a", "2026-07-12", 1).await.unwrap();
        record_delta(&db, "a", "2026-07-13", 8).await.unwrap();
        let totals = totals(&db).await.expect("totals");
        assert_eq!(totals.get("a"), Some(&50));
    }

    // The delta UPSERT against a real Postgres (its `EXCLUDED`-qualified
    // conflict arm differs from SQLite's). Gated on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn record_delta_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pg = crate::db::open_pg(&url).await.unwrap();
        sqlx::query("DELETE FROM spec_access")
            .execute(&pg)
            .await
            .unwrap();
        let db = ConfigDb::Postgres(pg);

        record_delta(&db, "a", "2026-07-12", 41).await.unwrap();
        record_delta(&db, "a", "2026-07-12", 1).await.unwrap();
        record_delta(&db, "b", "2026-07-12", 7).await.unwrap();
        let totals = totals(&db).await.expect("totals");
        assert_eq!(totals.get("a"), Some(&42));
        assert_eq!(totals.get("b"), Some(&7));
    }
}
