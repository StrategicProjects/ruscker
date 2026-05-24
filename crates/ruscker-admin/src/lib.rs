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
pub mod metrics_cache;
pub mod routes;
pub mod scaler;
pub mod sessions;
pub mod theme;
pub mod view_model;

use sqlx::SqlitePool;

/// Shared state injected into every request.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub locales: Arc<i18n::Locales>,
    pub admin_auth: auth::AdminAuth,
    /// Global rate limiter for `/admin/login` — bounds brute
    /// force against the admin token. Shared (Arc) so every
    /// cloned `AppState` sees the same window.
    pub login_limiter: Arc<auth::LoginRateLimiter>,
    /// Optional SQLite pool. `None` ⇒ admin CRUD routes 503
    /// because they have no source of truth to read or write.
    pub db: Option<SqlitePool>,
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
    pub sessions: std::sync::Arc<sessions::SessionTracker>,

    /// Per-replica metrics cache. Filled by a background
    /// refresher that fans out `backend.metrics()` calls every
    /// [`metrics_cache::REFRESH_INTERVAL`]; the dashboard reads
    /// straight from here without ever waiting on Docker.
    pub metrics: metrics_cache::MetricsCache,
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
            admin_auth: auth::AdminAuth::from_env(),
            login_limiter: Arc::new(auth::LoginRateLimiter::default_policy()),
            db: None,
            images_dir: None,
            master_key: crypto::MasterKey::from_env().context("load master key")?,
            backend: None,
            replicas: std::sync::Arc::new(tokio::sync::RwLock::new(
                ruscker_core::ReplicaRegistry::new(),
            )),
            cookie_key: ruscker_proxy::sticky::CookieKey::from_env_or_random()
                .context("load sticky cookie key")?,
            spawn_locks: std::sync::Arc::new(dashmap::DashMap::new()),
            sessions: std::sync::Arc::new(sessions::SessionTracker::new()),
            metrics: metrics_cache::MetricsCache::new(),
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

    /// Override the admin token (default: pulled from
    /// `RUSCKER_ADMIN_TOKEN` env var). Useful for tests that need
    /// a known token without touching the process environment.
    pub fn with_admin_token(mut self, token: impl Into<String>) -> Self {
        self.state.admin_auth = auth::AdminAuth::with_token(token);
        self
    }

    /// Override the credentials master key (default: pulled from
    /// `RUSCKER_MASTER_KEY`). Accepts hex (64ch) or base64 (44ch).
    pub fn with_master_key(mut self, raw: impl AsRef<str>) -> Result<Self> {
        self.state.master_key = crypto::MasterKey::from_str(raw.as_ref())?;
        Ok(self)
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

    /// Attach a SQLite pool. Required for the `/admin/*` routes
    /// that read or write the spec catalog. The pool is shared
    /// across all requests — sqlx handles the connection
    /// multiplexing.
    pub fn with_db(mut self, pool: SqlitePool) -> Self {
        self.state.db = Some(pool);
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
            let _ = scaler::spawn(self.state.clone(), scaler::DEFAULT_INTERVAL);
            // Session sweeper: evicts idle sessions per the
            // global `heartbeat-timeout`. `-1` (the ShinyProxy
            // idiom for "never expire") becomes a no-op loop
            // inside `sessions::spawn` so the call shape stays
            // uniform.
            let _ = sessions::spawn(
                self.state.sessions.clone(),
                self.state.replicas.clone(),
                self.state.config.clone(),
            );
            // Dashboard metrics: keep `state.metrics` fresh in
            // the background so dashboard renders are read-only
            // and never block on a Docker stats call.
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
        axum::serve(listener, app)
            .await
            .context("axum serve")?;
        Ok(())
    }
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

    own.merge(routes::proxy::routes())
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
        (
            "content-security-policy",
            "default-src 'self'; \
             script-src 'self' 'unsafe-inline'; \
             style-src 'self' 'unsafe-inline'; \
             img-src 'self' data:; \
             font-src 'self'; \
             connect-src 'self'; \
             frame-ancestors 'none'; \
             base-uri 'self'; \
             form-action 'self'",
        ),
    ];
    for (name, value) in defaults {
        if let Ok(hn) = HeaderName::from_bytes(name.as_bytes()) {
            h.entry(hn).or_insert(HeaderValue::from_static(value));
        }
    }
    resp
}
