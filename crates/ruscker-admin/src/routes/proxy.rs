//! HTTP and WebSocket forwarding for `/app/{spec}/{*path}` and
//! `/api/{spec}/{*path}`.
//!
//! Routing pipeline per request:
//!
//! ```text
//!   request comes in
//!         │
//!         ▼
//!   find spec by id ─── not found ──► 404
//!         │
//!         ▼
//!   spec is External? ─── yes ──► 302 to template-properties.link
//!         │
//!         ▼
//!   has sticky cookie? ── valid + replica alive ──► reuse
//!         │ no / stale
//!         ▼
//!   read or spawn a replica  (pick_or_spawn)
//!         │
//!         ▼
//!   WS upgrade? ─── yes ──► ws::pump
//!         │ no
//!         ▼
//!   HTTP forward via hyper-util
//!         │
//!         ▼
//!   for Shiny/InteractiveApp without a cookie ──► set sticky cookie
//! ```

use axum::body::Body;
use axum::extract::{FromRequestParts, Path, Request, State, WebSocketUpgrade};
use axum::http::{header, request::Parts, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::any;
use axum::Router;
use http_body_util::BodyExt;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use ruscker_config::{Spec, SpecKind};
use ruscker_core::Replica;
use ruscker_proxy::sticky::{self, CookieKey, StickySession, COOKIE_NAME};
use ruscker_proxy::ws;
use std::sync::OnceLock;
use tower_cookies::cookie::time::Duration;
use tower_cookies::{Cookie, Cookies};

use crate::AppState;

/// Wrapper extractor: `WebSocketUpgrade::FromRequestParts` returns
/// an error for non-upgrade requests; axum 0.8 doesn't blanket-
/// impl `Option<T: FromRequestParts>`, so we provide our own.
/// `MaybeWs(None)` = plain HTTP, `MaybeWs(Some)` = upgrade.
struct MaybeWs(Option<WebSocketUpgrade>);

impl<S: Send + Sync> FromRequestParts<S> for MaybeWs {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Ok(MaybeWs(
            WebSocketUpgrade::from_request_parts(parts, state).await.ok(),
        ))
    }
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/app/{spec_id}", any(forward_app_root))
        .route("/app/{spec_id}/", any(forward_app_root))
        .route("/app/{spec_id}/{*rest}", any(forward_app))
        .route("/api/{spec_id}", any(forward_api_root))
        .route("/api/{spec_id}/", any(forward_api_root))
        .route("/api/{spec_id}/{*rest}", any(forward_api))
}

// ── Hyper client (one per process) ────────────────────────────────

fn http_client() -> &'static Client<HttpConnector, Body> {
    static CLIENT: OnceLock<Client<HttpConnector, Body>> = OnceLock::new();
    CLIENT.get_or_init(|| Client::builder(TokioExecutor::new()).build_http())
}

// ── Path-strip handlers ───────────────────────────────────────────

#[axum::debug_handler]
async fn forward_app(
    state: State<AppState>,
    cookies: Cookies,
    ws: MaybeWs,
    Path((spec_id, rest)): Path<(String, String)>,
    req: Request,
) -> Response {
    forward(state, cookies, ws, spec_id, format!("/{rest}"), req).await
}
async fn forward_app_root(
    state: State<AppState>,
    cookies: Cookies,
    ws: MaybeWs,
    Path(spec_id): Path<String>,
    req: Request,
) -> Response {
    forward(state, cookies, ws, spec_id, "/".to_string(), req).await
}
async fn forward_api(
    state: State<AppState>,
    cookies: Cookies,
    ws: MaybeWs,
    Path((spec_id, rest)): Path<(String, String)>,
    req: Request,
) -> Response {
    forward(state, cookies, ws, spec_id, format!("/{rest}"), req).await
}
async fn forward_api_root(
    state: State<AppState>,
    cookies: Cookies,
    ws: MaybeWs,
    Path(spec_id): Path<String>,
    req: Request,
) -> Response {
    forward(state, cookies, ws, spec_id, "/".to_string(), req).await
}

// ── Core forward ───────────────────────────────────────────────────

