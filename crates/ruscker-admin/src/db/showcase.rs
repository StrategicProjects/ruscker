//! First-install showcase seed.
//!
//! On a brand-new DB we want operators to land on a portal that
//! actually shows what Ruscker is for — not an empty filter grid.
//! This module inserts a small set of "showcase" specs the first
//! time the DB opens: one card for the Ruscker docs and one per
//! supported framework with the logo bundled in the binary
//! (`/assets/showcase/*.svg`). Three of them are containerized
//! against well-known public hello-world images; the rest are
//! `external`-kind links to the respective docs pages.
//!
//! Idempotency: a row in `config_meta` keyed `showcase.seeded`
//! records the seed timestamp. Subsequent startups see the marker
//! and skip — even if the operator deleted some or all of the
//! seeded cards. That's intentional: re-adding cards an operator
//! explicitly removed would be obnoxious. To force a re-seed,
//! delete the marker row by hand.

use anyhow::{Context, Result};
use chrono::Utc;
use ruscker_config::Spec;
use serde_json::json;

use crate::db::ConfigDb;

const SEEDED_KEY: &str = "showcase.seeded";

/// Seed the showcase cards if (and only if) they've never been seeded
/// on this database. Called from [`crate::db::open`] /
/// [`crate::db::open_pg`] right after migrations.
pub async fn seed_if_unseeded(db: &ConfigDb) -> Result<()> {
    if already_seeded(db).await? {
        return Ok(());
    }
    // The previous welcome LandingBlock (migration 0008) is superseded
    // by the showcase cards. Clean it up so the new install isn't a
    // mix of the two shapes. No-op on a DB that never had it.
    drop_old_welcome_block(db).await?;

    for spec in showcase_specs()? {
        // If the operator somehow already has a spec with the same id
        // (e.g. they imported a YAML carrying one), don't overwrite.
        if super::specs::fetch_one(db, &spec.id).await?.is_some() {
            continue;
        }
        super::specs::upsert_one(db, &spec, None).await?;
    }

    mark_seeded(db).await?;
    Ok(())
}

async fn already_seeded(db: &ConfigDb) -> Result<bool> {
    let row: Option<(String,)> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as("SELECT value_json FROM config_meta WHERE key = ?")
                .bind(SEEDED_KEY)
                .fetch_optional(pool)
                .await
                .context("check showcase seed marker (sqlite)")?
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as("SELECT value_json FROM config_meta WHERE key = $1")
                .bind(SEEDED_KEY)
                .fetch_optional(pool)
                .await
                .context("check showcase seed marker (postgres)")?
        }
    };
    Ok(row.is_some())
}

async fn mark_seeded(db: &ConfigDb) -> Result<()> {
    let now = Utc::now();
    let value_json = serde_json::json!({ "seeded_at": now }).to_string();
    match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query(
                "INSERT OR REPLACE INTO config_meta (key, value_json, updated_at)
                 VALUES (?, ?, ?)",
            )
            .bind(SEEDED_KEY)
            .bind(&value_json)
            .bind(now)
            .execute(pool)
            .await
            .context("write showcase seed marker (sqlite)")?;
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query(
                "INSERT INTO config_meta (key, value_json, updated_at)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (key) DO UPDATE SET value_json = $2, updated_at = $3",
            )
            .bind(SEEDED_KEY)
            .bind(&value_json)
            .bind(now)
            .execute(pool)
            .await
            .context("write showcase seed marker (postgres)")?;
        }
    }
    Ok(())
}

/// Drop the welcome LandingBlock from migration 0008. The
/// showcase cards replace it.
async fn drop_old_welcome_block(db: &ConfigDb) -> Result<()> {
    match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query("DELETE FROM landing_blocks WHERE id = ?")
                .bind("welcome-seed")
                .execute(pool)
                .await
                .context("drop old welcome-seed block (sqlite)")?;
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query("DELETE FROM landing_blocks WHERE id = $1")
                .bind("welcome-seed")
                .execute(pool)
                .await
                .context("drop old welcome-seed block (postgres)")?;
        }
    }
    Ok(())
}

// ── Spec catalog ──────────────────────────────────────────────────

