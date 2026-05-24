//! Endpoints for setting per-user preferences (theme + locale).
//!
//! These are POST-only and respond with 303 See Other to the
//! `Referer` header (falling back to `/`). The HTML selectors in the
//! footer post a tiny form here instead of using JavaScript — works
//! without scripting and degrades gracefully.

use axum::{
    extract::Form,
    http::{header::REFERER, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::post,
    Router,
};
use serde::Deserialize;
use tower_cookies::cookie::time::Duration;
use tower_cookies::{Cookie, Cookies};

use crate::i18n::{self, Locale};
use crate::theme::{self, Theme};
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/__set/locale", post(set_locale))
        .route("/__set/theme", post(set_theme))
}

#[derive(Debug, Deserialize)]
pub struct LocaleForm {
    pub locale: String,
}

#[derive(Debug, Deserialize)]
pub struct ThemeForm {
    pub theme: String,
}

async fn set_locale(
    cookies: Cookies,
    headers: HeaderMap,
    Form(form): Form<LocaleForm>,
) -> Response {
    let Some(loc) = Locale::parse(&form.locale) else {
        return (StatusCode::BAD_REQUEST, "unknown locale").into_response();
    };
    cookies.add(persistent_cookie(i18n::COOKIE_NAME, loc.code().to_string()));
    redirect_back(&headers).into_response()
}

async fn set_theme(
    cookies: Cookies,
    headers: HeaderMap,
    Form(form): Form<ThemeForm>,
) -> Response {
    let theme = Theme::parse(&form.theme);
    match theme.cookie_value() {
        Some(val) => cookies.add(persistent_cookie(theme::COOKIE_NAME, val.to_string())),
        // Auto = no explicit choice; remove the cookie so the OS
        // preference takes over.
        None => cookies.remove(Cookie::new(theme::COOKIE_NAME, "")),
    }
    redirect_back(&headers).into_response()
}

/// Build a cookie that survives across visits. 365 days is plenty
/// for a UI preference; nothing here is security-sensitive.
fn persistent_cookie(name: &'static str, value: String) -> Cookie<'static> {
    let mut c = Cookie::new(name, value);
    c.set_path("/");
    c.set_http_only(true);
    c.set_same_site(tower_cookies::cookie::SameSite::Lax);
    c.set_max_age(Duration::days(365));
    c
}

fn redirect_back(headers: &HeaderMap) -> Redirect {
    // Same-origin only: an attacker-controlled `Referer` must not
    // turn this 303 into an open redirect off our origin.
    let referer = headers.get(REFERER).and_then(|v| v.to_str().ok());
    Redirect::to(&super::same_origin_path(referer, "/"))
}
