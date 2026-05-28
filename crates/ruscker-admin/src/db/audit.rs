//! Audit log read API.
//!
//! Writes happen across the codebase — every repository method
//! that mutates state inserts an `audit_log` row in the same
//! transaction as the change. This module is read-only and powers
//! the `/admin/audit` page.
//!
//! Filtering is intentionally permissive: every field on
//! [`AuditFilter`] is `Option`, and `None` means "don't restrict
//! on this dimension". Combined with pagination, this is enough
//! for the MVP. A full-text search over `diff_json` is a Phase
//! 2.5+ refinement.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::db::ConfigDb;

/// One row of `audit_log`, with `diff_json` parsed when valid.
#[derive(Debug, Clone)]
pub struct AuditEntry {
    pub id: i64,
    pub actor: Option<String>,
    /// e.g. `spec.update`, `image.upload`, `credential.delete`.
    pub action: String,
    /// e.g. `spec:sales-dashboard`, `image:abc-123`. `None` for
    /// global events (system imports).
    pub target: Option<String>,
    /// Parsed `diff_json`. `None` when the column was NULL or
    /// the JSON was unparseable (we don't fail the listing for
    /// one bad row; the operator gets `<malformed>` in the UI).
    pub diff: Option<serde_json::Value>,
    pub occurred_at: DateTime<Utc>,
}

/// Top-level action category. Maps to the prefix before the dot
/// in `audit_log.action` (e.g. `spec.*` → `Spec`). Used by the
/// page's filter dropdown so operators don't have to know exact
/// action names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionFamily {
    Spec,
    Image,
    Credential,
    Landing,
}

impl ActionFamily {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "spec" => Self::Spec,
            "image" => Self::Image,
            "credential" => Self::Credential,
            "landing" => Self::Landing,
            _ => return None,
        })
    }
    pub fn as_prefix(self) -> &'static str {
        match self {
            Self::Spec => "spec.",
            Self::Image => "image.",
            Self::Credential => "credential.",
            Self::Landing => "landing.",
        }
    }
}

#[derive(Debug, Default, Clone)]
pub struct AuditFilter {
    pub family: Option<ActionFamily>,
    pub target_contains: Option<String>,
    pub actor: Option<String>,
    pub limit: i64,
}

impl AuditFilter {
    pub fn new() -> Self {
        Self {
            limit: 100,
            ..Default::default()
        }
    }
}

/// Row tuple shared by both dialect arms of [`list`].
type ListRow = (
    i64,
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    DateTime<Utc>,
);

/// Append the dynamic WHERE/ORDER/LIMIT for [`list`] onto a
/// [`sqlx::QueryBuilder`]. Generic over the database so the SQLite and
/// Postgres arms share one filter definition — `push_bind` emits the
/// right placeholder (`?` vs `$n`) for each backend automatically.
fn push_filters<DB>(qb: &mut sqlx::QueryBuilder<DB>, filter: &AuditFilter)
where
    DB: sqlx::Database,
    String: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB>,
    i64: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB>,
{
    if let Some(family) = filter.family {
        qb.push(" AND action LIKE ");
        qb.push_bind(format!("{}%", family.as_prefix()));
    }
    if let Some(t) = &filter.target_contains {
        qb.push(" AND target LIKE ");
        qb.push_bind(format!("%{}%", t));
    }
    if let Some(a) = &filter.actor {
        qb.push(" AND actor = ");
        qb.push_bind(a.clone());
    }
    qb.push(" ORDER BY id DESC LIMIT ");
    qb.push_bind(filter.limit.clamp(1, 1000));
}

const LIST_SELECT: &str =
    "SELECT id, actor, action, target, diff_json, occurred_at FROM audit_log WHERE 1=1";

/// List rows matching `filter`, newest first.
///
/// Dynamic WHERE built via [`sqlx::QueryBuilder`] (the typed
/// `query_as` family takes `&'static str` only). All values are
/// bound through `push_bind`; no string interpolation, no SQL
/// injection.
pub async fn list(db: &ConfigDb, filter: &AuditFilter) -> Result<Vec<AuditEntry>> {
    let rows: Vec<ListRow> = match db {
        ConfigDb::Sqlite(pool) => {
            let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(LIST_SELECT);
            push_filters(&mut qb, filter);
            qb.build_query_as().fetch_all(pool).await
        }
        ConfigDb::Postgres(pool) => {
            let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(LIST_SELECT);
            push_filters(&mut qb, filter);
            qb.build_query_as().fetch_all(pool).await
        }
    }
    .context("list audit_log")?;

    Ok(rows
        .into_iter()
        .map(|(id, actor, action, target, diff_json, occurred_at)| AuditEntry {
            id,
            actor,
            action,
            target,
            diff: diff_json
                .as_deref()
                .and_then(|s| serde_json::from_str(s).ok()),
            occurred_at,
        })
        .collect())
}

