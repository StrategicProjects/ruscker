//! View-model: turn a parsed [`Spec`] into something a template can
//! render without re-deriving display details every time.
//!
//! Templates should never call `Spec::kind()` or guess colors — they
//! consume [`CardCtx`].
//!
//! ## Why two type concepts
//!
//! [`SpecKind`] (from `ruscker-config`) describes *how to run* the
//! spec — Shiny, API, External — and drives sticky-session and
//! routing decisions in the proxy.
//!
//! [`DisplayType`] describes *how to badge it on the landing card*
//! — `app`/`talk`/`report`/`package`/`api`/`link`. It is read from
//! `template-properties.type` in the YAML and is purely visual.
//!
//! These are orthogonal because the SEPE portal uses (e.g.) `talk`
//! and `report` to distinguish executive presentations from
//! consolidated reports — both are technically Shiny containers but
//! the landing should still tell them apart.

use chrono::{Duration, NaiveDate, Utc};
use regex::Regex;
use ruscker_config::{Spec, SpecKind};
use std::collections::BTreeMap;
use std::sync::LazyLock;

/// Visual badge category for landing cards. Read from the
/// `template-properties.type` field. Fallbacks computed via
/// [`DisplayType::from_spec`].
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum DisplayType {
    /// `app` — interactive Shiny/Streamlit/Dash dashboard.
    App,
    /// `talk` — executive presentation rendered as a Shiny app.
    Talk,
    /// `report` — consolidated written report.
    Report,
    /// `package` — an R/Python package, usually external link to docs.
    Package,
    /// `api` — REST/Plumber/FastAPI endpoint.
    Api,
    /// Fallback when neither `template-properties.type` is set nor
    /// can it be inferred from `SpecKind`.
    Link,
}

impl DisplayType {
    pub fn from_spec(spec: &Spec) -> Self {
        // 1. Explicit template-properties.type wins.
        if let Some(t) = spec.template_properties.type_field() {
            if let Some(dt) = DisplayType::parse(t) {
                return dt;
            }
        }
        // 2. Otherwise infer from SpecKind.
        match spec.kind() {
            SpecKind::Api => DisplayType::Api,
            SpecKind::External => DisplayType::Link,
            SpecKind::Shiny | SpecKind::InteractiveApp => DisplayType::App,
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "app" => DisplayType::App,
            "talk" => DisplayType::Talk,
            "report" => DisplayType::Report,
            "package" => DisplayType::Package,
            "api" => DisplayType::Api,
            "link" => DisplayType::Link,
            _ => return None,
        })
    }

    /// Stable kebab-case key used for `data-type=` filtering.
    pub fn key(self) -> &'static str {
        match self {
            DisplayType::App => "app",
            DisplayType::Talk => "talk",
            DisplayType::Report => "report",
            DisplayType::Package => "package",
            DisplayType::Api => "api",
            DisplayType::Link => "link",
        }
    }

    /// Fallback 3-letter abbreviation, used only when no Fluent
    /// translation key is available (e.g. doc generation). The UI
    /// always renders the localized form via [`abbr_key`].
    pub fn short_label(self) -> &'static str {
        match self {
            DisplayType::App => "APP",
            DisplayType::Talk => "APR",
            DisplayType::Report => "RLT",
            DisplayType::Package => "PCT",
            DisplayType::Api => "API",
            DisplayType::Link => "LNK",
        }
    }

    /// Fluent key for the localized 3-letter abbreviation rendered
    /// inside the card badge. Translations live in
    /// `assets/i18n/<lang>/landing.ftl` as `type-<kind>-abbr`.
    pub fn abbr_key(self) -> &'static str {
        match self {
            DisplayType::App => "type-app-abbr",
            DisplayType::Talk => "type-talk-abbr",
            DisplayType::Report => "type-report-abbr",
            DisplayType::Package => "type-package-abbr",
            DisplayType::Api => "type-api-abbr",
            DisplayType::Link => "type-link-abbr",
        }
    }

    /// Fluent key for the long-form chip label (Applications,
    /// Apresentações, etc.).
    pub fn label_key(self) -> &'static str {
        match self {
            DisplayType::App => "type-app",
            DisplayType::Talk => "type-talk",
            DisplayType::Report => "type-report",
            DisplayType::Package => "type-package",
            DisplayType::Api => "type-api",
            DisplayType::Link => "type-package",
        }
    }

    /// CSS class fragment mapped to design tokens in input.css.
    pub fn css_class(self) -> &'static str {
        match self {
            DisplayType::App => "kind-app",
            DisplayType::Talk => "kind-talk",
            DisplayType::Report => "kind-report",
            DisplayType::Package => "kind-package",
            DisplayType::Api => "kind-api",
            DisplayType::Link => "kind-package",
        }
    }
}