/// Build a [`Spec`] by deserializing a JSON literal. `Spec` carries
/// ~40 optional fields and no `Default` impl; building one by hand
/// would be all noise. Serde reads exactly what we set and fills the
/// rest with `None` / defaults via the `#[serde(default)]` on each
/// field.
// Internal seed-only helper with self-documenting positional args
// (id, name, description, logo, link, image, port, platform). Bundling
// them into a struct buys nothing for 12 fixed call sites in one file.
#[allow(clippy::too_many_arguments)]
fn card(
    id: &str,
    display_name: &str,
    description: &str,
    logo_path: &str,
    link_url: &str,
    container_image: Option<&str>,
    container_port: Option<u16>,
    platform: Option<&str>,
) -> Result<Spec> {
    let mut tp = serde_json::Map::new();
    tp.insert("logo".into(), json!(logo_path));
    tp.insert("subject".into(), json!("Documentação"));
    tp.insert("link".into(), json!(link_url));
    tp.insert("state".into(), json!("active"));
    // `type` drives the colored badge: "app" for containerized
    // cards, "package" for docs/reference links.
    tp.insert(
        "type".into(),
        json!(if container_image.is_some() { "app" } else { "package" }),
    );

    let mut spec_json = serde_json::Map::new();
    spec_json.insert("id".into(), json!(id));
    spec_json.insert("display-name".into(), json!(display_name));
    spec_json.insert("description".into(), json!(description));
    spec_json.insert("template-properties".into(), json!(tp));
    if let Some(image) = container_image {
        spec_json.insert("container-image".into(), json!(image));
        if let Some(p) = platform {
            spec_json.insert("platform".into(), json!(p));
        }
    } else {
        // External link card — explicit kind so Spec::kind() doesn't
        // need to infer.
        spec_json.insert("type".into(), json!("external"));
    }
    if let Some(port) = container_port {
        spec_json.insert("container-port".into(), json!(port));
    }

    serde_json::from_value(json!(spec_json))
        .with_context(|| format!("build showcase spec {id}"))
}

