//! Export the DB back to a `ruscker_config::Config` ready for YAML
//! serialization. Mirror of `import_all`: every byte the importer
//! persisted comes back, modulo whitespace/comments which YAML
//! can't preserve.
//!
//! Round-trip invariant (covered by tests): for any `Config c`,
//! `import_all(pool, c)` followed by `reconstruct_config(pool)`
//! returns a value that, when re-imported, reports
//! `unchanged = total_specs` and `updated = 0`.

use anyhow::{Context, Result};
use ruscker_config::{Config, Logging, Proxy, Server, Spec};
use sqlx::SqlitePool;

/// Rebuild a full [`Config`] from everything currently in the
/// database: spec rows, the landing-customization singleton, and
/// the `config_meta` key/value entries for the rest of
/// proxy/server/logging.
pub async fn reconstruct_config(pool: &SqlitePool) -> Result<Config> {
    // 1. Specs — read in stable order (id ASC) so YAML output is
    //    deterministic across runs.
    let spec_rows: Vec<(String,)> =
        sqlx::query_as("SELECT config_json FROM specs ORDER BY id ASC")
            .fetch_all(pool)
            .await
            .context("load specs")?;
    let mut specs: Vec<Spec> = Vec::with_capacity(spec_rows.len());
    for (json,) in spec_rows {
        let s: Spec = serde_json::from_str(&json).context("deserialize spec row")?;
        specs.push(s);
    }

    // 2. Landing customization singleton — reuse the repository
    //    loader so every field (incl. SEO) round-trips without
    //    duplicating the column list here. Then attach the custom
    //    HTML blocks (ordered by slot, position) so they round-trip too.
    let mut landing_customization = super::landing::fetch(pool).await?;
    landing_customization.blocks = super::landing_blocks::list_all(pool)
        .await?
        .iter()
        .map(|b| b.to_config())
        .collect();

    // 3. config_meta sections — start with default structs and let
    //    serde_json overlay whatever the import persisted. Falling
    //    back to default-when-missing means a DB with no
    //    config_meta row still gives a sensible Config.
    let proxy_meta = load_meta(pool, "proxy").await?;
    let server_value = load_meta(pool, "server").await?;
    let logging_value = load_meta(pool, "logging").await?;

    let mut proxy: Proxy = match proxy_meta {
        Some(v) => serde_json::from_value(v).context("deserialize proxy meta")?,
        None => Proxy::default(),
    };
    let server: Server = match server_value {
        Some(v) => serde_json::from_value(v).context("deserialize server meta")?,
        None => Server::default(),
    };
    let logging: Logging = match logging_value {
        Some(v) => serde_json::from_value(v).context("deserialize logging meta")?,
        None => Logging::default(),
    };

    // 4. Splice specs + landing-customization back onto proxy.
    proxy.specs = specs;
    proxy.landing_customization = landing_customization;

    Ok(Config {
        server,
        proxy,
        logging,
        raw_warnings: Vec::new(),
    })
}