/// Distinct values of `action` currently in the table — used to
/// populate the filter dropdown so the UI doesn't enumerate
/// actions that have never fired in this install.
pub async fn distinct_actions(db: &ConfigDb) -> Result<Vec<String>> {
    let sql = "SELECT DISTINCT action FROM audit_log ORDER BY action ASC";
    let rows: Vec<(String,)> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    }
    .context("distinct actions")?;
    Ok(rows.into_iter().map(|(a,)| a).collect())
}

/// Distinct non-null `actor` values — populates the actor filter
/// dropdown (only worth showing on multi-operator installs).
pub async fn distinct_actors(db: &ConfigDb) -> Result<Vec<String>> {
    let sql = "SELECT DISTINCT actor FROM audit_log WHERE actor IS NOT NULL ORDER BY actor ASC";
    let rows: Vec<(String,)> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    }
    .context("distinct actors")?;
    Ok(rows.into_iter().map(|(a,)| a).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;
    use chrono::Utc;

    async fn seed(db: &ConfigDb, action: &str, target: Option<&str>, actor: Option<&str>) {
        let pool = db.as_sqlite().unwrap();
        sqlx::query(
            "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
             VALUES (?, ?, ?, NULL, ?)",
        )
        .bind(actor)
        .bind(action)
        .bind(target)
        .bind(Utc::now())
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn list_returns_newest_first() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        seed(&db, "spec.create", Some("spec:a"), None).await;
        seed(&db, "spec.update", Some("spec:a"), None).await;
        seed(&db, "spec.delete", Some("spec:a"), None).await;

        let v = list(&db, &AuditFilter::new()).await.unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].action, "spec.delete");
        assert_eq!(v[2].action, "spec.create");
    }

    #[tokio::test]
    async fn family_filter_narrows_by_prefix() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        seed(&db, "spec.create", Some("spec:a"), None).await;
        seed(&db, "image.upload", Some("image:1"), None).await;
        seed(&db, "credential.create", Some("credential:dh"), None).await;

        let f = AuditFilter {
            family: Some(ActionFamily::Image),
            ..AuditFilter::new()
        };
        let v = list(&db, &f).await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].action, "image.upload");
    }

    #[tokio::test]
    async fn target_substring_filter() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        seed(&db, "spec.update", Some("spec:sales-dashboard"), None).await;
        seed(&db, "spec.update", Some("spec:ops-report"), None).await;

        let f = AuditFilter {
            target_contains: Some("sales".into()),
            ..AuditFilter::new()
        };
        let v = list(&db, &f).await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].target.as_deref(), Some("spec:sales-dashboard"));
    }

    #[tokio::test]
    async fn limit_caps_returned_rows() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        for i in 0..10 {
            seed(&db, "spec.create", Some(&format!("spec:{i}")), None).await;
        }
        let f = AuditFilter {
            limit: 3,
            ..AuditFilter::new()
        };
        let v = list(&db, &f).await.unwrap();
        assert_eq!(v.len(), 3);
    }

    // list (newest-first + family filter via QueryBuilder<Postgres>) +
    // distinct_actions / distinct_actors against a real daemon. Gated
    // on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn audit_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pg = crate::db::open_pg(&url).await.unwrap();
        sqlx::query("DELETE FROM audit_log")
            .execute(&pg)
            .await
            .unwrap();
        for (action, target, actor) in [
            ("spec.create", "spec:a", Some("root")),
            ("image.upload", "image:1", None),
            ("spec.update", "spec:a", Some("ed")),
        ] {
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, $2, $3, NULL, now())",
            )
            .bind(actor)
            .bind(action)
            .bind(target)
            .execute(&pg)
            .await
            .unwrap();
        }
        let db = ConfigDb::Postgres(pg);

        let all = list(&db, &AuditFilter::new()).await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].action, "spec.update", "newest first");

        let f = AuditFilter {
            family: Some(ActionFamily::Image),
            ..AuditFilter::new()
        };
        assert_eq!(list(&db, &f).await.unwrap().len(), 1);

        assert!(distinct_actions(&db)
            .await
            .unwrap()
            .contains(&"spec.create".to_string()));
        assert_eq!(
            distinct_actors(&db).await.unwrap(),
            vec!["ed".to_string(), "root".to_string()]
        );
    }
}
