//! Admin routes — login, logout, and the `/admin/*` subtree.
//!
//! All routes here either render the login form or assume an
//! authenticated session (via [`crate::auth::AdminSession`]). The
//! login form does NOT require auth; everything else does.

use askama::Template;
use axum::{
    extract::{Form, State},
    http::{header::REFERER, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use tower_cookies::cookie::time::Duration;
use tower_cookies::{Cookie, Cookies};

use crate::auth::{AdminSession, MaybeAdminSession, COOKIE_NAME};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub mod audit;
pub mod blocks;
pub mod credentials;
pub mod dashboard;
pub mod images;
pub mod landing;
pub mod logs;
pub mod spec_form;
pub mod specs;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin", get(redirect_to_dashboard))
        .route("/admin/login", get(login_form).post(login_submit))
        .route("/admin/logout", post(logout))
        .merge(dashboard::routes())
        .merge(specs::routes())
        .merge(spec_form::routes())
        .merge(images::routes())
        .merge(credentials::routes())
        .merge(landing::routes())
        .merge(blocks::routes())
        .merge(audit::routes())
        .merge(logs::routes())
}

// ── Templates ────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/login.html")]
struct LoginPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// "wrong-token" when the previous submit failed, otherwise empty.
    /// Drives the inline error banner.
    error: &'static str,
}

impl LoginPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

// ── Handlers ─────────────────────────────────────────────────────

async fn redirect_to_dashboard(_: AdminSession) -> Redirect {
    Redirect::to("/admin/dashboard")
}

async fn login_form(
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    let page = LoginPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        error: "",
    };
    render(&page)
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub token: String,
}

async fn login_submit(
    State(state): State<AppState>,
    cookies: Cookies,
    loc: Locale,
    theme: Theme,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    if !state.admin_auth.is_configured() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "RUSCKER_ADMIN_TOKEN is not set — admin disabled",
        )
            .into_response();
    }

    // Rate limit: bound brute force against the token. Saturated
    // window → 429 with Retry-After, before we even compare.
    if !state.login_limiter.allow() {
        let mut resp = (
            StatusCode::TOO_MANY_REQUESTS,
            "too many login attempts — try again shortly",
        )
            .into_response();
        resp.headers_mut()
            .insert(axum::http::header::RETRY_AFTER, "60".parse().unwrap());
        return resp;
    }

    // Match the token against each configured role token. `None`
    // ⇒ no token matched ⇒ failed login.
    let Some(role) = state.admin_auth.role_for(&form.token) else {
        // Count the failure toward the rate-limit window.
        state.login_limiter.record_failure();
        // Re-render the form with the error banner. 401 status so
        // CLI clients / tests can distinguish.
        let page = LoginPage {
            locale: loc,
            theme,
            locales: &state.locales,
            locales_all: &Locale::ALL,
            error: "wrong-token",
        };
        let body = match page.render() {
            Ok(s) => s,
            Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "render").into_response(),
        };
        let mut resp = (StatusCode::UNAUTHORIZED, Html(body)).into_response();
        // Failed login also clears any stale cookie.
        cookies.remove(Cookie::new(COOKIE_NAME, ""));
        // 401 keeps the URL at /admin/login so the user can retry.
        resp.headers_mut()
            .insert("X-Ruscker-Login", "failed".parse().unwrap());
        return resp;
    };

    // Success — clear the failure window so a few fat-fingered
    // attempts before getting it right don't linger.
    state.login_limiter.record_success();

    // Set the cookie. 24h lifetime; re-login required after that.
    // `Secure` is set only when the original request came over
    // HTTPS (signalled by the reverse proxy via X-Forwarded-Proto)
    // — setting it unconditionally would make the browser drop the
    // cookie on the plain-HTTP dev server.
    // Store an opaque session id in the cookie — never the token.
    // The session carries the matched role.
    let session_id = state.admin_sessions.create(role);
    let mut c = Cookie::new(COOKIE_NAME, session_id);
    c.set_path("/");
    c.set_http_only(true);
    c.set_same_site(tower_cookies::cookie::SameSite::Strict);
    c.set_secure(crate::auth::request_is_https(&headers));
    c.set_max_age(Duration::hours(24));
    cookies.add(c);

    // Redirect to the page the operator was trying to reach, reduced
    // to a same-origin path (no open redirect) — but only if this
    // role can actually reach it; otherwise land on the role's home
    // (the dashboard, which every role can see). This avoids bouncing
    // a Viewer straight into a 403.
    let referer = headers.get(REFERER).and_then(|v| v.to_str().ok());
    let path = super::same_origin_path(referer, role.home());
    let target = if path.starts_with("/admin/")
        && !path.starts_with("/admin/login")
        && role.can_access_section(section_for_admin_path(&path))
    {
        path
    } else {
        role.home().to_string()
    };
    Redirect::to(&target).into_response()
}

/// Map an `/admin/...` path to the `nav_section` it belongs to, for
/// role-gating the post-login redirect. Order matters: the per-replica
/// logs live under `/admin/dashboard/logs`, so that prefix must be
/// checked before the standalone daemon-log `/admin/logs`.
fn section_for_admin_path(path: &str) -> &'static str {
    if path.starts_with("/admin/dashboard") {
        "dashboard"
    } else if path.starts_with("/admin/specs") {
        "specs"
    } else if path.starts_with("/admin/media") {
        "images"
    } else if path.starts_with("/admin/credentials") {
        "credentials"
    } else if path.starts_with("/admin/landing") {
        "landing"
    } else if path.starts_with("/admin/blocks") {
        "blocks"
    } else if path.starts_with("/admin/audit") {
        "audit"
    } else if path.starts_with("/admin/logs") {
        "logs"
    } else {
        // /admin root and anything unrecognised → dashboard (every
        // role can reach it).
        "dashboard"
    }
}

async fn logout(_: MaybeAdminSession, State(state): State<AppState>, cookies: Cookies) -> Redirect {
    // Invalidate the session server-side so a copied cookie is dead too,
    // then clear the browser's copy.
    if let Some(c) = cookies.get(COOKIE_NAME) {
        state.admin_sessions.remove(c.value());
    }
    let mut c = Cookie::new(COOKIE_NAME, "");
    c.set_path("/");
    cookies.remove(c);
    Redirect::to("/admin/login")
}

/// Shared `Template → Response` (matches what landing.rs uses).
pub(crate) fn render<T: Template>(t: &T) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(error = ?err, "admin template render failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "template error").into_response()
        }
    }
}
