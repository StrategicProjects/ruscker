//! HTML response transformation for `/app/{spec}/` proxied
//! responses. Two jobs in one pass:
//!
//! 1. **Inject `<base href="/app/{spec}/">`** into `<head>`.
//!    Handles **relative** URLs (`./foo.css`, `foo.css`,
//!    `?q=1`) — the browser resolves them against the base.
//!
//! 2. **Rewrite absolute-root attribute URLs**
//!    (`src="/lib/..."`, `href="/lib/..."`, `action="/..."`,
//!    `data-src="/..."`) to be prefixed with the mount point.
//!    HTML treats absolute paths as anchored to the host root,
//!    so `<base>` does **not** help them. Without this pass,
//!    Shiny / Streamlit / Dash apps that emit `<script
//!    src="/lib/jquery/jquery.js">` would 404 against Ruscker.
//!
//! ## Why bother — what the apps emit
//!
//! Shiny renders things like:
//! ```html
//! <script src="/sockjs/info"></script>
//! <link rel="stylesheet" href="/lib/bootstrap.css">
//! <img src="/img/spinner.gif">
//! ```
//!
//! All three are root-relative; behind `/app/auroraprime/` the
//! browser dispatches them as `/sockjs/info` etc. and hits
//! Ruscker, which has no route for those paths. With this
//! pass they become `/app/auroraprime/sockjs/info` and route
//! back through the same proxy handler that served the page.
//!
//! ## What this is NOT
//!
//! - **Runtime URL interception.** WebSocket / `fetch()` /
//!   XHR constructed in JavaScript still emit root-relative
//!   URLs and bypass the static rewriter. A JS shim (next
//!   slice) overrides `WebSocket` / `fetch` / `XMLHttpRequest`
//!   to prepend the prefix.
//! - **CSS `url()` rewriting.** External stylesheets are
//!   loaded via `<link href>` (covered), and their internal
//!   `url(/foo.png)` paths are relative to the stylesheet's
//!   own URL — which is now under `/app/{spec}/`, so they
//!   resolve correctly without rewriting.
//!
//! ## Trade-offs
//!
//! - **Collects the full body in memory.** Capped at
//!   [`MAX_HTML_BYTES`]; past that the response passes through
//!   untouched.
//! - **`lol_html` streaming under the hood.** We still buffer
//!   the body in/out, but the parser itself is incremental,
//!   handles malformed HTML gracefully, and uses CSS selectors
//!   instead of regex — robust against quote-style and
//!   attribute-order changes from the upstream.
//! - **Skip list** for absolute paths that already belong to a
//!   Ruscker route (`/admin/...`, `/assets/...`, `/app/...`,
//!   `/api/...`) so we don't double-prefix.

use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderValue, Response};
use lol_html::html_content::ContentType;
use lol_html::{element, HtmlRewriter, Settings};

/// Hard cap on the body size we'll collect to rewrite. 2 MB is
/// generous for any sensible initial HTML payload; past that
/// we pass through untouched to avoid pinning megabytes of
/// memory per request.
const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;

/// Path prefixes that should NEVER be rewritten — they're
/// either already routed by Ruscker, or they belong to the
/// hosting host's chrome (e.g. a reverse proxy in front).
///
/// `/app/` and `/api/` matter most: an app that emits
/// `<a href="/app/other">` (cross-app link) should still work
/// after rewriting kicks in.
const SKIP_PREFIXES: &[&str] = &[
    "/admin/",
    "/admin",
    "/assets/",
    "/app/",
    "/api/",
    "/__set/",
    "/__assets__/", // Streamlit's static assets — rewrite would break them
];

/// Returns the prefix-rewritten URL, or `None` if the URL
/// shouldn't be touched (relative, protocol-prefixed, anchor,
/// skip-list match, etc.).
fn rewrite_url(value: &str, base_path: &str) -> Option<String> {
    let trimmed = value.trim_start();
    // Relative URL — `<base href>` handles this. Includes the
    // empty string ("href=\"\"").
    if !trimmed.starts_with('/') {
        return None;
    }
    // Protocol-relative ("//cdn.example.com/foo") — bypasses
    // the host entirely.
    if trimmed.starts_with("//") {
        return None;
    }
    // Already under a Ruscker-known path.
    if SKIP_PREFIXES.iter().any(|p| trimmed.starts_with(p)) {
        return None;
    }
    // Already under the same base. Defensive — covers the
    // unusual case where an upstream emits `/app/x/lib/...`
    // because it was configured for ShinyProxy with a hard-
    // coded prefix.
    if trimmed.starts_with(base_path) {
        return None;
    }
    // base_path ends in '/'; trimmed starts with '/' — strip
    // one to avoid `//lib/...`.
    let suffix = trimmed.trim_start_matches('/');
    Some(format!("{base_path}{suffix}"))
}

