//! Public landing page (`GET /`).
//!
//! Renders `templates/landing.html` with the parsed `Config`, the
//! resolved locale (cookie → Accept-Language → pt-BR), and the
//! user's theme choice (cookie → auto).

use askama::Template;
use axum::{
    extract::State,
    http::{header, HeaderValue, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::get,
    Router,
};
use fluent_bundle::{FluentArgs, FluentValue};

use crate::auth::{MaybeSession, Role};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::view_model::{
    build_type_chips, sort_by_recent, unique_subjects, CardCounts, CardCtx, TypeChip,
};
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/", get(index))
}

#[derive(Template)]
#[template(path = "landing.html")]
struct LandingPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    cards: Vec<CardCtx<'a>>,
    type_chips: Vec<TypeChip>,
    /// Unique themes present in this config, alphabetically. Drives
    /// the `<select>` filter at the top of the landing.
    subjects: Vec<&'a str>,
    counts: CardCounts,
    /// Resolved per-locale intro text, or empty string when no
    /// `landing-customization.intro` is configured.
    intro: String,
    /// Inline `style="..."` value for the `<header>` element when
    /// the operator set a custom background color. Empty string ⇒
    /// no override.
    header_style: String,
    /// Effective page title — `landing-customization.seo-title` when
    /// set, otherwise the localized `landing-title`. Drives both the
    /// `<title>` tag and `og:title`.
    page_title: String,
    /// `<meta name="description">` / `og:description` — `seo-description`
    /// when set, otherwise the resolved intro. Empty ⇒ tags omitted.
    seo_description: String,
    /// `og:image` URL, or empty ⇒ tag omitted.
    og_image: String,
    /// Operator analytics snippet, injected verbatim into `<head>`
    /// (rendered with `|safe`). Empty ⇒ nothing injected.
    analytics_html: String,
    /// Custom HTML blocks rendered after the header (`top` slot) and
    /// after the card grid (`bottom` slot), in `position` order.
    blocks_top: Vec<crate::db::landing_blocks::LandingBlock>,
    blocks_bottom: Vec<crate::db::landing_blocks::LandingBlock>,
    /// True when the request carries a live admin session. Drives the
    /// header affordance: a "go to the panel" link + sign-out instead
    /// of "sign in".
    signed_in: bool,
    /// Display name of the signed-in viewer (username, or empty for a
    /// break-glass token session). Shown next to the panel link.
    viewer_name: String,
}

impl<'a> LandingPage<'a> {
    /// Translation helper used by the template as `self.t("key")`.
    /// Centralizing here keeps templates clean of explicit
    /// bundle/locale handling.
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// Translation with a single Fluent variable (most common case
    /// is `{ $date }`). Avoids dragging the `FluentArgs` builder
    /// into templates.
    fn t_with(&self, key: &str, arg_name: &str, value: &str) -> String {
        let mut args = FluentArgs::new();
        args.set(arg_name.to_string(), FluentValue::from(value.to_string()));
        self.locales.t(self.locale, key, Some(&args))
    }
}

