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
use sqlx::SqlitePool;

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

/// List rows matching `filter`, newest first.
///
/// Dynamic WHERE built via [`sqlx::QueryBuilder`] (the typed
/// `query_as` family takes `&'static str` only). All values are
/// bound through `push_bind`; no string interpolation, no SQL
/// injection.
pub async fn list(pool: &SqlitePool, filter: &AuditFilter) -> Result<Vec<AuditEntry>> {
    let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(
        "SELECT id, actor, action, target, diff_json, occurred_at FROM audit_log WHERE 1=1",
    );
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
    qb.push_bind(filter.limit.max(1).min(1000));

    let rows: Vec<(i64, Option<String>, String, Option<String>, Option<String>, DateTime<Utc>)> =
        qb.build_query_as()
            .fetch_all(pool)
            .await
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
pub async fn distinct_actions(pool: &SqlitePool) -> Result<Vec<String>> {
    let rows: Vec<(String,)> =
        sqlx::query_as("SELECT DISTINCT action FROM audit_log ORDER BY action ASC")
            .fetch_all(pool)
            .await
            .context("distinct actions")?;
    Ok(rows.into_iter().map(|(a,)| a).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;
    use chrono::Utc;

    async fn seed(pool: &SqlitePool, action: &str, target: Option<&str>, actor: Option<&str>) {
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
        let pool = open_memory().await.unwrap();
        seed(&pool, "spec.create", Some("spec:a"), None).await;
        seed(&pool, "spec.update", Some("spec:a"), None).await;
        seed(&pool, "spec.delete", Some("spec:a"), None).await;

        let v = list(&pool, &AuditFilter::new()).await.unwrap();
        assert_eq!(v.len(), 3);
        assert_eq!(v[0].action, "spec.delete");
        assert_eq!(v[2].action, "spec.create");
    }

    #[tokio::test]
    async fn family_filter_narrows_by_prefix() {
        let pool = open_memory().await.unwrap();
        seed(&pool, "spec.create", Some("spec:a"), None).await;
        seed(&pool, "image.upload", Some("image:1"), None).await;
        seed(&pool, "credential.create", Some("credential:dh"), None).await;

        let f = AuditFilter {
            family: Some(ActionFamily::Image),
            ..AuditFilter::new()
        };
        let v = list(&pool, &f).await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].action, "image.upload");
    }

    #[tokio::test]
    async fn target_substring_filter() {
        let pool = open_memory().await.unwrap();
        seed(&pool, "spec.update", Some("spec:sales-dashboard"), None).await;
        seed(&pool, "spec.update", Some("spec:ops-report"), None).await;

        let f = AuditFilter {
            target_contains: Some("sales".into()),
            ..AuditFilter::new()
        };
        let v = list(&pool, &f).await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].target.as_deref(), Some("spec:sales-dashboard"));
    }

    #[tokio::test]
    async fn limit_caps_returned_rows() {
        let pool = open_memory().await.unwrap();
        for i in 0..10 {
            seed(&pool, "spec.create", Some(&format!("spec:{i}")), None).await;
        }
        let f = AuditFilter {
            limit: 3,
            ..AuditFilter::new()
        };
        let v = list(&pool, &f).await.unwrap();
        assert_eq!(v.len(), 3);
    }
}
