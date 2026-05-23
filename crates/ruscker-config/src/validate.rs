//! Non-fatal validation that surfaces warnings to the operator.
//!
//! "Fatal" errors (malformed YAML, missing required fields) are caught
//! during parsing. This module finds things that *parse* fine but
//! should be flagged:
//!
//! - Duplicate spec IDs (only the last would win)
//! - Empty descriptions or display names
//! - Embedded credentials (`docker-registry-password` not using
//!   `${ENV_VAR}` interpolation)
//! - References to unknown `template-properties.type` values
//! - `min-replicas > max-replicas`
//! - Auto-scale thresholds out of range or inverted
//!
//! ## Pre-parse vs post-parse
//!
//! Some checks must run on the raw YAML text BEFORE environment
//! interpolation, because they ask questions about the operator's
//! authoring style (e.g. "is the password in plain text or
//! `${ENV_VAR}` form?"). After interpolation, a credential that came
//! from `${VAR}` and one that was hardcoded look identical.
//!
//! [`scan_raw_text`] runs against raw YAML; [`run`] runs against the
//! parsed model. [`Config::validate`] merges results from both.

use crate::schema::{Config, Spec, SpecKind};
use once_cell::sync::Lazy;
use regex::Regex;
use serde::Serialize;
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize)]
pub struct ValidationReport {
    pub warnings: Vec<Warning>,
    pub stats: Stats,
}

