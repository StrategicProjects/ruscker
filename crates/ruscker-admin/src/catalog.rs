//! The effective runtime spec catalog (DB-first, YAML fallback).
//!
//! The landing and proxy resolve specs **DB-first** (#202 / #205), but
//! the lifecycle loops (auto-scaler, heartbeat sweeper) and the
//! dashboard restart historically read only the YAML `proxy.specs`.
//! That split meant specs created/edited in the admin — or seeded into
//! the showcase — ran on demand (the proxy spawns them) yet got no
//! min/max-replicas, no per-spec `heartbeat-timeout` override, and
//! couldn't be restarted from the dashboard (#257). This module is the
//! one DB-first resolver every lifecycle consumer shares, mirroring
//! `proxy::find_spec`'s per-id rule for the whole catalog.

use std::collections::HashSet;

use ruscker_config::{Config, Spec};

use crate::db::{self, ConfigDb};

/// Every spec the runtime should act on.
///
/// With a config DB attached, that's the DB catalog (admin edits +
/// showcase seed) **unioned** with the YAML `proxy.specs`, the DB
/// shadowing the YAML on an id collision (same rule as
/// `proxy::find_spec`). With no DB, it's just the YAML. On a DB error
/// it falls back to the YAML so a transient blip can't stall the
/// scaler.
///
/// Takes the pool + config directly (not `AppState`) so the heartbeat
/// sweeper — which only holds an `Arc<Config>` + an optional pool — can
/// share it.
pub(crate) async fn effective_specs(db: Option<&ConfigDb>, config: &Config) -> Vec<Spec> {
    let Some(db) = db else {
        return config.proxy.specs.clone();
    };
    match db::specs::list_all(db).await {
        Ok(db_specs) => {
            let db_ids: HashSet<String> = db_specs.iter().map(|s| s.id.clone()).collect();
            let mut out = db_specs;
            for s in &config.proxy.specs {
                if !db_ids.contains(&s.id) {
                    out.push(s.clone());
                }
            }
            out
        }
        Err(e) => {
            tracing::warn!(
                error = ?e,
                "spec catalog DB list failed; falling back to YAML proxy.specs"
            );
            config.proxy.specs.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    fn yaml_config(body: &str) -> Config {
        std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
        Config::from_yaml(body).expect("parse config")
    }

    #[tokio::test]
    async fn no_db_returns_yaml_specs() {
        let cfg = yaml_config("proxy:\n  specs:\n    - id: a\n      container-image: nginx\n");
        let specs = effective_specs(None, &cfg).await;
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].id, "a");
    }

    #[tokio::test]
    async fn unions_db_and_yaml_with_db_shadowing() {
        let cdb = ConfigDb::Sqlite(open_memory().await.unwrap());
        // DB carries a DB-only spec + one that collides with a YAML id.
        let db_only: Spec = serde_yaml_ng::from_str("id: db-only\ncontainer-image: nginx").unwrap();
        let shared_db: Spec =
            serde_yaml_ng::from_str("id: shared\ncontainer-image: nginx\ndisplay-name: from-db")
                .unwrap();
        db::specs::upsert_one(&cdb, &db_only, None).await.unwrap();
        db::specs::upsert_one(&cdb, &shared_db, None).await.unwrap();

        let cfg = yaml_config(
            "proxy:\n  specs:\n    - id: yaml-only\n      container-image: nginx\n    - id: shared\n      container-image: nginx\n      display-name: from-yaml\n",
        );

        let specs = effective_specs(Some(&cdb), &cfg).await;
        let ids: HashSet<&str> = specs.iter().map(|s| s.id.as_str()).collect();
        // DB-only + YAML-only both present, the shared id once.
        assert!(ids.contains("db-only"));
        assert!(ids.contains("yaml-only"));
        assert_eq!(specs.iter().filter(|s| s.id == "shared").count(), 1);
        // DB shadows YAML on the collision.
        let shared = specs.iter().find(|s| s.id == "shared").unwrap();
        assert_eq!(shared.display_name.as_deref(), Some("from-db"));
    }
}
