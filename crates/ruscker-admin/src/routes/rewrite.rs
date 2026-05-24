//! Response transformation: inject `<base href>` into HTML
//! responses so apps rendered behind `/app/{spec}/` resolve
//! their relative URLs against that prefix instead of the
//! server root.
//!
//! ## Why this exists
//!
//! Shiny / Streamlit / Dash / Voilà templates emit URLs as if
//! they were mounted at `/`:
//!
//! ```html
//! <script src="/sockjs/info"></script>
//! <link rel="stylesheet" href="/lib/bootstrap.css">
//! ```
//!
//! Behind Ruscker's `/app/{spec}/` prefix, the browser resolves
//! those against `/`, gets a 404 from Ruscker, and the app
//! fails to load.
//!
//! Injecting a single `<base href="/app/{spec}/">` into `<head>`
//! tells the browser to resolve every relative URL against the
//! app's prefix. **Relative** URLs are handled correctly with no
//! further intervention; **absolute** URLs (those starting with
//! `/`) still skip the base and break — those need either an
//! upstream config change (e.g. Shiny's `options(shiny.sanitize.errors)`)
//! or a JS shim, both out of scope for this slice.
//!
//! ## Trade-offs
//!
//! - **Collects the full body in memory** before injecting. Fine
//!   for the typical few-KB initial HTML payload; not appropriate
//!   for multi-MB responses. Capped at [`MAX_HTML_BYTES`] to
//!   prevent runaway memory on a misconfigured upstream.
//! - **No streaming.** Could chunk-scan for `</head>` to avoid
//!   the collect, but adds state machine complexity for a single
//!   small-payload use case. Re-evaluate if profiling shows it.
//! - **Case-insensitive** `</head>` match — HTML lets it be
//!   `</HEAD>` or `</Head>`.
//! - **Idempotent**: if the upstream already emits a `<base>`
//!   tag, we still insert ours. The browser uses the first
//!   `<base>` it sees; ours wins because we put it before any
//!   pre-existing one. Good for fix-mounted apps, bad if the
//!   operator deliberately set a different base — unlikely
//!   for the apps we host.

use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderValue, Response};

/// Hard cap on the body size we'll collect to inject the base
/// tag. 2 MB is generous for any sensible initial HTML payload;
/// past that we give up and pass the response through untouched
/// to avoid pinning megabytes of memory per request.
const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;

/// If `resp` is an HTML response, inject a `<base href>` tag
/// pointing at `base_path` and return the rewritten response.
/// Non-HTML responses pass through unchanged.
///
/// `base_path` should be the user-visible URL prefix the app is
/// mounted at, **with a trailing slash** — e.g. `/app/auroraprime/`.
/// The trailing slash matters because the browser strips
/// everything after the last `/` when resolving relative URLs.
pub async fn inject_base_href(resp: Response<Body>, base_path: &str) -> Response<Body> {
    if !is_html(&resp) {
        return resp;
    }

    let (mut parts, body) = resp.into_parts();
    let bytes = match to_bytes(body, MAX_HTML_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            tracing::warn!(
                error = ?e,
                "could not collect HTML body for base-href injection; passing through empty"
            );
            return Response::from_parts(parts, Body::empty());
        }
    };

    // No body to rewrite — most often a HEAD response. Keep
    // parts intact so the upstream's Content-Length (which
    // reflects what a GET *would* return) survives.
    if bytes.is_empty() {
        return Response::from_parts(parts, Body::empty());
    }

    let rewritten = inject_into_head(bytes.as_ref(), base_path);

    // Body length changed — overwrite Content-Length so the
    // client doesn't truncate on a stale value.
    parts.headers.remove(header::CONTENT_LENGTH);
    if let Ok(v) = HeaderValue::from_str(&rewritten.len().to_string()) {
        parts.headers.insert(header::CONTENT_LENGTH, v);
    }

    Response::from_parts(parts, Body::from(rewritten))
}

fn is_html(resp: &Response<Body>) -> bool {
    resp.headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            let lower = s.trim().to_ascii_lowercase();
            lower.starts_with("text/html")
        })
        .unwrap_or(false)
}

