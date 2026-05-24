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

use super::rewrite;
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

/// User-visible URL prefix for the two route families. Threaded
/// through to the core forward handler so the HTML rewriter knows
/// what to point `<base href>` at.
const APP_PREFIX: &str = "/app/";
const API_PREFIX: &str = "/api/";

#[axum::debug_handler]
async fn forward_app(
    state: State<AppState>,
    cookies: Cookies,
    ws: MaybeWs,
    Path((spec_id, rest)): Path<(String, String)>,
    req: Request,
) -> Response {
    forward(state, cookies, ws, APP_PREFIX, spec_id, format!("/{rest}"), req).await
}
async fn forward_app_root(
    state: State<AppState>,
    cookies: Cookies,
    ws: MaybeWs,
    Path(spec_id): Path<String>,
    req: Request,
) -> Response {
    forward(state, cookies, ws, APP_PREFIX, spec_id, "/".to_string(), req).await
}
async fn forward_api(
    state: State<AppState>,
    cookies: Cookies,
    ws: MaybeWs,
    Path((spec_id, rest)): Path<(String, String)>,
    req: Request,
) -> Response {
    forward(state, cookies, ws, API_PREFIX, spec_id, format!("/{rest}"), req).await
}
async fn forward_api_root(
    state: State<AppState>,
    cookies: Cookies,
    ws: MaybeWs,
    Path(spec_id): Path<String>,
    req: Request,
) -> Response {
    forward(state, cookies, ws, API_PREFIX, spec_id, "/".to_string(), req).await
}

// ── Core forward ───────────────────────────────────────────────────

async fn forward(
    State(state): State<AppState>,
    cookies: Cookies,
    ws_upgrade: MaybeWs,
    route_prefix: &'static str,
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
    //    pick/spawn. Also pin down the session_id we'll track
    //    this visitor under so the cookie and the tracker share
    //    the same identity.
    let (replica, session_id, cookie_used) =
        match resolve_replica(&state, &spec, &cookies).await {
            Ok(triple) => triple,
            Err(err) => {
                tracing::error!(spec = %spec.id, error = ?err, "resolve replica failed");
                return (
                    StatusCode::BAD_GATEWAY,
                    format!("backend error: {err}"),
                )
                    .into_response();
            }
        };

    // 4b. Heartbeat: record activity on the visitor's session.
    //     Only matters for sticky-needed specs — API requests are
    //     per-request, not per-session, so inflating
    //     `sessions_active` for them would mislead the scaler.
    if spec_kind_needs_sticky(spec.kind()) {
        let _outcome = state
            .sessions
            .touch_or_register(&state.replicas, session_id, &spec.id, &replica.id)
            .await;
    }

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

    // 6b. Inject `<base href>` into HTML responses on the
    //     `/app/` route family so relative URLs in the app's
    //     templates resolve against the prefix rather than the
    //     server root. API responses skip this entirely — APIs
    //     return JSON / binary, not HTML.
    let resp = if route_prefix == APP_PREFIX {
        let base = format!("{route_prefix}{}/", spec.id);
        rewrite::inject_base_href(resp, &base).await
    } else {
        resp
    };

    // 7. Issue sticky cookie when we just bound the visitor to a
    //    replica and the spec actually benefits from stickiness.
    //    The cookie carries the exact session_id we registered
    //    in the tracker, so subsequent requests touch the same
    //    entry instead of registering a duplicate.
    if !cookie_used && spec_kind_needs_sticky(spec.kind()) {
        let session = StickySession {
            session_id,
            spec_id: spec.id.clone(),
            replica_id: replica.id.clone(),
        };
        set_sticky_cookie(&cookies, &state.cookie_key, &session);
    }

    resp
}

fn spec_kind_needs_sticky(kind: SpecKind) -> bool {
    matches!(kind, SpecKind::Shiny | SpecKind::InteractiveApp)
}

