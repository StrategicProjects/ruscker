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

use super::blocks::{grouped_blocks, SlotGroup};
use crate::auth::{RequireAdmin, Role};
use crate::db;
use crate::db::landing_blocks;
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
    /// Mount prefix for base-path-correct URLs (#294).
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    /// Current session role (always Admin here) - drives nav gating.
    role: Role,
    form: LandingForm,
    flash_saved: bool,
    flash_error: Option<String>,
    /// The portal title (read from config — operator can't change
    /// it through this form yet; it's set in the YAML).
    portal_title: String,
    /// Custom HTML blocks, grouped per slot. The blocks editor is
    /// folded into this page (#242) and rendered below the form.
    groups: Vec<SlotGroup>,
    /// Media-library filenames for the logos picker (#451) — the same
    /// shared gallery the spec form uses (includes the built-in logos
    /// seeded into the library, #433). Empty when no DB is wired.
    logo_images: Vec<String>,
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

    /// JSON array of media-library filenames for the logos picker (#451),
    /// passed as the 2nd argument to the Alpine `landingForm` factory —
    /// mirrors the spec form's `logo_images_json`.
    fn logo_images_json(&self) -> String {
        serde_json::to_string(&self.logo_images).unwrap_or_else(|_| "[]".into())
    }
}

/// Flat form structure mirroring the customization fields. The
/// 4 named per-locale intros keep the form simple instead of
/// pushing a dynamic key/value table; if a fifth language ever
/// ships, this struct and the template add one field each.
#[derive(Debug, Default, serde::Serialize, Deserialize)]
#[serde(default)]
pub struct LandingForm {
    pub title: String,
    pub subtitle: String,
    // Per-theme color overrides (#475); blank ⇒ keep the theme default.
    pub theme_light_bg: String,
    pub theme_light_text: String,
    pub theme_light_accent: String,
    pub theme_dark_bg: String,
    pub theme_dark_text: String,
    pub theme_dark_accent: String,
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
    pub custom_css: String,
    /// Header/footer logos as a JSON array (the editor maintains the list
    /// client-side and submits it in one hidden field). Empty/blank ⇒ no
    /// logos.
    #[serde(default)]
    pub logos_json: String,
    /// "Featured" carousel toggle (#506) — an HTML checkbox: present ("on")
    /// when checked, absent when not (hence `#[serde(default)]`).
    #[serde(default)]
    pub show_highlights: String,
}

impl LandingForm {
    pub fn from_customization(lc: &LandingCustomization) -> Self {
        let g = |code: &str| lc.intro_locales.get(code).cloned().unwrap_or_default();
        let tc = &lc.theme_colors;
        let s = |o: &Option<String>| o.clone().unwrap_or_default();
        Self {
            title: lc.title.clone().unwrap_or_default(),
            subtitle: lc.subtitle.clone().unwrap_or_default(),
            theme_light_bg: s(&tc.light.bg),
            theme_light_text: s(&tc.light.text),
            theme_light_accent: s(&tc.light.accent),
            theme_dark_bg: s(&tc.dark.bg),
            theme_dark_text: s(&tc.dark.text),
            theme_dark_accent: s(&tc.dark.accent),
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
            custom_css: lc.custom_css.clone().unwrap_or_default(),
            logos_json: serde_json::to_string(&lc.logos).unwrap_or_else(|_| "[]".into()),
            // Pre-check the box from the stored value (defaults to on).
            show_highlights: if lc.effective_show_highlights() {
                "on".into()
            } else {
                String::new()
            },
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
        let theme_colors = ruscker_config::ThemeColors {
            light: ruscker_config::ThemePalette {
                bg: empty_to_none(self.theme_light_bg),
                text: empty_to_none(self.theme_light_text),
                accent: empty_to_none(self.theme_light_accent),
            },
            dark: ruscker_config::ThemePalette {
                bg: empty_to_none(self.theme_dark_bg),
                text: empty_to_none(self.theme_dark_text),
                accent: empty_to_none(self.theme_dark_accent),
            },
        };
        LandingCustomization {
            title: empty_to_none(self.title),
            subtitle: empty_to_none(self.subtitle),
            theme_colors,
            header_bg: empty_to_none(self.header_bg),
            header_fg: empty_to_none(self.header_fg),
            intro: empty_to_none(self.intro),
            intro_locales,
            seo_title: empty_to_none(self.seo_title),
            seo_description: empty_to_none(self.seo_description),
            og_image: empty_to_none(self.og_image),
            analytics_html: empty_to_none(self.analytics_html),
            analytics_origins: empty_to_none(self.analytics_origins),
            custom_css: empty_to_none(self.custom_css),
            // The landing editor doesn't manage blocks (their own
            // screen does); `landing::update` ignores this field, so
            // an empty Vec here never clobbers stored blocks.
            blocks: Vec::new(),
            // `show-admin-link` is a YAML deploy policy (#156), not an
            // editor field — never persisted from the form.
            show_admin_link: None,
            // "Featured" carousel toggle (#506) — checkbox present ⇒ on.
            show_highlights: Some(!self.show_highlights.trim().is_empty()),
            logos: parse_logos(&self.logos_json),
        }
    }
}

fn empty_to_none(s: String) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}

