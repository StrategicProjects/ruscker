//! Spec repository — bulk import, single-row CRUD, version history.
//!
//! Reads/writes happen through these functions rather than ad-hoc
//! SQL throughout the codebase. Keeps the schema's invariants
//! (audit log + version bump on update) in one place.

use crate::db::ConfigDb;
use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use ruscker_config::{Config, Spec};

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
/// The whole import runs in a single transaction. Dual-dialect: each
/// arm uses its backend's `*_in_tx` writers (SQLite vs Postgres twins)
/// and placeholders, but the spec-iteration + report bookkeeping is the
/// same shape.
pub async fn import_all(db: &ConfigDb, config: &Config) -> Result<ImportReport> {
    let now = Utc::now();
    let lc = &config.proxy.landing_customization;
    // The rest of `proxy` (minus specs + landing-customization) and the
    // top-level server / logging blocks round-trip as JSON in
    // config_meta — computed once, written inside the chosen arm.
    let proxy_meta = proxy_meta_value(&config.proxy)?;
    let server_meta = serde_json::to_value(&config.server)?;
    let logging_meta = serde_json::to_value(&config.logging)?;

    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin import tx")?;
            let mut report = ImportReport::default();
            for spec in &config.proxy.specs {
                tally(&mut report, upsert_in_tx(&mut tx, spec, now).await?);
            }
            super::landing::update_in_tx(&mut tx, lc, now).await?;
            // #199: only replace landing_blocks when the imported YAML
            // actually carries a `blocks:` list. The parser ignores
            // `blocks` today (DB-only — see YAML_SCHEMA.md), so this is
            // always skipped on import and the welcome seed / operator-
            // authored blocks survive. An unconditional replace here
            // wiped the table with an empty list.
            if !lc.blocks.is_empty() {
                super::landing_blocks::replace_all_in_tx(&mut tx, &lc.blocks, now).await?;
            }
            upsert_meta(&mut tx, "proxy", &proxy_meta, now).await?;
            upsert_meta(&mut tx, "server", &server_meta, now).await?;
            upsert_meta(&mut tx, "logging", &logging_meta, now).await?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (NULL, 'spec.import', NULL, ?, ?)",
            )
            .bind(import_diff(&report)?)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit import")?;
            tx.commit().await.context("commit import tx")?;
            Ok(report)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin import tx")?;
            let mut report = ImportReport::default();
            for spec in &config.proxy.specs {
                tally(&mut report, upsert_in_tx_pg(&mut tx, spec, now).await?);
            }
            super::landing::update_in_tx_pg(&mut tx, lc, now).await?;
            // #199: see the SQLite arm — only replace when the import
            // carries a non-empty `blocks:` list (never, today).
            if !lc.blocks.is_empty() {
                super::landing_blocks::replace_all_in_tx_pg(&mut tx, &lc.blocks, now).await?;
            }
            upsert_meta_pg(&mut tx, "proxy", &proxy_meta, now).await?;
            upsert_meta_pg(&mut tx, "server", &server_meta, now).await?;
            upsert_meta_pg(&mut tx, "logging", &logging_meta, now).await?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (NULL, 'spec.import', NULL, $1, $2)",
            )
            .bind(import_diff(&report)?)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit import")?;
            tx.commit().await.context("commit import tx")?;
            Ok(report)
        }
    }
}

