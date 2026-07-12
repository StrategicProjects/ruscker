//! Admin routes — login, logout, and the `/admin/*` subtree.
//!
//! All routes here either render the login form or assume an
//! authenticated session (via [`crate::auth::AdminSession`]). The
//! login form does NOT require auth; everything else does.

use askama::Template;
use axum::{
    extract::{Form, Query, State},
    http::{header::REFERER, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use tower_cookies::cookie::time::Duration;
use tower_cookies::{Cookie, Cookies};

use crate::auth::{AdminSession, MaybeAdminSession, RequireAdmin, Role, COOKIE_NAME};
use crate::db;
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub mod audit;
pub mod blocks;
pub mod credentials;
pub mod dashboard;
pub mod disk;
pub mod groups;
pub mod images;
pub mod landing;
pub mod logs;
pub mod spec_form;
pub mod specs;
pub mod system;
pub mod users;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin", get(redirect_to_dashboard))
        // Username/password login is the primary path; the admin token
        // is a break-glass that always grants an Admin session (and
        // bootstraps the first account).
        .route("/admin/login", get(login_form).post(login_submit))
        .route("/admin/login/token", post(token_login))
        .route("/admin/logout", post(logout))
        // First-admin bootstrap (only when no admin user exists).
        .route("/admin/setup", get(setup_form).post(setup_submit))
        // Self-service password change + first-login prompt.
        .route(
            "/admin/account/password",
            get(password_form).post(password_submit),
        )
        .merge(dashboard::routes())
        .merge(disk::routes())
        .merge(specs::routes())
        .merge(spec_form::routes())
        .merge(images::routes())
        .merge(credentials::routes())
        .merge(landing::routes())
        .merge(blocks::routes())
        .merge(audit::routes())
        .merge(groups::routes())
        .merge(logs::routes())
        .merge(users::routes())
        .merge(system::routes())
}

// ── Templates ────────────────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/login.html")]
struct LoginPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// URL prefix the portal is mounted under (`""` at root, `/box`
    /// under `--base-path`). Templates emit `{{ base }}/admin/...` so the
    /// HTML is already correct and needs no response-body rewrite (#294).
    base: std::sync::Arc<str>,
    /// Inline error banner key: "" | "wrong-login" | "wrong-token".
    error: &'static str,
    /// `true` ⇒ render the break-glass **token** form (bootstrap or
    /// `?token=1`); `false` ⇒ the username/password form.
    bootstrap: bool,
    /// Validated root-relative app destination carried through the form.
    next: String,
    /// Percent-encoded twin used by the password/token login switch links.
    next_encoded: String,
}

impl LoginPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

#[derive(Template)]
#[template(path = "admin/setup.html")]
struct SetupPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// Mount prefix for base-path-correct URLs (#294); see [`LoginPage`].
    base: std::sync::Arc<str>,
    /// "" | "mismatch" | "short" | "exists" | "db".
    error: &'static str,
}

impl SetupPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

#[derive(Template)]
#[template(path = "admin/account_password.html")]
struct AccountPasswordPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// Mount prefix for base-path-correct URLs (#294); see [`LoginPage`].
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    /// First-login prompt — shows the "keep current password" skip.
    first: bool,
    /// "" | "wrong-current" | "mismatch" | "short".
    error: &'static str,
    /// Validated app destination carried through a mandatory first-login
    /// password change.
    next: String,
}

impl AccountPasswordPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

// ── Handlers ─────────────────────────────────────────────────────

async fn redirect_to_dashboard(session: AdminSession) -> Redirect {
    // A Viewer has no panel (#857) — `/admin` sends it to its home (the
    // portal), not the dashboard it can't open.
    Redirect::to(session.role.home())
}

/// Issue the admin session cookie. 24h lifetime; `Secure` only under
/// TLS (signalled by `X-Forwarded-Proto`, honoured only when the
/// operator trusts forwarded headers — #744) so the plain-HTTP dev
/// server doesn't make the browser drop it. The value is an opaque
/// session id.
fn issue_session_cookie(state: &AppState, cookies: &Cookies, headers: &HeaderMap, session_id: String) {
    let mut c = Cookie::new(COOKIE_NAME, session_id);
    c.set_path("/");
    c.set_http_only(true);
    c.set_same_site(tower_cookies::cookie::SameSite::Strict);
    c.set_secure(crate::auth::request_is_https(
        headers,
        crate::routes::proxy::forward_headers_trusted(&state.config.server),
    ));
    c.set_max_age(Duration::hours(24));
    cookies.add(c);
}