/// Parse + sanitize the editor's logos JSON: drop entries without a URL,
/// normalize `slot`/`align` to known values (default `header`/`left`), and
/// collapse blank links to `None`. Malformed JSON ⇒ no logos.
fn parse_logos(json: &str) -> Vec<ruscker_config::LandingLogo> {
    let raw: Vec<ruscker_config::LandingLogo> =
        serde_json::from_str(json.trim()).unwrap_or_default();
    raw.into_iter()
        .filter(|l| !l.url.trim().is_empty())
        .map(|mut l| {
            l.url = l.url.trim().to_string();
            l.slot = if l.slot == "footer" { "footer" } else { "header" }.to_string();
            l.align = match l.align.as_str() {
                "center" => "center",
                "right" => "right",
                _ => "left",
            }
            .to_string();
            l.link = l.link.and_then(|s| {
                let t = s.trim().to_string();
                (!t.is_empty()).then_some(t)
            });
            l
        })
        .collect()
}


async fn index(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    render(&state, loc, theme, None, false, None).await
}

async fn save(
    admin: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Form(form): Form<LandingForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let lc = form.into_customization();
    match db::landing::update(pool, &lc, Some(admin.actor())).await {
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
    let Some(database) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let form = match preset_form {
        Some(f) => f,
        None => match db::landing::fetch(database).await {
            Ok(lc) => LandingForm::from_customization(&lc),
            Err(err) => {
                tracing::error!(error = ?err, "landing fetch failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        },
    };
    // Custom HTML blocks live in their own table; the editor is folded
    // into this page (#242), so load + group them here. `list_all`
    // already orders by (slot, position).
    let groups = match landing_blocks::list_all(database).await {
        Ok(blocks) => grouped_blocks(&blocks),
        Err(err) => {
            tracing::error!(error = ?err, "list blocks failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    // Media-library filenames for the shared logos picker (#451).
    // Non-fatal: a listing failure just leaves the gallery empty, the
    // manual URL field still works.
    let logo_images = match db::images::list_all(database).await {
        Ok(imgs) => imgs.into_iter().map(|i| i.filename).collect(),
        Err(err) => {
            tracing::warn!(error = ?err, "landing: list images for logo picker failed");
            Vec::new()
        }
    };
    let page = LandingPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "landing",
        role: Role::Admin,
        form,
        flash_saved,
        flash_error,
        portal_title: state.config.proxy.title.trim().to_string(),
        groups,
        logo_images,
    };
    super::render(&page)
}

#[cfg(test)]
mod logo_tests {
    use super::parse_logos;

    #[test]
    fn parse_logos_sanitizes() {
        let json = r#"[
            {"url":" /assets/brand/mark.svg ","slot":"footer","align":"right","link":" ","height":40},
            {"url":"","slot":"header","align":"left"},
            {"url":"/x.png","slot":"bogus","align":"bogus"}
        ]"#;
        let out = parse_logos(json);
        assert_eq!(out.len(), 2, "empty-url entry dropped");
        assert_eq!(out[0].url, "/assets/brand/mark.svg");
        assert_eq!(out[0].slot, "footer");
        assert_eq!(out[0].align, "right");
        assert_eq!(out[0].link, None, "blank link → None");
        assert_eq!(out[0].height, Some(40));
        assert_eq!(out[1].slot, "header", "unknown slot → header");
        assert_eq!(out[1].align, "left", "unknown align → left");
    }

    #[test]
    fn parse_logos_malformed_is_empty() {
        assert!(parse_logos("not json").is_empty());
        assert!(parse_logos("").is_empty());
    }
}
