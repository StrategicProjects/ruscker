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
use hyper_util::rt::{TokioExecutor, TokioTimer};
use ruscker_config::{RoutingStrategy, Spec, SpecKind};
use ruscker_core::{Replica, ReplicaState};
use ruscker_proxy::sticky::{self, CookieKey, StickySession, COOKIE_NAME};
use ruscker_proxy::ws;
use std::net::SocketAddr;
use std::sync::{Arc, OnceLock};
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

/// Authenticated identity resolved once for this proxied request. Group
/// membership is an `Arc` because cache hits should not clone the vector
/// for every asset in a page load (#1001).
struct Identity {
    username: String,
    groups: Arc<Vec<String>>,
}

impl Identity {
    fn header_pairs(&self) -> Vec<(String, String)> {
        vec![
            ("X-SP-UserId".into(), self.username.clone()),
            ("X-SP-UserGroups".into(), self.groups.join(",")),
        ]
    }
}

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
    CLIENT.get_or_init(|| {
        Client::builder(TokioExecutor::new())
            // Interactive app servers (RStudio Server, Shiny) close idle
            // keep-alive connections aggressively — often within a few
            // seconds. hyper's default 90 s pool idle timeout keeps those
            // dead sockets and then dispatches the next request onto one,
            // which surfaces as `client error (SendRequest)` → a spurious
            // "upstream error" on the visitor's first navigation (it works
            // on retry, since that opens a fresh connection). Evicting
            // pooled idle connections quickly keeps the pool fresh; the
            // per-request retry in `do_forward` covers the residual race.
            .pool_idle_timeout(std::time::Duration::from_secs(10))
            .pool_timer(TokioTimer::new())
            .build_http()
    })
}

// ── Path-strip handlers ───────────────────────────────────────────

/// User-visible URL prefix for the two route families. Threaded
/// through to the core forward handler so the HTML rewriter knows
/// what to point `<base href>` at.
const APP_PREFIX: &str = "/app/";
const API_PREFIX: &str = "/api/";

/// In-flight request count per replica (#336). API specs have no sticky
/// sessions, so their capacity is request-based, not seat-based: this
/// gauge lets the scaler treat concurrent in-flight requests the way it
/// treats sessions for interactive apps. A process-global (like the
/// scaler's failure log) so the proxy hot path and the scaler share it
/// without threading a new field through `AppState`; keyed by replica
/// id, each op takes only that shard's lock.
static INFLIGHT: std::sync::LazyLock<dashmap::DashMap<ruscker_core::ReplicaId, u32>> =
    std::sync::LazyLock::new(dashmap::DashMap::new);

/// RAII guard: bumps a replica's in-flight count on creation and drops
/// it on `Drop`, covering every return/error path of the forward.
pub(crate) struct InflightGuard(ruscker_core::ReplicaId);

impl InflightGuard {
    pub(crate) fn new(replica_id: ruscker_core::ReplicaId) -> Self {
        *INFLIGHT.entry(replica_id.clone()).or_insert(0) += 1;
        Self(replica_id)
    }
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        if let Some(mut v) = INFLIGHT.get_mut(&self.0) {
            *v = v.saturating_sub(1);
        }
    }
}

/// Current in-flight request count for a replica (0 if untracked).
pub(crate) fn inflight_count(replica_id: &ruscker_core::ReplicaId) -> u32 {
    INFLIGHT.get(replica_id).map(|v| *v).unwrap_or(0)
}

