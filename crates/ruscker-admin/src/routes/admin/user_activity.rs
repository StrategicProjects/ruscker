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

use super::KpiMetric;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/activity", get(index))
        // The same table, restricted to SSO events, as the "Access" tab
        // of the SSO section (#1079).
        .route("/admin/sso-access", get(sso_index))
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
    fn event_norm(&self, sso: bool) -> String {
        match self.event.as_deref() {
            Some("sso.login") => "sso.login".into(),
            Some("sso.denied") => "sso.denied".into(),
            Some("sso.suppressed") => "sso.suppressed".into(),
            Some("audit.suppressed") if !sso => "audit.suppressed".into(),
            // The SSO tab only knows SSO events.
            Some("login.success") if !sso => "login.success".into(),
            Some("login.failed") if !sso => "login.failed".into(),
            Some("app.access") if !sso => "app.access".into(),
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
    kpi_logins: i64,
    kpi_app_accesses: i64,
    kpi_users: i64,
    /// `true` ⇒ this is the SSO section's "Access" tab: only `sso.*`
    /// events, its own title, sub-nav, cards and URL.
    sso: bool,
    kpi_sso_logins: i64,
    kpi_sso_denied: i64,
    kpi_sso_clients: i64,
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

    /// The page's own path — filter form, clear link and pager all use it.
    fn path(&self) -> &'static str {
        if self.sso {
            "/admin/sso-access"
        } else {
            "/admin/activity"
        }
    }
    fn title_key(&self) -> &'static str {
        if self.sso {
            "admin-sso-access-title"
        } else {
            "admin-activity-title"
        }
    }
    fn subtitle_key(&self) -> &'static str {
        if self.sso {
            "admin-sso-access-subtitle"
        } else {
            "admin-activity-subtitle"
        }
    }

    /// Localized reason of a failure / refusal row, or `""`.
    fn detail_label(&self, detail: &Option<String>) -> String {
        match detail.as_deref() {
            Some("wrong-password") => self.t("admin-activity-detail-wrong-password"),
            Some("unknown-account") => self.t("admin-activity-detail-unknown-account"),
            Some("not-in-group") => self.t("admin-activity-detail-not-in-group"),
            Some("no-account") => self.t("admin-activity-detail-no-account"),
            Some("account-missing") => self.t("admin-activity-detail-account-missing"),
            Some("access-revoked") => self.t("admin-activity-detail-access-revoked"),
            // On an `audit.suppressed` marker the detail names the class.
            Some("login.failed") => self.t("admin-activity-detail-suppressed-login"),
            Some("sso.denied") => self.t("admin-activity-detail-suppressed-sso"),
            Some(other) => other.to_string(),
            None => String::new(),
        }
    }

    /// Human label + tone/icon for an event type.
    fn event_label(&self, event_type: &str) -> String {
        match event_type {
            "login.success" => self.t("admin-activity-event-login"),
            "app.access" => self.t("admin-activity-event-access"),
            "sso.login" => self.t("admin-activity-event-sso"),
            "sso.denied" => self.t("admin-activity-event-sso-denied"),
            "login.failed" => self.t("admin-activity-event-login-failed"),
            "audit.suppressed" | "sso.suppressed" => self.t("admin-activity-event-suppressed"),
            other => other.to_string(),
        }
    }
    fn event_icon(&self, event_type: &str) -> &'static str {
        match event_type {
            "login.success" => "ti-login",
            "app.access" => "ti-app-window",
            "sso.login" => "ti-key",
            "sso.denied" | "login.failed" => "ti-alert-circle",
            "audit.suppressed" | "sso.suppressed" => "ti-alert-triangle",
            _ => "ti-history",
        }
    }
    fn event_tone(&self, event_type: &str) -> &'static str {
        match event_type {
            "login.success" => "tone-update",
            "app.access" => "tone-create",
            "sso.login" => "tone-update",
            "sso.denied" | "login.failed" | "audit.suppressed" | "sso.suppressed" => "tone-delete",
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

    fn kpis(&self) -> [KpiMetric; 3] {
        if self.sso {
            return [
                KpiMetric::new("ti-key", "admin-sso-access-kpi-logins", self.kpi_sso_logins),
                KpiMetric::new("ti-alert-circle", "admin-sso-access-kpi-denied", self.kpi_sso_denied),
                KpiMetric::new("ti-login", "admin-sso-access-kpi-clients", self.kpi_sso_clients),
            ];
        }
        [
            KpiMetric::new("ti-login", "admin-activity-kpi-logins", self.kpi_logins),
            KpiMetric::new(
                "ti-app-window",
                "admin-activity-kpi-app-accesses",
                self.kpi_app_accesses,
            ),
            KpiMetric::new("ti-users", "admin-activity-kpi-users", self.kpi_users),
        ]
    }
}

async fn index(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(q): Query<ActivityQuery>,
) -> Response {
    render_activity(&state, loc, theme, q, false).await
}

/// SSO section → "Access": sign-ins to external systems and refusals.
async fn sso_index(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(q): Query<ActivityQuery>,
) -> Response {
    render_activity(&state, loc, theme, q, true).await
}

