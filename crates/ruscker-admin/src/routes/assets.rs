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

use std::sync::LazyLock;

use axum::{
    extract::{Path as AxumPath, Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use dashmap::DashMap;

use crate::AppState;

/// Max dimension (px) for a gallery thumbnail (`/assets/img/x?thumb`).
/// The picker tiles render at ~48 px; 96 covers HiDPI.
const THUMB_MAX_DIM: u32 = 96;

/// Process-wide cache of generated thumbnails, **content-addressed** by
/// `(filename, hash-of-source-bytes)`.
///
/// Keying on the source-blob hash makes the cache correct under
/// active-active HA: a re-upload on *another* node (the catalog DB is
/// shared) changes the bytes, hence the hash, hence the key — so this
/// node misses and regenerates instead of serving a stale thumbnail
/// (#313). Keying on filename alone could not see another node's upload,
/// because [`invalidate_thumb`] is process-local.
///
/// First request for a given `(name, bytes)` pays the decode+resize; the
/// rest hit the cache. We still fetch the (small) source blob on every
/// request — the genuinely expensive work the cache skips is the
/// decode+resize, not the DB read. [`invalidate_thumb`] still prunes a
/// filename's now-stale entries on the local node so memory stays bounded
/// after a re-upload/delete here (#301).
static THUMB_CACHE: LazyLock<DashMap<(String, u64), Vec<u8>>> = LazyLock::new(DashMap::new);

/// Drop a filename's cached thumbnails. With the content-addressed key
/// (#313) the cache is already cross-node correct — changed bytes hash to
/// a different key, so no node serves a stale thumbnail — but the old
/// entry would otherwise linger on *this* node until the process exits.
/// Pruning every entry for the filename on a local upload/delete keeps
/// memory bounded (the intent of #301).
pub(crate) fn invalidate_thumb(filename: &str) {
    THUMB_CACHE.retain(|(name, _), _| name != filename);
}

/// Fast, non-cryptographic content hash for the thumbnail cache key —
/// same hasher family as [`etag_for`]. Good enough to detect a changed
/// blob; not security-sensitive.
fn content_hash(bytes: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod thumb_cache_tests {
    use super::*;

    #[test]
    fn content_hash_changes_with_bytes() {
        // The whole point of #313: changed bytes ⇒ a different cache key,
        // so a re-upload (here or on another HA node) can't be served the
        // old thumbnail.
        let a = content_hash(b"original-png-bytes");
        let b = content_hash(b"re-uploaded-png-bytes");
        assert_ne!(a, b, "different bytes must hash to different keys");
        assert_eq!(a, content_hash(b"original-png-bytes"), "stable for same bytes");
    }

    #[test]
    fn invalidate_thumb_prunes_only_the_named_file() {
        // Seed two filenames (one with two content versions) and confirm
        // invalidating one leaves the other's entries intact.
        THUMB_CACHE.insert(("keep.png".into(), 1), vec![1]);
        THUMB_CACHE.insert(("drop.png".into(), 1), vec![2]);
        THUMB_CACHE.insert(("drop.png".into(), 2), vec![3]);

        invalidate_thumb("drop.png");

        assert!(THUMB_CACHE.contains_key(&("keep.png".into(), 1)), "other file kept");
        assert!(!THUMB_CACHE.contains_key(&("drop.png".into(), 1)), "stale version pruned");
        assert!(!THUMB_CACHE.contains_key(&("drop.png".into(), 2)), "all versions pruned");

        // Don't leak test state into other tests sharing the process-wide map.
        THUMB_CACHE.remove(&("keep.png".into(), 1));
    }
}

/// `?thumb` query flag on `/assets/img/{filename}`.
#[derive(serde::Deserialize)]
struct ImgQuery {
    #[serde(default)]
    thumb: Option<String>,
}

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
const SHOWCASE_SHINY_FOR_PYTHON: &[u8] =
    include_bytes!("../../assets/showcase/shiny-for-python.svg");
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

// Raster favicons (#374). The SVG favicon below is ignored by some
// browsers (notably Safari), which then fall back to `/favicon.ico` —
// previously unserved, so the tab showed a black placeholder. These are
// rendered from the flat mark (3 bars) and bundled.
const FAVICON_ICO: &[u8] = include_bytes!("../../assets/brand/favicon.ico");
const FAVICON_PNG: &[u8] = include_bytes!("../../assets/brand/favicon-32.png");
const APPLE_TOUCH_ICON: &[u8] = include_bytes!("../../assets/brand/apple-touch-icon.png");

// ── Router ────────────────────────────────────────────────────────

const SVG: &str = "image/svg+xml";
const PNG: &str = "image/png";
const ICO: &str = "image/x-icon";

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/assets/styles.css",
            get(|q| serve_versioned(q, STYLES_CSS, "text/css; charset=utf-8")),
        )
        .route(
            "/assets/icons/tabler-icons.min.css",
            get(|q| serve_versioned(q, TABLER_CSS, "text/css; charset=utf-8")),
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
            get(|q| serve_versioned(q, ALPINE_JS, "application/javascript; charset=utf-8")),
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
        .route(
            "/assets/showcase/shiny-for-python.svg",
            get(|| serve(SHOWCASE_SHINY_FOR_PYTHON, SVG)),
        )
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
        // Conventional well-known browser hooks. SVG for modern
        // browsers; raster `.ico`/`.png` so Safari & co. don't fall
        // back to a black placeholder (#374).
        .route("/favicon.svg", get(|| serve(BRAND_MARK_FLAT, SVG)))
        .route("/favicon.ico", get(|| serve(FAVICON_ICO, ICO)))
        .route("/favicon-32.png", get(|| serve(FAVICON_PNG, PNG)))
        .route("/apple-touch-icon.png", get(|| serve(APPLE_TOUCH_ICON, PNG)))
        // Operator-uploaded card images. DB first (Phase 2 image
        // library); on miss, fall back to the on-disk `images_dir`
        // (Phase 1 deploy that uses a static folder of files).
        .route("/assets/img/{filename}", get(serve_card_image))
}

async fn serve_card_image(
    State(state): State<AppState>,
    AxumPath(filename): AxumPath<String>,
    Query(q): Query<ImgQuery>,
    req_headers: HeaderMap,
) -> Response {
    // Don't allow `../` or directory components — Axum's path
    // extractor already collapses path segments per route, but
    // we double-check before any FS read.
    if filename.contains('/') || filename.contains("..") {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let want_thumb = q.thumb.is_some();

    // Resolve the full image bytes: DB first, then the on-disk fallback.
    // We fetch on every `?thumb` request too — the source hash is the
    // cache key (#313), so the bytes are needed to look the thumbnail up.
    // The blob is small; the expensive step the cache still skips is the
    // decode+resize below.
    let Some((mime, bytes)) = fetch_image_bytes(&state, &filename).await else {
        return StatusCode::NOT_FOUND.into_response();
    };

    // Vector / no resize needed → serve as-is. Raster + `?thumb` →
    // serve the content-addressed cached thumbnail if present, else
    // generate one (off the async worker), cache, and serve. The key is
    // `(filename, hash(bytes))`, so a re-upload on any HA node naturally
    // misses here and regenerates — never a stale thumbnail (#313).
    if want_thumb && mime != "image/svg+xml" {
        let key = (filename.clone(), content_hash(&bytes));
        if let Some(cached) = THUMB_CACHE.get(&key) {
            return serve_thumbnail_bytes(&req_headers, cached.clone());
        }
        let src = bytes.clone();
        match tokio::task::spawn_blocking(move || {
            crate::images::thumbnail_webp(&src, THUMB_MAX_DIM)
        })
        .await
        {
            Ok(Ok(thumb)) => {
                THUMB_CACHE.insert(key, thumb.clone());
                return serve_thumbnail_bytes(&req_headers, thumb);
            }
            Ok(Err(err)) => {
                tracing::warn!(error = ?err, filename, "thumbnail generation failed; serving full image");
            }
            Err(err) => {
                tracing::warn!(error = ?err, filename, "thumbnail task join failed; serving full image");
            }
        }
    }

    // Full image: ETag-validated so a revalidation returns a cheap 304
    // instead of re-sending the blob (#290).
    let etag = etag_for(&bytes);
    if if_none_match(&req_headers, &etag) {
        return not_modified(&etag);
    }
    serve_dynamic(bytes, &mime, &etag)
}

/// Fetch an operator-uploaded image's `(mime, bytes)` — DB first, then
/// the on-disk `images_dir` fallback. `None` if it isn't found anywhere.
async fn fetch_image_bytes(state: &AppState, filename: &str) -> Option<(String, Vec<u8>)> {
    if let Some(pool) = state.db.as_ref() {
        match crate::db::images::fetch_by_filename(pool, filename).await {
            Ok(Some((mime, bytes))) => return Some((mime, bytes)),
            Ok(None) => {}
            Err(err) => tracing::error!(error = ?err, filename, "image DB lookup failed"),
        }
    }
    if let Some(dir) = state.images_dir.as_ref() {
        let candidate = dir.join(filename);
        if let Ok(bytes) = tokio::fs::read(&candidate).await {
            return Some((mime_from_extension(filename).to_string(), bytes));
        }
    }
    None
}

/// Serve a generated WebP thumbnail. Derived deterministically from an
/// immutable filename, so it can be cached hard (a day).
fn serve_thumbnail_bytes(req_headers: &HeaderMap, body: Vec<u8>) -> Response {
    // A filename can be re-uploaded (#301), so a thumb isn't truly
    // immutable — cache modestly and lean on the ETag to make a
    // revalidation a cheap 304 when the bytes are unchanged.
    let etag = etag_for(&body);
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    if let Ok(v) = HeaderValue::from_str(&etag) {
        headers.insert(header::ETAG, v);
    }
    if if_none_match(req_headers, &etag) {
        return (StatusCode::NOT_MODIFIED, headers).into_response();
    }
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/webp"));
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    (StatusCode::OK, headers, body).into_response()
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
fn serve_dynamic(body: Vec<u8>, content_type: &str, etag: &str) -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(ct) = HeaderValue::from_str(content_type) {
        headers.insert(header::CONTENT_TYPE, ct);
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60, must-revalidate"),
    );
    // ETag so a revalidation can return a cheap 304 instead of re-sending
    // the whole blob (#290).
    if let Ok(v) = HeaderValue::from_str(etag) {
        headers.insert(header::ETAG, v);
    }
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

/// A weak-ish ETag for an image blob — a fast non-crypto hash of the
/// bytes (it's a cache validator, not a security control). Changes iff
/// the content changes.
fn etag_for(bytes: &[u8]) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut h);
    format!("\"{:x}\"", h.finish())
}

/// Does the request's `If-None-Match` match `etag` (or `*`)?
fn if_none_match(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .map(|inm| inm.split(',').any(|t| t.trim() == etag || t.trim() == "*"))
        .unwrap_or(false)
}

/// `304 Not Modified` carrying the validator + cache directives.
fn not_modified(etag: &str) -> Response {
    let mut headers = HeaderMap::new();
    if let Ok(v) = HeaderValue::from_str(etag) {
        headers.insert(header::ETAG, v);
    }
    headers.insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=60, must-revalidate"),
    );
    (StatusCode::NOT_MODIFIED, headers).into_response()
}

// ── Helpers ───────────────────────────────────────────────────────

/// `?v=<version>` cache-busting flag on a bundled asset URL (#289).
#[derive(serde::Deserialize)]
struct AssetVer {
    v: Option<String>,
}

fn serve_with_cache(body: &'static [u8], content_type: &'static str, cache: &'static str) -> Response {
    let mut headers = HeaderMap::new();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    (StatusCode::OK, headers, body).into_response()
}

async fn serve(body: &'static [u8], content_type: &'static str) -> Response {
    // Cache for a short window (no `must-revalidate`) so the admin's
    // full-page-reload navigation doesn't fire a conditional-GET
    // round-trip for every bundled asset on every click (#269). 5 min
    // covers an active session; a new binary's assets are picked up
    // promptly (a hard reload always bypasses). `immutable` would be
    // wrong for an un-versioned URL — the bytes change across upgrades.
    serve_with_cache(body, content_type, "public, max-age=300")
}

/// Serve a bundled asset whose URL carries `?v=<version>` (#289). The
/// version stamps the URL per release, so the bytes for a given URL are
/// immutable — cache them hard for a year. A request without `?v`
/// (a stale link, or a direct hit) falls back to the short cache, so we
/// never serve stale bytes under a non-versioned URL.
async fn serve_versioned(
    Query(q): Query<AssetVer>,
    body: &'static [u8],
    content_type: &'static str,
) -> Response {
    if q.v.is_some() {
        serve_with_cache(body, content_type, "public, max-age=31536000, immutable")
    } else {
        serve(body, content_type).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;

    #[tokio::test]
    async fn serve_dynamic_hardens_user_uploaded_images() {
        // A served (operator-uploaded) image must carry the
        // SVG-script mitigations regardless of its real type.
        let resp = serve_dynamic(b"<svg/>".to_vec(), "image/svg+xml", "\"abc\"");
        let h = resp.headers();
        assert_eq!(h.get(header::CONTENT_TYPE).unwrap(), "image/svg+xml");
        assert_eq!(h.get(header::X_CONTENT_TYPE_OPTIONS).unwrap(), "nosniff");
        assert_eq!(h.get(header::ETAG).unwrap(), "\"abc\"");
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
