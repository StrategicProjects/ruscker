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
//! All three are root-relative; behind `/app/sales-dashboard/` the
//! browser dispatches them as `/sockjs/info` etc. and hits
//! Ruscker, which has no route for those paths. With this
//! pass they become `/app/sales-dashboard/sockjs/info` and route
//! back through the same proxy handler that served the page.
//!
//! ## Runtime URL interception (JS shim)
//!
//! `WebSocket('/websocket/')`, `fetch('/api/x')`, and
//! `XMLHttpRequest.open(..., '/foo')` construct URLs at
//! runtime, after the HTML rewriter has finished. The same
//! prefix problem applies. We inject a small `<script>` at
//! the very top of `<head>` that monkey-patches the three
//! globals to prepend the mount prefix on absolute paths,
//! using the same skip rules as the HTML rewriter so behavior
//! is uniform.
//!
//! "Top of `<head>`" matters: any subsequent `<script>` —
//! including the page's own bundle — sees the patched globals,
//! so even modules captured into closures (e.g. Shiny's
//! `shiny.connections.js`) use the wrappers.
//!
//! ## What this is still NOT
//!
//! - **CSS `url()` rewriting.** External stylesheets are
//!   loaded via `<link href>` (covered), and their internal
//!   `url(/foo.png)` paths are relative to the stylesheet's
//!   own URL — which is now under `/app/{spec}/`, so they
//!   resolve correctly without rewriting.
//! - **Inline `<script>` source.** A string literal like
//!   `var url = '/foo'` inside a `<script>` block is opaque
//!   to us and will reach the patched fetch/WebSocket only
//!   when the script eventually calls those APIs.
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
//! - **Skip list** for absolute paths that already belong to
//!   Ruscker's own chrome (`/admin/...`, `/assets/...`,
//!   `/app/...`, …) so we don't double-prefix. Note `/api/` is
//!   *not* skipped: under `/app/{spec}/` it's the app's own
//!   namespace (Jupyter's REST + kernel WebSocket live there).
//!
//! ## Redirects
//!
//! A proxied app often answers `/app/{spec}/` with a `302` to a
//! root-absolute path (Jupyter → `/lab`, `/login?next=/lab`). We
//! prefix the `Location` header with the mount the same way, so
//! the redirect stays inside the app instead of escaping to a
//! Ruscker 404.

use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderValue, Response};
use lol_html::html_content::ContentType;
use lol_html::{element, HtmlRewriter, Settings};
use regex::Regex;
use std::sync::LazyLock;

/// Hard cap on the body size we'll collect to rewrite. 2 MB is
/// generous for any sensible initial HTML payload; past that
/// we pass through untouched to avoid pinning megabytes of
/// memory per request.
const MAX_HTML_BYTES: usize = 2 * 1024 * 1024;

