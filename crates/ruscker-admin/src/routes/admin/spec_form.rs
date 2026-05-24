//! Admin > Add/Edit spec form.
//!
//! Single template (`templates/admin/spec_form.html`) used for both
//! New and Edit, parameterized by `mode`. On submit, the form posts
//! to `/admin/specs` (create) or `/admin/specs/:id` (update); on
//! delete, to `/admin/specs/:id/delete`.
//!
//! Field scope covers what the SEPE YAML actually uses today
//! (~80% of specs). Phase 2.5 adds advanced fields (replicas,
//! scaling, env vars, volumes) behind a collapsible section.

use anyhow::Result;
use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use chrono::Utc;
use ruscker_config::{Spec, SpecKindOverride, TemplateProperties};
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value as YamlValue;
use std::collections::HashMap;

use crate::auth::AdminSession;
use crate::db;
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::view_model::DisplayType;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/specs/new", get(new_form))
        .route("/admin/specs", post(create))
        .route(
            "/admin/specs/{id}/edit",
            get(edit_form),
        )
        .route("/admin/specs/{id}", post(update))
        .route("/admin/specs/{id}/delete", post(delete))
}

// ── Form payload ────────────────────────────────────────────────

/// Mirror of the form fields. Strings are unconditional so empty
/// inputs round-trip as `""` rather than disappearing; conversion
/// to [`Spec`] handles "empty means None".
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SpecForm {
    pub id: String,
    pub display_name: String,
    pub description: String,
    /// "app" | "talk" | "report" | "package" | "api" | "link"
    pub display_type: String,
    pub container_image: String,
    /// "active" | "inactive"
    pub state: String,
    /// "lock" (restricted) | "lock_open" (public)
    pub access: String,
    pub tema: String,
    pub logo: String,
    /// Updated date in DD/MM/YYYY. Empty ⇒ stamp with today.
    pub updated: String,
    /// External link target (for type=link/package).
    pub link: String,
    pub seats_per_container: String,
    pub max_lifetime: String,
}

impl SpecForm {
    /// Build a form pre-filled from an existing [`Spec`] for the
    /// edit view.
    pub fn from_spec(spec: &Spec) -> Self {
        let tp = &spec.template_properties;
        let dt = DisplayType::from_spec(spec);
        Self {
            id: spec.id.clone(),
            display_name: spec.display_name.clone().unwrap_or_default(),
            description: spec.description.clone().unwrap_or_default(),
            display_type: dt.key().to_string(),
            container_image: spec.container_image.clone().unwrap_or_default(),
            state: tp
                .get_str("state")
                .map(str::to_string)
                .unwrap_or_else(|| "active".into()),
            access: tp
                .get_str("icon")
                .map(str::to_string)
                .unwrap_or_else(|| "lock".into()),
            tema: tp.get_str("tema").map(str::to_string).unwrap_or_default(),
            logo: tp.get_str("logo").map(str::to_string).unwrap_or_default(),
            updated: tp.get_str("updated").map(str::to_string).unwrap_or_default(),
            link: tp.get_str("link").map(str::to_string).unwrap_or_default(),
            seats_per_container: spec
                .seats_per_container
                .map(|n| n.to_string())
                .unwrap_or_default(),
            max_lifetime: spec.max_lifetime.map(|n| n.to_string()).unwrap_or_default(),
        }
    }