/// Coarse status indicator rendered as a small dot next to the
/// updated date. Computed from `template-properties.updated`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum StatusKind {
    /// Updated within the last [`NEW_THRESHOLD_DAYS`] days. Blue dot.
    New,
    /// Has an updated date, older than "new". Green dot.
    Updated,
    /// No date or unparseable. No dot rendered.
    Unknown,
}

impl StatusKind {
    /// Fluent key for the meta-row text. Returns `None` when the
    /// status is [`Unknown`] (template skips the meta row entirely).
    pub fn label_key(self) -> Option<&'static str> {
        match self {
            StatusKind::New => Some("status-new"),
            StatusKind::Updated => Some("status-updated"),
            StatusKind::Unknown => None,
        }
    }

    /// CSS class fragment for the dot. Tokens defined in input.css.
    pub fn dot_class(self) -> &'static str {
        match self {
            StatusKind::New => "status-dot-new",
            StatusKind::Updated => "status-dot-updated",
            StatusKind::Unknown => "status-dot-unknown",
        }
    }

    /// Fluent key for the dot's `title` tooltip — unlike [`label_key`]
    /// this covers all three states (including `Unknown`) so the
    /// recency dot + the "—" date placeholder are self-explanatory on
    /// hover, not a mystery symbol (#349). Self-contained (no `$date`).
    pub fn title_key(self) -> &'static str {
        match self {
            StatusKind::New => "status-title-new",
            StatusKind::Updated => "status-title-updated",
            StatusKind::Unknown => "status-title-none",
        }
    }
}

/// Threshold below which an updated card is labelled "new" instead
/// of "updated". 60 days is a soft choice; long enough to survive a
/// quarter, short enough that monthly batch releases still get the
/// blue dot.
pub const NEW_THRESHOLD_DAYS: i64 = 60;

/// What the landing card needs. One per spec, built once per render.
///
/// `description` is **always plain text**, with any HTML in the
/// source YAML stripped. The card itself is an `<a>` element and
/// nested anchors are illegal in HTML — a `<a href>` inside the
/// description would force the browser to auto-close the outer
/// card-link and fragment the DOM (visible bug: cards appear "torn
/// in half"). See [`strip_html`].
#[derive(Debug, Clone)]
pub struct CardCtx<'a> {
    pub id: &'a str,
    pub display_name: &'a str,
    pub description: String,
    pub display_type: DisplayType,
    pub access_open: bool,
    pub active: bool,
    /// Sector/theme classification (e.g. "Saúde Pública"). Read
    /// verbatim from `template-properties.subject` and rendered as-is
    /// in the UI — operators choose their own taxonomy.
    pub subject: Option<&'a str>,
    /// Highlighted in the landing's "Featured" carousel (#506).
    pub featured: bool,
    /// Card logo URL. Stored base-agnostic; a root-absolute value
    /// (e.g. `/assets/img/x.png`) is prefixed with the portal base
    /// path at construction (#294) so it resolves under `--base-path`.
    pub logo: Option<String>,
    /// Optional CSS background for the card cover, read verbatim
    /// from `template-properties.cover` (a solid color or a
    /// `linear-/radial-gradient(...)`). When `None`, the card
    /// falls back to the per-kind tint class. Interpolated raw
    /// into a `style="background: …"` — the browser fail-softs on
    /// invalid CSS, and the value round-trips through YAML/export
    /// untouched. (Not server-validated, same policy as
    /// `landing_customization.header_bg`.) Any embedded
    /// `url(/assets/…)` is base-prefixed at construction (#294).
    pub cover: Option<String>,
    pub updated_raw: Option<&'a str>,
    /// "DD/MM" — short form used in the meta row.
    pub updated_short: Option<String>,
    pub status: StatusKind,
    /// Parsed updated date (for sorting). `None` sorts last.
    pub updated_date: Option<NaiveDate>,
    /// Target href: external link for `External`, `/app/<id>/` for
    /// containerized specs (still 404 in Phase 1 — no proxy yet).
    pub href: String,
}

