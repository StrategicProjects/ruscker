//! Admin > SSO > Clients.
//!
//! The registry of systems allowed to "Sign in with Ruscker" (OIDC
//! provider, slice 1 — `docs/adr/0005-oidc-provider.md`). Admin-only.
//!
//! * `GET  /admin/sso-clients`              list + create form
//! * `POST /admin/sso-clients`              create (shows the secret ONCE)
//! * `GET  /admin/sso-clients/{id}/edit`    edit form
//! * `POST /admin/sso-clients/{id}/edit`    save settings
//! * `POST /admin/sso-clients/{id}/rotate`  new secret (shown ONCE)
//! * `POST /admin/sso-clients/{id}/delete`  remove
//!
//! The create and rotate handlers RENDER their result instead of
//! redirecting: the cleartext secret exists only in that one response
//! (`Cache-Control: no-store`), so there is no URL that could replay it.

use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    http::{header::CACHE_CONTROL, HeaderValue, StatusCode},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;

use crate::auth::{RequireAdmin, Role};
use crate::db;
use crate::db::oidc_clients::{
    is_valid_client_id, parse_redirect_uris, validate_redirect_uri, ClientSettings, OidcClient,
    RedirectUriError, RotateOutcome, MAX_REDIRECT_URIS,
};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

use super::KpiMetric;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/sso-clients", get(index).post(create))
        .route("/admin/sso-clients/{id}/edit", get(edit_form).post(edit_submit))
        .route("/admin/sso-clients/{id}/rotate", post(rotate))
        .route("/admin/sso-clients/{id}/delete", post(delete))
}

/// A secret that was just generated, to be displayed exactly once.
struct NewSecret {
    client_id: String,
    secret: String,
}

#[derive(Template)]
#[template(path = "admin/sso_clients.html")]
struct ClientsPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    clients: Vec<OidcClient>,
    /// Sticky create-form values, so a validation error doesn't wipe
    /// what the admin typed.
    form: ClientForm,
    /// Media-library filenames offered as the system's logo.
    logos: Vec<String>,
    new_secret: Option<NewSecret>,
    flash_ok: Option<String>,
    flash_error: Option<String>,
}

impl ClientsPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    fn kpis(&self) -> [KpiMetric; 3] {
        let enabled = self.clients.iter().filter(|c| c.enabled).count() as i64;
        let mfa = self.clients.iter().filter(|c| c.require_mfa).count() as i64;
        [
            KpiMetric::new("ti-login", "admin-sso-kpi-total", self.clients.len() as i64),
            KpiMetric::new("ti-check", "admin-sso-kpi-enabled", enabled),
            KpiMetric::new("ti-lock", "admin-sso-kpi-mfa", mfa),
        ]
    }
}

#[derive(Template)]
#[template(path = "admin/sso_client_edit.html")]
struct ClientEditPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    client_id: String,
    /// Carried by the rotate form: the compare-and-set token that makes
    /// a re-submitted rotation a no-op.
    secret_version: i64,
    form: ClientForm,
    logos: Vec<String>,
    new_secret: Option<NewSecret>,
    flash_ok: Option<String>,
    flash_error: Option<String>,
}

impl ClientEditPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

/// The create/edit form as posted. Checkboxes arrive as `"on"` or are
/// absent, hence `Option<String>`.
#[derive(Debug, Default, Deserialize)]
pub struct ClientForm {
    #[serde(default)]
    pub client_id: String,
    #[serde(default)]
    pub name: String,
    /// One redirect URI per line.
    #[serde(default)]
    pub redirect_uris: String,
    /// Comma-separated group names; empty = any signed-in user.
    #[serde(default)]
    pub access_groups: String,
    #[serde(default)]
    pub require_mfa: Option<String>,
    #[serde(default)]
    pub enabled: Option<String>,
    /// Media-library filename of the system's logo; empty = none.
    #[serde(default)]
    pub logo: String,
}

