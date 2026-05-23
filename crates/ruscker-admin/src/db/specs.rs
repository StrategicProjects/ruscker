//! Spec repository — bulk import, single-row CRUD, version history.
//!
//! Reads/writes happen through these functions rather than ad-hoc
//! SQL throughout the codebase. Keeps the schema's invariants
//! (audit log + version bump on update) in one place.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use ruscker_config::{Config, Spec};
use sqlx::SqlitePool;

/// What happened during [`import_all`].
#[derive(Debug, Default, Clone)]
pub struct ImportReport {
    /// Specs newly inserted on this run.
    pub created: usize,
    /// Specs that already existed and were updated to a new version.
    pub updated: usize,
    /// Specs that existed unchanged (config_json identical).
    pub unchanged: usize,
}

/// Import every spec from a parsed [`Config`] into the database.
///
/// Idempotent: re-running with the same YAML produces zero updates
/// (specs with identical `config_json` are left alone). Modified
/// specs bump their `version` counter and append a row to
/// `spec_versions`. Specs present in the DB but **not** in the
/// incoming config are NOT deleted — that's a separate operator
/// action (no surprise destruction).
///
/// The whole import runs in a single transaction.
pub async fn import_all(pool: &SqlitePool, config: &Config) -> Result<ImportReport> {
    let now = Utc::now();
    let mut tx = pool.begin().await.context("begin import tx")?;

    let mut report = ImportReport::default();

    for spec in &config.proxy.specs {
        let outcome = upsert_in_tx(&mut tx, spec, now).await?;
        match outcome {
            UpsertOutcome::Created => report.created += 1,
            UpsertOutcome::Updated => report.updated += 1,
            UpsertOutcome::Unchanged => report.unchanged += 1,
        }
    }

    // Landing customization is a singleton — replace it whole.
    let lc = &config.proxy.landing_customization;
    let intro_locales_json = serde_json::to_string(&lc.intro_locales)
        .context("serialize intro_locales")?;
    sqlx::query(
        "UPDATE landing_customization
            SET header_bg = ?, header_fg = ?, intro = ?,
                intro_locales_json = ?, updated_at = ?
          WHERE id = 1",
    )
    .bind(&lc.header_bg)
    .bind(&lc.header_fg)
    .bind(&lc.intro)
    .bind(&intro_locales_json)
    .bind(now)
    .execute(&mut *tx)
    .await
    .context("update landing_customization")?;

    // Persist the rest of `proxy` (everything except specs and
    // landing-customization) + the top-level server / logging
    // blocks as JSON blobs in config_meta. Lets export reconstruct
    // a byte-equivalent (mod whitespace) YAML without inventing
    // schema for every field the admin UI doesn't expose yet.
    let proxy_meta = proxy_meta_value(&config.proxy)?;
    upsert_meta(&mut tx, "proxy", &proxy_meta, now).await?;
    upsert_meta(&mut tx, "server", &serde_json::to_value(&config.server)?, now).await?;
    upsert_meta(&mut tx, "logging", &serde_json::to_value(&config.logging)?, now).await?;

    // System-level audit entry for the import.
    sqlx::query(
        "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
         VALUES (NULL, 'spec.import', NULL, ?, ?)",
    )
    .bind(serde_json::to_string(&serde_json::json!({
        "created": report.created,
        "updated": report.updated,
        "unchanged": report.unchanged,
    }))?)
    .bind(now)
    .execute(&mut *tx)
    .await
    .context("audit import")?;

    tx.commit().await.context("commit import tx")?;
    Ok(report)
}

#[derive(Debug, PartialEq, Eq)]
enum UpsertOutcome {
    Created,
    Updated,
    Unchanged,
}

async fn upsert_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    spec: &Spec,
    now: DateTime<Utc>,
) -> Result<UpsertOutcome> {
    let config_json = canonical_json(spec)
        .with_context(|| format!("serialize spec {}", spec.id))?;
    let kind = kind_str(spec);
    let state = if spec.template_properties.is_active() { "active" } else { "inactive" };

    // Check existing
    let existing: Option<(String, i64)> = sqlx::query_as(
        "SELECT config_json, version FROM specs WHERE id = ?",
    )
    .bind(&spec.id)
    .fetch_optional(&mut **tx)
    .await
    .with_context(|| format!("lookup spec {}", spec.id))?;

    match existing {
        None => {
            sqlx::query(
                "INSERT INTO specs (id, display_name, description, kind,
                                    container_image, config_json, state,
                                    created_at, updated_at, version)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1)",
            )
            .bind(&spec.id)
            .bind(spec.display_name.as_deref())
            .bind(spec.description.as_deref())
            .bind(kind)
            .bind(spec.container_image.as_deref())
            .bind(&config_json)
            .bind(state)
            .bind(now)
            .bind(now)
            .execute(&mut **tx)
            .await
            .with_context(|| format!("insert spec {}", spec.id))?;

            sqlx::query(
                "INSERT INTO spec_versions (spec_id, version, config_json, changed_at, changed_by)
                 VALUES (?, 1, ?, ?, NULL)",
            )
            .bind(&spec.id)
            .bind(&config_json)
            .bind(now)
            .execute(&mut **tx)
            .await
            .with_context(|| format!("insert v1 history for {}", spec.id))?;

            Ok(UpsertOutcome::Created)
        }
        Some((existing_json, version)) => {
            if existing_json == config_json {
                return Ok(UpsertOutcome::Unchanged);
            }
            let next_version = version + 1;
            sqlx::query(
                "UPDATE specs
                    SET display_name = ?, description = ?, kind = ?,
                        container_image = ?, config_json = ?, state = ?,
                        updated_at = ?, version = ?
                  WHERE id = ?",
            )
            .bind(spec.display_name.as_deref())
            .bind(spec.description.as_deref())
            .bind(kind)
            .bind(spec.container_image.as_deref())
            .bind(&config_json)
            .bind(state)
            .bind(now)
            .bind(next_version)
            .bind(&spec.id)
            .execute(&mut **tx)
            .await
            .with_context(|| format!("update spec {}", spec.id))?;

            sqlx::query(
                "INSERT INTO spec_versions (spec_id, version, config_json, changed_at, changed_by)
                 VALUES (?, ?, ?, ?, NULL)",
            )
            .bind(&spec.id)
            .bind(next_version)
            .bind(&config_json)
            .bind(now)
            .execute(&mut **tx)
            .await
            .with_context(|| format!("history v{next_version} for {}", spec.id))?;

            Ok(UpsertOutcome::Updated)
        }
    }
}

