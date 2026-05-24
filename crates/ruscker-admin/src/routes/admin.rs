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

pub mod credentials;
pub mod images;
pub mod spec_form;
pub mod specs;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin", get(redirect_to_specs))
        .route("/admin/login", get(login_form).post(login_submit))
        .route("/admin/logout", post(logout))
        .merge(specs::routes())
        .merge(spec_form::routes())
        .merge(images::routes())
        .merge(credentials::routes())
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

async fn redirect_to_specs(_: AdminSession) -> Redirect {
    Redirect::to("/admin/specs")
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

    if !state.admin_auth.matches(&form.token) {
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
    }

    // Success — set the cookie. 24h lifetime; re-login required
    // after that. Long enough that operators don't have to log in
    // multiple times per day, short enough that a left-open tab on
    // a shared device expires within a workday.
    let mut c = Cookie::new(COOKIE_NAME, form.token);
    c.set_path("/");
    c.set_http_only(true);
    c.set_same_site(tower_cookies::cookie::SameSite::Strict);
    c.set_max_age(Duration::hours(24));
    cookies.add(c);

    // Redirect to the page the operator was trying to reach. The
    // login form preserves the destination in a `?next=` param —
    // not implemented yet, so default to /admin/specs.
    let target = headers
        .get(REFERER)
        .and_then(|v| v.to_str().ok())
        .filter(|r| r.contains("/admin/") && !r.contains("/admin/login"))
        .unwrap_or("/admin/specs");
    Redirect::to(target).into_response()
}

async fn logout(_: MaybeAdminSession, cookies: Cookies) -> Redirect {
    // Remove sets an expired cookie with the same name+path.
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
