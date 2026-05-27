//! # ruscker-admin
//!
//! Admin panel and public landing for Ruscker.
//!
//! ## Status (Phase 1)
//!
//! - Public landing page served from `Config` (read-only)
//! - i18n scaffold: pt-BR (100%), en/es/fr (placeholders)
//! - Theme: light/dark via cookie + `prefers-color-scheme`
//!
//! Admin CRUD (Phase 2), proxy (Phase 3), dashboard (Phase 4) come
//! later.

#![allow(dead_code)]

use anyhow::{Context, Result};
use axum::Router;
use ruscker_config::Config;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_cookies::CookieManagerLayer;
use tracing::info;

pub mod auth;
pub mod crypto;
pub mod db;
pub mod i18n;
pub mod images;
pub mod leader;
pub mod logbuf;
pub mod metrics_cache;
pub mod ratelimit;
pub mod routes;
pub mod scaler;
pub mod sessions;
pub mod sessions_pg;
pub mod theme;
pub mod view_model;

use sqlx::SqlitePool;

/// Shared state injected into every request.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub locales: Arc<i18n::Locales>,
    /// Normalized base path the portal is served under (#173): `""` for
    /// root (default) or e.g. `"/box"`. The router nests everything
    /// under it and the response rewriter prefixes generated URLs /
    /// redirects / cookie paths with it.
    pub base_path: Arc<str>,
    pub admin_auth: auth::AdminAuth,
    /// Opaque server-side admin session store. The cookie holds a
    /// random session id (not the token); shared so every cloned
    /// `AppState` sees the same live sessions.
    pub admin_sessions: Arc<auth::AdminSessions>,
    /// Global rate limiter for `/admin/login` — bounds brute
    /// force against the admin token. Shared (Arc) so every
    /// cloned `AppState` sees the same window.
    pub login_limiter: Arc<auth::LoginRateLimiter>,
    /// Per-client, per-spec request limiter enforcing each API
    /// spec's `api.rate-limit`. Shared (Arc) so every cloned
    /// `AppState` sees the same sliding windows. Specs without a
    /// configured limit never touch it.
    pub api_limiter: Arc<ratelimit::ApiRateLimiter>,
    /// Optional admin config database (SQLite by default, Postgres in
    /// HA mode). `None` ⇒ admin CRUD routes 503 because they have no
    /// source of truth. Reach the SQLite pool for not-yet-ported
    /// repositories via [`AppState::sqlite`].
    pub db: Option<db::ConfigDb>,
    /// On-disk fallback directory for `/assets/img/<file>`. When
    /// the DB lookup misses (or `db` is `None`), the assets route
    /// falls through to this directory before 404'ing. `None`
    /// disables the disk fallback entirely.
    pub images_dir: Option<Arc<Path>>,
    /// Master key for the credentials store. When unset, the
    /// `/admin/credentials` route 503s with a hint to set
    /// `RUSCKER_MASTER_KEY`.
    pub master_key: crypto::MasterKey,

    /// Container backend used by the proxy to spawn/list/stop
    /// replicas. `None` ⇒ proxy routes (`/app/*`, `/api/*`) return
    /// 503 with a hint that no backend is wired (Phase 1/2 mode,
    /// landing-only).
    pub backend: Option<std::sync::Arc<dyn ruscker_core::ContainerBackend>>,

    /// Recent Ruscker log lines for the admin "Logs" tab. `None` when no
    /// log buffer was wired (e.g. tests, or non-`serve` commands).
    pub log_buffer: Option<logbuf::LogBuffer>,

    /// Live registry of running replicas, keyed by spec id.
    /// Shared across handlers via an RwLock; writes happen only
    /// on spawn / stop, reads happen per-request to pick a replica.
    pub replicas: std::sync::Arc<tokio::sync::RwLock<ruscker_core::ReplicaRegistry>>,

    /// HMAC key used to sign sticky-session cookies. Auto-
    /// generated per-process unless `RUSCKER_COOKIE_KEY` is set.
    pub cookie_key: ruscker_proxy::sticky::CookieKey,

    /// Per-spec coalescer for first-request spawns. The fast path
    /// in `pick_or_spawn` only touches the read lock on
    /// `replicas`; on miss it acquires this spec's mutex, double-
    /// checks, and (if still empty) does the spawn. Concurrent
    /// first-requests for **different** specs go in parallel
    /// because the mutex is per-key. Concurrent first-requests
    /// for the **same** spec wait for one spawn instead of
    /// racing.
    pub spawn_locks: std::sync::Arc<
        dashmap::DashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>,
    >,

    /// Per-visitor session tracker. The proxy calls
    /// `touch_or_register` on every routed request so the
    /// replica's `sessions_active` reflects the real number of
    /// live visitors. A background sweeper (started from
    /// `AdminServer::run`) evicts idle sessions after
    /// `proxy.heartbeat_timeout` ms.
    pub sessions: std::sync::Arc<dyn sessions::SessionStore>,

    /// Decides whether this instance runs the auto-scaler. Single-node
    /// installs use [`leader::AlwaysLeader`]; HA installs inject a
    /// [`leader::PgLeaderLock`] so exactly one instance scales. The
    /// proxy and session tracking never consult this — only the scaler.
    pub leader: std::sync::Arc<dyn leader::LeaderElector>,

    /// Per-replica metrics cache. Filled by a background
    /// refresher that fans out `backend.metrics()` calls every
    /// [`metrics_cache::REFRESH_INTERVAL`]; the dashboard reads
    /// straight from here without ever waiting on Docker.
    pub metrics: metrics_cache::MetricsCache,

    /// Set to `true` when a graceful shutdown begins (SIGTERM /
    /// Ctrl-C). While set, `/readyz` reports `draining` (503) so
    /// load balancers stop routing new traffic before the listener
    /// closes. Shared (Arc) so the signal handler and every request
    /// handler observe the same flag.
    pub draining: Arc<std::sync::atomic::AtomicBool>,
}

