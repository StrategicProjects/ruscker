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

// Ruscker brand kit — see docs/BRAND.md for usage rules.
const BRAND_MARK: &[u8] = include_bytes!("../../assets/brand/ruscker-mark.svg");
const BRAND_MARK_FLAT: &[u8] = include_bytes!("../../assets/brand/ruscker-mark-flat.svg");
const BRAND_MARK_MONO: &[u8] = include_bytes!("../../assets/brand/ruscker-mark-mono-black.svg");
const BRAND_MARK_KNOCKOUT: &[u8] = include_bytes!("../../assets/brand/ruscker-mark-knockout.svg");
const BRAND_LOCKUP_H: &[u8] = include_bytes!("../../assets/brand/ruscker-lockup-horizontal.svg");
const BRAND_LOCKUP_V: &[u8] = include_bytes!("../../assets/brand/ruscker-lockup-vertical.svg");
const BRAND_WORDMARK: &[u8] = include_bytes!("../../assets/brand/ruscker-wordmark.svg");
const BRAND_APP_ICON: &[u8] = include_bytes!("../../assets/brand/ruscker-app-icon.svg");

// ── Router ────────────────────────────────────────────────────────

const SVG: &str = "image/svg+xml";

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
        // Brand kit. URLs match docs/BRAND.md.
        .route("/assets/brand/mark.svg", get(|| serve(BRAND_MARK, SVG)))
        .route("/assets/brand/mark-flat.svg", get(|| serve(BRAND_MARK_FLAT, SVG)))
        .route("/assets/brand/mark-mono-black.svg", get(|| serve(BRAND_MARK_MONO, SVG)))
        .route("/assets/brand/mark-knockout.svg", get(|| serve(BRAND_MARK_KNOCKOUT, SVG)))
        .route("/assets/brand/lockup-horizontal.svg", get(|| serve(BRAND_LOCKUP_H, SVG)))
        .route("/assets/brand/lockup-vertical.svg", get(|| serve(BRAND_LOCKUP_V, SVG)))
        .route("/assets/brand/wordmark.svg", get(|| serve(BRAND_WORDMARK, SVG)))
        .route("/assets/brand/app-icon.svg", get(|| serve(BRAND_APP_ICON, SVG)))
        // Conventional well-known browser hooks.
        .route("/favicon.svg", get(|| serve(BRAND_MARK_FLAT, SVG)))
        .route("/apple-touch-icon.png", get(|| serve(BRAND_APP_ICON, SVG)))
}

// ── Helpers ───────────────────────────────────────────────────────

async fn serve(body: &'static [u8], content_type: &'static str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    // We DO want browser caching, but `immutable` makes Chrome never
    // revalidate — even on a hard reload — which surprises users when
    // the binary ships a new asset under the same URL. `no-cache` here
    // means "always revalidate", not "never cache". A proper fix
    // (Phase 5) is hash-bearing URLs (`/assets/styles-<hash>.css`)
    // served back with the immutable header.
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=0, must-revalidate"),
    );
    (StatusCode::OK, headers, body).into_response()
}
