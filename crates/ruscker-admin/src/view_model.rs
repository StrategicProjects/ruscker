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

    /// Short 3-letter abbreviation shown on the card badge —
    /// preserves the mockup's visual rhythm where every tag has the
    /// same width.
    pub fn short_label(self) -> &'static str {
        match self {
            DisplayType::App => "APP",
            DisplayType::Talk => "APT",
            DisplayType::Report => "RLT",
            DisplayType::Package => "PCT",
            DisplayType::Api => "API",
            DisplayType::Link => "LNK",
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
    pub logo: Option<&'a str>,
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
    pub fn from_spec(spec: &'a Spec) -> Self {
        let kind = spec.kind();
        let display_type = DisplayType::from_spec(spec);
        let tp = &spec.template_properties;
        let access_open = tp
            .get_str("icon")
            .map(|s| s == "lock_open")
            .unwrap_or(false);
        let active = tp.is_active();
        let logo = tp.get_str("logo");
        let updated_raw = tp.get_str("updated");
        let updated_date = updated_raw.and_then(parse_dmy);
        let updated_short = updated_date.map(|d| d.format("%d/%m").to_string());
        let status = compute_status(updated_date, Utc::now().date_naive());
        let href = match kind {
            SpecKind::External => tp.get_str("link").unwrap_or("#").to_string(),
            _ => format!("/app/{}/", spec.id),
        };
        Self {
            id: &spec.id,
            display_name: spec.display_name.as_deref().unwrap_or(&spec.id),
            description: strip_html(spec.description.as_deref().unwrap_or("")),
            display_type,
            access_open,
            active,
            logo,
            updated_raw,
            updated_short,
            status,
            updated_date,
            href,
        }
    }
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
    cards.sort_by(|a, b| b.updated_date.cmp(&a.updated_date));
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