impl AppState {
    /// The SQLite pool when the config DB is SQLite, else `None`.
    /// Repositories not yet ported to Postgres call this; in Postgres
    /// mode they get `None` and 503. See [`db::ConfigDb`].
    pub fn sqlite(&self) -> Option<&SqlitePool> {
        self.db.as_ref().and_then(db::ConfigDb::as_sqlite)
    }
}

/// HTTP server hosting the landing and (later) the admin panel.
pub struct AdminServer {
    addr: SocketAddr,
    state: AppState,
    /// Operator-provided directory served at `/assets/img/`. Cards
    /// reference logos via `template-properties.logo:
    /// "/assets/img/<file>"` (the ShinyProxy convention). `None`
    /// means no `/assets/img/` route is mounted — cards fall back
    /// to tint-only covers.
    images_dir: Option<PathBuf>,
}

impl AdminServer {
    /// Build a server bound to `addr`, serving the given config.
    ///
    /// The config is what the landing page reads to render cards.
    /// In Phase 2 this is replaced with a SQLite-backed source of
    /// truth and the YAML becomes import/export only.
    pub fn new(addr: SocketAddr, config: Config) -> Result<Self> {
        let locales = i18n::Locales::load().context("load locale bundles")?;
        let state = AppState {
            config: Arc::new(config),
            locales: Arc::new(locales),
            base_path: Arc::from(""),
            admin_auth: auth::AdminAuth::from_env(),
            admin_sessions: Arc::new(auth::AdminSessions::default_policy()),
            login_limiter: Arc::new(auth::LoginRateLimiter::default_policy()),
            api_limiter: Arc::new(ratelimit::ApiRateLimiter::new()),
            db: None,
            images_dir: None,
            master_key: crypto::MasterKey::from_env().context("load master key")?,
            backend: None,
            log_buffer: None,
            replicas: std::sync::Arc::new(tokio::sync::RwLock::new(
                ruscker_core::ReplicaRegistry::new(),
            )),
            cookie_key: ruscker_proxy::sticky::CookieKey::from_env_or_random()
                .context("load sticky cookie key")?,
            spawn_locks: std::sync::Arc::new(dashmap::DashMap::new()),
            sessions: std::sync::Arc::new(sessions::InMemorySessionStore::new()),
            leader: std::sync::Arc::new(leader::AlwaysLeader),
            metrics: metrics_cache::MetricsCache::new(),
            draining: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        };
        Ok(Self {
            addr,
            state,
            images_dir: None,
        })
    }