/// 429 + Retry-After when the shared login rate-limit window is full.
fn rate_limited() -> Response {
    let mut resp = (
        StatusCode::TOO_MANY_REQUESTS,
        "too many login attempts — try again shortly",
    )
        .into_response();
    resp.headers_mut()
        .insert(axum::http::header::RETRY_AFTER, "60".parse().unwrap());
    resp
}

/// Whether `/admin/login` should show the break-glass **token** form
/// rather than username/password: forced via `?token=1`, or because
/// there's no DB / no user accounts yet (fresh install).
async fn show_token_form(state: &AppState, force_token: bool) -> bool {
    if force_token {
        return true;
    }
    match state.db.as_ref() {
        Some(pool) => !db::users::any_user_exists(pool).await.unwrap_or(false),
        None => true,
    }
}

#[derive(Debug, Deserialize)]
pub struct LoginQuery {
    /// `?token=1` forces the break-glass token form.
    pub token: Option<String>,
    /// Optional app destination captured by the restricted-app guard.
    pub next: Option<String>,
}

async fn login_form(
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(q): Query<LoginQuery>,
) -> Response {
    let bootstrap = show_token_form(&state, q.token.is_some()).await;
    let next = app_next_path(q.next.as_deref(), &state.base_path).unwrap_or_default();
    let page = login_page(&state, loc, theme, "", bootstrap, next);
    render(&page)
}

fn login_page<'a>(
    state: &'a AppState,
    loc: Locale,
    theme: Theme,
    error: &'static str,
    bootstrap: bool,
    next: String,
) -> LoginPage<'a> {
    let next_encoded = url::form_urlencoded::byte_serialize(next.as_bytes()).collect();
    LoginPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        error,
        bootstrap,
        next,
        next_encoded,
    }
}

fn login_error(
    state: &AppState,
    loc: Locale,
    theme: Theme,
    error: &'static str,
    bootstrap: bool,
    next: String,
) -> Response {
    let page = login_page(state, loc, theme, error, bootstrap, next);
    let body = match page.render() {
        Ok(s) => s,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "render").into_response(),
    };
    let mut resp = (StatusCode::UNAUTHORIZED, Html(body)).into_response();
    resp.headers_mut()
        .insert("X-Ruscker-Login", "failed".parse().unwrap());
    resp
}

#[derive(Debug, Deserialize)]
pub struct LoginForm {
    pub username: String,
    pub password: String,
    pub next: Option<String>,
}

/// Primary login: username + password against the `users` table.
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
    if !state.login_limiter.allow() {
        return rate_limited();
    }
    let next = app_next_path(form.next.as_deref(), &state.base_path);

    // Username/password login needs the user store. Without a DB the
    // only way in is the break-glass token.
    let Some(pool) = state.db.as_ref() else {
        // Count this against the limiter too (#328) — symmetric with the
        // DB-present and token paths, so a no-DB deploy can't be probed
        // without tripping the same backoff.
        state.login_limiter.record_failure();
        return login_error(
            &state,
            loc,
            theme,
            "wrong-login",
            false,
            next.clone().unwrap_or_default(),
        );
    };

    let user = match db::users::verify_login(pool, &form.username, &form.password).await {
        Ok(Some(u)) => u,
        Ok(None) => {
            state.login_limiter.record_failure();
            return login_error(
                &state,
                loc,
                theme,
                "wrong-login",
                false,
                next.clone().unwrap_or_default(),
            );
        }
        Err(e) => {
            tracing::error!(error = ?e, "login lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "login error").into_response();
        }
    };

    state.login_limiter.record_success();
    let session_id = state
        .admin_sessions
        .create(user.role, Some(user.username.clone()))
        .await;
    issue_session_cookie(&state, &cookies, &headers, session_id);

    // First login with an admin-assigned password ⇒ ask whether to
    // change it.
    if user.must_change_password {
        let target = next
            .as_deref()
            .map(|next| super::with_next_query("/admin/account/password", next))
            .unwrap_or_else(|| "/admin/account/password".to_string());
        return Redirect::to(&target).into_response();
    }

    let target = match next {
        Some(target) => target,
        None => {
            let referer = headers.get(REFERER).and_then(|v| v.to_str().ok());
            let raw_path = super::same_origin_path(referer, user.role.home());
            // Under a base path (#173), referer paths come in as
            // `/box/admin/specs` — strip the prefix before the `/admin/`
            // and `/` checks so a non-admin signing in from `/box/admin/X`
            // returns to `X`, not the dashboard. The response middleware
            // re-prefixes the `Location` on the way out.
            let path = strip_base_prefix(&raw_path, &state.base_path);
            if path == "/" {
                // Signed in from the public landing — return there so the
                // viewer sees the apps their groups unlock (#155), rather
                // than bouncing a non-admin into the panel.
                path.to_string()
            } else if path.starts_with("/admin/")
                && !path.starts_with("/admin/login")
                && user.role.can_access_section(section_for_admin_path(path))
            {
                path.to_string()
            } else {
                user.role.home().to_string()
            }
        }
    };
    Redirect::to(&target).into_response()
}