impl ValidationReport {
    pub fn is_clean(&self) -> bool {
        self.warnings.is_empty()
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Stats {
    pub total_specs: usize,
    pub by_kind: HashMap<String, usize>,
    pub by_state: HashMap<String, usize>,
    pub by_access: HashMap<String, usize>,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Warning {
    DuplicateSpecId {
        id: String,
    },
    EmptyDisplayName {
        spec_id: String,
    },
    EmptyDescription {
        spec_id: String,
    },
    EmbeddedCredential {
        field: String,
        line: usize,
    },
    UnknownTypeProperty {
        spec_id: String,
        value: String,
    },
    InvalidReplicaRange {
        spec_id: String,
        min: u32,
        max: u32,
    },
    InvalidScaleThreshold {
        spec_id: String,
        scale_up: f64,
        scale_down: f64,
    },
    SpecLackingContainerHasContainerFields {
        spec_id: String,
    },
}

const KNOWN_TYPES: &[&str] = &["app", "package", "talk", "report", "api"];

/// Sensitive fields that, if present in raw YAML without `${VAR}` form,
/// indicate an embedded credential.
const SENSITIVE_FIELDS: &[&str] = &[
    "docker-registry-password",
    "auth-token",
    "api-key",
    "secret",
];

static SENSITIVE_LINE: Lazy<Regex> = Lazy::new(|| {
    let pattern = format!(
        r"^\s*({})\s*:\s*(.+)$",
        SENSITIVE_FIELDS
            .iter()
            .map(|f| regex::escape(f))
            .collect::<Vec<_>>()
            .join("|")
    );
    Regex::new(&pattern).expect("sensitive-line regex is valid")
});

/// Scan raw YAML text for credential-like fields that don't use the
/// `${VAR}` interpolation form. Runs before parsing, so it works on
/// the operator's literal authoring (commented lines are skipped).
pub fn scan_raw_text(raw: &str) -> Vec<Warning> {
    let mut warnings = Vec::new();
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        if let Some(caps) = SENSITIVE_LINE.captures(line) {
            let field = caps.get(1).unwrap().as_str().to_string();
            let value = caps.get(2).unwrap().as_str().trim();
            let unquoted = value.trim_matches(|c| c == '"' || c == '\'');
            if unquoted.is_empty() || unquoted.starts_with("${") {
                continue;
            }
            warnings.push(Warning::EmbeddedCredential {
                field,
                line: idx + 1,
            });
        }
    }
    warnings
}

pub fn run(config: &Config) -> ValidationReport {
    let mut warnings = Vec::new();
    let mut id_counts: HashMap<&str, usize> = HashMap::new();

    for spec in &config.proxy.specs {
        *id_counts.entry(spec.id.as_str()).or_insert(0) += 1;
        check_spec(spec, &mut warnings);
    }

    for (id, count) in id_counts {
        if count > 1 {
            warnings.push(Warning::DuplicateSpecId { id: id.to_string() });
        }
    }

    let stats = collect_stats(config);

    ValidationReport { warnings, stats }
}

fn check_spec(spec: &Spec, warnings: &mut Vec<Warning>) {
    if spec
        .display_name
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        warnings.push(Warning::EmptyDisplayName {
            spec_id: spec.id.clone(),
        });
    }

    if spec
        .description
        .as_ref()
        .map(|s| s.trim().is_empty())
        .unwrap_or(true)
    {
        warnings.push(Warning::EmptyDescription {
            spec_id: spec.id.clone(),
        });
    }

    if let Some(t) = spec.template_properties.type_field() {
        if !KNOWN_TYPES.contains(&t) {
            warnings.push(Warning::UnknownTypeProperty {
                spec_id: spec.id.clone(),
                value: t.to_string(),
            });
        }
    }

    let min = spec.effective_min_replicas();
    let max = spec.effective_max_replicas();
    if max < min {
        warnings.push(Warning::InvalidReplicaRange {
            spec_id: spec.id.clone(),
            min,
            max,
        });
    }

    let up = spec.scale_up_threshold.map(|f| f.0).unwrap_or(0.8);
    let down = spec.scale_down_threshold.map(|f| f.0).unwrap_or(0.3);
    if (spec.scale_up_threshold.is_some() || spec.scale_down_threshold.is_some())
        && (up <= down || up > 1.0 || down < 0.0)
    {
        warnings.push(Warning::InvalidScaleThreshold {
            spec_id: spec.id.clone(),
            scale_up: up,
            scale_down: down,
        });
    }

    if spec.container_image.is_none() {
        let has_container_fields = spec.seats_per_container.is_some()
            || spec.docker_registry_password.is_some()
            || spec.max_lifetime.is_some();
        if has_container_fields {
            warnings.push(Warning::SpecLackingContainerHasContainerFields {
                spec_id: spec.id.clone(),
            });
        }
    }
}

fn collect_stats(config: &Config) -> Stats {
    let mut by_kind: HashMap<String, usize> = HashMap::new();
    let mut by_state: HashMap<String, usize> = HashMap::new();
    let mut by_access: HashMap<String, usize> = HashMap::new();

    for spec in &config.proxy.specs {
        let kind_label = match spec.kind() {
            SpecKind::Shiny => "shiny",
            SpecKind::InteractiveApp => "interactive",
            SpecKind::Api => "api",
            SpecKind::External => "external",
        };
        *by_kind.entry(kind_label.to_string()).or_insert(0) += 1;

        let state = spec.template_properties.state().to_string();
        *by_state.entry(state).or_insert(0) += 1;

        let access = spec
            .template_properties
            .get_str("icon")
            .unwrap_or("unknown")
            .to_string();
        *by_access.entry(access).or_insert(0) += 1;
    }

    Stats {
        total_specs: config.proxy.specs.len(),
        by_kind,
        by_state,
        by_access,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_scan_flags_plain_credential() {
        let yaml = "docker-registry-password: dckr_pat_abc123\n";
        let warnings = scan_raw_text(yaml);
        assert_eq!(warnings.len(), 1);
        assert!(matches!(
            &warnings[0],
            Warning::EmbeddedCredential { field, .. } if field == "docker-registry-password"
        ));
    }

    #[test]
    fn raw_scan_accepts_env_var_form() {
        let yaml = "docker-registry-password: ${DOCKER_REGISTRY_PASSWORD}\n";
        let warnings = scan_raw_text(yaml);
        assert!(warnings.is_empty());
    }

    #[test]
    fn raw_scan_ignores_commented_lines() {
        let yaml = "# docker-registry-password: dckr_pat_abc123\n";
        let warnings = scan_raw_text(yaml);
        assert!(warnings.is_empty());
    }

    #[test]
    fn raw_scan_reports_line_numbers() {
        let yaml = "line1: foo\nline2: bar\ndocker-registry-password: leaked\n";
        let warnings = scan_raw_text(yaml);
        assert_eq!(warnings.len(), 1);
        match &warnings[0] {
            Warning::EmbeddedCredential { line, .. } => assert_eq!(*line, 3),
            _ => panic!("expected EmbeddedCredential"),
        }
    }
}