    /// Set the on-disk fallback directory for `/assets/img/<file>`.
    /// When the DB image library has the filename it wins; otherwise
    /// the handler reads from this directory. `None` disables the
    /// fallback (DB-only mode).
    pub fn with_images_dir(mut self, dir: impl AsRef<Path>) -> Self {
        let pathbuf = dir.as_ref().to_path_buf();
        let arc: Arc<Path> = Arc::from(pathbuf.clone().into_boxed_path());
        self.images_dir = Some(pathbuf);
        self.state.images_dir = Some(arc);
        self
    }

    /// Override the **admin** token (default: pulled from
    /// `RUSCKER_ADMIN_TOKEN` env var). Useful for tests that need
    /// a known token without touching the process environment.
    ///
    /// Only the admin token is replaced — the optional `editor` /
    /// `viewer` tokens that [`auth::AdminAuth::from_env`] already read
    /// stay intact, so the CLI's `--admin-token` flag (which clap also
    /// feeds from `RUSCKER_ADMIN_TOKEN`) doesn't wipe the other roles.
    pub fn with_admin_token(mut self, token: impl Into<String>) -> Self {
        self.state.admin_auth.admin = Some(Arc::from(token.into()));
        self
    }

    /// Override the credentials master key (default: pulled from
    /// `RUSCKER_MASTER_KEY`). Accepts hex (64ch) or base64 (44ch).
    pub fn with_master_key(mut self, raw: impl AsRef<str>) -> Result<Self> {
        self.state.master_key = crypto::MasterKey::from_str(raw.as_ref())?;
        Ok(self)
    }

    /// Wire the in-memory log buffer feeding the admin "Logs" tab.
    /// Created and populated by a `tracing` layer in `ruscker-cli`.
    pub fn with_log_buffer(mut self, buffer: logbuf::LogBuffer) -> Self {
        self.state.log_buffer = Some(buffer);
        self
    }

    /// Serve the whole portal under a base path (#173), e.g. `/box` for
    /// mounting at `example.org/box/`. Normalized via
    /// [`ruscker_config::normalize_base_path`]; empty ⇒ root (default).
    pub fn with_base_path(mut self, path: impl AsRef<str>) -> Self {
        let norm = ruscker_config::normalize_base_path(path.as_ref());
        self.state.base_path = Arc::from(norm.as_str());
        self
    }

    /// Attach a container backend (e.g. `LocalDockerBackend`).
    /// Without this, the proxy routes `/app/*` and `/api/*` reply
    /// 503 with a hint that no backend is wired.
    pub fn with_backend(
        mut self,
        backend: std::sync::Arc<dyn ruscker_core::ContainerBackend>,
    ) -> Self {
        self.state.backend = Some(backend);
        self
    }

    /// Swap in a different session store. The default is the
    /// single-node [`sessions::InMemorySessionStore`]; HA deployments
    /// pass a [`sessions_pg::PostgresSessionStore`] so several Ruscker
    /// instances share one session table. The proxy and sweeper only
    /// ever see the `dyn SessionStore`, so nothing else changes.
    pub fn with_session_store(
        mut self,
        store: std::sync::Arc<dyn sessions::SessionStore>,
    ) -> Self {
        self.state.sessions = store;
        self
    }

    /// Attach a SQLite pool. Required for the `/admin/*` routes
    /// that read or write the spec catalog. The pool is shared
    /// across all requests — sqlx handles the connection
    /// multiplexing.
    pub fn with_db(mut self, pool: SqlitePool) -> Self {
        self.state.db = Some(db::ConfigDb::Sqlite(pool));
        self
    }