async fn forward(
    State(state): State<AppState>,
    cookies: Cookies,
    ws_upgrade: MaybeWs,
    spec_id: String,
    upstream_path: String,
    req: Request,
) -> Response {
    // 1. Find the spec.
    let Some(spec) = find_spec(&state.config, &spec_id) else {
        return (StatusCode::NOT_FOUND, format!("spec `{spec_id}` not found")).into_response();
    };
    let spec = spec.clone();

    // 2. External link: bounce.
    if spec.kind() == SpecKind::External {
        if let Some(target) = spec.template_properties.get_str("link") {
            return Redirect::to(target).into_response();
        }
        return (
            StatusCode::NOT_FOUND,
            "external spec has no template-properties.link",
        )
            .into_response();
    }

    // 3. Backend required to proxy.
    if state.backend.is_none() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no container backend wired — start with --docker",
        )
            .into_response();
    }

    // 4. Resolve the replica: sticky-first, fall back to
    //    pick/spawn. Capture whether the cookie was honored so we
    //    only Set-Cookie when we just minted a new session.
    let (replica, cookie_used) =
        match resolve_replica(&state, &spec, &cookies).await {
            Ok(pair) => pair,
            Err(err) => {
                tracing::error!(spec = %spec.id, error = ?err, "resolve replica failed");
                return (
                    StatusCode::BAD_GATEWAY,
                    format!("backend error: {err}"),
                )
                    .into_response();
            }
        };

    // 5. WebSocket branch hijacks the upgrade and pumps frames;
    //    after the upgrade response is sent, the rest of axum's
    //    response pipeline ignores anything we'd add (cookies
    //    can't be set on a 101). Issuing the sticky cookie on the
    //    preceding HTTP request is how WS-only apps stay sticky.
    if let MaybeWs(Some(upgrade)) = ws_upgrade {
        let upstream_ws_url = format!("ws://{}{}", replica.upstream, upstream_path);
        tracing::debug!(
            spec = %spec.id, replica = %replica.id, url = %upstream_ws_url,
            "ws upgrade"
        );
        return upgrade.on_upgrade(move |socket| ws::pump(socket, upstream_ws_url));
    }

    // 6. HTTP forward.
    tracing::debug!(
        spec = %spec.id, replica = %replica.id,
        upstream = %replica.upstream, path = %upstream_path,
        "forwarding"
    );
    let resp = match do_forward(&replica, upstream_path, req).await {
        Ok(r) => r,
        Err(err) => {
            tracing::error!(
                spec = %spec.id, replica = %replica.id,
                error = ?err, "forward failed"
            );
            return (
                StatusCode::BAD_GATEWAY,
                format!("upstream error: {err}"),
            )
                .into_response();
        }
    };

    // 7. Issue sticky cookie when we just bound the visitor to a
    //    replica and the spec actually benefits from stickiness.
    if !cookie_used && spec_kind_needs_sticky(spec.kind()) {
        set_sticky_cookie(&cookies, &state.cookie_key, &spec.id, &replica);
    }

    resp
}

fn spec_kind_needs_sticky(kind: SpecKind) -> bool {
    matches!(kind, SpecKind::Shiny | SpecKind::InteractiveApp)
}

fn find_spec<'a>(config: &'a ruscker_config::Config, id: &str) -> Option<&'a Spec> {
    config.proxy.specs.iter().find(|s| s.id == id)
}

/// Returns the chosen `Replica` and whether the sticky cookie was
/// honored (so the caller knows whether to set a fresh one).
async fn resolve_replica(
    state: &AppState,
    spec: &Spec,
    cookies: &Cookies,
) -> anyhow::Result<(Replica, bool)> {
    if let Some(raw) = cookies.get(COOKIE_NAME) {
        if let Ok(session) = sticky::decode(&state.cookie_key, raw.value()) {
            // Defense in depth: a cookie for spec A must not
            // route to a replica registered against spec B.
            if session.spec_id == spec.id {
                let reg = state.replicas.read().await;
                let alive = reg
                    .replicas_of(&spec.id)
                    .iter()
                    .find(|r| r.id == session.replica_id)
                    .cloned();
                if let Some(r) = alive {
                    return Ok((r, true));
                }
            }
        }
    }
    let r = pick_or_spawn(state, spec).await?;
    Ok((r, false))
}

fn set_sticky_cookie(
    cookies: &Cookies,
    key: &CookieKey,
    spec_id: &str,
    replica: &Replica,
) {
    let session = StickySession::new(spec_id.to_string(), replica.id.clone());
    let value = match sticky::encode(key, &session) {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(error = ?err, "encode sticky cookie failed");
            return;
        }
    };
    let mut c = Cookie::new(COOKIE_NAME, value);
    c.set_path("/");
    c.set_http_only(true);
    c.set_same_site(tower_cookies::cookie::SameSite::Lax);
    // 8h matches a typical workday — long enough that returning
    // visitors don't lose state mid-session, short enough that a
    // forgotten browser tab doesn't pin a container forever.
    c.set_max_age(Duration::hours(8));
    cookies.add(c);
}