/// Insert `<base href="{base_path}">` just before the first
/// case-insensitive `</head>` occurrence. If no `</head>` is
/// found (malformed HTML, fragment response) we return the
/// bytes untouched — better to ship a slightly-broken page
/// than to mangle one that might still render.
fn inject_into_head(html: &[u8], base_path: &str) -> Vec<u8> {
    let needle = b"</head>";
    let Some(pos) = find_case_insensitive(html, needle) else {
        return html.to_vec();
    };
    let tag = format!("<base href=\"{}\">", base_path);
    let mut out = Vec::with_capacity(html.len() + tag.len());
    out.extend_from_slice(&html[..pos]);
    out.extend_from_slice(tag.as_bytes());
    out.extend_from_slice(&html[pos..]);
    out
}

/// Case-insensitive byte-slice search. Compares the needle as
/// already-lowercase, the haystack one window at a time. O(n*m)
/// but m=7 for `</head>` so it's effectively O(n).
fn find_case_insensitive(haystack: &[u8], needle_lower: &[u8]) -> Option<usize> {
    if needle_lower.is_empty() || haystack.len() < needle_lower.len() {
        return None;
    }
    haystack
        .windows(needle_lower.len())
        .position(|win| {
            win.iter()
                .zip(needle_lower.iter())
                .all(|(h, n)| h.eq_ignore_ascii_case(n))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::StatusCode;
    use http_body_util::BodyExt;

    fn html_response(body: &str) -> Response<Body> {
        Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CONTENT_LENGTH, body.len().to_string())
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    async fn body_string(resp: Response<Body>) -> String {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    #[test]
    fn inject_finds_head_close_lowercase() {
        let html = b"<html><head><title>x</title></head><body></body></html>";
        let out = inject_into_head(html, "/app/foo/");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("<base href=\"/app/foo/\"></head>"));
    }

    #[test]
    fn inject_is_case_insensitive() {
        let html = b"<HTML><HEAD></HEAD><BODY></BODY></HTML>";
        let out = inject_into_head(html, "/app/x/");
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("<base href=\"/app/x/\"></HEAD>"));
    }

    #[test]
    fn inject_leaves_html_alone_when_no_head_close() {
        let html = b"<html><body>oops no head close</body></html>";
        let out = inject_into_head(html, "/app/foo/");
        assert_eq!(out, html);
    }

    #[test]
    fn inject_inserts_before_first_head_close_only() {
        let html = b"<head></head>middle</head>more";
        let out = inject_into_head(html, "/p/");
        let s = std::str::from_utf8(&out).unwrap();
        // First </head> got the tag, second one is untouched
        let first = s.find("<base").unwrap();
        let second = s.rfind("</head>").unwrap();
        assert!(first < second);
        // Only ONE injection
        assert_eq!(s.matches("<base").count(), 1);
    }

    #[tokio::test]
    async fn inject_base_href_rewrites_html_response() {
        let body = "<html><head><title>x</title></head><body>hi</body></html>";
        let resp = html_response(body);
        let out = inject_base_href(resp, "/app/foo/").await;
        let content_length: usize = out
            .headers()
            .get(header::CONTENT_LENGTH)
            .unwrap()
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        let s = body_string(out).await;
        assert!(s.contains("<base href=\"/app/foo/\">"));
        assert_eq!(
            content_length,
            s.len(),
            "Content-Length must match the rewritten body"
        );
    }

    #[tokio::test]
    async fn inject_base_href_skips_non_html() {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from("{\"a\":1}"))
            .unwrap();
        let out = inject_base_href(resp, "/app/foo/").await;
        let s = body_string(out).await;
        assert_eq!(s, "{\"a\":1}");
    }

    #[tokio::test]
    async fn inject_base_href_preserves_content_length_for_head_responses() {
        // A HEAD response carries Content-Length describing the
        // body a GET would return, but the body itself is empty.
        // The rewriter must not stomp that header to 0.
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CONTENT_LENGTH, "1234")
            .body(Body::empty())
            .unwrap();
        let out = inject_base_href(resp, "/app/foo/").await;
        let cl = out
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        assert_eq!(cl.as_deref(), Some("1234"));
        let s = body_string(out).await;
        assert!(s.is_empty(), "HEAD body stays empty");
    }

    #[tokio::test]
    async fn inject_base_href_skips_when_content_type_missing() {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .body(Body::from("<html><head></head></html>"))
            .unwrap();
        let out = inject_base_href(resp, "/app/foo/").await;
        let s = body_string(out).await;
        assert!(!s.contains("<base"));
    }
}