    /// Attach a config database directly as a [`db::ConfigDb`] — used to
    /// select the shared Postgres catalog for HA. `with_db` is the
    /// SQLite shorthand for `with_config_db(ConfigDb::Sqlite(pool))`.
    pub fn with_config_db(mut self, db: db::ConfigDb) -> Self {
        self.state.db = Some(db);
        self
    }

    /// Replace the leader elector. Default is [`leader::AlwaysLeader`]
    /// (single node). HA installs pass a [`leader::PgLeaderLock`] so
    /// only one instance runs the auto-scaler.
    pub fn with_leader_elector(
        mut self,
        elector: std::sync::Arc<dyn leader::LeaderElector>,
    ) -> Self {
        self.state.leader = elector;
        self
    }

    /// Start listening. Blocks until the process is shut down.
    pub async fn run(self) -> Result<()> {
        // Reconcile the in-memory replica registry with whatever
        // the backend reports as already running. This lets
        // Ruscker survive its own restart without losing track
        // of containers spawned in a prior incarnation — the
        // `ruscker.replica_id` label on each container is the
        // durable identifier.
        if let Some(backend) = self.state.backend.as_ref() {
            match backend.list().await {
                Ok(mut existing) => {
                    // The backend can't know each replica's seat
                    // cap — that lives in the spec config. Enrich
                    // each reconciled replica with its spec's
                    // `effective_seats` so `sessions_max` is
                    // accurate and the scaler's saturation /
                    // available-seats math works on first tick.
                    for r in &mut existing {
                        if let Some(spec) = self
                            .state
                            .config
                            .proxy
                            .specs
                            .iter()
                            .find(|s| s.id == r.spec_id)
                        {
                            r.sessions_max = spec.effective_seats();
                        }
                    }
                    let n = existing.len();
                    self.state.replicas.write().await.reset(existing);
                    if n > 0 {
                        info!(replicas = n, "reconciled existing replicas from backend");
                    }
                }
                Err(err) => {
                    tracing::warn!(error = ?err, "backend.list() failed at startup; starting with empty registry");
                }
            }
        }

        // Start the auto-scaler. With no backend wired the task is
        // still spawned but its loop short-circuits on every tick,
        // so this is safe in landing-only mode. The JoinHandle is
        // deliberately dropped — there's no graceful shutdown
        // protocol for the scaler because every tick is idempotent.
        if let Some(backend) = self.state.backend.clone() {
            // These `spawn`s return a `JoinHandle` (itself a future) we
            // deliberately detach — each loop is idempotent and has no
            // shutdown protocol — so the `let_underscore_future` lint is
            // a false positive here.
            #[allow(clippy::let_underscore_future)]
            let _ = scaler::spawn(self.state.clone(), scaler::DEFAULT_INTERVAL);
            // Session sweeper: evicts idle sessions per the
            // global `heartbeat-timeout`. `-1` (the ShinyProxy
            // idiom for "never expire") becomes a no-op loop
            // inside `sessions::spawn` so the call shape stays
            // uniform.
            #[allow(clippy::let_underscore_future)]
            let _ = sessions::spawn(
                self.state.sessions.clone(),
                self.state.replicas.clone(),
                self.state.config.clone(),
                self.state.leader.clone(),
            );
            // Dashboard metrics: keep `state.metrics` fresh in
            // the background so dashboard renders are read-only
            // and never block on a Docker stats call.
            #[allow(clippy::let_underscore_future)]
            let _ = metrics_cache::spawn(
                self.state.metrics.clone(),
                backend,
                self.state.replicas.clone(),
                metrics_cache::REFRESH_INTERVAL,
            );
        }

        let app = router_with_images(self.state.clone(), self.images_dir.as_deref());
        let listener = TcpListener::bind(self.addr)
            .await
            .with_context(|| format!("bind {}", self.addr))?;
        info!(addr = %self.addr, images_dir = ?self.images_dir, "ruscker-admin listening");
        // `into_make_service_with_connect_info` exposes the TCP peer
        // address to handlers via `ConnectInfo<SocketAddr>` — the
        // proxy uses it as the per-client key for API rate limiting
        // when no trusted `X-Forwarded-For` is present.
        let make_service =
            app.into_make_service_with_connect_info::<SocketAddr>();
        axum::serve(listener, make_service)
            .with_graceful_shutdown(shutdown_signal(self.state.clone()))
            .await
            .context("axum serve")?;
        info!("shutdown complete");
        Ok(())
    }
}

