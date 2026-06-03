//! Admin > Apps list page.
//!
//! Read-only for now: lists every spec in the DB with kind, state,
//! version, updated_at. Edit/delete buttons are placeholders until
//! the spec form lands in the next slice.

use askama::Template;
use axum::{
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use sqlx::FromRow;

use crate::auth::{RequireEditor, Role};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

/// Body cap for the YAML import upload. ShinyProxy configs are
/// tiny (<100 KB typically); 2 MB is generous and stops a huge
/// paste from buffering unbounded.
const IMPORT_BODY_LIMIT: usize = 2 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/specs", get(index))
        .route("/admin/specs/import", post(import))
        .route("/admin/specs/import/confirm", post(import_confirm))
        .route(
            "/admin/specs/{id}/featured/toggle",
            post(toggle_featured),
        )
        .layer(DefaultBodyLimit::max(IMPORT_BODY_LIMIT))
}

#[derive(serde::Serialize)]
struct ToggleResult {
    featured: bool,
}

/// `POST /admin/specs/{id}/featured/toggle` — flip a spec's `featured`
/// flag from the Apps table's star (#521), without opening the editor.
/// Editor-gated; returns the new state as JSON for the optimistic UI.
async fn toggle_featured(
    editor: RequireEditor,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(db) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let mut spec = match crate::db::specs::fetch_one(db, &id).await {
        Ok(Some(s)) => s,
        Ok(None) => return (StatusCode::NOT_FOUND, "spec not found").into_response(),
        Err(e) => {
            tracing::error!(id, error = ?e, "fetch spec for toggle failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let now_featured = !spec.is_featured();
    // None when off so a normal spec carries no `featured` noise in JSON.
    spec.featured = now_featured.then_some(true);
    if let Err(e) = crate::db::specs::upsert_one(db, &spec, Some(editor.actor())).await {
        tracing::error!(id, error = ?e, "save featured toggle failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "save failed").into_response();
    }
    Json(ToggleResult {
        featured: now_featured,
    })
    .into_response()
}

/// One row out of the `specs` table, picked for the list view.
/// Distinct from `Spec` (the YAML model) so we don't need to
/// deserialize `config_json` just to render a table.
#[derive(Debug, FromRow)]
pub struct SpecRow {
    pub id: String,
    pub display_name: Option<String>,
    pub kind: String,
    pub state: String,
    pub updated_at: DateTime<Utc>,
    pub version: i64,
    /// `true` for a spec that exists only in the YAML `--config`, not the
    /// DB (#303) — it runs + shows on the landing but is **read-only**
    /// here (edit the file to change it). `#[sqlx(default)]` so the DB
    /// `SELECT` (which doesn't have the column) still maps.
    #[sqlx(default)]
    pub config_only: bool,
    /// Highlighted in the landing's Featured carousel (#506). Not a column
    /// — `featured` lives in `config_json`; the index fills this in from a
    /// `list_all` pass so the table's star toggle (#521) reflects state.
    #[sqlx(default)]
    pub featured: bool,
    /// Total accesses (#549) — filled by the index from a `spec_access`
    /// SUM, not a column on `specs`. `0` when never accessed.
    #[sqlx(default)]
    pub access_count: i64,
    /// Inline-SVG daily-usage sparkline (#549 follow-up), rendered from the
    /// last days' buckets. Empty when there's no usage to draw.
    #[sqlx(default)]
    pub trend_svg: String,
}

/// Post-import flash, carried back via query params on the
/// redirect (stateless — no session flash store needed).
#[derive(Debug, Deserialize, Default)]
pub struct SpecsQuery {
    /// "ok" or "err" — present only right after an import.
    #[serde(default)]
    pub import: Option<String>,
    #[serde(default)]
    pub created: Option<usize>,
    #[serde(default)]
    pub updated: Option<usize>,
    #[serde(default)]
    pub unchanged: Option<usize>,
    #[serde(default)]
    pub warnings: Option<usize>,
    /// Error message (URL-encoded) when `import=err`.
    #[serde(default)]
    pub msg: Option<String>,
}

/// A pre-rendered flash banner: tone (`ok`/`warn`/`err`) drives
/// the CSS class, `text` is the already-localized message. Built
/// in `index` from the import query params so the template stays
/// arg-free.
struct Flash {
    tone: &'static str,
    text: String,
}

#[derive(Template)]
#[template(path = "admin/specs.html")]
struct SpecsPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// Mount prefix for base-path-correct URLs (#294).
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    /// Current session role (Editor or Admin) — drives nav gating.
    role: Role,
    specs: Vec<SpecRow>,
    flash: Option<Flash>,
}

impl<'a> SpecsPage<'a> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

/// One row in the import preview (#557).
struct ImportRow {
    id: String,
    display_name: String,
    kind: &'static str,
    /// `true` ⇒ the import would **update** a spec already in the DB;
    /// `false` ⇒ it's **new**.
    exists: bool,
}

#[derive(Template)]
#[template(path = "admin/import_preview.html")]
struct ImportPreviewPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    /// The apps the uploaded YAML carries, in file order.
    rows: Vec<ImportRow>,
    /// The original YAML, round-tripped through a hidden field so the
    /// confirm step is stateless (`${VAR}` stays literal — #260).
    raw_yaml: String,
    /// Validation warnings across the whole file (shown as a count).
    warning_count: usize,
}

impl<'a> ImportPreviewPage<'a> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

/// Build the localized import flash from the redirect query
/// params. `None` when there was no import this request.
fn build_flash(locales: &Locales, loc: Locale, q: &SpecsQuery) -> Option<Flash> {
    use fluent_bundle::FluentArgs;
    match q.import.as_deref() {
        Some("ok") => {
            let mut args = FluentArgs::new();
            args.set("created", q.created.unwrap_or(0) as i64);
            args.set("updated", q.updated.unwrap_or(0) as i64);
            args.set("unchanged", q.unchanged.unwrap_or(0) as i64);
            let mut text = locales.t(loc, "admin-import-ok", Some(&args));
            let warnings = q.warnings.unwrap_or(0);
            if warnings > 0 {
                let mut wargs = FluentArgs::new();
                wargs.set("warnings", warnings as i64);
                text.push(' ');
                text.push_str(&locales.t(loc, "admin-import-ok-warnings", Some(&wargs)));
                return Some(Flash { tone: "warn", text });
            }
            Some(Flash { tone: "ok", text })
        }
        Some("err") => {
            let mut args = FluentArgs::new();
            let msg = q.msg.clone().unwrap_or_default().replace('+', " ");
            args.set("msg", msg);
            Some(Flash {
                tone: "err",
                text: locales.t(loc, "admin-import-err", Some(&args)),
            })
        }
        _ => None,
    }
}

async fn index(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(flash): Query<SpecsQuery>,
) -> Response {
    let Some(database) = state.db.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "database not attached — start with --db <path>",
        )
            .into_response();
    };

    // Placeholder-free SELECT, so one query string serves both backends.
    let sql = "SELECT id, display_name, kind, state, updated_at, version
           FROM specs
           ORDER BY updated_at DESC, id ASC";
    let loaded = match database {
        crate::db::ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        crate::db::ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    };
    let mut specs: Vec<SpecRow> = match loaded {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(error = ?err, "load specs failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };

    // Access totals (#549) — a `spec_access` SUM per spec; absent ⇒ 0 —
    // and the last 14 days' series for the sparkline.
    let access = crate::db::spec_access::totals(database)
        .await
        .unwrap_or_default();
    let series = crate::db::spec_access::recent_series(database, 14)
        .await
        .unwrap_or_default();
    let spark = |id: &str| {
        series
            .get(id)
            .map(|s| crate::db::spec_access::sparkline_svg(s))
            .unwrap_or_default()
    };

    // Featured flag lives in `config_json`, not a column — fill it from a
    // single `list_all` pass so the table's star toggle (#521) shows state.
    {
        use std::collections::HashSet;
        let featured: HashSet<String> = crate::db::specs::list_all(database)
            .await
            .unwrap_or_default()
            .iter()
            .filter(|s| s.is_featured())
            .map(|s| s.id.clone())
            .collect();
        for row in &mut specs {
            row.featured = featured.contains(&row.id);
            row.access_count = access.get(&row.id).copied().unwrap_or(0);
            row.trend_svg = spark(&row.id);
        }
    }

    // Append specs that exist only in the YAML `--config` (not the DB) as
    // read-only "config-defined" rows (#303). They run + show on the
    // landing, so showing them here avoids the "where did my spec go?"
    // confusion; they can't be edited/deleted from the admin (the file is
    // their source).
    {
        use std::collections::HashSet;
        let db_ids: HashSet<String> = specs.iter().map(|s| s.id.clone()).collect();
        for s in &state.config.proxy.specs {
            if db_ids.contains(&s.id) {
                continue;
            }
            let kind = match s.kind() {
                ruscker_config::SpecKind::Shiny => "shiny",
                ruscker_config::SpecKind::InteractiveApp => "interactive",
                ruscker_config::SpecKind::Api => "api",
                ruscker_config::SpecKind::External => "external",
            };
            specs.push(SpecRow {
                id: s.id.clone(),
                display_name: s.display_name.clone(),
                kind: kind.to_string(),
                state: if s.template_properties.is_active() {
                    "active".into()
                } else {
                    "inactive".into()
                },
                updated_at: Utc::now(), // not shown for config rows
                version: 0,
                config_only: true,
                featured: s.is_featured(),
                access_count: access.get(&s.id).copied().unwrap_or(0),
                trend_svg: spark(&s.id),
            });
        }
    }

    let flash = build_flash(&state.locales, loc, &flash);
    let page = SpecsPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "specs",
        role: editor.role,
        specs,
        flash,
    };
    super::render(&page)
}

/// `POST /admin/specs/import` — multipart upload of a ShinyProxy
/// / Ruscker `application.yml`. Parses, runs the same warning
/// scan as `ruscker validate`, then `import_all` (idempotent +
/// non-destructive: specs in the DB but absent from the YAML are
/// kept). Redirects back to the list with a flash summary.
///
/// No separate dry-run/preview step yet (issue #8 wants one) —
/// import is idempotent and never deletes, so re-importing is
/// safe; a preview is a follow-up.
/// Step 1 of the import (#557): parse the uploaded YAML and render a
/// **preview** — the list of apps it carries, each marked new vs updates,
/// with a checkbox — so the operator confirms which to import. Nothing is
/// written to the DB here.
async fn import(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    mut multipart: Multipart,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };

    // Pull the YAML out of the `file` field.
    let mut raw: Option<String> = None;
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                if field.name() != Some("file") {
                    continue;
                }
                match field.bytes().await {
                    Ok(b) if !b.is_empty() => {
                        raw = Some(String::from_utf8_lossy(&b).into_owned());
                    }
                    Ok(_) => {} // empty file input
                    Err(e) => return redirect_err(&format!("read upload: {e}")),
                }
            }
            Ok(None) => break,
            Err(e) => return redirect_err(&format!("multipart parse: {e}")),
        }
    }
    let Some(raw) = raw else {
        return redirect_err("no file selected");
    };

    // Parse (env-interpolation + raw-text credential scan happen inside
    // from_yaml; parse failure → error flash).
    let config = match ruscker_config::Config::from_yaml(&raw) {
        Ok(c) => c,
        Err(e) => return redirect_err(&format!("YAML parse failed: {e}")),
    };
    let report = ruscker_config::validate::run(&config);
    let warning_count = report.warnings.len() + config.raw_warnings.len();

    // Which ids already exist in the DB (→ "updates", vs "new").
    let existing: std::collections::HashSet<String> = crate::db::specs::list_all(pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|s| s.id)
        .collect();

    let rows: Vec<ImportRow> = config
        .proxy
        .specs
        .iter()
        .map(|s| ImportRow {
            exists: existing.contains(&s.id),
            kind: match s.kind() {
                ruscker_config::SpecKind::Shiny => "shiny",
                ruscker_config::SpecKind::InteractiveApp => "interactive",
                ruscker_config::SpecKind::Api => "api",
                ruscker_config::SpecKind::External => "external",
            },
            display_name: s.display_name.clone().unwrap_or_default(),
            id: s.id.clone(),
        })
        .collect();

    let page = ImportPreviewPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "specs",
        role: editor.role,
        rows,
        raw_yaml: raw,
        warning_count,
    };
    super::render(&page)
}

