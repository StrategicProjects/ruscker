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
use axum::extract::{ConnectInfo, FromRequestParts, Path, Request, State, WebSocketUpgrade};
use axum::http::{header, request::Parts, HeaderMap, HeaderValue, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Redirect, Response};
use axum::routing::any;
use axum::Router;
use http_body_util::BodyExt;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use ruscker_config::{RoutingStrategy, Spec, SpecKind};
use ruscker_core::{Replica, ReplicaState};
use ruscker_proxy::sticky::{self, CookieKey, StickySession, COOKIE_NAME};
use ruscker_proxy::ws;
use std::net::SocketAddr;
use std::sync::OnceLock;
use tower_cookies::cookie::time::Duration;
use tower_cookies::{Cookie, Cookies};

use super::rewrite;
use crate::ratelimit;
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

/// Optional TCP peer address. `ConnectInfo<SocketAddr>` is present
/// only when the server is run via
/// `into_make_service_with_connect_info` (production); under
/// `Router::oneshot` (tests) there's no socket. axum 0.8 doesn't
/// blanket-impl `Option<T: FromRequestParts>`, so — like `MaybeWs`
/// — we provide our own infallible wrapper that yields `None`
/// instead of rejecting.
struct MaybePeer(Option<SocketAddr>);

impl<S: Send + Sync> FromRequestParts<S> for MaybePeer {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        Ok(MaybePeer(
            ConnectInfo::<SocketAddr>::from_request_parts(parts, state)
                .await
                .ok()
                .map(|ci| ci.0),
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

// Each handler extracts `Option<ConnectInfo<SocketAddr>>`: it's
// `Some` in production (the listener is served with connect-info)
// and `None` under `Router::oneshot` tests, where there's no socket
// — the client key then falls back to a trusted XFF or "unknown".
#[axum::debug_handler]
async fn forward_app(
    state: State<AppState>,
    cookies: Cookies,
    ws: MaybeWs,
    peer: MaybePeer,
    session: crate::auth::MaybeSession,
    Path((spec_id, rest)): Path<(String, String)>,
    req: Request,
) -> Response {
    forward(state, cookies, ws, peer.0, session, APP_PREFIX, spec_id, format!("/{rest}"), req).await
}
async fn forward_app_root(
    state: State<AppState>,
    cookies: Cookies,
    ws: MaybeWs,
    peer: MaybePeer,
    session: crate::auth::MaybeSession,
    Path(spec_id): Path<String>,
    req: Request,
) -> Response {
    forward(state, cookies, ws, peer.0, session, APP_PREFIX, spec_id, "/".to_string(), req).await
}
async fn forward_api(
    state: State<AppState>,
    cookies: Cookies,
    ws: MaybeWs,
    peer: MaybePeer,
    session: crate::auth::MaybeSession,
    Path((spec_id, rest)): Path<(String, String)>,
    req: Request,
) -> Response {
    forward(state, cookies, ws, peer.0, session, API_PREFIX, spec_id, format!("/{rest}"), req).await
}
async fn forward_api_root(
    state: State<AppState>,
    cookies: Cookies,
    ws: MaybeWs,
    peer: MaybePeer,
    session: crate::auth::MaybeSession,
    Path(spec_id): Path<String>,
    req: Request,
) -> Response {
    forward(state, cookies, ws, peer.0, session, API_PREFIX, spec_id, "/".to_string(), req).await
}

// ── Core forward ───────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn forward(
    State(state): State<AppState>,
    cookies: Cookies,
    ws_upgrade: MaybeWs,
    peer: Option<SocketAddr>,
    session: crate::auth::MaybeSession,
    route_prefix: &'static str,
    spec_id: String,
    upstream_path: String,
    req: Request,
) -> Response {
    // Capture the request scheme before `req` is consumed by the
    // forward — used to decide the `Secure` flag on the sticky
    // cookie we may set at the end.
    let is_https = crate::auth::request_is_https(req.headers());

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

    // 2b. API policies (CORS + rate limit). These apply only to the
    //     `/api/` route family and are configured under `spec.api`.
    //     We run them *before* touching the backend so a preflight or
    //     a throttled request never spawns or wakes a container.
    //     `cors_on` is threaded to the exits below so every API
    //     response a browser sees carries the headers (via `with_cors`).
    let api = if route_prefix == API_PREFIX {
        spec.api.clone()
    } else {
        None
    };
    let cors_on = api.as_ref().map(|a| a.cors).unwrap_or(false);

    if let Some(api) = &api {
        // CORS preflight: answer `OPTIONS` ourselves, no upstream.
        if cors_on && *req.method() == Method::OPTIONS {
            return cors_preflight_response();
        }

        // Per-client rate limit, if the spec configured a valid one.
        if let Some(policy) = api.rate_policy() {
            let client = client_key(&state, req.headers(), peer.as_ref());
            if let ratelimit::RateDecision::Deny { retry_after_secs } =
                state.api_limiter.check(&spec.id, &client, &policy)
            {
                tracing::debug!(
                    spec = %spec.id, client = %client, retry_after_secs,
                    "rate limit exceeded"
                );
                let mut resp = (
                    StatusCode::TOO_MANY_REQUESTS,
                    format!("rate limit exceeded; retry after {retry_after_secs}s\n"),
                )
                    .into_response();
                if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                    resp.headers_mut().insert(header::RETRY_AFTER, v);
                }
                return with_cors(resp, cors_on);
            }
        }
    }

    // 2c. Max body size. Applies to both route families (`/app/` and
    //     `/api/`). The effective limit is the spec's override or the
    //     global `proxy.max-body-size`. We reject early on a declared
    //     `Content-Length` over the cap (no backend work); the body is
    //     *also* wrapped in `Limited` at the forward step (below) so a
    //     chunked or under-declared body can't slip past the cap.
    let max_body = spec.effective_max_body_bytes(state.config.proxy.max_body_bytes());
    if let Some(limit) = max_body {
        if let Some(len) = req
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse::<i64>().ok())
        {
            if len > limit {
                tracing::debug!(spec = %spec.id, content_length = len, limit, "body too large");
                return with_cors(
                    (
                        StatusCode::PAYLOAD_TOO_LARGE,
                        format!("request body exceeds {limit} bytes\n"),
                    )
                        .into_response(),
                    cors_on,
                );
            }
        }
    }
    // Byte cap to hand to the forward step; `None` ⇒ unlimited.
    let body_cap = max_body.map(|l| usize::try_from(l).unwrap_or(usize::MAX));

