//! Static assets served directly from the binary.
//!
//! Everything under `/assets/` is embedded at compile time:
//!
//! - `/assets/styles.css` — output of Tailwind 4, built by `build.rs`
//! - `/assets/htmx.min.js` — HTMX 2.0.x (pinned in Cargo.toml comment)
//! - `/assets/alpine.min.js` — Alpine.js 3.x
//! - `/assets/tabler-icons.min.css` — Tabler icon font
//! - `/assets/fonts/*` — self-hosted Jost
//!
//! Phase 1 ships styles.css from Tailwind. The JS / icon / font
//! assets fold in as the templates start using them.

use axum::{
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

use crate::AppState;

/// Compiled Tailwind output, embedded in the binary.
const STYLES_CSS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/styles.css"));

pub fn routes() -> Router<AppState> {
    Router::new().route("/assets/styles.css", get(styles_css))
}

async fn styles_css() -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    // Embedded assets are immutable per build — long-cache safely.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    (StatusCode::OK, headers, STYLES_CSS).into_response()
}
