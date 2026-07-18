//! Self-service TOTP enrollment for every password-backed account.

use askama::Template;
use axum::{
    extract::{Form, Query, State},
    http::{header::CACHE_CONTROL, header::RETRY_AFTER, HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;

use crate::auth::{AdminSession, Role, COOKIE_NAME};
use crate::db;
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;
use tower_cookies::{Cookie, Cookies};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/account/mfa", get(status))
        .route("/admin/account/mfa/start", post(start))
        .route("/admin/account/mfa/confirm", post(confirm))
        .route(
            "/admin/account/mfa/challenge",
            get(challenge).post(challenge_submit),
        )
        .route("/admin/account/mfa/device/forget", post(forget_device))
        .route("/admin/account/mfa/devices/revoke", post(revoke_devices))
}

#[derive(Template)]
#[template(path = "admin/account_mfa.html")]
struct AccountMfaPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    enrolled_at: String,
    pending: bool,
    break_glass: bool,
    error: &'static str,
    next: String,
}

impl AccountMfaPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

#[derive(Template)]
#[template(path = "admin/account_mfa_setup.html")]
struct AccountMfaSetupPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    qr_svg: String,
    secret: String,
    error: &'static str,
    next: String,
}

impl AccountMfaSetupPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

#[derive(Template)]
#[template(path = "admin/account_mfa_recovery.html")]
struct AccountMfaRecoveryPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    codes: Vec<String>,
    next: String,
}

impl AccountMfaRecoveryPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

#[derive(Template)]
#[template(path = "admin/account_mfa_challenge.html")]
struct AccountMfaChallengePage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    break_glass: bool,
    error: &'static str,
    next: String,
}

impl AccountMfaChallengePage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

