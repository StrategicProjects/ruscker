//! Public landing page (`GET /`).
//!
//! Renders `templates/landing.html` with the parsed `Config`, the
//! resolved locale (cookie → Accept-Language → pt-BR), and the
//! user's theme choice (cookie → auto).

use askama::Template;
use axum::{
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use fluent_bundle::{FluentArgs, FluentValue};

use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::view_model::{build_type_chips, sort_by_recent, CardCounts, CardCtx, TypeChip};
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(index))
}

#[derive(Template)]
#[template(path = "landing.html")]
struct LandingPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    cards: Vec<CardCtx<'a>>,
    type_chips: Vec<TypeChip>,
    counts: CardCounts,
}

impl<'a> LandingPage<'a> {
    /// Translation helper used by the template as `self.t("key")`.
    /// Centralizing here keeps templates clean of explicit
    /// bundle/locale handling.
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// Translation with a single Fluent variable (most common case
    /// is `{ $date }`). Avoids dragging the `FluentArgs` builder
    /// into templates.
    fn t_with(&self, key: &str, arg_name: &str, value: &str) -> String {
        let mut args = FluentArgs::new();
        args.set(arg_name.to_string(), FluentValue::from(value.to_string()));
        self.locales.t(self.locale, key, Some(&args))
    }
}

async fn index(State(state): State<AppState>, loc: Locale, theme: Theme) -> Response {
    let mut cards: Vec<CardCtx<'_>> = state
        .config
        .proxy
        .specs
        .iter()
        .map(CardCtx::from_spec)
        .collect();
    sort_by_recent(&mut cards);
    let type_chips = build_type_chips(&cards);
    let counts = CardCounts {
        total: cards.len(),
    };
    let page = LandingPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        cards,
        type_chips,
        counts,
    };
    render(&page)
}

/// Centralized `askama::Template` → axum `Response`. Replaces the
/// deprecated `askama_axum` crate.
fn render<T: Template>(t: &T) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(error = ?err, "template render failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "template error").into_response()
        }
    }
}
