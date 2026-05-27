//! Admin > Audit log viewer.
//!
//! Read-only listing of the `audit_log` table with a handful of
//! filters (action family, target substring, actor). Rows expand
//! to show `diff_json` pretty-printed. No mutation routes — this
//! is a forensic surface, not an editor.

use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use serde::Deserialize;

use crate::auth::{RequireAdmin, Role};
use crate::db;
use crate::db::audit::{ActionFamily, AuditEntry, AuditFilter};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/admin/audit", get(index))
}

#[derive(Debug, Deserialize, Default)]
pub struct AuditQuery {
    /// "spec" | "image" | "credential" | "landing" | "" (all)
    #[serde(default)]
    pub family: String,
    /// Substring match against target (`spec:sales-dashboard`,
    /// `image:abc-123`, …).
    #[serde(default)]
    pub target: String,
    /// Exact match against actor column.
    #[serde(default)]
    pub actor: String,
}

#[derive(Template)]
#[template(path = "admin/audit.html")]
struct AuditPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    nav_section: &'static str,
    /// Current session role (always Admin here) - drives nav gating.
    role: Role,
    entries: Vec<AuditEntry>,
    filter: AuditQuery,
    /// Distinct actors ever seen — populates the actor select.
    /// Empty in the typical single-operator install.
    distinct_actors: Vec<String>,
}

impl<'a> AuditPage<'a> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// Pretty-printed JSON for the expanded row. Returns "" when
    /// the entry had no diff_json (so the template can `{% if %}`
    /// it out without complicating the helper signature).
    fn pretty_diff(&self, e: &AuditEntry) -> String {
        match &e.diff {
            Some(v) => serde_json::to_string_pretty(v).unwrap_or_default(),
            None => String::new(),
        }
    }

    /// `spec.create` → "spec.create", with a Tabler icon name
    /// keyed to the family. Used by the row badge.
    fn action_icon(&self, action: &str) -> &'static str {
        match action.split_once('.').map(|(f, _)| f) {
            Some("spec") => "ti-app-window",
            Some("image") => "ti-photo",
            Some("credential") => "ti-key",
            Some("landing") => "ti-layout",
            _ => "ti-history",
        }
    }

    /// Color hint for the action verb (suffix after the dot).
    /// create=green, update=blue, delete=red, import=teal.
    fn action_tone(&self, action: &str) -> &'static str {
        match action.rsplit('.').next() {
            Some("create") | Some("upload") => "tone-create",
            Some("update") => "tone-update",
            Some("delete") => "tone-delete",
            Some("import") => "tone-import",
            _ => "tone-other",
        }
    }
}

async fn index(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(q): Query<AuditQuery>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };

    let filter = AuditFilter {
        family: ActionFamily::parse(&q.family),
        target_contains: empty_to_none(&q.target),
        actor: empty_to_none(&q.actor),
        ..AuditFilter::new()
    };

    let entries = match db::audit::list(pool, &filter).await {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(error = ?err, "audit list failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };

    // Distinct actors only worth showing when there are at least
    // two — single-operator installs leak no information by
    // omitting the select.
    let distinct_actors: Vec<String> = db::audit::distinct_actors(pool).await.unwrap_or_default();

    let page = AuditPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        nav_section: "audit",
        role: Role::Admin,
        entries,
        filter: q,
        distinct_actors,
    };
    super::render(&page)
}

fn empty_to_none(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        None
    } else {
        Some(t.to_string())
    }
}