impl<'a> CardCtx<'a> {
    /// Build a card view-model. `base` is the portal base path (`""`
    /// or e.g. `/box`); root-absolute asset URLs in `href`/`logo`/
    /// `cover` are prefixed with it so the landing renders correct
    /// links under `--base-path` (the per-request HTML body rewriter
    /// was removed in #294, so prefixing happens at construction now).
    pub fn from_spec(spec: &'a Spec, base: &str) -> Self {
        let kind = spec.kind();
        let display_type = DisplayType::from_spec(spec);
        let tp = &spec.template_properties;
        // The card's access lock reflects the *real* access rule
        // (`Spec::is_open()` — no `access-groups`/`access-users`), not a
        // decorative `template-properties.icon` flag. This keeps the
        // landing badge + the `data-access` filter consistent with the
        // `/app` + `/api` enforcement guard, which also keys off
        // `is_open()` (#346).
        let access_open = spec.is_open();
        let active = tp.is_active();
        let subject = tp.get_str("subject");
        let logo = tp.get_str("logo").map(|l| prefix_asset_url(l, base));
        // Empty string ⇒ treat as unset so the kind-tint fallback
        // kicks in rather than rendering `background: ;`.
        let cover = tp
            .get_str("cover")
            .filter(|s| !s.trim().is_empty())
            .map(|c| prefix_css_asset_urls(c, base));
        let updated_raw = tp.get_str("updated");
        let updated_date = updated_raw.and_then(parse_dmy);
        let updated_short = updated_date.map(|d| d.format("%d/%m").to_string());
        let status = compute_status(updated_date, Utc::now().date_naive());
        let href = match kind {
            // External links are operator-authored absolute URLs — never
            // base-prefixed. Containerized specs route under the portal,
            // so their `/app/<id>/` path must carry the base path (#294).
            SpecKind::External => tp.get_str("link").unwrap_or("#").to_string(),
            _ => format!("{base}/app/{}/", spec.id),
        };
        Self {
            id: &spec.id,
            display_name: spec.display_name.as_deref().unwrap_or(&spec.id),
            description: strip_html(spec.description.as_deref().unwrap_or("")),
            display_type,
            access_open,
            active,
            subject,
            featured: spec.is_featured(),
            logo,
            cover,
            updated_raw,
            updated_short,
            status,
            updated_date,
            href,
        }
    }
}

/// Prefix the portal `base` path onto a root-absolute asset URL
/// (`/assets/img/x` → `/box/assets/img/x`). A full URL (`https://…`),
/// an anchor, or any non-root-absolute value is returned unchanged.
/// Mirrors the client-side `assetUrl` helper used by the admin pickers
/// — the stored value stays base-agnostic; only the rendered URL gets
/// the prefix (#294/#270).
fn prefix_asset_url(value: &str, base: &str) -> String {
    if base.is_empty() || !value.starts_with('/') {
        value.to_string()
    } else {
        format!("{base}{value}")
    }
}

/// Prefix the portal `base` path onto root-absolute `url(/assets/…)`
/// targets embedded in a CSS value (the card `cover` may be
/// `url('/assets/img/x.png')`). Solid colors / gradients pass through
/// untouched. Mirrors the client-side `coverStyle` helper exactly —
/// only `/assets/` URLs are rewritten, so a protocol-relative
/// `url(//cdn/…)` is left alone (#294/#270).
fn prefix_css_asset_urls(value: &str, base: &str) -> String {
    if base.is_empty() {
        return value.to_string();
    }
    value
        .replace("url(/assets/", &format!("url({base}/assets/"))
        .replace("url('/assets/", &format!("url('{base}/assets/"))
        .replace("url(\"/assets/", &format!("url(\"{base}/assets/"))
}

/// Parse the operator-authored `DD/MM/YYYY` form used in the SEPE
/// YAML. Tolerates leading/trailing whitespace. Returns `None` for
/// any unrecognized format.
fn parse_dmy(s: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(s.trim(), "%d/%m/%Y").ok()
}

/// Strip every HTML tag and decode the four entities that matter
/// for plain-text display. Run on operator-authored YAML
/// descriptions so they render cleanly inside the card link
/// without nested-anchor bugs or stray formatting.
///
/// Not a security boundary — Askama still auto-escapes the output
/// at template render time. The purpose is *cosmetic*: descriptions
/// in the SEPE YAML occasionally include `<a href="...">` tags,
/// which break the outer `<a class="rcard">` if rendered with
/// `|safe`. Stripping yields readable two-line summaries.
fn strip_html(s: &str) -> String {
    static TAG_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"<[^>]*>").unwrap());
    static WS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s+").unwrap());
    // Replace tags with a single space so `a<br/>b` becomes `a b`,
    // not `ab`. WS_RE collapses runs of whitespace below.
    let no_tags = TAG_RE.replace_all(s, " ");
    let decoded = no_tags
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ");
    WS_RE.replace_all(decoded.trim(), " ").to_string()
}