/// Forget in-flight entries for replicas that have left the registry —
/// called by the scaler's per-tick GC so the map can't grow unbounded.
pub(crate) fn inflight_gc(alive: &std::collections::HashSet<ruscker_core::ReplicaId>) {
    INFLIGHT.retain(|rid, _| alive.contains(rid));
}

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
    mut req: Request,
) -> Response {
    // Whether the operator opted into trusting X-Forwarded-* (#328);
    // gates both the scheme below and the X-Forwarded-For handling in
    // the forward (#744).
    let xfwd_trusted = forward_headers_trusted(&state.config.server);
    // Capture the request scheme before `req` is consumed by the
    // forward — used to decide the `Secure` flag on the sticky
    // cookie we may set at the end. Same trust gate as every other
    // forwarded header (#744): without the opt-in, a direct client
    // could flip the cookie's `Secure` flag with a spoofed header.
    let is_https = crate::auth::request_is_https(req.headers(), xfwd_trusted);

    // Signed-in username (if any), captured before the access-guard below
    // consumes `session.0`. Used to index the session for `stop-on-logout`
    // (#337). A borrow + clone — leaves `session` intact for the guard.
    let acting_user: Option<String> = session.0.as_ref().and_then(|s| s.actor.clone());

    // 1. Find the spec. DB-first when attached (covers the showcase
    // seed + operator edits via the admin), falling back to the YAML
    // `proxy.specs` so deployments that load specs exclusively from
    // the config file keep working. Matches the landing handler's
    // spec source rule.
    let Some(spec) = find_spec(&state, &spec_id).await else {
        return (StatusCode::NOT_FOUND, format!("spec `{spec_id}` not found")).into_response();
    };

    // 2. External link: bounce.
    if spec.kind() == SpecKind::External {
        if let Some(target) = spec.template_properties.get_str("link") {
            // Count the external-card click (#549). The landing routes
            // external cards through `/app/{id}` precisely so this click
            // is visible to Ruscker. Best-effort — never blocks the bounce.
            record_access(&state, &spec.id);
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
    //     hit on `/app/{spec}`. Group membership comes from the same users
    //     store as the landing, through the identity hot-path cache (#1001).
    //
    // Resolve identity once when either access control or upstream header
    // disclosure needs it. Open specs with disclosure disabled do no user
    // lookup at all.
    let identity = match acting_user.as_ref() {
        Some(username) if !spec.is_open() || spec.effective_add_default_http_headers() => {
            Some(Identity {
                username: username.clone(),
                groups: find_user_groups(&state, username).await,
            })
        }
        _ => None,
    };
    if !spec.is_open() {
        let is_admin = session
            .0
            .as_ref()
            .map(|s| s.role == crate::auth::Role::Admin)
            .unwrap_or(false);
        let groups = identity
            .as_ref()
            .map_or(&[][..], |identity| identity.groups.as_slice());
        if !spec.access_allows(is_admin, acting_user.as_deref(), groups) {
            tracing::info!(
                spec = %spec.id,
                user = acting_user.as_deref().unwrap_or("-"),
                "access denied to restricted spec"
            );
            // An anonymous visitor hitting a restricted interactive app
            // is sent to log in; everyone else (and all API clients) get
            // a flat 403 (CORS-wrapped for the `/api/` family).
            if route_prefix == APP_PREFIX && session.0.is_none() {
                // Proxy routes are not wrapped by the chrome's
                // `prefix_base_path` Location-rewriter, so build the
                // base-prefixed login URL ourselves (#294) — otherwise a
                // `/box/app/<spec>` visitor is bounced to a `/admin/login`
                // that 404s outside the mount.
                // `nest` has already stripped `state.base_path` from this
                // URI. Carry its untouched path-and-query rather than the
                // decoded `Path` extractors, preserving percent-encoding.
                let next = req
                    .uri()
                    .path_and_query()
                    .map(|value| value.as_str())
                    .unwrap_or_else(|| req.uri().path());
                let login = super::with_next_query(
                    &format!("{}/admin/login", state.base_path),
                    next,
                );
                return Redirect::to(&login).into_response();
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

    // 3b. Cold-start splash (interactive apps only). A top-level
    //     navigation to an app whose container isn't up yet gets an
    //     elegant "starting…" interstitial that polls readiness, rather
    //     than a blank wait through the whole image pull + boot. The
    //     real spawn runs in the background (coalesced); the next
    //     navigation, once a replica is `Ready`, proxies normally.
    if route_prefix == APP_PREFIX && ws_upgrade.0.is_none() {
        // Tiny readiness probe the splash polls. Never spawns or blocks.
        // MUST use the same readiness test as the splash *gate* below
        // (`has_ready_replica`) — otherwise a ready-but-full `seats: 1` app
        // makes the probe say "ready" while the gate keeps re-serving the
        // splash, and the page reload-loops forever. The probe therefore
        // advances only when there's a replica that can actually accept the
        // visitor (Ready + free seat); cold start has a free seat, so it
        // advances normally, and a full app correctly keeps waiting while
        // the proxy path scales out (#582 follow-up).
        if upstream_path == COLD_PROBE_PATH {
            // Advance when a fresh visitor could be admitted (Ready + free
            // seat) OR when *this* session already holds a seat on a live
            // replica — otherwise a single-seat app that already reserved
            // its seat for this session would report "not ready" to its own
            // splash forever (see `has_sticky_seat`).
            let ready = has_ready_replica(&state, &spec).await
                || has_sticky_seat(&state, &spec, &cookies).await;
            let body = if ready {
                "{\"ready\":true}"
            } else {
                "{\"ready\":false}"
            };
            return (
                [
                    (header::CONTENT_TYPE, "application/json"),
                    (header::CACHE_CONTROL, "no-store"),
                ],
                body,
            )
                .into_response();
        }
        // Cold navigation → show the interstitial. If the spec can still
        // scale, kick the spawn off in the background ("Starting…"); if it's
        // already at its replica ceiling with no free seat, a spawn won't
        // help, so show the "full — waiting for a slot" copy instead and
        // don't spawn. Both poll readiness and open the app once a seat is
        // free. (Saturated-but-warm specs still serve immediately.)
        if *req.method() == Method::GET
            && wants_html(req.headers())
            && !has_ready_replica(&state, &spec).await
            // A returning session whose replica is already serving it keeps
            // proxying — never bounce it back to the splash (#623 cast fix).
            && !has_sticky_seat(&state, &spec, &cookies).await
        {
            let full = at_capacity(&state, &spec).await;
            if !full {
                let (st, sp) = (state.clone(), spec.clone());
                tokio::spawn(async move {
                    if let Err(e) = crate::scaler::ensure_replica_available(&st, &sp).await {
                        tracing::warn!(spec = %sp.id, error = ?e, "background spawn (splash) failed");
                    }
                });
            }
            return cold_start_splash(&spec, &state.base_path, full);
        }
    }

    // 4. Resolve the replica: sticky-first, fall back to
    //    pick/spawn. Also pin down the session_id we'll track
    //    this visitor under so the cookie and the tracker share
    //    the same identity.
    // A "visit" is a top-level document navigation (Accept: text/html).
    // Only a visit opens a session; subresources/XHR/WS without a sticky
    // cookie ride an existing replica without counting (#623 cast leak).
    let is_visit = wants_html(req.headers());
    let (replica, session_id, cookie_used, seat_reserved, track_session) =
        match resolve_replica(&state, &spec, &cookies, is_visit).await {
            Ok(quad) => quad,
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
    //     `sessions_active` for them would mislead the scaler. Skip
    //     untracked subresource forwards (`track_session == false`).
    if track_session && spec_kind_needs_sticky(spec.kind()) {
        let outcome = state
            .sessions
            .touch_or_register(&state.replicas, session_id, &spec.id, &replica.id, seat_reserved)
            .await;
        // stop-on-logout (#337): when a *known* user first registers a
        // session on a spec that opts in, index it so their logout can
        // end it immediately. Only on first registration and only for
        // such specs — no cost on the common path.
        if outcome == crate::sessions::TouchOutcome::Registered && spec.effective_stop_on_logout() {
            if let Some(user) = &acting_user {
                state
                    .logout_index
                    .entry(user.clone())
                    .or_default()
                    .insert(session_id);
            }
        }
    }

    // 5. WebSocket branch hijacks the upgrade and pumps frames;
    //    after the upgrade response is sent, the rest of axum's
    //    response pipeline ignores anything we'd add (cookies
    //    can't be set on a 101). Issuing the sticky cookie on the
    //    preceding HTTP request is how WS-only apps stay sticky.
    if let MaybeWs(Some(upgrade)) = ws_upgrade {
        // The query string must ride along (#730): Jupyter kernel channels
        // (`/api/kernels/<id>/channels?session_id=…`), SockJS cache-busters
        // and any app keying WS reconnection on query params depend on it.
        // Mirrors what `do_forward` does for the HTTP path.
        let query = req
            .uri()
            .query()
            .map(|q| format!("?{q}"))
            .unwrap_or_default();
        let upstream_ws_url = format!("ws://{}{}{}", replica.upstream, upstream_path, query);
        // Forward the client's *app* cookies and requested subprotocol
        // onto the upstream handshake so the app keeps its session — but
        // strip Ruscker's own cookies first (#258). The HTTP path strips
        // these in `do_forward`; the WS upgrade bypasses that, so without
        // this the admin session id (a bearer) leaks to the app container
        // over the WS handshake — the most-used transport for Shiny/
        // Jupyter/RStudio.
        // The WS connector only forwards explicitly selected headers, but
        // still scrub the inbound map before building that handshake so the
        // reserved namespace has one unconditional rule across transports.
        strip_reserved_identity_headers(req.headers_mut());
        let cookie = req
            .headers()
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .and_then(filter_ruscker_cookies);
        let subprotocols = req
            .headers()
            .get(header::SEC_WEBSOCKET_PROTOCOL)
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        tracing::debug!(
            spec = %spec.id, replica = %replica.id, url = %upstream_ws_url,
            "ws upgrade"
        );
        // Connect the upstream BEFORE answering the client's 101 (#730):
        // a dead replica then gets the client a real 502 instead of an
        // opaque post-upgrade 1006 drop, and the subprotocol the upstream
        // selected can be echoed on our 101 — a browser that offered
        // subprotocols and receives a 101 without one selected must fail
        // the connection (RFC 6455 §4.1).
        let identity_headers = if spec.effective_add_default_http_headers() {
            identity
                .as_ref()
                .map(Identity::header_pairs)
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let handshake = match ws::connect_with_headers(
            &upstream_ws_url,
            cookie.as_deref(),
            subprotocols.as_deref(),
            &identity_headers,
        )
        .await
        {
            Ok(h) => h,
            Err(err) => {
                tracing::error!(
                    spec = %spec.id, replica = %replica.id, error = ?err,
                    "upstream ws connect failed"
                );
                return with_cors(
                    (StatusCode::BAD_GATEWAY, "upstream unavailable").into_response(),
                    cors_on,
                );
            }
        };
        let upgrade = match handshake.selected_protocol.clone() {
            // axum echoes a protocol only if the client offered it — which
            // it did, since the upstream chose from the client's own list.
            Some(proto) => upgrade.protocols([proto]),
            None => upgrade,
        };
        let pump_spec = spec.id.clone();
        let pump_replica = replica.id.to_string();
        return upgrade.on_upgrade(move |socket| {
            ws::pump_with_context(socket, handshake.stream, pump_spec, pump_replica)
        });
    }

    // 6. HTTP forward.
    // The mount prefix we advertise to the upstream via
    // `X-Forwarded-Prefix` / `X-Script-Name` / `X-RStudio-Root-Path` —
    // the *public* path the spec is reachable at, with no trailing
    // slash (`route_prefix` already carries one), e.g. `/app/my-shiny`
    // or `/api/my-api`. Must include the base path (#173): under
    // `--base-path /box` the spec lives at `/box/app/my-shiny`, and an
    // app that builds its own URLs from this header (RStudio via
    // `X-RStudio-Root-Path`, Jupyter via `X-Script-Name`) would
    // otherwise emit `/app/...` links/redirects that 404 behind the
    // base-path reverse proxy. Mirrors `inject_base_href`'s `base`.
    let forwarded_prefix = mount_prefix(&state.base_path, route_prefix, &spec.id);
    tracing::debug!(
        spec = %spec.id, replica = %replica.id,
        upstream = %replica.upstream, path = %upstream_path,
        prefix = %forwarded_prefix,
        "forwarding"
    );
    // API capacity is request-based, not session-based (#336): count this
    // request as in-flight on the replica. The guard is moved INTO the
    // response body on the success path below, so it only drops when the
    // (possibly streaming) body finishes — not when this handler returns
    // (#424). Early error returns drop it here, which is correct (no body
    // to meter). Only API specs — interactive apps meter via sticky sessions.
    let inflight =
        (route_prefix == API_PREFIX).then(|| InflightGuard::new(replica.id.clone()));
    // Whether 6b below will transform HTML bodies. Decided here because
    // the *request* must match: when we transform, the upstream must not
    // compress — nothing decompresses in between, so a gzip/br HTML body
    // would reach the rewriter as opaque bytes: no `<base href>`, no
    // runtime shim, and lol_html mutating compressed bytes corrupts the
    // stream outright (#732). `identity` asks the app for plain bytes
    // (ShinyProxy does the same). Non-transformed routes (the `/api/`
    // family, `inject-base-href: false`) keep end-to-end compression.
    let transform_html = route_prefix == APP_PREFIX && spec.effective_inject_base_href();
    if transform_html {
        req.headers_mut().insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
    }
    // X-Forwarded-For (#744): apps behind a proxy expect the standard
    // chain, and before this the client's header passed through
    // VERBATIM with the real peer never appended — upstream apps that
    // log or trust XFF saw spoofable data and never the true client.
    // Trusted mode (server.useForwardHeaders) appends the peer to the
    // inbound chain; untrusted mode replaces the (spoofable) inbound
    // value with just the peer.
    {
        let peer_ip = peer.map(|p| p.ip().to_string());
        let existing = req
            .headers()
            .get("x-forwarded-for")
            .and_then(|v| v.to_str().ok())
            .map(String::from);
        let new_xff = match (peer_ip, existing) {
            (Some(ip), Some(chain)) if xfwd_trusted => Some(format!("{chain}, {ip}")),
            (Some(ip), _) => Some(ip),
            // No peer (Router::oneshot tests): keep a trusted chain,
            // drop an untrusted one.
            (None, Some(chain)) if xfwd_trusted => Some(chain),
            (None, _) => None,
        };
        match new_xff.and_then(|v| HeaderValue::from_str(&v).ok()) {
            Some(v) => {
                req.headers_mut().insert("x-forwarded-for", v);
            }
            None => {
                req.headers_mut().remove("x-forwarded-for");
            }
        }
    }
    let resp = match do_forward(
        &replica,
        upstream_path,
        &forwarded_prefix,
        is_https,
        req,
        body_cap,
        if spec.effective_add_default_http_headers() {
            identity.as_ref()
        } else {
            None
        },
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
    let resp = if transform_html {
        // Include the portal base path (#173) so the app's `<base href>`
        // is `/box/app/{spec}/` when Ruscker is mounted under `/box`.
        let base = format!("{}{route_prefix}{}/", state.base_path, spec.id);
        // Pass the container's internal authority so the rewriter can
        // map any `http://{upstream}/…` self-URLs the app leaks back to
        // the public mount (#373, FastAPI url_for).
        rewrite::inject_base_href(resp, &base, &replica.upstream.to_string()).await
    } else {
        resp
    };

    // 7. Issue sticky cookie when we just bound the visitor to a
    //    replica and the spec actually benefits from stickiness.
    //    The cookie carries the exact session_id we registered
    //    in the tracker, so subsequent requests touch the same
    //    entry instead of registering a duplicate.
    if !cookie_used && spec_kind_needs_sticky(spec.kind()) {
        // A new sticky session = a new app visit (#549). Counting here
        // (not per request) means assets/WebSocket/polling don't inflate
        // it, and direct `/app/{id}` URLs (no landing) still count.
        record_access(&state, &spec.id);
        let session = StickySession {
            session_id,
            spec_id: spec.id.clone(),
            replica_id: replica.id.clone(),
        };
        set_sticky_cookie(&cookies, &state.cookie_key, &session, &forwarded_prefix, is_https);
    }

    // Expire a legacy global sticky cookie (pre-#731 `Path=/`) when the
    // browser still carries one: nothing reads it anymore, and it would
    // otherwise ride along on every portal request for up to 8h.
    if cookies.get(COOKIE_NAME).is_some() {
        let mut dead = Cookie::new(COOKIE_NAME, "");
        dead.set_path("/");
        cookies.remove(dead);
    }

    // API specs aren't sticky, so count each forwarded call here (#549
    // follow-up). One per request — for an API, each call *is* the
    // access. A synchronous in-memory bump (#944): the per-request
    // `tokio::spawn` from #744 kept the task and write rate equal to
    // the request rate; now the drain task batches everything into one
    // UPSERT per flush window.
    if spec.kind() == SpecKind::Api {
        record_access(&state, &spec.id);
    }

    let resp = with_cors(resp, cors_on);
    // Keep the in-flight count up until the response body is fully streamed
    // to the client, not just until this handler returns (#424).
    match inflight {
        Some(guard) => attach_inflight_to_body(resp, guard),
        None => resp,
    }
}

/// Wrap a response body so `guard` only drops once the body is fully
/// consumed/dropped — keeping the replica's in-flight count accurate for
/// long downloads/streams (#424).
fn attach_inflight_to_body(resp: Response, guard: InflightGuard) -> Response {
    use futures_util::StreamExt;
    let (parts, body) = resp.into_parts();
    // Tie the guard's lifetime to the stream: it's captured in the closure
    // state and dropped when the stream (hence the body) is dropped.
    let guarded = body.into_data_stream().map(move |chunk| {
        let _ = &guard; // keep `guard` owned by the stream
        chunk
    });
    Response::from_parts(parts, Body::from_stream(guarded))
}

fn spec_kind_needs_sticky(kind: SpecKind) -> bool {
    matches!(kind, SpecKind::Shiny | SpecKind::InteractiveApp)
}

/// Best-effort per-spec access count (#549). Since #944 this is a plain
/// in-memory bump — no task, no DB round-trip, nothing that could break
/// the request being served. A single drain task (started in
/// `AdminServer::run`) batches the deltas into the DB; without a DB
/// there's no drain task, so skip the bump instead of buffering counts
/// that would never land anywhere.
fn record_access(state: &AppState, spec_id: &str) {
    if state.db.is_some() {
        state.access_counter.bump(spec_id);
    }
}

// ── API policy helpers (rate limit + CORS) ─────────────────────────

/// Whether to believe an inbound `X-Forwarded-For` header. We only
/// do when the operator opted into forwarded headers (ShinyProxy's
/// `server.useForwardHeaders`, or a `forward-headers-strategy` other
/// than `none`). Without that opt-in, a direct client could spoof
/// the header to dodge a per-IP rate limit — so we ignore it and key
/// on the real TCP peer instead.
pub(crate) fn forward_headers_trusted(server: &ruscker_config::Server) -> bool {
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

/// The public mount prefix advertised to the upstream via
/// `X-Forwarded-Prefix` / `X-Script-Name` / `X-RStudio-Root-Path`.
///
/// `base_path` is the portal's base path (`""` or e.g. `/box`),
/// `route_prefix` carries its own trailing slash (`/app/` or `/api/`).
/// The result has no trailing slash, e.g. `/box/app/my-shiny`. Apps
/// that build their own absolute URLs from these headers (RStudio,
/// Jupyter) rely on the base path being present, or their links 404
/// behind a base-path reverse proxy (#173).
fn mount_prefix(base_path: &str, route_prefix: &str, spec_id: &str) -> String {
    format!("{base_path}{route_prefix}{spec_id}")
}

/// Token, substituted at spawn with the spec's [`public_path`], that an
/// operator (or the showcase seed) can drop into `container-cmd` /
/// `container-env` so a path-sensitive app can self-route behind
/// `--base-path` (#371). Ruscker's analog of ShinyProxy's
/// `#{proxy.getRuntimeValue('SHINYPROXY_PUBLIC_PATH')}`. Distinct from the
/// parse-time `${VAR}` env interpolation, which never sees this runtime
/// value. Example: Jupyter `--ServerApp.base_url=#{publicPath}`.
pub(crate) const PUBLIC_PATH_TOKEN: &str = "#{publicPath}";

/// The public mount path a spec is reachable at, **with** a trailing
/// slash (e.g. `/box/app/jupyter/`) — what an app should use as its
/// base-url so the URLs it emits and the paths it receives line up with
/// what the proxy forwards. Derived from [`mount_prefix`] + the spec's
/// route family (`/api/` for APIs, else `/app/`).
pub(crate) fn public_path(base_path: &str, spec: &Spec) -> String {
    let route_prefix = match spec.kind() {
        ruscker_config::SpecKind::Api => API_PREFIX,
        _ => APP_PREFIX,
    };
    format!("{}/", mount_prefix(base_path, route_prefix, &spec.id))
}

/// Resolve the [`PUBLIC_PATH_TOKEN`] in a spec's env (`NAME=value`) and
/// cmd argv at spawn — an **explicit opt-in** for an operator who needs
/// to feed the public mount path to an app.
///
/// It deliberately does **NOT** auto-inject `SHINYPROXY_PUBLIC_PATH`:
/// Ruscker **strips** the mount prefix before forwarding (the container
/// receives `/lab/...`, not `/box/app/jupyter/...` — proven live), the
/// opposite of ShinyProxy's no-strip model. Advertising the public path
/// to a ShinyProxy-style demo (which reads `SHINYPROXY_PUBLIC_PATH` to
/// self-prefix) made it configure the WRONG prefix and 404/500 every
/// request (Jupyter #371, Dash #372). Apps should serve at root; the
/// `/app` rewriter + the #348 jupyter-config rewrite handle the
/// browser-side prefixing. The token stays for the rare case an operator
/// genuinely wants it.
pub(crate) fn apply_public_path(env: &mut [String], cmd: &mut Option<Vec<String>>, path: &str) {
    for e in env.iter_mut() {
        if e.contains(PUBLIC_PATH_TOKEN) {
            *e = e.replace(PUBLIC_PATH_TOKEN, path);
        }
    }
    if let Some(args) = cmd {
        for a in args.iter_mut() {
            if a.contains(PUBLIC_PATH_TOKEN) {
                *a = a.replace(PUBLIC_PATH_TOKEN, path);
            }
        }
    }
}

/// How long a resolved spec stays cached on the proxy hot path (#587).
/// Short enough that an admin edit (including an access-control change)
/// takes effect within this window without explicit invalidation, long
/// enough that one page load's burst of subresource requests all hit the
/// cache instead of the DB.
pub(crate) const SPEC_CACHE_TTL: std::time::Duration = std::time::Duration::from_secs(1);

/// Group memberships change far less often than proxy requests. A 30s TTL
/// prevents one SELECT per asset while bounding how long an admin edit can
/// take to reach access checks and injected identity headers (#1001).
pub(crate) const IDENTITY_CACHE_TTL: std::time::Duration =
    std::time::Duration::from_secs(30);

async fn find_user_groups(state: &AppState, username: &str) -> Arc<Vec<String>> {
    if let Some(groups) = state.identity_cache.get(username, IDENTITY_CACHE_TTL) {
        return groups;
    }

    // Snapshot BEFORE the read: `store` rejects this fill if an admin
    // mutation invalidated the cache while we were at the DB, so a
    // pre-revocation read can never repopulate the cache (#1001).
    let generation = state.identity_cache.generation();
    let groups = match state.db.as_ref() {
        Some(db) => match crate::db::users::fetch(db, username).await {
            Ok(row) => Arc::new(row.map(|user| user.groups).unwrap_or_default()),
            Err(error) => {
                tracing::warn!(user = username, error = ?error, "identity lookup failed");
                return Arc::new(Vec::new());
            }
        },
        None => Arc::new(Vec::new()),
    };
    state.identity_cache.store(generation, username, groups.clone());
    groups
}

pub(crate) async fn find_spec(state: &AppState, id: &str) -> Option<Spec> {
    // Hot path: `find_spec` runs on every proxied request. Serve a recent
    // resolution from the in-memory cache to skip a DB SELECT +
    // config_json parse; the TTL bounds staleness (#587).
    if let Some(entry) = state.spec_cache.get(id) {
        if entry.1.elapsed() < SPEC_CACHE_TTL {
            return Some((*entry.0).clone());
        }
    }
    let resolved = load_spec(state, id).await;
    match &resolved {
        // Cache only positives, so the map stays bounded by the real
        // catalog; refresh the timestamp on every (re)load.
        Some(spec) => {
            state
                .spec_cache
                .insert(id.to_string(), (std::sync::Arc::new(spec.clone()), std::time::Instant::now()));
        }
        // A now-missing spec (deleted/renamed): drop any stale entry so
        // the cache doesn't pin a removed spec.
        None => {
            state.spec_cache.remove(id);
        }
    }
    resolved
}

/// Resolve a spec by id without the cache: DB-first — the operator-
/// editable catalog (admin UI + showcase seed) shadows the YAML for
/// matching ids — then the YAML `--config`. A single indexed SELECT by
/// primary key.
async fn load_spec(state: &AppState, id: &str) -> Option<Spec> {
    if let Some(db) = state.db.as_ref() {
        match crate::db::specs::fetch_one(db, id).await {
            Ok(Some(spec)) => return Some(spec),
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(error = ?err, spec_id = id, "spec DB lookup failed; falling back to YAML");
            }
        }
    }
    state
        .config
        .proxy
        .specs
        .iter()
        .find(|s| s.id == id)
        .cloned()
}

/// Returns the chosen `Replica`, the session_id we'll track this
/// visitor under (either decoded from a valid cookie or freshly
/// minted), and whether the cookie was honored (so the caller
/// only sets a Set-Cookie header on fresh sessions).
async fn resolve_replica(
    state: &AppState,
    spec: &Spec,
    cookies: &Cookies,
    is_visit: bool,
) -> anyhow::Result<(Replica, uuid::Uuid, bool, bool, bool)> {
    if let Some(raw) = cookies.get(&sticky::cookie_name(&spec.id)) {
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
                        // Existing session — its seat is already counted, so
                        // nothing is reserved here (4th = false); track=true
                        // keeps it alive (touch).
                        return Ok((r, session.session_id, true, false, true));
                    }
                }
            }
        }
    }
    // No usable sticky cookie. A request that is NOT a top-level document
    // navigation (a subresource / XHR / WebSocket / asset — e.g. a
    // `crossorigin` JS bundle that drops the cookie) is part of some
    // existing visit, not a new one. Forward it to an existing replica
    // WITHOUT minting a session or reserving a seat, so those requests
    // don't inflate `sessions_active` (the cast RStudio/Jupyter "7/1, 9/1
    // climbing" leak). Only a real visit (the document) opens a session.
    if !is_visit {
        let routing = spec.effective_routing();
        let api = matches!(spec.kind(), SpecKind::Api);
        let reg = state.replicas.read().await;
        if let Some(r) = pick_replica(reg.replicas_of(&spec.id), routing, api) {
            // cookie_used=true → don't issue a Set-Cookie; track=false →
            // don't register/count a session.
            return Ok((r, uuid::Uuid::new_v4(), true, false, false));
        }
        // No replica yet (an asset somehow raced ahead of the document):
        // fall through and spawn, treating it as a visit.
    }
    // New visit: `pick_or_spawn` already reserved its seat (when it
    // returns `reserved = true`), so the session tracker must not re-count.
    let (r, reserved) = pick_or_spawn(state, spec).await?;
    Ok((r, uuid::Uuid::new_v4(), false, reserved, true))
}

/// Build + set the sticky cookie from an explicit `StickySession`
/// — keeps the session_id consistent with what we tracked in the
/// `SessionStore` rather than minting a fresh, untracked id
/// inside the cookie helper.
fn set_sticky_cookie(
    cookies: &Cookies,
    key: &CookieKey,
    session: &StickySession,
    mount_path: &str,
    is_https: bool,
) {
    let value = match sticky::encode(key, session) {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(error = ?err, "encode sticky cookie failed");
            return;
        }
    };
    // One cookie per spec, scoped to the app's own mount (#731). A
    // single `Path=/` cookie made two apps in the same browser fight
    // over it: opening B overwrote A's session, orphaning A's seat
    // (the splash then told A's user the app was full — on a seat THEY
    // held) and sending B's cookie on A's subresources broke multi-
    // replica stickiness. Per-spec name + path means each app keeps
    // its own session and the cookie never even travels cross-app.
    // `mount_path` carries no trailing slash (`/box/app/my-shiny`), so
    // it path-matches `/box/app/my-shiny` and everything below it.
    let mut c = Cookie::new(sticky::cookie_name(&session.spec_id), value);
    c.set_path(mount_path.to_string());
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

// ── Cold-start splash (#…) ─────────────────────────────────────────

/// Reserved `/app/{spec}` sub-path the splash polls for readiness.
const COLD_PROBE_PATH: &str = "/__ruscker_ready";

/// Whether `spec` has a replica that can **accept a new session right
/// now** (Ready with a free seat) — no spawn. Strict (`pick_accepting`):
/// a full replica returns `false`, so the splash *gate* scales out instead
/// of routing the visitor onto an over-subscribed `seats-per-container`
/// replica (#582).
/// Used by **both** the cold-start splash gate and the readiness probe, so
/// they always agree on "is the app serveable to this visitor now?" — a
/// Ready replica with a free seat. They MUST share this: a probe that
/// advanced on a full replica (an earlier attempt used a seat-agnostic
/// pick) made the splash reload into the gate, which re-served the splash,
/// looping forever on a busy `seats: 1` app.
async fn has_ready_replica(state: &AppState, spec: &Spec) -> bool {
    let routing = spec.effective_routing();
    let reg = state.replicas.read().await;
    pick_accepting(reg.replicas_of(&spec.id), routing, matches!(spec.kind(), SpecKind::Api)).is_some()
}

/// True when the visitor already holds a sticky session pinned to a live
/// (`Ready`/`Draining`) replica for this spec — i.e. the app is already up
/// and serving *them*.
///
/// The cold-start splash must NOT fire for such a request. `has_ready_replica`
/// requires a *free* seat, so once a single-seat app (`seats: 1`, the default
/// for RStudio/Shiny/Jupyter) reserves its only seat for this very session,
/// the app's own follow-up navigations — RStudio's redirect to
/// `/auth-sign-in`, Jupyter's to `/lab` — would hit the gate, find no free
/// seat, be handed the splash again, and loop forever even though the
/// session's replica is ready. Bypassing the splash when the session already
/// has its seat fixes that (mirrors the sticky-first branch of
/// `resolve_replica`; read-only, reserves nothing).
async fn has_sticky_seat(state: &AppState, spec: &Spec, cookies: &Cookies) -> bool {
    let Some(raw) = cookies.get(&sticky::cookie_name(&spec.id)) else {
        return false;
    };
    let Ok(session) = sticky::decode(&state.cookie_key, raw.value()) else {
        return false;
    };
    let reg = state.replicas.read().await;
    sticky_replica_is_live(&reg, &spec.id, &session)
}

/// True when the spec is already at its replica ceiling — a spawn can't add
/// capacity (mirrors `spawn_one`'s `live >= max` cap). Combined with "no
/// accepting replica", it means every seat is taken and the visitor is
/// waiting for one to *free*, not for a container to boot — so the
/// interstitial says "full" instead of "starting" (#623).
async fn at_capacity(state: &AppState, spec: &Spec) -> bool {
    let max = spec.effective_max_replicas() as usize;
    let reg = state.replicas.read().await;
    // Count only replicas a spawn would actually compete with —
    // mirroring `spawn_one`'s notion of "live" (#744). Counting
    // Failed/Stopped leftovers made the splash say "full — waiting for
    // a slot" and skip the background spawn while `pick_or_spawn`'s own
    // capped branch would happily have spawned over them.
    reg.replicas_of(&spec.id)
        .iter()
        .filter(|r| {
            matches!(
                r.state,
                ReplicaState::Starting | ReplicaState::Ready | ReplicaState::Draining
            )
        })
        .count()
        >= max
}

/// The post-decode core of [`has_sticky_seat`]: does this sticky session
/// still point at a live (`Ready`/`Draining`) replica of `spec_id`? Split
/// out so it's unit-testable without a `Cookies` jar / `AppState`.
fn sticky_replica_is_live(
    reg: &ruscker_core::ReplicaRegistry,
    spec_id: &str,
    session: &StickySession,
) -> bool {
    // Defense in depth: a cookie for spec A must not satisfy spec B.
    session.spec_id == spec_id
        && reg.replicas_of(spec_id).iter().any(|r| {
            r.id == session.replica_id
                && matches!(r.state, ReplicaState::Ready | ReplicaState::Draining)
        })
}

/// True for a top-level document navigation (a GET whose `Accept`
/// prefers HTML) — the request that should get the splash. Subresource
/// and fetch/XHR requests send other `Accept`s and fall through.
fn wants_html(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .map(|a| a.contains("text/html"))
        .unwrap_or(false)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Inline cold-start interstitial template. `{{LOGO}}` / `{{NAME}}` /
/// `{{PROBE}}` are substituted at render time. Self-contained — `/app`
/// responses carry no CSP, so inline CSS/JS is fine.
const SPLASH_TEMPLATE: &str = r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width,initial-scale=1">
<title>Starting…</title>
<style>
  :root{color-scheme:light dark}
  html,body{height:100%;margin:0}
  body{display:flex;align-items:center;justify-content:center;
    font-family:Jost,ui-sans-serif,system-ui,-apple-system,sans-serif;
    color:#e8f0ec;background:#0c1512;
    background:radial-gradient(1100px 560px at 50% -8%,#103a2e 0%,#0c1512 62%)}
  .box{text-align:center;max-width:340px;padding:24px}
  .logo{width:64px;height:64px;margin:0 auto 20px;display:block;
    animation:pulse 2.4s ease-in-out infinite}
  .ring{width:42px;height:42px;margin:0 auto 20px;border-radius:50%;
    border:3px solid rgba(93,202,165,.22);border-top-color:#1D9E75;
    animation:spin .9s linear infinite}
  h1{font-size:18px;font-weight:600;margin:0 0 6px;letter-spacing:.2px}
  p{font-size:13px;line-height:1.55;color:#9fb6ad;margin:0}
  .app{color:#5DCAA5}
  @keyframes spin{to{transform:rotate(360deg)}}
  @keyframes pulse{0%,100%{opacity:.85;transform:scale(1)}50%{opacity:1;transform:scale(1.05)}}
</style></head>
<body><div class="box">
  <img class="logo" src="{{LOGO}}" alt="" onerror="this.style.display='none'">
  <div class="ring" role="status" aria-label="loading"></div>
  {{HEADING}}
  {{NOTE}}
</div>
<script>
(function(){
  var probe="{{PROBE}}";
  function tick(){
    fetch(probe,{cache:"no-store"}).then(function(r){return r.json()}).then(function(d){
      if(d&&d.ready){location.reload()}else{setTimeout(tick,1200)}
    }).catch(function(){setTimeout(tick,2000)});
  }
  setTimeout(tick,1000);
})();
</script></body></html>"##;

/// Render the cold-start interstitial for `spec`. `base` is the portal
/// base path (`""` or e.g. `/box`), used to build same-origin asset and
/// probe URLs that survive sub-path mounting.
///
/// `at_capacity` switches the copy: `false` = a container is booting
/// ("Starting…"); `true` = the spec is at its replica ceiling with every
/// seat taken, so the visitor is waiting for a *slot to free*, not for a
/// boot. Both poll the same readiness probe and open the app the moment a
/// seat becomes available (#623).
fn cold_start_splash(spec: &Spec, base: &str, at_capacity: bool) -> Response {
    let name = html_escape(spec.display_name.as_deref().unwrap_or(&spec.id));
    let (heading, note) = if at_capacity {
        (
            format!("<h1><span class=\"app\">{name}</span> is full right now</h1>"),
            "<p>Every slot for this app is in use. This page opens \
             automatically as soon as one frees up — you can keep it open.</p>"
                .to_string(),
        )
    } else {
        (
            format!("<h1>Starting <span class=\"app\">{name}</span>…</h1>"),
            "<p>The container is booting — this can take a few seconds the \
             first time. This page opens it automatically when it's ready.</p>"
                .to_string(),
        )
    };
    let html = SPLASH_TEMPLATE
        .replace("{{LOGO}}", &format!("{base}/assets/brand/mark.svg"))
        .replace("{{HEADING}}", &heading)
        .replace("{{NOTE}}", &note)
        .replace("{{PROBE}}", &format!("{base}/app/{}{COLD_PROBE_PATH}", spec.id));
    (
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CACHE_CONTROL, "no-store"),
        ],
        html,
    )
        .into_response()
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
async fn pick_or_spawn(state: &AppState, spec: &Spec) -> anyhow::Result<(Replica, bool)> {
    let routing = spec.effective_routing();
    // APIs balance by in-flight requests, not seats (#424).
    let api = matches!(spec.kind(), SpecKind::Api);
    // Seat-based specs (Shiny / interactive apps) reserve the chosen
    // replica's seat ATOMICALLY with the pick — under the same write lock —
    // so two concurrent first-requests can't both grab the last free seat
    // (#582 part 1). APIs don't use seats. The returned `bool` tells the
    // caller a seat was already counted here, so the session tracker must
    // not increment it again. This path runs only for a *new* session (a
    // live sticky cookie short-circuits in `resolve_replica`), so the
    // write lock here is per-new-session, not per-request.
    let reserve = !api;

    // Fast path: pick + reserve under one write lock.
    {
        let mut reg = state.replicas.write().await;
        if let Some(r) = pick_accepting(reg.replicas_of(&spec.id), routing, api) {
            if reserve {
                reg.inc_sessions(&r.id);
            }
            return Ok((r, reserve));
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
        let mut reg = state.replicas.write().await;
        // An accepting replica (Ready + free seat) → use it (and reserve).
        if let Some(r) = pick_accepting(reg.replicas_of(&spec.id), routing, api) {
            if reserve {
                reg.inc_sessions(&r.id);
            }
            return Ok((r, reserve));
        }
        // Coalescing: a sibling that holds the spawn mutex before us may
        // have spawned a replica that's still `Starting` — reuse it instead
        // of spawning a duplicate, BUT only if it still has a free seat
        // (its spawner already reserved one): otherwise we'd oversubscribe
        // it, so fall through to spawn our own (#582 part 1).
        if let Some(r) = reg
            .replicas_of(&spec.id)
            .iter()
            .find(|r| r.state == ReplicaState::Starting && r.available_seats() > 0)
            .cloned()
        {
            if reserve {
                reg.inc_sessions(&r.id);
            }
            return Ok((r, reserve));
        }
        // Every existing replica is full (no free seat) and none coming up
        // with room. Seat-based specs honour `seats-per-container` by
        // scaling out: spawn another when under `max-replicas` (#582) rather
        // than oversubscribing. APIs don't use seats (the auto-scaler sizes
        // them), so they overload immediately. Only at the replica cap (or
        // when a spawn would have nothing to fall back to) do seat specs
        // overload via `pick_replica`'s Ready fallback — never exceeding
        // `max`. The spec mutex serializes us with the splash gate's
        // background spawn, so the cap holds.
        let count = reg.replicas_of(&spec.id).len();
        let max = spec.effective_max_replicas() as usize;
        if api || count >= max {
            if let Some(r) = pick_replica(reg.replicas_of(&spec.id), routing, api) {
                if reserve {
                    reg.inc_sessions(&r.id);
                }
                return Ok((r, reserve));
            }
            // Nothing usable (all Failed/Stopped) — fall through to spawn.
        }
        // Seat-based + under max (or nothing usable) → spawn a new replica.
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

    let creds = resolve_creds(state, spec).await?;
    let limits = limits_from_spec(spec);
    tracing::info!(
        spec = %spec.id,
        image,
        inner_port = ?inner_port,
        with_creds = creds.is_some(),
        with_limits = !limits.is_empty(),
        "spawning replica on demand"
    );
    // Resolve `${VAR}` in container-env here, at the point of use, and
    // fail the spawn naming the missing var (#314) — never inject a
    // literal `${VAR}` into the container.
    let env = spec
        .resolved_env_pairs()
        .map_err(|e| anyhow::anyhow!("spec {} container-env: {e}", spec.id))?;
    let mut req = ruscker_core::SpawnRequest::new(&spec.id, image)
        .with_limits(limits)
        .with_volumes(spec.volumes.clone().unwrap_or_default())
        .with_env(env)
        .with_placement(spec.effective_placement())
        .with_anti_affinity(spec.effective_anti_affinity());
    if let Some(port) = inner_port {
        req = req.with_port(port);
    }
    if let Some(platform) = spec.platform.as_deref() {
        req = req.with_platform(platform);
    }
    if let Some(cmd) = spec.container_cmd.clone() {
        req = req.with_cmd(cmd);
    }
    if let Some(net) = spec.effective_container_network() {
        req = req.with_network(net);
    }
    let labels = spec.effective_labels();
    if !labels.is_empty() {
        req = req.with_labels(labels);
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

    // Take the write lock only for the insert — a few microseconds — and
    // release before the spec mutex unwinds. Reserve the new replica's
    // first seat for the request that triggered the spawn (#582 part 1),
    // so the session tracker doesn't double-count it.
    {
        let mut reg = state.replicas.write().await;
        reg.add(replica.clone());
        if reserve {
            reg.inc_sessions(&replica.id);
        }
    }
    Ok((replica, reserve))
}

/// Build optional registry credentials from a spec. Returns
/// `None` if the spec doesn't carry both a username and a
/// password — partial credentials make no sense (Docker would
/// just reject the pull). Lives next to the proxy spawn path
/// so the scaler can reuse it via `pub(crate)`.
///
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

/// The password is stored as the literal `${VAR}` (never resolved into
/// the DB / config in memory — #260), so we interpolate it **here**, at
/// the point of use, right before a pull. Idempotent: a value with no
/// `${...}` (or a `docker-registry-credential` flow) is unaffected.
///
/// Returns `Err` with the real cause (`MissingEnvVar { name }`) when the
/// password references an unset variable, so the spawn fails naming the
/// missing var (#314). Previously the error was swallowed and the literal
/// `${VAR}` was left for the backend to detect by scanning for a residual
/// `${` — a scan that also false-rejected a legitimately-resolved value
/// containing the literal `${`. `Ok(None)` means "anonymous pull" (no, or
/// only partial, credentials — partial creds make no sense to Docker).
pub(crate) fn creds_from_spec(
    spec: &Spec,
) -> anyhow::Result<Option<ruscker_core::RegistryCredentials>> {
    let user = spec.docker_registry_username.as_deref().filter(|s| !s.is_empty());
    let pass = spec.docker_registry_password.as_deref().filter(|s| !s.is_empty());
    match (user, pass) {
        (Some(u), Some(p)) => Ok(Some(ruscker_core::RegistryCredentials {
            username: u.to_string(),
            password: ruscker_config::env::interpolate_value(p)?,
            server_address: spec
                .docker_registry_domain
                .as_deref()
                .filter(|s| !s.is_empty())
                .map(str::to_string),
            credential_name: None,
        })),
        _ => Ok(None),
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
///
/// `Err` carries the real cause when the inline password references an
/// unset env var (#314); a DB-store credential is already decrypted so
/// that branch never errors here.
pub(crate) async fn resolve_creds(
    state: &AppState,
    spec: &Spec,
) -> anyhow::Result<Option<ruscker_core::RegistryCredentials>> {
    if let Some(name) = spec
        .docker_registry_credential
        .as_deref()
        .filter(|s| !s.is_empty())
    {
        match (state.db.as_ref(), state.master_key.is_configured()) {
            (Some(pool), true) => {
                match crate::db::credentials::resolve(pool, &state.master_key, name).await {
                    Ok(Some(c)) => return Ok(Some(c)),
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
fn pick_replica(replicas: &[Replica], routing: RoutingStrategy, api: bool) -> Option<Replica> {
    pick_accepting(replicas, routing, api).or_else(|| {
        // Overload fallback: no replica has a free seat, so route to any
        // `Ready` one rather than 502. Callers that can scale out instead
        // (the proxy's `pick_or_spawn`, the splash gate) use the strict
        // `pick_accepting` and only fall back to this at `max-replicas`
        // (#582).
        select(
            replicas.iter().filter(|r| r.state == ReplicaState::Ready),
            routing,
            api,
        )
    })
}

/// Strict pick: only a `Ready` replica that still has a **free seat**
/// (`is_accepting`). Returns `None` when every replica is full — the
/// signal to scale out rather than oversubscribe a `seats-per-container`
/// limit (#582). Unlike [`pick_replica`], it never returns a full replica.
fn pick_accepting(replicas: &[Replica], routing: RoutingStrategy, api: bool) -> Option<Replica> {
    select(replicas.iter().filter(|r| r.is_accepting()), routing, api)
}

/// Pick one replica from `candidates` per `routing`. Round-robin spreads
/// across the candidates; least-connections (and, for now, weighted-
/// random / resource-aware) favor the least-loaded replica.
///
/// "Least loaded" depends on the spec kind (#424): API specs meter
/// capacity by **in-flight requests** (#336), so fewest in-flight wins;
/// session-based specs (Shiny / interactive) use the most free seats. The
/// old code always used `available_seats()`, which for an API is constant
/// (no sessions are tracked) — so least-connections never actually spread
/// API load.
fn select<'a>(
    candidates: impl Iterator<Item = &'a Replica>,
    routing: RoutingStrategy,
    api: bool,
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
            if api {
                // Fewest in-flight requests wins; ties break on first seen.
                cands.iter().copied().min_by_key(|r| inflight_count(&r.id))?
            } else {
                // Most free seats wins; ties break on the first seen.
                cands.iter().copied().max_by_key(|r| r.available_seats())?
            }
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
        headers.insert("x-script-name", v.clone());
        // RStudio Server's official "behind a path-rewriting proxy"
        // mechanism: we strip `/app/{spec}/` on the way in and tell
        // RStudio its public root via this header, so it rewrites its
        // own URLs / redirects / session WebSocket with the prefix
        // (#230). Harmless for apps that don't read it.
        headers.insert("x-rstudio-root-path", v);
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
    identity: Option<&Identity>,
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
    // These namespaces are reserved for Ruscker-authenticated identity.
    // Strip them for every request — anonymous, feature-off, and feature-on
    // alike — before optionally adding authoritative values (#1001).
    strip_reserved_identity_headers(req.headers_mut());
    // Never forward Ruscker's own cookies (admin session, sticky,
    // prefs) to the app container — the admin session id is a bearer
    // (#258).
    strip_ruscker_cookies(req.headers_mut());

    if let Some(identity) = identity {
        for (name, value) in identity.header_pairs() {
            if let (Ok(name), Ok(value)) = (
                name.parse::<header::HeaderName>(),
                HeaderValue::from_str(&value),
            ) {
                req.headers_mut().insert(name, value);
            }
        }
    }

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

    let client = http_client();

    // For an idempotent, body-less request — GET/HEAD, which is the bulk of
    // interactive-app traffic (navigations, assets, readiness polls) — we
    // can safely replay it on a fresh connection if the first attempt fails
    // to send. This rescues the hyper pool race where the app closed an idle
    // pooled connection just as we dispatched onto it (`client error
    // (SendRequest)`): the visitor's first navigation would otherwise get a
    // bare "upstream error" and only work on a manual retry. We capture the
    // request head only for retryable methods, so the success path and any
    // request with a body (POST/PUT — can't be replayed without buffering)
    // are unaffected.
    let retry_head = matches!(*req.method(), Method::GET | Method::HEAD).then(|| {
        (
            req.method().clone(),
            req.uri().clone(),
            req.version(),
            req.headers().clone(),
        )
    });
    let upstream_resp = match client.request(req).await {
        Ok(resp) => resp,
        Err(first) => {
            let Some((method, uri, version, headers)) = retry_head else {
                return Err(first.into());
            };
            tracing::warn!(
                upstream = %replica.upstream, error = ?first,
                "upstream send failed; retrying once on a fresh connection"
            );
            let mut retry = Request::builder()
                .method(method)
                .uri(uri)
                .version(version)
                .body(Body::empty())
                .map_err(|e| anyhow::anyhow!("rebuild retry request: {e}"))?;
            *retry.headers_mut() = headers;
            client.request(retry).await?
        }
    };

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

/// Ruscker-owned cookies that must never reach an upstream app
/// container. The browser sends them on same-origin `/app` + `/api`
/// requests (the admin/sticky/pref cookies are all `Path=/`), but the
/// app has no business seeing them — and the admin session id is a
/// bearer, so a malicious or compromised container could replay it
/// against `/admin` (#258). Ruscker has already consumed the sticky
/// cookie (replica resolution) before this runs, so dropping it here
/// is safe.
const RUSCKER_COOKIE_NAMES: &[&str] = &[
    crate::auth::COOKIE_NAME,  // ruscker_admin_session
    crate::theme::COOKIE_NAME, // ruscker_theme
    crate::i18n::COOKIE_NAME,  // ruscker_locale
];

/// Is `name` a cookie Ruscker owns (and must therefore never reach an
/// upstream app)? Sticky cookies are per-spec since #731
/// (`__ruscker_session_{spec}`), so they match by prefix — which also
/// covers the legacy un-suffixed `__ruscker_session`.
fn is_ruscker_cookie(name: &str) -> bool {
    RUSCKER_COOKIE_NAMES.contains(&name) || name.starts_with(COOKIE_NAME)
}

/// Drop Ruscker's own cookies from a raw `Cookie` header value,
/// preserving any cookies the app itself set. Returns `None` when
/// nothing remains (the caller should then omit the header entirely).
fn filter_ruscker_cookies(raw: &str) -> Option<String> {
    let kept: Vec<&str> = raw
        .split(';')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .filter(|pair| {
            let name = pair.split('=').next().unwrap_or("").trim();
            !is_ruscker_cookie(name)
        })
        .collect();
    if kept.is_empty() {
        None
    } else {
        Some(kept.join("; "))
    }
}

/// Drop Ruscker's own cookies from the upstream-bound `Cookie` header,
/// preserving any cookies the app itself set. If nothing remains, the
/// header is removed entirely.
fn strip_ruscker_cookies(headers: &mut HeaderMap) {
    let Some(raw) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) else {
        return;
    };
    match filter_ruscker_cookies(raw) {
        None => {
            headers.remove(header::COOKIE);
        }
        Some(kept) => {
            if let Ok(v) = HeaderValue::from_str(&kept) {
                headers.insert(header::COOKIE, v);
            }
        }
    }
}

/// Remove client-supplied identity claims before a request crosses the
/// trust boundary into an app container. The WHOLE `X-SP-*` and
/// `X-Ruscker-User-*` namespaces are reserved (codex review, #1001):
/// stripping only the exact names we inject would let an anonymous
/// client forge any *other* identity-looking attribute an app might
/// trust (e.g. `X-SP-UserEmail`). `HeaderName` is normalized to
/// lowercase, so the comparisons are case-insensitive by construction.
fn strip_reserved_identity_headers(headers: &mut HeaderMap) {
    let names: Vec<_> = headers
        .keys()
        .filter(|name| {
            let name = name.as_str();
            name.starts_with("x-sp-") || name.starts_with("x-ruscker-user-")
        })
        .cloned()
        .collect();
    for name in names {
        headers.remove(name);
    }
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
    fn strip_ruscker_cookies_drops_ours_keeps_app_cookies() {
        // The admin session id must never reach the upstream app (#258),
        // while the app's own cookies pass through untouched.
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!(
                "{}=secret-admin-sid; {}=abc; {}=def; app_token=keepme; {}=dark; {}=pt",
                crate::auth::COOKIE_NAME,
                // Legacy global sticky AND a per-spec one (#731) — both
                // must match the prefix filter.
                ruscker_proxy::sticky::COOKIE_NAME,
                ruscker_proxy::sticky::cookie_name("my-shiny"),
                crate::theme::COOKIE_NAME,
                crate::i18n::COOKIE_NAME,
            ))
            .unwrap(),
        );
        strip_ruscker_cookies(&mut h);
        let got = h.get(header::COOKIE).unwrap().to_str().unwrap();
        assert!(!got.contains("secret-admin-sid"), "admin cookie leaked: {got}");
        assert!(!got.contains(crate::auth::COOKIE_NAME));
        assert!(!got.contains(ruscker_proxy::sticky::COOKIE_NAME));
        assert!(!got.contains(crate::theme::COOKIE_NAME));
        assert!(!got.contains(crate::i18n::COOKIE_NAME));
        assert!(got.contains("app_token=keepme"), "app cookie dropped: {got}");
    }

    #[test]
    fn strip_ruscker_cookies_removes_header_when_only_ours() {
        let mut h = HeaderMap::new();
        h.insert(
            header::COOKIE,
            HeaderValue::from_str(&format!("{}=sid", crate::auth::COOKIE_NAME)).unwrap(),
        );
        strip_ruscker_cookies(&mut h);
        assert!(!h.contains_key(header::COOKIE), "empty Cookie header should be removed");
    }

    #[test]
    fn strip_reserved_identity_headers_drops_sp_and_ruscker_claims() {
        let mut headers = HeaderMap::new();
        headers.insert("x-sp-userid", HeaderValue::from_static("mallory"));
        headers.insert("x-sp-usergroups", HeaderValue::from_static("attackers"));
        // The whole namespace is reserved, not just the injected pair —
        // an app could trust any X-SP-* attribute (codex review, #1001).
        headers.insert("x-sp-useremail", HeaderValue::from_static("m@evil.test"));
        headers.insert(
            "x-ruscker-user-email",
            HeaderValue::from_static("mallory@example.test"),
        );
        headers.insert("x-app-header", HeaderValue::from_static("keep"));

        strip_reserved_identity_headers(&mut headers);

        assert!(!headers.contains_key("x-sp-userid"));
        assert!(!headers.contains_key("x-sp-usergroups"));
        assert!(!headers.contains_key("x-sp-useremail"));
        assert!(!headers.contains_key("x-ruscker-user-email"));
        assert_eq!(headers.get("x-app-header").unwrap(), "keep");
    }

    #[test]
    fn filter_ruscker_cookies_strips_for_ws_handshake() {
        // The WS upgrade branch forwards the cookie via this filter
        // (not `strip_ruscker_cookies`), so the admin session id must not
        // survive here either (#258).
        let raw = format!(
            "{}=secret-admin-sid; app_token=keepme; {}=pt",
            crate::auth::COOKIE_NAME,
            crate::i18n::COOKIE_NAME,
        );
        let kept = filter_ruscker_cookies(&raw).expect("app cookie remains");
        assert!(!kept.contains("secret-admin-sid"), "admin cookie leaked: {kept}");
        assert!(!kept.contains(crate::auth::COOKIE_NAME));
        assert!(!kept.contains(crate::i18n::COOKIE_NAME));
        assert_eq!(kept, "app_token=keepme");
        // Only-ours ⇒ None, so the WS branch sends no Cookie header.
        let only_ours = format!("{}=sid", crate::auth::COOKIE_NAME);
        assert!(filter_ruscker_cookies(&only_ours).is_none());
    }

    #[test]
    fn inflight_guard_counts_and_gc_prunes() {
        // #336: the RAII guard bumps the count and drops it on scope exit;
        // GC forgets replicas no longer alive. Fresh uuids keep this test
        // isolated from the process-global map.
        let rid = ruscker_core::ReplicaId(uuid::Uuid::new_v4());
        assert_eq!(inflight_count(&rid), 0);
        {
            let _g1 = InflightGuard::new(rid.clone());
            let _g2 = InflightGuard::new(rid.clone());
            assert_eq!(inflight_count(&rid), 2);
        }
        // Both guards dropped → back to zero.
        assert_eq!(inflight_count(&rid), 0);

        // GC drops the (now-zero) entry when the replica isn't alive.
        let _g = InflightGuard::new(rid.clone());
        assert_eq!(inflight_count(&rid), 1);
        inflight_gc(&std::collections::HashSet::new());
        assert_eq!(inflight_count(&rid), 0);
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
        let chosen = pick_replica(&reps, RoutingStrategy::LeastConnections, false).unwrap();
        assert_eq!(chosen.sessions_active, 1);
    }

    #[test]
    fn pick_replica_least_connections_picks_most_free() {
        let reps = vec![
            rep(ReplicaState::Ready, 3, 10), // 7 free
            rep(ReplicaState::Ready, 1, 10), // 9 free ✓
            rep(ReplicaState::Ready, 8, 10), // 2 free
        ];
        let chosen = pick_replica(&reps, RoutingStrategy::LeastConnections, false).unwrap();
        assert_eq!(chosen.sessions_active, 1);
    }

    #[test]
    fn api_least_connections_picks_fewest_inflight() {
        // reps[0] has MORE free seats (100 vs 50) but MORE in-flight (2 vs 0).
        // Seat-based routing would pick reps[0]; in-flight routing reps[1].
        let reps = vec![rep(ReplicaState::Ready, 0, 100), rep(ReplicaState::Ready, 50, 100)];
        let g1 = InflightGuard::new(reps[0].id.clone());
        let g2 = InflightGuard::new(reps[0].id.clone());
        // api=true → fewest in-flight → reps[1].
        let api_pick = pick_replica(&reps, RoutingStrategy::LeastConnections, true).unwrap();
        assert_eq!(
            api_pick.id, reps[1].id,
            "API least-connections must pick the replica with fewest in-flight"
        );
        // api=false → most free seats → reps[0] (in-flight ignored).
        let seat_pick = pick_replica(&reps, RoutingStrategy::LeastConnections, false).unwrap();
        assert_eq!(seat_pick.id, reps[0].id);
        drop((g1, g2));
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
                pick_replica(&reps, RoutingStrategy::LeastConnections, false).is_none(),
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
        let chosen = pick_replica(&reps, RoutingStrategy::LeastConnections, false).unwrap();
        assert_eq!(chosen.state, ReplicaState::Ready);
    }

    #[test]
    fn pick_replica_none_when_no_ready_replica() {
        let reps = vec![
            rep(ReplicaState::Starting, 0, 5),
            rep(ReplicaState::Draining, 0, 5),
        ];
        assert!(pick_replica(&reps, RoutingStrategy::RoundRobin, false).is_none());
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
            base_path: std::sync::Arc::from(""),
            locales: std::sync::Arc::new(
                crate::i18n::Locales::load().expect("load locales"),
            ),
            admin_auth: Default::default(),
            admin_sessions: StdArc::new(crate::auth::InMemoryAdminSessionStore::default()),
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
            logout_index: StdArc::new(dashmap::DashMap::new()),
            leader: StdArc::new(crate::leader::AlwaysLeader),
            metrics: crate::metrics_cache::MetricsCache::new(),
            draining: StdArc::new(std::sync::atomic::AtomicBool::new(false)),
            spec_cache: StdArc::new(dashmap::DashMap::new()),
            identity_cache: Default::default(),
            catalog_cache: StdArc::new(tokio::sync::RwLock::new(None)),
            access_counter: StdArc::new(crate::access_counter::AccessCounter::default()),
            alerts: crate::alerts::AlertSink::default(),
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
        assert!(creds_from_spec(&s).unwrap().is_none());
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
        let c = creds_from_spec(&s).unwrap().expect("creds present");
        assert_eq!(c.username, "bot");
        assert_eq!(c.password, "hunter2");
        assert_eq!(c.server_address.as_deref(), Some("priv.io"));
        assert!(c.credential_name.is_none(), "YAML credentials are inline");
    }

    #[test]
    fn creds_from_spec_resolves_password_env_at_use() {
        // #260: the password is stored as a `${VAR}` literal (never
        // resolved into the DB/config); creds_from_spec resolves it at
        // the point of use, right before the pull.
        std::env::set_var("RUSCKER_TEST_REG_PW", "fromenv");
        let s = spec_yaml(
            r#"
id: p
display-name: P
container-image: priv.io/app:1
docker-registry-username: bot
docker-registry-password: ${RUSCKER_TEST_REG_PW}
"#,
        );
        let c = creds_from_spec(&s).unwrap().expect("creds present");
        assert_eq!(c.password, "fromenv", "resolved at use");
        std::env::remove_var("RUSCKER_TEST_REG_PW");
    }

    #[test]
    fn creds_from_spec_errors_when_password_env_unset() {
        // #314: an unset password var is a hard error naming the var, not
        // a silently-kept `${VAR}` literal for the backend to scan for.
        let s = spec_yaml(
            r#"
id: p
display-name: P
container-image: priv.io/app:1
docker-registry-username: bot
docker-registry-password: ${RUSCKER_DEFINITELY_UNSET_REG_PW}
"#,
        );
        let err = creds_from_spec(&s).expect_err("unset var must error");
        assert!(
            err.to_string().contains("RUSCKER_DEFINITELY_UNSET_REG_PW"),
            "error names the missing var: {err}"
        );
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
            creds_from_spec(&s).unwrap().is_none(),
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
        assert!(creds_from_spec(&s).unwrap().is_none());
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
            creds_from_spec(&s).unwrap().is_none(),
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
    fn mount_prefix_includes_base_path() {
        // Root deploy (no base path) — unchanged behaviour.
        assert_eq!(mount_prefix("", "/app/", "my-shiny"), "/app/my-shiny");
        assert_eq!(mount_prefix("", "/api/", "data"), "/api/data");
        // Under `--base-path /box` the advertised prefix MUST carry it,
        // or RStudio/Jupyter build `/app/...` URLs that 404 behind the
        // base-path proxy (the cast 404 / blank-page bug). No trailing
        // slash — `route_prefix` already has one.
        assert_eq!(mount_prefix("/box", "/app/", "rstudio"), "/box/app/rstudio");
        assert_eq!(mount_prefix("/box", "/api/", "data-api"), "/box/api/data-api");
    }

    #[test]
    fn public_path_carries_base_and_route_family() {
        // App/Shiny specs route under `/app/`, with a trailing slash and
        // the base path baked in (#371).
        assert_eq!(public_path("/box", &fake_spec("jupyter")), "/box/app/jupyter/");
        assert_eq!(public_path("", &fake_spec("jupyter")), "/app/jupyter/");
        // API specs route under `/api/`.
        let api = spec_yaml("id: data\ntype: api\ncontainer-image: x:1\n");
        assert_eq!(public_path("/box", &api), "/box/api/data/");
    }

    #[test]
    fn apply_public_path_substitutes_token_only() {
        let path = "/box/app/jupyter/";
        let mut env = vec!["FOO=bar".to_string(), "BASE=#{publicPath}".to_string()];
        let mut cmd = Some(vec![
            "start.py".to_string(),
            "--ServerApp.base_url=#{publicPath}".to_string(),
        ]);
        apply_public_path(&mut env, &mut cmd, path);
        // Token resolved in both the cmd argv and an env value.
        assert!(cmd
            .as_ref()
            .unwrap()
            .contains(&"--ServerApp.base_url=/box/app/jupyter/".to_string()));
        assert!(env.contains(&"BASE=/box/app/jupyter/".to_string()));
        assert!(env.contains(&"FOO=bar".to_string()), "untouched entries survive");
        // Does NOT auto-inject SHINYPROXY_PUBLIC_PATH — Ruscker strips the
        // prefix, so advertising it would mis-prefix ShinyProxy-style apps
        // (#372). No token → no change.
        assert!(
            !env.iter().any(|e| e.starts_with("SHINYPROXY_PUBLIC_PATH=")),
            "must not inject SHINYPROXY_PUBLIC_PATH"
        );
        let mut env2 = vec!["A=1".to_string()];
        apply_public_path(&mut env2, &mut None, path);
        assert_eq!(env2, vec!["A=1".to_string()], "no token ⇒ env untouched");
    }

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

    // ── Cold-start splash helpers (#…) ──────────────────────────

    #[test]
    fn wants_html_only_for_html_accept() {
        let mut h = HeaderMap::new();
        assert!(!wants_html(&h), "no Accept ⇒ not a navigation");
        h.insert(header::ACCEPT, HeaderValue::from_static("application/json"));
        assert!(!wants_html(&h), "json fetch ⇒ no splash");
        h.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/html,application/xhtml+xml,*/*"),
        );
        assert!(wants_html(&h), "document navigation ⇒ splash");
    }

    #[tokio::test]
    async fn cold_start_splash_renders_name_probe_and_logo_under_base() {
        let s = spec_yaml("id: nb\ndisplay-name: Jupyter\ncontainer-image: x");
        // Scaling up → "Starting…" copy.
        let resp = cold_start_splash(&s, "/box", false);
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers().get(header::CONTENT_TYPE).unwrap().to_str().unwrap();
        assert!(ct.starts_with("text/html"));
        let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let html = String::from_utf8(body.to_vec()).unwrap();
        assert!(html.contains("Starting <span class=\"app\">Jupyter</span>"));
        assert!(html.contains("\"/box/app/nb/__ruscker_ready\""), "base-prefixed probe");
        assert!(html.contains("src=\"/box/assets/brand/mark.svg\""), "base-prefixed logo");
        assert!(!html.contains("{{"), "all template slots filled");

        // At capacity → "full" copy, same probe, no "Starting".
        let full = cold_start_splash(&s, "/box", true);
        let fbody = axum::body::to_bytes(full.into_body(), 1 << 20).await.unwrap();
        let fhtml = String::from_utf8(fbody.to_vec()).unwrap();
        assert!(fhtml.contains("is full right now"), "capacity copy");
        assert!(!fhtml.contains("Starting <span"), "no 'Starting' when full");
        assert!(fhtml.contains("\"/box/app/nb/__ruscker_ready\""), "still polls readiness");
        assert!(!fhtml.contains("{{"), "all slots filled");
    }

    #[test]
    fn html_escape_neutralizes_markup() {
        assert_eq!(html_escape("a<b>&c"), "a&lt;b&gt;&amp;c");
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
        let c = creds_from_spec(&s).unwrap().expect("creds present");
        assert!(c.server_address.is_none(), "Docker Hub default");
    }

    #[tokio::test]
    async fn coalescer_spawns_once_under_concurrent_first_requests() {
        let backend = StdArc::new(CountingBackend {
            spawns: AtomicU32::new(0),
            delay: StdDuration::from_millis(80),
        });
        let state = coalescer_state(backend.clone() as StdArc<dyn ContainerBackend>);
        // Pin max-replicas:1 so this exercises pure coalescing — one spawn
        // for a burst of first-requests — independent of the global default
        // ceiling. Scale-out beyond one replica is covered by the
        // scaleout/capped tests.
        let spec = spec_yaml("id: coalesced\ncontainer-image: test:latest\nmax-replicas: 1\n");

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
            let (replica, _reserved) = t.await.expect("join");
            replica_ids.insert(replica.id);
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

    #[tokio::test]
    async fn find_spec_serves_from_cache_within_ttl() {
        // #587: a recent resolution is served from the in-memory cache
        // instead of re-querying the DB; clearing the cache re-reads.
        let backend = StdArc::new(CountingBackend {
            spawns: AtomicU32::new(0),
            delay: StdDuration::from_millis(0),
        });
        let mut state = coalescer_state(backend as StdArc<dyn ContainerBackend>);
        let cdb = crate::db::ConfigDb::Sqlite(crate::db::open_memory().await.unwrap());
        let v1: Spec =
            serde_yaml_ng::from_str("id: alpha\ncontainer-image: nginx\ndisplay-name: Old").unwrap();
        crate::db::specs::upsert_one(&cdb, &v1, None).await.unwrap();
        state.db = Some(cdb);

        // First lookup resolves from the DB and populates the cache.
        let first = find_spec(&state, "alpha").await.expect("found");
        assert_eq!(first.display_name.as_deref(), Some("Old"));

        // Change it in the DB out from under the cache.
        let v2: Spec =
            serde_yaml_ng::from_str("id: alpha\ncontainer-image: nginx\ndisplay-name: New").unwrap();
        crate::db::specs::upsert_one(state.db.as_ref().unwrap(), &v2, None)
            .await
            .unwrap();

        // Within the TTL, the cache still serves the old value.
        let cached = find_spec(&state, "alpha").await.expect("found");
        assert_eq!(
            cached.display_name.as_deref(),
            Some("Old"),
            "served from cache within TTL"
        );

        // Clearing the cache makes the next lookup re-read the DB.
        state.spec_cache.clear();
        let fresh = find_spec(&state, "alpha").await.expect("found");
        assert_eq!(
            fresh.display_name.as_deref(),
            Some("New"),
            "DB value after cache clear"
        );

        // A missing id is not cached (map stays bounded by the catalog).
        assert!(find_spec(&state, "ghost").await.is_none());
        assert!(state.spec_cache.get("ghost").is_none());
    }

    // A GET whose first upstream connection dies before responding — the
    // hyper pool race that surfaced on cast as `client error (SendRequest)`
    // → a spurious "upstream error" on the visitor's first RStudio open —
    // must be rescued by the one-shot retry in do_forward, not 502'd. The
    // fake upstream drops the first connection without a reply and answers
    // 200 on the second; reaching the 200 proves a second connection (the
    // retry) was made, since otherwise the second accept never completes.
    #[tokio::test]
    async fn do_forward_retries_a_get_when_the_upstream_connection_dies() {
        use tokio::io::AsyncWriteExt;
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            // 1st connection: accept, then close with no response.
            let (conn1, _) = listener.accept().await.unwrap();
            drop(conn1);
            // 2nd connection (the retry): answer a real 200.
            let (mut conn2, _) = listener.accept().await.unwrap();
            let resp = "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
            conn2.write_all(resp.as_bytes()).await.unwrap();
            conn2.flush().await.unwrap();
        });

        let replica = Replica {
            id: ruscker_core::ReplicaId(uuid::Uuid::new_v4()),
            spec_id: "rstudio".into(),
            container_id: "c".into(),
            upstream: addr,
            state: ReplicaState::Ready,
            started_at: chrono::Utc::now(),
            sessions_active: 0,
            sessions_max: 1,
            host: None,
        };
        let req = Request::builder()
            .method(Method::GET)
            .uri("http://placeholder/")
            .body(Body::empty())
            .unwrap();

        let resp = do_forward(
            &replica,
            "/".to_string(),
            "/app/rstudio",
            false,
            req,
            None,
            None,
        )
        .await
        .expect("the dead-connection GET should be retried, not surfaced as an error");
        assert_eq!(resp.status(), StatusCode::OK);
        server.await.unwrap();
    }

    /// A full Ready replica (seats=1, 1 session) for `spec_id`.
    fn full_replica(spec_id: &str) -> Replica {
        Replica {
            id: ReplicaId(uuid::Uuid::new_v4()),
            spec_id: spec_id.to_string(),
            container_id: "full".into(),
            upstream: "127.0.0.1:1".parse::<SocketAddr>().unwrap(),
            state: ReplicaState::Ready,
            started_at: chrono::Utc::now(),
            sessions_active: 1,
            sessions_max: 1,
            host: None,
        }
    }

    /// A Ready replica with one FREE seat (seats=1, 0 sessions).
    fn accepting_replica(spec_id: &str) -> Replica {
        Replica {
            sessions_active: 0,
            ..full_replica(spec_id)
        }
    }

    #[tokio::test]
    async fn pick_or_spawn_reserves_seat_atomically_under_race() {
        // #582 part 1: two concurrent first-requests for the same spec,
        // one free seat. With the atomic pick+reserve they must NOT both
        // grab it — one reserves the seat, the other sees it full and
        // scales out (max-replicas: 2), so they get distinct replicas.
        let backend = StdArc::new(CountingBackend {
            spawns: AtomicU32::new(0),
            delay: StdDuration::from_millis(40),
        });
        let state = coalescer_state(backend.clone() as StdArc<dyn ContainerBackend>);
        let spec: Spec = serde_yaml_ng::from_str(
            "id: race\ncontainer-image: test:latest\nseats-per-container: 1\nmax-replicas: 2",
        )
        .unwrap();
        state.replicas.write().await.add(accepting_replica("race"));

        let (s1, s2) = (state.clone(), state.clone());
        let (p1, p2) = (spec.clone(), spec.clone());
        let (a, b) = tokio::join!(
            tokio::spawn(async move { pick_or_spawn(&s1, &p1).await.expect("a").0 }),
            tokio::spawn(async move { pick_or_spawn(&s2, &p2).await.expect("b").0 }),
        );
        let ra = a.unwrap();
        let rb = b.unwrap();
        assert_ne!(
            ra.id, rb.id,
            "concurrent picks must not share the last seat (atomic reserve)"
        );
        assert_eq!(
            state.replicas.read().await.replicas_of("race").len(),
            2,
            "scaled out to a second replica instead of oversubscribing"
        );
        // No replica holds more than its 1 seat.
        for r in state.replicas.read().await.replicas_of("race") {
            assert!(r.sessions_active <= r.sessions_max, "no over-admission");
        }
    }

    #[tokio::test]
    async fn pick_or_spawn_scales_out_when_all_full_under_max() {
        // #582: the only replica is full (seats=1) and we're under
        // max-replicas → spawn another rather than overloading it.
        let backend = StdArc::new(CountingBackend {
            spawns: AtomicU32::new(0),
            delay: StdDuration::from_millis(0),
        });
        let state = coalescer_state(backend.clone() as StdArc<dyn ContainerBackend>);
        let spec: Spec = serde_yaml_ng::from_str(
            "id: scaleout\ncontainer-image: test:latest\nseats-per-container: 1\nmax-replicas: 3",
        )
        .unwrap();
        state.replicas.write().await.add(full_replica("scaleout"));

        let _ = pick_or_spawn(&state, &spec).await.expect("pick_or_spawn");
        assert_eq!(
            backend.spawns.load(Ordering::SeqCst),
            1,
            "scaled out instead of overloading the full replica"
        );
        assert_eq!(state.replicas.read().await.replicas_of("scaleout").len(), 2);
    }

    #[tokio::test]
    async fn pick_or_spawn_overloads_at_max_instead_of_spawning() {
        // At the replica cap with everything full → reuse the least-loaded
        // replica, never exceed max-replicas.
        let backend = StdArc::new(CountingBackend {
            spawns: AtomicU32::new(0),
            delay: StdDuration::from_millis(0),
        });
        let state = coalescer_state(backend.clone() as StdArc<dyn ContainerBackend>);
        let spec: Spec = serde_yaml_ng::from_str(
            "id: capped\ncontainer-image: test:latest\nseats-per-container: 1\nmax-replicas: 1",
        )
        .unwrap();
        state.replicas.write().await.add(full_replica("capped"));

        let _ = pick_or_spawn(&state, &spec).await.expect("pick_or_spawn");
        assert_eq!(
            backend.spawns.load(Ordering::SeqCst),
            0,
            "no spawn at max-replicas"
        );
        assert_eq!(state.replicas.read().await.replicas_of("capped").len(), 1);
    }

    #[test]
    fn splash_probe_and_gate_agree_on_a_full_ready_replica() {
        // The splash probe and gate share `has_ready_replica` (pick_accepting),
        // so they never disagree. A `seats: 1` app whose only seat is taken is
        // NOT accepting — pick_accepting returns None — so the probe reports
        // "not ready" and the gate keeps the splash up; the visitor doesn't get
        // reload-looped into a replica that can't take them. (A free seat — the
        // cold-start case — makes pick_accepting Some and the splash advances.)
        let full = Replica {
            id: ruscker_core::ReplicaId::new(),
            spec_id: "shiny".into(),
            container_id: "c".into(),
            upstream: "127.0.0.1:8000".parse().unwrap(),
            state: ReplicaState::Ready,
            started_at: chrono::Utc::now(),
            sessions_active: 1,
            sessions_max: 1, // full
            host: None,
        };
        assert!(
            pick_accepting(std::slice::from_ref(&full), RoutingStrategy::LeastConnections, false).is_none(),
            "a full replica is not accepting — probe and gate both say 'not ready'"
        );
        let free = Replica { sessions_active: 0, ..full };
        assert!(
            pick_accepting(&[free], RoutingStrategy::LeastConnections, false).is_some(),
            "a free seat (cold start) is accepting — the splash advances"
        );
    }

    #[test]
    fn splash_lets_the_seat_owner_through() {
        // A *full* single-seat replica is "not accepting", so a fresh
        // visitor waits on the splash (the test above). But the session that
        // already OWNS that seat must be let straight through — otherwise the
        // app's own follow-up navigation (RStudio → /auth-sign-in, Jupyter →
        // /lab) is bounced back to a splash that can never clear, because the
        // seat it's waiting for is the one it already holds. This is the cast
        // RStudio/Jupyter "stuck on Starting…" bug.
        use ruscker_core::{Replica, ReplicaId, ReplicaRegistry, ReplicaState};
        let rid = ReplicaId::new();
        let mk = |state: ReplicaState| Replica {
            id: rid.clone(),
            spec_id: "rstudio".into(),
            container_id: "c".into(),
            upstream: "127.0.0.1:8787".parse().unwrap(),
            state,
            started_at: chrono::Utc::now(),
            sessions_active: 1,
            sessions_max: 1, // full — its only seat is this session's
            host: None,
        };
        let mut ready = ReplicaRegistry::new();
        ready.add(mk(ReplicaState::Ready));
        let owner = StickySession::new("rstudio", rid.clone());

        assert!(
            sticky_replica_is_live(&ready, "rstudio", &owner),
            "the session holding the full single seat must bypass the splash"
        );
        assert!(
            !sticky_replica_is_live(&ready, "rstudio", &StickySession::new("rstudio", ReplicaId::new())),
            "a cookie pointing at an unknown replica falls through to a fresh pick"
        );
        assert!(
            !sticky_replica_is_live(&ready, "jupyter", &owner),
            "spec guard: a cookie for spec A must not satisfy spec B"
        );

        let mut starting = ReplicaRegistry::new();
        starting.add(mk(ReplicaState::Starting));
        assert!(
            !sticky_replica_is_live(&starting, "rstudio", &owner),
            "a Starting (not-yet-Ready) replica is not a usable seat yet"
        );
    }

    // #730 end-to-end: the WS upgrade must (a) carry the query string to
    // the upstream handshake, (b) echo the upstream-SELECTED subprotocol
    // on the 101 to the client, and (c) pump frames both ways. The mock
    // upstream picks the SECOND offered protocol, so a pass proves the
    // echo is the upstream's choice, not the client's first offer.
    #[tokio::test]
    // tungstenite's accept_hdr callback returns its large ErrorResponse
    // by value; fine for a test.
    #[allow(clippy::result_large_err)]
    async fn ws_upgrade_preserves_query_and_echoes_upstream_subprotocol() {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        use tokio_tungstenite::tungstenite::handshake::server::{
            Request as WsRequest, Response as WsResponse,
        };
        use tokio_tungstenite::tungstenite::Message as TgMsg;

        // Mock upstream: record the handshake URI, select a subprotocol,
        // then echo one text frame.
        let up_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let up_addr = up_listener.local_addr().unwrap();
        let upstream = tokio::spawn(async move {
            let (tcp, _) = up_listener.accept().await.unwrap();
            let mut seen_uri = String::new();
            let mut ws = tokio_tungstenite::accept_hdr_async(
                tcp,
                |req: &WsRequest, mut resp: WsResponse| {
                    seen_uri = req.uri().to_string();
                    resp.headers_mut().insert(
                        header::SEC_WEBSOCKET_PROTOCOL,
                        HeaderValue::from_static("superchat"),
                    );
                    Ok(resp)
                },
            )
            .await
            .unwrap();
            if let Some(Ok(msg)) = ws.next().await {
                ws.send(msg).await.unwrap();
            }
            seen_uri
        });

        // Proxy state: one spec, one Ready replica pointing at the mock.
        let backend = StdArc::new(CountingBackend {
            spawns: AtomicU32::new(0),
            delay: StdDuration::ZERO,
        });
        let mut state = coalescer_state(backend as StdArc<dyn ContainerBackend>);
        state.config = std::sync::Arc::new(
            ruscker_config::Config::from_yaml(
                "proxy:\n  specs:\n    - id: wsapp\n      container-image: test:latest\n",
            )
            .expect("config"),
        );
        state.replicas.write().await.add(Replica {
            id: ReplicaId(uuid::Uuid::new_v4()),
            spec_id: "wsapp".into(),
            container_id: "c".into(),
            upstream: up_addr,
            state: ReplicaState::Ready,
            started_at: chrono::Utc::now(),
            sessions_active: 0,
            sessions_max: 10,
            host: None,
        });

        let app = Router::new()
            .merge(routes())
            .layer(tower_cookies::CookieManagerLayer::new())
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        // WS client through the proxy, offering two subprotocols + a query.
        let mut req = format!("ws://{proxy_addr}/app/wsapp/ws/echo?session_id=abc")
            .into_client_request()
            .expect("client request");
        req.headers_mut().insert(
            header::SEC_WEBSOCKET_PROTOCOL,
            HeaderValue::from_static("chat, superchat"),
        );
        let (mut client, resp) = tokio_tungstenite::connect_async(req)
            .await
            .expect("ws handshake through the proxy");
        assert_eq!(
            resp.headers()
                .get(header::SEC_WEBSOCKET_PROTOCOL)
                .and_then(|v| v.to_str().ok()),
            Some("superchat"),
            "the proxy 101 must echo the subprotocol the upstream selected"
        );

        client.send(TgMsg::text("hello")).await.expect("send");
        let echoed = client.next().await.expect("a frame").expect("ok frame");
        assert_eq!(echoed.into_text().expect("text").as_str(), "hello");

        assert_eq!(
            upstream.await.unwrap(),
            "/ws/echo?session_id=abc",
            "query string must reach the upstream handshake"
        );
    }

    // #730: a WS request whose replica is unreachable must get a real
    // 502 — not a 101 followed by an opaque drop (the pre-#730 shape).
    #[tokio::test]
    async fn ws_upgrade_to_a_dead_replica_returns_502_not_101() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        use tokio_tungstenite::tungstenite::Error as TgError;

        let backend = StdArc::new(CountingBackend {
            spawns: AtomicU32::new(0),
            delay: StdDuration::ZERO,
        });
        let mut state = coalescer_state(backend as StdArc<dyn ContainerBackend>);
        state.config = std::sync::Arc::new(
            ruscker_config::Config::from_yaml(
                "proxy:\n  specs:\n    - id: wsapp\n      container-image: test:latest\n",
            )
            .expect("config"),
        );
        // Ready replica whose upstream is a dead port.
        state.replicas.write().await.add(Replica {
            id: ReplicaId(uuid::Uuid::new_v4()),
            spec_id: "wsapp".into(),
            container_id: "c".into(),
            upstream: "127.0.0.1:1".parse::<SocketAddr>().unwrap(),
            state: ReplicaState::Ready,
            started_at: chrono::Utc::now(),
            sessions_active: 0,
            sessions_max: 10,
            host: None,
        });

        let app = Router::new()
            .merge(routes())
            .layer(tower_cookies::CookieManagerLayer::new())
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

        let req = format!("ws://{proxy_addr}/app/wsapp/ws")
            .into_client_request()
            .expect("client request");
        match tokio_tungstenite::connect_async(req).await {
            Err(TgError::Http(resp)) => {
                assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
            }
            other => panic!("expected an HTTP 502 handshake rejection, got {other:?}"),
        }
    }

    // #744: `at_capacity` must count only replicas a spawn would compete
    // with (Starting/Ready/Draining) — a registry still holding a crashed
    // replica made the splash claim "full — waiting for a slot" and skip
    // the background spawn, while pick_or_spawn would happily have
    // spawned over the corpse.
    #[tokio::test]
    async fn at_capacity_ignores_failed_and_stopped_replicas() {
        let backend = StdArc::new(CountingBackend {
            spawns: AtomicU32::new(0),
            delay: StdDuration::from_millis(1),
        });
        let state = coalescer_state(backend as StdArc<dyn ContainerBackend>);
        let spec: Spec = serde_yaml_ng::from_str(
            "id: capp
container-image: t:1
max-replicas: 1",
        )
        .unwrap();

        let mk = |st: ReplicaState| Replica {
            id: ReplicaId(uuid::Uuid::new_v4()),
            spec_id: "capp".into(),
            container_id: "c".into(),
            upstream: "127.0.0.1:1".parse::<SocketAddr>().unwrap(),
            state: st,
            started_at: chrono::Utc::now(),
            sessions_active: 0,
            sessions_max: 1,
            host: None,
        };

        state.replicas.write().await.add(mk(ReplicaState::Failed));
        assert!(
            !at_capacity(&state, &spec).await,
            "a Failed leftover must not count toward the ceiling"
        );
        state.replicas.write().await.add(mk(ReplicaState::Ready));
        assert!(
            at_capacity(&state, &spec).await,
            "a live replica does count (max-replicas: 1)"
        );
    }

    // #744: X-Forwarded-For handling on the forward. Untrusted (the
    // default): a client-supplied chain is spoofable and must be
    // dropped/replaced. Trusted (server.useForwardHeaders): the inbound
    // chain is preserved (the peer is appended when one exists — under
    // Router::oneshot there is no TCP peer, so the chain passes as-is).
    #[tokio::test]
    async fn xff_dropped_when_untrusted_and_kept_when_trusted() {
        use tower::ServiceExt;

        for (trusted, expect_xff) in [(false, false), (true, true)] {
            let (up_addr, mut heads) = capture_upstream().await;
            let backend = StdArc::new(CountingBackend {
                spawns: AtomicU32::new(0),
                delay: StdDuration::from_millis(1),
            });
            let mut state = coalescer_state(backend as StdArc<dyn ContainerBackend>);
            let yaml = if trusted {
                "server:
  useForwardHeaders: true
proxy:
  specs:
    - id: x
      container-image: t:1
"
            } else {
                "proxy:
  specs:
    - id: x
      container-image: t:1
"
            };
            state.config = std::sync::Arc::new(ruscker_config::Config::from_yaml(yaml).unwrap());
            state.replicas.write().await.add(Replica {
                id: ReplicaId(uuid::Uuid::new_v4()),
                spec_id: "x".into(),
                container_id: "c".into(),
                upstream: up_addr,
                state: ReplicaState::Ready,
                started_at: chrono::Utc::now(),
                sessions_active: 0,
                sessions_max: 10,
                host: None,
            });
            let app = Router::new()
                .merge(routes())
                .layer(tower_cookies::CookieManagerLayer::new())
                .with_state(state);
            let resp = app
                .oneshot(
                    Request::builder()
                        .uri("/app/x/data")
                        .header("x-forwarded-for", "6.6.6.6")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            let head = heads.recv().await.expect("upstream saw the request").to_ascii_lowercase();
            assert_eq!(
                head.contains("x-forwarded-for: 6.6.6.6"),
                expect_xff,
                "trusted={trusted}: spoofable XFF must only survive when the operator opted in\n{head}"
            );
        }
    }

    /// Mock HTTP upstream that captures each connection's request head
    /// and answers a minimal 200. Returns its address and the channel
    /// the heads arrive on.
    async fn capture_upstream() -> (
        SocketAddr,
        tokio::sync::mpsc::UnboundedReceiver<String>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        tokio::spawn(async move {
            loop {
                let Ok((mut conn, _)) = listener.accept().await else {
                    return;
                };
                let tx = tx.clone();
                tokio::spawn(async move {
                    let mut buf = Vec::new();
                    let mut chunk = [0u8; 1024];
                    while !buf.windows(4).any(|w| w == b"\r\n\r\n") {
                        match conn.read(&mut chunk).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => buf.extend_from_slice(&chunk[..n]),
                        }
                    }
                    let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
                    let _ = conn
                        .write_all(
                            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                        )
                        .await;
                });
            }
        });
        (addr, rx)
    }

    // #732: when the response will be HTML-transformed (the `/app/`
    // family with `inject-base-href` on — the default), the upstream
    // request must carry `Accept-Encoding: identity`: a gzip/br HTML
    // body would reach the rewriter as opaque bytes (no <base href>,
    // possibly corrupted). With the transform disabled per spec, the
    // client's Accept-Encoding must pass through untouched.
    #[tokio::test]
    async fn app_forward_asks_upstream_for_identity_encoding_iff_transforming() {
        use tower::ServiceExt;

        let (up_addr, mut heads) = capture_upstream().await;

        let backend = StdArc::new(CountingBackend {
            spawns: AtomicU32::new(0),
            delay: StdDuration::ZERO,
        });
        let mut state = coalescer_state(backend as StdArc<dyn ContainerBackend>);
        state.config = std::sync::Arc::new(
            ruscker_config::Config::from_yaml(
                "proxy:\n  specs:\n    - id: rewritten\n      container-image: t:1\n    - id: raw\n      container-image: t:1\n      inject-base-href: false\n",
            )
            .expect("config"),
        );
        for id in ["rewritten", "raw"] {
            state.replicas.write().await.add(Replica {
                id: ReplicaId(uuid::Uuid::new_v4()),
                spec_id: id.into(),
                container_id: "c".into(),
                upstream: up_addr,
                state: ReplicaState::Ready,
                started_at: chrono::Utc::now(),
                sessions_active: 0,
                sessions_max: 10,
                host: None,
            });
        }
        let app = Router::new()
            .merge(routes())
            .layer(tower_cookies::CookieManagerLayer::new())
            .with_state(state);

        // Transformed spec: the client's gzip offer must NOT reach the app.
        let resp = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/app/rewritten/data.html")
                    .header(header::ACCEPT_ENCODING, "gzip, br")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let head = heads.recv().await.expect("upstream saw the request");
        let head_lower = head.to_ascii_lowercase();
        assert!(
            head_lower.contains("accept-encoding: identity"),
            "transforming forward must ask for identity, got:\n{head}"
        );
        assert!(
            !head_lower.contains("gzip"),
            "client's compressed offer leaked through:\n{head}"
        );

        // Transform disabled: end-to-end compression stays available.
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/app/raw/data.html")
                    .header(header::ACCEPT_ENCODING, "gzip, br")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let head = heads.recv().await.expect("upstream saw the request");
        assert!(
            head.to_ascii_lowercase().contains("accept-encoding: gzip, br"),
            "non-transforming forward must pass the client's offer through, got:\n{head}"
        );
    }

    /// AppState with two interactive specs (`alpha`, `beta`), each with a
    /// Ready multi-seat replica pointing at `up_addr`, served as a router.
    async fn two_app_router(up_addr: SocketAddr) -> Router {
        let backend = StdArc::new(CountingBackend {
            spawns: AtomicU32::new(0),
            delay: StdDuration::ZERO,
        });
        let mut state = coalescer_state(backend as StdArc<dyn ContainerBackend>);
        state.config = std::sync::Arc::new(
            ruscker_config::Config::from_yaml(
                "proxy:\n  specs:\n    - id: alpha\n      container-image: t:1\n    - id: beta\n      container-image: t:1\n",
            )
            .expect("config"),
        );
        for id in ["alpha", "beta"] {
            state.replicas.write().await.add(Replica {
                id: ReplicaId(uuid::Uuid::new_v4()),
                spec_id: id.into(),
                container_id: "c".into(),
                upstream: up_addr,
                state: ReplicaState::Ready,
                started_at: chrono::Utc::now(),
                sessions_active: 0,
                sessions_max: 10,
                host: None,
            });
        }
        Router::new()
            .merge(routes())
            .layer(tower_cookies::CookieManagerLayer::new())
            .with_state(state)
    }

    /// All `Set-Cookie` values of a response.
    fn set_cookies(resp: &Response) -> Vec<String> {
        resp.headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .filter_map(|v| v.to_str().ok().map(String::from))
            .collect()
    }

    // #731: each app gets its OWN sticky cookie, scoped to its mount —
    // opening app B must not overwrite app A's session, and a returning
    // visit with A's cookie keeps A's session (no fresh Set-Cookie).
    #[tokio::test]
    async fn sticky_cookies_are_per_spec_and_path_scoped() {
        use tower::ServiceExt;

        // capture_upstream answers 200 to every request; the head
        // channel is unused here.
        let (up_addr, _heads) = capture_upstream().await;
        let app = two_app_router(up_addr).await;
        let visit = |path: &str, cookie: Option<String>| {
            let mut b = Request::builder()
                .uri(path)
                .header(header::ACCEPT, "text/html");
            if let Some(c) = cookie {
                b = b.header(header::COOKIE, c);
            }
            b.body(Body::empty()).unwrap()
        };

        // First visit to alpha → per-spec cookie, path-scoped to its mount.
        let resp = app.clone().oneshot(visit("/app/alpha/", None)).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let alpha_cookie = set_cookies(&resp)
            .into_iter()
            .find(|c| c.starts_with("__ruscker_session_alpha="))
            .expect("alpha visit sets its per-spec sticky cookie");
        assert!(
            alpha_cookie.contains("Path=/app/alpha"),
            "cookie must be scoped to alpha's mount: {alpha_cookie}"
        );
        let alpha_pair = alpha_cookie.split(';').next().unwrap().to_string();

        // Visiting beta (with alpha's cookie still in the jar) sets ONLY
        // beta's cookie — alpha's is neither replaced nor expired.
        let resp = app
            .clone()
            .oneshot(visit("/app/beta/", Some(alpha_pair.clone())))
            .await
            .unwrap();
        let beta_cookies = set_cookies(&resp);
        assert!(
            beta_cookies.iter().any(|c| c.starts_with("__ruscker_session_beta=")),
            "beta visit sets beta's cookie: {beta_cookies:?}"
        );
        assert!(
            !beta_cookies.iter().any(|c| c.starts_with("__ruscker_session_alpha=")),
            "beta visit must not touch alpha's cookie: {beta_cookies:?}"
        );

        // Returning to alpha with its cookie: the session is honored —
        // no fresh sticky Set-Cookie is issued.
        let resp = app
            .clone()
            .oneshot(visit("/app/alpha/", Some(alpha_pair)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(
            !set_cookies(&resp)
                .iter()
                .any(|c| c.starts_with("__ruscker_session_alpha=")),
            "an honored sticky session must not re-issue its cookie"
        );
    }

    // #731: a lingering pre-#731 global cookie (`__ruscker_session`,
    // `Path=/`) is actively expired so it doesn't ride every portal
    // request for another 8h.
    #[tokio::test]
    async fn legacy_global_sticky_cookie_is_expired() {
        use tower::ServiceExt;

        let (up_addr, _heads) = capture_upstream().await;
        let app = two_app_router(up_addr).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/app/alpha/")
                    .header(header::ACCEPT, "text/html")
                    .header(header::COOKIE, format!("{COOKIE_NAME}=stale-legacy-value"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let removal = set_cookies(&resp)
            .into_iter()
            .find(|c| c.starts_with(&format!("{COOKIE_NAME}=")) && !c.contains("_alpha"))
            .expect("legacy cookie gets a removal Set-Cookie");
        assert!(
            removal.contains("Max-Age=0") && removal.contains("Path=/"),
            "removal must expire the Path=/ legacy cookie: {removal}"
        );
    }
}