fn find_spec<'a>(config: &'a ruscker_config::Config, id: &str) -> Option<&'a Spec> {
    config.proxy.specs.iter().find(|s| s.id == id)
}

/// Returns the chosen `Replica`, the session_id we'll track this
/// visitor under (either decoded from a valid cookie or freshly
/// minted), and whether the cookie was honored (so the caller
/// only sets a Set-Cookie header on fresh sessions).
async fn resolve_replica(
    state: &AppState,
    spec: &Spec,
    cookies: &Cookies,
) -> anyhow::Result<(Replica, uuid::Uuid, bool)> {
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
                    return Ok((r, session.session_id, true));
                }
            }
        }
    }
    let r = pick_or_spawn(state, spec).await?;
    Ok((r, uuid::Uuid::new_v4(), false))
}

/// Build + set the sticky cookie from an explicit `StickySession`
/// — keeps the session_id consistent with what we tracked in the
/// `SessionTracker` rather than minting a fresh, untracked id
/// inside the cookie helper.
fn set_sticky_cookie(cookies: &Cookies, key: &CookieKey, session: &StickySession) {
    let value = match sticky::encode(key, session) {
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

/// Pick an existing replica or spawn a new one.
///
/// **Fast path** (`O(1)` read lock): if at least one replica
/// exists for this spec, return one via round-robin.
///
/// **Slow path** (cold start, coalesced per spec): acquire this
/// spec's mutex from the per-spec coalescer, double-check the
/// registry, then spawn. Concurrent first-requests for
/// *different* specs go in parallel — only same-spec requests
/// wait. After the spawn the mutex releases; any thread that
/// was waiting now sees the registry already populated and
/// returns from the double-check without re-spawning.
///
/// The per-spec mutex lives in `state.spawn_locks`, a DashMap
/// keyed by spec id. Entries stay around forever; that's a few
/// dozen bytes per spec and `Mutex<()>` has no payload to
/// matter. Phase 4 GC sweeps them when a spec is deleted.
async fn pick_or_spawn(state: &AppState, spec: &Spec) -> anyhow::Result<Replica> {
    // Fast path: read lock, no spawn coordination needed.
    {
        let reg = state.replicas.read().await;
        let replicas = reg.replicas_of(&spec.id);
        if !replicas.is_empty() {
            let idx = pick_index(replicas.len());
            return Ok(replicas[idx].clone());
        }
    }

    // Slow path: coalesce concurrent first-requests for this spec.
    // Clone the Arc<Mutex> out of the DashMap (cheap), then drop
    // the DashMap entry guard before awaiting — never hold a
    // sync DashMap guard across `.await`.
    let spec_mutex: std::sync::Arc<tokio::sync::Mutex<()>> = state
        .spawn_locks
        .entry(spec.id.clone())
        .or_insert_with(|| std::sync::Arc::new(tokio::sync::Mutex::new(())))
        .clone();
    let _spawn_guard = spec_mutex.lock().await;

    // Double-check under the spec mutex. A sibling first-request
    // that ran ahead of us already populated the registry; we
    // skip the spawn and reuse what they made.
    {
        let reg = state.replicas.read().await;
        let replicas = reg.replicas_of(&spec.id);
        if !replicas.is_empty() {
            let idx = pick_index(replicas.len());
            return Ok(replicas[idx].clone());
        }
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

    let creds = creds_from_spec(spec);
    let limits = limits_from_spec(spec);
    tracing::info!(
        spec = %spec.id,
        image,
        inner_port = ?inner_port,
        with_creds = creds.is_some(),
        with_limits = !limits.is_empty(),
        "spawning first replica"
    );
    let mut req = ruscker_core::SpawnRequest::new(&spec.id, image).with_limits(limits);
    if let Some(port) = inner_port {
        req = req.with_port(port);
    }
    if let Some(c) = creds {
        req = req.with_creds(c);
    }
    let mut replica = backend
        .spawn_request(&req)
        .await
        .map_err(|e| anyhow::anyhow!("backend spawn: {e}"))?;
    // The backend doesn't know the spec's seat cap (lives in
    // config). Enrich so the session tracker and scaler see the
    // right capacity from the first request.
    replica.sessions_max = spec.effective_seats();

    // Take the write lock only for the insert — a few microseconds
    // — and release before the spec mutex unwinds.
    state.replicas.write().await.add(replica.clone());
    Ok(replica)
}

fn infer_inner_port(spec: &Spec) -> Option<u16> {
    match spec.kind() {
        SpecKind::Api => Some(8080),
        _ => None,
    }
}

/// Build optional registry credentials from a spec. Returns
/// `None` if the spec doesn't carry both a username and a
/// password — partial credentials make no sense (Docker would
/// just reject the pull). Lives next to the proxy spawn path
/// so the scaler can reuse it via `pub(crate)`.
///
/// The password comes through already env-interpolated by
/// `ruscker-config`'s loader — there's no `${VAR}` left to
/// resolve here.
/// Build [`ResourceLimits`] from a spec's `container-*-limit` /
/// `container-*-request` fields. Returns an empty (all-`None`)
/// `ResourceLimits` when the spec sets nothing — the backend
/// treats that as "no limits applied" and leaves the bollard
/// HostConfig minimal.
pub(crate) fn limits_from_spec(spec: &Spec) -> ruscker_core::ResourceLimits {
    ruscker_core::ResourceLimits {
        memory_bytes: spec.effective_memory_limit_bytes(),
        memory_reservation_bytes: spec.effective_memory_request_bytes(),
        cpu_fraction: spec
            .container_cpu_limit
            .filter(|c| c.is_finite() && *c > 0.0),
    }
}

pub(crate) fn creds_from_spec(spec: &Spec) -> Option<ruscker_core::RegistryCredentials> {
    let user = spec.docker_registry_username.as_deref().filter(|s| !s.is_empty());
    let pass = spec.docker_registry_password.as_deref().filter(|s| !s.is_empty());
    match (user, pass) {
        (Some(u), Some(p)) => Some(ruscker_core::RegistryCredentials {
            username: u.to_string(),
            password: p.to_string(),
            server_address: spec
                .docker_registry_domain
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(str::to_string),
        }),
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

    // -------------------------------------------------------------
    // Spawn coalescer: when N requests for the same spec land on a
    // cold cache, only ONE should call into the backend. The rest
    // wait at the per-spec mutex, see the registry populated, and
    // return without spawning. Without the coalescer (write-lock
    // approach), they'd all be serialized but each would still
    // spawn — N containers for one cold start.
    // -------------------------------------------------------------

    use async_trait::async_trait;
    use ruscker_config::Spec;
    use ruscker_core::{
        CoreResult, ContainerBackend, ReplicaId, ReplicaMetrics, ReplicaRegistry, ReplicaState,
    };
    use std::net::SocketAddr;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc as StdArc;
    use std::time::Duration as StdDuration;
    use tokio::sync::RwLock;

    struct CountingBackend {
        spawns: AtomicU32,
        delay: StdDuration,
    }

    #[async_trait]
    impl ContainerBackend for CountingBackend {
        async fn spawn(&self, spec_id: &str, _image: &str) -> CoreResult<ruscker_core::Replica> {
            self.spawns.fetch_add(1, Ordering::SeqCst);
            // Sleep so concurrent callers actually race past the
            // double-check. Without this, the first caller would
            // already be in the registry before its siblings ever
            // touch the mutex.
            tokio::time::sleep(self.delay).await;
            Ok(ruscker_core::Replica {
                id: ReplicaId(uuid::Uuid::new_v4()),
                spec_id: spec_id.to_string(),
                container_id: "fake".into(),
                upstream: "127.0.0.1:1".parse::<SocketAddr>().unwrap(),
                state: ReplicaState::Ready,
                started_at: chrono::Utc::now(),
                sessions_active: 0,
                sessions_max: 1,
            })
        }
        async fn spawn_with_port(
            &self,
            spec_id: &str,
            image: &str,
            _port: u16,
        ) -> CoreResult<ruscker_core::Replica> {
            self.spawn(spec_id, image).await
        }
        async fn stop(&self, _id: &ReplicaId) -> CoreResult<()> {
            Ok(())
        }
        async fn list(&self) -> CoreResult<Vec<ruscker_core::Replica>> {
            Ok(vec![])
        }
        async fn metrics(&self, _id: &ReplicaId) -> CoreResult<ReplicaMetrics> {
            Ok(ReplicaMetrics {
                cpu_percent: 0.0,
                memory_bytes: 0,
                network_rx_bytes: 0,
                network_tx_bytes: 0,
            })
        }
    }

    fn coalescer_state(backend: StdArc<dyn ContainerBackend>) -> AppState {
        // Minimal AppState — only the fields pick_or_spawn touches
        // need to be real. Everything else uses Default or empty.
        use ruscker_config::Config;
        let cfg = Config::from_yaml("specs: []").expect("empty config");
        AppState {
            config: std::sync::Arc::new(cfg),
            locales: std::sync::Arc::new(
                crate::i18n::Locales::load().expect("load locales"),
            ),
            admin_auth: Default::default(),
            db: None,
            images_dir: None,
            master_key: Default::default(),
            backend: Some(backend),
            replicas: StdArc::new(RwLock::new(ReplicaRegistry::new())),
            cookie_key: CookieKey::random(),
            spawn_locks: StdArc::new(dashmap::DashMap::new()),
            sessions: StdArc::new(crate::sessions::SessionTracker::new()),
            metrics: crate::metrics_cache::MetricsCache::new(),
        }
    }

    fn fake_spec(id: &str) -> Spec {
        let yaml = format!(
            r#"
id: {id}
display-name: Test
container-image: test:latest
"#
        );
        serde_yaml_ng::from_str(&yaml).expect("parse fake spec")
    }

    fn spec_yaml(yaml: &str) -> Spec {
        serde_yaml_ng::from_str(yaml).expect("parse spec yaml")
    }

    #[test]
    fn creds_from_spec_returns_none_for_anonymous_spec() {
        let s = fake_spec("x");
        assert!(creds_from_spec(&s).is_none());
    }

    #[test]
    fn creds_from_spec_returns_full_creds_when_both_present() {
        let s = spec_yaml(
            r#"
id: privapp
display-name: Private
container-image: priv.io/team/app:1
docker-registry-username: bot
docker-registry-password: hunter2
docker-registry-domain: priv.io
"#,
        );
        let c = creds_from_spec(&s).expect("creds present");
        assert_eq!(c.username, "bot");
        assert_eq!(c.password, "hunter2");
        assert_eq!(c.server_address.as_deref(), Some("priv.io"));
    }

    #[test]
    fn creds_from_spec_skips_when_only_username() {
        let s = spec_yaml(
            r#"
id: half
display-name: Half
container-image: x:1
docker-registry-username: bot
"#,
        );
        assert!(
            creds_from_spec(&s).is_none(),
            "half-credentials are no credentials"
        );
    }

    #[test]
    fn creds_from_spec_skips_when_only_password() {
        let s = spec_yaml(
            r#"
id: half2
display-name: Half2
container-image: x:1
docker-registry-password: secret
"#,
        );
        assert!(creds_from_spec(&s).is_none());
    }

    #[test]
    fn creds_from_spec_treats_empty_strings_as_absent() {
        let s = spec_yaml(
            r#"
id: blanks
display-name: Blanks
container-image: x:1
docker-registry-username: ""
docker-registry-password: ""
docker-registry-domain: ""
"#,
        );
        assert!(
            creds_from_spec(&s).is_none(),
            "empty strings shouldn't authenticate"
        );
    }

    #[test]
    fn limits_from_spec_empty_when_no_fields_set() {
        let s = fake_spec("x");
        let l = limits_from_spec(&s);
        assert!(l.is_empty());
    }

    #[test]
    fn limits_from_spec_parses_memory_and_cpu() {
        let s = spec_yaml(
            r#"
id: capped
display-name: Capped
container-image: x:1
container-memory-limit: 512m
container-memory-request: 256m
container-cpu-limit: 1.5
"#,
        );
        let l = limits_from_spec(&s);
        assert_eq!(l.memory_bytes, Some(512 * 1024 * 1024));
        assert_eq!(l.memory_reservation_bytes, Some(256 * 1024 * 1024));
        assert_eq!(l.cpu_fraction, Some(1.5));
        assert!(!l.is_empty());
    }

    #[test]
    fn limits_from_spec_rejects_nonsense_cpu() {
        let s = spec_yaml(
            r#"
id: nan
display-name: NaN
container-image: x:1
container-cpu-limit: -1.0
"#,
        );
        // Negative CPU is filtered out; the rest of limits stays empty.
        let l = limits_from_spec(&s);
        assert!(l.cpu_fraction.is_none());
    }

    #[test]
    fn creds_from_spec_allows_no_domain_for_docker_hub() {
        let s = spec_yaml(
            r#"
id: hubapp
display-name: Hub
container-image: bot/app:1
docker-registry-username: bot
docker-registry-password: hunter2
"#,
        );
        let c = creds_from_spec(&s).expect("creds present");
        assert!(c.server_address.is_none(), "Docker Hub default");
    }

    #[tokio::test]
    async fn coalescer_spawns_once_under_concurrent_first_requests() {
        let backend = StdArc::new(CountingBackend {
            spawns: AtomicU32::new(0),
            delay: StdDuration::from_millis(80),
        });
        let state = coalescer_state(backend.clone() as StdArc<dyn ContainerBackend>);
        let spec = fake_spec("coalesced");

        // Fan out 8 concurrent callers for the SAME spec.
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let st = state.clone();
            let sp = spec.clone();
            tasks.push(tokio::spawn(async move {
                pick_or_spawn(&st, &sp).await.expect("pick_or_spawn")
            }));
        }
        let mut replica_ids = std::collections::HashSet::new();
        for t in tasks {
            let r = t.await.expect("join");
            replica_ids.insert(r.id);
        }

        assert_eq!(
            backend.spawns.load(Ordering::SeqCst),
            1,
            "exactly one backend spawn for {} concurrent first-requests",
            8
        );
        assert_eq!(replica_ids.len(), 1, "all callers got the same replica");
    }

    #[tokio::test]
    async fn coalescer_does_not_serialize_different_specs() {
        let backend = StdArc::new(CountingBackend {
            spawns: AtomicU32::new(0),
            delay: StdDuration::from_millis(120),
        });
        let state = coalescer_state(backend.clone() as StdArc<dyn ContainerBackend>);

        // Two different specs — should spawn in parallel, not in
        // series. Wall time bounded by 1× delay + overhead, not 2×.
        let start = std::time::Instant::now();
        let st1 = state.clone();
        let st2 = state.clone();
        let s1 = fake_spec("alpha");
        let s2 = fake_spec("beta");
        let (r1, r2) = tokio::join!(
            tokio::spawn(async move { pick_or_spawn(&st1, &s1).await.expect("a") }),
            tokio::spawn(async move { pick_or_spawn(&st2, &s2).await.expect("b") }),
        );
        r1.unwrap();
        r2.unwrap();
        let elapsed = start.elapsed();

        assert_eq!(backend.spawns.load(Ordering::SeqCst), 2);
        assert!(
            elapsed < StdDuration::from_millis(220),
            "two cold spawns should run in parallel, took {:?}",
            elapsed
        );
    }
}
