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

use ruscker_config::{Spec, SpecKind};
use std::collections::BTreeMap;

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

/// What the landing card needs. One per spec, built once per render.
#[derive(Debug, Clone)]
pub struct CardCtx<'a> {
    pub id: &'a str,
    pub display_name: &'a str,
    pub description: &'a str,
    pub display_type: DisplayType,
    pub access_open: bool,
    pub active: bool,
    pub logo: Option<&'a str>,
    pub updated: Option<&'a str>,
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
        let updated = tp.get_str("updated");
        let href = match kind {
            SpecKind::External => tp.get_str("link").unwrap_or("#").to_string(),
            _ => format!("/app/{}/", spec.id),
        };
        Self {
            id: &spec.id,
            display_name: spec.display_name.as_deref().unwrap_or(&spec.id),
            description: spec.description.as_deref().unwrap_or(""),
            display_type,
            access_open,
            active,
            logo,
            updated,
            href,
        }
    }

    /// Fluent key for the CTA button. Per-type so the label matches
    /// what the card actually opens (a relatório, apresentação,
    /// documentação, etc.).
    pub fn cta_key(&self) -> &'static str {
        match self.display_type {
            DisplayType::App => "card-cta-open-app",
            DisplayType::Talk => "card-cta-open-talk",
            DisplayType::Report => "card-cta-open-report",
            DisplayType::Package => "card-cta-open-package",
            DisplayType::Api => "card-cta-open-api",
            DisplayType::Link => "card-cta-link",
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