    /// Build a fresh [`Spec`] from the submitted form values.
    /// Empty optional strings become `None`. Numeric strings parse
    /// optimistically; non-parseable values fall back to None
    /// (operator gets a 400 from `validate` before reaching here).
    pub fn into_spec(self) -> Result<Spec> {
        let dt = DisplayType::parse(&self.display_type).unwrap_or(DisplayType::App);
        let kind_override = match dt {
            DisplayType::App => Some(SpecKindOverride::Shiny),
            DisplayType::Talk | DisplayType::Report => None, // these are visual
            DisplayType::Package | DisplayType::Link => Some(SpecKindOverride::External),
            DisplayType::Api => Some(SpecKindOverride::Api),
        };

        let updated = if self.updated.trim().is_empty() {
            Utc::now().format("%d/%m/%Y").to_string()
        } else {
            self.updated.trim().to_string()
        };

        let mut tp_map: HashMap<String, YamlValue> = HashMap::new();
        // type, state, icon are always set so chips/filters render
        tp_map.insert("type".into(), YamlValue::String(dt.key().to_string()));
        tp_map.insert("state".into(), YamlValue::String(self.state.clone()));
        tp_map.insert("icon".into(), YamlValue::String(self.access.clone()));
        tp_map.insert("updated".into(), YamlValue::String(updated));

        if !self.tema.trim().is_empty() {
            tp_map.insert(
                "tema".into(),
                YamlValue::String(self.tema.trim().to_string()),
            );
        }
        if !self.logo.trim().is_empty() {
            tp_map.insert(
                "logo".into(),
                YamlValue::String(self.logo.trim().to_string()),
            );
        }
        if !self.link.trim().is_empty() {
            tp_map.insert(
                "link".into(),
                YamlValue::String(self.link.trim().to_string()),
            );
        }

        let container_image = match dt {
            DisplayType::Package | DisplayType::Link => None,
            _ => empty_to_none(&self.container_image),
        };

        Ok(Spec {
            id: self.id.trim().to_string(),
            display_name: empty_to_none(&self.display_name),
            description: empty_to_none(&self.description),
            container_image,
            seats_per_container: parse_opt(&self.seats_per_container),
            max_lifetime: parse_opt(&self.max_lifetime),
            container_lifetime: None,
            heartbeat_timeout: None,
            stop_on_logout: None,
            docker_registry_username: None,
            docker_registry_password: None,
            docker_registry_domain: None,
            docker_registry_credential: None,
            container_cpu_limit: None,
            container_cpu_request: None,
            container_memory_limit: None,
            container_memory_request: None,
            template_properties: TemplateProperties(tp_map),
            kind_override,
            api: None,
            min_replicas: None,
            max_replicas: None,
            scale_up_threshold: None,
            scale_down_threshold: None,
            scale_down_grace: None,
            drain_timeout: None,
            routing_strategy: None,
            concurrent_requests_per_replica: None,
        })
    }

    /// Server-side validation. Returns a list of fluent message
    /// keys describing each problem; empty list = OK.
    pub fn validate(&self, mode: FormMode) -> Vec<&'static str> {
        let mut errs = Vec::new();
        if self.id.trim().is_empty() {
            errs.push("spec-form-error-id-required");
        } else if !is_kebab_id(self.id.trim()) {
            errs.push("spec-form-error-id-shape");
        }
        if self.display_name.trim().is_empty() {
            errs.push("spec-form-error-name-required");
        }
        if matches!(mode, FormMode::New) && self.id.trim().is_empty() {
            // duplicate-id check happens later (needs DB access)
        }
        errs
    }
}

fn empty_to_none(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}
fn parse_opt<T: std::str::FromStr>(s: &str) -> Option<T> {
    s.trim().parse().ok()
}
fn is_kebab_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && s.chars()
            .next()
            .map(|c| c.is_ascii_alphabetic())
            .unwrap_or(false)
}

// ── Template ────────────────────────────────────────────────────

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FormMode {
    New,
    Edit,
}
impl FormMode {
    pub fn is_new(self) -> bool {
        matches!(self, FormMode::New)
    }
    pub fn is_edit(self) -> bool {
        matches!(self, FormMode::Edit)
    }
}

#[derive(Template)]
#[template(path = "admin/spec_form.html")]
struct SpecFormPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    nav_section: &'static str,
    mode: FormMode,
    form: SpecForm,
    /// Pre-validation errors (Fluent keys) shown above the form.
    errors: Vec<&'static str>,
}

