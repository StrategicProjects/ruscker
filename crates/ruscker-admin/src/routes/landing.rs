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
    /// Mount prefix for base-path-correct URLs (#294): templates emit
    /// `{{ base }}/...` so the HTML needs no response-body rewrite.
    base: std::sync::Arc<str>,
    cards: Vec<CardCtx<'a>>,
    type_chips: Vec<TypeChip>,
    /// Unique themes present in this config, alphabetically. Drives
    /// the `<select>` filter at the top of the landing.
    subjects: Vec<&'a str>,
    counts: CardCounts,
    /// Resolved per-locale intro text, or empty string when no
    /// `landing-customization.intro` is configured.
    intro: String,
    /// Header title — `landing-customization.title` override, else
    /// `proxy.title`, else the localized `landing-title` (#468).
    header_title: String,
    /// Header subtitle — `landing-customization.subtitle` override, else
    /// the localized `landing-subtitle` (#468).
    header_subtitle: String,
    /// Footer text — `landing-customization.footer` override. Empty ⇒ the
    /// built-in version + wordmark lockup renders unchanged.
    footer: String,
    /// `<style>` body setting the theme CSS variables from the operator's
    /// per-theme color overrides (#475). Empty ⇒ no `<style>` emitted.
    theme_style: String,
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
    /// Operator custom CSS (#232), injected as a `<style>` near the end
    /// of `<head>` (rendered with `|safe`). Empty ⇒ nothing injected.
    custom_css: String,
    /// Header/footer logos (admin-managed). Rendered in left/center/right
    /// groups via [`Self::logo_groups`].
    logos: Vec<ruscker_config::LandingLogo>,
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
    /// Whether to render the anonymous "Sign in" entrance (#156). A
    /// deploy policy from `landing-customization.show-admin-link`
    /// (default true); false hides the admin entrance on public portals.
    show_admin_link: bool,
    /// Whether the "Featured" carousel may render (#506). The template also
    /// checks that at least one card is featured via [`Self::has_featured`].
    show_highlights: bool,
}

/// A landing logo resolved for rendering: `src` already carries the mount
/// prefix and `height` has its per-slot default applied. Produced by
/// [`LandingPage::logo_view`] and consumed by the template's `logos` macro
/// so the four chrome inserts (#468) share one rendering path.
struct LogoView {
    src: String,
    link: Option<String>,
    height: u32,
    margin: Option<u32>,
}

impl<'a> LandingPage<'a> {
    /// Translation helper used by the template as `self.t("key")`.
    /// Centralizing here keeps templates clean of explicit
    /// bundle/locale handling.
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// Any logo configured for this `slot`/`align` bucket? Used to decide
    /// whether to render a slot's chrome insert (header-left replaces the
    /// Ruscker mark; header-right sits after the chrome cluster; the
    /// `center` bucket still renders as a separate bar). See #468.
    /// Any featured card? Gates the "Featured" carousel together with
    /// `show_highlights` (#506).
    fn has_featured(&self) -> bool {
        self.cards.iter().any(|c| c.featured)
    }

    fn has_logos_at(&self, slot: &str, align: &str) -> bool {
        self.logos.iter().any(|l| l.slot == slot && l.align == align)
    }

    /// Resolve a logo's `<img src>`: an absolute/protocol-relative URL is
    /// used as-is; a root-absolute path gets the mount prefix (#173).
    fn logo_src(&self, logo: &ruscker_config::LandingLogo) -> String {
        let u = &logo.url;
        if u.starts_with("http://") || u.starts_with("https://") || u.starts_with("//") {
            u.clone()
        } else {
            format!("{}{}", self.base, u)
        }
    }

