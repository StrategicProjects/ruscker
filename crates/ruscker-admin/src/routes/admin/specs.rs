//! Admin > Apps list page.
//!
//! Read-only for now: lists every spec in the DB with kind, state,
//! version, updated_at. Edit/delete buttons are placeholders until
//! the spec form lands in the next slice.

use askama::Template;
use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use chrono::{DateTime, Utc};
use sqlx::FromRow;

use crate::auth::AdminSession;
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/admin/specs", get(index))
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
}

#[derive(Template)]
#[template(path = "admin/specs.html")]
struct SpecsPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    nav_section: &'static str,
    specs: Vec<SpecRow>,
}

impl<'a> SpecsPage<'a> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

async fn index(
    _: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "database not attached — start with --db <path>",
        )
            .into_response();
    };

    let specs: Vec<SpecRow> = match sqlx::query_as(
        "SELECT id, display_name, kind, state, updated_at, version
           FROM specs
           ORDER BY updated_at DESC, id ASC",
    )
    .fetch_all(pool)
    .await
    {
        Ok(rows) => rows,
        Err(err) => {
            tracing::error!(error = ?err, "load specs failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };

    let page = SpecsPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        nav_section: "specs",
        specs,
    };
    super::render(&page)
}
