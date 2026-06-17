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
use std::sync::Arc;

use ruscker_config::{Config, Spec};

use crate::db::{self, ConfigDb};
use crate::AppState;

/// Shared, signature-validated cache of the effective spec catalog for the
/// admin pages (#902). Lives on [`AppState`]; see [`effective_specs_cached`].
pub type CatalogCache =
    Arc<tokio::sync::RwLock<Option<(db::specs::CatalogSignature, Arc<Vec<Spec>>)>>>;

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

/// [`effective_specs`] with a signature-validated cache for the admin
/// pages (#902). Reads the cheap [`db::specs::catalog_signature`]; on a
/// match against `state.catalog_cache` it returns the cached
/// `Arc<Vec<Spec>>` and skips the full `list_all` + per-spec
/// `config_json` deserialize. Any catalog write moves the signature, so
/// the cache is never stale (and HA-safe — the signature reflects writes
/// from any node). With no DB, or if the signature query fails, it falls
/// back to building uncached (never serving stale data).
pub(crate) async fn effective_specs_cached(state: &AppState) -> Arc<Vec<Spec>> {
    let Some(db) = state.db.as_ref() else {
        return Arc::new(effective_specs(None, &state.config).await);
    };
    let sig = match db::specs::catalog_signature(db).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(error = ?e, "catalog signature failed; bypassing cache");
            return Arc::new(effective_specs(Some(db), &state.config).await);
        }
    };
    if let Some((cached_sig, specs)) = state.catalog_cache.read().await.as_ref() {
        if *cached_sig == sig {
            return specs.clone();
        }
    }
    let specs = Arc::new(effective_specs(Some(db), &state.config).await);
    *state.catalog_cache.write().await = Some((sig, specs.clone()));
    specs
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

    // #902: the catalog signature must move on every write (insert /
    // update / delete) so the cache it guards is never stale. Also pins
    // that the dual-dialect aggregate query decodes (count + Σversion +
    // max updated_at).
    #[tokio::test]
    async fn catalog_signature_moves_on_every_write() {
        let cdb = ConfigDb::Sqlite(open_memory().await.unwrap());
        let empty = db::specs::catalog_signature(&cdb).await.unwrap();
        assert_eq!(empty.0, 0, "no specs yet");

        let a: Spec = serde_yaml_ng::from_str("id: a\ncontainer-image: nginx").unwrap();
        db::specs::upsert_one(&cdb, &a, None).await.unwrap();
        let after_insert = db::specs::catalog_signature(&cdb).await.unwrap();
        assert_ne!(after_insert, empty, "insert moved the signature");

        // Update (version bumps) → signature changes.
        let a2: Spec =
            serde_yaml_ng::from_str("id: a\ncontainer-image: nginx\ndisplay-name: edited").unwrap();
        db::specs::upsert_one(&cdb, &a2, None).await.unwrap();
        let after_update = db::specs::catalog_signature(&cdb).await.unwrap();
        assert_ne!(after_update, after_insert, "update moved the signature");

        // Delete → signature changes.
        db::specs::delete_one(&cdb, "a", None).await.unwrap();
        let after_delete = db::specs::catalog_signature(&cdb).await.unwrap();
        assert_ne!(after_delete, after_update, "delete moved the signature");
        assert_eq!(after_delete.0, 0, "back to empty");
    }
}