async fn load_meta(pool: &SqlitePool, key: &str) -> Result<Option<serde_json::Value>> {
    let row: Option<(String,)> =
        sqlx::query_as("SELECT value_json FROM config_meta WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await
            .with_context(|| format!("load config_meta[{key}]"))?;
    match row {
        None => Ok(None),
        Some((json,)) => Ok(Some(serde_json::from_str(&json)?)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::{open_memory, specs::import_all};

    fn fixture_yaml() -> String {
        std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
        std::fs::read_to_string("../../examples/application.yml").unwrap()
    }

    #[tokio::test]
    async fn round_trip_preserves_specs_count() {
        let pool = open_memory().await.unwrap();
        let cfg_in = Config::from_yaml(&fixture_yaml()).unwrap();
        import_all(&pool, &cfg_in).await.unwrap();

        let cfg_out = reconstruct_config(&pool).await.unwrap();
        assert_eq!(cfg_out.proxy.specs.len(), cfg_in.proxy.specs.len());
    }

    #[tokio::test]
    async fn round_trip_then_reimport_is_unchanged() {
        let pool = open_memory().await.unwrap();
        let cfg_in = Config::from_yaml(&fixture_yaml()).unwrap();
        import_all(&pool, &cfg_in).await.unwrap();

        let cfg_out = reconstruct_config(&pool).await.unwrap();

        // Re-import the exported Config into a fresh DB — every
        // spec should land as Created.
        let pool2 = open_memory().await.unwrap();
        let r = import_all(&pool2, &cfg_out).await.unwrap();
        assert_eq!(r.created, cfg_in.proxy.specs.len());

        // And re-import the same Config into the SAME DB — every
        // spec unchanged.
        let r2 = import_all(&pool2, &cfg_out).await.unwrap();
        assert_eq!(r2.unchanged, cfg_in.proxy.specs.len());
        assert_eq!(r2.updated, 0);
    }

    #[tokio::test]
    async fn round_trip_preserves_landing_customization() {
        let pool = open_memory().await.unwrap();
        let cfg_in = Config::from_yaml(&fixture_yaml()).unwrap();
        import_all(&pool, &cfg_in).await.unwrap();
        let cfg_out = reconstruct_config(&pool).await.unwrap();

        assert_eq!(
            cfg_out.proxy.landing_customization.header_bg,
            cfg_in.proxy.landing_customization.header_bg
        );
        assert_eq!(
            cfg_out.proxy.landing_customization.intro_locales,
            cfg_in.proxy.landing_customization.intro_locales
        );
    }

    #[tokio::test]
    async fn round_trip_preserves_landing_blocks() {
        let pool = open_memory().await.unwrap();
        let yaml = r#"
proxy:
  landing-customization:
    blocks:
      - { slot: top, title: Banner, html: "<div>hi</div>", csp-origins: "https://cdn.x", enabled: true }
      - { slot: top, title: Second, html: "<p>2</p>" }
      - { slot: bottom, title: Foot, html: "<footer>f</footer>", enabled: false }
  specs: []
"#;
        let cfg_in = Config::from_yaml(yaml).unwrap();
        assert_eq!(cfg_in.proxy.landing_customization.blocks.len(), 3);
        // `enabled` defaults to true when the key is omitted.
        assert!(cfg_in.proxy.landing_customization.blocks[1].enabled);

        import_all(&pool, &cfg_in).await.unwrap();
        let cfg_out = reconstruct_config(&pool).await.unwrap();
        let out = &cfg_out.proxy.landing_customization.blocks;

        assert_eq!(out.len(), 3);
        // Per-slot order + content preserved (export groups by slot:
        // bottom sorts before top).
        let top: Vec<_> = out.iter().filter(|b| b.slot == "top").collect();
        assert_eq!(
            top.iter().map(|b| b.title.as_str()).collect::<Vec<_>>(),
            ["Banner", "Second"]
        );
        assert_eq!(top[0].html, "<div>hi</div>");
        assert_eq!(top[0].csp_origins, "https://cdn.x");
        let bottom: Vec<_> = out.iter().filter(|b| b.slot == "bottom").collect();
        assert_eq!(bottom.len(), 1);
        assert!(!bottom[0].enabled);
    }

    #[tokio::test]
    async fn round_trip_preserves_proxy_settings() {
        let pool = open_memory().await.unwrap();
        let cfg_in = Config::from_yaml(&fixture_yaml()).unwrap();
        import_all(&pool, &cfg_in).await.unwrap();
        let cfg_out = reconstruct_config(&pool).await.unwrap();

        assert_eq!(cfg_out.proxy.title, cfg_in.proxy.title);
        assert_eq!(cfg_out.proxy.port, cfg_in.proxy.port);
        assert_eq!(cfg_out.proxy.bind_address, cfg_in.proxy.bind_address);
        assert_eq!(cfg_out.proxy.heartbeat_rate, cfg_in.proxy.heartbeat_rate);
    }
}