/// Strip a normalized base-path prefix (e.g. `/box`) from a
/// same-origin path so the `/admin/...` and `/` matchers behave the
/// same with or without subpath mounting. `base` is the already-
/// normalized prefix from `AppState.base_path` (`""` for root —
/// leading slash, no trailing slash for everything else, see
/// [`ruscker_config::normalize_base_path`]).
///
/// Query strings ride along: `strip_base_prefix("/box/x?q=1", "/box")`
/// returns `"/x?q=1"`. Callers that only want the path part should
/// split on `?` themselves; today the matchers use `starts_with` /
/// equality, so the query suffix is just preserved as-is.
fn strip_base_prefix<'a>(path: &'a str, base: &str) -> &'a str {
    if base.is_empty() {
        return path;
    }
    // `base` is the normalized form from `AppState.base_path`. If a
    // caller hands us a trailing-slashed or empty-as-`/` form, the
    // strip logic below silently misbehaves.
    debug_assert!(
        !base.ends_with('/') && base != "/" && base.starts_with('/'),
        "base must be normalized: leading slash, no trailing slash, never bare `/` ({base:?})"
    );
    if let Some(rest) = path.strip_prefix(base) {
        // `/box` → "" (the canonical landing) or `/box/foo` → `/foo`.
        if rest.is_empty() {
            "/"
        } else if rest.starts_with('/') {
            rest
        } else {
            // `/boxes` happens to start with `/box` but isn't under it.
            path
        }
    } else {
        path
    }
}

/// Validate an explicit post-login destination and normalize it to the
/// router's base-less form. The restricted-app guard is the only producer
/// today, so deliberately accept `/app/...` destinations only: arbitrary
/// admin paths keep using the existing role-aware Referer fallback.
fn app_next_path(next: Option<&str>, base: &str) -> Option<String> {
    let path = super::local_next_path(next)?;
    let path = strip_base_prefix(path, base);
    let path_only = path.split(['?', '#']).next().unwrap_or(path);
    path_only
        .starts_with("/app/")
        .then(|| path.to_string())
}

#[derive(Debug, Deserialize)]
pub struct TokenForm {
    pub token: String,
    pub next: Option<String>,
}

/// Break-glass login with `RUSCKER_ADMIN_TOKEN`. Always grants an
/// Admin session (actor = `None` ⇒ "token" in the audit log). If no
/// admin account exists yet, it routes to the first-admin setup.
async fn token_login(
    State(state): State<AppState>,
    cookies: Cookies,
    loc: Locale,
    theme: Theme,
    headers: HeaderMap,
    Form(form): Form<TokenForm>,
) -> Response {
    if !state.admin_auth.is_configured() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "RUSCKER_ADMIN_TOKEN is not set — admin disabled",
        )
            .into_response();
    }
    if !state.login_limiter.allow() {
        return rate_limited();
    }
    let next = app_next_path(form.next.as_deref(), &state.base_path);
    if !state.admin_auth.matches_token(&form.token) {
        state.login_limiter.record_failure();
        cookies.remove(Cookie::new(COOKIE_NAME, ""));
        return login_error(
            &state,
            loc,
            theme,
            "wrong-token",
            true,
            next.clone().unwrap_or_default(),
        );
    }

    state.login_limiter.record_success();
    let session_id = state.admin_sessions.create(Role::Admin, None).await;
    issue_session_cookie(&state, &cookies, &headers, session_id);

    // No admin account yet (and a DB to create one in) ⇒ run setup.
    let need_setup = match state.db.as_ref() {
        Some(pool) => !db::users::any_admin_exists(pool).await.unwrap_or(false),
        None => false,
    };
    if need_setup {
        Redirect::to("/admin/setup").into_response()
    } else {
        Redirect::to(next.as_deref().unwrap_or("/admin/dashboard")).into_response()
    }
}

