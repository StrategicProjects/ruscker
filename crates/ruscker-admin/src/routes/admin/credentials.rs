//! Admin > Registry credentials.
//!
//! List + create-or-update + delete on a single page. The list
//! shows names, registries and usernames; passwords never leave
//! the server. The form does both create (new name) and update
//! (existing name) via the same `upsert` repository call.

use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;

use crate::auth::{RequireAdmin, Role};
use crate::db;
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/credentials", get(index).post(create_or_update))
        .route("/admin/credentials/{name}/delete", post(delete))
}

#[derive(Template)]
#[template(path = "admin/credentials.html")]
struct CredentialsPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    nav_section: &'static str,
    /// Current session role (always Admin here) - drives nav gating.
    role: Role,
    credentials: Vec<db::credentials::CredentialMeta>,
    /// Banner shown when the master key isn't configured. When
    /// true, the form is disabled and a hint is rendered.
    key_missing: bool,
    flash_saved: Option<String>,
    flash_error: Option<String>,
}

impl<'a> CredentialsPage<'a> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

async fn index(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    render_index(&state, loc, theme, None, None).await
}

async fn render_index(
    state: &AppState,
    loc: Locale,
    theme: Theme,
    flash_saved: Option<String>,
    flash_error: Option<String>,
) -> Response {
    let Some(pool) = state.sqlite() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let credentials = match db::credentials::list_all(pool).await {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(error = ?err, "list credentials failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let page = CredentialsPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        nav_section: "credentials",
        role: Role::Admin,
        credentials,
        key_missing: !state.master_key.is_configured(),
        flash_saved,
        flash_error,
    };
    super::render(&page)
}

#[derive(Debug, Deserialize)]
pub struct CredentialForm {
    pub name: String,
    pub registry: String,
    pub username: String,
    pub password: String,
}

async fn create_or_update(
    admin: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Form(form): Form<CredentialForm>,
) -> Response {
    let Some(pool) = state.sqlite() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    if !state.master_key.is_configured() {
        return render_index(
            &state,
            loc,
            theme,
            None,
            Some("RUSCKER_MASTER_KEY is not set".into()),
        )
        .await;
    }

    if form.name.trim().is_empty()
        || form.username.trim().is_empty()
        || form.password.is_empty()
    {
        return render_index(
            &state,
            loc,
            theme,
            None,
            Some("name, username and password are required".into()),
        )
        .await;
    }

    let registry = if form.registry.trim().is_empty() {
        "docker.io".to_string()
    } else {
        form.registry.trim().to_string()
    };

    match db::credentials::upsert(
        pool,
        &state.master_key,
        form.name.trim(),
        &registry,
        form.username.trim(),
        &form.password,
        Some(admin.actor()),
    )
    .await
    {
        Ok(_) => render_index(&state, loc, theme, Some(form.name.trim().to_string()), None).await,
        Err(err) => {
            tracing::error!(error = ?err, "credential upsert failed");
            render_index(&state, loc, theme, None, Some(err.to_string())).await
        }
    }
}

async fn delete(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Path(name): Path<String>,
) -> Response {
    let Some(pool) = state.sqlite() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    match db::credentials::delete_one(pool, &name, Some(admin.actor())).await {
        Ok(_) => Redirect::to("/admin/credentials").into_response(),
        Err(err) => {
            tracing::error!(error = ?err, name, "credential delete failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response()
        }
    }
}