/// Resolves when the process receives a termination signal, then
/// runs the graceful-drain sequence. Returning from this future is
/// what tells `axum::serve` to stop accepting new connections and
/// finish the in-flight ones.
///
/// Sequence on signal:
/// 1. Flip [`AppState::draining`] so `/readyz` starts replying
///    `503 draining` — load balancers deregister this instance.
/// 2. Arm a hard-deadline watchdog that force-exits the process if
///    draining overruns. Long-lived WebSocket sessions (Shiny,
///    Streamlit) never close on their own, so without this the
///    in-flight wait would hang forever.
/// 3. Wait for active sessions to drain, polling
///    [`sessions::SessionStore::len`], up to
///    `proxy.shutdown-grace-ms`. Exits the wait early the moment
///    the last session ends.
async fn shutdown_signal(state: AppState) {
    use std::sync::atomic::Ordering;
    use tokio::time::{sleep, Duration, Instant};

    // SIGINT (Ctrl-C) on every platform; SIGTERM too on Unix
    // (what `systemctl stop` / `docker stop` / k8s send).
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            // If we can't install the handler, never resolve this
            // arm — Ctrl-C still works.
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }

    let grace = Duration::from_millis(state.config.proxy.shutdown_grace_ms);
    let active = state.sessions.len();
    info!(
        grace_ms = state.config.proxy.shutdown_grace_ms,
        active_sessions = active,
        "shutdown signal received; draining"
    );
    state.draining.store(true, Ordering::SeqCst);

    // Watchdog: guarantee the process exits even if in-flight
    // connections (notably long-lived WebSockets) never close. The
    // extra slack covers the time axum spends finishing in-flight
    // HTTP requests after this future resolves.
    let watchdog = grace + Duration::from_secs(5);
    tokio::spawn(async move {
        sleep(watchdog).await;
        tracing::warn!(
            deadline_ms = watchdog.as_millis() as u64,
            "graceful-shutdown deadline exceeded; forcing exit"
        );
        std::process::exit(0);
    });

    // Drain loop: wait for sessions to wind down, but never longer
    // than the grace window. Idle instances (no sessions) fall
    // through immediately.
    let deadline = Instant::now() + grace;
    while !state.sessions.is_empty() && Instant::now() < deadline {
        sleep(Duration::from_millis(250)).await;
    }
    info!(
        remaining_sessions = state.sessions.len(),
        "drain window elapsed; closing listener"
    );
}

/// Compose the axum router. Pulled out so tests can hit it via
/// `Router::oneshot` without a real socket.
pub fn router(state: AppState) -> Router {
    router_with_images(state, None)
}