impl<'a> SpecFormPage<'a> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// JSON-encoded initial form values, ready to drop into the
    /// `x-data` attribute of the live-preview Alpine component.
    fn form_initial_json(&self) -> String {
        serde_json::to_string(&self.form).unwrap_or_else(|_| "{}".into())
    }

    /// Options for the kind picker: (key, label-fluent-key, tabler-icon).
    /// Order intentional — mirrors the public landing chip order.
    fn display_type_options(&self) -> &'static [(&'static str, &'static str, &'static str)] {
        &[
            ("app", "spec-form-kind-app", "app-window"),
            ("talk", "spec-form-kind-talk", "presentation"),
            ("report", "spec-form-kind-report", "file-text"),
            ("package", "spec-form-kind-package", "package"),
            ("api", "spec-form-kind-api", "api"),
            ("link", "spec-form-kind-link", "external-link"),
        ]
    }
}

// ── Handlers ─────────────────────────────────────────────────────

async fn new_form(
    _: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    let page = SpecFormPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        nav_section: "specs",
        mode: FormMode::New,
        form: SpecForm {
            // Sensible defaults for a new app
            display_type: "app".into(),
            state: "active".into(),
            access: "lock".into(),
            ..Default::default()
        },
        errors: Vec::new(),
    };
    super::render(&page)
}

async fn edit_form(
    _: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Path(id): Path<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let spec = match db::specs::fetch_one(pool, &id).await {
        Ok(Some(s)) => s,
        Ok(None) => return (StatusCode::NOT_FOUND, "spec not found").into_response(),
        Err(e) => {
            tracing::error!(error = ?e, id, "fetch spec failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let page = SpecFormPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        nav_section: "specs",
        mode: FormMode::Edit,
        form: SpecForm::from_spec(&spec),
        errors: Vec::new(),
    };
    super::render(&page)
}

async fn create(
    _: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Form(form): Form<SpecForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };

    let mut errors = form.validate(FormMode::New);

    // Uniqueness check
    if errors.is_empty() {
        match db::specs::fetch_one(pool, form.id.trim()).await {
            Ok(Some(_)) => errors.push("spec-form-error-id-duplicate"),
            Ok(None) => {}
            Err(e) => {
                tracing::error!(error = ?e, "duplicate-check failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        }
    }

    if !errors.is_empty() {
        return render_form_with_errors(&state, loc, theme, FormMode::New, form, errors);
    }

    let id = form.id.trim().to_string();
    let spec = match form.into_spec() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = ?e, "form → spec failed");
            return (StatusCode::BAD_REQUEST, "invalid form data").into_response();
        }
    };

    match db::specs::upsert_one(pool, &spec, Some("admin")).await {
        Ok(_) => Redirect::to(&format!("/admin/specs/{}/edit", id)).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "save failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "save failed").into_response()
        }
    }
}

async fn update(
    _: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Path(id): Path<String>,
    Form(mut form): Form<SpecForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };

    // The URL id wins over the form id — operators don't get to
    // rename specs through the form (it would orphan the audit
    // log target). Renaming is a separate planned action.
    form.id = id.clone();

    let errors = form.validate(FormMode::Edit);
    if !errors.is_empty() {
        return render_form_with_errors(&state, loc, theme, FormMode::Edit, form, errors);
    }

    let spec = match form.into_spec() {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = ?e, "form → spec failed");
            return (StatusCode::BAD_REQUEST, "invalid form data").into_response();
        }
    };

    match db::specs::upsert_one(pool, &spec, Some("admin")).await {
        Ok(_) => Redirect::to(&format!("/admin/specs/{}/edit", id)).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "save failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "save failed").into_response()
        }
    }
}

async fn delete(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    match db::specs::delete_one(pool, &id, Some("admin")).await {
        Ok(_) => Redirect::to("/admin/specs").into_response(),
        Err(e) => {
            tracing::error!(error = ?e, id, "delete failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response()
        }
    }
}

fn render_form_with_errors(
    state: &AppState,
    loc: Locale,
    theme: Theme,
    mode: FormMode,
    form: SpecForm,
    errors: Vec<&'static str>,
) -> Response {
    let page = SpecFormPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        nav_section: "specs",
        mode,
        form,
        errors,
    };
    let body = match page.render() {
        Ok(s) => s,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "render").into_response(),
    };
    (StatusCode::UNPROCESSABLE_ENTITY, axum::response::Html(body)).into_response()
}
