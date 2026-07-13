//! Admin **System** tab (#766) — a read-only diagnostic of the running
//! server's config + runtime state, plus the one operational control
//! that belongs here: the **alert webhook** (#930 — URL + send-test).
//! Restarting stays an operator/systemd action, not an in-app button —
//! the page just shows the command. Admin-only.

use askama::Template;
use axum::{
    extract::{Form, Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;

use crate::auth::{RequireAdmin, Role};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/system", get(index))
        .route("/admin/system/alerts", post(save_alerts))
        .route("/admin/system/alerts/test", post(test_alert))
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
    /// Alert webhook URL from the settings table (#930); empty = off.
    webhook_url: String,
    /// "" | "alerts-saved" | "alerts-bad-url" | "alerts-test" |
    /// "alerts-no-url"
    flash: &'static str,
}

impl SystemPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

#[derive(Debug, Deserialize)]
struct SystemQuery {
    flash: Option<String>,
}

async fn index(
    admin: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(q): Query<SystemQuery>,
) -> Response {
    let (docker_connected, docker_version) = match state.backend.as_ref() {
        Some(b) => (true, b.backend_version().await.ok().flatten()),
        None => (false, None),
    };
    let db_kind = match state.db.as_ref() {
        None => "none",
        Some(crate::db::ConfigDb::Sqlite(_)) => "sqlite",
        Some(crate::db::ConfigDb::Postgres(_)) => "postgres",
    };
    let spec_count = crate::catalog::effective_specs_cached(&state)
        .await
        .len();
    let replica_count = state.replicas.read().await.all().count();
    let base_path = if state.base_path.is_empty() {
        "/".to_string()
    } else {
        state.base_path.to_string()
    };
    let webhook_url = match state.db.as_ref() {
        Some(db) => crate::db::settings::get(db, crate::db::settings::ALERT_WEBHOOK_URL)
            .await
            .ok()
            .flatten()
            .unwrap_or_default(),
        None => String::new(),
    };
    let flash = match q.flash.as_deref() {
        Some("alerts-saved") => "alerts-saved",
        Some("alerts-bad-url") => "alerts-bad-url",
        Some("alerts-test") => "alerts-test",
        Some("alerts-no-url") => "alerts-no-url",
        _ => "",
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
        webhook_url,
        flash,
    };
    super::render(&page).into_response()
}

#[derive(Debug, Deserialize)]
struct AlertsForm {
    #[serde(default)]
    webhook_url: String,
}

/// Accept empty (webhook off) or an absolute `http(s)://` URL. Nothing
/// fancier — the send-test button is the real validation.
fn webhook_url_acceptable(url: &str) -> bool {
    url.is_empty() || url.starts_with("https://") || url.starts_with("http://")
}

async fn save_alerts(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Form(form): Form<AlertsForm>,
) -> Response {
    let Some(db) = state.db.as_ref() else {
        return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let url = form.webhook_url.trim();
    if !webhook_url_acceptable(url) {
        return Redirect::to("/admin/system?flash=alerts-bad-url").into_response();
    }
    if let Err(e) = crate::db::settings::set(
        db,
        crate::db::settings::ALERT_WEBHOOK_URL,
        url,
        Some(admin.actor()),
    )
    .await
    {
        tracing::error!(error = ?e, "save alert webhook failed");
        return (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
    }
    Redirect::to("/admin/system?flash=alerts-saved").into_response()
}

async fn test_alert(
    admin: RequireAdmin,
    State(state): State<AppState>,
) -> Response {
    let Some(db) = state.db.as_ref() else {
        return (axum::http::StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let configured = crate::db::settings::get(db, crate::db::settings::ALERT_WEBHOOK_URL)
        .await
        .ok()
        .flatten()
        .is_some_and(|u| !u.trim().is_empty());
    if !configured {
        return Redirect::to("/admin/system?flash=alerts-no-url").into_response();
    }
    state.alerts.notify(crate::alerts::AlertEvent {
        kind: crate::alerts::AlertKind::Test,
        spec: "-".to_string(),
        replica: None,
        message: format!("test alert sent by {}", admin.actor()),
    });
    Redirect::to("/admin/system?flash=alerts-test").into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn webhook_url_validation() {
        assert!(webhook_url_acceptable(""));
        assert!(webhook_url_acceptable("https://hooks.slack.com/x"));
        assert!(webhook_url_acceptable("http://10.0.0.5:9000/hook"));
        assert!(!webhook_url_acceptable("ftp://nope"));
        assert!(!webhook_url_acceptable("hooks.slack.com/x"));
        assert!(!webhook_url_acceptable("javascript:alert(1)"));
    }
}
