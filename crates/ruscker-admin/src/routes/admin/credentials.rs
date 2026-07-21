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
use std::collections::BTreeSet;

use crate::auth::{RequireAdmin, Role};
use crate::db;
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

use super::KpiMetric;

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
    /// Mount prefix for base-path-correct URLs (#294).
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    /// Current session role (always Admin here) - drives nav gating.
    role: Role,
    credentials: Vec<db::credentials::CredentialMeta>,
    kpi_total: i64,
    kpi_in_use: i64,
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

    fn kpis(&self) -> [KpiMetric; 2] {
        [
            KpiMetric::new("ti-key", "admin-creds-kpi-total", self.kpi_total),
            KpiMetric::new("ti-app-window", "admin-creds-kpi-in-use", self.kpi_in_use),
        ]
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
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let credentials = match db::credentials::list_all(pool).await {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(error = ?err, "list credentials failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let referenced: BTreeSet<String> = crate::catalog::effective_specs_cached(state)
        .await
        .iter()
        .filter_map(|spec| spec.docker_registry_credential.clone())
        .collect();
    let kpi_total = credentials.len() as i64;
    let kpi_in_use = credentials
        .iter()
        .filter(|credential| referenced.contains(&credential.name))
        .count() as i64;
    let page = CredentialsPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "credentials",
        role: Role::Admin,
        credentials,
        kpi_total,
        kpi_in_use,
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

/// A credential name is safe iff it's non-empty (after trim) and made only
/// of identifier-ish chars. It lands un-encoded in the delete URL's single
/// path segment, so anything else (`/`, `?`, `#`, space, …) would make the
/// credential undeletable from the UI (#423).
fn is_valid_credential_name(name: &str) -> bool {
    let t = name.trim();
    !t.is_empty()
        && t.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
}

async fn create_or_update(
    admin: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Form(form): Form<CredentialForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
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

    // Restrict the name to a safe charset (#423). The delete action puts the
    // name in a single URL path segment (`/admin/credentials/{name}/delete`)
    // un-encoded, so a `/`, `?`, `#`, space, … would make the credential
    // impossible to delete from the UI. Keep it to identifier-ish chars.
    if !is_valid_credential_name(&form.name) {
        return render_index(
            &state,
            loc,
            theme,
            None,
            Some("name may only contain letters, digits, '_', '.' and '-'".into()),
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
    let Some(pool) = state.db.as_ref() else {
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

#[cfg(test)]
mod tests {
    use super::is_valid_credential_name;

    #[test]
    fn credential_name_charset_is_restricted() {
        assert!(is_valid_credential_name("dockerhub-milkway"));
        assert!(is_valid_credential_name("reg_1.prod"));
        assert!(is_valid_credential_name("  trimmed-ok  "));
        // Undeletable-name cases (#423): break the delete URL path segment.
        assert!(!is_valid_credential_name("foo/bar"));
        assert!(!is_valid_credential_name("a?b"));
        assert!(!is_valid_credential_name("a#b"));
        assert!(!is_valid_credential_name("with space"));
        assert!(!is_valid_credential_name(""));
        assert!(!is_valid_credential_name("   "));
    }
}
