//! Configuration schema definitions.
//!
//! This module models the supported subset of ShinyProxy's `application.yml`
//! plus Ruscker-specific extensions (API specs, replica pools, load
//! balancing).
//!
//! # Field naming
//!
//! ShinyProxy uses kebab-case in YAML. We mirror that with serde renaming
//! at struct boundaries. Rust field names use snake_case as idiomatic.
//!
//! # Optional vs required
//!
//! Every field is optional in the YAML and falls back to a sensible
//! default. The only truly required field is `spec.id`, since you can't
//! route to an anonymous spec.

use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// Root of the configuration tree, equivalent to the top of `application.yml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server: Server,

    #[serde(default)]
    pub proxy: Proxy,

    #[serde(default)]
    pub logging: Logging,

    /// Warnings detected by scanning the raw YAML text before parsing
    /// (e.g. embedded credentials). Populated by `Config::from_yaml`,
    /// merged into [`Config::validate`] output.
    #[serde(skip)]
    pub raw_warnings: Vec<crate::validate::Warning>,
}

/// Spring Boot's `server.*` block. We accept it for compatibility but
/// only some fields actually drive Ruscker's behaviour.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Server {
    /// Whether to honour `X-Forwarded-*` headers when computing the
    /// request URL. Maps to ShinyProxy's `server.useForwardHeaders`.
    #[serde(rename = "useForwardHeaders")]
    pub use_forward_headers: bool,

    /// Strategy for trusting forwarded headers. Accepted values mirror
    /// Spring's: `native`, `framework`, `none`.
    #[serde(rename = "forward-headers-strategy")]
    pub forward_headers_strategy: Option<String>,

    /// Whether to set the `Secure` flag on cookies issued by the proxy.
    #[serde(rename = "secure-cookies")]
    pub secure_cookies: bool,

    /// Servlet session timeout in seconds. ShinyProxy emits this as
    /// `server.servlet.session.timeout`; we accept both flat and nested.
    pub servlet: Option<ServletConfig>,

    /// Spring Boot allows `server.servlet.session.timeout: 3600` as a
    /// single dotted key. We accept that form too — it's common in
    /// existing ShinyProxy configs.
    #[serde(rename = "servlet.session.timeout")]
    pub flat_servlet_session_timeout: Option<u64>,
}

