//! View-model: turn a parsed [`Spec`] into something a template can
//! render without re-deriving display details every time.
//!
//! Templates should never call `Spec::kind()` or guess colors — they
//! consume [`CardCtx`].

use ruscker_config::{Spec, SpecKind};
use std::collections::BTreeMap;

/// What the landing card needs. One per spec, built once per render.
#[derive(Debug, Clone)]
pub struct CardCtx<'a> {
    pub id: &'a str,
    pub display_name: &'a str,
    pub description: &'a str,
    pub kind_label: &'static str,
    pub kind_color_class: &'static str,
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
        let (kind_label, color) = kind_visuals(kind);
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
            kind_label,
            kind_color_class: color,
            access_open,
            active,
            logo,
            updated,
            href,
        }
    }
}

/// Tailwind class fragments for each spec kind. Kept in code (not
/// CSS) so adding a new kind requires touching exactly this match —
/// the compiler tells you when you've forgotten one.
fn kind_visuals(kind: SpecKind) -> (&'static str, &'static str) {
    match kind {
        SpecKind::Shiny => ("APP", "kind-app"),
        SpecKind::InteractiveApp => ("APP", "kind-app"),
        SpecKind::Api => ("API", "kind-api"),
        SpecKind::External => ("LINK", "kind-link"),
    }
}

/// One row of the filter-chip bar at the top of the landing.
#[derive(Debug, Clone)]
pub struct TypeChip {
    /// Stable filter key matched against [`CardCtx::kind_color_class`].
    pub key: &'static str,
    /// Fluent translation key for the chip label.
    pub label_key: &'static str,
    /// Tailwind class fragment for the chip's color.
    pub css_class: &'static str,
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
pub fn build_type_chips(cards: &[CardCtx<'_>]) -> Vec<TypeChip> {
    let mut counts: BTreeMap<&'static str, usize> = BTreeMap::new();
    for c in cards {
        *counts.entry(c.kind_color_class).or_default() += 1;
    }
    [
        ("kind-app", "type-app", "kind-app"),
        ("kind-api", "type-api", "kind-api"),
        ("kind-link", "type-package", "kind-link"),
    ]
    .into_iter()
    .filter_map(|(key, label_key, css_class)| {
        let count = *counts.get(key).unwrap_or(&0);
        if count == 0 {
            None
        } else {
            Some(TypeChip {
                key,
                label_key,
                css_class,
                count,
            })
        }
    })
    .collect()
}
