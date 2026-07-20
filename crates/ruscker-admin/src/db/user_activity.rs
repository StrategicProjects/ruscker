//! User-activity store (#1021).
//!
//! Append-only persistence for [`crate::activity`] — one row per login or
//! interactive app access, with identity. Writes go through
//! [`insert_batch`] (called by the supervised drain task, never inline on
//! the proxy hot path); reads power the "Atividades dos usuários" table via
//! [`list_page`] + [`count_filtered`] (server-side pagination, like
//! `db::users`).
//!
//! Every value is bound through `push_bind` — no string interpolation, no
//! SQL injection. The dynamic WHERE is shared across both dialects via a
//! generic [`sqlx::QueryBuilder`], which emits `?` for SQLite and `$n` for
//! Postgres automatically.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::activity::ActivityEvent;
use crate::db::ConfigDb;

/// One row of `user_activity` for the admin table. `event_key` is an
/// internal dedup key and deliberately not surfaced here.
#[derive(Debug, Clone)]
pub struct ActivityRow {
    pub id: i64,
    /// `login.success` | `app.access`.
    pub event_type: String,
    pub username: Option<String>,
    pub spec_id: Option<String>,
    pub auth_method: Option<String>,
    pub client_ip: Option<String>,
    pub occurred_at: DateTime<Utc>,
}

/// Read filter for [`list_page`] / [`count_filtered`]. Every field is
/// optional (`None` ⇒ don't restrict on that dimension). `limit`/`offset`
/// drive server-side pagination.
#[derive(Debug, Clone)]
pub struct ActivityFilter {
    pub event_type: Option<String>,
    pub username: Option<String>,
    pub spec_id: Option<String>,
    /// Inclusive lower bound on `occurred_at`.
    pub since: Option<DateTime<Utc>>,
    /// Inclusive upper bound on `occurred_at`.
    pub until: Option<DateTime<Utc>>,
    pub limit: i64,
    pub offset: i64,
}

impl Default for ActivityFilter {
    fn default() -> Self {
        Self {
            event_type: None,
            username: None,
            spec_id: None,
            since: None,
            until: None,
            limit: 50,
            offset: 0,
        }
    }
}

/// Row tuple shared by both dialect arms of [`list_page`].
type ListRow = (
    i64,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    DateTime<Utc>,
);

/// Push the shared WHERE predicates onto a builder that already ends in a
/// truthy anchor (`WHERE 1=1`). Generic over the database so SQLite and
/// Postgres share one filter definition.
fn push_where<DB>(qb: &mut sqlx::QueryBuilder<DB>, filter: &ActivityFilter)
where
    DB: sqlx::Database,
    String: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB>,
    DateTime<Utc>: sqlx::Type<DB> + for<'q> sqlx::Encode<'q, DB>,
{
    if let Some(t) = &filter.event_type {
        qb.push(" AND event_type = ");
        qb.push_bind(t.clone());
    }
    if let Some(u) = &filter.username {
        qb.push(" AND username = ");
        qb.push_bind(u.clone());
    }
    if let Some(s) = &filter.spec_id {
        qb.push(" AND spec_id = ");
        qb.push_bind(s.clone());
    }
    if let Some(since) = filter.since {
        qb.push(" AND occurred_at >= ");
        qb.push_bind(since);
    }
    if let Some(until) = filter.until {
        qb.push(" AND occurred_at <= ");
        qb.push_bind(until);
    }
}

const LIST_SELECT: &str = "SELECT id, event_type, username, spec_id, auth_method, client_ip, \
     occurred_at FROM user_activity WHERE 1=1";
const COUNT_SELECT: &str = "SELECT COUNT(*) FROM user_activity WHERE 1=1";