impl Server {
    /// Resolved session timeout in seconds, regardless of which YAML
    /// notation the operator used.
    pub fn session_timeout_secs(&self) -> Option<u64> {
        self.servlet
            .as_ref()
            .and_then(|s| s.session.as_ref())
            .and_then(|s| s.timeout)
            .or(self.flat_servlet_session_timeout)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ServletConfig {
    pub session: Option<SessionConfig>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SessionConfig {
    /// Session timeout in seconds.
    pub timeout: Option<u64>,
}

/// The `proxy.*` block — Ruscker's main configuration surface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Proxy {
    /// Display title for the portal (used in `<title>` tag and header).
    pub title: String,

    /// Where the landing page is mounted. Defaults to `/`.
    #[serde(rename = "landing-page")]
    pub landing_page: String,

    /// Whether to hide the default ShinyProxy/Ruscker navbar in favour of
    /// a custom template.
    #[serde(rename = "hide-navbar")]
    pub hide_navbar: bool,

    /// Path to the Askama template directory (overrides defaults).
    #[serde(rename = "template-path")]
    pub template_path: Option<PathBuf>,

    /// How often the client should send heartbeat pings, in milliseconds.
    #[serde(rename = "heartbeat-rate")]
    pub heartbeat_rate: u64,

    /// How long a session can be inactive (no heartbeats) before its
    /// container is reaped, in milliseconds. `-1` means never expire.
    #[serde(rename = "heartbeat-timeout")]
    pub heartbeat_timeout: i64,

    /// How long to wait for a container to become healthy after start,
    /// in milliseconds.
    #[serde(rename = "container-wait-time")]
    pub container_wait_time: u64,

    /// Directory to write per-container logs to.
    #[serde(rename = "container-log-path")]
    pub container_log_path: Option<PathBuf>,

    /// HTTP port to bind the proxy on. Defaults to 8080.
    pub port: u16,

    /// IP address to bind on. Defaults to `0.0.0.0` for Ruscker
    /// (ShinyProxy traditionally used 127.0.0.1 behind a reverse proxy).
    #[serde(rename = "bind-address")]
    pub bind_address: String,

    /// Authentication scheme. Only `none` is supported in the MVP — auth
    /// is expected to happen inside individual apps for now.
    pub authentication: AuthScheme,

    /// The list of app/link/api specs available in this Ruscker instance.
    pub specs: Vec<Spec>,

    /// Optional visual customization of the public landing page.
    /// Ruscker extension — not present in ShinyProxy YAML.
    ///
    /// In Phase 1 these fields are operator-edited in the YAML. The
    /// admin landing-page editor in Phase 2 exposes the same knobs
    /// through a UI (see `docs/mockups/admin-landing-editor.html`).
    #[serde(default, rename = "landing-customization")]
    pub landing_customization: LandingCustomization,
}

impl Default for Proxy {
    fn default() -> Self {
        Self {
            title: "Ruscker".to_string(),
            landing_page: "/".to_string(),
            hide_navbar: false,
            template_path: None,
            heartbeat_rate: 10_000,
            heartbeat_timeout: 3_600_000,
            container_wait_time: 60_000,
            container_log_path: None,
            port: 8080,
            bind_address: "0.0.0.0".to_string(),
            authentication: AuthScheme::None,
            specs: Vec::new(),
            landing_customization: LandingCustomization::default(),
        }
    }
}

/// Visual knobs the operator can twist on the public landing page
/// without writing a custom template. Everything optional — empty
/// values fall back to the built-in design.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LandingCustomization {
    /// CSS color (any form `#rrggbb`, `rgb(...)`, named color)
    /// applied to the header background. Useful for branding to
    /// match an organization's primary color.
    #[serde(default, rename = "header-bg")]
    pub header_bg: Option<String>,

    /// CSS color for header text. Override when `header-bg` is
    /// dark and the default `--text` no longer contrasts.
    #[serde(default, rename = "header-fg")]
    pub header_fg: Option<String>,

    /// Free-form intro paragraph rendered between the header and
    /// the filter section. Operator-authored, plain text (no HTML).
    ///
    /// For multilingual portals, prefer [`Self::intro_locales`]
    /// which lets you provide per-language strings.
    #[serde(default)]
    pub intro: Option<String>,

    /// Map of locale short code → intro text. When the resolved
    /// request locale matches a key here, its value wins over
    /// [`Self::intro`]. Locales not present fall back to `intro`.
    ///
    /// Example:
    /// ```yaml
    /// landing-customization:
    ///   intro-locales:
    ///     pt: "Bem-vindo ao Monitoramento Estratégico..."
    ///     en: "Welcome to Strategic Monitoring..."
    /// ```
    #[serde(default, rename = "intro-locales")]
    pub intro_locales: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthScheme {
    None,
    #[serde(rename = "openid")]
    OpenId,
    Ldap,
    Saml,
    Simple,
}

impl Default for AuthScheme {
    fn default() -> Self {
        AuthScheme::None
    }
}

/// A single spec — an app, API, or external link surfaced on the landing
/// page and (if containerized) orchestrated by Ruscker.
///
/// The presence of `container_image` is what distinguishes a runnable
/// spec from a pure link card. We expose this via [`Spec::kind`] for
/// convenience.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Spec {
    /// Unique identifier. Used as the URL path segment (`/app/<id>/`).
    /// Must be kebab-case-friendly (lowercase, digits, hyphens).
    pub id: String,

    /// Human-readable name shown on the card and inside the app frame.
    #[serde(rename = "display-name")]
    pub display_name: Option<String>,

    /// Description shown on the card. Inline HTML is permitted.
    pub description: Option<String>,

    /// Docker image reference. If absent, this spec is a "link card"
    /// (external URL only, no container management).
    #[serde(rename = "container-image")]
    pub container_image: Option<String>,

    /// Maximum number of concurrent sessions on a single container.
    /// When all containers in a spec's pool reach this limit, the
    /// auto-scaler spawns a new replica (if `max_replicas` allows).
    #[serde(rename = "seats-per-container")]
    pub seats_per_container: Option<u32>,

    /// Hard maximum lifetime for a container in minutes. After this,
    /// the container is recycled even if sessions are active.
    #[serde(rename = "max-lifetime")]
    pub max_lifetime: Option<u64>,

    /// Soft lifetime in minutes. Containers older than this are
    /// preferred for replacement during natural turnover.
    #[serde(rename = "container-lifetime")]
    pub container_lifetime: Option<u64>,

    /// Per-spec heartbeat timeout override in milliseconds.
    /// `-1` disables expiration entirely for this spec.
    #[serde(rename = "heartbeat-timeout")]
    pub heartbeat_timeout: Option<i64>,

    /// Whether the container should be stopped when its last user logs
    /// out. Only meaningful when authentication is enabled.
    #[serde(rename = "stop-on-logout")]
    pub stop_on_logout: Option<bool>,

    // -- Docker registry credentials --
    #[serde(rename = "docker-registry-username")]
    pub docker_registry_username: Option<String>,

    /// Registry password. Strongly recommended to use `${ENV_VAR}`
    /// interpolation rather than embedding credentials in YAML.
    #[serde(rename = "docker-registry-password")]
    pub docker_registry_password: Option<String>,

    #[serde(rename = "docker-registry-domain")]
    pub docker_registry_domain: Option<String>,

    // -- Container resource limits (ShinyProxy-compatible) --
    /// CPU hard limit, expressed as a fraction of a single core.
    /// `0.5` = half a core, `2.0` = two cores. Maps to Docker's
    /// `--cpus`. Backend translates to `cpu_period` + `cpu_quota`.
    #[serde(rename = "container-cpu-limit")]
    pub container_cpu_limit: Option<f64>,

    /// CPU soft request. ShinyProxy accepts this but Docker has
    /// no first-class "request" concept; for the local backend
    /// it's accepted-and-ignored. Captured so K8s/Swarm backends
    /// can honor it later without a schema change.
    #[serde(rename = "container-cpu-request")]
    pub container_cpu_request: Option<f64>,

    /// Memory hard limit. Accepts plain bytes ("536870912") or
    /// a suffix-tagged string ("512m", "1g", "1500M"). Suffixes
    /// follow the Docker convention: `b`/none = bytes, `k`/`K` =
    /// 1024, `m`/`M` = 1024², `g`/`G` = 1024³. Parsed lazily by
    /// [`Spec::effective_memory_limit_bytes`].
    #[serde(rename = "container-memory-limit")]
    pub container_memory_limit: Option<String>,

    /// Memory soft request. Same format as the limit. Maps to
    /// Docker `memory_reservation` on the local backend; ignored
    /// elsewhere.
    #[serde(rename = "container-memory-request")]
    pub container_memory_request: Option<String>,

    /// Free-form properties consumed by the landing page template.
    /// Common keys: `logo`, `icon`, `type`, `updated`, `state`, `link`.
    #[serde(rename = "template-properties", default)]
    pub template_properties: TemplateProperties,

    // -- Ruscker extensions: spec type --
    /// Spec type. When absent:
    /// - `App` if `container_image` is set
    /// - `External` otherwise
    /// Use this field explicitly to mark a containerized API
    /// (`api`), a Streamlit/Dash app, etc.
    #[serde(rename = "type")]
    pub kind_override: Option<SpecKindOverride>,

    /// API-specific configuration. Used when `type: api`.
    pub api: Option<ApiSpec>,

    // -- Ruscker extensions: load balancing --
    /// Minimum number of container replicas to keep running.
    /// Defaults to 1.
    #[serde(rename = "min-replicas")]
    pub min_replicas: Option<u32>,

    /// Maximum number of container replicas to scale up to.
    /// Defaults to the value of `min-replicas` (no auto-scaling).
    #[serde(rename = "max-replicas")]
    pub max_replicas: Option<u32>,

    /// Utilization fraction at which to spawn a new replica.
    /// Defaults to 0.8 (80%).
    #[serde(rename = "scale-up-threshold")]
    pub scale_up_threshold: Option<OrderedFloat<f64>>,

    /// Utilization fraction at which to retire a replica after the
    /// grace period. Defaults to 0.3 (30%).
    #[serde(rename = "scale-down-threshold")]
    pub scale_down_threshold: Option<OrderedFloat<f64>>,

    /// Seconds a replica must remain below `scale-down-threshold` before
    /// being retired. Defaults to 300 (5 minutes).
    #[serde(rename = "scale-down-grace")]
    pub scale_down_grace: Option<u64>,

    /// Seconds to wait for in-flight sessions to drain before killing
    /// a replica being retired. Defaults to 60.
    #[serde(rename = "drain-timeout")]
    pub drain_timeout: Option<u64>,

    /// Routing strategy for distributing new sessions across replicas.
    #[serde(rename = "routing-strategy")]
    pub routing_strategy: Option<RoutingStrategy>,

    /// Maximum concurrent requests handled by a single API replica.
    /// Only meaningful for `type: api`. Defaults to 100.
    #[serde(rename = "concurrent-requests-per-replica")]
    pub concurrent_requests_per_replica: Option<u32>,
}

/// Loose key-value bag for `template-properties`.
///
/// We don't enforce a schema here — the template renders whatever keys
/// are present. Known keys used by the current landing template are
/// described in `docs/YAML_SCHEMA.md`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TemplateProperties(pub HashMap<String, serde_yaml_ng::Value>);

impl TemplateProperties {
    pub fn get_str(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.as_str())
    }

