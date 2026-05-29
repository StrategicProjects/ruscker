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
    extract::{Path as AxumPath, State},
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

// Tech showcase logos — bundled into the binary so the seeded
// showcase specs (see migration 0009) render covers on a fresh
// install without operator-uploaded images. Files live under
// crates/ruscker-admin/assets/showcase/ and were provided as a
// package by the maintainer.
const SHOWCASE_BOKEH: &[u8] = include_bytes!("../../assets/showcase/bokeh.svg");
const SHOWCASE_DASH: &[u8] = include_bytes!("../../assets/showcase/dash.svg");
const SHOWCASE_FASTAPI: &[u8] = include_bytes!("../../assets/showcase/fastapi.svg");
const SHOWCASE_JUPYTER: &[u8] = include_bytes!("../../assets/showcase/jupyter.svg");
const SHOWCASE_PLUMBER: &[u8] = include_bytes!("../../assets/showcase/plumber.svg");
const SHOWCASE_QUARTO: &[u8] = include_bytes!("../../assets/showcase/quarto.svg");
const SHOWCASE_RMARKDOWN: &[u8] = include_bytes!("../../assets/showcase/rmarkdown.svg");
const SHOWCASE_RSTUDIO: &[u8] = include_bytes!("../../assets/showcase/rstudio.svg");
const SHOWCASE_SHINY: &[u8] = include_bytes!("../../assets/showcase/shiny.svg");
const SHOWCASE_STREAMLIT: &[u8] = include_bytes!("../../assets/showcase/streamlit.svg");
const SHOWCASE_VOILA: &[u8] = include_bytes!("../../assets/showcase/voila.svg");

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
        // Tech showcase logos. URLs match the seeded specs in migration 0009.
        .route("/assets/showcase/bokeh.svg", get(|| serve(SHOWCASE_BOKEH, SVG)))
        .route("/assets/showcase/dash.svg", get(|| serve(SHOWCASE_DASH, SVG)))
        .route("/assets/showcase/fastapi.svg", get(|| serve(SHOWCASE_FASTAPI, SVG)))
        .route("/assets/showcase/jupyter.svg", get(|| serve(SHOWCASE_JUPYTER, SVG)))
        .route("/assets/showcase/plumber.svg", get(|| serve(SHOWCASE_PLUMBER, SVG)))
        .route("/assets/showcase/quarto.svg", get(|| serve(SHOWCASE_QUARTO, SVG)))
        .route("/assets/showcase/rmarkdown.svg", get(|| serve(SHOWCASE_RMARKDOWN, SVG)))
        .route("/assets/showcase/rstudio.svg", get(|| serve(SHOWCASE_RSTUDIO, SVG)))
        .route("/assets/showcase/shiny.svg", get(|| serve(SHOWCASE_SHINY, SVG)))
        .route("/assets/showcase/streamlit.svg", get(|| serve(SHOWCASE_STREAMLIT, SVG)))
        .route("/assets/showcase/voila.svg", get(|| serve(SHOWCASE_VOILA, SVG)))
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
        // Operator-uploaded card images. DB first (Phase 2 image
        // library); on miss, fall back to the on-disk `images_dir`
        // (Phase 1 deploy that uses a static folder of files).
        .route("/assets/img/{filename}", get(serve_card_image))
}

async fn serve_card_image(
    State(state): State<AppState>,
    AxumPath(filename): AxumPath<String>,
) -> Response {
    // Don't allow `../` or directory components — Axum's path
    // extractor already collapses path segments per route, but
    // we double-check before any FS read.
    if filename.contains('/') || filename.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    // 1. DB lookup
    if let Some(pool) = state.db.as_ref() {
        match crate::db::images::fetch_by_filename(pool, &filename).await {
            Ok(Some((mime, bytes))) => return serve_dynamic(bytes, &mime),
            Ok(None) => {}
            Err(err) => {
                tracing::error!(error = ?err, filename, "image DB lookup failed");
            }
        }
    }

    // 2. Disk fallback
    if let Some(dir) = state.images_dir.as_ref() {
        let candidate = dir.join(&filename);
        if let Ok(bytes) = tokio::fs::read(&candidate).await {
            let mime = mime_from_extension(&filename);
            return serve_dynamic(bytes, mime);
        }
    }

    StatusCode::NOT_FOUND.into_response()
}

fn mime_from_extension(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, ext)| ext.to_ascii_lowercase()) {
        Some(ref e) if e == "webp" => "image/webp",
        Some(ref e) if e == "png" => "image/png",
        Some(ref e) if e == "jpg" || e == "jpeg" => "image/jpeg",
        Some(ref e) if e == "svg" => "image/svg+xml",
        Some(ref e) if e == "gif" => "image/gif",
        _ => "application/octet-stream",
    }
}

/// Like [`serve`] but takes owned bytes (DB blob / FS read) and
/// short-cache headers (these images can change without a binary
/// rebuild so `immutable` would be wrong here).
///
/// These bytes are **operator-uploaded** (DB / disk), so the
/// response is hardened against the SVG-script vector: a
/// malicious `<script>`/`<foreignObject>` inside an uploaded SVG
/// must not execute even if someone navigates to it directly or
/// embeds it via `<object>`/`<iframe>`.
///
/// - `X-Content-Type-Options: nosniff` — the browser honors our
///   declared type, no sniffing a PNG-named blob into HTML.
/// - `Content-Security-Policy: default-src 'none'; …; sandbox` —
///   neuters any active content in an SVG document regardless of
///   embedding context. Harmless for raster images (they need no
///   sources). Does NOT interfere with the common
///   `<img src="/assets/img/x.svg">` use — scripts never run in
///   `<img>` context anyway.
fn serve_dynamic(body: Vec<u8>, content_type: &str) -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(ct) = HeaderValue::from_str(content_type) {
        headers.insert(header::CONTENT_TYPE, ct);
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60, must-revalidate"),
    );
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"),
    );
    (StatusCode::OK, headers, body).into_response()
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn serve_dynamic_hardens_user_uploaded_images() {
        // A served (operator-uploaded) image must carry the
        // SVG-script mitigations regardless of its real type.
        let resp = serve_dynamic(b"<svg/>".to_vec(), "image/svg+xml");
        let h = resp.headers();
        assert_eq!(h.get(header::CONTENT_TYPE).unwrap(), "image/svg+xml");
        assert_eq!(h.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
        let csp = h
            .get(header::CONTENT_SECURITY_POLICY)
            .unwrap()
            .to_str()
            .unwrap();
        assert!(csp.contains("default-src 'none'"));
        assert!(csp.contains("sandbox"));
        // Body survives unchanged.
        let body = to_bytes(resp.into_body(), 1024).await.unwrap();
        assert_eq!(&body[..], b"<svg/>");
    }

    #[test]
    fn mime_from_extension_maps_known_types() {
        assert_eq!(mime_from_extension("a.png"), "image/png");
        assert_eq!(mime_from_extension("a.svg"), "image/svg+xml");
        assert_eq!(mime_from_extension("a.webp"), "image/webp");
        assert_eq!(mime_from_extension("a.unknown"), "application/octet-stream");
    }
}