/// Import only the specs whose id is in `ids` (selective import, #557).
/// Unlike [`import_all`], it touches **only the chosen specs** (plus an
/// audit row) — never the landing customization or the proxy/server
/// settings — so importing a subset of apps can't clobber a portal the
/// operator configured in the panel. Mirrors `import_all`'s
/// per-spec upsert + transaction.
pub async fn import_selected(
    db: &ConfigDb,
    config: &Config,
    ids: &[String],
) -> Result<ImportReport> {
    use std::collections::HashSet;
    let want: HashSet<&str> = ids.iter().map(String::as_str).collect();
    let now = Utc::now();
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin import tx")?;
            let mut report = ImportReport::default();
            for spec in config
                .proxy
                .specs
                .iter()
                .filter(|s| want.contains(s.id.as_str()))
            {
                tally(&mut report, upsert_in_tx(&mut tx, spec, now).await?);
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (NULL, 'spec.import', NULL, ?, ?)",
            )
            .bind(import_diff(&report)?)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit import")?;
            tx.commit().await.context("commit import tx")?;
            Ok(report)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin import tx")?;
            let mut report = ImportReport::default();
            for spec in config
                .proxy
                .specs
                .iter()
                .filter(|s| want.contains(s.id.as_str()))
            {
                tally(&mut report, upsert_in_tx_pg(&mut tx, spec, now).await?);
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (NULL, 'spec.import', NULL, $1, $2)",
            )
            .bind(import_diff(&report)?)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit import")?;
            tx.commit().await.context("commit import tx")?;
            Ok(report)
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum UpsertOutcome {
    Created,
    Updated,
    Unchanged,
}

/// Fold one per-spec [`UpsertOutcome`] into the running [`ImportReport`].
fn tally(report: &mut ImportReport, outcome: UpsertOutcome) {
    match outcome {
        UpsertOutcome::Created => report.created += 1,
        UpsertOutcome::Updated => report.updated += 1,
        UpsertOutcome::Unchanged => report.unchanged += 1,
    }
}

/// The `diff_json` for the system-level import audit row.
fn import_diff(report: &ImportReport) -> Result<String> {
    Ok(serde_json::to_string(&serde_json::json!({
        "created": report.created,
        "updated": report.updated,
        "unchanged": report.unchanged,
    }))?)
}

/// Every spec in the catalog, deserialized back to [`Spec`]. Used by
/// the landing handler when a DB is attached so the public portal
/// reflects DB edits + the showcase seed (rather than only the
/// startup YAML's `proxy.specs`).
///
/// Insertion order is stable via `created_at` so the showcase cards
/// keep the maintainer-curated order (Ruscker first, then the rest).
pub async fn list_all(db: &ConfigDb) -> Result<Vec<Spec>> {
    let rows: Vec<(String,)> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(
            "SELECT config_json FROM specs ORDER BY created_at, id",
        )
        .fetch_all(pool)
        .await,
        ConfigDb::Postgres(pool) => sqlx::query_as(
            "SELECT config_json FROM specs ORDER BY created_at, id",
        )
        .fetch_all(pool)
        .await,
    }
    .context("list all specs")?;
    let mut out = Vec::with_capacity(rows.len());
    for (json,) in rows {
        let s: Spec =
            serde_json::from_str(&json).context("deserialize spec row")?;
        out.push(s);
    }
    Ok(out)
}

/// Fetch a single spec by id, deserializing `config_json` back to
/// a [`Spec`]. Returns `None` if no row matches.
///
/// Dual-dialect (Phase 7c-4): one bound parameter, so the only
/// difference is the placeholder (`?` vs `$1`).
pub async fn fetch_one(db: &ConfigDb, id: &str) -> Result<Option<Spec>> {
    let row: Option<(String,)> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as("SELECT config_json FROM specs WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await,
        ConfigDb::Postgres(pool) => sqlx::query_as("SELECT config_json FROM specs WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await,
    }
    .with_context(|| format!("fetch spec {id}"))?;
    match row {
        None => Ok(None),
        Some((json,)) => {
            let s: Spec = serde_json::from_str(&json)
                .with_context(|| format!("deserialize spec {id}"))?;
            Ok(Some(s))
        }
    }
}

/// The audit action for an upsert outcome, or `None` when nothing
/// changed (no audit row written).
fn upsert_audit_action(outcome: &UpsertOutcome) -> Option<&'static str> {
    match outcome {
        UpsertOutcome::Created => Some("spec.create"),
        UpsertOutcome::Updated => Some("spec.update"),
        UpsertOutcome::Unchanged => None,
    }
}