    pub fn type_field(&self) -> Option<&str> {
        self.get_str("type")
    }

    pub fn state(&self) -> &str {
        self.get_str("state").unwrap_or("active")
    }

    pub fn is_active(&self) -> bool {
        self.state() == "active"
    }
}

/// Explicit `type:` field. Overrides the auto-detection from
/// `container_image` presence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SpecKindOverride {
    Shiny,
    Streamlit,
    Dash,
    Voila,
    Api,
    External,
}

/// Effective spec kind, computed from `kind_override` and the presence
/// of `container_image`. This is what runtime code dispatches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpecKind {
    /// A Shiny-style container with WebSocket reactivity. Requires
    /// sticky sessions and container-per-N-users model.
    Shiny,
    /// Streamlit, Dash, Voilà — also WebSocket-based, similar lifecycle.
    InteractiveApp,
    /// Plumber/FastAPI/etc. — HTTP-only, stateless, round-robin-friendly.
    Api,
    /// External link only, no container management.
    External,
}

impl Spec {
    /// Compute the effective [`SpecKind`].
    ///
    /// Priority:
    /// 1. Explicit `type:` field if set
    /// 2. Auto: `Shiny` if `container_image` is set, else `External`
    pub fn kind(&self) -> SpecKind {
        match self.kind_override {
            Some(SpecKindOverride::Api) => SpecKind::Api,
            Some(SpecKindOverride::Shiny) => SpecKind::Shiny,
            Some(SpecKindOverride::Streamlit | SpecKindOverride::Dash | SpecKindOverride::Voila) => {
                SpecKind::InteractiveApp
            }
            Some(SpecKindOverride::External) => SpecKind::External,
            None => {
                if self.container_image.is_some() {
                    SpecKind::Shiny
                } else {
                    SpecKind::External
                }
            }
        }
    }