/// Minimum length for an admin-set or self-chosen password.
const MIN_PASSWORD_LEN: usize = 8;

// ── First-admin setup ────────────────────────────────────────────

async fn setup_form(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    // Setup is admin-gated and only meaningful before the first admin
    // account exists. Once one does, send the operator to the dashboard.
    if let Some(pool) = state.db.as_ref() {
        if db::users::any_admin_exists(pool).await.unwrap_or(false) {
            return Redirect::to("/admin/dashboard").into_response();
        }
    } else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no database — start with --db",
        )
            .into_response();
    }
    render(&SetupPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        error: "",
    })
}

#[derive(Debug, Deserialize)]
pub struct SetupForm {
    pub username: String,
    pub password: String,
    pub confirm: String,
}

async fn setup_submit(
    _: RequireAdmin,
    State(state): State<AppState>,
    cookies: Cookies,
    loc: Locale,
    theme: Theme,
    headers: HeaderMap,
    Form(form): Form<SetupForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no database — start with --db",
        )
            .into_response();
    };
    // Guard re-setup: only valid while no admin exists.
    if db::users::any_admin_exists(pool).await.unwrap_or(false) {
        return Redirect::to("/admin/dashboard").into_response();
    }

    let setup_err = |error: &'static str| {
        render(&SetupPage {
            locale: loc,
            theme,
            locales: &state.locales,
            locales_all: &Locale::ALL,
            base: state.base_path.clone(),
            error,
        })
    };
    let username = db::users::normalize_username(&form.username);
    if username.is_empty() {
        return setup_err("exists"); // empty username reuses the generic field error
    }
    if form.password.len() < MIN_PASSWORD_LEN {
        return setup_err("short");
    }
    if form.password != form.confirm {
        return setup_err("mismatch");
    }

    if let Err(e) = db::users::create(
        pool,
        &username,
        &form.password,
        Role::Admin,
        false,
        &[],
        Some("token"),
    )
    .await
    {
        tracing::error!(error = ?e, "create first admin failed");
        return setup_err("exists");
    }

    // Swap the break-glass token session for a real account session.
    if let Some(c) = cookies.get(COOKIE_NAME) {
        state.admin_sessions.remove(c.value()).await;
    }
    let session_id = state
        .admin_sessions
        .create(Role::Admin, Some(username))
        .await;
    issue_session_cookie(&state, &cookies, &headers, session_id);
    Redirect::to("/admin/dashboard").into_response()
}

// ── Self-service password change + first-login prompt ─────────────

#[derive(Debug, Deserialize)]
struct PasswordQuery {
    next: Option<String>,
}

async fn password_form(
    session: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(query): Query<PasswordQuery>,
) -> Response {
    // Break-glass token sessions have no account to change.
    let Some(actor) = session.actor.clone() else {
        return Redirect::to("/admin/dashboard").into_response();
    };
    let first = match state.db.as_ref() {
        Some(pool) => db::users::fetch(pool, &actor)
            .await
            .ok()
            .flatten()
            .map(|u| u.must_change_password)
            .unwrap_or(false),
        None => false,
    };
    let next = app_next_path(query.next.as_deref(), &state.base_path).unwrap_or_default();
    render(&AccountPasswordPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "",
        role: session.role,
        first,
        error: "",
        next,
    })
}

#[derive(Debug, Deserialize)]
pub struct PasswordForm {
    pub current: String,
    pub new_password: String,
    pub confirm: String,
    pub next: Option<String>,
}

