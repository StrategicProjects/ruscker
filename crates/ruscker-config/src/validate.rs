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

use crate::schema::{AuthScheme, Config, Spec, SpecKind};
use std::sync::LazyLock;
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
    /// `api.rate-limit` is set but doesn't parse as `N/unit`
    /// (e.g. `100/min`). The proxy ignores an unparseable limit
    /// — applying no limit at all — so the operator should know
    /// their intended cap isn't in effect.
    InvalidRateLimit {
        spec_id: String,
        value: String,
    },
    /// A `max-body-size` value (global `proxy.max-body-size` or a
    /// per-spec override) doesn't parse as a Docker-style size
    /// (`"10m"`, `"1g"`, plain bytes). The proxy falls back to "no
    /// limit there", so the intended cap isn't enforced. `location`
    /// is `"proxy"` for the global, or the spec id.
    InvalidMaxBodySize {
        location: String,
        value: String,
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

static SENSITIVE_LINE: LazyLock<Regex> = LazyLock::new(|| {
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

/// A ShinyProxy feature that a migrated config uses but Ruscker
/// does **not** honour. Surfaced only by `ruscker validate
/// --strict-compat` — it's an opt-in migration aid, separate from
/// the always-on [`Warning`] checks, so a normal `validate` run
/// stays quiet about features that simply aren't built yet.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum CompatWarning {
    /// `proxy.authentication` is set to a scheme other than `none`.
    /// Ruscker's MVP only does `none` (auth happens inside apps).
    UnsupportedAuth { scheme: String },
    /// A per-spec ShinyProxy field that parses but is ignored, or
    /// isn't modelled at all. `note` tells the operator what to do.
    UnsupportedSpecField {
        spec_id: String,
        field: String,
        note: String,
    },
    /// A top-level `proxy.*` field that's unsupported (e.g.
    /// `proxy.docker`).
    UnsupportedProxyField { field: String, note: String },
}

/// Per-spec ShinyProxy keys Ruscker accepts-but-ignores or doesn't
/// model yet, paired with operator-facing guidance. These are
/// invisible in the parsed model (serde drops unknown keys), so the
/// scan walks the raw YAML tree.
const UNSUPPORTED_SPEC_FIELDS: &[(&str, &str)] = &[
    (
        "port",
        "explicit upstream port is ignored — Ruscker auto-detects (use `api.port` for APIs)",
    ),
    (
        "minimum-seats-available",
        "pre-warm pool not implemented — use `min-replicas`",
    ),
    ("labels", "custom container labels are not applied (phase 3.5)"),
    (
        "network-connections",
        "container network wiring is not implemented (phase 3.5)",
    ),
    ("volumes", "volume mounts are not implemented (phase 3)"),
    (
        "environment",
        "per-spec environment injection is not implemented (phase 3)",
    ),
];

/// Top-level `proxy.*` keys that are unsupported.
const UNSUPPORTED_PROXY_FIELDS: &[(&str, &str)] = &[(
    "docker",
    "global docker config is ignored — rely on daemon defaults or env vars",
)];

/// Scan a config for ShinyProxy features Ruscker doesn't honour.
///
/// Two sources, because the unsupported surface splits cleanly:
/// - `proxy.authentication` is a typed field, so it's read from the
///   parsed `config` (post env-interpolation).
/// - Everything else (`kubernetes-*`, `port`, `volumes`, …) is
///   dropped by serde at parse time, so it's only visible by walking
///   the raw YAML tree. Keys survive interpolation untouched, so the
///   raw text is parsed as-is (no env vars required).
///
/// Returns an empty vec for a fully-supported config.
pub fn compat_scan(config: &Config, raw: &str) -> Vec<CompatWarning> {
    let mut out = Vec::new();

    if config.proxy.authentication != AuthScheme::None {
        out.push(CompatWarning::UnsupportedAuth {
            scheme: format!("{:?}", config.proxy.authentication).to_lowercase(),
        });
    }

    // Unparseable raw is the parser's problem, not ours — bail with
    // whatever we already found.
    let Ok(root) = serde_yaml_ng::from_str::<serde_yaml_ng::Value>(raw) else {
        return out;
    };
    let proxy = &root["proxy"];

    for (field, note) in UNSUPPORTED_PROXY_FIELDS {
        if proxy.get(*field).is_some_and(|v| !v.is_null()) {
            out.push(CompatWarning::UnsupportedProxyField {
                field: (*field).to_string(),
                note: (*note).to_string(),
            });
        }
    }

    if let Some(specs) = proxy.get("specs").and_then(|v| v.as_sequence()) {
        for spec in specs {
            let spec_id = spec
                .get("id")
                .and_then(|v| v.as_str())
                .unwrap_or("<unknown>")
                .to_string();

            // Any `kubernetes-*` key signals a Kubernetes backend
            // config (phase 6).
            if let Some(map) = spec.as_mapping() {
                for key in map.keys().filter_map(|k| k.as_str()) {
                    if key.starts_with("kubernetes-") {
                        out.push(CompatWarning::UnsupportedSpecField {
                            spec_id: spec_id.clone(),
                            field: key.to_string(),
                            note: "Kubernetes backend is not implemented (phase 6)".to_string(),
                        });
                    }
                }
            }

            for (field, note) in UNSUPPORTED_SPEC_FIELDS {
                if spec.get(*field).is_some_and(|v| !v.is_null()) {
                    out.push(CompatWarning::UnsupportedSpecField {
                        spec_id: spec_id.clone(),
                        field: (*field).to_string(),
                        note: (*note).to_string(),
                    });
                }
            }
        }
    }

    out
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

    // Global max-body-size: set but unparseable means the intended
    // cap silently does nothing.
    if config.proxy.max_body_size.is_some() && config.proxy.max_body_bytes().is_none() {
        warnings.push(Warning::InvalidMaxBodySize {
            location: "proxy".to_string(),
            value: config.proxy.max_body_size.clone().unwrap_or_default(),
        });
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

    // A rate-limit string that's present but unparseable means the
    // operator intended a cap that won't actually be enforced. Flag
    // it so the typo doesn't silently leave the API wide open.
    if let Some(api) = &spec.api {
        if let Some(raw) = &api.rate_limit {
            if crate::schema::parse_rate_limit(raw).is_none() {
                warnings.push(Warning::InvalidRateLimit {
                    spec_id: spec.id.clone(),
                    value: raw.clone(),
                });
            }
        }
    }

    // Per-spec max-body-size override that's set but unparseable —
    // same hazard: the cap silently does nothing. Parsing with a
    // `None` global isolates the spec's own value.
    if let Some(raw) = &spec.max_body_size {
        if spec.effective_max_body_bytes(None).is_none() {
            warnings.push(Warning::InvalidMaxBodySize {
                location: spec.id.clone(),
                value: raw.clone(),
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

    #[test]
    fn flags_unparseable_api_rate_limit() {
        let yaml = r#"
proxy:
  specs:
    - id: api1
      container-image: org/api:1
      type: api
      api:
        rate-limit: "lots/fortnight"
"#;
        let report = Config::from_yaml(yaml).expect("parse").validate();
        assert!(
            report.warnings.iter().any(|w| matches!(
                w,
                Warning::InvalidRateLimit { spec_id, value }
                    if spec_id == "api1" && value == "lots/fortnight"
            )),
            "expected InvalidRateLimit, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn accepts_valid_api_rate_limit() {
        let yaml = r#"
proxy:
  specs:
    - id: api1
      container-image: org/api:1
      type: api
      api:
        rate-limit: "100/min"
"#;
        let report = Config::from_yaml(yaml).expect("parse").validate();
        assert!(
            !report
                .warnings
                .iter()
                .any(|w| matches!(w, Warning::InvalidRateLimit { .. })),
            "valid rate-limit should not warn, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn flags_malformed_global_max_body_size() {
        let yaml = "proxy:\n  max-body-size: huge\n  specs: []\n";
        let report = Config::from_yaml(yaml).expect("parse").validate();
        assert!(
            report.warnings.iter().any(|w| matches!(
                w,
                Warning::InvalidMaxBodySize { location, value }
                    if location == "proxy" && value == "huge"
            )),
            "expected proxy InvalidMaxBodySize, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn flags_malformed_spec_max_body_size() {
        let yaml = r#"
proxy:
  specs:
    - id: api1
      container-image: org/api:1
      max-body-size: "500frogs"
"#;
        let report = Config::from_yaml(yaml).expect("parse").validate();
        assert!(
            report.warnings.iter().any(|w| matches!(
                w,
                Warning::InvalidMaxBodySize { location, value }
                    if location == "api1" && value == "500frogs"
            )),
            "expected spec InvalidMaxBodySize, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn accepts_valid_max_body_sizes() {
        let yaml = r#"
proxy:
  max-body-size: 50m
  specs:
    - id: api1
      container-image: org/api:1
      max-body-size: "10m"
"#;
        let report = Config::from_yaml(yaml).expect("parse").validate();
        assert!(
            !report
                .warnings
                .iter()
                .any(|w| matches!(w, Warning::InvalidMaxBodySize { .. })),
            "valid sizes should not warn, got {:?}",
            report.warnings
        );
    }

    fn compat(yaml: &str) -> Vec<CompatWarning> {
        let config = Config::from_yaml(yaml).expect("parse config");
        compat_scan(&config, yaml)
    }

    #[test]
    fn compat_scan_clean_config_is_empty() {
        let yaml = "\
proxy:
  authentication: none
  specs:
  - id: app1
    container-image: rocker/shiny
";
        assert!(compat(yaml).is_empty());
    }

    #[test]
    fn compat_scan_flags_unsupported_auth() {
        let yaml = "\
proxy:
  authentication: ldap
  specs: []
";
        let issues = compat(yaml);
        assert_eq!(issues.len(), 1);
        assert!(matches!(
            &issues[0],
            CompatWarning::UnsupportedAuth { scheme } if scheme == "ldap"
        ));
    }

    #[test]
    fn compat_scan_flags_dropped_spec_fields() {
        let yaml = "\
proxy:
  specs:
  - id: legacy
    container-image: rocker/shiny
    port: 3838
    volumes:
    - /data:/data
    kubernetes-pod-patches: '[]'
";
        let issues = compat(yaml);
        let fields: Vec<&str> = issues
            .iter()
            .filter_map(|w| match w {
                CompatWarning::UnsupportedSpecField { spec_id, field, .. } => {
                    assert_eq!(spec_id, "legacy");
                    Some(field.as_str())
                }
                _ => None,
            })
            .collect();
        assert!(fields.contains(&"port"), "fields: {fields:?}");
        assert!(fields.contains(&"volumes"), "fields: {fields:?}");
        assert!(
            fields.contains(&"kubernetes-pod-patches"),
            "fields: {fields:?}"
        );
    }

    #[test]
    fn compat_scan_does_not_flag_api_port() {
        // `api.port` is a supported field — only a *spec-level* `port`
        // is unsupported. The tree walk must not confuse the two.
        let yaml = "\
proxy:
  specs:
  - id: myapi
    container-image: my/api
    type: api
    api:
      port: 8000
";
        assert!(compat(yaml).is_empty(), "{:?}", compat(yaml));
    }

    #[test]
    fn compat_scan_flags_proxy_docker() {
        let yaml = "\
proxy:
  docker:
    url: tcp://localhost:2375
  specs: []
";
        let issues = compat(yaml);
        assert_eq!(issues.len(), 1);
        assert!(matches!(
            &issues[0],
            CompatWarning::UnsupportedProxyField { field, .. } if field == "docker"
        ));
    }
}
