//! Admin **System** tab (#766) — a read-only diagnostic of the running
//! server's config + runtime state. No edit controls (restarting is an
//! operator/systemd action, not an in-app button — the page just shows
//! the command). Admin-only.

use askama::Template;
use axum::{extract::State, response::Response, routing::get, Router};

use crate::auth::{RequireAdmin, Role};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/admin/system", get(index))
}

#[derive(Template)]
#[template(path = "admin/system.html")]
struct SystemPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    /// Ruscker version (`CARGO_PKG_VERSION`).
    version: &'static str,
    /// Mount prefix, or `/` at the root.
    base_path: String,
    /// Configured listen address (`bind-address:port`; `--bind` may
    /// override it at launch).
    bind: String,
    /// Docker backend wired in?
    docker_connected: bool,
    /// Docker daemon version, when readable.
    docker_version: Option<String>,
    /// `none` | `sqlite` | `postgres`.
    db_kind: &'static str,
    /// Effective catalog size (DB ∪ YAML).
    spec_count: usize,
    /// Running replicas across all specs right now.
    replica_count: usize,
    /// `server.useForwardHeaders` trust (drives Secure cookie + XFF).
    forward_headers: bool,
    /// `proxy.metrics-enabled` (the unauthenticated `/metrics`).
    metrics_enabled: bool,
    /// HA leader (always true on a single node).
    is_leader: bool,
    /// Graceful-shutdown drain in progress.
    draining: bool,
}

impl SystemPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

async fn index(
    admin: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    use axum::response::IntoResponse;

    let (docker_connected, docker_version) = match state.backend.as_ref() {
        Some(b) => (true, b.backend_version().await.ok().flatten()),
        None => (false, None),
    };
    let db_kind = match state.db.as_ref() {
        None => "none",
        Some(crate::db::ConfigDb::Sqlite(_)) => "sqlite",
        Some(crate::db::ConfigDb::Postgres(_)) => "postgres",
    };
    let spec_count = crate::catalog::effective_specs(state.db.as_ref(), &state.config)
        .await
        .len();
    let replica_count = state.replicas.read().await.all().count();
    let base_path = if state.base_path.is_empty() {
        "/".to_string()
    } else {
        state.base_path.to_string()
    };

    let page = SystemPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "system",
        role: admin.role,
        version: crate::APP_VERSION,
        base_path,
        bind: format!(
            "{}:{}",
            state.config.proxy.bind_address, state.config.proxy.port
        ),
        docker_connected,
        docker_version,
        db_kind,
        spec_count,
        replica_count,
        forward_headers: crate::routes::proxy::forward_headers_trusted(&state.config.server),
        metrics_enabled: state.config.proxy.metrics_enabled,
        is_leader: state.leader.is_leader().await,
        draining: state
            .draining
            .load(std::sync::atomic::Ordering::Relaxed),
    };
    super::render(&page).into_response()
}