#[derive(Debug, Deserialize, Default)]
struct NextQuery {
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
struct StartForm {
    current_password: String,
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ConfirmForm {
    code: String,
    next: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChallengeForm {
    code: String,
    kind: String,
    next: Option<String>,
}

fn safe_next(raw: Option<&str>, base: &str) -> String {
    let Some(path) = crate::routes::local_next_path(raw) else {
        return String::new();
    };
    super::strip_base_prefix(path, base).to_string()
}

fn key_missing(state: &AppState, locale: Locale) -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        state.locales.t(locale, "admin-mfa-error-key", None),
    )
        .into_response()
}

async fn status(
    session: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(query): Query<NextQuery>,
) -> Response {
    let next = safe_next(query.next.as_deref(), &state.base_path);
    render_status(&state, session, loc, theme, "", next, StatusCode::OK).await
}

async fn render_status(
    state: &AppState,
    session: AdminSession,
    loc: Locale,
    theme: Theme,
    error: &'static str,
    next: String,
    status: StatusCode,
) -> Response {
    let Some(db) = state.db.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no database — start with --db",
        )
            .into_response();
    };
    let row = match session.actor.as_deref() {
        Some(username) => match db::mfa::fetch(db, username).await {
            Ok(row) => row,
            Err(err) => {
                tracing::error!(error = ?err, "fetch own MFA status failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        },
        None => None,
    };
    let enrolled_at = row
        .as_ref()
        .and_then(|row| row.confirmed_at)
        .map(|at| at.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default();
    let pending = row.is_some_and(|row| row.confirmed_at.is_none());
    let page = AccountMfaPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "account",
        role: session.role,
        enrolled_at,
        pending,
        break_glass: session.actor.is_none(),
        error,
        next,
    };
    let body = match page.render() {
        Ok(body) => body,
        Err(err) => {
            tracing::error!(error = ?err, "render MFA status failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "render error").into_response();
        }
    };
    (status, axum::response::Html(body)).into_response()
}

/// Cookie binding the pending enrollment ceremony to the browser that
/// passed the password re-auth (codex review, #1005). Without it, ANY
/// session for the same username — e.g. a stolen cookie that never
/// re-authenticated — could post a wrong code and be handed the decrypted
/// secret + QR via the retry re-render. Short-lived; scoped to the MFA
/// pages; cleared on success.
const CEREMONY_COOKIE: &str = "__ruscker_mfa_ceremony";

fn ceremony_cookie_path(base: &str) -> String {
    format!("{base}/admin/account/mfa")
}

fn issue_ceremony_cookie(state: &AppState, cookies: &Cookies, headers: &HeaderMap, value: String) {
    let mut c = Cookie::new(CEREMONY_COOKIE, value);
    c.set_path(ceremony_cookie_path(&state.base_path));
    c.set_http_only(true);
    c.set_same_site(tower_cookies::cookie::SameSite::Strict);
    c.set_secure(crate::auth::request_is_https(
        headers,
        crate::routes::proxy::forward_headers_trusted(&state.config.server),
    ));
    c.set_max_age(tower_cookies::cookie::time::Duration::minutes(15));
    cookies.add(c);
}

fn clear_ceremony_cookie(state: &AppState, cookies: &Cookies) {
    let mut c = Cookie::new(CEREMONY_COOKIE, "");
    // The removal path MUST match the issuing path — a Path-less removal
    // never clears a scoped cookie (the #923 lesson).
    c.set_path(ceremony_cookie_path(&state.base_path));
    cookies.remove(c);
}

fn device_cookie_path(base: &str) -> String {
    if base.is_empty() {
        "/".to_string()
    } else {
        format!("{base}/")
    }
}

fn issue_device_cookie(
    state: &AppState,
    cookies: &Cookies,
    headers: &HeaderMap,
    id: &str,
    token: &str,
) {
    let mut c = Cookie::new(crate::mfa::DEVICE_COOKIE, format!("{id}.{token}"));
    c.set_path(device_cookie_path(&state.base_path));
    c.set_http_only(true);
    c.set_same_site(tower_cookies::cookie::SameSite::Strict);
    c.set_secure(crate::auth::request_is_https(
        headers,
        crate::routes::proxy::forward_headers_trusted(&state.config.server),
    ));
    c.set_max_age(tower_cookies::cookie::time::Duration::days(i64::from(
        ruscker_config::MAX_MFA_VALIDITY_DAYS,
    )));
    cookies.add(c);
}

fn clear_device_cookie(state: &AppState, cookies: &Cookies) {
    let mut c = Cookie::new(crate::mfa::DEVICE_COOKIE, "");
    // The removal path must exactly match the issuing path (#923).
    c.set_path(device_cookie_path(&state.base_path));
    cookies.remove(c);
}

async fn start(
    session: AdminSession,
    State(state): State<AppState>,
    cookies: Cookies,
    headers: HeaderMap,
    loc: Locale,
    theme: Theme,
    Form(form): Form<StartForm>,
) -> Response {
    let next = safe_next(form.next.as_deref(), &state.base_path);
    let Some(username) = session.actor.clone() else {
        return render_status(
            &state,
            session,
            loc,
            theme,
            "break-glass",
            next,
            StatusCode::FORBIDDEN,
        )
        .await;
    };
    if !state.master_key.is_configured() {
        return key_missing(&state, loc);
    }
    let Some(db) = state.db.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no database — start with --db",
        )
            .into_response();
    };
    if !crate::mfa::REAUTH_LIMITER.try_reserve(&username) {
        let mut response = render_status(
            &state,
            session,
            loc,
            theme,
            "rate-limited",
            next,
            StatusCode::TOO_MANY_REQUESTS,
        )
        .await;
        response
            .headers_mut()
            .insert(RETRY_AFTER, "60".parse().unwrap());
        return response;
    }
    match db::users::verify_login(db, &username, &form.current_password).await {
        Ok(Some(_)) => {
            crate::mfa::REAUTH_LIMITER.record_success(&username);
        }
        Ok(None) => {
            return render_status(
                &state,
                session,
                loc,
                theme,
                "wrong-password",
                next,
                StatusCode::UNAUTHORIZED,
            )
            .await;
        }
        Err(err) => {
            tracing::error!(error = ?err, %username, "MFA re-authentication failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "login error").into_response();
        }
    }

    match db::mfa::fetch(db, &username).await {
        Ok(Some(row)) if row.confirmed_at.is_some() => {
            return render_status(
                &state,
                session,
                loc,
                theme,
                "already-enrolled",
                next,
                StatusCode::CONFLICT,
            )
            .await;
        }
        Ok(_) => {}
        Err(err) => {
            tracing::error!(error = ?err, %username, "fetch MFA state before enrollment failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    }

    let enrollment = match crate::mfa::begin(&username) {
        Ok(enrollment) => enrollment,
        Err(err) => {
            tracing::error!(error = ?err, %username, "generate MFA enrollment failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "MFA setup error").into_response();
        }
    };
    let (secret_enc, nonce) = match state
        .master_key
        .encrypt(enrollment.secret_base32.as_bytes())
    {
        Ok(parts) => parts,
        Err(err) => {
            tracing::error!(error = ?err, %username, "encrypt MFA secret failed");
            return key_missing(&state, loc);
        }
    };
    let ceremony = uuid::Uuid::new_v4().to_string();
    if let Err(err) =
        db::mfa::begin_enrollment(db, &username, &secret_enc, &nonce, &ceremony).await
    {
        tracing::warn!(error = ?err, %username, "persist pending MFA enrollment failed");
        return render_status(
            &state,
            session,
            loc,
            theme,
            "already-enrolled",
            next,
            StatusCode::CONFLICT,
        )
        .await;
    }
    // A password-reauthenticated restart is a new setup ceremony, so stale
    // mistakes against the previous pending secret must not carry over.
    crate::mfa::CONFIRM_LIMITER.record_success(&username);
    issue_ceremony_cookie(&state, &cookies, &headers, ceremony);
    render_setup(
        &state,
        session.role,
        loc,
        theme,
        enrollment,
        "",
        next,
        StatusCode::OK,
    )
}

#[allow(clippy::too_many_arguments)]
fn render_setup(
    state: &AppState,
    role: Role,
    loc: Locale,
    theme: Theme,
    enrollment: crate::mfa::Enrollment,
    error: &'static str,
    next: String,
    status: StatusCode,
) -> Response {
    let page = AccountMfaSetupPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "account",
        role,
        qr_svg: enrollment.qr_svg,
        secret: enrollment.secret_base32,
        error,
        next,
    };
    match page.render() {
        Ok(body) => {
            let mut response = (status, axum::response::Html(body)).into_response();
            response
                .headers_mut()
                .insert(CACHE_CONTROL, "no-store".parse().unwrap());
            response
        }
        Err(err) => {
            tracing::error!(error = ?err, "render MFA setup failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "render error").into_response()
        }
    }
}

async fn confirm(
    session: AdminSession,
    State(state): State<AppState>,
    cookies: Cookies,
    loc: Locale,
    theme: Theme,
    Form(form): Form<ConfirmForm>,
) -> Response {
    let next = safe_next(form.next.as_deref(), &state.base_path);
    let Some(username) = session.actor.as_deref() else {
        return (
            StatusCode::FORBIDDEN,
            "break-glass sessions cannot enroll MFA",
        )
            .into_response();
    };
    if !state.master_key.is_configured() {
        return key_missing(&state, loc);
    }
    let Some(db) = state.db.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no database — start with --db",
        )
            .into_response();
    };
    let row = match db::mfa::fetch(db, username).await {
        Ok(Some(row)) if row.confirmed_at.is_none() => row,
        Ok(_) => return (StatusCode::CONFLICT, "no pending MFA enrollment").into_response(),
        Err(err) => {
            tracing::error!(error = ?err, %username, "fetch pending MFA enrollment failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    // Ceremony gate BEFORE any decrypt/re-render (codex review, #1005):
    // only the browser that passed the password re-auth holds this cookie,
    // so another session for the same user can never fish the secret out
    // of the wrong-code retry path, and a raced re-start (new ceremony)
    // invalidates in-flight confirms instead of confirming the wrong
    // secret. Generic 409 — no secret material in the response.
    let ceremony_ok = cookies
        .get(CEREMONY_COOKIE)
        .is_some_and(|c| {
            // Constant-time compare: the token gates secret disclosure.
            let got = c.value().as_bytes();
            let want = row.ceremony.as_bytes();
            got.len() == want.len()
                && got
                    .iter()
                    .zip(want)
                    .fold(0u8, |acc, (a, b)| acc | (a ^ b))
                    == 0
        });
    if !ceremony_ok {
        return (
            StatusCode::CONFLICT,
            "this browser did not start the pending enrollment — restart it",
        )
            .into_response();
    }
    let plaintext = match state.master_key.decrypt(&row.secret_enc, &row.secret_nonce) {
        Ok(plaintext) => plaintext,
        Err(err) => {
            tracing::error!(error = ?err, %username, "decrypt pending MFA secret failed");
            return (
                StatusCode::SERVICE_UNAVAILABLE,
                "MFA secret cannot be decrypted with RUSCKER_MASTER_KEY",
            )
                .into_response();
        }
    };
    let secret = match std::str::from_utf8(&plaintext) {
        Ok(secret) => secret,
        Err(err) => {
            tracing::error!(error = ?err, %username, "pending MFA secret is not UTF-8");
            return (StatusCode::INTERNAL_SERVER_ERROR, "invalid MFA state").into_response();
        }
    };
    let enrollment = match crate::mfa::render_enrollment(secret, username) {
        Ok(enrollment) => enrollment,
        Err(err) => {
            tracing::error!(error = ?err, %username, "render pending MFA enrollment failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "invalid MFA state").into_response();
        }
    };

    if !crate::mfa::CONFIRM_LIMITER.try_reserve(username) {
        let mut response = render_setup(
            &state,
            session.role,
            loc,
            theme,
            enrollment,
            "rate-limited",
            next,
            StatusCode::TOO_MANY_REQUESTS,
        );
        response
            .headers_mut()
            .insert(RETRY_AFTER, "60".parse().unwrap());
        return response;
    }
    let accepted_step = match crate::mfa::verify_totp_step(secret, username, form.code.trim()) {
        Ok(step) => step,
        Err(err) => {
            tracing::error!(error = ?err, %username, "verify enrollment TOTP failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "MFA verification error").into_response();
        }
    };
    let Some(accepted_step) = accepted_step else {
        return render_setup(
            &state,
            session.role,
            loc,
            theme,
            enrollment,
            "wrong-code",
            next,
            StatusCode::UNAUTHORIZED,
        );
    };
    match db::mfa::record_used_step(db, username, accepted_step).await {
        Ok(true) => {}
        Ok(false) => {
            return render_setup(
                &state,
                session.role,
                loc,
                theme,
                enrollment,
                "wrong-code",
                next,
                StatusCode::UNAUTHORIZED,
            );
        }
        Err(err) => {
            tracing::error!(error = ?err, %username, "record enrollment TOTP step failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "MFA verification error").into_response();
        }
    }

    let codes = match crate::mfa::generate_recovery_codes() {
        Ok(codes) => codes,
        Err(err) => {
            tracing::error!(error = ?err, %username, "generate recovery codes failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "recovery-code error").into_response();
        }
    };
    let mut hashes = Vec::with_capacity(codes.len());
    for code in &codes {
        match crate::mfa::hash_recovery_code(code) {
            Ok(hash) => hashes.push(hash),
            Err(err) => {
                tracing::error!(error = ?err, %username, "hash recovery code failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "recovery-code error").into_response();
            }
        }
    }
    if let Err(err) = db::mfa::confirm_with_recovery_codes(
        db,
        username,
        username,
        Some(&hashes),
        &row.ceremony,
    )
    .await
    {
        tracing::warn!(error = ?err, %username, "confirm MFA enrollment failed");
        return (StatusCode::CONFLICT, "MFA enrollment was already confirmed").into_response();
    }
    crate::mfa::CONFIRM_LIMITER.record_success(username);
    clear_ceremony_cookie(&state, &cookies);
    let page = AccountMfaRecoveryPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "account",
        role: session.role,
        codes,
        next,
    };
    let mut response = super::render(&page);
    response
        .headers_mut()
        .insert(CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

#[allow(clippy::too_many_arguments)]
fn render_challenge(
    state: &AppState,
    role: Role,
    loc: Locale,
    theme: Theme,
    break_glass: bool,
    error: &'static str,
    next: String,
    status: StatusCode,
) -> Response {
    let page = AccountMfaChallengePage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "account",
        role,
        break_glass,
        error,
        next,
    };
    match page.render() {
        Ok(body) => {
            let mut response = (status, axum::response::Html(body)).into_response();
            response
                .headers_mut()
                .insert(CACHE_CONTROL, "no-store".parse().unwrap());
            response
        }
        Err(err) => {
            tracing::error!(error = ?err, "render MFA challenge failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "render error").into_response()
        }
    }
}

async fn challenge(
    session: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(query): Query<NextQuery>,
) -> Response {
    let next = safe_next(query.next.as_deref(), &state.base_path);
    let Some(username) = session.actor.as_deref() else {
        return render_challenge(
            &state,
            session.role,
            loc,
            theme,
            true,
            "",
            next,
            StatusCode::OK,
        );
    };
    let Some(db) = state.db.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no database — start with --db",
        )
            .into_response();
    };
    match db::mfa::is_enrolled(db, username).await {
        Ok(true) => render_challenge(
            &state,
            session.role,
            loc,
            theme,
            false,
            "",
            next,
            StatusCode::OK,
        ),
        Ok(false) => {
            let path = format!("{}/admin/account/mfa", state.base_path);
            Redirect::to(&crate::routes::with_next_query(&path, &next)).into_response()
        }
        Err(err) => {
            tracing::error!(error = ?err, %username, "fetch MFA status for challenge failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

async fn challenge_submit(
    session: AdminSession,
    State(state): State<AppState>,
    cookies: Cookies,
    headers: HeaderMap,
    loc: Locale,
    theme: Theme,
    Form(form): Form<ChallengeForm>,
) -> Response {
    let next = safe_next(form.next.as_deref(), &state.base_path);
    let Some(username) = session.actor.as_deref() else {
        return render_challenge(
            &state,
            session.role,
            loc,
            theme,
            true,
            "",
            next,
            StatusCode::FORBIDDEN,
        );
    };
    if !state.master_key.is_configured() {
        return key_missing(&state, loc);
    }
    let Some(db) = state.db.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "no database — start with --db",
        )
            .into_response();
    };
    let row = match db::mfa::fetch(db, username).await {
        Ok(Some(row)) if row.confirmed_at.is_some() => row,
        Ok(_) => {
            let path = format!("{}/admin/account/mfa", state.base_path);
            return Redirect::to(&crate::routes::with_next_query(&path, &next)).into_response();
        }
        Err(err) => {
            tracing::error!(error = ?err, %username, "fetch MFA factor for challenge failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    if !crate::mfa::CHALLENGE_LIMITER.try_reserve(username) {
        let mut response = render_challenge(
            &state,
            session.role,
            loc,
            theme,
            false,
            "rate-limited",
            next,
            StatusCode::TOO_MANY_REQUESTS,
        );
        response
            .headers_mut()
            .insert(RETRY_AFTER, "60".parse().unwrap());
        return response;
    }

    let audit_action = match form.kind.as_str() {
        "totp" => {
            let plaintext = match state.master_key.decrypt(&row.secret_enc, &row.secret_nonce) {
                Ok(plaintext) => plaintext,
                Err(err) => {
                    tracing::error!(error = ?err, %username, "decrypt MFA challenge secret failed");
                    return key_missing(&state, loc);
                }
            };
            let secret = match std::str::from_utf8(&plaintext) {
                Ok(secret) => secret,
                Err(err) => {
                    tracing::error!(error = ?err, %username, "MFA challenge secret is not UTF-8");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "invalid MFA state")
                        .into_response();
                }
            };
            let step = match crate::mfa::verify_totp_step(secret, username, form.code.trim()) {
                Ok(step) => step,
                Err(err) => {
                    tracing::error!(error = ?err, %username, "verify challenge TOTP failed");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "MFA verification error")
                        .into_response();
                }
            };
            let Some(step) = step else {
                return render_challenge(
                    &state,
                    session.role,
                    loc,
                    theme,
                    false,
                    "wrong-code",
                    next,
                    StatusCode::UNAUTHORIZED,
                );
            };
            match db::mfa::record_used_step(db, username, step).await {
                Ok(true) => "mfa.verify",
                Ok(false) => {
                    return render_challenge(
                        &state,
                        session.role,
                        loc,
                        theme,
                        false,
                        "replayed",
                        next,
                        StatusCode::UNAUTHORIZED,
                    );
                }
                Err(err) => {
                    tracing::error!(error = ?err, %username, "record challenge TOTP step failed");
                    return (StatusCode::INTERNAL_SERVER_ERROR, "MFA verification error")
                        .into_response();
                }
            }
        }
        "recovery" => match db::mfa::consume_recovery_code(db, username, form.code.trim()).await {
            Ok(true) => "mfa.recovery_used",
            Ok(false) => {
                return render_challenge(
                    &state,
                    session.role,
                    loc,
                    theme,
                    false,
                    "wrong-code",
                    next,
                    StatusCode::UNAUTHORIZED,
                );
            }
            Err(err) => {
                tracing::error!(error = ?err, %username, "consume MFA recovery code failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "MFA verification error")
                    .into_response();
            }
        },
        _ => {
            return render_challenge(
                &state,
                session.role,
                loc,
                theme,
                false,
                "wrong-code",
                next,
                StatusCode::UNAUTHORIZED,
            );
        }
    };

    let Some(session_id) = cookies.get(COOKIE_NAME).map(|c| c.value().to_string()) else {
        return (StatusCode::UNAUTHORIZED, "admin session cookie missing").into_response();
    };
    let token = match crate::mfa::generate_device_token() {
        Ok(token) => token,
        Err(err) => {
            tracing::error!(error = ?err, %username, "generate MFA device token failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "MFA grant error").into_response();
        }
    };
    let token_hash = match crate::mfa::hash_device_token(&token) {
        Ok(hash) => hash,
        Err(err) => {
            tracing::error!(error = ?err, %username, "hash MFA device token failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "MFA grant error").into_response();
        }
    };
    let verified_at = chrono::Utc::now();
    let expires_at = verified_at
        + chrono::Duration::days(i64::from(ruscker_config::MAX_MFA_VALIDITY_DAYS));
    let Some(factor_confirmed_at) = row.confirmed_at else {
        return (StatusCode::CONFLICT, "MFA factor is not confirmed").into_response();
    };
    let id = match db::mfa_grants::create(
        db,
        username,
        &token_hash,
        &crate::mfa::session_binding(&session_id),
        factor_confirmed_at,
        verified_at,
        expires_at,
    )
    .await
    {
        Ok(id) => id,
        Err(err) => {
            tracing::error!(error = ?err, %username, "create MFA device grant failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "MFA grant error").into_response();
        }
    };
    if let Err(err) = db::audit::record(
        db,
        username,
        audit_action,
        &format!("user:{username}"),
        None,
    )
    .await
    {
        tracing::error!(error = ?err, %username, "audit MFA proof failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "audit error").into_response();
    }
    crate::mfa::CHALLENGE_LIMITER.record_success(username);
    issue_device_cookie(&state, &cookies, &headers, &id, &token);
    let destination = if next.is_empty() {
        format!("{}/admin/account/mfa", state.base_path)
    } else {
        format!("{}{}", state.base_path, next)
    };
    Redirect::to(&destination).into_response()
}

async fn forget_device(
    session: AdminSession,
    State(state): State<AppState>,
    cookies: Cookies,
) -> Response {
    let Some(username) = session.actor.as_deref() else {
        return (StatusCode::FORBIDDEN, "break-glass session has no MFA device").into_response();
    };
    let Some(db) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    if let Some(cookie) = cookies.get(crate::mfa::DEVICE_COOKIE) {
        if let Some((id, _)) = crate::mfa::device_cookie_parts(cookie.value()) {
            if let Err(err) = db::mfa_grants::revoke_one(db, id, username, username).await {
                tracing::error!(error = ?err, %username, "forget MFA device failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        }
    }
    clear_device_cookie(&state, &cookies);
    Redirect::to(&format!("{}/admin/account/mfa", state.base_path)).into_response()
}

async fn revoke_devices(
    session: AdminSession,
    State(state): State<AppState>,
    cookies: Cookies,
) -> Response {
    let Some(username) = session.actor.as_deref() else {
        return (StatusCode::FORBIDDEN, "break-glass session has no MFA devices").into_response();
    };
    let Some(db) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no database").into_response();
    };
    if let Err(err) =
        db::mfa_grants::revoke_all(db, username, username, "all-devices").await
    {
        tracing::error!(error = ?err, %username, "revoke all MFA devices failed");
        return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
    }
    clear_device_cookie(&state, &cookies);
    Redirect::to(&format!("{}/admin/account/mfa", state.base_path)).into_response()
}