    // 2d. Access control (#155). An open spec (no `access-groups` /
    //     `access-users`) stays reachable by anyone — including
    //     anonymous `/api` clients, so unrestricted APIs keep working.
    //     A restricted spec requires a session whose username or groups
    //     match; an Admin role (incl. the break-glass token) passes
    //     everything. This is the *enforcement* that the landing's card
    //     filtering only hints at — hiding a card never stopped a direct
    //     hit on `/app/{spec}`. We resolve groups per request (only for
    //     restricted specs) from the same `users` store the landing uses.
    if !spec.is_open() {
        let session = session.0;
        let is_admin = session
            .as_ref()
            .map(|s| s.role == crate::auth::Role::Admin)
            .unwrap_or(false);
        let username = session.as_ref().and_then(|s| s.actor.clone());
        let groups: Vec<String> = match (username.as_deref(), state.db.as_ref()) {
            (Some(user), Some(db)) => crate::db::users::fetch(db, user)
                .await
                .ok()
                .flatten()
                .map(|row| row.groups)
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        if !spec.access_allows(is_admin, username.as_deref(), &groups) {
            tracing::info!(
                spec = %spec.id,
                user = username.as_deref().unwrap_or("-"),
                "access denied to restricted spec"
            );
            // An anonymous visitor hitting a restricted interactive app
            // is sent to log in; everyone else (and all API clients) get
            // a flat 403 (CORS-wrapped for the `/api/` family).
            if route_prefix == APP_PREFIX && session.is_none() {
                return Redirect::to("/admin/login").into_response();
            }
            return with_cors(
                (StatusCode::FORBIDDEN, "access denied\n").into_response(),
                cors_on,
            );
        }
    }

    // 3. Backend required to proxy.
    if state.backend.is_none() {
        return with_cors(
            (
                StatusCode::SERVICE_UNAVAILABLE,
                "no container backend wired — start with --docker",
            )
                .into_response(),
            cors_on,
        );
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
                return with_cors(
                    // Detail is logged above; don't leak it to the client.
                    (StatusCode::BAD_GATEWAY, "backend unavailable").into_response(),
                    cors_on,
                );
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
        // Forward the client's session cookie and requested subprotocol
        // onto the upstream handshake so the app keeps its session.
        let cookie = req
            .headers()
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let subprotocols = req
            .headers()
            .get(header::SEC_WEBSOCKET_PROTOCOL)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        tracing::debug!(
            spec = %spec.id, replica = %replica.id, url = %upstream_ws_url,
            "ws upgrade"
        );
        return upgrade
            .on_upgrade(move |socket| ws::pump(socket, upstream_ws_url, cookie, subprotocols));
    }

    // 6. HTTP forward.
    // The mount prefix we advertise to the upstream via
    // `X-Forwarded-Prefix` / `X-Script-Name` — the public path the
    // spec is reachable at, with no trailing slash (`route_prefix`
    // already carries one), e.g. `/app/my-shiny` or `/api/my-api`.
    let forwarded_prefix = format!("{route_prefix}{}", spec.id);
    tracing::debug!(
        spec = %spec.id, replica = %replica.id,
        upstream = %replica.upstream, path = %upstream_path,
        prefix = %forwarded_prefix,
        "forwarding"
    );
    let resp = match do_forward(
        &replica,
        upstream_path,
        &forwarded_prefix,
        is_https,
        req,
        body_cap,
    )
    .await
    {
        Ok(r) => r,
        Err(err) => {
            tracing::error!(
                spec = %spec.id, replica = %replica.id,
                error = ?err, "forward failed"
            );
            return with_cors(
                // Detail is logged above; keep the client-facing body generic.
                (StatusCode::BAD_GATEWAY, "upstream error").into_response(),
                cors_on,
            );
        }
    };

    // 6b. Inject `<base href>` into HTML responses on the
    //     `/app/` route family so relative URLs in the app's
    //     templates resolve against the prefix rather than the
    //     server root. API responses skip this entirely — APIs
    //     return JSON / binary, not HTML. Operators can also disable
    //     the transform per spec (`inject-base-href: false`) once the
    //     app self-routes from the forwarded-prefix headers above.
    let resp = if route_prefix == APP_PREFIX && spec.effective_inject_base_href() {
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
        set_sticky_cookie(&cookies, &state.cookie_key, &session, is_https);
    }

    with_cors(resp, cors_on)
}

fn spec_kind_needs_sticky(kind: SpecKind) -> bool {
    matches!(kind, SpecKind::Shiny | SpecKind::InteractiveApp)
}

// ── API policy helpers (rate limit + CORS) ─────────────────────────

/// Whether to believe an inbound `X-Forwarded-For` header. We only
/// do when the operator opted into forwarded headers (ShinyProxy's
/// `server.useForwardHeaders`, or a `forward-headers-strategy` other
/// than `none`). Without that opt-in, a direct client could spoof
/// the header to dodge a per-IP rate limit — so we ignore it and key
/// on the real TCP peer instead.
fn forward_headers_trusted(server: &ruscker_config::Server) -> bool {
    server.use_forward_headers
        || server
            .forward_headers_strategy
            .as_deref()
            .map(|s| !s.eq_ignore_ascii_case("none"))
            .unwrap_or(false)
}

/// Derive the per-client key used for rate limiting.
///
/// Derives the per-client key for rate limiting.
///
/// When the operator trusts forwarded headers (Ruscker behind a reverse
/// proxy), this takes the **right-most** `X-Forwarded-For` entry — the
/// address our own trusted proxy appended (nginx
/// `proxy_add_x_forwarded_for`). The *left-most* entry is whatever the
/// client sent and is trivially spoofable, so keying on it would let a
/// caller dodge the limit by rotating an injected value. The chosen
/// entry must parse as an IP; otherwise we fall back to the TCP peer,
/// and finally to `"unknown"` (e.g. `Router::oneshot` in tests).
///
/// This assumes a single trusted proxy in front (the documented
/// deployment). With a chain of N proxies the real client is N-from-the-
/// right; a configurable trusted-hop count is a future refinement.
fn client_key(state: &AppState, headers: &HeaderMap, peer: Option<&SocketAddr>) -> String {
    if forward_headers_trusted(&state.config.server) {
        if let Some(ip) = headers
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .and_then(rightmost_forwarded_ip)
        {
            return ip.to_string();
        }
    }
    peer.map(|a| a.ip().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

/// The right-most parseable IP in an `X-Forwarded-For` value — the hop
/// our trusted proxy appended. Returns `None` if no entry parses as an
/// IP (so the caller falls back to the TCP peer).
fn rightmost_forwarded_ip(xff: &str) -> Option<std::net::IpAddr> {
    xff.rsplit(',')
        .map(str::trim)
        .find(|p| !p.is_empty())
        .and_then(|p| p.parse().ok())
}

/// Permissive CORS headers for API responses. Origin is `*` (no
/// credentials) — appropriate for a public, token-or-internally-
/// authenticated API where the browser just needs to read the
/// response cross-origin. Never clobbers headers the upstream app
/// already set, so an API that does its own CORS wins.
fn apply_cors_headers(headers: &mut HeaderMap) {
    let defaults: &[(header::HeaderName, &str)] = &[
        (header::ACCESS_CONTROL_ALLOW_ORIGIN, "*"),
        (
            header::ACCESS_CONTROL_ALLOW_METHODS,
            "GET, POST, PUT, PATCH, DELETE, OPTIONS, HEAD",
        ),
        (header::ACCESS_CONTROL_ALLOW_HEADERS, "*"),
        (header::ACCESS_CONTROL_MAX_AGE, "86400"),
    ];
    for (name, value) in defaults {
        headers
            .entry(name.clone())
            .or_insert_with(|| HeaderValue::from_static(value));
    }
}

/// Apply CORS headers to `resp` when `enabled`; otherwise pass it
/// through untouched. Centralises the "API spec + cors: true" check
/// at every exit of `forward`.
fn with_cors(mut resp: Response, enabled: bool) -> Response {
    if enabled {
        apply_cors_headers(resp.headers_mut());
    }
    resp
}

/// Synthetic `204 No Content` response for a CORS preflight
/// (`OPTIONS`) on an API spec — answered by the proxy without ever
/// reaching the upstream container.
fn cors_preflight_response() -> Response {
    let mut resp = StatusCode::NO_CONTENT.into_response();
    apply_cors_headers(resp.headers_mut());
    resp
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
                // Keep the session pinned to its replica while that
                // replica can still serve it: `Ready`, or `Draining`
                // (let an in-flight session finish during shutdown). A
                // Failed/Stopped/Starting one falls through to a fresh
                // pick + new session rather than 502-ing the visitor.
                if let Some(r) = alive {
                    if matches!(r.state, ReplicaState::Ready | ReplicaState::Draining) {
                        return Ok((r, session.session_id, true));
                    }
                }
            }
        }
    }
    let r = pick_or_spawn(state, spec).await?;
    Ok((r, uuid::Uuid::new_v4(), false))
}