/// Upsert a single spec — used by the admin form. Runs the per-row
/// upsert in its own transaction with an audit-log entry tagged with
/// `actor` (unless the spec was unchanged).
///
/// Dual-dialect (Phase 7c-4): the transaction type and placeholders
/// differ per backend, so the body forks per arm — but each arm reuses
/// the dialect's `upsert_in_tx*` helper and the shared
/// [`upsert_audit_action`].
/// The spec's current `version` counter, or `None` when absent. Powers
/// the edit form's optimistic-concurrency check (#745).
pub async fn fetch_version(db: &ConfigDb, id: &str) -> Result<Option<i64>> {
    let row: Option<(i64,)> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as("SELECT version FROM specs WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await,
        ConfigDb::Postgres(pool) => sqlx::query_as("SELECT version FROM specs WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await,
    }
    .with_context(|| format!("fetch version of {id}"))?;
    Ok(row.map(|(v,)| v))
}

/// Insert a NEW spec, failing closed when the id already exists (#745).
/// The create form's pre-check + `upsert_one` was a TOCTOU: two
/// concurrent creates of the same id both passed the check and the
/// second silently overwrote the first. The existence check here runs
/// inside the write transaction, and a racing insert that still slips
/// past it dies on the primary-key constraint instead of clobbering.
/// Returns `Ok(false)` when the id is already taken.
pub async fn insert_new(db: &ConfigDb, spec: &Spec, actor: Option<&str>) -> Result<bool> {
    let now = Utc::now();
    let target = format!("spec:{}", spec.id);
    let is_unique_violation = |e: &anyhow::Error| {
        e.downcast_ref::<sqlx::Error>()
            .and_then(|se| se.as_database_error())
            .is_some_and(|de| de.is_unique_violation())
    };
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin insert_new tx")?;
            let exists: Option<(i64,)> =
                sqlx::query_as("SELECT 1 FROM specs WHERE id = ?")
                    .bind(&spec.id)
                    .fetch_optional(&mut *tx)
                    .await
                    .context("insert_new existence check")?;
            if exists.is_some() {
                return Ok(false);
            }
            match upsert_in_tx(&mut tx, spec, now).await {
                Ok(_) => {}
                Err(e) if is_unique_violation(&e) => return Ok(false),
                Err(e) => return Err(e),
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'spec.create', ?, NULL, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit insert_new")?;
            tx.commit().await.context("commit insert_new tx")?;
            Ok(true)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin insert_new tx")?;
            let exists: Option<(i64,)> =
                sqlx::query_as("SELECT 1 FROM specs WHERE id = $1")
                    .bind(&spec.id)
                    .fetch_optional(&mut *tx)
                    .await
                    .context("insert_new existence check")?;
            if exists.is_some() {
                return Ok(false);
            }
            match upsert_in_tx_pg(&mut tx, spec, now).await {
                Ok(_) => {}
                Err(e) if is_unique_violation(&e) => return Ok(false),
                Err(e) => return Err(e),
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'spec.create', $2, NULL, $3)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit insert_new")?;
            tx.commit().await.context("commit insert_new tx")?;
            Ok(true)
        }
    }
}

pub async fn upsert_one(
    db: &ConfigDb,
    spec: &Spec,
    actor: Option<&str>,
) -> Result<UpsertOutcome> {
    let now = Utc::now();
    let target = format!("spec:{}", spec.id);
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin upsert tx")?;
            let outcome = upsert_in_tx(&mut tx, spec, now).await?;
            if let Some(action) = upsert_audit_action(&outcome) {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES (?, ?, ?, NULL, ?)",
                )
                .bind(actor)
                .bind(action)
                .bind(&target)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit upsert_one")?;
            }
            tx.commit().await.context("commit upsert_one tx")?;
            Ok(outcome)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin upsert tx")?;
            let outcome = upsert_in_tx_pg(&mut tx, spec, now).await?;
            if let Some(action) = upsert_audit_action(&outcome) {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES ($1, $2, $3, NULL, $4)",
                )
                .bind(actor)
                .bind(action)
                .bind(&target)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit upsert_one")?;
            }
            tx.commit().await.context("commit upsert_one tx")?;
            Ok(outcome)
        }
    }
}