/// Serialize a `Spec` to canonical (alphabetically sorted) JSON.
///
/// Plain `serde_json::to_string(spec)` is NOT deterministic across
/// parses of the same YAML, because [`TemplateProperties`] wraps a
/// `HashMap` whose iteration order is randomized per-process. The
/// importer relies on byte-equal JSON to detect "unchanged" specs;
/// without canonicalization every re-import marks every spec as
/// updated and pollutes spec_versions with phantom history.
///
/// Round-tripping through `serde_json::Value` normalizes objects
/// to `serde_json::Map`, which is BTreeMap-backed and therefore
/// alphabetically sorted on serialization.
fn canonical_json(spec: &Spec) -> Result<String> {
    let v = serde_json::to_value(spec)?;
    Ok(serde_json::to_string(&v)?)
}

/// Strip specs + landing-customization from a serialized Proxy so
/// what lands in `config_meta` is just the "settings" parts. Those
/// two fields have their own tables and would otherwise be stored
/// twice with risk of drift.
fn proxy_meta_value(proxy: &ruscker_config::Proxy) -> Result<serde_json::Value> {
    let mut v = serde_json::to_value(proxy).context("serialize proxy")?;
    if let Some(obj) = v.as_object_mut() {
        obj.remove("specs");
        obj.remove("landing-customization");
    }
    Ok(v)
}

/// INSERT-or-REPLACE a row in `config_meta`. Helper so the import
/// flow reads cleanly.
async fn upsert_meta(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    key: &str,
    value: &serde_json::Value,
    now: DateTime<Utc>,
) -> Result<()> {
    let json = serde_json::to_string(value)?;
    sqlx::query(
        "INSERT OR REPLACE INTO config_meta (key, value_json, updated_at)
         VALUES (?, ?, ?)",
    )
    .bind(key)
    .bind(&json)
    .bind(now)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("upsert config_meta[{key}]"))?;
    Ok(())
}

fn kind_str(spec: &Spec) -> &'static str {
    use ruscker_config::SpecKind;
    match spec.kind() {
        SpecKind::Shiny => "shiny",
        SpecKind::InteractiveApp => "interactive",
        SpecKind::Api => "api",
        SpecKind::External => "external",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    fn fixture_yaml() -> String {
        std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
        std::fs::read_to_string("../../examples/application.yml").unwrap()
    }

    #[tokio::test]
    async fn imports_31_specs_then_roundtrip_is_unchanged() {
        let pool = open_memory().await.unwrap();
        let cfg = Config::from_yaml(&fixture_yaml()).unwrap();

        let r1 = import_all(&pool, &cfg).await.unwrap();
        assert_eq!(r1.created, 31, "first import inserts all");
        assert_eq!(r1.updated, 0);
        assert_eq!(r1.unchanged, 0);

        let r2 = import_all(&pool, &cfg).await.unwrap();
        assert_eq!(r2.created, 0, "second import sees them as existing");
        assert_eq!(r2.updated, 0, "no changes → no version bumps");
        assert_eq!(r2.unchanged, 31);

        // Audit log got both import events.
        let audit_count: (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM audit_log WHERE action = 'spec.import'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(audit_count.0, 2);
    }

    #[tokio::test]
    async fn modifying_a_spec_bumps_version() {
        let pool = open_memory().await.unwrap();
        let mut cfg = Config::from_yaml(&fixture_yaml()).unwrap();
        import_all(&pool, &cfg).await.unwrap();

        // Tweak one spec's description and re-import.
        cfg.proxy.specs[0].description = Some("Updated description".to_string());
        let r = import_all(&pool, &cfg).await.unwrap();
        assert_eq!(r.updated, 1);
        assert_eq!(r.unchanged, 30);

        let (version,): (i64,) = sqlx::query_as("SELECT version FROM specs WHERE id = ?")
            .bind(&cfg.proxy.specs[0].id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(version, 2);

        let (history_count,): (i64,) = sqlx::query_as(
            "SELECT COUNT(*) FROM spec_versions WHERE spec_id = ?",
        )
        .bind(&cfg.proxy.specs[0].id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(history_count, 2, "v1 + v2 in history");
    }

    #[tokio::test]
    async fn landing_customization_round_trips() {
        let pool = open_memory().await.unwrap();
        let cfg = Config::from_yaml(&fixture_yaml()).unwrap();
        import_all(&pool, &cfg).await.unwrap();

        let row: (Option<String>, Option<String>, Option<String>, String) = sqlx::query_as(
            "SELECT header_bg, header_fg, intro, intro_locales_json
               FROM landing_customization WHERE id = 1",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        // examples/application.yml has header-bg=#0f6e56 and intro-locales for all 4 langs
        assert_eq!(row.0.as_deref(), Some("#0f6e56"));
        let parsed: std::collections::HashMap<String, String> =
            serde_json::from_str(&row.3).unwrap();
        assert!(parsed.contains_key("pt"));
        assert!(parsed.contains_key("en"));
    }
}