async fn password_submit(
    session: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    cookies: Cookies,
    headers: HeaderMap,
    Form(form): Form<PasswordForm>,
) -> Response {
    let Some(actor) = session.actor.clone() else {
        return Redirect::to("/admin/dashboard").into_response();
    };
    let Some(pool) = state.db.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no database — start with --db",
        )
            .into_response();
    };
    let next = app_next_path(form.next.as_deref(), &state.base_path);

    // First login is mandatory (#454): there is no "keep current"
    // escape — the only way past the prompt is a real password change
    // below, which clears `must_change_password` via `set_password`.
    let first = db::users::fetch(pool, &actor)
        .await
        .ok()
        .flatten()
        .map(|u| u.must_change_password)
        .unwrap_or(false);
    let pw_err = |error: &'static str| {
        render(&AccountPasswordPage {
            locale: loc,
            theme,
            locales: &state.locales,
            locales_all: &Locale::ALL,
            base: state.base_path.clone(),
            nav_section: "",
            role: session.role,
            first,
            error,
            next: next.clone().unwrap_or_default(),
        })
    };

    if form.new_password.len() < MIN_PASSWORD_LEN {
        return pw_err("short");
    }
    if form.new_password != form.confirm {
        return pw_err("mismatch");
    }
    // Verify the current password before allowing a change.
    if db::users::verify_login(pool, &actor, &form.current)
        .await
        .ok()
        .flatten()
        .is_none()
    {
        return pw_err("wrong-current");
    }

    if let Err(e) =
        db::users::set_password(pool, &actor, &form.new_password, false, Some(&actor)).await
    {
        tracing::error!(error = ?e, "password change failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "save failed").into_response();
    }

    // A self-service password change must invalidate every OTHER live
    // session for this account (#739) — changing the password is the
    // natural response to a suspected session compromise, and there is
    // no separate "log out everywhere" control. The admin-reset path
    // already does this (#544); this path was the gap. `revoke_by_actor`
    // also kills the session making this request, so mint a fresh one
    // and re-issue the cookie: every other session dies, the user
    // stays signed in.
    state.admin_sessions.revoke_by_actor(&actor).await;
    let session_id = state
        .admin_sessions
        .create(session.role, Some(actor.clone()))
        .await;
    issue_session_cookie(&state, &cookies, &headers, session_id);
    // Return to the app that triggered authentication when one was carried
    // through the mandatory password rotation. Otherwise use the role home
    // (a Viewer goes to the portal, not the dashboard it cannot open).
    Redirect::to(next.as_deref().unwrap_or(session.role.home())).into_response()
}