async fn index(
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    MaybeSession(session): MaybeSession,
) -> Response {
    // Resolve the viewer for app-visibility filtering (#155):
    //  - an Admin role (incl. the break-glass token) sees every spec;
    //  - a named login sees open specs plus those matching its username
    //    or any of its groups;
    //  - an anonymous visitor sees only the open specs.
    let is_admin = session.as_ref().map(|s| s.role == Role::Admin).unwrap_or(false);
    let username = session.as_ref().and_then(|s| s.actor.clone());
    let groups: Vec<String> = match (username.as_deref(), state.db.as_ref()) {
        (Some(user), Some(db)) => crate::db::users::fetch(db, user)
            .await
            .ok()
            .flatten()
            .map(|row| row.groups)
            .unwrap_or_default(),
        _ => Vec::new(),
    };

    let mut cards: Vec<CardCtx<'_>> = state
        .config
        .proxy
        .specs
        .iter()
        .filter(|spec| spec.access_allows(is_admin, username.as_deref(), &groups))
        .map(CardCtx::from_spec)
        .collect();
    sort_by_recent(&mut cards);
    let type_chips = build_type_chips(&cards);
    let subjects = unique_subjects(&cards);
    let counts = CardCounts {
        total: cards.iter().filter(|c| c.active).count(),
    };

    // Landing customization: read from DB when available
    // (admin-editable), fall back to the YAML-derived value
    // otherwise (Phase 1 / no-DB deployments).
    let lc = match state.db.as_ref() {
        Some(db) => crate::db::landing::fetch(db)
            .await
            .unwrap_or_else(|err| {
                tracing::warn!(error = ?err, "landing customization fetch failed; using YAML");
                state.config.proxy.landing_customization.clone()
            }),
        None => state.config.proxy.landing_customization.clone(),
    };
    let header_style = match (&lc.header_bg, &lc.header_fg) {
        (Some(bg), Some(fg)) => format!("background: {}; color: {};", bg, fg),
        (Some(bg), None) => format!("background: {};", bg),
        (None, Some(fg)) => format!("color: {};", fg),
        (None, None) => String::new(),
    };
    let intro = lc
        .intro_locales
        .get(loc.code())
        .cloned()
        .or_else(|| lc.intro.clone())
        .unwrap_or_default();

    // SEO: explicit overrides win; otherwise sensible fallbacks
    // (title → localized `landing-title`, description → intro).
    let not_blank = |s: &Option<String>| {
        s.as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(str::to_string)
    };
    let page_title =
        not_blank(&lc.seo_title).unwrap_or_else(|| state.locales.t(loc, "landing-title", None));
    let seo_description = not_blank(&lc.seo_description).unwrap_or_else(|| intro.clone());
    let og_image = not_blank(&lc.og_image).unwrap_or_default();
    let analytics_html = not_blank(&lc.analytics_html).unwrap_or_default();

    // Custom HTML blocks (DB-only). Split into the two slots; collect
    // their CSP origins alongside the analytics ones so embedded
    // content can load.
    let (mut blocks_top, mut blocks_bottom) = (Vec::new(), Vec::new());
    let mut origins = not_blank(&lc.analytics_origins).unwrap_or_default();
    if let Some(db) = state.db.as_ref() {
        match crate::db::landing_blocks::list_enabled(db).await {
            Ok(blocks) => {
                for b in blocks {
                    if !b.csp_origins.trim().is_empty() {
                        origins.push(' ');
                        origins.push_str(b.csp_origins.trim());
                    }
                    match b.slot.as_str() {
                        "bottom" => blocks_bottom.push(b),
                        _ => blocks_top.push(b),
                    }
                }
            }
            Err(err) => tracing::warn!(error = ?err, "landing blocks fetch failed"),
        }
    }
    let analytics_origins = origins.trim().to_string();

    let page = LandingPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        cards,
        type_chips,
        subjects,
        counts,
        intro,
        header_style,
        page_title,
        seo_description,
        og_image,
        analytics_html,
        blocks_top,
        blocks_bottom,
        signed_in: session.is_some(),
        viewer_name: username.unwrap_or_default(),
    };
    let mut resp = render(&page);
    // Widen *this page's* CSP so the analytics script can load/report.
    // `security_headers` uses `or_insert`, so this handler-set value
    // wins. Only applied when the operator listed origins.
    if !analytics_origins.is_empty() {
        if let Ok(v) = HeaderValue::from_str(&crate::content_security_policy(&analytics_origins)) {
            resp.headers_mut()
                .insert(header::CONTENT_SECURITY_POLICY, v);
        }
    }
    resp
}

/// Centralized `askama::Template` → axum `Response`. Replaces the
/// deprecated `askama_axum` crate.
fn render<T: Template>(t: &T) -> Response {
    match t.render() {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!(error = ?err, "template render failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "template error").into_response()
        }
    }
}
