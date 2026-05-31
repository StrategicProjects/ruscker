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

/// The 13 cards seeded on first install. Order matters: Ruscker's
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
            "Reactive web apps in R.",
            "/assets/showcase/shiny.svg",
            "https://shiny.posit.co",
            Some("openanalytics/shinyproxy-demo:latest"),
            Some(3838),
            Some("linux/amd64"),
        )?,
        {
            // Shiny for Python — same reactive model, Python ecosystem.
            // `shinyproxy-shiny-for-python-demo` listens on :8080; routes
            // like the R Shiny card (kind Shiny, sticky + WebSocket).
            // The bundled logo is a white wordmark, so give the card a
            // deep Shiny-blue → cyan gradient cover that makes the white
            // artwork pop (the SVG's only ink is `#FFFFFF`).
            let mut s = card(
                "shiny-for-python",
                "Shiny for Python",
                "Reactive web apps in Python — Shiny's model, Python's ecosystem.",
                "/assets/showcase/shiny-for-python.svg",
                "https://shiny.posit.co/py/",
                Some("openanalytics/shinyproxy-shiny-for-python-demo:latest"),
                Some(8080),
                Some("linux/amd64"),
            )?;
            s.template_properties.0.insert(
                "cover".into(),
                serde_yaml_ng::Value::String(
                    "linear-gradient(135deg, #16335b 0%, #2aa9c9 100%)".into(),
                ),
            );
            s
        },
        {
            // Jupyter needs runtime config to work behind the proxy: a
            // notebook server otherwise demands a token nobody has and
            // rejects the kernel WebSocket on an origin mismatch. The
            // `container-cmd` below runs it token-less with a permissive
            // origin so the showcase card opens out of the box. (Demo
            // posture — the card is open; tighten with a token/auth for
            // anything real.)
            //
            // `--ServerApp.base_url=/` (revert of #371): a live probe on
            // the cast `/box` deploy showed the proxy STRIPS the mount
            // prefix before forwarding (the container receives `/lab/...`,
            // not `/box/app/jupyter/lab/...`) — so `base_url=#{publicPath}`
            // made the container 404 *every* path. With `base_url=/` the
            // container serves the stripped paths, and the #348
            // jupyter-config rewrite (which prefixes the page-config
            // `baseUrl`/`fullStaticUrl` to the mount) makes the browser's
            // static-chunk + REST requests route back correctly. (The
            // public-path injection mechanism from #377 stays — it's
            // useful for env-based apps — Jupyter just doesn't use it.)
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
        // ── Containerized framework demos (OpenAnalytics images) ────
        // These run the same demo images ShinyProxy uses, behind the
        // `/app/{spec}/` proxy. Ruscker has no `SHINYPROXY_PUBLIC_PATH`
        // cmd-templating, so we DON'T pass a base-url path; the apps run
        // at `base_url=/` and rely on the `/app` HTML rewriter + runtime
        // shim (+ the #348 jupyter-config rewrite for Voilà), the same
        // mechanism the Shiny/Jupyter cards use. amd64-only images, so
        // the platform is pinned. Ports come from each demo's
        // `application.yml` (#354/#365).
        {
            let mut s = card(
                "streamlit",
                "Streamlit",
                "Data and machine-learning apps in Python.",
                "/assets/showcase/streamlit.svg",
                "https://streamlit.io",
                Some("openanalytics/shinyproxy-streamlit-demo:latest"),
                Some(8501),
                Some("linux/amd64"),
            )?;
            s.kind_override = Some(ruscker_config::SpecKindOverride::Streamlit);
            s
        },
        {
            let mut s = card(
                "dash",
                "Dash",
                "Analytical web apps built on Plotly.",
                "/assets/showcase/dash.svg",
                "https://dash.plotly.com",
                Some("openanalytics/shinyproxy-dash-demo:latest"),
                Some(8050),
                Some("linux/amd64"),
            )?;
            s.kind_override = Some(ruscker_config::SpecKindOverride::Dash);
            s
        },
        {
            // Quarto demo on :8080. Use the `:prerendered` tag (#372):
            // the `:latest` image renders the docs on startup, which took
            // >60s — the container accepted TCP but never answered HTTP
            // inside the readiness window, so it never went Ready and the
            // splash hung. The prerendered variant serves a pre-built site
            // immediately. No dedicated kind — generic interactive app.
            let mut s = card(
                "quarto",
                "Quarto",
                "Open-source publishing system for technical documents.",
                "/assets/showcase/quarto.svg",
                "https://quarto.org",
                Some("openanalytics/shinyproxy-quarto-demo:prerendered"),
                Some(8080),
                Some("linux/amd64"),
            )?;
            s.kind_override = Some(ruscker_config::SpecKindOverride::App);
            s
        },
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
        {
            // FastAPI demo — a stateless API (kind Api: no sticky/WS,
            // round-robin). ShinyProxy passes a dynamic `SCRIPT_NAME`
            // (the public path) for the OpenAPI/docs prefix; Ruscker
            // can't template that, so `/docs` assets may need the
            // forwarded-prefix headers / a follow-up — the raw API
            // endpoints work regardless.
            let mut s = card(
                "fastapi",
                "FastAPI",
                "Modern Python framework for building APIs.",
                "/assets/showcase/fastapi.svg",
                "https://fastapi.tiangolo.com",
                Some("openanalytics/shinyproxy-fastapi-demo:latest"),
                Some(8000),
                Some("linux/amd64"),
            )?;
            s.kind_override = Some(ruscker_config::SpecKindOverride::Api);
            s
        },
        {
            // Voilà demo — Jupyter-server based (kind Voila). Like
            // Jupyter, it builds its webpack public path from the page
            // config; we run it at `base_url=/` (dropping ShinyProxy's
            // dynamic `--base_url`) and lean on the #348 jupyter-config
            // rewrite + the runtime shim. `basics.ipynb` ships in the
            // image; bind to all interfaces on :8080.
            let mut s = card(
                "voila",
                "Voilà",
                "Standalone web apps from Jupyter notebooks.",
                "/assets/showcase/voila.svg",
                "https://voila.readthedocs.io",
                Some("openanalytics/shinyproxy-voila-demo:latest"),
                Some(8080),
                Some("linux/amd64"),
            )?;
            s.kind_override = Some(ruscker_config::SpecKindOverride::Voila);
            s.container_cmd = Some(vec![
                "voila".into(),
                "basics.ipynb".into(),
                "--no-browser".into(),
                "--port=8080".into(),
                "--Voila.ip=0.0.0.0".into(),
            ]);
            s
        },
    ])
}

// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_memory, ConfigDb};

    #[tokio::test]
    async fn seed_inserts_all_showcase_cards_on_empty_db() {
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
        assert_eq!(n, 13, "13 showcase specs seeded");
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
        assert_eq!(kind("shiny-for-python"), ruscker_config::SpecKind::Shiny);
        // The Shiny-for-Python card carries a gradient cover (white logo
        // needs a colored background to pop).
        let sfp = specs.iter().find(|s| s.id == "shiny-for-python").unwrap();
        assert!(
            sfp.template_properties
                .get_str("cover")
                .is_some_and(|c| c.contains("linear-gradient")),
            "shiny-for-python has a gradient cover"
        );
        // #371 revert: Jupyter runs at base_url=/ (the proxy strips the
        // mount prefix), NOT the #{publicPath} token which 404'd every
        // path. Lock it so the wrong direction can't sneak back.
        let jup_cmd = specs.iter().find(|s| s.id == "jupyter").unwrap().container_cmd.clone();
        let jup_cmd = jup_cmd.unwrap_or_default();
        assert!(
            jup_cmd.iter().any(|a| a == "--ServerApp.base_url=/"),
            "jupyter must run at base_url=/"
        );
        assert!(
            !jup_cmd.iter().any(|a| a.contains("#{publicPath}")),
            "jupyter must NOT use the public-path token for base_url (#371)"
        );
        // Framework demos converted from external links (#354/#365):
        // Streamlit/Dash/Voilà are interactive (sticky + WS); FastAPI is
        // a stateless API; rmarkdown is Shiny-backed; bokeh/plumber have
        // no demo image so they stay external links.
        assert_eq!(kind("rmarkdown"), ruscker_config::SpecKind::Shiny);
        assert_eq!(kind("streamlit"), ruscker_config::SpecKind::InteractiveApp);
        assert_eq!(kind("dash"), ruscker_config::SpecKind::InteractiveApp);
        assert_eq!(kind("voila"), ruscker_config::SpecKind::InteractiveApp);
        assert_eq!(kind("quarto"), ruscker_config::SpecKind::InteractiveApp);
        assert_eq!(kind("fastapi"), ruscker_config::SpecKind::Api);
        assert_eq!(kind("bokeh"), ruscker_config::SpecKind::External, "no demo image → link");
        assert_eq!(kind("plumber"), ruscker_config::SpecKind::External, "no demo image → link");
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