/// Classify how recently the card was updated. Pure function so it
/// can be unit-tested with a fixed `today`.
fn compute_status(updated: Option<NaiveDate>, today: NaiveDate) -> StatusKind {
    match updated {
        None => StatusKind::Unknown,
        Some(d) => {
            let age = today.signed_duration_since(d);
            if age <= Duration::days(NEW_THRESHOLD_DAYS) {
                StatusKind::New
            } else {
                StatusKind::Updated
            }
        }
    }
}

/// One row of the filter-chip bar at the top of the landing.
#[derive(Debug, Clone)]
pub struct TypeChip {
    pub display_type: DisplayType,
    /// How many cards belong to this type.
    pub count: usize,
}

/// Counts to display alongside filter chips.
#[derive(Debug, Clone, Default)]
pub struct CardCounts {
    pub total: usize,
}

/// Sort cards by recency (newest first). Cards without a parseable
/// `updated` date sink to the end, preserving their relative
/// declaration order. This matches the "Recentes" sort the mockup
/// shows as the default option.
pub fn sort_by_recent(cards: &mut [CardCtx<'_>]) {
    cards.sort_by_key(|c| std::cmp::Reverse(c.updated_date));
}

/// Unique `subject` values across all cards, alphabetically sorted.
/// Powers the "Filtrar por subject" `<select>` on the landing. Cards
/// with no subject contribute nothing — only the "Todos os subjects"
/// option (always present) matches them.
pub fn unique_subjects<'a>(cards: &[CardCtx<'a>]) -> Vec<&'a str> {
    let mut set: std::collections::BTreeSet<&str> = std::collections::BTreeSet::new();
    for c in cards {
        if let Some(t) = c.subject {
            set.insert(t);
        }
    }
    set.into_iter().collect()
}

