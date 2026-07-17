//! Self-service TOTP enrollment for every password-backed account.

use askama::Template;
use axum::{
    extract::{Form, Query, State},
    http::{header::CACHE_CONTROL, header::RETRY_AFTER, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;

use crate::auth::{AdminSession, Role};
use crate::db;
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/account/mfa", get(status))
        .route("/admin/account/mfa/start", post(start))
        .route("/admin/account/mfa/confirm", post(confirm))
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

async fn start(
    session: AdminSession,
    State(state): State<AppState>,
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
    match db::users::verify_login(db, &username, &form.current_password).await {
        Ok(Some(_)) => {}
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
    if let Err(err) = db::mfa::begin_enrollment(db, &username, &secret_enc, &nonce).await {
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

    if !crate::mfa::CONFIRM_LIMITER.allow(username) {
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
    let valid = match crate::mfa::verify_totp(secret, username, form.code.trim()) {
        Ok(valid) => valid,
        Err(err) => {
            tracing::error!(error = ?err, %username, "verify enrollment TOTP failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "MFA verification error").into_response();
        }
    };
    if !valid {
        crate::mfa::CONFIRM_LIMITER.record_failure(username);
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
    if let Err(err) =
        db::mfa::confirm_with_recovery_codes(db, username, username, Some(&hashes)).await
    {
        tracing::warn!(error = ?err, %username, "confirm MFA enrollment failed");
        return (StatusCode::CONFLICT, "MFA enrollment was already confirmed").into_response();
    }
    crate::mfa::CONFIRM_LIMITER.record_success(username);
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
