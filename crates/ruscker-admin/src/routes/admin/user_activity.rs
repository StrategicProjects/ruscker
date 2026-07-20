//! Admin > Atividades dos usuários (#1021).
//!
//! Read-only listing of the `user_activity` table (logins + interactive
//! app accesses, with identity), with filters for event kind, user, app,
//! and time window, and server-side pagination. Sibling to the
//! administrative audit log (`/admin/audit`); a two-pill sub-nav switches
//! between them under the "Atividades" section.

use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use chrono::{Duration, Utc};
use serde::Deserialize;

use crate::auth::{RequireAdmin, Role};
use crate::db::{self, user_activity::ActivityFilter, user_activity::ActivityRow};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/admin/activity", get(index))
}

/// Rows per page — server-side paginated like the users table (#999) so a
/// long history doesn't fetch/render thousands of rows.
const ACTIVITY_PER_PAGE: i64 = 50;

#[derive(Debug, Deserialize, Default)]
pub struct ActivityQuery {
    /// "" (all) | "login.success" | "app.access".
    #[serde(default)]
    pub event: Option<String>,
    /// Exact username (from the dropdown of distinct users).
    #[serde(default)]
    pub user: Option<String>,
    /// Exact spec id (from the dropdown of distinct apps).
    #[serde(default)]
    pub app: Option<String>,
    /// "" (all) | "24h" | "7d" | "30d".
    #[serde(default)]
    pub period: Option<String>,
    /// 1-based page; out-of-range clamps.
    #[serde(default)]
    pub page: Option<i64>,
}

impl ActivityQuery {
    /// The event filter normalized to a recognized value or `""` (all) — an
    /// unknown/forged value is treated as "all", never passed to the query.
    fn event_norm(&self) -> String {
        match self.event.as_deref() {
            Some("login.success") => "login.success".into(),
            Some("app.access") => "app.access".into(),
            _ => String::new(),
        }
    }
    fn user_norm(&self) -> String {
        self.user.as_deref().unwrap_or("").trim().to_string()
    }
    fn app_norm(&self) -> String {
        self.app.as_deref().unwrap_or("").trim().to_string()
    }
    /// Period normalized to a recognized preset or `""` (all).
    fn period_norm(&self) -> String {
        match self.period.as_deref() {
            Some("24h") => "24h".into(),
            Some("7d") => "7d".into(),
            Some("30d") => "30d".into(),
            _ => String::new(),
        }
    }
}

#[derive(Template)]
#[template(path = "admin/activity.html")]
struct ActivityPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// Mount prefix for base-path-correct URLs (#294).
    base: std::sync::Arc<str>,
    /// "audit" so the top-nav "Atividades" stays highlighted for both the
    /// users and the administrative views.
    nav_section: &'static str,
    role: Role,
    rows: Vec<ActivityRow>,
    /// Distinct values for the user / app dropdowns.
    users: Vec<String>,
    apps: Vec<String>,
    /// Echoed filter state (also carried on the pager links).
    event: String,
    user: String,
    app: String,
    period: String,
    /// Server-side pagination state (1-based page, total pages, match count).
    page: i64,
    pages: i64,
    total: i64,
}

impl ActivityPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// Localized "Página X de Y · N registros" line under the table.
    fn pager_status(&self) -> String {
        use fluent_bundle::FluentArgs;
        let mut args = FluentArgs::new();
        args.set("page", self.page);
        args.set("pages", self.pages);
        args.set("total", self.total);
        self.locales
            .t(self.locale, "admin-activity-pager-status", Some(&args))
    }

    /// Human label + tone/icon for an event type.
    fn event_label(&self, event_type: &str) -> String {
        match event_type {
            "login.success" => self.t("admin-activity-event-login"),
            "app.access" => self.t("admin-activity-event-access"),
            other => other.to_string(),
        }
    }
    fn event_icon(&self, event_type: &str) -> &'static str {
        match event_type {
            "login.success" => "ti-login",
            "app.access" => "ti-app-window",
            _ => "ti-history",
        }
    }
    fn event_tone(&self, event_type: &str) -> &'static str {
        match event_type {
            "login.success" => "tone-update",
            "app.access" => "tone-create",
            _ => "tone-other",
        }
    }

    /// The active filters as a URL query suffix (`&event=…&user=…`), for the
    /// pager links. Values are percent-encoded; empty filters are omitted.
    fn query_suffix(&self) -> String {
        let mut s = String::new();
        for (k, v) in [
            ("event", &self.event),
            ("user", &self.user),
            ("app", &self.app),
            ("period", &self.period),
        ] {
            if !v.is_empty() {
                s.push('&');
                s.push_str(k);
                s.push('=');
                s.push_str(&urlencoding::encode(v));
            }
        }
        s
    }

    /// Whether any filter is active (drives the "clear" link).
    fn filtered(&self) -> bool {
        !self.event.is_empty()
            || !self.user.is_empty()
            || !self.app.is_empty()
            || !self.period.is_empty()
    }
}

async fn index(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(q): Query<ActivityQuery>,
) -> Response {
    let Some(db) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };

    let event = q.event_norm();
    let user = q.user_norm();
    let app = q.app_norm();
    let period = q.period_norm();
    let since = match period.as_str() {
        "24h" => Some(Utc::now() - Duration::hours(24)),
        "7d" => Some(Utc::now() - Duration::days(7)),
        "30d" => Some(Utc::now() - Duration::days(30)),
        _ => None,
    };

    let mut filter = ActivityFilter {
        event_type: (!event.is_empty()).then(|| event.clone()),
        username: (!user.is_empty()).then(|| user.clone()),
        spec_id: (!app.is_empty()).then(|| app.clone()),
        since,
        until: None,
        limit: ACTIVITY_PER_PAGE,
        offset: 0,
    };

    let total = match db::user_activity::count_filtered(db, &filter).await {
        Ok(n) => n,
        Err(e) => {
            tracing::error!(error = ?e, "count user_activity failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    // Ceiling division (i64::div_ceil is still unstable on our floor).
    let pages = ((total + ACTIVITY_PER_PAGE - 1) / ACTIVITY_PER_PAGE).max(1);
    let page = q.page.unwrap_or(1).clamp(1, pages);
    filter.offset = (page - 1) * ACTIVITY_PER_PAGE;

    let rows = match db::user_activity::list_page(db, &filter).await {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = ?e, "list user_activity failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };

    let users = db::user_activity::distinct_usernames(db).await.unwrap_or_default();
    let apps = db::user_activity::distinct_specs(db).await.unwrap_or_default();

    let page = ActivityPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "audit",
        role: Role::Admin,
        rows,
        users,
        apps,
        event,
        user,
        app,
        period,
        page,
        pages,
        total,
    };
    super::render(&page)
}