impl ClientForm {
    fn from_client(c: &OidcClient) -> Self {
        Self {
            client_id: c.client_id.clone(),
            name: c.name.clone(),
            redirect_uris: c.redirect_uris.join("\n"),
            access_groups: c.access_groups.join(", "),
            require_mfa: c.require_mfa.then(|| "on".to_string()),
            enabled: c.enabled.then(|| "on".to_string()),
            logo: c.logo.clone().unwrap_or_default(),
        }
    }

    /// Validate the editable fields. `Err` carries the i18n key of the
    /// message to show.
    fn settings(&self, enabled: bool) -> Result<ClientSettings, &'static str> {
        let name = self.name.trim();
        if name.is_empty() || name.chars().count() > 80 {
            return Err("admin-sso-error-name");
        }
        let redirect_uris = parse_redirect_uris(&self.redirect_uris);
        if redirect_uris.is_empty() {
            return Err("admin-sso-error-uri-required");
        }
        if redirect_uris.len() > MAX_REDIRECT_URIS {
            return Err("admin-sso-error-uri-too-many");
        }
        for uri in &redirect_uris {
            if let Err(e) = validate_redirect_uri(uri) {
                return Err(match e {
                    RedirectUriError::NotAbsolute => "admin-sso-error-uri-absolute",
                    RedirectUriError::InsecureScheme => "admin-sso-error-uri-insecure",
                    RedirectUriError::UnsupportedScheme => "admin-sso-error-uri-scheme",
                    RedirectUriError::HasFragment => "admin-sso-error-uri-fragment",
                    RedirectUriError::HasUserinfo => "admin-sso-error-uri-userinfo",
                    RedirectUriError::HasWildcard => "admin-sso-error-uri-wildcard",
                    RedirectUriError::InvalidCharacters => "admin-sso-error-uri-chars",
                });
            }
        }
        // A bare Media-library filename: it ends up in an image URL on
        // the sign-in page, so no path separators and no leading dot.
        let logo = self.logo.trim();
        if logo.starts_with('.') || logo.contains(['/', '\\']) || logo.chars().count() > 200 {
            return Err("admin-sso-error-logo");
        }
        Ok(ClientSettings {
            name: name.to_string(),
            redirect_uris,
            access_groups: db::users::parse_groups(&self.access_groups),
            require_mfa: self.require_mfa.is_some(),
            enabled,
            logo: (!logo.is_empty()).then(|| logo.to_string()),
        })
    }
}