/// Build the chip bar with live counts. Only chips that have at
/// least one matching card are emitted, so the UI never shows
/// "Reports (0)" — that signal is more useful than the chip itself.
/// Order matches the mockup's visual order: app, talk, report,
/// package, api, link.
pub fn build_type_chips(cards: &[CardCtx<'_>]) -> Vec<TypeChip> {
    let mut counts: BTreeMap<DisplayType, usize> = BTreeMap::new();
    for c in cards {
        *counts.entry(c.display_type).or_default() += 1;
    }
    [
        DisplayType::App,
        DisplayType::Talk,
        DisplayType::Report,
        DisplayType::Package,
        DisplayType::Api,
        DisplayType::Link,
    ]
    .into_iter()
    .filter_map(|dt| {
        let count = *counts.get(&dt).unwrap_or(&0);
        if count == 0 {
            None
        } else {
            Some(TypeChip {
                display_type: dt,
                count,
            })
        }
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_brazilian_dates() {
        assert_eq!(
            parse_dmy("18/05/2026"),
            Some(NaiveDate::from_ymd_opt(2026, 5, 18).unwrap())
        );
        assert_eq!(parse_dmy("  01/01/2025  "), parse_dmy("01/01/2025"));
        assert_eq!(parse_dmy("nope"), None);
        assert_eq!(parse_dmy(""), None);
        assert_eq!(parse_dmy("2026-05-18"), None, "ISO form is not the operator format");
    }

    #[test]
    fn strip_html_keeps_text_only() {
        assert_eq!(
            strip_html("Painel <a href='x' style='color:red'>SEGPR</a>"),
            "Painel SEGPR"
        );
        // Self-closing and entities
        assert_eq!(strip_html("a<br/>b &amp; c"), "a b & c");
        // Multiple whitespace collapses
        assert_eq!(strip_html("  hello   world  "), "hello world");
        // Plain text untouched
        assert_eq!(strip_html("just text"), "just text");
    }

    #[test]
    fn status_buckets() {
        let today = NaiveDate::from_ymd_opt(2026, 5, 23).unwrap();
        // within 60 days → New
        assert_eq!(
            compute_status(NaiveDate::from_ymd_opt(2026, 5, 1), today),
            StatusKind::New
        );
        // older than 60 days → Updated
        assert_eq!(
            compute_status(NaiveDate::from_ymd_opt(2025, 1, 1), today),
            StatusKind::Updated
        );
        // missing → Unknown
        assert_eq!(compute_status(None, today), StatusKind::Unknown);
    }

    fn spec_with_tp(yaml_tp: &str) -> ruscker_config::Spec {
        let yaml = format!(
            "id: x\ndisplay-name: X\ncontainer-image: a:1\ntemplate-properties:\n{yaml_tp}"
        );
        serde_yaml_ng::from_str(&yaml).expect("parse spec")
    }

    #[test]
    fn card_cover_read_from_template_properties() {
        let spec = spec_with_tp("  cover: \"linear-gradient(135deg, #0f6e56, #5dcaa5)\"\n");
        let card = CardCtx::from_spec(&spec, "");
        assert_eq!(
            card.cover.as_deref(),
            Some("linear-gradient(135deg, #0f6e56, #5dcaa5)")
        );
    }

    #[test]
    fn card_cover_absent_falls_back_to_tint() {
        let spec = spec_with_tp("  subject: Saúde\n");
        let card = CardCtx::from_spec(&spec, "");
        assert_eq!(card.cover, None);
    }

    #[test]
    fn card_cover_empty_string_is_none() {
        // Empty cover must not render `background: ;` — it should
        // fall through to the kind tint.
        let spec = spec_with_tp("  cover: \"  \"\n");
        let card = CardCtx::from_spec(&spec, "");
        assert_eq!(card.cover, None);
    }

    #[test]
    fn card_lock_reflects_real_access_not_icon() {
        // #346: the card access lock is derived from `Spec::is_open()`
        // (`access-groups` / `access-users`), NOT the retired decorative
        // `template-properties.icon` flag.

        // No access lists ⇒ open, even with a stale `icon: lock` lingering
        // from an older DB.
        let open = spec_with_tp("  icon: lock\n");
        assert!(
            CardCtx::from_spec(&open, "").access_open,
            "no access lists ⇒ open (the `icon` flag is ignored)"
        );

        // An access list ⇒ restricted, even if `icon: lock_open` says
        // otherwise — the visual can no longer contradict enforcement.
        let restricted: ruscker_config::Spec = serde_yaml_ng::from_str(
            "id: x\ndisplay-name: X\ncontainer-image: a:1\n\
             access-groups: [staff]\ntemplate-properties:\n  icon: lock_open\n",
        )
        .expect("parse spec");
        assert!(
            !CardCtx::from_spec(&restricted, "").access_open,
            "access-groups present ⇒ restricted"
        );
    }

    #[test]
    fn card_urls_prefixed_under_base_path() {
        // #294: with the per-request HTML body rewriter gone, root-
        // absolute URLs must be base-prefixed at construction. The href
        // of a containerized spec, a root-absolute logo, and a
        // `url(/assets/…)` cover all carry `/box`; external links and
        // gradients are left untouched.
        let spec = spec_with_tp(
            "  logo: \"/assets/img/logo.png\"\n  cover: \"url('/assets/img/c.png')\"\n",
        );
        let card = CardCtx::from_spec(&spec, "/box");
        assert_eq!(card.href, "/box/app/x/");
        assert_eq!(card.logo.as_deref(), Some("/box/assets/img/logo.png"));
        assert_eq!(card.cover.as_deref(), Some("url('/box/assets/img/c.png')"));

        // Root deploy (no base) leaves everything bare.
        let card0 = CardCtx::from_spec(&spec, "");
        assert_eq!(card0.href, "/app/x/");
        assert_eq!(card0.logo.as_deref(), Some("/assets/img/logo.png"));
    }

    #[test]
    fn prefix_asset_url_only_touches_root_absolute() {
        assert_eq!(prefix_asset_url("/assets/x.png", "/box"), "/box/assets/x.png");
        // Full URLs and anchors pass through untouched.
        assert_eq!(
            prefix_asset_url("https://cdn/x.png", "/box"),
            "https://cdn/x.png"
        );
        assert_eq!(prefix_asset_url("#", "/box"), "#");
        // Empty base is a no-op (root deploy).
        assert_eq!(prefix_asset_url("/assets/x.png", ""), "/assets/x.png");
    }

    #[test]
    fn prefix_css_asset_urls_handles_quote_variants_and_leaves_others() {
        assert_eq!(
            prefix_css_asset_urls("url(/assets/a.png)", "/box"),
            "url(/box/assets/a.png)"
        );
        assert_eq!(
            prefix_css_asset_urls("url('/assets/a.png')", "/box"),
            "url('/box/assets/a.png')"
        );
        assert_eq!(
            prefix_css_asset_urls("url(\"/assets/a.png\")", "/box"),
            "url(\"/box/assets/a.png\")"
        );
        // Gradients and protocol-relative CDNs are left alone.
        assert_eq!(
            prefix_css_asset_urls("linear-gradient(#000, #fff)", "/box"),
            "linear-gradient(#000, #fff)"
        );
        assert_eq!(
            prefix_css_asset_urls("url(//cdn/a.png)", "/box"),
            "url(//cdn/a.png)"
        );
    }
}