/// Path prefixes that should NEVER be rewritten — they're
/// either already routed by Ruscker, or they belong to the
/// hosting host's chrome (e.g. a reverse proxy in front).
///
/// `/app/` is kept so a cross-app link (`<a href="/app/other">`)
/// still resolves after rewriting kicks in.
///
/// **`/api/` is deliberately NOT here.** A containerized app
/// mounted at `/app/{spec}/` is served with its prefix stripped,
/// so from the app's point of view it lives at the host root —
/// every root-absolute URL it emits, including `/api/...`, is the
/// *app's own*. Notebook servers (Jupyter, Voilà) put their whole
/// REST surface and the kernel WebSocket under `/api/...`; if we
/// skipped it those calls would land on Ruscker's `/api/{spec}`
/// router instead of the container and the app would never
/// connect. The rare cross-spec link to a real Ruscker API can use
/// an absolute URL.
const SKIP_PREFIXES: &[&str] = &[
    "/admin/",
    "/admin",
    "/assets/",
    "/app/",
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
/// `/app/sales-dashboard/`.
///
/// Name kept as `inject_base_href` for source-compat with the
/// proxy handler that calls this; the function now does both
/// jobs.
pub async fn inject_base_href(
    resp: Response<Body>,
    base_path: &str,
    upstream: &str,
) -> Response<Body> {
    let (mut parts, body) = resp.into_parts();

    // Redirects first — they carry no HTML body, so they'd otherwise
    // slip past the `is_html` gate below untouched. Prefix a root-
    // absolute `Location` with the mount (same rule as body URLs) so an
    // app's `302 → /lab` stays under `/app/{spec}/` instead of escaping
    // to a Ruscker 404.
    if let Some(loc) = parts.headers.get(header::LOCATION).and_then(|v| v.to_str().ok()) {
        if let Some(new_loc) = rewrite_url(loc, base_path) {
            if let Ok(v) = HeaderValue::from_str(&new_loc) {
                parts.headers.insert(header::LOCATION, v);
            }
        }
    }

    if !headers_are_html(&parts.headers) {
        return Response::from_parts(parts, body);
    }

    // Defense-in-depth (#732): the proxy asks the upstream for
    // `Accept-Encoding: identity` whenever this transform is enabled,
    // but a non-compliant server may compress anyway. A content-encoded
    // body is opaque bytes — running the rewriter over it would corrupt
    // the stream — so pass it through untouched instead.
    let content_encoded = parts
        .headers
        .get(header::CONTENT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|s| !s.trim().eq_ignore_ascii_case("identity"));
    if content_encoded {
        return Response::from_parts(parts, body);
    }

    // Decide *before* consuming the body. A declared Content-Length over
    // the transform cap streams the original through untouched — we
    // can't buffer-then-fail, because `to_bytes` would have already
    // consumed bytes we can no longer hand back. (This is the case the
    // module doc promises: "past that we pass through untouched.")
    let declared_len = parts
        .headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok());
    if declared_len.is_some_and(|n| n > MAX_HTML_BYTES) {
        return Response::from_parts(parts, body);
    }

    let bytes = match to_bytes(body, MAX_HTML_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            // Unknown-length (chunked) body that overran the cap mid-
            // stream: it's already partially consumed, so we can neither
            // transform nor hand it back. Fail loudly with a 502 rather
            // than serve a broken empty 200 with a stale Content-Length.
            tracing::warn!(
                error = ?e,
                "HTML body exceeded the transform cap and can't be streamed back"
            );
            return Response::builder()
                .status(502)
                .body(Body::from("upstream HTML too large to transform"))
                .unwrap_or_else(|_| Response::new(Body::empty()));
        }
    };

    // No body to rewrite — most often a HEAD response. Keep
    // parts intact so the upstream's Content-Length (which
    // reflects what a GET *would* return) survives.
    if bytes.is_empty() {
        return Response::from_parts(parts, Body::empty());
    }

    // #373: rewrite the container's OWN internal authority
    // (`http://127.0.0.1:<published-port>/…`) to the mount before the
    // lol_html pass. Apps like the FastAPI demo build absolute URLs from
    // the Host they receive (the internal upstream), so their assets
    // point the browser at an unreachable `127.0.0.1:<port>`. No-op when
    // the body doesn't mention the upstream or it isn't valid UTF-8.
    let authority_rewritten = std::str::from_utf8(bytes.as_ref())
        .ok()
        .and_then(|s| rewrite_upstream_authority(s, upstream, base_path));
    let input: &[u8] = authority_rewritten.as_deref().map_or(bytes.as_ref(), str::as_bytes);

    let rewritten = match transform(input, base_path) {
        Ok(r) => r,
        Err(e) => {
            // lol_html failures are exceptional (allocation
            // errors, etc.). Fall back to the original body
            // rather than nuking the response.
            tracing::warn!(error = ?e, "HTML transform failed; serving upstream body as-is");
            input.to_vec()
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

fn headers_are_html(headers: &header::HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| {
            let lower = s.trim().to_ascii_lowercase();
            lower.starts_with("text/html")
        })
        .unwrap_or(false)
}

/// Build the runtime-shim `<script>` that monkey-patches
/// `WebSocket`, `fetch`, and `XMLHttpRequest.open` to prefix
/// absolute-path URLs at call time. Mirrors the same skip
/// rules as [`rewrite_url`] but in JavaScript.
///
/// `base_path` is embedded as a string literal — kept in sync
/// with the `<base href>` injection so the static and dynamic
/// passes agree on the prefix.
///
/// IIFE-wrapped so it doesn't leak helpers onto `window`.
/// `try { ... } catch (_) { }` around each wrap so a single
/// browser oddity (older Edge, no `WebSocket`, etc.) can't
/// take the whole page down.
fn build_runtime_shim(base_path: &str) -> String {
    // The JS-level skip prefixes mirror SKIP_PREFIXES on the
    // Rust side. Kept inline (not loaded from a constant)
    // because they're a different scope and we want each
    // injection to be self-contained.
    format!(
        r##"<script>(function(){{
  var base = {base:?};
  function prefix(u) {{
    if (typeof u !== "string") return u;
    if (u.length === 0) return u;
    if (u.charAt(0) !== "/") return u;          // relative
    if (u.charAt(1) === "/") return u;           // protocol-relative
    var skips = ["/admin/", "/admin", "/assets/", "/app/", "/__set/", "/__assets__/"];
    for (var i = 0; i < skips.length; i++) {{
      if (u.indexOf(skips[i]) === 0) return u;
    }}
    if (u.indexOf(base) === 0) return u;
    return base + u.replace(/^\/+/, "");
  }}
  function prefixWsUrl(u) {{
    if (typeof u !== "string") return u;
    var m = u.match(/^(wss?:)\/\/([^\/]+)(\/.*)?$/);
    if (!m) return u;
    var path = m[3] || "/";
    var newPath = prefix(path);
    return m[1] + "//" + m[2] + newPath;
  }}
  // WebSocket
  try {{
    var OrigWS = window.WebSocket;
    if (OrigWS) {{
      function PatchedWS(url, protocols) {{
        var u = prefixWsUrl(url);
        return arguments.length > 1 ? new OrigWS(u, protocols) : new OrigWS(u);
      }}
      PatchedWS.prototype = OrigWS.prototype;
      PatchedWS.CONNECTING = OrigWS.CONNECTING;
      PatchedWS.OPEN = OrigWS.OPEN;
      PatchedWS.CLOSING = OrigWS.CLOSING;
      PatchedWS.CLOSED = OrigWS.CLOSED;
      window.WebSocket = PatchedWS;
    }}
  }} catch (_) {{}}
  // fetch
  try {{
    var origFetch = window.fetch;
    if (origFetch) {{
      window.fetch = function (input, init) {{
        if (typeof input === "string") input = prefix(input);
        else if (input && typeof input.url === "string") {{
          var p = prefix(input.url);
          if (p !== input.url) input = new Request(p, input);
        }}
        return origFetch.call(this, input, init);
      }};
    }}
  }} catch (_) {{}}
  // XMLHttpRequest
  try {{
    var origOpen = XMLHttpRequest.prototype.open;
    XMLHttpRequest.prototype.open = function (method, url) {{
      var rest = Array.prototype.slice.call(arguments, 2);
      return origOpen.apply(this, [method, prefix(url)].concat(rest));
    }};
  }} catch (_) {{}}
  // Dynamic resource URLs assigned to an element's property at RUNTIME —
  // RequireJS/webpack (`script.src`), CSS chunks (`link.href`), and images
  // (`img.src`, e.g. Jupyter kernel-spec logos served root-absolute from
  // the API). These are the browser's own resource fetches, a path the
  // fetch/XHR/WS wrappers above never see. Patch the prototype accessors
  // so the same `prefix()` applies generically — no per-framework HTML
  // rewrite needed. This is the Class-3 fix that makes Voilà's RequireJS
  // bootstrap, Jupyter's webpack chunks, and runtime-set images load under
  // the mount without bespoke `rewrite_*` passes.
  function patchUrlProp(proto, name) {{
    try {{
      var d = Object.getOwnPropertyDescriptor(proto, name);
      if (d && typeof d.set === "function" && typeof d.get === "function" && d.configurable) {{
        Object.defineProperty(proto, name, {{
          configurable: true,
          enumerable: d.enumerable,
          get: function () {{ return d.get.call(this); }},
          set: function (v) {{ d.set.call(this, prefix(v)); }}
        }});
      }}
    }} catch (_) {{}}
  }}
  try {{
    patchUrlProp(HTMLScriptElement.prototype, "src");
    patchUrlProp(HTMLLinkElement.prototype, "href");
    patchUrlProp(HTMLImageElement.prototype, "src");
    patchUrlProp(HTMLIFrameElement.prototype, "src");
    patchUrlProp(HTMLMediaElement.prototype, "src");
    patchUrlProp(HTMLSourceElement.prototype, "src");
  }} catch (_) {{}}
  // Some loaders set the URL via setAttribute instead of the property.
  // Catch that path too, scoped to the same resource-loading elements.
  try {{
    var SRC_TAGS = {{ SCRIPT: 1, IMG: 1, IFRAME: 1, AUDIO: 1, VIDEO: 1, SOURCE: 1 }};
    var origSetAttr = Element.prototype.setAttribute;
    Element.prototype.setAttribute = function (name, value) {{
      var n = (this.tagName || "").toUpperCase();
      if ((name === "src" && SRC_TAGS[n]) || (name === "href" && n === "LINK")) {{
        value = prefix(value);
      }}
      return origSetAttr.call(this, name, value);
    }};
  }} catch (_) {{}}
}})();</script>"##,
        base = base_path
    )
}