    /// Effective number of seats per container, falling back to a
    /// sensible default per spec kind.
    pub fn effective_seats(&self) -> u32 {
        self.seats_per_container.unwrap_or_else(|| match self.kind() {
            SpecKind::Api => 100,
            SpecKind::Shiny | SpecKind::InteractiveApp => 1,
            SpecKind::External => 0,
        })
    }

    /// Effective minimum replicas (default 1 for containerized, 0 for
    /// external).
    pub fn effective_min_replicas(&self) -> u32 {
        self.min_replicas.unwrap_or_else(|| match self.kind() {
            SpecKind::External => 0,
            _ => 1,
        })
    }

    /// Effective maximum replicas (defaults to `min_replicas`, so no
    /// auto-scaling unless the operator opts in).
    pub fn effective_max_replicas(&self) -> u32 {
        self.max_replicas.unwrap_or_else(|| self.effective_min_replicas())
    }

    /// Effective routing strategy.
    pub fn effective_routing(&self) -> RoutingStrategy {
        self.routing_strategy.unwrap_or_else(|| match self.kind() {
            SpecKind::Api => RoutingStrategy::RoundRobin,
            _ => RoutingStrategy::LeastConnections,
        })
    }

    /// Does this spec need sticky session affinity?
    pub fn needs_sticky_sessions(&self) -> bool {
        matches!(
            self.kind(),
            SpecKind::Shiny | SpecKind::InteractiveApp
        )
    }

