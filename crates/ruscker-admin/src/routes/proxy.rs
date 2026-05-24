//! HTTP forwarding for `/app/{spec_id}/{*path}` and
//! `/api/{spec_id}/{*path}`. WebSocket handling lives in a
//! sibling module in a later commit; this batch is HTTP-only.
//!
//! Spawn-on-demand: the first request for a spec that has no
//! replicas triggers a synchronous `ContainerBackend::spawn`,
//! which is then cached in `state.replicas`. Subsequent requests
//! skip the spawn and go straight to forwarding. Concurrent first
//! requests race the write lock — the loser sees the winner's
//! replica and reuses it. This is "good enough" for the MVP; a
//! per-spec spawn coalescer is a Phase 3.5 refinement.
//!
//! External-link specs (no `container-image`) get a 302 to
//! `template-properties.link` instead of being proxied.

use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::any;
use axum::Router;
use http_body_util::BodyExt;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use ruscker_config::{Spec, SpecKind};
use ruscker_core::Replica;
use std::sync::OnceLock;

use crate::AppState;

pub fn routes() -> Router<AppState> {
    // axum 0.8 doesn't merge trailing-slash variants: `{spec}` and
    // `{spec}/{*rest}` together still don't catch `{spec}/`. Register
    // all three variants explicitly so `/api/foo`, `/api/foo/`, and
    // `/api/foo/bar` all land on a handler.
    Router::new()
        .route("/app/{spec_id}", any(forward_app_root))
        .route("/app/{spec_id}/", any(forward_app_root))
        .route("/app/{spec_id}/{*rest}", any(forward_app))
        .route("/api/{spec_id}", any(forward_api_root))
        .route("/api/{spec_id}/", any(forward_api_root))
        .route("/api/{spec_id}/{*rest}", any(forward_api))
}

// ── Hyper client (one per process) ────────────────────────────────

/// Lazily-built `hyper-util` HTTP/1 client. We only target
/// container `127.0.0.1:<port>` upstreams, so plain HTTP without
/// TLS is enough; switching to HTTPS would be a connector swap.
fn http_client() -> &'static Client<HttpConnector, Body> {
    static CLIENT: OnceLock<Client<HttpConnector, Body>> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder(TokioExecutor::new()).build_http()
    })
}

// ── Path-strip handlers ───────────────────────────────────────────

async fn forward_app(
    State(state): State<AppState>,
    Path((spec_id, rest)): Path<(String, String)>,
    req: Request,
) -> Response {
    forward(state, spec_id, format!("/{rest}"), req).await
}

async fn forward_app_root(
    State(state): State<AppState>,
    Path(spec_id): Path<String>,
    req: Request,
) -> Response {
    forward(state, spec_id, "/".to_string(), req).await
}

async fn forward_api(
    State(state): State<AppState>,
    Path((spec_id, rest)): Path<(String, String)>,
    req: Request,
) -> Response {
    forward(state, spec_id, format!("/{rest}"), req).await
}

async fn forward_api_root(
    State(state): State<AppState>,
    Path(spec_id): Path<String>,
    req: Request,
) -> Response {
    forward(state, spec_id, "/".to_string(), req).await
}

// ── Core forward ───────────────────────────────────────────────────

async fn forward(
    state: AppState,
    spec_id: String,
    upstream_path: String,
    req: Request,
) -> Response {
    // 1. Find the spec.
    let Some(spec) = find_spec(&state.config, &spec_id) else {
        return (StatusCode::NOT_FOUND, format!("spec `{spec_id}` not found")).into_response();
    };
    let spec = spec.clone();

    // 2. External links: bounce to template-properties.link.
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
            "no container backend wired — start the server with --docker",
        )
            .into_response();
    }

    // 4. Pick or spawn a replica.
    let replica = match pick_or_spawn(&state, &spec).await {
        Ok(r) => r,
        Err(err) => {
            tracing::error!(spec = %spec.id, error = ?err, "pick/spawn failed");
            return (
                StatusCode::BAD_GATEWAY,
                format!("backend error: {err}"),
            )
                .into_response();
        }
    };

    // 5. Forward.
    tracing::debug!(
        spec = %spec.id,
        replica = %replica.id,
        upstream = %replica.upstream,
        path = %upstream_path,
        "forwarding"
    );
    match do_forward(&replica, upstream_path, req).await {
        Ok(resp) => resp,
        Err(err) => {
            tracing::error!(spec = %spec.id, replica = %replica.id, error = ?err, "forward failed");
            (
                StatusCode::BAD_GATEWAY,
                format!("upstream error: {err}"),
            )
                .into_response()
        }
    }
}

fn find_spec<'a>(config: &'a ruscker_config::Config, id: &str) -> Option<&'a Spec> {
    config.proxy.specs.iter().find(|s| s.id == id)
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
            // For now: round-robin via simple index rotation.
            // Phase 3.5 plugs in `ruscker_core::Router::pick`
            // with the spec's effective_routing() strategy.
            let idx = pick_index(replicas.len());
            return Ok(replicas[idx].clone());
        }
    }

    // Slow path: take write lock, re-check, spawn.
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

    // The default `ContainerBackend::spawn` uses 3838 (Shiny). When
    // the spec is an API with an `api.port`, honor that instead so
    // the host-port mapping reaches the actual upstream socket.
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

/// Best-guess inner port when the spec doesn't carry an explicit
/// `api.port`. Returns `None` for Shiny (the ContainerBackend uses
/// its own 3838 default).
fn infer_inner_port(spec: &Spec) -> Option<u16> {
    match spec.kind() {
        SpecKind::Api => Some(8080), // Plumber/FastAPI default
        _ => None,                   // Shiny / InteractiveApp use backend default
    }
}

/// Cheap round-robin counter. Per-process global; fine for the
/// MVP. Real least-connections / weighted picks come with the
/// `Router` integration.
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
    // Rewrite URI to point at the upstream container, preserving
    // the original query string.
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let new_uri: Uri = format!("http://{}{}{}", replica.upstream, upstream_path, query)
        .parse()
        .map_err(|e| anyhow::anyhow!("build upstream uri: {e}"))?;
    *req.uri_mut() = new_uri;

    // Hop-by-hop headers RFC 7230 §6.1 must NOT be forwarded.
    // We also drop `Host`: hyper recomputes it from the URI.
    strip_hop_headers(req.headers_mut());

    let client = http_client().clone();
    let upstream_resp = client.request(req).await?;

    // Convert hyper Response to axum Response. The body type from
    // hyper-util is Incoming; we wrap it as axum Body.
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
    // Honor Connection: <token> — anything listed there is also
    // hop-by-hop. RFC 7230 §6.1.
    let extra: Vec<String> = headers
        .get("connection")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.split(',').map(|t| t.trim().to_lowercase()).collect())
        .unwrap_or_default();
    for h in HOP_BY_HOP.iter().copied().chain(extra.iter().map(String::as_str)) {
        headers.remove(h);
    }
    // Mark the request as having traversed a proxy (best-effort).
    let _ = headers
        .entry("via")
        .or_insert(HeaderValue::from_static("1.1 ruscker"));
    // We're an internal proxy with TLS terminated upstream by the
    // operator; don't claim to forward TLS state.
    headers.remove("x-forwarded-proto");
    headers.remove("x-forwarded-port");
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