/// Matches the `<script id="jupyter-config-data" …>…</script>` block
/// JupyterLab emits in its page bootstrap. Attribute order varies
/// (`type` may precede `id`), so we match any `<script>` whose
/// attributes include `id="jupyter-config-data"` and capture the inner
/// JSON. Case-insensitive + dot-matches-newline so the JSON body (which
/// spans many lines) is captured whole.
static JUPYTER_CONFIG_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"(?is)(<script\b[^>]*\bid=["']jupyter-config-data["'][^>]*>)(.*?)(</script>)"#)
        .expect("jupyter-config-data regex is valid")
});

/// Rewrite the URL fields inside JupyterLab's `jupyter-config-data`
/// JSON so the app works behind the `/app/{spec}/` mount (#348).
///
/// ## Why this is needed on top of the `<base href>` + shim
///
/// JupyterLab is served with `--ServerApp.base_url=/` (so it doesn't
/// need to know its proxied path), which means its page config reports
/// `baseUrl: "/"` and `fullStaticUrl: "/static/lab"`. The Lab bootstrap
/// sets webpack's `__webpack_public_path__` from those, then loads its
/// lazy chunks by **injecting `<script src=…>` elements** at runtime —
/// a path the fetch/XHR/WebSocket shim never sees. So the chunks are
/// requested at the host root (`/static/lab/<chunk>.js`), miss the
/// mount, and 404 → blank Lab.
///
/// `<base href>` can't fix this either: the public path is an absolute
/// string, not a relative URL the browser resolves against the base.
///
/// The fix: prefix every root-absolute URL field in the config with the
/// mount (same rule as [`rewrite_url`]), so `baseUrl` →
/// `/box/app/jupyter/` and `fullStaticUrl` → `/box/app/jupyter/static/lab`.
/// Webpack then loads chunks from under the mount, the proxy strips the
/// prefix, and the container (still at `base_url=/`) serves them.
///
/// We rewrite **any** key whose name ends in `url` (case-insensitive) —
/// `baseUrl`, `wsUrl`, `fullStaticUrl`, `fullLabextensionsUrl`,
/// `fullThemesUrl`, … — so the fix is robust across JupyterLab versions
/// regardless of which field the bootstrap reads. Non-`/` values (empty
/// `wsUrl`, CDN `http(s)://…` URLs) are left untouched by `rewrite_url`.
///
/// Returns `Some(new_html)` only when something changed; `None` when the
/// page has no Jupyter config block (every non-Jupyter app) or the JSON
/// is unparseable — caller then uses the original bytes.
fn rewrite_jupyter_config(html: &str, base_path: &str) -> Option<String> {
    let caps = JUPYTER_CONFIG_RE.captures(html)?;
    let full = caps.get(0)?;
    let open_tag = caps.get(1)?.as_str();
    let json_text = caps.get(2)?.as_str();
    let close_tag = caps.get(3)?.as_str();

    let mut value: serde_json::Value = serde_json::from_str(json_text.trim()).ok()?;
    let obj = value.as_object_mut()?;
    let mut changed = false;
    for (key, val) in obj.iter_mut() {
        // Rewrite ONLY `baseUrl` and the absolute `full*Url` fields.
        // JupyterLab's config carries pairs: a short relative key
        // (`appUrl`, `workspacesApiUrl`, `settingsUrl`, … = `/lab/…`)
        // that the Lab JS **joins with baseUrl**, and the pre-joined
        // absolute `full*Url`. Rewriting the short keys to absolute made
        // `URLExt.join(baseUrl, workspacesApiUrl)` DOUBLE the prefix
        // (`/box/app/jupyter/box/app/jupyter/lab/api/workspaces` → 404,
        // #371). So leave them relative; only `baseUrl` (the join root)
        // and the `full*Url` (used as-is) need the mount prefix.
        let k = key.to_ascii_lowercase();
        let rewrite_this = k == "baseurl" || (k.starts_with("full") && k.ends_with("url"));
        if !rewrite_this {
            continue;
        }
        if let Some(s) = val.as_str() {
            if let Some(new) = rewrite_url(s, base_path) {
                *val = serde_json::Value::String(new);
                changed = true;
            }
        }
    }
    if !changed {
        return None;
    }

    let new_json = serde_json::to_string(&value).ok()?;
    let mut out = String::with_capacity(html.len() + new_json.len());
    out.push_str(&html[..full.start()]);
    out.push_str(open_tag);
    out.push_str(&new_json);
    out.push_str(close_tag);
    out.push_str(&html[full.end()..]);
    Some(out)
}