/// Stamp a spec's `template-properties.updated` date **only if it has
/// none yet**, returning whether it wrote. Used to fill a card's
/// recency date from the publish date of the image it runs (#375) — a
/// one-time enrichment on first spawn. No-op (returns `false`) when the
/// id isn't a DB spec (YAML-only specs aren't created here) or already
/// carries a date, so it never clobbers an operator-set value.
pub async fn set_updated_if_absent(
    db: &ConfigDb,
    id: &str,
    date: &str,
    actor: Option<&str>,
) -> Result<bool> {
    let Some(mut spec) = fetch_one(db, id).await? else {
        return Ok(false); // not a DB spec — nothing to enrich
    };
    let has_date = spec
        .template_properties
        .get_str("updated")
        .is_some_and(|s| !s.trim().is_empty());
    if has_date {
        return Ok(false);
    }
    spec.template_properties
        .0
        .insert("updated".into(), serde_yaml_ng::Value::String(date.to_string()));
    upsert_one(db, &spec, actor).await?;
    Ok(true)
}

/// Delete a spec and all its history. Returns `true` if a row was
/// actually removed (false if the id didn't exist). Audit log records
/// the action when something was deleted. `ON DELETE CASCADE` clears
/// `spec_versions` on both backends.
pub async fn delete_one(db: &ConfigDb, id: &str, actor: Option<&str>) -> Result<bool> {
    let now = Utc::now();
    let target = format!("spec:{id}");
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin delete tx")?;
            let rows = sqlx::query("DELETE FROM specs WHERE id = ?")
                .bind(id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("delete spec {id}"))?;
            let removed = rows.rows_affected() > 0;
            if removed {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES (?, 'spec.delete', ?, NULL, ?)",
                )
                .bind(actor)
                .bind(&target)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit delete")?;
            }
            tx.commit().await.context("commit delete tx")?;
            Ok(removed)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin delete tx")?;
            let rows = sqlx::query("DELETE FROM specs WHERE id = $1")
                .bind(id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("delete spec {id}"))?;
            let removed = rows.rows_affected() > 0;
            if removed {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES ($1, 'spec.delete', $2, NULL, $3)",
                )
                .bind(actor)
                .bind(&target)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit delete")?;
            }
            tx.commit().await.context("commit delete tx")?;
            Ok(removed)
        }
    }
}

pub(crate) async fn upsert_in_tx(
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
                                    created_at, updated_at, version, featured)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, 1, ?)",
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
            .bind(spec.is_featured())
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
                        updated_at = ?, version = ?, featured = ?
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
            .bind(spec.is_featured())
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