async fn render_activity(
    state: &AppState,
    loc: Locale,
    theme: Theme,
    q: ActivityQuery,
    sso: bool,
) -> Response {
    let Some(db) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };

    let event = q.event_norm(sso);
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
        event_prefix: sso.then_some("sso."),
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

    let users = if sso {
        db::user_activity::distinct_usernames_with_prefix(db, "sso.").await
    } else {
        db::user_activity::distinct_usernames(db).await
    }
    .unwrap_or_default();
    let apps = if sso {
        db::user_activity::distinct_specs_with_prefix(db, "sso.").await
    } else {
        db::user_activity::distinct_specs(db).await
    }
    .unwrap_or_default();
    let mut kpi_filter = filter.clone();
    // The general page's cards. Skipped on the SSO tab, where the prefix
    // makes both counts zero by construction.
    let (mut kpi_logins, mut kpi_app_accesses) = (0, 0);
    if !sso {
        kpi_filter.event_type = Some("login.success".into());
        kpi_logins = match db::user_activity::count_filtered(db, &kpi_filter).await {
            Ok(count) => count,
            Err(e) => {
                tracing::error!(error = ?e, "count login activity failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        };
        kpi_filter.event_type = Some("app.access".into());
        kpi_app_accesses = match db::user_activity::count_filtered(db, &kpi_filter).await {
            Ok(count) => count,
            Err(e) => {
                tracing::error!(error = ?e, "count app access activity failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        };
    }
    let kpi_users = users.len() as i64;

    // SSO cards: sign-ins and refusals under the active user / system /
    // period filters (not the event filter — the two cards ARE the
    // events), and how many client systems are enabled.
    let (mut kpi_sso_logins, mut kpi_sso_denied, mut kpi_sso_clients) = (0, 0, 0);
    if sso {
        kpi_filter.event_prefix = None;
        kpi_filter.event_type = Some("sso.login".into());
        kpi_sso_logins = db::user_activity::count_filtered(db, &kpi_filter).await.unwrap_or(0);
        kpi_filter.event_type = Some("sso.denied".into());
        kpi_sso_denied = db::user_activity::count_filtered(db, &kpi_filter).await.unwrap_or(0);
        kpi_sso_clients = db::oidc_clients::list_all(db)
            .await
            .map(|c| c.iter().filter(|c| c.enabled).count() as i64)
            .unwrap_or(0);
    }

    let page = ActivityPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: if sso { "sso" } else { "audit" },
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
        kpi_logins,
        kpi_app_accesses,
        kpi_users,
        sso,
        kpi_sso_logins,
        kpi_sso_denied,
        kpi_sso_clients,
    };
    super::render(&page)
}

/// A short, human "browser · system" for a raw `User-Agent`, e.g.
/// `Chrome · macOS`. Best effort and deliberately coarse: the full
/// string stays available as the cell's tooltip, and nothing here is
/// used for a decision. Order matters — Edge and Opera also say
/// "Chrome", Chrome also says "Safari".
pub(crate) fn browser_summary(ua: &str) -> String {
    let has = |needle: &str| ua.contains(needle);
    let browser = if has("Edg/") || has("EdgA/") || has("EdgiOS/") {
        "Edge"
    } else if has("OPR/") || has("Opera") {
        "Opera"
    } else if has("Firefox/") || has("FxiOS/") {
        "Firefox"
    } else if has("Chrome/") || has("CriOS/") {
        "Chrome"
    } else if has("Safari/") {
        "Safari"
    } else if has("curl/") {
        "curl"
    } else {
        ""
    };
    let system = if has("Android") {
        "Android"
    } else if has("iPhone") || has("iPad") {
        "iOS"
    } else if has("Windows") {
        "Windows"
    } else if has("Mac OS X") || has("Macintosh") {
        "macOS"
    } else if has("Linux") || has("X11") {
        "Linux"
    } else {
        ""
    };
    match (browser, system) {
        ("", "") => ua.chars().take(40).collect(),
        (b, "") => b.to_string(),
        ("", s) => s.to_string(),
        (b, s) => format!("{b} · {s}"),
    }
}

#[cfg(test)]
mod tests {
    use super::browser_summary;

    #[test]
    fn browser_summary_names_the_usual_suspects() {
        let cases = [
            ("Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36", "Chrome · macOS"),
            ("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36 Edg/141.0.0.0", "Edge · Windows"),
            ("Mozilla/5.0 (X11; Linux x86_64; rv:152.0) Gecko/20100101 Firefox/152.0", "Firefox · Linux"),
            ("Mozilla/5.0 (iPhone; CPU iPhone OS 19_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/19.0 Mobile/15E148 Safari/604.1", "Safari · iOS"),
            ("Mozilla/5.0 (Linux; Android 16; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Mobile Safari/537.36", "Chrome · Android"),
            ("curl/8.7.1", "curl"),
        ];
        for (ua, want) in cases {
            assert_eq!(browser_summary(ua), want, "{ua}");
        }
        // Unknown agents fall back to a short prefix of the raw string.
        assert_eq!(browser_summary("my-backend/1.0"), "my-backend/1.0");
        assert_eq!(browser_summary(&"x".repeat(100)).len(), 40);
    }
}
