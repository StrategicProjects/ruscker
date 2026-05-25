//! Admin > Landing-page editor.
//!
//! MVP scope (matches what `LandingCustomization` already
//! supports): header background + foreground colors, fallback
//! intro paragraph, and per-locale intro overrides for the four
//! shipped languages.
//!
//! The mockup `docs/mockups/admin-landing-editor.html` shows a
//! bigger vision: drag-to-reorder sections, custom HTML blocks,
//! analytics, OG tags. Those need schema additions and ship in
//! later slices on this branch.

use askama::Template;
use axum::{
    extract::{Form, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::get,
    Router,
};
use ruscker_config::LandingCustomization;
use serde::Deserialize;
use std::collections::HashMap;

use crate::auth::AdminSession;
use crate::db;
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/admin/landing", get(index).post(save))
}

#[derive(Template)]
#[template(path = "admin/landing.html")]
struct LandingPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    nav_section: &'static str,
    form: LandingForm,
    flash_saved: bool,
    flash_error: Option<String>,
    /// The portal title (read from config — operator can't change
    /// it through this form yet; it's set in the YAML).
    portal_title: String,
}

impl<'a> LandingPage<'a> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// JSON literal for Alpine x-data. Same trick as the spec
    /// form: don't `|safe` it — Askama auto-escape turns quotes
    /// into `&quot;` so the attribute parses cleanly.
    fn form_initial_json(&self) -> String {
        serde_json::to_string(&self.form).unwrap_or_else(|_| "{}".into())
    }
}

/// Flat form structure mirroring the customization fields. The
/// 4 named per-locale intros keep the form simple instead of
/// pushing a dynamic key/value table; if a fifth language ever
/// ships, this struct and the template add one field each.
#[derive(Debug, Default, serde::Serialize, Deserialize)]
#[serde(default)]
pub struct LandingForm {
    pub header_bg: String,
    pub header_fg: String,
    pub intro: String,
    pub intro_pt: String,
    pub intro_en: String,
    pub intro_es: String,
    pub intro_fr: String,
    pub seo_title: String,
    pub seo_description: String,
    pub og_image: String,
    pub analytics_html: String,
    pub analytics_origins: String,
}

impl LandingForm {
    pub fn from_customization(lc: &LandingCustomization) -> Self {
        let g = |code: &str| lc.intro_locales.get(code).cloned().unwrap_or_default();
        Self {
            header_bg: lc.header_bg.clone().unwrap_or_default(),
            header_fg: lc.header_fg.clone().unwrap_or_default(),
            intro: lc.intro.clone().unwrap_or_default(),
            intro_pt: g("pt"),
            intro_en: g("en"),
            intro_es: g("es"),
            intro_fr: g("fr"),
            seo_title: lc.seo_title.clone().unwrap_or_default(),
            seo_description: lc.seo_description.clone().unwrap_or_default(),
            og_image: lc.og_image.clone().unwrap_or_default(),
            analytics_html: lc.analytics_html.clone().unwrap_or_default(),
            analytics_origins: lc.analytics_origins.clone().unwrap_or_default(),
        }
    }

    pub fn into_customization(self) -> LandingCustomization {
        let mut intro_locales: HashMap<String, String> = HashMap::new();
        for (code, val) in [
            ("pt", self.intro_pt),
            ("en", self.intro_en),
            ("es", self.intro_es),
            ("fr", self.intro_fr),
        ] {
            let trimmed = val.trim();
            if !trimmed.is_empty() {
                intro_locales.insert(code.into(), trimmed.to_string());
            }
        }
        LandingCustomization {
            header_bg: empty_to_none(self.header_bg),
            header_fg: empty_to_none(self.header_fg),
            intro: empty_to_none(self.intro),
            intro_locales,
            seo_title: empty_to_none(self.seo_title),
            seo_description: empty_to_none(self.seo_description),
            og_image: empty_to_none(self.og_image),
            analytics_html: empty_to_none(self.analytics_html),
            analytics_origins: empty_to_none(self.analytics_origins),
            // The landing editor doesn't manage blocks (their own
            // screen does); `landing::update` ignores this field, so
            // an empty Vec here never clobbers stored blocks.
            blocks: Vec::new(),
        }
    }
}

fn empty_to_none(s: String) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

async fn index(
    _: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    render(&state, loc, theme, None, false, None).await
}

async fn save(
    _: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Form(form): Form<LandingForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let lc = form.into_customization();
    match db::landing::update(pool, &lc, Some("admin")).await {
        Ok(_) => render(&state, loc, theme, Some(LandingForm::from_customization(&lc)), true, None).await,
        Err(err) => {
            tracing::error!(error = ?err, "landing update failed");
            render(
                &state,
                loc,
                theme,
                Some(LandingForm::from_customization(&lc)),
                false,
                Some(err.to_string()),
            )
            .await
        }
    }
}

async fn render(
    state: &AppState,
    loc: Locale,
    theme: Theme,
    preset_form: Option<LandingForm>,
    flash_saved: bool,
    flash_error: Option<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let form = match preset_form {
        Some(f) => f,
        None => match db::landing::fetch(pool).await {
            Ok(lc) => LandingForm::from_customization(&lc),
            Err(err) => {
                tracing::error!(error = ?err, "landing fetch failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        },
    };
    let page = LandingPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        nav_section: "landing",
        form,
        flash_saved,
        flash_error,
        portal_title: state.config.proxy.title.trim().to_string(),
    };
    super::render(&page)
}
