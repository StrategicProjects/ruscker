//! Landing-page customization repository.
//!
//! Wraps the `landing_customization` singleton (id=1) — header
//! bg/fg, default intro paragraph, and per-locale intro overrides.
//! The same shape as [`ruscker_config::LandingCustomization`] so
//! import/export round-trip without translation gymnastics.

use anyhow::{Context, Result};
use chrono::Utc;
use ruscker_config::LandingCustomization;
use sqlx::SqlitePool;

/// Read the singleton row. Returns the default
/// [`LandingCustomization`] when the row hasn't been initialized
/// yet (which shouldn't normally happen — migration 0001 inserts
/// it — but defensive code keeps tests with a fresh DB happy).
pub async fn fetch(pool: &SqlitePool) -> Result<LandingCustomization> {
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
    let row: Option<Row> = sqlx::query_as(
        "SELECT header_bg, header_fg, intro, intro_locales_json,
                seo_title, seo_description, og_image,
                analytics_html, analytics_origins
           FROM landing_customization WHERE id = 1",
    )
    .fetch_optional(pool)
    .await
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
            })
        }
    }
}

/// Replace the singleton with `lc`. Empty strings on the form
/// arrive as `Some("")` from axum's Form extractor; this helper
/// collapses them to `None` so empty values disappear from the
/// exported YAML rather than serializing as `""`.
pub async fn update(
    pool: &SqlitePool,
    lc: &LandingCustomization,
    actor: Option<&str>,
) -> Result<()> {
    let now = Utc::now();
    let intro_locales_json =
        serde_json::to_string(&lc.intro_locales).context("serialize intro_locales")?;

    let mut tx = pool.begin().await.context("begin landing update tx")?;

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
    .execute(&mut *tx)
    .await
    .context("update landing_customization")?;

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
        let lc = fetch(&pool).await.unwrap();
        assert!(lc.header_bg.is_none());
        assert!(lc.intro.is_none());
        assert!(lc.intro_locales.is_empty());
    }

    #[tokio::test]
    async fn update_then_fetch_roundtrip() {
        let pool = open_memory().await.unwrap();
        let mut lc = LandingCustomization::default();
        lc.header_bg = Some("#0f6e56".into());
        lc.header_fg = Some("#ffffff".into());
        lc.intro = Some("Welcome".into());
        lc.intro_locales.insert("pt".into(), "Bem-vindo".into());
        lc.intro_locales.insert("en".into(), "Welcome".into());

        update(&pool, &lc, Some("admin")).await.unwrap();
        let got = fetch(&pool).await.unwrap();
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
        let mut lc = LandingCustomization::default();
        lc.seo_title = Some("Portal SEPE".into());
        lc.seo_description = Some("Monitoramento estratégico do estado".into());
        lc.og_image = Some("/assets/img/og.png".into());
        update(&pool, &lc, Some("admin")).await.unwrap();
        let got = fetch(&pool).await.unwrap();
        assert_eq!(got.seo_title.as_deref(), Some("Portal SEPE"));
        assert_eq!(
            got.seo_description.as_deref(),
            Some("Monitoramento estratégico do estado")
        );
        assert_eq!(got.og_image.as_deref(), Some("/assets/img/og.png"));
    }

    #[tokio::test]
    async fn analytics_fields_roundtrip() {
        let pool = open_memory().await.unwrap();
        let snippet = "<script src=\"https://plausible.io/js/script.js\"></script>";
        let origins = "https://plausible.io";
        let mut lc = LandingCustomization::default();
        lc.analytics_html = Some(snippet.into());
        lc.analytics_origins = Some(origins.into());
        update(&pool, &lc, Some("admin")).await.unwrap();
        let got = fetch(&pool).await.unwrap();
        assert_eq!(got.analytics_html.as_deref(), Some(snippet));
        assert_eq!(got.analytics_origins.as_deref(), Some(origins));
    }

    #[tokio::test]
    async fn empty_strings_collapse_to_none() {
        let pool = open_memory().await.unwrap();
        let mut lc = LandingCustomization::default();
        lc.header_bg = Some("   ".into()); // whitespace only
        lc.intro = Some("".into());
        update(&pool, &lc, None).await.unwrap();
        let got = fetch(&pool).await.unwrap();
        assert!(got.header_bg.is_none(), "whitespace-only becomes None");
        assert!(got.intro.is_none());
    }
}
