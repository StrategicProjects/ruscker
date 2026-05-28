//! Landing-page customization repository.
//!
//! Wraps the `landing_customization` singleton (id=1) — header
//! bg/fg, default intro paragraph, and per-locale intro overrides.
//! The same shape as [`ruscker_config::LandingCustomization`] so
//! import/export round-trip without translation gymnastics.

use crate::db::ConfigDb;
use anyhow::{Context, Result};
use chrono::Utc;
use ruscker_config::LandingCustomization;

/// Read the singleton row. Returns the default
/// [`LandingCustomization`] when the row hasn't been initialized
/// yet (which shouldn't normally happen — migration 0001 inserts
/// it — but defensive code keeps tests with a fresh DB happy).
///
/// First repository read ported to dual-dialect (Phase 7c-3): the
/// SELECT carries no placeholders and reads only TEXT columns, so the
/// SQL is identical on both backends — only the pool differs.
pub async fn fetch(db: &ConfigDb) -> Result<LandingCustomization> {
    type Row = (
        Option<String>, // header_bg
        Option<String>, // header_fg
        Option<String>, // intro
        String,         // intro_locales_json
        Option<String>, // seo_title
        Option<String>, // seo_description
        Option<String>, // og_image
        Option<String>, // analytics_html
        Option<String>, // analytics_origins
    );
    let sql = "SELECT header_bg, header_fg, intro, intro_locales_json,
                seo_title, seo_description, og_image,
                analytics_html, analytics_origins
           FROM landing_customization WHERE id = 1";
    let row: Option<Row> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_optional(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_optional(pool).await,
    }
    .context("load landing_customization")?;
    match row {
        None => Ok(LandingCustomization::default()),
        Some((
            bg,
            fg,
            intro,
            locales_json,
            seo_title,
            seo_description,
            og_image,
            analytics_html,
            analytics_origins,
        )) => {
            let intro_locales =
                serde_json::from_str(&locales_json).context("parse intro_locales_json")?;
            Ok(LandingCustomization {
                header_bg: bg,
                header_fg: fg,
                intro,
                intro_locales,
                seo_title,
                seo_description,
                og_image,
                analytics_html,
                analytics_origins,
                // Blocks live in their own table; callers that render
                // or export them load via `landing_blocks`.
                blocks: Vec::new(),
                // Deploy policy (#156) read from YAML config, not the
                // DB-backed editable customization.
                show_admin_link: None,
            })
        }
    }
}

/// Replace the singleton with `lc`. Empty strings on the form
/// arrive as `Some("")` from axum's Form extractor; this helper
/// collapses them to `None` so empty values disappear from the
/// exported YAML rather than serializing as `""`.
pub async fn update(
    db: &ConfigDb,
    lc: &LandingCustomization,
    actor: Option<&str>,
) -> Result<()> {
    let now = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin landing update tx")?;
            update_in_tx(&mut tx, lc, now).await?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'landing.update', 'landing:customization', NULL, ?)",
            )
            .bind(actor)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit landing.update")?;
            tx.commit().await.context("commit landing update")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin landing update tx")?;
            update_in_tx_pg(&mut tx, lc, now).await?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'landing.update', 'landing:customization', NULL, $2)",
            )
            .bind(actor)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit landing.update")?;
            tx.commit().await.context("commit landing update")?;
        }
    }
    Ok(())
}

