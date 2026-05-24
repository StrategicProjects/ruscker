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
pub mod routes;
pub mod theme;
pub mod view_model;

use sqlx::SqlitePool;

/// Shared state injected into every request.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub locales: Arc<i18n::Locales>,
    pub admin_auth: auth::AdminAuth,
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
            db: None,
            images_dir: None,
            master_key: crypto::MasterKey::from_env().context("load master key")?,
            backend: None,
            replicas: std::sync::Arc::new(tokio::sync::RwLock::new(
                ruscker_core::ReplicaRegistry::new(),
            )),
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
    Router::new()
        .merge(routes::landing::routes())
        .merge(routes::assets::routes())
        .merge(routes::prefs::routes())
        .merge(routes::admin::routes())
        .merge(routes::proxy::routes())
        .layer(CookieManagerLayer::new())
        .with_state(state)
}
