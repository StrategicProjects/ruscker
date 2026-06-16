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
    /// `container-cpu-limit` / `container-cpu-request` is set but isn't
    /// a positive, finite number of CPUs (e.g. `0,5` with a comma, `-1`,
    /// `abc`). The backend applies *no* CPU limit in that case, so the
    /// intended cap silently isn't enforced. `field` is the offending
    /// key (`limit`/`request`).
    InvalidCpuLimit {
        spec_id: String,
        field: String,
        value: String,
    },
    /// `container-memory-limit` / `container-memory-request` is set but
    /// doesn't parse as a Docker-style size (`"512m"`, `"1.5g"`, plain
    /// bytes) — e.g. the `"512mb"` typo. The backend applies *no* memory
    /// cap, the opposite of intent.
    InvalidMemoryLimit {
        spec_id: String,
        field: String,
        value: String,
    },
    /// A containerized spec sets `seats-per-container: 0`. Zero seats
    /// makes a replica look saturated and idle at the same time, which
    /// confuses the auto-scaler (it wants to scale up forever). Almost
    /// always a typo for `1` or more.
    ZeroSeats {
        spec_id: String,
    },
    /// A `volumes` entry isn't valid Docker bind syntax — expected
    /// `"/host:/container"` (optionally `":ro"`), with an absolute
    /// container path. The mount would be silently skipped.
    InvalidVolume {
        spec_id: String,
        value: String,
    },
    /// A `proxy.hosts` entry is malformed: empty/duplicate `id`, an
    /// `address` with an unsupported scheme, or `tls` set on a non-`tcp`
    /// address (or missing on a `tcp://` one). The host would fail to
    /// connect at startup. `host` is the id (or index when blank).
    InvalidHost {
        host: String,
        reason: String,
    },
    /// A containerized spec's `max-replicas` is explicitly `0` — every
    /// spawn is refused (`live >= max`), so the app can never start and
    /// visitors wait on the splash forever. The default floor exists
    /// precisely to prevent this; an explicit zero is almost always a
    /// mistake (#743).
    ReplicaCeilingZero {
        spec_id: String,
    },
    /// `type:` declares a containerized kind but the spec has no
    /// `container-image` — the spawn fails only when a visitor first
    /// opens the app (#743).
    MissingContainerImage {
        spec_id: String,
    },
    /// `type: external` together with a `container-image` — the image
    /// is silently ignored; one of the two is a mistake (#743).
    ExternalWithContainerImage {
        spec_id: String,
    },
    /// A modeled ShinyProxy field is set to a non-default value but has
    /// no runtime effect in Ruscker — the operator gets none of what
    /// they configured. `server.secure-cookies` is the sharpest case: a
    /// false security expectation (the real `Secure` flag follows
    /// `X-Forwarded-Proto`). Surfaced always (not just under
    /// `--strict-compat`) per the project policy that unsupported
    /// features must be loud (#743).
    IgnoredCompatField {
        field: &'static str,
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
            // Only a PURE `${VAR}` reference is exempt — the loose
            // `starts_with("${")` gate let `${VAR}-literalsecret` pass
            // unflagged even though interpolation preserves the whole
            // line verbatim, landing the literal tail in the DB on
            // import (#743; same reasoning as the credential-store
            // regression #422 that motivated `is_pure_env_ref`).
            if unquoted.is_empty() || crate::env::is_pure_env_ref(unquoted) {
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
    // NOTE: ShinyProxy's spec-level `port` IS now supported — it maps
    // to Ruscker's `container-port` (see schema). So it's intentionally
    // absent here.
    (
        "minimum-seats-available",
        "pre-warm pool not implemented — use `min-replicas`",
    ),
    (
        "network-connections",
        "multi-network attach is not implemented — map it to the single \
         `container-network` field (Ruscker creates + attaches that one)",
    ),
    // NOTE: per-spec env/cmd injection IS now supported — map your
    // ShinyProxy `container-env` / `container-cmd` straight across. They
    // are intentionally absent from this ignored-fields list.
    //
    // NOTE: the whole #326 family of scaling/lifecycle knobs is enforced
    // now and is intentionally absent from this list:
    //   `scale-up-threshold` / `scale-down-threshold` / `scale-down-grace`
    //     — scale on pool utilization vs the thresholds, per-spec grace
    //       (#333);
    //   `max-lifetime` / `container-lifetime` — recycle past the age cap
    //     (#334);
    //   `drain-timeout` — grace for a busy `max-lifetime` recycle (#335);
    //   `stop-on-logout` — end a user's sticky sessions on logout (#337);
    //   `concurrent-requests-per-replica` — API capacity metered by
    //     in-flight requests (#336).
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
/// - Everything else (`kubernetes-*`, `minimum-seats-available`, …) is
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

    check_hosts(&config.proxy.hosts, &mut warnings);
    check_ignored_compat_fields(config, &mut warnings);

    let stats = collect_stats(config);

    ValidationReport { warnings, stats }
}

/// Modeled ShinyProxy fields that parse fine but have **no runtime
/// consumer** in Ruscker (#743). They all carry defaults, so "the
/// operator set it" means "differs from the default". Each one set is
/// one warning — silently ignoring configured behaviour violates the
/// project's compat policy, and `server.secure-cookies` in particular
/// builds a false security expectation.
fn check_ignored_compat_fields(config: &Config, warnings: &mut Vec<Warning>) {
    let mut ignored = |field: &'static str, set: bool| {
        if set {
            warnings.push(Warning::IgnoredCompatField { field });
        }
    };
    ignored("server.secure-cookies", config.server.secure_cookies);
    ignored(
        "server.servlet.session.timeout",
        config.server.session_timeout_secs().is_some(),
    );
    ignored(
        "proxy.heartbeat-rate",
        config.proxy.heartbeat_rate != 10_000,
    );
    ignored("proxy.hide-navbar", config.proxy.hide_navbar);
    ignored("proxy.landing-page", config.proxy.landing_page != "/");
    ignored(
        "proxy.container-log-path",
        config.proxy.container_log_path.is_some(),
    );
    ignored("logging.file", config.logging.file.is_some());
}