/// Deprecated alias kept for the CLI's call site — the
/// `images_dir` argument is now ignored because state carries the
/// fallback directory itself (set via [`AdminServer::with_images_dir`]).
pub fn router_with_images(state: AppState, _images_dir: Option<&Path>) -> Router {
    // Ruscker's own surfaces (landing, admin, prefs, assets) get
    // security response headers. The proxy routes do NOT — those
    // forward upstream app responses verbatim, and injecting our
    // X-Frame-Options / CSP into a Shiny page would interfere
    // with how the app expects to be served. Apps own their own
    // security headers.
    let own = Router::new()
        .merge(routes::landing::routes())
        .merge(routes::assets::routes())
        .merge(routes::prefs::routes())
        .merge(routes::admin::routes())
        .layer(axum::middleware::from_fn(security_headers));

    // Health probes (`/healthz`, `/readyz`) sit outside the
    // `security_headers` layer: they return JSON for orchestrators,
    // not HTML for browsers, so CSP / X-Frame-Options are
    // irrelevant. Like the proxy routes, they're merged at the
    // outer level.
    // The full portal surface (chrome + proxy + metrics). Health is
    // kept separate so it can stay at the root even under a base path —
    // load-balancer probes shouldn't have to know the prefix.
    let portal = own
        .merge(routes::proxy::routes())
        // `/metrics` (opt-in via proxy.metrics-enabled) is likewise
        // unauthenticated and outside `security_headers` — it serves
        // Prometheus text for a scraper, not HTML for a browser.
        .merge(routes::metrics::routes());

    // Mount under the configured base path (#173). `""` ⇒ root (the
    // default), so single-host deploys are unchanged. With e.g. `/box`,
    // every portal route matches under `/box/...`; the response rewriter
    // (added in a later slice) prefixes the URLs the handlers emit.
    let base = state.base_path.clone();
    let portal = if base.is_empty() {
        portal
    } else {
        // axum 0.8 `nest` maps the inner `/` route to exactly `/box`
        // (no trailing slash) and does NOT match `/box/` — but that's
        // the URL a browser / nginx sends for the landing root. Redirect
        // `/box/` → `/box` so both forms work (the deeper routes like
        // `/box/admin/...` are matched by nest directly).
        let canonical = base.to_string();
        let trailing = format!("{base}/");
        Router::new().nest(&base, portal).route(
            &trailing,
            axum::routing::get(move || {
                let to = canonical.clone();
                async move { axum::response::Redirect::permanent(&to) }
            }),
        )
    };

    portal
        .merge(routes::health::routes())
        .layer(CookieManagerLayer::new())
        .with_state(state)
}

/// Baseline security response headers for Ruscker's own pages.
///
/// - `X-Content-Type-Options: nosniff` — stop MIME sniffing
///   (defends against polyglot uploads being interpreted as
///   active content).
/// - `X-Frame-Options: DENY` — the portal/admin is not meant to
///   be embedded; blocks clickjacking.
/// - `Referrer-Policy: same-origin` — don't leak admin URLs to
///   third parties.
/// - `Content-Security-Policy` — restricts resource origins to
///   self. `'unsafe-inline'` is currently required because the
///   landing + dashboard use inline `<script>`/`<style>`; a
///   nonce-based CSP that drops `unsafe-inline` is a tracked
///   follow-up. Even with it, this still blocks loading scripts
///   / frames / objects from other origins.
async fn security_headers(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::header::HeaderName;
    use axum::http::HeaderValue;
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    // Provide defaults only — never clobber a header the handler
    // already set. The `/assets/img/*` route sets a STRICTER CSP
    // (`default-src 'none'; … sandbox`) for operator-uploaded
    // SVGs; an unconditional `insert` here would overwrite it
    // with the looser page policy. `entry().or_insert` leaves any
    // handler-set value intact.
    let defaults: &[(&str, &str)] = &[
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "same-origin"),
    ];
    for (name, value) in defaults {
        if let Ok(hn) = HeaderName::from_bytes(name.as_bytes()) {
            h.entry(hn).or_insert(HeaderValue::from_static(value));
        }
    }
    // CSP as a default too — but the landing route may set its own
    // (widened for an operator's analytics origins), so `or_insert`
    // must leave a handler-set value intact.
    if let Ok(v) = HeaderValue::from_str(&content_security_policy("")) {
        h.entry(axum::http::header::CONTENT_SECURITY_POLICY)
            .or_insert(v);
    }
    resp
}