/// Postgres twin of [`upsert_in_tx`] — identical logic, `$n`
/// placeholders. Used by the Postgres arms of [`upsert_one`] and
/// [`import_all`] (each dispatches on the `ConfigDb` dialect).
pub(crate) async fn upsert_in_tx_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    spec: &Spec,
    now: DateTime<Utc>,
) -> Result<UpsertOutcome> {
    let config_json =
        canonical_json(spec).with_context(|| format!("serialize spec {}", spec.id))?;
    let kind = kind_str(spec);
    let state = if spec.template_properties.is_active() {
        "active"
    } else {
        "inactive"
    };

    let existing: Option<(String, i64)> =
        sqlx::query_as("SELECT config_json, version FROM specs WHERE id = $1")
            .bind(&spec.id)
            .fetch_optional(&mut **tx)
            .await
            .with_context(|| format!("lookup spec {}", spec.id))?;

    match existing {
        None => {
            sqlx::query(
                "INSERT INTO specs (id, display_name, description, kind,
                                    container_image, config_json, state,
                                    created_at, updated_at, version, featured)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 1, $10)",
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
            .bind(spec.is_featured())
            .execute(&mut **tx)
            .await
            .with_context(|| format!("insert spec {}", spec.id))?;

            sqlx::query(
                "INSERT INTO spec_versions (spec_id, version, config_json, changed_at, changed_by)
                 VALUES ($1, 1, $2, $3, NULL)",
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
                    SET display_name = $1, description = $2, kind = $3,
                        container_image = $4, config_json = $5, state = $6,
                        updated_at = $7, version = $8, featured = $9
                  WHERE id = $10",
            )
            .bind(spec.display_name.as_deref())
            .bind(spec.description.as_deref())
            .bind(kind)
            .bind(spec.container_image.as_deref())
            .bind(&config_json)
            .bind(state)
            .bind(now)
            .bind(next_version)
            .bind(spec.is_featured())
            .bind(&spec.id)
            .execute(&mut **tx)
            .await
            .with_context(|| format!("update spec {}", spec.id))?;

            sqlx::query(
                "INSERT INTO spec_versions (spec_id, version, config_json, changed_at, changed_by)
                 VALUES ($1, $2, $3, $4, NULL)",
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

/// Postgres twin of [`upsert_meta`]. SQLite's `INSERT OR REPLACE`
/// becomes the standard `ON CONFLICT (key) DO UPDATE`.
async fn upsert_meta_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    key: &str,
    value: &serde_json::Value,
    now: DateTime<Utc>,
) -> Result<()> {
    let json = serde_json::to_string(value)?;
    sqlx::query(
        "INSERT INTO config_meta (key, value_json, updated_at)
         VALUES ($1, $2, $3)
         ON CONFLICT (key) DO UPDATE SET
           value_json = excluded.value_json,
           updated_at = excluded.updated_at",
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

    // #745: the create path must fail CLOSED on an existing id — the old
    // pre-check + upsert let a concurrent create silently overwrite.
    #[tokio::test]
    async fn insert_new_refuses_an_existing_id_without_clobbering() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        let first: Spec =
            serde_yaml_ng::from_str("id: app
display-name: Original
container-image: a:1")
                .unwrap();
        assert!(insert_new(&db, &first, Some("alice")).await.unwrap());

        let second: Spec =
            serde_yaml_ng::from_str("id: app
display-name: Usurper
container-image: b:2")
                .unwrap();
        assert!(
            !insert_new(&db, &second, Some("bob")).await.unwrap(),
            "existing id must be refused"
        );
        let kept = fetch_one(&db, "app").await.unwrap().expect("still there");
        assert_eq!(
            kept.display_name.as_deref(),
            Some("Original"),
            "the loser must not overwrite the original"
        );
    }

    // #745: `fetch_version` tracks the per-save counter the edit form's
    // optimistic-concurrency check compares against.
    #[tokio::test]
    async fn fetch_version_follows_saves() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        assert_eq!(fetch_version(&db, "ghost").await.unwrap(), None);
        let mut spec: Spec =
            serde_yaml_ng::from_str("id: app
display-name: V1
container-image: a:1").unwrap();
        insert_new(&db, &spec, None).await.unwrap();
        let v1 = fetch_version(&db, "app").await.unwrap().expect("versioned");
        spec.display_name = Some("V2".into());
        upsert_one(&db, &spec, None).await.unwrap();
        let v2 = fetch_version(&db, "app").await.unwrap().expect("versioned");
        assert!(v2 > v1, "a save must bump the version ({v1} → {v2})");
    }

    // #775: the Apps-list archive toggle flips `template-properties.state`
    // and saves through `upsert_one` — both readers must agree afterwards:
    // the list view reads the `state` COLUMN, the landing reads
    // `is_active()` off the deserialized spec.
    #[tokio::test]
    async fn state_flip_updates_column_and_roundtrips() {
        let pool = open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool.clone());
        let mut spec: Spec =
            serde_yaml_ng::from_str("id: app
container-image: a:1").unwrap();
        insert_new(&db, &spec, None).await.unwrap();

        spec.template_properties.set_str("state", "inactive");
        upsert_one(&db, &spec, Some("admin")).await.unwrap();

        let (col,): (String,) = sqlx::query_as("SELECT state FROM specs WHERE id = 'app'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(col, "inactive", "list view reads the column");
        let back = fetch_one(&db, "app").await.unwrap().expect("still there");
        assert!(!back.template_properties.is_active(), "landing reads the spec");

        // …and back to active.
        spec.template_properties.set_str("state", "active");
        upsert_one(&db, &spec, Some("admin")).await.unwrap();
        let (col,): (String,) = sqlx::query_as("SELECT state FROM specs WHERE id = 'app'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(col, "active");
    }

    fn fixture_yaml() -> String {
        std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
        std::fs::read_to_string("../../examples/application.yml").unwrap()
    }

    #[tokio::test]
    async fn imports_all_specs_then_roundtrip_is_unchanged() {
        let pool = open_memory().await.unwrap();
        let cfg = Config::from_yaml(&fixture_yaml()).unwrap();
        let n = cfg.proxy.specs.len();

        let r1 = import_all(&ConfigDb::Sqlite(pool.clone()), &cfg).await.unwrap();
        assert_eq!(r1.created, n, "first import inserts all");
        assert_eq!(r1.updated, 0);
        assert_eq!(r1.unchanged, 0);

        let r2 = import_all(&ConfigDb::Sqlite(pool.clone()), &cfg).await.unwrap();
        assert_eq!(r2.created, 0, "second import sees them as existing");
        assert_eq!(r2.updated, 0, "no changes → no version bumps");
        assert_eq!(r2.unchanged, n);

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
    async fn import_selected_imports_only_the_chosen_subset() {
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        let cfg = Config::from_yaml(&fixture_yaml()).unwrap();
        assert!(cfg.proxy.specs.len() >= 2, "fixture needs ≥2 specs");
        let chosen = cfg.proxy.specs[0].id.clone();
        let other = cfg.proxy.specs[1].id.clone();

        let r = import_selected(&db, &cfg, std::slice::from_ref(&chosen))
            .await
            .unwrap();
        assert_eq!(r.created, 1, "only the one selected spec is imported");
        assert_eq!(r.updated + r.unchanged, 0);

        assert!(
            fetch_one(&db, &chosen).await.unwrap().is_some(),
            "chosen spec landed in the DB"
        );
        assert!(
            fetch_one(&db, &other).await.unwrap().is_none(),
            "unselected spec was NOT imported"
        );
    }

    #[tokio::test]
    async fn set_updated_if_absent_fills_once_then_no_clobber() {
        std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
        let db = ConfigDb::Sqlite(open_memory().await.unwrap());
        let cfg =
            Config::from_yaml("proxy:\n  specs:\n    - id: appx\n      container-image: img:1\n")
                .unwrap();
        upsert_one(&db, &cfg.proxy.specs[0], None).await.unwrap();

        // Fills when the date is absent (#375).
        assert!(set_updated_if_absent(&db, "appx", "22/08/2025", Some("system")).await.unwrap());
        let s = fetch_one(&db, "appx").await.unwrap().unwrap();
        assert_eq!(s.template_properties.get_str("updated"), Some("22/08/2025"));

        // No-op when a date already exists — never clobbers it.
        assert!(!set_updated_if_absent(&db, "appx", "01/01/2099", Some("system")).await.unwrap());
        let s2 = fetch_one(&db, "appx").await.unwrap().unwrap();
        assert_eq!(s2.template_properties.get_str("updated"), Some("22/08/2025"));

        // No-op for an id that isn't a DB spec.
        assert!(!set_updated_if_absent(&db, "nope", "01/01/2025", None).await.unwrap());
    }

    // #199: import must not wipe landing_blocks. Migration 0008 seeds a
    // `welcome-seed` block on a fresh DB; importing a YAML that carries
    // no `blocks:` (the parser ignores it — DB-only) must leave it be.
    #[tokio::test]
    async fn import_preserves_existing_landing_blocks() {
        let pool = open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool.clone());
        let count = |p: sqlx::SqlitePool| async move {
            let n: (i64,) =
                sqlx::query_as("SELECT COUNT(*) FROM landing_blocks WHERE id = 'welcome-seed'")
                    .fetch_one(&p)
                    .await
                    .unwrap();
            n.0
        };
        assert_eq!(count(pool.clone()).await, 1, "welcome-seed present before import");

        let cfg = Config::from_yaml(&fixture_yaml()).unwrap();
        import_all(&db, &cfg).await.unwrap();

        assert_eq!(
            count(pool.clone()).await,
            1,
            "import must not delete pre-existing landing_blocks"
        );
    }

    #[tokio::test]
    async fn modifying_a_spec_bumps_version() {
        let pool = open_memory().await.unwrap();
        let mut cfg = Config::from_yaml(&fixture_yaml()).unwrap();
        import_all(&ConfigDb::Sqlite(pool.clone()), &cfg).await.unwrap();

        // Tweak one spec's description and re-import.
        cfg.proxy.specs[0].description = Some("Updated description".to_string());
        let r = import_all(&ConfigDb::Sqlite(pool.clone()), &cfg).await.unwrap();
        assert_eq!(r.updated, 1);
        assert_eq!(r.unchanged, cfg.proxy.specs.len() - 1);

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
        import_all(&ConfigDb::Sqlite(pool.clone()), &cfg).await.unwrap();

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

    // Exercises the ported CRUD (create / unchanged / fetch / update +
    // version bump / delete + cascade) through the `ConfigDb::Postgres`
    // arm against a real daemon. Gated on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn spec_crud_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pool = crate::db::open_pg(&url).await.unwrap();
        // Isolate: clear specs (ON DELETE CASCADE clears spec_versions).
        sqlx::query("DELETE FROM specs")
            .execute(&pool)
            .await
            .unwrap();
        let db = ConfigDb::Postgres(pool.clone());

        let cfg = Config::from_yaml(&fixture_yaml()).unwrap();
        let mut spec = cfg.proxy.specs[0].clone();

        assert_eq!(
            upsert_one(&db, &spec, Some("admin")).await.unwrap(),
            UpsertOutcome::Created
        );
        assert_eq!(
            upsert_one(&db, &spec, Some("admin")).await.unwrap(),
            UpsertOutcome::Unchanged
        );
        assert_eq!(fetch_one(&db, &spec.id).await.unwrap().unwrap().id, spec.id);

        spec.description = Some("changed in pg".into());
        assert_eq!(
            upsert_one(&db, &spec, Some("admin")).await.unwrap(),
            UpsertOutcome::Updated
        );
        let (version,): (i64,) = sqlx::query_as("SELECT version FROM specs WHERE id = $1")
            .bind(&spec.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(version, 2);
        let (history,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM spec_versions WHERE spec_id = $1")
                .bind(&spec.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(history, 2, "v1 + v2 in history");

        assert!(delete_one(&db, &spec.id, Some("admin")).await.unwrap());
        assert!(fetch_one(&db, &spec.id).await.unwrap().is_none());
        let (history,): (i64,) =
            sqlx::query_as("SELECT COUNT(*) FROM spec_versions WHERE spec_id = $1")
                .bind(&spec.id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(history, 0, "cascade cleared history");
    }

    // The full `import_all` transaction (specs + landing + blocks +
    // config_meta + audit) through the Postgres arm, plus idempotency.
    // Gated on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn import_all_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pool = crate::db::open_pg(&url).await.unwrap();
        // Clear what the import writes (DELETE specs cascades spec_versions).
        for stmt in [
            "DELETE FROM specs",
            "DELETE FROM landing_blocks",
            "DELETE FROM config_meta",
        ] {
            sqlx::query(stmt).execute(&pool).await.unwrap();
        }
        let db = ConfigDb::Postgres(pool.clone());

        let cfg = Config::from_yaml(&fixture_yaml()).unwrap();
        let n = cfg.proxy.specs.len();

        let r1 = import_all(&db, &cfg).await.unwrap();
        assert_eq!(r1.created, n, "first import inserts all");
        assert_eq!(r1.updated, 0);

        let r2 = import_all(&db, &cfg).await.unwrap();
        assert_eq!(r2.unchanged, n, "re-import sees them unchanged");
        assert_eq!(r2.created, 0);

        // proxy / server / logging round-tripped into config_meta.
        let (meta,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM config_meta")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(meta, 3);
    }
}