/// Insert a batch of events in one transaction, newest kept. The unique
/// index on `(event_type, event_key)` + `ON CONFLICT DO NOTHING` deduplicates
/// the same session logged by two HA instances. Returns the number of rows
/// actually inserted (conflicts are skipped, not counted).
pub async fn insert_batch(db: &ConfigDb, events: &[ActivityEvent]) -> Result<u64> {
    if events.is_empty() {
        return Ok(0);
    }
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("activity insert begin (sqlite)")?;
            let mut inserted = 0u64;
            for e in events {
                let res = sqlx::query(
                    "INSERT INTO user_activity
                        (event_type, username, spec_id, event_key, auth_method, client_ip, occurred_at)
                     VALUES (?, ?, ?, ?, ?, ?, ?)
                     ON CONFLICT (event_type, event_key) DO NOTHING",
                )
                .bind(e.event_type.as_str())
                .bind(e.username.as_deref())
                .bind(e.spec_id.as_deref())
                .bind(&e.event_key)
                .bind(e.auth_method.map(|m| m.as_str()))
                .bind(e.client_ip.as_deref())
                .bind(e.occurred_at)
                .execute(&mut *tx)
                .await
                .context("activity insert (sqlite)")?;
                inserted += res.rows_affected();
            }
            tx.commit().await.context("activity insert commit (sqlite)")?;
            Ok(inserted)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("activity insert begin (pg)")?;
            let mut inserted = 0u64;
            for e in events {
                let res = sqlx::query(
                    "INSERT INTO user_activity
                        (event_type, username, spec_id, event_key, auth_method, client_ip, occurred_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7)
                     ON CONFLICT (event_type, event_key) DO NOTHING",
                )
                .bind(e.event_type.as_str())
                .bind(e.username.as_deref())
                .bind(e.spec_id.as_deref())
                .bind(&e.event_key)
                .bind(e.auth_method.map(|m| m.as_str()))
                .bind(e.client_ip.as_deref())
                .bind(e.occurred_at)
                .execute(&mut *tx)
                .await
                .context("activity insert (pg)")?;
                inserted += res.rows_affected();
            }
            tx.commit().await.context("activity insert commit (pg)")?;
            Ok(inserted)
        }
    }
}

/// One page of activity rows matching `filter`, newest first.
pub async fn list_page(db: &ConfigDb, filter: &ActivityFilter) -> Result<Vec<ActivityRow>> {
    let limit = filter.limit.clamp(1, 500);
    let offset = filter.offset.max(0);
    let rows: Vec<ListRow> = match db {
        ConfigDb::Sqlite(pool) => {
            let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(LIST_SELECT);
            push_where(&mut qb, filter);
            qb.push(" ORDER BY id DESC LIMIT ");
            qb.push_bind(limit);
            qb.push(" OFFSET ");
            qb.push_bind(offset);
            qb.build_query_as().fetch_all(pool).await
        }
        ConfigDb::Postgres(pool) => {
            let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(LIST_SELECT);
            push_where(&mut qb, filter);
            qb.push(" ORDER BY id DESC LIMIT ");
            qb.push_bind(limit);
            qb.push(" OFFSET ");
            qb.push_bind(offset);
            qb.build_query_as().fetch_all(pool).await
        }
    }
    .context("list user_activity")?;

    Ok(rows
        .into_iter()
        .map(
            |(id, event_type, username, spec_id, auth_method, client_ip, occurred_at)| {
                ActivityRow {
                    id,
                    event_type,
                    username,
                    spec_id,
                    auth_method,
                    client_ip,
                    occurred_at,
                }
            },
        )
        .collect())
}

/// Total rows matching `filter` (ignores limit/offset) — for the pager.
pub async fn count_filtered(db: &ConfigDb, filter: &ActivityFilter) -> Result<i64> {
    let (count,): (i64,) = match db {
        ConfigDb::Sqlite(pool) => {
            let mut qb: sqlx::QueryBuilder<sqlx::Sqlite> = sqlx::QueryBuilder::new(COUNT_SELECT);
            push_where(&mut qb, filter);
            qb.build_query_as().fetch_one(pool).await
        }
        ConfigDb::Postgres(pool) => {
            let mut qb: sqlx::QueryBuilder<sqlx::Postgres> = sqlx::QueryBuilder::new(COUNT_SELECT);
            push_where(&mut qb, filter);
            qb.build_query_as().fetch_one(pool).await
        }
    }
    .context("count user_activity")?;
    Ok(count)
}