/// Write the singleton's columns inside an existing transaction
/// (shared by [`update`] and the YAML import). Empty strings collapse
/// to NULL so they vanish from the exported YAML rather than
/// serializing as `""`.
pub(crate) async fn update_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    lc: &LandingCustomization,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    let intro_locales_json =
        serde_json::to_string(&lc.intro_locales).context("serialize intro_locales")?;
    sqlx::query(
        "UPDATE landing_customization
            SET header_bg = ?, header_fg = ?, intro = ?,
                intro_locales_json = ?, seo_title = ?, seo_description = ?,
                og_image = ?, analytics_html = ?, analytics_origins = ?,
                updated_at = ?
          WHERE id = 1",
    )
    .bind(none_if_empty(&lc.header_bg))
    .bind(none_if_empty(&lc.header_fg))
    .bind(none_if_empty(&lc.intro))
    .bind(&intro_locales_json)
    .bind(none_if_empty(&lc.seo_title))
    .bind(none_if_empty(&lc.seo_description))
    .bind(none_if_empty(&lc.og_image))
    .bind(none_if_empty(&lc.analytics_html))
    .bind(none_if_empty(&lc.analytics_origins))
    .bind(now)
    .execute(&mut **tx)
    .await
    .context("update landing_customization")?;
    Ok(())
}

/// Postgres twin of [`update_in_tx`] — `$n` placeholders. Used by the
/// Postgres arm of [`update`]. (`import_all` stays SQLite-only and
/// keeps `update_in_tx`.)
pub(crate) async fn update_in_tx_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    lc: &LandingCustomization,
    now: chrono::DateTime<Utc>,
) -> Result<()> {
    let intro_locales_json =
        serde_json::to_string(&lc.intro_locales).context("serialize intro_locales")?;
    sqlx::query(
        "UPDATE landing_customization
            SET header_bg = $1, header_fg = $2, intro = $3,
                intro_locales_json = $4, seo_title = $5, seo_description = $6,
                og_image = $7, analytics_html = $8, analytics_origins = $9,
                updated_at = $10
          WHERE id = 1",
    )
    .bind(none_if_empty(&lc.header_bg))
    .bind(none_if_empty(&lc.header_fg))
    .bind(none_if_empty(&lc.intro))
    .bind(&intro_locales_json)
    .bind(none_if_empty(&lc.seo_title))
    .bind(none_if_empty(&lc.seo_description))
    .bind(none_if_empty(&lc.og_image))
    .bind(none_if_empty(&lc.analytics_html))
    .bind(none_if_empty(&lc.analytics_origins))
    .bind(now)
    .execute(&mut **tx)
    .await
    .context("update landing_customization")?;
    Ok(())
}