/// Build the Content-Security-Policy for Ruscker's own pages.
///
/// `extra_origins` (space-separated, may be empty) is appended to the
/// directives that fetch third-party resources (`script-src`,
/// `img-src`, `connect-src`). The landing uses this to allow an
/// operator-configured analytics provider without loosening the
/// policy for every other page. `'unsafe-inline'` on script/style is
/// still required by the inline landing/dashboard scripts (a
/// nonce-based CSP is a tracked follow-up).
pub(crate) fn content_security_policy(extra_origins: &str) -> String {
    // Sanitize operator-supplied origins before they reach the header:
    // a stray `*`, `'unsafe-eval'`, or a `;`-smuggled directive would
    // otherwise neuter the whole policy for every visitor.
    let e = sanitize_csp_origins(extra_origins);
    let extra = if e.is_empty() {
        String::new()
    } else {
        format!(" {e}")
    };
    format!(
        "default-src 'self'; \
         script-src 'self' 'unsafe-inline'{extra}; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:{extra}; \
         font-src 'self'; \
         connect-src 'self'{extra}; \
         frame-ancestors 'none'; \
         base-uri 'self'; \
         form-action 'self'"
    )
}

/// Keep only space-separated tokens that are safe CSP *host/scheme
/// sources*, dropping anything that could subvert the policy.
fn sanitize_csp_origins(raw: &str) -> String {
    raw.split_whitespace()
        .filter(|t| is_safe_csp_source(t))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A token is accepted only if it's a plain host source
/// (`[scheme://][*.]host[:port][/path]`, must contain a `.` or `:`) or a
/// scheme-only source (`https:`, `data:`). Rejected: the bare wildcard
/// `*`, any `'...'` keyword (`'unsafe-inline'`/`'unsafe-eval'`/…), and
/// anything carrying `;`/`,`/quotes/whitespace — i.e. directive
/// smuggling or policy-loosening keywords.
fn is_safe_csp_source(t: &str) -> bool {
    if t.is_empty()
        || t == "*"
        || t.contains(|c: char| c == ';' || c == ',' || c == '\'' || c.is_whitespace())
    {
        return false;
    }
    if let Some(scheme) = t.strip_suffix(':') {
        return !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    }
    let body = t
        .strip_prefix("https://")
        .or_else(|| t.strip_prefix("http://"))
        .unwrap_or(t);
    let body = body.strip_prefix("*.").unwrap_or(body);
    let valid = !body.is_empty()
        && body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '/' | '_'))
        && body
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric());
    // Must look like a host (has a dot or a port) — rejects bare words.
    valid && (body.contains('.') || body.contains(':'))
}

#[cfg(test)]
mod csp_tests {
    use super::content_security_policy;

    #[test]
    fn base_policy_is_self_only() {
        let csp = content_security_policy("");
        assert!(csp.contains("script-src 'self' 'unsafe-inline';"));
        assert!(csp.contains("connect-src 'self';"));
        assert!(!csp.contains("plausible"));
    }

    #[test]
    fn extra_origins_widen_script_img_connect() {
        let csp = content_security_policy("https://plausible.io");
        assert!(csp.contains("script-src 'self' 'unsafe-inline' https://plausible.io;"));
        assert!(csp.contains("img-src 'self' data: https://plausible.io;"));
        assert!(csp.contains("connect-src 'self' https://plausible.io;"));
        // Directives that must NOT be widened.
        assert!(csp.contains("style-src 'self' 'unsafe-inline';"));
        assert!(csp.contains("frame-ancestors 'none';"));
    }

    // #82: unsafe origin tokens must be stripped before reaching the
    // header so a careless/hostile entry can't neuter the landing CSP.
    #[test]
    fn dangerous_origins_are_dropped() {
        let csp = content_security_policy(
            "* 'unsafe-eval' 'unsafe-inline' evil.com;script-src https://ok.example.com",
        );
        // Only the one clean host source survives.
        assert!(csp.contains("https://ok.example.com"));
        assert!(!csp.contains('*'));
        assert!(!csp.contains("unsafe-eval"));
        // The `evil.com;script-src` token (directive smuggling) is gone.
        assert!(!csp.contains("evil.com"));
        // The base policy is intact.
        assert!(csp.contains("default-src 'self';"));
    }

    #[test]
    fn safe_sources_are_kept() {
        let csp = content_security_policy("https://a.example.com data: *.cdn.example.org");
        assert!(csp.contains("https://a.example.com"));
        assert!(csp.contains("data:"));
        assert!(csp.contains("*.cdn.example.org"));
    }
}