/// Step 2 of the import (#557): import only the **checked** specs. The
/// preview form re-posts the original YAML (hidden) plus one `ids` field
/// per selected spec, as multipart so repeated `ids` collect cleanly.
async fn import_confirm(
    _: RequireEditor,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let mut raw: Option<String> = None;
    let mut ids: Vec<String> = Vec::new();
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                let name = field.name().map(str::to_string);
                match name.as_deref() {
                    Some("yaml") => raw = field.text().await.ok(),
                    Some("ids") => {
                        if let Ok(v) = field.text().await {
                            ids.push(v);
                        }
                    }
                    _ => {}
                }
            }
            Ok(None) => break,
            Err(e) => return redirect_err(&format!("multipart parse: {e}")),
        }
    }
    let Some(raw) = raw else {
        return redirect_err("missing config");
    };
    // Nothing checked → no-op, back to the list with a zero flash.
    if ids.is_empty() {
        return Redirect::to("/admin/specs?import=ok&created=0&updated=0&unchanged=0&warnings=0")
            .into_response();
    }
    let config = match ruscker_config::Config::from_yaml(&raw) {
        Ok(c) => c,
        Err(e) => return redirect_err(&format!("YAML parse failed: {e}")),
    };
    match crate::db::specs::import_selected(pool, &config, &ids).await {
        Ok(r) => {
            tracing::info!(
                created = r.created, updated = r.updated, unchanged = r.unchanged,
                selected = ids.len(), "selective YAML import via admin"
            );
            Redirect::to(&format!(
                "/admin/specs?import=ok&created={}&updated={}&unchanged={}&warnings=0",
                r.created, r.updated, r.unchanged
            ))
            .into_response()
        }
        Err(e) => {
            tracing::error!(error = ?e, "import_selected failed");
            redirect_err(&format!("import failed: {e}"))
        }
    }
}

fn redirect_err(msg: &str) -> Response {
    // Keep the message short + URL-safe enough for a query param.
    let encoded: String = msg
        .chars()
        .map(|c| match c {
            ' ' => '+',
            '&' | '#' | '?' | '=' => '_',
            c => c,
        })
        .take(200)
        .collect();
    Redirect::to(&format!("/admin/specs?import=err&msg={encoded}")).into_response()
}