/// Rewrite the container's own internal authority to the mount path
/// (#373). An app behind the proxy that builds absolute URLs from the
/// `Host` it sees (e.g. Starlette/FastAPI `url_for`) emits
/// `http://127.0.0.1:<port>/static/x` — the published *internal* address,
/// unreachable from the browser. Replace `http(s)://{upstream}` with the
/// public mount prefix so `…/static/x` resolves under `/box/app/{spec}/`.
/// `Some` only when something changed; `None` for an empty `upstream` or
/// a body that doesn't mention it.
fn rewrite_upstream_authority(html: &str, upstream: &str, base_path: &str) -> Option<String> {
    if upstream.is_empty() {
        return None;
    }
    let mount = base_path.trim_end_matches('/'); // e.g. /box/app/fastapi
    let http = format!("http://{upstream}");
    let https = format!("https://{upstream}");
    if !html.contains(&http) && !html.contains(&https) {
        return None;
    }
    Some(html.replace(&http, mount).replace(&https, mount))
}

/// Run the streaming transform: inject `<base href>`, the
/// runtime JS shim, and rewrite known URL-bearing attributes.
/// Wrapped in a fallible function so the caller can swallow
/// the (vanishingly rare) lol_html error.
fn transform(html: &[u8], base_path: &str) -> Result<Vec<u8>, lol_html::errors::RewritingError> {
    // JupyterLab special case (#348): rewrite the `jupyter-config-data`
    // URL fields before the lol_html pass so the Lab webpack bootstrap
    // loads its lazy chunks from under the mount. No-op (and cheap — one
    // regex scan) for every other app and for non-UTF-8 bodies.
    let jup = std::str::from_utf8(html)
        .ok()
        .and_then(|s| rewrite_jupyter_config(s, base_path));
    let html: &[u8] = jup.as_deref().map_or(html, str::as_bytes);
    // NOTE: there is no Voilà-specific pass. Voilà's RequireJS bootstrap
    // builds its static URLs (`/voila/…`) at runtime and assigns them to
    // `script.src` — the generalized runtime shim below (which patches the
    // `HTMLScriptElement.prototype.src` setter) prefixes them to the mount
    // generically, so no bespoke string-rewrite is needed (#372,
    // validated against the real container in a headless browser).

    let mut out = Vec::with_capacity(html.len() + 256);
    // Track whether we've already injected the `<base>` tag —
    // we only want one, and only if a `<head>` exists.
    let base_tag = format!("<base href=\"{}\">", base_path);
    let runtime_shim = build_runtime_shim(base_path);

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

    // `<head>`: prepend `<base href>` AND the runtime shim as
    // the first children. Order matters: we prepend in
    // sequence so the final document order is
    // `<head><base><script>(shim)</script>...everything else`.
    // The shim must come before any page script so that when
    // the page's own JS opens a WebSocket / calls fetch, our
    // wrapper is already in place.
    //
    // Both prepends run on the same `<head>` open-tag event;
    // each call inserts at the start, so the *last* prepend
    // becomes the first child. We prepend the shim LAST so
    // it lands BEFORE the base tag — both are equally
    // unobservable to the browser, but the shim wraps
    // globals that any other inline script could touch, and
    // running first removes ambiguity.
    element_handlers.push(element!("head", move |el| {
        el.prepend(&base_tag, ContentType::Html);
        el.prepend(&runtime_shim, ContentType::Html);
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

// ════════════════════════════════════════════════════════════════
// Base-path mounting (#173)
//
// The opposite job to the `/app/{spec}/` rewrite above: when Ruscker
// itself is served under a subpath (e.g. `/box`), the *chrome*
// (landing + admin) emits root-absolute URLs (`/admin/x`, `/assets/x`,
// `/__set/...`) and redirects (`Location: /admin/dashboard`) that must
// all be prefixed with the base. We reuse the same lol_html plumbing,
// but the rewrite rule is inverted: prefix *every* root-absolute URL
// that isn't already under the base (no skip-list — those Ruscker paths
// are exactly what we want to move under the prefix).
// ════════════════════════════════════════════════════════════════

/// Prefix a root-absolute URL with `base` (e.g. `/box`), unless it's
/// relative, protocol-relative, or already under `base`. `base` has a
/// leading slash and no trailing slash.
fn rewrite_url_base(value: &str, base: &str) -> Option<String> {
    let t = value.trim_start();
    if !t.starts_with('/') || t.starts_with("//") {
        return None; // relative, anchor (#/?), protocol-relative, scheme
    }
    if t == base || t.starts_with(&format!("{base}/")) {
        return None; // already prefixed
    }
    Some(format!("{base}{t}"))
}

/// Prefix a root-absolute `Location` header (redirects) with `base` so a
/// handler's `Redirect::to("/admin/x")` stays under the mount as
/// `/box/admin/x`. No-op when `base` is empty.
///
/// The chrome's HTML **body** no longer needs rewriting: every template
/// now emits its URLs already prefixed via `{{ base }}` and sets
/// `window.RUSCKER_BASE` for the page JS that builds URLs at runtime
/// (#294). So this dropped the per-request body buffering + `lol_html`
/// parse that used to run on every admin page; only the redirect
/// `Location` header — which handlers emit raw — still needs fixing here.
/// Cookies set `Path=/`, which the browser already sends for `{base}/…`.
pub async fn prefix_base_path(resp: Response<Body>, base: &str) -> Response<Body> {
    if base.is_empty() {
        return resp;
    }
    let (mut parts, body) = resp.into_parts();

    // Redirects: `Location: /admin/x` → `/box/admin/x`.
    if let Some(loc) = parts.headers.get(header::LOCATION).and_then(|v| v.to_str().ok()) {
        if let Some(new_loc) = rewrite_url_base(loc, base) {
            if let Ok(v) = HeaderValue::from_str(&new_loc) {
                parts.headers.insert(header::LOCATION, v);
            }
        }
    }

    Response::from_parts(parts, body)
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
            rewrite_url("/sockjs/info", "/app/sales-dashboard/").as_deref(),
            Some("/app/sales-dashboard/sockjs/info")
        );
    }

    // ── rewrite_jupyter_config: the #348 fix ────────────────

    #[test]
    fn jupyter_config_urls_prefixed_to_mount() {
        // Includes a short relative key (`workspacesApiUrl`) alongside its
        // absolute pair (`fullWorkspacesApiUrl`): only baseUrl + full*Url
        // get the mount; the short key STAYS relative or the Lab JS would
        // double-prefix `URLExt.join(baseUrl, workspacesApiUrl)` (#371).
        let html = r#"<html><head><script id="jupyter-config-data" type="application/json">{"baseUrl": "/", "wsUrl": "", "fullStaticUrl": "/static/lab", "workspacesApiUrl": "/lab/api/workspaces", "fullWorkspacesApiUrl": "/lab/api/workspaces", "appName": "JupyterLab"}</script></head></html>"#;
        let out = rewrite_jupyter_config(html, "/box/app/jupyter/").expect("should rewrite");
        // baseUrl "/" → the mount itself (kept with trailing slash).
        assert!(out.contains(r#""baseUrl":"/box/app/jupyter/""#), "baseUrl in: {out}");
        // Absolute `full*Url` keys → under the mount (used as-is by the Lab).
        assert!(
            out.contains(r#""fullStaticUrl":"/box/app/jupyter/static/lab""#),
            "fullStaticUrl in: {out}"
        );
        assert!(
            out.contains(r#""fullWorkspacesApiUrl":"/box/app/jupyter/lab/api/workspaces""#),
            "fullWorkspacesApiUrl in: {out}"
        );
        // The SHORT relative key is left untouched (joined with baseUrl by
        // the Lab) — rewriting it would double the prefix (#371).
        assert!(
            out.contains(r#""workspacesApiUrl":"/lab/api/workspaces""#),
            "workspacesApiUrl must stay relative: {out}"
        );
        // Empty wsUrl and the non-URL appName are left as-is.
        assert!(out.contains(r#""wsUrl":"""#), "wsUrl in: {out}");
        assert!(out.contains(r#""appName":"JupyterLab""#), "appName in: {out}");
    }

    #[test]
    fn jupyter_config_handles_attr_order_and_is_idempotent() {
        // `type` before `id`, single-quoted id — attribute order varies.
        let html = r#"<script type="application/json" id='jupyter-config-data'>{"baseUrl":"/"}</script>"#;
        let out = rewrite_jupyter_config(html, "/app/jupyter/").expect("should rewrite");
        assert!(out.contains(r#""baseUrl":"/app/jupyter/""#), "out: {out}");
        // Re-running on the already-prefixed config changes nothing.
        assert_eq!(rewrite_jupyter_config(&out, "/app/jupyter/"), None);
    }

    #[test]
    fn jupyter_config_absent_is_noop() {
        // A normal (non-Jupyter) app page has no config block.
        let html = r#"<html><head><script src="/lib/x.js"></script></head></html>"#;
        assert_eq!(rewrite_jupyter_config(html, "/box/app/shiny/"), None);
    }

    #[tokio::test]
    async fn inject_base_href_rewrites_jupyter_config_end_to_end() {
        // Through the full HTML transform: the config URLs are prefixed
        // AND the `<base href>` + shim are injected in one pass.
        let html = r#"<html><head><script id="jupyter-config-data" type="application/json">{"baseUrl": "/", "fullStaticUrl": "/static/lab"}</script></head><body></body></html>"#;
        let out = body_string(inject_base_href(html_response(html), "/box/app/jupyter/", "").await).await;
        assert!(out.contains(r#""baseUrl":"/box/app/jupyter/""#), "config: {out}");
        assert!(out.contains(r#""fullStaticUrl":"/box/app/jupyter/static/lab""#));
        assert!(out.contains(r#"<base href="/box/app/jupyter/">"#), "base href: {out}");
    }

    #[test]
    fn rewrite_upstream_authority_maps_internal_host_to_mount() {
        // #373: an app that leaks its internal host:port in absolute URLs
        // (FastAPI `url_for`) gets them rewritten to the public mount.
        let html = r#"<link href="http://127.0.0.1:32800/static/logo.png"><script src="https://127.0.0.1:32800/static/index.js"></script>"#;
        let out = rewrite_upstream_authority(html, "127.0.0.1:32800", "/box/app/fastapi/").unwrap();
        assert!(out.contains(r#"href="/box/app/fastapi/static/logo.png""#), "{out}");
        assert!(out.contains(r#"src="/box/app/fastapi/static/index.js""#), "{out}");
        assert!(!out.contains("127.0.0.1:32800"), "internal authority gone: {out}");
        // No-op: empty upstream, or a body that doesn't mention it.
        assert_eq!(rewrite_upstream_authority(html, "", "/box/app/fastapi/"), None);
        assert_eq!(rewrite_upstream_authority("<p>nope</p>", "127.0.0.1:32800", "/x/"), None);
    }

    #[tokio::test]
    async fn inject_base_href_rewrites_leaked_upstream_authority() {
        // Full pass: the leaked `http://{upstream}/static/x` becomes
        // `/box/app/fastapi/static/x` (and isn't double-prefixed by the
        // attribute rewriter, since it now starts with the base).
        let html = r#"<html><head></head><body><img src="http://127.0.0.1:32800/static/logo.png"></body></html>"#;
        let out = body_string(
            inject_base_href(html_response(html), "/box/app/fastapi/", "127.0.0.1:32800").await,
        )
        .await;
        assert!(
            out.contains(r#"src="/box/app/fastapi/static/logo.png""#),
            "leaked authority rewritten: {out}"
        );
        assert!(!out.contains("127.0.0.1:32800"), "no internal authority remains: {out}");
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
    }

    #[test]
    fn rewrite_url_prefixes_api_path() {
        // `/api/...` is the app's own namespace under the mount (Jupyter's
        // REST + kernel WS), so it must be prefixed, not skipped.
        assert_eq!(
            rewrite_url("/api/contents", "/app/jupyter/").as_deref(),
            Some("/app/jupyter/api/contents")
        );
        assert_eq!(
            rewrite_url("/api/kernels/abc/channels", "/app/jupyter/").as_deref(),
            Some("/app/jupyter/api/kernels/abc/channels")
        );
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

    // ── runtime shim ────────────────────────────────────────

    #[test]
    fn build_runtime_shim_embeds_base_path_as_string() {
        let shim = build_runtime_shim("/app/alpha/");
        assert!(shim.starts_with("<script>"));
        assert!(shim.ends_with("</script>"));
        // base must be a quoted JS string so `{base:?}` does
        // the right thing.
        assert!(
            shim.contains(r#"var base = "/app/alpha/";"#),
            "base path not quoted as a JS string literal: {shim}"
        );
    }

    #[test]
    fn build_runtime_shim_patches_the_runtime_url_surface() {
        let shim = build_runtime_shim("/app/x/");
        // Network APIs the page calls directly.
        assert!(shim.contains("window.WebSocket"));
        assert!(shim.contains("window.fetch"));
        assert!(shim.contains("XMLHttpRequest.prototype.open"));
        // Dynamic resource loading — the Class-3 fix (#372) that lets the
        // generalized shim subsume per-framework rewrites: RequireJS /
        // webpack assign root-absolute URLs to script.src / link.href, and
        // runtime-set images (img.src, e.g. Jupyter kernel-spec logos).
        assert!(shim.contains("HTMLScriptElement.prototype"));
        assert!(shim.contains("HTMLLinkElement.prototype"));
        assert!(shim.contains("HTMLImageElement.prototype"));
        assert!(shim.contains("Element.prototype.setAttribute"));
    }

    #[test]
    fn build_runtime_shim_wraps_each_patch_in_try_catch() {
        // A single browser quirk shouldn't break the others — every patch
        // (WS, fetch, XHR, the patchUrlProp helper, the prop-patch block,
        // and setAttribute) is independently guarded.
        let shim = build_runtime_shim("/app/x/");
        assert_eq!(
            shim.matches("} catch (_) {}").count(),
            6,
            "expected one try/catch per patched global / block"
        );
    }

    #[test]
    fn transform_injects_runtime_shim_before_other_head_scripts() {
        let html = br#"<html><head><script src="/lib/app.js"></script></head><body></body></html>"#;
        let out = transform(html, "/app/x/").unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        let shim_idx = s.find("(function(){").expect("shim present");
        let app_idx = s.find("/app/x/lib/app.js").expect("app script present");
        assert!(
            shim_idx < app_idx,
            "shim must precede any page script so its wrappers are in place"
        );
    }

    #[test]
    fn transform_no_head_skips_shim_injection() {
        // Fragment without <head> means no shim. The
        // attribute rewriting still runs.
        let html = br#"<div><img src="/foo.png"></div>"#;
        let out = transform(html, "/app/x/").unwrap();
        let s = std::str::from_utf8(&out).unwrap();
        assert!(!s.contains("XMLHttpRequest.prototype.open"));
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
        let out = inject_base_href(resp, "/app/foo/", "").await;
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
        let out = inject_base_href(resp, "/app/a/", "").await;
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
        let out = inject_base_href(resp, "/app/foo/", "").await;
        let s = body_string(out).await;
        assert_eq!(s, "{\"a\":1}");
    }

    // #732: a content-encoded HTML body is opaque bytes — the rewriter
    // must pass it through bit-identical (no <base href>, no shim)
    // instead of corrupting the compressed stream.
    #[tokio::test]
    async fn inject_base_href_passes_compressed_html_through_untouched() {
        // Not real gzip — just bytes that must come back identical.
        let payload: &[u8] = b"\x1f\x8b\x08\x00<html><head></head></html>";
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CONTENT_ENCODING, "gzip")
            .body(Body::from(payload))
            .unwrap();
        let out = inject_base_href(resp, "/app/foo/", "").await;
        assert_eq!(
            out.headers().get(header::CONTENT_ENCODING).unwrap(),
            "gzip"
        );
        let bytes = out.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(bytes.as_ref(), payload, "compressed body must not be touched");
    }

    // `identity` means "not encoded" — the transform must still run.
    #[tokio::test]
    async fn inject_base_href_treats_identity_encoding_as_plain() {
        let mut resp = html_response("<html><head></head><body></body></html>");
        resp.headers_mut()
            .insert(header::CONTENT_ENCODING, "identity".parse().unwrap());
        let out = body_string(inject_base_href(resp, "/app/foo/", "").await).await;
        assert!(out.contains("<base href=\"/app/foo/\">"), "got: {out}");
    }

    #[tokio::test]
    async fn inject_base_href_preserves_content_length_for_head_responses() {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
            .header(header::CONTENT_LENGTH, "1234")
            .body(Body::empty())
            .unwrap();
        let out = inject_base_href(resp, "/app/foo/", "").await;
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
    async fn inject_base_href_prefixes_redirect_location() {
        // Jupyter's first hit: `302 → /lab`. The Location must land
        // under the mount, not escape to a Ruscker 404.
        let resp = Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, "/lab")
            .body(Body::empty())
            .unwrap();
        let out = inject_base_href(resp, "/app/jupyter/", "").await;
        assert_eq!(
            out.headers().get(header::LOCATION).unwrap().to_str().unwrap(),
            "/app/jupyter/lab"
        );
    }

    #[tokio::test]
    async fn inject_base_href_prefixes_api_location_with_query() {
        // A login bounce keeps its `next=` query verbatim; only the
        // leading path is prefixed. And `/api/...` redirects are
        // prefixed now that `/api/` isn't skipped.
        let resp = Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, "/login?next=/lab")
            .body(Body::empty())
            .unwrap();
        let out = inject_base_href(resp, "/app/jupyter/", "").await;
        assert_eq!(
            out.headers().get(header::LOCATION).unwrap().to_str().unwrap(),
            "/app/jupyter/login?next=/lab"
        );
    }

    #[tokio::test]
    async fn inject_base_href_leaves_external_location_alone() {
        let resp = Response::builder()
            .status(StatusCode::FOUND)
            .header(header::LOCATION, "https://auth.example.com/sso")
            .body(Body::empty())
            .unwrap();
        let out = inject_base_href(resp, "/app/jupyter/", "").await;
        assert_eq!(
            out.headers().get(header::LOCATION).unwrap().to_str().unwrap(),
            "https://auth.example.com/sso"
        );
    }

    #[tokio::test]
    async fn inject_base_href_skips_when_content_type_missing() {
        let resp = Response::builder()
            .status(StatusCode::OK)
            .body(Body::from("<html><head></head></html>"))
            .unwrap();
        let out = inject_base_href(resp, "/app/foo/", "").await;
        let s = body_string(out).await;
        assert!(!s.contains("<base"));
    }

    // Regression for #75: an HTML page whose declared Content-Length is
    // over the cap is streamed through untouched (not emptied, not
    // transformed) instead of served as a broken empty body.
    #[tokio::test]
    async fn oversize_content_length_passes_through_untouched() {
        let body = "<html><head></head><body>big</body></html>";
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html")
            .header(header::CONTENT_LENGTH, (MAX_HTML_BYTES + 1).to_string())
            .body(Body::from(body))
            .unwrap();
        let out = inject_base_href(resp, "/app/foo/", "").await;
        assert_eq!(out.status(), StatusCode::OK);
        let s = body_string(out).await;
        assert_eq!(s, body, "body must pass through verbatim");
        assert!(!s.contains("<base"), "no transform on oversize body");
    }

    // A chunked (no Content-Length) body that overruns the cap mid-stream
    // can't be recovered → fail loudly with 502, never a silent empty 200.
    #[tokio::test]
    async fn oversize_chunked_body_returns_502() {
        let big = "x".repeat(MAX_HTML_BYTES + 1);
        let resp = Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/html")
            .body(Body::from(big))
            .unwrap();
        let out = inject_base_href(resp, "/app/foo/", "").await;
        assert_eq!(out.status(), StatusCode::BAD_GATEWAY);
    }

    // ── base-path (#173) ────────────────────────────────────

    #[test]
    fn rewrite_url_base_prefixes_and_skips() {
        assert_eq!(rewrite_url_base("/admin/x", "/box").as_deref(), Some("/box/admin/x"));
        assert_eq!(rewrite_url_base("/assets/a.css", "/box").as_deref(), Some("/box/assets/a.css"));
        assert_eq!(rewrite_url_base("/", "/box").as_deref(), Some("/box/"));
        // already under base, relative, anchor, protocol, external ⇒ skip
        assert_eq!(rewrite_url_base("/box/admin/x", "/box"), None);
        assert_eq!(rewrite_url_base("/box", "/box"), None);
        assert_eq!(rewrite_url_base("admin/x", "/box"), None);
        assert_eq!(rewrite_url_base("#top", "/box"), None);
        assert_eq!(rewrite_url_base("//cdn/x", "/box"), None);
        assert_eq!(rewrite_url_base("https://x/y", "/box"), None);
    }

    // The chrome HTML body is NOT rewritten anymore (#294): templates emit
    // `{{ base }}`-prefixed URLs directly. `prefix_base_path` now only
    // touches the `Location` header — covered by the redirect test below
    // and the body-untouched assertion in the base-path integration test.
    #[tokio::test]
    async fn prefix_base_path_leaves_html_body_untouched() {
        let resp = html_response(
            r#"<html><head></head><body><a href="/box/admin/specs">x</a></body></html>"#,
        );
        let out = prefix_base_path(resp, "/box").await;
        let s = body_string(out).await;
        // Body passes through verbatim — no double-prefix, no shim injected.
        assert!(s.contains(r#"href="/box/admin/specs""#), "body unchanged");
        assert!(!s.contains("/box/box/"), "no double-prefix");
        assert!(!s.contains("RUSCKER_BASE = "), "no injected shim");
    }

    #[tokio::test]
    async fn prefix_base_path_rewrites_redirect_location() {
        let resp = Response::builder()
            .status(StatusCode::SEE_OTHER)
            .header(header::LOCATION, "/admin/dashboard")
            .body(Body::empty())
            .unwrap();
        let out = prefix_base_path(resp, "/box").await;
        assert_eq!(
            out.headers().get(header::LOCATION).unwrap().to_str().unwrap(),
            "/box/admin/dashboard"
        );
    }

    #[tokio::test]
    async fn prefix_base_path_empty_base_is_noop() {
        let resp = html_response(r#"<html><head></head><body><a href="/admin/x">x</a></body></html>"#);
        let s = body_string(prefix_base_path(resp, "").await).await;
        assert!(s.contains(r#"href="/admin/x""#), "no base ⇒ unchanged");
        assert!(!s.contains("BASE ="), "no shim when no base");
    }
}