/// The 12 cards seeded on first install. Order matters: Ruscker's
/// own docs card is first; the rest follow the order the maintainer
/// curated for the showcase.
fn showcase_specs() -> Result<Vec<Spec>> {
    Ok(vec![
        // ── Ruscker's own docs card ─────────────────────────────────
        card(
            "ruscker-docs",
            "Ruscker",
            "Container portal and load balancer for interactive web apps and APIs.",
            "/assets/brand/mark.svg",
            "https://ruscker.com",
            None,
            None,
            None,
        )?,
        // ── Containerized hello-world demos ─────────────────────────
        // The Shiny card runs `openanalytics/shinyproxy-demo` — a real,
        // single-page Shiny demo that opens straight into a working app
        // (vs `rocker/shiny`, which lands on a sample-app index). This
        // is the image the cast lab used for its hand-written
        // `shiny-demo` card; folding it into the showcase seed retires
        // that duplicate CONFIG card (#354). amd64-only manifest, so pin
        // the platform to run it via emulation on arm64 hosts (Apple
        // Silicon / Graviton); the other two demo images ship multi-arch
        // manifests so they don't need the override.
        card(
            "shiny",
            "Shiny",
            "Reactive web apps in R and Python.",
            "/assets/showcase/shiny.svg",
            "https://shiny.posit.co",
            Some("openanalytics/shinyproxy-demo:latest"),
            Some(3838),
            Some("linux/amd64"),
        )?,
        {
            // Jupyter needs runtime config to work behind the proxy: a
            // notebook server otherwise demands a token nobody has and
            // rejects the kernel WebSocket on an origin mismatch. The
            // `container-cmd` below runs it token-less with a permissive
            // origin so the showcase card opens out of the box. (Demo
            // posture — the card is open; tighten with a token/auth for
            // anything real.)
            let mut jup = card(
                "jupyter",
                "Jupyter",
                "Interactive notebooks served as a web app.",
                "/assets/showcase/jupyter.svg",
                "https://jupyter.org",
                Some("quay.io/jupyter/minimal-notebook:latest"),
                Some(8888),
                None,
            )?;
            jup.container_cmd = Some(vec![
                "start-notebook.py".into(),
                "--IdentityProvider.token=".into(),
                "--ServerApp.allow_origin=*".into(),
                "--ServerApp.allow_remote_access=True".into(),
                "--ServerApp.disable_check_xsrf=True".into(),
                "--ServerApp.base_url=/".into(),
            ]);
            // Generic interactive app, not Shiny (#231).
            jup.kind_override = Some(ruscker_config::SpecKindOverride::App);
            jup
        },
        {
            // RStudio Server works behind the proxy via the
            // `X-RStudio-Root-Path` header (set in
            // `apply_smart_routing_headers`) — RStudio's official
            // "behind a path-rewriting proxy" mechanism, the same one
            // ShinyProxy uses (#230). The demo ships a fixed login so it
            // actually shows RStudio's sign-in screen (#347) instead of
            // dropping straight into the IDE: `rocker/rstudio` reads
            // `PASSWORD` and uses the `rstudio` user. The credentials are
            // surfaced in the card description so the demo is usable. For
            // anything real, replace the seeded `PASSWORD` (write-only in
            // the spec form) with a `${VAR}` and a real secret, or wire
            // SSO in front. (Previously this ran `DISABLE_AUTH=true`,
            // which skipped the login entirely.)
            let mut rst = card(
                "rstudio",
                "RStudio Server",
                "The RStudio IDE in your browser, served per session. Demo login — user: rstudio, password: ruscker.",
                "/assets/showcase/rstudio.svg",
                "https://posit.co/products/open-source/rstudio-server/",
                Some("rocker/rstudio:latest"),
                Some(8787),
                None,
            )?;
            rst.container_env = Some(std::collections::BTreeMap::from([(
                "PASSWORD".to_string(),
                "ruscker".to_string(),
            )]));
            rst.kind_override = Some(ruscker_config::SpecKindOverride::App);
            rst
        },
        // R Markdown demo — OpenAnalytics' `shinyproxy-rmarkdown-demo`
        // renders an `.Rmd` document with a Shiny backend, so it routes
        // exactly like the Shiny card (sticky + WebSocket, port 3838) and
        // the existing `/app` rewriter handles its URLs without extra
        // base-url flags. amd64-only image → pin the platform (#354).
        card(
            "rmarkdown",
            "R Markdown",
            "Reproducible R documents rendered live, with a Shiny backend.",
            "/assets/showcase/rmarkdown.svg",
            "https://rmarkdown.rstudio.com",
            Some("openanalytics/shinyproxy-rmarkdown-demo:latest"),
            Some(3838),
            Some("linux/amd64"),
        )?,
        // ── External-link cards (no public hello image) ─────────────
        card(
            "streamlit",
            "Streamlit",
            "Data and machine-learning apps in Python.",
            "/assets/showcase/streamlit.svg",
            "https://streamlit.io",
            None,
            None,
            None,
        )?,
        card(
            "dash",
            "Dash",
            "Analytical web apps built on Plotly.",
            "/assets/showcase/dash.svg",
            "https://dash.plotly.com",
            None,
            None,
            None,
        )?,
        card(
            "quarto",
            "Quarto",
            "Open-source publishing system for technical documents.",
            "/assets/showcase/quarto.svg",
            "https://quarto.org",
            None,
            None,
            None,
        )?,
        card(
            "bokeh",
            "Bokeh",
            "Interactive visualization library for Python.",
            "/assets/showcase/bokeh.svg",
            "https://bokeh.org",
            None,
            None,
            None,
        )?,
        card(
            "plumber",
            "Plumber",
            "Turn R functions into HTTP APIs.",
            "/assets/showcase/plumber.svg",
            "https://plumber2.posit.co",
            None,
            None,
            None,
        )?,
        card(
            "fastapi",
            "FastAPI",
            "Modern Python framework for building APIs.",
            "/assets/showcase/fastapi.svg",
            "https://fastapi.tiangolo.com",
            None,
            None,
            None,
        )?,
        card(
            "voila",
            "Voilà",
            "Standalone web apps from Jupyter notebooks.",
            "/assets/showcase/voila.svg",
            "https://voila.readthedocs.io",
            None,
            None,
            None,
        )?,
    ])
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_memory, ConfigDb};

    #[tokio::test]
    async fn seed_inserts_all_twelve_cards_on_empty_db() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        seed_if_unseeded(&db).await.unwrap();
        let pool = match &db {
            ConfigDb::Sqlite(p) => p,
            _ => unreachable!(),
        };
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM specs")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(n, 12, "12 showcase specs seeded");
        // The Ruscker docs card is the canonical first entry.
        let row: (String,) =
            sqlx::query_as("SELECT display_name FROM specs WHERE id = 'ruscker-docs'")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(row.0, "Ruscker");
    }

    #[test]
    fn showcase_card_kinds() {
        // #231: containerized non-Shiny cards (Jupyter, RStudio) are
        // tagged `App` → InteractiveApp, not the Shiny default. Shiny
        // stays Shiny. (#230: RStudio works behind the proxy via the
        // X-RStudio-Root-Path header, so it's a real container again.)
        let specs = showcase_specs().unwrap();
        let kind = |id: &str| specs.iter().find(|s| s.id == id).unwrap().kind();
        assert_eq!(kind("jupyter"), ruscker_config::SpecKind::InteractiveApp);
        assert_eq!(kind("rstudio"), ruscker_config::SpecKind::InteractiveApp);
        assert_eq!(kind("shiny"), ruscker_config::SpecKind::Shiny, "shiny stays Shiny");
    }

    #[tokio::test]
    async fn seed_is_idempotent_via_config_meta() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        seed_if_unseeded(&db).await.unwrap();
        let pool = match &db {
            ConfigDb::Sqlite(p) => p,
            _ => unreachable!(),
        };
        // Operator deletes a showcase card.
        sqlx::query("DELETE FROM specs WHERE id = 'shiny'")
            .execute(pool)
            .await
            .unwrap();
        // Restart re-runs the seed; the deletion must stick.
        seed_if_unseeded(&db).await.unwrap();
        let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM specs WHERE id = 'shiny'")
            .fetch_one(pool)
            .await
            .unwrap();
        assert_eq!(n, 0, "deletion sticks on subsequent boots");
    }

    #[tokio::test]
    async fn seed_drops_old_welcome_block() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        // Migration 0008 already seeded `welcome-seed` on the fresh
        // memory DB; confirm it's there before our seed runs.
        let pool = match &db {
            ConfigDb::Sqlite(p) => p,
            _ => unreachable!(),
        };
        let pre: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM landing_blocks WHERE id = 'welcome-seed'")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(pre.0, 1);

        seed_if_unseeded(&db).await.unwrap();

        let post: (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM landing_blocks WHERE id = 'welcome-seed'")
                .fetch_one(pool)
                .await
                .unwrap();
        assert_eq!(post.0, 0, "showcase seed dropped the old welcome block");
    }
}
