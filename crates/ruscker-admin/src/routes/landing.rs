//! Public landing page (`GET /`).
//!
//! Phase 1: minimal text response proving the i18n/state pipeline is
//! wired. The full Tailwind/Askama template lands in a follow-up
//! commit on this same branch.

use crate::i18n::Locale;
use crate::view_model::CardCtx;
use crate::AppState;
use axum::{extract::State, response::Html, routing::get, Router};

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(index))
}

async fn index(State(state): State<AppState>) -> Html<String> {
    // Locale negotiation will read cookie + Accept-Language in the
    // next commit. For the bring-up we render in the default locale.
    let loc = Locale::Pt;
    let title = state.locales.t(loc, "landing-title", None);

    let cards: Vec<CardCtx<'_>> = state
        .config
        .proxy
        .specs
        .iter()
        .map(CardCtx::from_spec)
        .collect();

    // Stand-in HTML until the Askama template is added. Listing the
    // specs proves both Config and view_model are wired correctly.
    let items = cards
        .iter()
        .map(|c| {
            format!(
                "<li><strong>{}</strong> [{}] — {} <em>(active={}, href={})</em></li>",
                escape(c.display_name),
                c.kind_label,
                escape(c.description),
                c.active,
                escape(&c.href),
            )
        })
        .collect::<String>();

    Html(format!(
        "<!doctype html><meta charset=utf-8><title>{title}</title>\
         <h1>{title}</h1>\
         <p>Ruscker phase 1 bring-up. {count} specs loaded.</p>\
         <ul>{items}</ul>",
        title = escape(&title),
        count = cards.len(),
    ))
}

/// Tiny HTML escape until the Askama templates take over. Escapes
/// the five characters that matter for text/attribute contexts.
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}