/// Build + set the sticky cookie from an explicit `StickySession`
/// — keeps the session_id consistent with what we tracked in the
/// `SessionStore` rather than minting a fresh, untracked id
/// inside the cookie helper.
fn set_sticky_cookie(
    cookies: &Cookies,
    key: &CookieKey,
    session: &StickySession,
    is_https: bool,
) {
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
    // `Secure` only under TLS (X-Forwarded-Proto: https) — the
    // plain-HTTP dev server would otherwise see the browser drop
    // the cookie.
    c.set_secure(is_https);
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
    let routing = spec.effective_routing();

    // Fast path: read lock, no spawn coordination needed. Route to a
    // replica that's actually `Ready` (preferably with a free seat) per
    // the spec's strategy; only fall through to spawn when none is
    // usable — never hand traffic to a Starting/Draining/Failed one.
    {
        let reg = state.replicas.read().await;
        if let Some(r) = pick_replica(reg.replicas_of(&spec.id), routing) {
            return Ok(r);
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
        if let Some(r) = pick_replica(replicas, routing) {
            return Ok(r);
        }
        // A sibling may have spawned one that's still coming up (not yet
        // `Ready`): reuse it rather than spawning a duplicate — that's
        // the whole point of the coalescer.
        if let Some(r) = replicas.first() {
            return Ok(r.clone());
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

    let inner_port = spec.effective_inner_port();

    let creds = resolve_creds(state, spec).await;
    let limits = limits_from_spec(spec);
    tracing::info!(
        spec = %spec.id,
        image,
        inner_port = ?inner_port,
        with_creds = creds.is_some(),
        with_limits = !limits.is_empty(),
        "spawning first replica"
    );
    let mut req = ruscker_core::SpawnRequest::new(&spec.id, image)
        .with_limits(limits)
        .with_volumes(spec.volumes.clone().unwrap_or_default())
        .with_placement(spec.effective_placement())
        .with_anti_affinity(spec.effective_anti_affinity());
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

/// Resolve the registry credentials a spec should pull with,
/// consulting (in priority order):
///
/// 1. **DB credential store** — if the spec sets
///    `docker-registry-credential: "name"` and the admin DB +
///    master key are available, look the name up and decrypt.
///    Keeps secrets entirely out of YAML.
/// 2. **Inline fields** — the env-interpolated
///    `docker-registry-{username,password,domain}` on the spec.
///
/// Async (unlike `creds_from_spec`) because the DB lookup +
/// decrypt is I/O. Both spawn paths call this.
pub(crate) async fn resolve_creds(
    state: &AppState,
    spec: &Spec,
) -> Option<ruscker_core::RegistryCredentials> {
    if let Some(name) = spec
        .docker_registry_credential
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        match (state.db.as_ref(), state.master_key.is_configured()) {
            (Some(pool), true) => {
                match crate::db::credentials::resolve(pool, &state.master_key, name).await {
                    Ok(Some(c)) => return Some(c),
                    Ok(None) => {
                        tracing::warn!(
                            credential = name, spec = %spec.id,
                            "spec references a credential not in the store; \
                             falling back to inline fields"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            credential = name, spec = %spec.id, error = ?e,
                            "failed to resolve stored credential; \
                             falling back to inline fields"
                        );
                    }
                }
            }
            _ => {
                tracing::warn!(
                    credential = name, spec = %spec.id,
                    "spec references a stored credential but DB / master key \
                     is unavailable; falling back to inline fields"
                );
            }
        }
    }
    creds_from_spec(spec)
}

fn pick_index(n: usize) -> usize {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static N: AtomicUsize = AtomicUsize::new(0);
    N.fetch_add(1, Ordering::Relaxed) % n
}

/// Choose a replica to route a (non-sticky) request to, honoring the
/// spec's [`RoutingStrategy`] and the replicas' state/capacity.
///
/// Preference order: a `Ready` replica with a free seat (per strategy);
/// failing that, any `Ready` replica (soft over-subscription — still
/// serves, better than a 503 or routing to a non-`Ready` container; the
/// scaler adds real capacity on sustained saturation). Returns `None`
/// only when no `Ready` replica exists, so the caller spawns.
fn pick_replica(replicas: &[Replica], routing: RoutingStrategy) -> Option<Replica> {
    select(replicas.iter().filter(|r| r.is_accepting()), routing).or_else(|| {
        select(
            replicas.iter().filter(|r| r.state == ReplicaState::Ready),
            routing,
        )
    })
}

/// Pick one replica from `candidates` per `routing`. Round-robin spreads
/// across the candidates; least-connections (and, for now, weighted-
/// random) favor the replica with the most free seats.
fn select<'a>(
    candidates: impl Iterator<Item = &'a Replica>,
    routing: RoutingStrategy,
) -> Option<Replica> {
    let cands: Vec<&Replica> = candidates.collect();
    if cands.is_empty() {
        return None;
    }
    let chosen = match routing {
        RoutingStrategy::RoundRobin => cands[pick_index(cands.len())],
        RoutingStrategy::LeastConnections
        | RoutingStrategy::WeightedRandom
        | RoutingStrategy::ResourceAware => {
            // Most free seats wins; ties break on the first seen.
            cands.iter().copied().max_by_key(|r| r.available_seats())?
        }
    };
    Some(chosen.clone())
}

/// Stamp the smart-routing headers onto an outbound upstream request.
///
/// These let a containerised app figure out the public path / scheme /
/// host it's served behind, so it can build correct links and route
/// its own assets without Ruscker rewriting its HTML:
///
/// - `X-Forwarded-Prefix` — the mount path with no trailing slash
///   (e.g. `/app/my-shiny`). De-facto standard honoured by Spring,
///   Traefik, and FastAPI's `root_path` proxy support.
/// - `X-Script-Name` — the WSGI / Dash / Plumber spelling of the same
///   mount path.
/// - `X-Forwarded-Proto` — `https`/`http` as seen by the *client*
///   (taken from `X-Forwarded-Proto` on the inbound request, or the
///   connection scheme), so the app emits `https://` links behind TLS
///   termination.
/// - `X-Forwarded-Host` — the public `Host` the client used.
///
/// A malformed prefix (one that can't be a header value) is skipped
/// rather than failing the request; the HTML rewriter still covers it.
fn apply_smart_routing_headers(
    headers: &mut HeaderMap,
    forwarded_prefix: &str,
    is_https: bool,
    fwd_host: Option<HeaderValue>,
) {
    if let Ok(v) = HeaderValue::from_str(forwarded_prefix) {
        headers.insert("x-forwarded-prefix", v.clone());
        headers.insert("x-script-name", v);
    }
    headers.insert(
        "x-forwarded-proto",
        HeaderValue::from_static(if is_https { "https" } else { "http" }),
    );
    if let Some(host) = fwd_host {
        headers.insert("x-forwarded-host", host);
    }
}

async fn do_forward(
    replica: &Replica,
    upstream_path: String,
    forwarded_prefix: &str,
    is_https: bool,
    mut req: Request,
    max_body: Option<usize>,
) -> anyhow::Result<Response> {
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let new_uri: Uri = format!("http://{}{}{}", replica.upstream, upstream_path, query)
        .parse()
        .map_err(|e| anyhow::anyhow!("build upstream uri: {e}"))?;
    *req.uri_mut() = new_uri;

    // Capture the client's `Host` before `strip_hop_headers` drops it
    // (it's hop-by-hop for us; hyper re-derives it from the upstream
    // authority). We re-publish it as `X-Forwarded-Host` so the app
    // can build absolute URLs against the public hostname, not the
    // container's internal address.
    let fwd_host = req.headers().get(header::HOST).cloned();

    strip_hop_headers(req.headers_mut());

    // Smart-routing headers — tell the upstream what public prefix,
    // scheme, and host it's mounted behind so it can self-route. Set
    // *after* the hop-by-hop strip (which removes any inbound
    // `X-Forwarded-Proto`) so our values are authoritative.
    apply_smart_routing_headers(req.headers_mut(), forwarded_prefix, is_https, fwd_host);

    // Hard cap on the streamed request body. The `Content-Length`
    // fast-path in `forward` already rejected declared-oversize
    // uploads with a clean 413; this catches a chunked / under-
    // declared body that tries to slip past — it surfaces as a
    // forward error (502) once the limit trips mid-stream.
    if let Some(limit) = max_body {
        let (parts, body) = req.into_parts();
        let limited = http_body_util::Limited::new(body, limit);
        req = Request::from_parts(parts, Body::new(limited));
    }

    let client = http_client().clone();
    let upstream_resp = client.request(req).await?;

    let (parts, body) = upstream_resp.into_parts();
    let body = Body::new(body.map_err(std::io::Error::other));
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
    // Non-standard but widely sent; strip so it can't confuse a
    // request-smuggling-aware upstream.
    "proxy-connection",
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

    // ── pick_replica: state- and seat-aware routing (#76) ───────
    fn rep(state: ReplicaState, active: u32, max: u32) -> Replica {
        Replica {
            id: ruscker_core::ReplicaId(uuid::Uuid::new_v4()),
            spec_id: "s".into(),
            container_id: "c".into(),
            upstream: "127.0.0.1:1".parse().unwrap(),
            state,
            started_at: chrono::Utc::now(),
            sessions_active: active,
            sessions_max: max,
            host: None,
        }
    }

    #[test]
    fn pick_replica_prefers_ready_with_a_free_seat() {
        let reps = vec![
            rep(ReplicaState::Starting, 0, 5), // not ready
            rep(ReplicaState::Ready, 5, 5),    // ready but full
            rep(ReplicaState::Ready, 1, 5),    // ready, has seats ✓
        ];
        let chosen = pick_replica(&reps, RoutingStrategy::LeastConnections).unwrap();
        assert_eq!(chosen.sessions_active, 1);
    }

    #[test]
    fn pick_replica_least_connections_picks_most_free() {
        let reps = vec![
            rep(ReplicaState::Ready, 3, 10), // 7 free
            rep(ReplicaState::Ready, 1, 10), // 9 free ✓
            rep(ReplicaState::Ready, 8, 10), // 2 free
        ];
        let chosen = pick_replica(&reps, RoutingStrategy::LeastConnections).unwrap();
        assert_eq!(chosen.sessions_active, 1);
    }

    #[test]
    fn pick_replica_never_routes_to_a_lone_non_ready_replica() {
        for st in [
            ReplicaState::Starting,
            ReplicaState::Draining,
            ReplicaState::Failed,
            ReplicaState::Stopped,
        ] {
            let reps = vec![rep(st, 0, 5)];
            assert!(
                pick_replica(&reps, RoutingStrategy::LeastConnections).is_none(),
                "must not route a new session to {st:?}"
            );
        }
    }

    #[test]
    fn pick_replica_oversubscribes_a_ready_replica_when_all_full() {
        // All Ready but saturated → still serve via a Ready replica
        // (not None, not the Starting one).
        let reps = vec![
            rep(ReplicaState::Ready, 5, 5),
            rep(ReplicaState::Starting, 0, 5),
        ];
        let chosen = pick_replica(&reps, RoutingStrategy::LeastConnections).unwrap();
        assert_eq!(chosen.state, ReplicaState::Ready);
    }

    #[test]
    fn pick_replica_none_when_no_ready_replica() {
        let reps = vec![
            rep(ReplicaState::Starting, 0, 5),
            rep(ReplicaState::Draining, 0, 5),
        ];
        assert!(pick_replica(&reps, RoutingStrategy::RoundRobin).is_none());
    }

    // #80: the rate-limit client key must come from the right-most XFF
    // entry (the one a trusted proxy appended), not the spoofable left.
    #[test]
    fn rightmost_forwarded_ip_ignores_spoofed_left() {
        assert_eq!(
            rightmost_forwarded_ip("9.9.9.9, 203.0.113.7"),
            Some("203.0.113.7".parse().unwrap())
        );
        // A client-injected left-most value is ignored.
        assert_eq!(
            rightmost_forwarded_ip("evil-spoof, 203.0.113.7"),
            Some("203.0.113.7".parse().unwrap())
        );
        assert_eq!(
            rightmost_forwarded_ip("  10.0.0.1  "),
            Some("10.0.0.1".parse().unwrap())
        );
        // Right-most doesn't parse as an IP → None (caller uses peer).
        assert_eq!(rightmost_forwarded_ip("203.0.113.7, junk"), None);
        assert_eq!(rightmost_forwarded_ip(""), None);
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
                host: None,
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
            admin_sessions: Default::default(),
            log_buffer: None,
            login_limiter: StdArc::new(crate::auth::LoginRateLimiter::default_policy()),
            api_limiter: StdArc::new(crate::ratelimit::ApiRateLimiter::new()),
            db: None,
            images_dir: None,
            master_key: Default::default(),
            backend: Some(backend),
            replicas: StdArc::new(RwLock::new(ReplicaRegistry::new())),
            cookie_key: CookieKey::random(),
            spawn_locks: StdArc::new(dashmap::DashMap::new()),
            sessions: StdArc::new(crate::sessions::InMemorySessionStore::new()),
            leader: StdArc::new(crate::leader::AlwaysLeader),
            metrics: crate::metrics_cache::MetricsCache::new(),
            draining: StdArc::new(std::sync::atomic::AtomicBool::new(false)),
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

    // ── Smart-routing headers (#102) ────────────────────────────────

    #[test]
    fn smart_routing_headers_set_prefix_proto_and_host() {
        let mut h = HeaderMap::new();
        let host = HeaderValue::from_static("portal.example.org");
        apply_smart_routing_headers(&mut h, "/app/my-shiny", true, Some(host));

        assert_eq!(h.get("x-forwarded-prefix").unwrap(), "/app/my-shiny");
        // X-Script-Name carries the same mount path for WSGI/Dash apps.
        assert_eq!(h.get("x-script-name").unwrap(), "/app/my-shiny");
        // is_https = true ⇒ the app should emit https:// links.
        assert_eq!(h.get("x-forwarded-proto").unwrap(), "https");
        assert_eq!(h.get("x-forwarded-host").unwrap(), "portal.example.org");
    }

    #[test]
    fn smart_routing_proto_is_http_when_not_https() {
        let mut h = HeaderMap::new();
        apply_smart_routing_headers(&mut h, "/api/data", false, None);
        assert_eq!(h.get("x-forwarded-proto").unwrap(), "http");
        // No inbound Host ⇒ we don't fabricate X-Forwarded-Host.
        assert!(h.get("x-forwarded-host").is_none());
    }

    #[test]
    fn smart_routing_overwrites_inbound_proto() {
        // A client (or upstream proxy) that sent its own forwarded
        // headers must not win — Ruscker is the trust boundary here.
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-proto", HeaderValue::from_static("https"));
        apply_smart_routing_headers(&mut h, "/app/x", false, None);
        assert_eq!(h.get("x-forwarded-proto").unwrap(), "http");
        // insert (not append) ⇒ exactly one value, no smuggling.
        assert_eq!(h.get_all("x-forwarded-proto").iter().count(), 1);
    }

    #[test]
    fn inject_base_href_defaults_on_and_honours_false() {
        // Default (unset) ⇒ HTML rewriting stays on.
        assert!(fake_spec("x").effective_inject_base_href());
        // Explicit opt-out ⇒ off.
        let off = spec_yaml(
            r#"
id: selfrouting
display-name: Self
container-image: x:1
inject-base-href: false
"#,
        );
        assert!(!off.effective_inject_base_href());
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