/// Validate `proxy.hosts` (Phase 6): non-empty unique ids, a supported
/// address scheme, and `tls` paired correctly with `tcp://`. Pure
/// string-level checks — the actual daemon connection happens in
/// `ruscker-docker` at startup.
fn check_hosts(hosts: &[crate::schema::Host], warnings: &mut Vec<Warning>) {
    let mut seen: HashMap<&str, usize> = HashMap::new();
    for (i, host) in hosts.iter().enumerate() {
        let label = if host.id.trim().is_empty() {
            format!("#{i}")
        } else {
            host.id.clone()
        };

        if host.id.trim().is_empty() {
            warnings.push(Warning::InvalidHost {
                host: label.clone(),
                reason: "empty `id`".to_string(),
            });
        } else {
            *seen.entry(host.id.as_str()).or_insert(0) += 1;
        }

        let addr = host.address.trim();
        let scheme = addr.split("://").next().filter(|s| *s != addr);
        match scheme {
            Some("ssh") | Some("http") | Some("unix") => {
                if host.tls.is_some() {
                    warnings.push(Warning::InvalidHost {
                        host: label.clone(),
                        reason: format!("`tls` is only used with `tcp://` (address is {addr})"),
                    });
                }
            }
            Some("tcp") => {
                if host.tls.is_none() {
                    warnings.push(Warning::InvalidHost {
                        host: label.clone(),
                        reason: "`tcp://` host needs `tls` (ca/cert/key) for mutual TLS"
                            .to_string(),
                    });
                }
            }
            _ => warnings.push(Warning::InvalidHost {
                host: label.clone(),
                reason: format!(
                    "address `{addr}` must start with ssh:// , tcp:// , http:// or unix://"
                ),
            }),
        }
    }

    for (id, count) in seen {
        if count > 1 {
            warnings.push(Warning::InvalidHost {
                host: id.to_string(),
                reason: "duplicate host id".to_string(),
            });
        }
    }
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
    // An explicit `max-replicas: 0` on a containerized spec refuses
    // every spawn — the exact hang the default floor exists to prevent,
    // reachable via config with zero feedback (#743). `max >= min`
    // keeps this from doubling up with InvalidReplicaRange above.
    if spec.kind() != SpecKind::External && max == 0 && max >= min {
        warnings.push(Warning::ReplicaCeilingZero {
            spec_id: spec.id.clone(),
        });
    }

    // `type:` vs `container-image` conflicts are silent in both
    // directions (#743): a containerized kind with no image fails only
    // at visit time; `type: external` with an image silently ignores it.
    if spec.kind() != SpecKind::External && spec.container_image.is_none() {
        warnings.push(Warning::MissingContainerImage {
            spec_id: spec.id.clone(),
        });
    }
    if spec.kind() == SpecKind::External && spec.container_image.is_some() {
        warnings.push(Warning::ExternalWithContainerImage {
            spec_id: spec.id.clone(),
        });
    }

    // Zero seats on a containerized spec makes the replica look both
    // saturated and idle — a foot-gun for the scaler.
    if spec.kind() != crate::schema::SpecKind::External && spec.seats_per_container == Some(0) {
        warnings.push(Warning::ZeroSeats {
            spec_id: spec.id.clone(),
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

    // CPU limit/request set but not a positive, finite number of CPUs →
    // the backend applies no CPU cap, so the intended limit silently
    // does nothing.
    for (field, val) in [
        ("limit", spec.container_cpu_limit),
        ("request", spec.container_cpu_request),
    ] {
        if let Some(v) = val {
            if !(v.is_finite() && v > 0.0) {
                warnings.push(Warning::InvalidCpuLimit {
                    spec_id: spec.id.clone(),
                    field: field.to_string(),
                    value: v.to_string(),
                });
            }
        }
    }

    // Memory limit/request set but unparseable (e.g. the `512mb` typo) →
    // no memory cap applied, the opposite of intent.
    for (field, raw, parsed) in [
        (
            "limit",
            &spec.container_memory_limit,
            spec.effective_memory_limit_bytes(),
        ),
        (
            "request",
            &spec.container_memory_request,
            spec.effective_memory_request_bytes(),
        ),
    ] {
        if let Some(s) = raw {
            if parsed.is_none() {
                warnings.push(Warning::InvalidMemoryLimit {
                    spec_id: spec.id.clone(),
                    field: field.to_string(),
                    value: s.clone(),
                });
            }
        }
    }

    // Volume bind syntax: "/host:/container[:ro]" with an absolute
    // container path. A malformed entry would be silently skipped.
    if let Some(vols) = &spec.volumes {
        for v in vols {
            if !is_valid_volume_bind(v) {
                warnings.push(Warning::InvalidVolume {
                    spec_id: spec.id.clone(),
                    value: v.clone(),
                });
            }
        }
    }
}

/// A Docker bind spec: `host:container` plus an optional `:ro`/`:rw`
/// mode, with a non-empty host and an absolute container path. Public so
/// the admin form validates with the same rule.
///
/// Peels the optional trailing mode first, then splits the rest into
/// `host:container` on the **last** colon (the container path is the
/// final segment and must be absolute). This drops the old rigid "at
/// most 3 colon-separated parts" rule that wrongly rejected a host path
/// containing a colon (legal on Linux) (#328). This is a *syntax* check
/// — whether the host path exists is a runtime concern.
pub fn is_valid_volume_bind(v: &str) -> bool {
    // Strip a trailing `:ro` / `:rw` mode if present.
    let body = match v.rsplit_once(':') {
        Some((head, mode)) if matches!(mode.trim(), "ro" | "rw") => head,
        _ => v,
    };
    // The remainder is `host:container`. The container path is the last
    // colon-segment (it must be absolute), so split on the LAST colon —
    // that way a host path that itself contains a colon stays with the
    // host instead of being mistaken for the container.
    let Some((host, container)) = body.rsplit_once(':') else {
        return false;
    };
    !host.trim().is_empty() && container.starts_with('/')
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

        // Access breakdown reflects the real rule (`Spec::is_open()` —
        // `access-groups`/`access-users`), not the retired decorative
        // `template-properties.icon` flag (#346).
        let access = if spec.is_open() { "open" } else { "restricted" };
        *by_access.entry(access.to_string()).or_insert(0) += 1;
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
    fn flags_malformed_memory_limit() {
        // `512mb` is the classic typo — only `512m` is valid.
        let yaml = r#"
proxy:
  specs:
    - id: app1
      container-image: org/app:1
      container-memory-limit: "512mb"
"#;
        let report = Config::from_yaml(yaml).expect("parse").validate();
        assert!(
            report.warnings.iter().any(|w| matches!(
                w,
                Warning::InvalidMemoryLimit { spec_id, field, value }
                    if spec_id == "app1" && field == "limit" && value == "512mb"
            )),
            "expected InvalidMemoryLimit, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn flags_nonpositive_cpu_limit() {
        let yaml = r#"
proxy:
  specs:
    - id: app1
      container-image: org/app:1
      container-cpu-limit: -1
"#;
        let report = Config::from_yaml(yaml).expect("parse").validate();
        assert!(
            report.warnings.iter().any(|w| matches!(
                w,
                Warning::InvalidCpuLimit { spec_id, field, .. }
                    if spec_id == "app1" && field == "limit"
            )),
            "expected InvalidCpuLimit, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn flags_zero_seats_on_containerized_spec() {
        let yaml = r#"
proxy:
  specs:
    - id: app1
      container-image: org/app:1
      seats-per-container: 0
"#;
        let report = Config::from_yaml(yaml).expect("parse").validate();
        assert!(
            report
                .warnings
                .iter()
                .any(|w| matches!(w, Warning::ZeroSeats { spec_id } if spec_id == "app1")),
            "expected ZeroSeats, got {:?}",
            report.warnings
        );
    }

    #[test]
    fn flags_malformed_volume_but_accepts_valid_ones() {
        let yaml = r#"
proxy:
  specs:
    - id: app1
      container-image: org/app:1
      volumes:
        - "/srv/data:/data"
        - "/srv/www:/www:ro"
        - "not-a-bind"
"#;
        let report = Config::from_yaml(yaml).expect("parse").validate();
        let bad: Vec<&str> = report
            .warnings
            .iter()
            .filter_map(|w| match w {
                Warning::InvalidVolume { value, .. } => Some(value.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(bad, vec!["not-a-bind"], "only the malformed bind warns");
    }

    #[test]
    fn volume_bind_parser_handles_modes_and_colon_paths() {
        // Plain + mode forms.
        assert!(is_valid_volume_bind("/srv/data:/data"));
        assert!(is_valid_volume_bind("/srv/www:/www:ro"));
        assert!(is_valid_volume_bind("/srv/www:/www:rw"));
        // #328: a host path containing a colon (legal on Linux) is no
        // longer wrongly rejected by a rigid 3-parts cap. The container
        // is the last segment; the host keeps the rest.
        assert!(is_valid_volume_bind("/srv/a:b:/data"));
        assert!(is_valid_volume_bind("/srv/a:b:/data:ro"));
        // Still rejected: no container path, non-absolute container,
        // empty host.
        assert!(!is_valid_volume_bind("not-a-bind"));
        assert!(!is_valid_volume_bind("/host:relative"));
        assert!(!is_valid_volume_bind(":/data"));
    }

    #[test]
    fn accepts_valid_cpu_and_memory_limits() {
        let yaml = r#"
proxy:
  specs:
    - id: app1
      container-image: org/app:1
      container-cpu-limit: 0.5
      container-memory-limit: "512m"
"#;
        let report = Config::from_yaml(yaml).expect("parse").validate();
        assert!(
            !report.warnings.iter().any(|w| matches!(
                w,
                Warning::InvalidCpuLimit { .. } | Warning::InvalidMemoryLimit { .. }
            )),
            "valid cpu/memory should not warn, got {:?}",
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
    minimum-seats-available: 2
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
        // A still-unsupported field is flagged.
        assert!(
            fields.contains(&"minimum-seats-available"),
            "fields: {fields:?}"
        );
        // `volumes` (#99) and `port`→`container-port` (#120) are
        // supported now — they must NOT be flagged.
        assert!(
            !fields.contains(&"volumes"),
            "volumes is supported now: {fields:?}"
        );
        assert!(
            !fields.contains(&"port"),
            "port maps to container-port now: {fields:?}"
        );
        assert!(
            fields.contains(&"kubernetes-pod-patches"),
            "fields: {fields:?}"
        );
    }

    #[test]
    fn compat_scan_no_longer_flags_scaling_knobs() {
        // The whole #326 family is enforced now (#333/#334/#335/#336/#337),
        // so strict-compat must NOT flag any of them anymore.
        let yaml = "\
proxy:
  specs:
  - id: scaled
    container-image: rocker/shiny
    scale-up-threshold: 0.9
    scale-down-threshold: 0.2
    scale-down-grace: 600
    drain-timeout: 90
    concurrent-requests-per-replica: 4
    max-lifetime: 3600
    container-lifetime: 1800
    stop-on-logout: true
";
        let fields: Vec<String> = compat(yaml)
            .into_iter()
            .filter_map(|w| match w {
                CompatWarning::UnsupportedSpecField { field, .. } => Some(field),
                _ => None,
            })
            .collect();
        for enforced in [
            "scale-up-threshold",
            "scale-down-threshold",
            "scale-down-grace",
            "drain-timeout",
            "concurrent-requests-per-replica",
            "max-lifetime",
            "container-lifetime",
            "stop-on-logout",
        ] {
            assert!(
                !fields.iter().any(|f| f == enforced),
                "{enforced} is enforced now: {fields:?}"
            );
        }
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

    // ── proxy.hosts validation (Phase 6) ────────────────────────────

    fn host_warnings(yaml: &str) -> Vec<String> {
        let config = Config::from_yaml(yaml).expect("parse");
        run(&config)
            .warnings
            .iter()
            .filter_map(|w| match w {
                Warning::InvalidHost { host, reason } => Some(format!("{host}: {reason}")),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn valid_hosts_produce_no_warnings() {
        let yaml = "\
proxy:
  hosts:
    - id: ssh-1
      address: ssh://ops@10.0.0.11
    - id: tcp-1
      address: tcp://10.0.0.12:2376
      tls: { ca: /c/ca.pem, cert: /c/cert.pem, key: /c/key.pem }
    - id: local
      address: unix:///var/run/docker.sock
  specs: []
";
        assert!(host_warnings(yaml).is_empty(), "{:?}", host_warnings(yaml));
    }

    #[test]
    fn host_flags_bad_scheme_dup_and_tls_mismatch() {
        let yaml = "\
proxy:
  hosts:
    - id: weird
      address: rdp://nope
    - id: tcp-notls
      address: tcp://h:2376
    - id: ssh-withtls
      address: ssh://ops@h
      tls: { ca: /a, cert: /b, key: /c }
    - id: dup
      address: ssh://a
    - id: dup
      address: ssh://b
  specs: []
";
        let w = host_warnings(yaml);
        let has = |id: &str, frag: &str| w.iter().any(|s| s.starts_with(id) && s.contains(frag));
        assert!(has("weird:", "ssh://"));
        assert!(has("tcp-notls:", "needs `tls`"));
        assert!(has("ssh-withtls:", "only used with"));
        assert!(has("dup:", "duplicate"));
    }

    #[test]
    fn no_hosts_is_clean() {
        // Absent `hosts` is the default single-local-daemon mode.
        assert!(host_warnings("proxy:\n  specs: []\n").is_empty());
    }

    // ── #743: new validation gaps ────────────────────────────────

    #[test]
    fn flags_explicit_zero_replica_ceiling() {
        let yaml = "proxy:\n  specs:\n    - id: dead\n      container-image: a:1\n      min-replicas: 0\n      max-replicas: 0\n";
        let report = Config::from_yaml(yaml).expect("parse").validate();
        assert!(
            report.warnings.iter().any(|w| matches!(
                w,
                Warning::ReplicaCeilingZero { spec_id } if spec_id == "dead"
            )),
            "got {:?}",
            report.warnings
        );
        // The default ceiling is fine.
        let ok = Config::from_yaml("proxy:\n  specs:\n    - id: ok\n      container-image: a:1\n")
            .expect("parse")
            .validate();
        assert!(
            !ok.warnings
                .iter()
                .any(|w| matches!(w, Warning::ReplicaCeilingZero { .. })),
            "got {:?}",
            ok.warnings
        );
    }

    #[test]
    fn flags_type_vs_image_conflicts_both_directions() {
        // Containerized type with no image: fails only at visit time.
        let yaml = "proxy:\n  specs:\n    - id: imageless\n      type: shiny\n";
        let report = Config::from_yaml(yaml).expect("parse").validate();
        assert!(
            report.warnings.iter().any(|w| matches!(
                w,
                Warning::MissingContainerImage { spec_id } if spec_id == "imageless"
            )),
            "got {:?}",
            report.warnings
        );
        // `type: external` with an image: the image is silently ignored.
        let yaml = "proxy:\n  specs:\n    - id: ext\n      type: external\n      container-image: a:1\n";
        let report = Config::from_yaml(yaml).expect("parse").validate();
        assert!(
            report.warnings.iter().any(|w| matches!(
                w,
                Warning::ExternalWithContainerImage { spec_id } if spec_id == "ext"
            )),
            "got {:?}",
            report.warnings
        );
        // A plain link spec (no type, no image) is a legitimate External.
        let yaml = "proxy:\n  specs:\n    - id: link\n      external-url: https://example.org\n";
        let report = Config::from_yaml(yaml).expect("parse").validate();
        assert!(
            !report.warnings.iter().any(|w| matches!(
                w,
                Warning::MissingContainerImage { .. } | Warning::ExternalWithContainerImage { .. }
            )),
            "got {:?}",
            report.warnings
        );
    }

    #[test]
    fn scan_flags_partial_env_ref_credential() {
        // `${VAR}-tail` is NOT a pure env-ref: interpolation preserves
        // the line verbatim, so the literal tail would land in the DB.
        let raw = "docker-registry-password: ${REG_PASS}-literal\n";
        let warnings = scan_raw_text(raw);
        assert!(
            warnings
                .iter()
                .any(|w| matches!(w, Warning::EmbeddedCredential { .. })),
            "got {warnings:?}"
        );
        // A pure ref stays exempt.
        assert!(scan_raw_text("docker-registry-password: ${REG_PASS}\n").is_empty());
    }

    #[test]
    fn flags_ignored_compat_fields_only_when_set() {
        let yaml = "\
server:
  secure-cookies: true
proxy:
  hide-navbar: true
  specs: []
";
        let report = Config::from_yaml(yaml).expect("parse").validate();
        let ignored: Vec<&'static str> = report
            .warnings
            .iter()
            .filter_map(|w| match w {
                Warning::IgnoredCompatField { field } => Some(*field),
                _ => None,
            })
            .collect();
        assert!(ignored.contains(&"server.secure-cookies"), "got {ignored:?}");
        assert!(ignored.contains(&"proxy.hide-navbar"), "got {ignored:?}");

        // Defaults (nothing set) produce none of these warnings.
        let clean = Config::from_yaml("proxy:\n  specs: []\n")
            .expect("parse")
            .validate();
        assert!(
            !clean
                .warnings
                .iter()
                .any(|w| matches!(w, Warning::IgnoredCompatField { .. })),
            "got {:?}",
            clean.warnings
        );
    }
}