    /// Parsed memory hard limit in bytes, or `None` if the spec
    /// didn't set one. A malformed value (e.g. `"500frogs"`)
    /// also yields `None` — the validator catches that at
    /// load time; downstream callers can safely treat parse
    /// failure as "no limit".
    pub fn effective_memory_limit_bytes(&self) -> Option<i64> {
        self.container_memory_limit
            .as_deref()
            .and_then(parse_memory_string)
    }

    /// Parsed memory soft request in bytes (Docker's
    /// `memory_reservation`), or `None`.
    pub fn effective_memory_request_bytes(&self) -> Option<i64> {
        self.container_memory_request
            .as_deref()
            .and_then(parse_memory_string)
    }
}

/// Parse Docker-style memory strings: `"512"` (bytes), `"512m"`,
/// `"1g"`, `"1.5G"`. Suffix is single-letter ASCII, case-
/// insensitive: `b` = bytes, `k` = KiB (1024), `m` = MiB,
/// `g` = GiB. Returns `None` on any parse failure or negative
/// result.
///
/// We deliberately use binary (1024-based) units to match the
/// ShinyProxy / Docker convention. Operators who type `512m`
/// expect ~512 MiB, not ~512 MB.
fn parse_memory_string(s: &str) -> Option<i64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let last = s.chars().last()?;
    let (num_part, multiplier) = match last.to_ascii_lowercase() {
        'b' => (&s[..s.len() - 1], 1_i64),
        'k' => (&s[..s.len() - 1], 1024_i64),
        'm' => (&s[..s.len() - 1], 1024_i64 * 1024),
        'g' => (&s[..s.len() - 1], 1024_i64 * 1024 * 1024),
        'a'..='z' => return None, // unknown suffix
        _ => (s, 1_i64),           // plain bytes, no suffix
    };
    let n: f64 = num_part.trim().parse().ok()?;
    if !n.is_finite() || n < 0.0 {
        return None;
    }
    // Multiply in f64 then cast — handles "1.5g" cleanly.
    let bytes = (n * multiplier as f64) as i64;
    Some(bytes).filter(|&b| b >= 0)
}

#[cfg(test)]
mod parse_memory_tests {
    use super::parse_memory_string;

    #[test]
    fn plain_bytes() {
        assert_eq!(parse_memory_string("1024"), Some(1024));
        assert_eq!(parse_memory_string("0"), Some(0));
    }

