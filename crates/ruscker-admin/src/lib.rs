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
use tower_http::services::ServeDir;
use tracing::info;

pub mod auth;
pub mod db;
pub mod i18n;
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
        };
        Ok(Self {
            addr,
            state,
            images_dir: None,
        })
    }

    /// Set the on-disk directory served at `/assets/img/`. Files
    /// must already exist when the server starts — `ServeDir`
    /// responds 404 for missing files, never directory-lists.
    pub fn with_images_dir(mut self, dir: impl AsRef<Path>) -> Self {
        self.images_dir = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Override the admin token (default: pulled from
    /// `RUSCKER_ADMIN_TOKEN` env var). Useful for tests that need
    /// a known token without touching the process environment.
    pub fn with_admin_token(mut self, token: impl Into<String>) -> Self {
        self.state.admin_auth = auth::AdminAuth::with_token(token);
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

/// Same as [`router`], but also mounts `images_dir` at
/// `/assets/img/` via [`ServeDir`]. `None` skips the mount entirely.
pub fn router_with_images(state: AppState, images_dir: Option<&Path>) -> Router {
    let mut r = Router::new()
        .merge(routes::landing::routes())
        .merge(routes::assets::routes())
        .merge(routes::prefs::routes())
        .merge(routes::admin::routes());
    if let Some(dir) = images_dir {
        // ServeDir handles 404, content-type sniffing, and range
        // requests. Specific routes in routes::assets (e.g.
        // /assets/styles.css) take precedence — only the `img`
        // subtree is delegated to disk.
        r = r.nest_service("/assets/img", ServeDir::new(dir));
    }
    r.layer(CookieManagerLayer::new()).with_state(state)
}