/// Distinct non-null usernames ever recorded — populates the user filter
/// dropdown. Ordered so the select is stable.
pub async fn distinct_usernames(db: &ConfigDb) -> Result<Vec<String>> {
    let sql =
        "SELECT DISTINCT username FROM user_activity WHERE username IS NOT NULL ORDER BY username ASC";
    let rows: Vec<(String,)> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    }
    .context("distinct activity usernames")?;
    Ok(rows.into_iter().map(|(u,)| u).collect())
}

/// Distinct non-null spec ids ever recorded — populates the app filter
/// dropdown.
pub async fn distinct_specs(db: &ConfigDb) -> Result<Vec<String>> {
    let sql =
        "SELECT DISTINCT spec_id FROM user_activity WHERE spec_id IS NOT NULL ORDER BY spec_id ASC";
    let rows: Vec<(String,)> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    }
    .context("distinct activity specs")?;
    Ok(rows.into_iter().map(|(s,)| s).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::{ActivityEvent, AuthMethod};
    use crate::db::open_memory;

    fn login(user: &str, key: &str) -> ActivityEvent {
        ActivityEvent::login_success(user, AuthMethod::Password, key, None)
    }
    fn access(user: Option<&str>, spec: &str, key: &str) -> ActivityEvent {
        ActivityEvent::app_access(user.map(String::from), spec, key, None, None)
    }

    #[tokio::test]
    async fn list_returns_newest_first_and_counts() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        insert_batch(&db, &[login("alice", "l1"), access(Some("alice"), "sales", "a1")])
            .await
            .unwrap();
        insert_batch(&db, &[access(Some("bob"), "ops", "a2")])
            .await
            .unwrap();

        let all = list_page(&db, &ActivityFilter::default()).await.unwrap();
        assert_eq!(all.len(), 3);
        assert_eq!(all[0].spec_id.as_deref(), Some("ops"), "newest first");
        assert_eq!(
            count_filtered(&db, &ActivityFilter::default())
                .await
                .unwrap(),
            3
        );
    }

    #[tokio::test]
    async fn filters_by_event_type_username_and_spec() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        insert_batch(
            &db,
            &[
                login("alice", "l1"),
                access(Some("alice"), "sales", "a1"),
                access(Some("bob"), "sales", "a2"),
                access(None, "public", "a3"),
            ],
        )
        .await
        .unwrap();

        let by_type = ActivityFilter {
            event_type: Some("login.success".into()),
            ..Default::default()
        };
        assert_eq!(count_filtered(&db, &by_type).await.unwrap(), 1);

        let by_user = ActivityFilter {
            username: Some("alice".into()),
            ..Default::default()
        };
        // alice's login + alice's app access.
        assert_eq!(count_filtered(&db, &by_user).await.unwrap(), 2);

        let by_spec = ActivityFilter {
            spec_id: Some("sales".into()),
            ..Default::default()
        };
        assert_eq!(count_filtered(&db, &by_spec).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn pagination_pages_through_with_stable_total() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        let events: Vec<_> = (0..10)
            .map(|i| access(Some("u"), "s", &format!("k{i}")))
            .collect();
        insert_batch(&db, &events).await.unwrap();

        let total = count_filtered(&db, &ActivityFilter::default())
            .await
            .unwrap();
        assert_eq!(total, 10);

        let page1 = list_page(
            &db,
            &ActivityFilter {
                limit: 4,
                offset: 0,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let page3 = list_page(
            &db,
            &ActivityFilter {
                limit: 4,
                offset: 8,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(page1.len(), 4);
        assert_eq!(page3.len(), 2, "last page has the remainder");
        // No overlap between pages (ids strictly decreasing across pages).
        assert!(page1.last().unwrap().id > page3.first().unwrap().id);
    }

    #[tokio::test]
    async fn distinct_dropdowns_exclude_nulls_and_sort() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        insert_batch(
            &db,
            &[
                access(Some("bob"), "ops", "a1"),
                access(Some("alice"), "sales", "a2"),
                access(None, "public", "a3"), // anonymous → no username
                login("alice", "l1"),         // login → no spec
            ],
        )
        .await
        .unwrap();
        assert_eq!(
            distinct_usernames(&db).await.unwrap(),
            vec!["alice".to_string(), "bob".to_string()]
        );
        assert_eq!(
            distinct_specs(&db).await.unwrap(),
            vec!["ops".to_string(), "public".to_string(), "sales".to_string()]
        );
    }

    #[tokio::test]
    async fn since_until_filter_bounds_the_time_window() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        // Three events at distinct, explicit timestamps (bypass the emit-time
        // stamp so the window is deterministic).
        let mk = |key: &str, ts: DateTime<Utc>| {
            let mut e = access(Some("u"), "s", key);
            e.occurred_at = ts;
            e
        };
        let t = |h: u32| {
            format!("2026-07-20T{h:02}:00:00Z")
                .parse::<DateTime<Utc>>()
                .unwrap()
        };
        insert_batch(&db, &[mk("k08", t(8)), mk("k12", t(12)), mk("k18", t(18))])
            .await
            .unwrap();

        // Window [10:00, 15:00] captures only the 12:00 event.
        let f = ActivityFilter {
            since: Some(t(10)),
            until: Some(t(15)),
            ..Default::default()
        };
        assert_eq!(count_filtered(&db, &f).await.unwrap(), 1);
        let rows = list_page(&db, &f).await.unwrap();
        assert_eq!(rows.len(), 1);

        // since-only lower bound (inclusive) keeps 12:00 and 18:00.
        let f = ActivityFilter {
            since: Some(t(12)),
            ..Default::default()
        };
        assert_eq!(count_filtered(&db, &f).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn insert_dedups_on_event_type_and_key() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        // Same key, same type → deduped to one.
        assert_eq!(insert_batch(&db, &[access(Some("a"), "s", "dup")]).await.unwrap(), 1);
        assert_eq!(insert_batch(&db, &[access(Some("a"), "s", "dup")]).await.unwrap(), 0);
        // Same key but DIFFERENT type is a distinct event (login vs access).
        assert_eq!(insert_batch(&db, &[login("a", "dup")]).await.unwrap(), 1);
        assert_eq!(
            count_filtered(&db, &ActivityFilter::default())
                .await
                .unwrap(),
            2
        );
    }

    // insert_batch + list_page/count_filtered (QueryBuilder<Postgres>, the
    // dedup ON CONFLICT, DateTime binding) against a real daemon. Gated on
    // `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn user_activity_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pg = crate::db::open_pg(&url).await.unwrap();
        sqlx::query("DELETE FROM user_activity")
            .execute(&pg)
            .await
            .unwrap();
        let db = ConfigDb::Postgres(pg);

        insert_batch(
            &db,
            &[
                login("alice", "l1"),
                access(Some("alice"), "sales", "a1"),
                access(None, "public", "a2"),
            ],
        )
        .await
        .unwrap();
        // Dedup across a "second instance".
        assert_eq!(
            insert_batch(&db, &[access(Some("alice"), "sales", "a1")])
                .await
                .unwrap(),
            0
        );

        assert_eq!(count_filtered(&db, &ActivityFilter::default()).await.unwrap(), 3);
        let all = list_page(&db, &ActivityFilter::default()).await.unwrap();
        assert_eq!(all[0].spec_id.as_deref(), Some("public"), "newest first");

        let by_user = ActivityFilter {
            username: Some("alice".into()),
            ..Default::default()
        };
        assert_eq!(count_filtered(&db, &by_user).await.unwrap(), 2);
        assert_eq!(all.iter().filter(|r| r.username.is_none()).count(), 1);

        // Distinct dropdowns exclude NULLs and are ordered.
        assert_eq!(distinct_usernames(&db).await.unwrap(), vec!["alice".to_string()]);
        assert_eq!(
            distinct_specs(&db).await.unwrap(),
            vec!["public".to_string(), "sales".to_string()]
        );
    }
}