/// Read-then-spawn replica picker. Holds the write lock across
/// spawn — slow under heavy first-request contention but
/// straightforward and correct. Phase 3.5 swaps this for a
/// `tokio::sync::OnceCell`-per-spec coalescer.
async fn pick_or_spawn(state: &AppState, spec: &Spec) -> anyhow::Result<Replica> {
    // Fast path
    {
        let reg = state.replicas.read().await;
        let replicas = reg.replicas_of(&spec.id);
        if !replicas.is_empty() {
            let idx = pick_index(replicas.len());
            return Ok(replicas[idx].clone());
        }
    }

    let mut reg = state.replicas.write().await;
    let replicas = reg.replicas_of(&spec.id);
    if !replicas.is_empty() {
        return Ok(replicas[0].clone());
    }

    let backend = state
        .backend
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("no backend"))?;
    let image = spec
        .container_image
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("spec {} has no container-image", spec.id))?;

    let inner_port = spec
        .api
        .as_ref()
        .and_then(|a| a.port)
        .or_else(|| infer_inner_port(spec));

    tracing::info!(spec = %spec.id, image, inner_port = ?inner_port, "spawning first replica");
    let replica = match inner_port {
        Some(port) => backend.spawn_with_port(&spec.id, image, port).await,
        None => backend.spawn(&spec.id, image).await,
    }
    .map_err(|e| anyhow::anyhow!("backend spawn: {e}"))?;
    reg.add(replica.clone());
    Ok(replica)
}

fn infer_inner_port(spec: &Spec) -> Option<u16> {
    match spec.kind() {
        SpecKind::Api => Some(8080),
        _ => None,
    }
}

fn pick_index(n: usize) -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    N.fetch_add(1, Ordering::Relaxed) % n
}

async fn do_forward(
    replica: &Replica,
    upstream_path: String,
    mut req: Request,
) -> anyhow::Result<Response> {
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let new_uri: Uri = format!("http://{}{}{}", replica.upstream, upstream_path, query)
        .parse()
        .map_err(|e| anyhow::anyhow!("build upstream uri: {e}"))?;
    *req.uri_mut() = new_uri;

    strip_hop_headers(req.headers_mut());

    let client = http_client().clone();
    let upstream_resp = client.request(req).await?;

    let (parts, body) = upstream_resp.into_parts();
    let body = Body::new(body.map_err(|e| std::io::Error::other(e)));
    let mut resp = Response::from_parts(parts, body);
    strip_hop_headers(resp.headers_mut());
    Ok(resp)
}

const HOP_BY_HOP: &[&str] = &[
    "connection",
    "keep-alive",
    "proxy-authenticate",
    "proxy-authorization",
    "te",
    "trailers",
    "transfer-encoding",
    "upgrade",
    "host",
];

fn strip_hop_headers(headers: &mut HeaderMap) {
    let extra: Vec<String> = headers
        .get("connection")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').map(|t| t.trim().to_lowercase()).collect())
        .unwrap_or_default();
    for h in HOP_BY_HOP.iter().copied().chain(extra.iter().map(String::as_str)) {
        headers.remove(h);
    }
    let _ = headers
        .entry("via")
        .or_insert(HeaderValue::from_static("1.1 ruscker"));
    headers.remove("x-forwarded-proto");
    headers.remove("x-forwarded-port");
    let _ = header::CONTENT_TYPE; // silence unused import in some builds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_hop_removes_connection_and_listed_tokens() {
        let mut h = HeaderMap::new();
        h.insert("connection", HeaderValue::from_static("keep-alive, x-custom"));
        h.insert("keep-alive", HeaderValue::from_static("timeout=5"));
        h.insert("x-custom", HeaderValue::from_static("yes"));
        h.insert("content-type", HeaderValue::from_static("text/plain"));
        strip_hop_headers(&mut h);
        assert!(!h.contains_key("connection"));
        assert!(!h.contains_key("keep-alive"));
        assert!(!h.contains_key("x-custom"));
        assert_eq!(h.get("content-type").unwrap(), "text/plain");
        assert_eq!(h.get("via").unwrap(), "1.1 ruscker");
    }

    #[test]
    fn pick_index_round_robins() {
        let a = pick_index(3);
        let b = pick_index(3);
        let c = pick_index(3);
        let d = pick_index(3);
        assert_eq!((b + 3 - a) % 3, 1);
        assert_eq!((c + 3 - b) % 3, 1);
        assert_eq!((d + 3 - c) % 3, 1);
    }
}