/// Map an `/admin/...` path to the `nav_section` it belongs to, for
/// role-gating the post-login redirect. Order matters: the per-replica
/// logs live under `/admin/dashboard/logs`, so that prefix must be
/// checked before the standalone daemon-log `/admin/logs`.
fn section_for_admin_path(path: &str) -> &'static str {
    if path.starts_with("/admin/dashboard") {
        "dashboard"
    } else if path.starts_with("/admin/disk") {
        "disk"
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
        let sid = c.value().to_string();
        // stop-on-logout (#337): resolve the user *before* invalidating,
        // then end their indexed app sessions (on specs that opted in)
        // right away — the replicas go idle and the scaler reaps them,
        // instead of waiting out the heartbeat timeout.
        if let Some(info) = state.admin_sessions.validate(&sid).await {
            if let Some(user) = info.actor {
                if let Some((_, session_ids)) = state.logout_index.remove(&user) {
                    let n = session_ids.len();
                    for app_sid in session_ids {
                        state.sessions.end_session(&state.replicas, app_sid).await;
                    }
                    if n > 0 {
                        tracing::info!(user = %user, ended = n, "stop-on-logout: ended user's app sessions");
                    }
                }
            }
        }
        state.admin_sessions.remove(&sid).await;
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

#[cfg(test)]
mod tests {
    use super::{app_next_path, strip_base_prefix};
    use super::{LoginPage, SetupPage};
    use crate::i18n::{Locale, Locales};
    use crate::theme::Theme;
    use askama::Template;

    /// Minimal AppState over an in-memory SQLite DB with admin auth
    /// configured — just enough for account-route tests.
    async fn state_with_db() -> crate::AppState {
        use std::sync::Arc;
        let pool = crate::db::ConfigDb::Sqlite(crate::db::open_memory().await.expect("memory db"));
        crate::AppState {
            config: Arc::new(ruscker_config::Config::from_yaml("proxy:\n  specs: []").expect("config")),
            base_path: Arc::from(""),
            locales: Arc::new(Locales::load().expect("load locales")),
            admin_auth: crate::auth::AdminAuth::with_token("break-glass-token"),
            admin_sessions: Arc::new(crate::auth::InMemoryAdminSessionStore::default()),
            log_buffer: None,
            login_limiter: Arc::new(crate::auth::LoginRateLimiter::default_policy()),
            api_limiter: Arc::new(crate::ratelimit::ApiRateLimiter::new()),
            db: Some(pool),
            images_dir: None,
            master_key: Default::default(),
            backend: None,
            replicas: Arc::new(tokio::sync::RwLock::new(ruscker_core::ReplicaRegistry::new())),
            cookie_key: ruscker_proxy::sticky::CookieKey::random(),
            spawn_locks: Arc::new(dashmap::DashMap::new()),
            sessions: Arc::new(crate::sessions::InMemorySessionStore::new()),
            logout_index: Arc::new(dashmap::DashMap::new()),
            leader: Arc::new(crate::leader::AlwaysLeader),
            metrics: crate::metrics_cache::MetricsCache::new(),
            draining: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            spec_cache: Arc::new(dashmap::DashMap::new()),
            catalog_cache: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    // #739: a self-service password change must revoke every OTHER live
    // session for the account (an attacker's stolen session must die),
    // while the user making the change stays signed in on a freshly
    // minted session.
    #[tokio::test]
    async fn self_password_change_revokes_other_sessions_but_keeps_the_user_in() {
        use axum::body::Body;
        use axum::http::{header, Request, StatusCode};
        use tower::ServiceExt;

        let state = state_with_db().await;
        let pool = state.db.as_ref().unwrap();
        crate::db::users::create(
            pool,
            "alice",
            "old-password-1",
            crate::auth::Role::Editor,
            false,
            &[],
            None,
        )
        .await
        .expect("create user");

        // Two live sessions: the requester's and a second device's
        // (the one a compromise would ride).
        let mine = state
            .admin_sessions
            .create(crate::auth::Role::Editor, Some("alice".into()))
            .await;
        let other = state
            .admin_sessions
            .create(crate::auth::Role::Editor, Some("alice".into()))
            .await;

        let app = axum::Router::new()
            .route(
                "/admin/account/password",
                axum::routing::post(super::password_submit),
            )
            .layer(tower_cookies::CookieManagerLayer::new())
            .with_state(state.clone());
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/account/password")
                    .header(header::COOKIE, format!("{}={}", crate::auth::COOKIE_NAME, mine))
                    .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                    .body(Body::from(
                        "current=old-password-1&new_password=new-password-2&confirm=new-password-2",
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SEE_OTHER, "change accepted");

        // The other device's session is dead; so is the requester's old id.
        assert!(
            state.admin_sessions.validate(&other).await.is_none(),
            "the second session must be revoked by the password change"
        );
        assert!(
            state.admin_sessions.validate(&mine).await.is_none(),
            "the old session id must not survive either"
        );

        // The response re-issues a fresh, VALID session cookie — the
        // user who changed the password stays signed in.
        let set = resp
            .headers()
            .get(header::SET_COOKIE)
            .and_then(|v| v.to_str().ok())
            .expect("a session cookie is re-issued");
        let new_sid = set
            .split(';')
            .next()
            .and_then(|kv| kv.strip_prefix(&format!("{}=", crate::auth::COOKIE_NAME)))
            .expect("cookie carries the new session id");
        let info = state
            .admin_sessions
            .validate(new_sid)
            .await
            .expect("the fresh session is live");
        assert_eq!(info.actor.as_deref(), Some("alice"));

        // And the new password is the one that verifies.
        assert!(crate::db::users::verify_login(pool, "alice", "new-password-2")
            .await
            .expect("verify")
            .is_some());
    }

    #[test]
    fn strip_base_prefix_root_is_noop() {
        assert_eq!(strip_base_prefix("/admin/specs", ""), "/admin/specs");
        assert_eq!(strip_base_prefix("/", ""), "/");
    }

    #[test]
    fn strip_base_prefix_removes_matching_prefix() {
        assert_eq!(strip_base_prefix("/box/admin/specs", "/box"), "/admin/specs");
        assert_eq!(strip_base_prefix("/box/", "/box"), "/");
        // Bare canonical landing under the prefix.
        assert_eq!(strip_base_prefix("/box", "/box"), "/");
    }

    #[test]
    fn strip_base_prefix_keeps_non_matching_paths() {
        // `/boxes` starts with `/box` but is not under the prefix.
        assert_eq!(strip_base_prefix("/boxes", "/box"), "/boxes");
        assert_eq!(strip_base_prefix("/other/path", "/box"), "/other/path");
    }

    #[test]
    fn strip_base_prefix_preserves_query_strings() {
        // Query strings ride along the suffix — same matcher
        // behavior as a no-base-path deployment.
        assert_eq!(
            strip_base_prefix("/box/admin/specs?page=2", "/box"),
            "/admin/specs?page=2"
        );
        assert_eq!(strip_base_prefix("/box/?theme=dark", "/box"), "/?theme=dark");
    }

    #[test]
    fn app_next_accepts_only_app_paths_and_strips_the_base_path() {
        assert_eq!(
            app_next_path(Some("/box/app/analysts/report?tab=2"), "/box"),
            Some("/app/analysts/report?tab=2".to_string())
        );
        assert_eq!(
            app_next_path(Some("/app/demo/"), ""),
            Some("/app/demo/".to_string())
        );
        assert_eq!(
            app_next_path(Some("https://evil.example/app/demo"), ""),
            None
        );
        assert_eq!(app_next_path(Some("//evil.example/app/demo"), ""), None);
        assert_eq!(app_next_path(Some("/admin/dashboard"), ""), None);
    }

    #[test]
    fn login_page_carries_app_next_across_login_modes() {
        let locales = Locales::load().expect("load locales");
        let html = LoginPage {
            locale: Locale::En,
            theme: Theme::Auto,
            locales: &locales,
            locales_all: &Locale::ALL,
            base: std::sync::Arc::from(""),
            error: "",
            bootstrap: false,
            next: "/app/demo/?tab=1".to_string(),
            next_encoded: "%2Fapp%2Fdemo%2F%3Ftab%3D1".to_string(),
        }
        .render()
        .expect("render login");
        assert!(html.contains(r#"name="next" value="/app/demo/?tab=1""#));
        assert!(html.contains(
            r#"href="/admin/login?token=1&amp;next=%2Fapp%2Fdemo%2F%3Ftab%3D1""#
        ));
    }

    // The favicon set lives in the shared `_favicons.html` partial; the
    // standalone login/setup heads must include it so Safari has the raster
    // .ico/.png to fall back on (Safari can ignore the SVG favicon and
    // otherwise keep a stale origin-root icon when moving admin → landing).
    #[test]
    fn login_page_includes_raster_favicons() {
        let locales = Locales::load().expect("load locales");
        let html = LoginPage {
            locale: Locale::En,
            theme: Theme::Auto,
            locales: &locales,
            locales_all: &Locale::ALL,
            base: std::sync::Arc::from(""),
            error: "",
            bootstrap: false,
            next: String::new(),
            next_encoded: String::new(),
        }
        .render()
        .expect("render login");
        assert!(html.contains("/favicon.ico"), "login must link /favicon.ico");
        assert!(
            html.contains("/favicon-32.png"),
            "login must link /favicon-32.png"
        );
    }

    #[test]
    fn setup_page_includes_raster_favicons() {
        let locales = Locales::load().expect("load locales");
        let html = SetupPage {
            locale: Locale::En,
            theme: Theme::Auto,
            locales: &locales,
            locales_all: &Locale::ALL,
            base: std::sync::Arc::from(""),
            error: "",
        }
        .render()
        .expect("render setup");
        assert!(html.contains("/favicon.ico"), "setup must link /favicon.ico");
        assert!(
            html.contains("/favicon-32.png"),
            "setup must link /favicon-32.png"
        );
    }

    // The perceived-speed partial (#623) — the top navigation progress bar
    // and its MPA driver script — is included by both admin layouts and the
    // standalone login/setup pages, so every navigable admin page gets it.
    // Login is standalone (own <body>); render it and assert the bar is
    // wired (the `.topbar-progress` primitive itself lives in input.css).
    #[test]
    fn admin_layout_mounts_nav_progress_bar() {
        let locales = Locales::load().expect("load locales");
        let html = LoginPage {
            locale: Locale::En,
            theme: Theme::Auto,
            locales: &locales,
            locales_all: &Locale::ALL,
            base: std::sync::Arc::from(""),
            error: "",
            bootstrap: false,
            next: String::new(),
            next_encoded: String::new(),
        }
        .render()
        .expect("render login");
        assert!(
            html.contains(r#"id="ruscker-progress""#),
            "admin layout must mount the nav progress bar"
        );
        assert!(
            html.contains("topbar-progress"),
            "nav progress bar must carry the .topbar-progress class"
        );
    }
}