    #[test]
    fn kilo_mega_giga_binary() {
        assert_eq!(parse_memory_string("1k"), Some(1024));
        assert_eq!(parse_memory_string("1m"), Some(1024 * 1024));
        assert_eq!(parse_memory_string("1g"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_memory_string("512m"), Some(512 * 1024 * 1024));
    }

    #[test]
    fn case_insensitive_suffix() {
        assert_eq!(parse_memory_string("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_memory_string("256M"), Some(256 * 1024 * 1024));
    }

    #[test]
    fn fractional_values() {
        assert_eq!(parse_memory_string("1.5g"), Some(1610612736)); // 1.5 * 1024^3
        assert_eq!(parse_memory_string("0.5m"), Some(524288));
    }

    #[test]
    fn explicit_byte_suffix() {
        assert_eq!(parse_memory_string("100b"), Some(100));
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_memory_string(""), None);
        assert_eq!(parse_memory_string("abc"), None);
        assert_eq!(parse_memory_string("500frogs"), None);
        assert_eq!(parse_memory_string("-1g"), None);
        assert_eq!(parse_memory_string("1.5x"), None); // unknown suffix
    }

    #[test]
    fn whitespace_tolerated() {
        assert_eq!(parse_memory_string("  512m  "), Some(512 * 1024 * 1024));
    }
}

/// Configuration block for `type: api` specs (Plumber, FastAPI, etc.).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct ApiSpec {
    /// Container port the API listens on. Defaults to 8080.
    pub port: Option<u16>,

    /// Path where auto-generated OpenAPI docs live. Defaults to
    /// `/__docs__`. Used by the landing page to link to the docs UI.
    #[serde(rename = "docs-path")]
    pub docs_path: Option<String>,

    /// Path to hit for readiness checks before adding the replica to
    /// the pool. Defaults to `/__healthz__`.
    #[serde(rename = "health-path")]
    pub health_path: Option<String>,

    /// Rate limit applied at the proxy layer, e.g. `100/min`.
    /// `None` disables limiting.
    #[serde(rename = "rate-limit")]
    pub rate_limit: Option<String>,

    /// Whether to add permissive CORS headers. Defaults to false.
    pub cors: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RoutingStrategy {
    /// Pick the replica with the most available capacity. Best default
    /// for Shiny/interactive apps with seat limits.
    LeastConnections,
    /// Cycle through replicas evenly. Good for stateless APIs.
    RoundRobin,
    /// Random pick weighted by remaining capacity.
    WeightedRandom,
    /// Pick based on actual CPU/memory utilization (requires metrics).
    ResourceAware,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Logging {
    pub file: Option<LoggingFile>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LoggingFile {
    pub name: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let yaml = r#"
proxy:
  title: Test
  specs:
    - id: hello
      display-name: Hello
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        assert_eq!(config.proxy.title, "Test");
        assert_eq!(config.proxy.specs.len(), 1);
        assert_eq!(config.proxy.specs[0].id, "hello");
    }

    #[test]
    fn auto_classifies_external_when_no_image() {
        let spec = Spec {
            id: "pkg".to_string(),
            display_name: None,
            description: None,
            container_image: None,
            seats_per_container: None,
            max_lifetime: None,
            container_lifetime: None,
            heartbeat_timeout: None,
            stop_on_logout: None,
            docker_registry_username: None,
            docker_registry_password: None,
            docker_registry_domain: None,
            container_cpu_limit: None,
            container_cpu_request: None,
            container_memory_limit: None,
            container_memory_request: None,
            template_properties: TemplateProperties::default(),
            kind_override: None,
            api: None,
            min_replicas: None,
            max_replicas: None,
            scale_up_threshold: None,
            scale_down_threshold: None,
            scale_down_grace: None,
            drain_timeout: None,
            routing_strategy: None,
            concurrent_requests_per_replica: None,
        };
        assert_eq!(spec.kind(), SpecKind::External);
        assert!(!spec.needs_sticky_sessions());
    }

    #[test]
    fn auto_classifies_shiny_when_image_present() {
        let spec = Spec {
            id: "app".to_string(),
            display_name: None,
            description: None,
            container_image: Some("foo/bar:latest".to_string()),
            seats_per_container: None,
            max_lifetime: None,
            container_lifetime: None,
            heartbeat_timeout: None,
            stop_on_logout: None,
            docker_registry_username: None,
            docker_registry_password: None,
            docker_registry_domain: None,
            container_cpu_limit: None,
            container_cpu_request: None,
            container_memory_limit: None,
            container_memory_request: None,
            template_properties: TemplateProperties::default(),
            kind_override: None,
            api: None,
            min_replicas: None,
            max_replicas: None,
            scale_up_threshold: None,
            scale_down_threshold: None,
            scale_down_grace: None,
            drain_timeout: None,
            routing_strategy: None,
            concurrent_requests_per_replica: None,
        };
        assert_eq!(spec.kind(), SpecKind::Shiny);
        assert!(spec.needs_sticky_sessions());
    }

    #[test]
    fn api_routing_default_is_round_robin() {
        let mut spec = Spec {
            id: "api".to_string(),
            display_name: None,
            description: None,
            container_image: Some("foo/api:latest".to_string()),
            seats_per_container: None,
            max_lifetime: None,
            container_lifetime: None,
            heartbeat_timeout: None,
            stop_on_logout: None,
            docker_registry_username: None,
            docker_registry_password: None,
            docker_registry_domain: None,
            container_cpu_limit: None,
            container_cpu_request: None,
            container_memory_limit: None,
            container_memory_request: None,
            template_properties: TemplateProperties::default(),
            kind_override: Some(SpecKindOverride::Api),
            api: None,
            min_replicas: None,
            max_replicas: None,
            scale_up_threshold: None,
            scale_down_threshold: None,
            scale_down_grace: None,
            drain_timeout: None,
            routing_strategy: None,
            concurrent_requests_per_replica: None,
        };
        spec.kind_override = Some(SpecKindOverride::Api);
        assert_eq!(spec.kind(), SpecKind::Api);
        assert_eq!(spec.effective_routing(), RoutingStrategy::RoundRobin);
        assert!(!spec.needs_sticky_sessions());
    }
}
