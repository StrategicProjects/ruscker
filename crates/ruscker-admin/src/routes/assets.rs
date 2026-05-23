//! Static assets served directly from the binary.
//!
//! Everything under `/assets/` is embedded at compile time:
//!
//! - `/assets/styles.css` — Tailwind 4 output (built by `build.rs`)
//! - `/assets/icons/tabler-icons.min.css` — Tabler icon font CSS
//! - `/assets/icons/tabler-icons.woff2` — Tabler icon font binary
//! - `/assets/fonts/jost-latin-{400,500,600}-normal.woff2` — Jost
//! - `/assets/js/alpine.min.js` — Alpine.js 3.x
//!
//! Assets are immutable per build (their bytes can only change when
//! the binary itself does), so they all carry a one-year
//! `cache-control: immutable` header — safe to cache aggressively.

use axum::{
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};

use crate::AppState;

// ── Embedded assets ───────────────────────────────────────────────

const STYLES_CSS: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/styles.css"));

const TABLER_CSS: &[u8] = include_bytes!("../../assets/icons/tabler-icons.min.css");
const TABLER_FONT: &[u8] = include_bytes!("../../assets/icons/tabler-icons.woff2");

const JOST_400: &[u8] = include_bytes!("../../assets/fonts/jost-latin-400-normal.woff2");
const JOST_500: &[u8] = include_bytes!("../../assets/fonts/jost-latin-500-normal.woff2");
const JOST_600: &[u8] = include_bytes!("../../assets/fonts/jost-latin-600-normal.woff2");

const ALPINE_JS: &[u8] = include_bytes!("../../assets/js/alpine.min.js");

// ── Router ────────────────────────────────────────────────────────

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/assets/styles.css",
            get(|| serve(STYLES_CSS, "text/css; charset=utf-8")),
        )
        .route(
            "/assets/icons/tabler-icons.min.css",
            get(|| serve(TABLER_CSS, "text/css; charset=utf-8")),
        )
        .route(
            "/assets/icons/tabler-icons.woff2",
            get(|| serve(TABLER_FONT, "font/woff2")),
        )
        .route(
            "/assets/fonts/jost-latin-400-normal.woff2",
            get(|| serve(JOST_400, "font/woff2")),
        )
        .route(
            "/assets/fonts/jost-latin-500-normal.woff2",
            get(|| serve(JOST_500, "font/woff2")),
        )
        .route(
            "/assets/fonts/jost-latin-600-normal.woff2",
            get(|| serve(JOST_600, "font/woff2")),
        )
        .route(
            "/assets/js/alpine.min.js",
            get(|| serve(ALPINE_JS, "application/javascript; charset=utf-8")),
        )
}

// ── Helpers ───────────────────────────────────────────────────────

async fn serve(body: &'static [u8], content_type: &'static str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=31536000, immutable"),
    );
    (StatusCode::OK, headers, body).into_response()
}