/// Mark a response that carries a cleartext secret as uncacheable.
fn no_store(mut resp: Response) -> Response {
    resp.headers_mut()
        .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// Media-library filenames for the logo select; empty without a DB.
async fn media_filenames(state: &AppState) -> Vec<String> {
    match state.db.as_ref() {
        Some(pool) => db::images::list_all(pool)
            .await
            .map(|imgs| imgs.into_iter().map(|i| i.filename).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

fn no_db() -> Response {
    (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response()
}

fn db_error(err: anyhow::Error, what: &str) -> Response {
    tracing::error!(error = ?err, "{what} failed");
    (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
}

#[derive(Debug, Default, Deserialize)]
struct FlashQuery {
    flash: Option<String>,
}

async fn index(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(q): Query<FlashQuery>,
) -> Response {
    let flash_ok = match q.flash.as_deref() {
        Some("deleted") => Some(state.locales.t(loc, "admin-sso-flash-deleted", None)),
        _ => None,
    };
    render_index(&state, loc, theme, ClientForm::default(), None, flash_ok, None).await
}

async fn render_index(
    state: &AppState,
    loc: Locale,
    theme: Theme,
    form: ClientForm,
    new_secret: Option<NewSecret>,
    flash_ok: Option<String>,
    flash_error: Option<&'static str>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return no_db();
    };
    let clients = match db::oidc_clients::list_all(pool).await {
        Ok(v) => v,
        Err(err) => return db_error(err, "list oidc clients"),
    };
    let has_secret = new_secret.is_some();
    let page = ClientsPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "sso",
        role: Role::Admin,
        clients,
        form,
        logos: media_filenames(state).await,
        new_secret,
        flash_ok,
        flash_error: flash_error.map(|k| state.locales.t(loc, k, None)),
    };
    let resp = super::render(&page);
    if has_secret {
        no_store(resp)
    } else {
        resp
    }
}

async fn create(
    admin: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Form(form): Form<ClientForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return no_db();
    };
    let client_id = form.client_id.trim().to_string();
    if !is_valid_client_id(&client_id) {
        return render_index(&state, loc, theme, form, None, None, Some("admin-sso-error-id")).await;
    }
    // A new client always starts enabled; disabling is an edit.
    let settings = match form.settings(true) {
        Ok(s) => s,
        Err(key) => return render_index(&state, loc, theme, form, None, None, Some(key)).await,
    };
    match db::oidc_clients::create(pool, &client_id, &settings, Some(admin.actor())).await {
        Ok(Some(secret)) => {
            let shown = NewSecret { client_id, secret };
            render_index(&state, loc, theme, ClientForm::default(), Some(shown), None, None).await
        }
        Ok(None) => {
            render_index(&state, loc, theme, form, None, None, Some("admin-sso-error-exists")).await
        }
        Err(err) => db_error(err, "create oidc client"),
    }
}

/// What the edit page reports back after an action — at most one of
/// these applies to a given response.
enum EditNotice {
    None,
    /// Settings saved (i18n key of the message).
    Ok(&'static str),
    /// Validation failed (i18n key of the message).
    Error(&'static str),
    /// A secret was just rotated and must be shown once.
    Secret(NewSecret),
}

async fn render_edit(
    state: &AppState,
    loc: Locale,
    theme: Theme,
    client: &OidcClient,
    form: ClientForm,
    notice: EditNotice,
) -> Response {
    let (new_secret, flash_ok, flash_error) = match notice {
        EditNotice::None => (None, None, None),
        EditNotice::Ok(k) => (None, Some(k), None),
        EditNotice::Error(k) => (None, None, Some(k)),
        EditNotice::Secret(s) => (Some(s), None, None),
    };
    let has_secret = new_secret.is_some();
    let page = ClientEditPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "sso",
        role: Role::Admin,
        client_id: client.client_id.clone(),
        secret_version: client.secret_version,
        form,
        logos: media_filenames(state).await,
        new_secret,
        flash_ok: flash_ok.map(|k| state.locales.t(loc, k, None)),
        flash_error: flash_error.map(|k| state.locales.t(loc, k, None)),
    };
    let resp = super::render(&page);
    if has_secret {
        no_store(resp)
    } else {
        resp
    }
}

/// Load the client a per-client route is about, or the response to
/// return instead (404 when it does not exist). Every such handler
/// starts here so an unknown id is a 404 on ALL of them, whatever the
/// rest of the request looks like.
async fn load(state: &AppState, id: &str) -> Result<OidcClient, Response> {
    let Some(pool) = state.db.as_ref() else {
        return Err(no_db());
    };
    match db::oidc_clients::fetch(pool, id).await {
        Ok(Some(c)) => Ok(c),
        Ok(None) => Err(StatusCode::NOT_FOUND.into_response()),
        Err(err) => Err(db_error(err, "fetch oidc client")),
    }
}

async fn edit_form(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Path(id): Path<String>,
) -> Response {
    let client = match load(&state, &id).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let form = ClientForm::from_client(&client);
    render_edit(&state, loc, theme, &client, form, EditNotice::None).await
}

async fn edit_submit(
    admin: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Path(id): Path<String>,
    Form(form): Form<ClientForm>,
) -> Response {
    // Existence first: a validation error must not render an edit page
    // (with its rotate/delete forms) for a client that does not exist.
    let client = match load(&state, &id).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let Some(pool) = state.db.as_ref() else {
        return no_db();
    };
    let settings = match form.settings(form.enabled.is_some()) {
        Ok(s) => s,
        Err(key) => {
            return render_edit(&state, loc, theme, &client, form, EditNotice::Error(key)).await
        }
    };
    match db::oidc_clients::update(pool, &id, &settings, Some(admin.actor())).await {
        Ok(true) => {
            let saved = EditNotice::Ok("admin-sso-flash-saved");
            render_edit(&state, loc, theme, &client, form, saved).await
        }
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => db_error(err, "update oidc client"),
    }
}

#[derive(Debug, Deserialize)]
struct RotateForm {
    /// The `secret_version` the edit page was rendered with.
    secret_version: i64,
}

async fn rotate(
    admin: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Path(id): Path<String>,
    Form(f): Form<RotateForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return no_db();
    };
    let outcome =
        match db::oidc_clients::rotate_secret(pool, &id, f.secret_version, Some(admin.actor())).await {
            Ok(o) => o,
            Err(err) => return db_error(err, "rotate oidc client secret"),
        };
    // Re-read AFTER the rotation so the page (and its rotate form)
    // carries the new version.
    let client = match load(&state, &id).await {
        Ok(c) => c,
        Err(resp) => return resp,
    };
    let form = ClientForm::from_client(&client);
    let notice = match outcome {
        RotateOutcome::Rotated(secret) => EditNotice::Secret(NewSecret {
            client_id: client.client_id.clone(),
            secret,
        }),
        // A replayed request (F5 on the result page, a second tab):
        // nothing was rotated, and the admin is told why.
        RotateOutcome::Stale => EditNotice::Error("admin-sso-error-rotate-stale"),
        RotateOutcome::Missing => return StatusCode::NOT_FOUND.into_response(),
    };
    render_edit(&state, loc, theme, &client, form, notice).await
}

async fn delete(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return no_db();
    };
    match db::oidc_clients::delete(pool, &id, Some(admin.actor())).await {
        Ok(true) => Redirect::to("/admin/sso-clients?flash=deleted").into_response(),
        Ok(false) => StatusCode::NOT_FOUND.into_response(),
        Err(err) => db_error(err, "delete oidc client"),
    }
}

#[cfg(test)]
mod tests {
    use super::ClientForm;

    fn form(uris: &str) -> ClientForm {
        ClientForm {
            client_id: "radar".into(),
            name: "Radar".into(),
            redirect_uris: uris.into(),
            ..Default::default()
        }
    }

    #[test]
    fn settings_maps_each_failure_to_its_message() {
        assert_eq!(form("").settings(true).unwrap_err(), "admin-sso-error-uri-required");
        assert_eq!(
            form("http://app.example/cb").settings(true).unwrap_err(),
            "admin-sso-error-uri-insecure"
        );
        assert_eq!(
            form("https://ok.example/cb\nhttps://*.example/cb").settings(true).unwrap_err(),
            "admin-sso-error-uri-wildcard"
        );
        let many: String = (0..11).map(|i| format!("https://a.example/{i}\n")).collect();
        assert_eq!(form(&many).settings(true).unwrap_err(), "admin-sso-error-uri-too-many");
        let mut f = form("https://ok.example/cb");
        f.name = "  ".into();
        assert_eq!(f.settings(true).unwrap_err(), "admin-sso-error-name");
    }

    #[test]
    fn logo_must_be_a_bare_media_filename() {
        let mut f = form("https://ok.example/cb");
        assert_eq!(f.settings(true).unwrap().logo, None);
        f.logo = " radar logo.svg ".into();
        assert_eq!(f.settings(true).unwrap().logo.as_deref(), Some("radar logo.svg"));
        for bad in ["../secret", "a/b.svg", "a\\b.svg", ".hidden"] {
            f.logo = bad.into();
            assert_eq!(f.settings(true).unwrap_err(), "admin-sso-error-logo", "{bad}");
        }
    }

    #[test]
    fn settings_reads_checkboxes_and_groups() {
        let mut f = form("https://ok.example/cb");
        f.access_groups = " fiscal, gestores ,fiscal".into();
        f.require_mfa = Some("on".into());
        let s = f.settings(false).unwrap();
        assert_eq!(s.access_groups, vec!["fiscal".to_string(), "gestores".to_string()]);
        assert!(s.require_mfa);
        assert!(!s.enabled);
    }
}
