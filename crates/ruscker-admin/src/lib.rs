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
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_cookies::CookieManagerLayer;
use tracing::info;

pub mod i18n;
pub mod routes;
pub mod theme;
pub mod view_model;

/// Shared state injected into every request.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub locales: Arc<i18n::Locales>,
}

/// HTTP server hosting the landing and (later) the admin panel.
pub struct AdminServer {
    addr: SocketAddr,
    state: AppState,
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
        };
        Ok(Self { addr, state })
    }

    /// Start listening. Blocks until the process is shut down.
    pub async fn run(self) -> Result<()> {
        let app = router(self.state.clone());
        let listener = TcpListener::bind(self.addr)
            .await
            .with_context(|| format!("bind {}", self.addr))?;
        info!(addr = %self.addr, "ruscker-admin listening");
        axum::serve(listener, app)
            .await
            .context("axum serve")?;
        Ok(())
    }
}

/// Compose the axum router. Pulled out so tests can hit it via
/// `Router::oneshot` without a real socket.
pub fn router(state: AppState) -> Router {
    Router::new()
        .merge(routes::landing::routes())
        .layer(CookieManagerLayer::new())
        .with_state(state)
}