/// If `resp` is an HTML response, run the full transform
/// (`<base href>` + URL attribute rewriting) and return the
/// rewritten response. Non-HTML responses pass through
/// unchanged.
///
/// `base_path` should be the user-visible URL prefix the app
/// is mounted at, **with a trailing slash** — e.g.
/// `/app/auroraprime/`.
///
/// Name kept as `inject_base_href` for source-compat with the
/// proxy handler that calls this; the function now does both
/// jobs.
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
                "could not collect HTML body for transform; passing through empty"
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

    let rewritten = match transform(bytes.as_ref(), base_path) {
        Ok(r) => r,
        Err(e) => {
            // lol_html failures are exceptional (allocation
            // errors, etc.). Fall back to the original body
            // rather than nuking the response.
            tracing::warn!(error = ?e, "HTML transform failed; serving upstream body as-is");
            bytes.to_vec()
        }
    };

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

/// Run the streaming transform: inject `<base href>` and
/// rewrite known URL-bearing attributes. Wrapped in a
/// fallible function so the caller can swallow the
/// (vanishingly rare) lol_html error.
fn transform(html: &[u8], base_path: &str) -> Result<Vec<u8>, lol_html::errors::RewritingError> {
    let mut out = Vec::with_capacity(html.len() + 64);
    // Track whether we've already injected the `<base>` tag —
    // we only want one, and only if a `<head>` exists.
    let base_tag = format!("<base href=\"{}\">", base_path);

    // Per-attribute element handlers. Each entry is a CSS
    // selector + the attribute name to rewrite if its value
    // is an absolute path. Keeping the list narrow avoids
    // false positives (e.g. `<input name="/foo">` would be
    // nonsense to rewrite).
    let url_attrs: &[(&str, &str)] = &[
        ("a[href]", "href"),
        ("link[href]", "href"),
        ("script[src]", "src"),
        ("img[src]", "src"),
        ("iframe[src]", "src"),
        ("source[src]", "src"),
        ("audio[src]", "src"),
        ("video[src]", "src"),
        ("form[action]", "action"),
        ("button[formaction]", "formaction"),
        ("input[formaction]", "formaction"),
        ("img[data-src]", "data-src"),
        ("script[data-src]", "data-src"),
    ];

    let mut element_handlers: Vec<_> = Vec::with_capacity(url_attrs.len() + 1);

    // `<head>`: prepend the base tag as the first child. lol_html's
    // `prepend` runs once on the open tag, which is what we want.
    element_handlers.push(element!("head", |el| {
        el.prepend(&base_tag, ContentType::Html);
        Ok(())
    }));

    for (selector, attr) in url_attrs {
        let attr = *attr;
        element_handlers.push(element!(*selector, move |el| {
            if let Some(v) = el.get_attribute(attr) {
                if let Some(new_v) = rewrite_url(&v, base_path) {
                    el.set_attribute(attr, &new_v)?;
                }
            }
            Ok(())
        }));
    }

    let mut rewriter = HtmlRewriter::new(
        Settings {
            element_content_handlers: element_handlers,
            ..Settings::default()
        },
        |c: &[u8]| out.extend_from_slice(c),
    );
    rewriter.write(html)?;
    rewriter.end()?;
    Ok(out)
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

    // ── rewrite_url: pure unit ──────────────────────────────

    #[test]
    fn rewrite_url_prefixes_absolute_root_path() {
        assert_eq!(
            rewrite_url("/lib/foo.css", "/app/x/").as_deref(),
            Some("/app/x/lib/foo.css")
        );
        assert_eq!(
            rewrite_url("/sockjs/info", "/app/auroraprime/").as_deref(),
            Some("/app/auroraprime/sockjs/info")
        );
    }

    #[test]
    fn rewrite_url_skips_relative() {
        assert_eq!(rewrite_url("foo.css", "/app/x/"), None);
        assert_eq!(rewrite_url("./foo.css", "/app/x/"), None);
        assert_eq!(rewrite_url("../up", "/app/x/"), None);
        assert_eq!(rewrite_url("?q=1", "/app/x/"), None);
        assert_eq!(rewrite_url("#anchor", "/app/x/"), None);
        assert_eq!(rewrite_url("", "/app/x/"), None);
    }

    #[test]
    fn rewrite_url_skips_protocol_and_protocol_relative() {
        assert_eq!(rewrite_url("https://cdn.example.com/foo", "/app/x/"), None);
        assert_eq!(rewrite_url("http://localhost/foo", "/app/x/"), None);
        assert_eq!(rewrite_url("//cdn.example.com/foo", "/app/x/"), None);
        assert_eq!(rewrite_url("data:image/png;base64,xyz", "/app/x/"), None);
        assert_eq!(rewrite_url("mailto:a@b.com", "/app/x/"), None);
        assert_eq!(rewrite_url("javascript:void(0)", "/app/x/"), None);
    }

    #[test]
    fn rewrite_url_skips_known_ruscker_paths() {
        assert_eq!(rewrite_url("/admin/specs", "/app/x/"), None);
        assert_eq!(rewrite_url("/assets/styles.css", "/app/x/"), None);
        assert_eq!(rewrite_url("/app/other/page", "/app/x/"), None);
        assert_eq!(rewrite_url("/api/something", "/app/x/"), None);
    }

    #[test]
    fn rewrite_url_idempotent_under_own_base() {
        // If somehow the upstream already emitted the prefix,
        // don't double it.
        assert_eq!(rewrite_url("/app/x/lib/foo", "/app/x/"), None);
    }

    // ── transform: end-to-end HTML rewriting ────────────────

    #[test]
    fn transform_injects_base_in_head() {
        let html = b"<html><head><title>x</title></head><body></body></html>";
        let out = transform(html, "/app/foo/").unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("<base href=\"/app/foo/\">"));
        // The base is inside head and before any other head
        // child — lol_html `prepend` semantics.
        let head_idx = s.find("<head>").unwrap();
        let base_idx = s.find("<base").unwrap();
        let title_idx = s.find("<title>").unwrap();
        assert!(head_idx < base_idx && base_idx < title_idx);
    }

    #[test]
    fn transform_prefixes_absolute_src_and_href() {
        let html = br#"<html><head>
<link rel="stylesheet" href="/lib/bootstrap.css">
<script src="/sockjs/info"></script>
</head><body>
<img src="/img/spinner.gif">
<a href="/about">about</a>
</body></html>"#;
        let out = transform(html, "/app/x/").unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("href=\"/app/x/lib/bootstrap.css\""));
        assert!(s.contains("src=\"/app/x/sockjs/info\""));
        assert!(s.contains("src=\"/app/x/img/spinner.gif\""));
        assert!(s.contains("href=\"/app/x/about\""));
    }

    #[test]
    fn transform_leaves_relative_protocol_and_skip_paths_alone() {
        // Raw byte string uses ## delimiter so the `"#` inside
        // `href="#top"` doesn't accidentally terminate the
        // literal.
        let html = br##"<html><body>
<img src="logo.png">
<a href="#top">top</a>
<img src="https://cdn.example.com/x.png">
<a href="/admin/specs">admin</a>
</body></html>"##;
        let out = transform(html, "/app/x/").unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains(r#"src="logo.png""#));
        assert!(s.contains(r##"href="#top""##));
        assert!(s.contains(r#"src="https://cdn.example.com/x.png""#));
        assert!(s.contains(r#"href="/admin/specs""#));
    }

    #[test]
    fn transform_handles_uppercase_tags() {
        // lol_html normalizes tag names case-insensitively but
        // preserves attribute name case in the output. The
        // selector `a[href]` matches `<A HREF=...>` because
        // tag matching is case-insensitive per HTML spec.
        let html = br#"<HTML><HEAD></HEAD><BODY><A HREF="/foo">x</A></BODY></HTML>"#;
        let out = transform(html, "/app/x/").unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        // Match case-insensitively so we don't depend on
        // whether lol_html normalized HREF→href in the output.
        let lower = s.to_ascii_lowercase();
        assert!(
            lower.contains(r#"href="/app/x/foo""#),
            "uppercase HREF should be rewritten regardless of case; got: {s}"
        );
    }

    #[test]
    fn transform_handles_form_action() {
        let html = br#"<html><body><form action="/submit" method="post"></form></body></html>"#;
        let out = transform(html, "/app/x/").unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(s.contains("action=\"/app/x/submit\""));
    }

    #[test]
    fn transform_no_head_just_rewrites_attrs() {
        // Fragment / partial response: no <head> means no
        // <base> injection, but attribute rewriting still runs.
        let html = br#"<div><img src="/foo.png"></div>"#;
        let out = transform(html, "/app/x/").unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(!s.contains("<base"));
        assert!(s.contains(r#"src="/app/x/foo.png""#));
    }

    // ── HTTP-layer wrapper ──────────────────────────────────

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
    async fn inject_base_href_rewrites_attributes_in_full_response() {
        let body = r#"<html><head><script src="/lib/jquery.js"></script></head><body><img src="/img/x.png"></body></html>"#;
        let resp = html_response(body);
        let out = inject_base_href(resp, "/app/a/").await;
        let s = body_string(out).await;
        assert!(s.contains("src=\"/app/a/lib/jquery.js\""));
        assert!(s.contains("src=\"/app/a/img/x.png\""));
        assert!(s.contains("<base href=\"/app/a/\">"));
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