    /// Render-ready logos for a `slot`/`align` bucket, in insertion order:
    /// `src` is already mount-prefixed and `height` defaults to `default_h`
    /// when the operator left it unset. The template's `logos` macro walks
    /// the returned views (header/footer chrome inserts + the center bar).
    fn logo_view(&self, slot: &str, align: &str, default_h: u32) -> Vec<LogoView> {
        self.logos
            .iter()
            .filter(|l| l.slot == slot && l.align == align)
            .map(|l| LogoView {
                src: self.logo_src(l),
                link: l.link.clone(),
                height: l.height.unwrap_or(default_h),
                margin: l.margin,
            })
            .collect()
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

    // Specs source: the SAME effective catalog the proxy and the
    // lifecycle loops use (#271) — DB unioned with the YAML
    // `proxy.specs`, the DB shadowing on an id collision; YAML-only
    // when no DB. Routing all surfaces through one resolver keeps the
    // landing, the `/app` guard, the scaler/sweeper, and the dashboard
    // from disagreeing on what specs exist (an earlier "DB-only when
    // non-empty" rule here hid an admin-deleted spec from the landing
    // while `find_spec` still resolved it from the YAML and spawned it).
    let owned_specs = crate::catalog::effective_specs(state.db.as_ref(), &state.config).await;
    let mut cards: Vec<CardCtx<'_>> = owned_specs
        .iter()
        .filter(|spec| spec.access_allows(is_admin, username.as_deref(), &groups))
        .map(|spec| CardCtx::from_spec(spec, &state.base_path))
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
    // Social-share image with an auto-default chain: the explicit
    // `og-image` wins; otherwise the operator's header-left brand logo
    // (the one that replaces the Ruscker mark) is reused so a shared link
    // carries the portal's identity without setting it twice; otherwise
    // the built-in Ruscker mark. A root-absolute path carries the base
    // path so it resolves under `--base-path` (#328); a full URL is left
    // untouched. (For best social rendering operators should still upload
    // a ~1200×630 raster — an SVG logo may not render on every platform.)
    let og_raw = not_blank(&lc.og_image)
        .or_else(|| {
            lc.logos
                .iter()
                .find(|l| l.slot == "header" && l.align == "left" && !l.url.trim().is_empty())
                .map(|l| l.url.trim().to_string())
        })
        .unwrap_or_else(|| "/assets/brand/mark.svg".to_string());
    let og_image = if og_raw.starts_with("http://")
        || og_raw.starts_with("https://")
        || og_raw.starts_with("//")
    {
        og_raw
    } else if og_raw.starts_with('/') {
        format!("{}{}", state.base_path, og_raw)
    } else {
        og_raw
    };
    let analytics_html = not_blank(&lc.analytics_html).unwrap_or_default();
    let custom_css = not_blank(&lc.custom_css).unwrap_or_default();

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

    // Header title/subtitle (#468): the editor override wins; otherwise
    // the configured `proxy.title`, then the localized default. This also
    // fixes the header ignoring `proxy.title` (it used to always show the
    // i18n `landing-title`).
    let cfg_title = state.config.proxy.title.trim();
    let header_title = not_blank(&lc.title).unwrap_or_else(|| {
        if cfg_title.is_empty() {
            state.locales.t(loc, "landing-title", None)
        } else {
            cfg_title.to_string()
        }
    });
    // The subtitle is optional (#468): a blank override hides it entirely
    // (the template skips the `<p>` when empty). The localized default is
    // only a placeholder hint in the editor, not a forced fallback.
    let header_subtitle = not_blank(&lc.subtitle).unwrap_or_default();

    // Per-theme color overrides (#475) → a small `<style>` setting the
    // theme CSS variables for light / dark / OS-auto.
    let theme_style = build_theme_style(&lc.theme_colors);

    // Operator default theme for a cookieless visitor: when the request
    // carries no explicit light/dark choice (Auto), honor the configured
    // `default-theme`. The visitor's own toggle still overrides it (it sets
    // the cookie, which makes `theme` non-Auto here).
    let theme = if theme.is_auto() {
        match lc.default_theme.as_deref() {
            Some("light") => Theme::Light,
            Some("dark") => Theme::Dark,
            _ => theme,
        }
    } else {
        theme
    };
    let page = LandingPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        cards,
        type_chips,
        subjects,
        counts,
        intro,
        header_title,
        header_subtitle,
        // Footer override (blank ⇒ the default version+wordmark lockup).
        footer: lc.footer.clone().unwrap_or_default(),
        theme_style,
        header_style,
        page_title,
        seo_description,
        og_image,
        analytics_html,
        custom_css,
        logos: lc.logos.clone(),
        blocks_top,
        blocks_bottom,
        signed_in: session.is_some(),
        viewer_name: username.unwrap_or_default(),
        // Deploy policy from YAML config (not the DB editor): whether
        // anonymous visitors see the "Sign in" entrance (#156).
        show_admin_link: state
            .config
            .proxy
            .landing_customization
            .effective_show_admin_link(),
        // Carousel toggle from the DB-backed editor (#506).
        show_highlights: lc.effective_show_highlights(),
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

/// Accept a CSS color value only if it's plausibly a color and can't
/// break out of a `{ }` declaration block — even though the editor is
/// Admin-only, a stray `}`/`;`/`<` would corrupt the whole `<style>`.
fn sanitize_css_color(s: &Option<String>) -> Option<String> {
    let t = s.as_deref().map(str::trim).filter(|t| !t.is_empty())?;
    let ok = t
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '#' | '(' | ')' | ',' | '%' | '.' | ' ' | '-'));
    ok.then(|| t.to_string())
}

/// Build the `<style>` body that recolors the theme CSS variables from
/// the operator's overrides (#475). Light values land on `:root` (the
/// base, which also covers OS-auto-light); dark values land on both the
/// explicit `[data-theme="dark"]` and the OS-auto-dark media query —
/// mirroring the cascade in `input.css`. Empty when nothing is set.
fn build_theme_style(tc: &ruscker_config::ThemeColors) -> String {
    fn vars(p: &ruscker_config::ThemePalette) -> String {
        let mut s = String::new();
        if let Some(c) = sanitize_css_color(&p.bg) {
            s.push_str(&format!("--bg:{c};"));
        }
        if let Some(c) = sanitize_css_color(&p.text) {
            s.push_str(&format!("--text:{c};"));
        }
        if let Some(c) = sanitize_css_color(&p.accent) {
            s.push_str(&format!("--color-teal-600:{c};"));
        }
        s
    }
    let light = vars(&tc.light);
    let dark = vars(&tc.dark);
    let mut css = String::new();
    if !light.is_empty() {
        css.push_str(&format!(":root{{{light}}}"));
    }
    if !dark.is_empty() {
        css.push_str(&format!(
            "@media (prefers-color-scheme:dark){{:root:not([data-theme=\"light\"]){{{dark}}}}}"
        ));
        css.push_str(&format!(":root[data-theme=\"dark\"]{{{dark}}}"));
    }
    css
}