fn none_if_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().and_then(|x| {
        let t = x.trim();
        if t.is_empty() { None } else { Some(t) }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    #[tokio::test]
    async fn defaults_on_fresh_db() {
        let pool = open_memory().await.unwrap();
        let lc = fetch(&ConfigDb::Sqlite(pool.clone())).await.unwrap();
        assert!(lc.header_bg.is_none());
        assert!(lc.intro.is_none());
        assert!(lc.intro_locales.is_empty());
    }

    #[tokio::test]
    async fn update_then_fetch_roundtrip() {
        let pool = open_memory().await.unwrap();
        let mut intro_locales = std::collections::HashMap::new();
        intro_locales.insert("pt".into(), "Bem-vindo".into());
        intro_locales.insert("en".into(), "Welcome".into());
        let lc = LandingCustomization {
            header_bg: Some("#0f6e56".into()),
            header_fg: Some("#ffffff".into()),
            intro: Some("Welcome".into()),
            intro_locales,
            ..Default::default()
        };

        update(&ConfigDb::Sqlite(pool.clone()), &lc, Some("admin")).await.unwrap();
        let got = fetch(&ConfigDb::Sqlite(pool.clone())).await.unwrap();
        assert_eq!(got.header_bg.as_deref(), Some("#0f6e56"));
        assert_eq!(got.header_fg.as_deref(), Some("#ffffff"));
        assert_eq!(got.intro.as_deref(), Some("Welcome"));
        assert_eq!(got.intro_locales.get("pt").map(String::as_str), Some("Bem-vindo"));
        assert_eq!(got.intro_locales.get("en").map(String::as_str), Some("Welcome"));

        // audit captured the event
        let (n,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM audit_log WHERE action='landing.update'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(n, 1);
    }

    #[tokio::test]
    async fn seo_fields_roundtrip() {
        let pool = open_memory().await.unwrap();
        let lc = LandingCustomization {
            seo_title: Some("Demo Portal".into()),
            seo_description: Some("Demo portal description".into()),
            og_image: Some("/assets/img/og.png".into()),
            ..Default::default()
        };
        update(&ConfigDb::Sqlite(pool.clone()), &lc, Some("admin")).await.unwrap();
        let got = fetch(&ConfigDb::Sqlite(pool.clone())).await.unwrap();
        assert_eq!(got.seo_title.as_deref(), Some("Demo Portal"));
        assert_eq!(
            got.seo_description.as_deref(),
            Some("Demo portal description")
        );
        assert_eq!(got.og_image.as_deref(), Some("/assets/img/og.png"));
    }

    #[tokio::test]
    async fn analytics_fields_roundtrip() {
        let pool = open_memory().await.unwrap();
        let snippet = "<script src=\"https://plausible.io/js/script.js\"></script>";
        let origins = "https://plausible.io";
        let lc = LandingCustomization {
            analytics_html: Some(snippet.into()),
            analytics_origins: Some(origins.into()),
            ..Default::default()
        };
        update(&ConfigDb::Sqlite(pool.clone()), &lc, Some("admin")).await.unwrap();
        let got = fetch(&ConfigDb::Sqlite(pool.clone())).await.unwrap();
        assert_eq!(got.analytics_html.as_deref(), Some(snippet));
        assert_eq!(got.analytics_origins.as_deref(), Some(origins));
    }

    #[tokio::test]
    async fn empty_strings_collapse_to_none() {
        let pool = open_memory().await.unwrap();
        let lc = LandingCustomization {
            header_bg: Some("   ".into()), // whitespace only
            intro: Some(String::new()),
            ..Default::default()
        };
        update(&ConfigDb::Sqlite(pool.clone()), &lc, None).await.unwrap();
        let got = fetch(&ConfigDb::Sqlite(pool.clone())).await.unwrap();
        assert!(got.header_bg.is_none(), "whitespace-only becomes None");
        assert!(got.intro.is_none());
    }

    // Proves the ported `fetch` runs against a real Postgres through
    // the `ConfigDb::Postgres` arm. Gated:
    //   RUSCKER_TEST_PG_URL=postgres://… \
    //     cargo test -p ruscker-admin --features postgres-it -- --nocapture
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn fetch_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pool = crate::db::open_pg(&url).await.unwrap();
        // The 0001 migration seeds the singleton with empty fields.
        let lc = fetch(&ConfigDb::Postgres(pool)).await.unwrap();
        assert!(lc.header_bg.is_none());
        assert!(lc.intro.is_none());
        assert!(lc.intro_locales.is_empty());
    }

    // Writes the singleton through the Postgres arm, then reads it
    // back (and confirms empty-string collapse). Gated on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn update_then_fetch_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let db = ConfigDb::Postgres(crate::db::open_pg(&url).await.unwrap());

        let mut lc = LandingCustomization::default();
        lc.header_bg = Some("#0f6e56".into());
        lc.intro = Some("Bem-vindo".into());
        lc.seo_title = Some("Portal".into());
        lc.intro_locales.insert("pt".into(), "Olá".into());
        update(&db, &lc, Some("admin")).await.unwrap();

        let got = fetch(&db).await.unwrap();
        assert_eq!(got.header_bg.as_deref(), Some("#0f6e56"));
        assert_eq!(got.intro.as_deref(), Some("Bem-vindo"));
        assert_eq!(got.seo_title.as_deref(), Some("Portal"));
        assert_eq!(got.intro_locales.get("pt").map(String::as_str), Some("Olá"));

        // Whitespace-only collapses to NULL on write.
        let mut empty = LandingCustomization::default();
        empty.header_bg = Some("   ".into());
        update(&db, &empty, None).await.unwrap();
        assert!(fetch(&db).await.unwrap().header_bg.is_none());
    }
}
