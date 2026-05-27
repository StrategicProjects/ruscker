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

    /// How long to wait for active sessions to drain on graceful
    /// shutdown (SIGTERM / Ctrl-C) before forcing exit, in
    /// milliseconds. During the drain window `/readyz` reports
    /// `draining` so load balancers stop routing new traffic; the
    /// process exits early once the last session ends. `0` exits
    /// as soon as in-flight HTTP requests finish. Ruscker
    /// extension — not present in ShinyProxy YAML.
    #[serde(rename = "shutdown-grace-ms")]
    pub shutdown_grace_ms: u64,

    /// Maximum request body accepted on proxied routes (`/app/*`,
    /// `/api/*`), as a Docker-style size string (`"10m"`, `"1g"`,
    /// or plain bytes). A request whose `Content-Length` exceeds
    /// this — or whose streamed body grows past it — gets
    /// `413 Payload Too Large`. `None` (the default) means no limit.
    /// A per-spec `max-body-size` overrides this for that spec.
    /// Ruscker extension — not present in ShinyProxy YAML.
    #[serde(rename = "max-body-size")]
    pub max_body_size: Option<String>,

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

    /// Expose a Prometheus `/metrics` endpoint. **Off by default.**
    /// When enabled, `/metrics` is served *unauthenticated* (like
    /// `/healthz`), so only turn it on where the endpoint is reachable
    /// only by your scraper (private network / firewall). Ruscker
    /// extension — not present in ShinyProxy YAML.
    #[serde(rename = "metrics-enabled")]
    pub metrics_enabled: bool,

    /// Docker hosts for multi-host scheduling (Phase 6). **Empty (the
    /// default) ⇒ the single local Docker daemon** — today's behaviour.
    /// When set, Ruscker spawns app containers across these hosts.
    /// Ruscker extension.
    #[serde(default)]
    pub hosts: Vec<Host>,
}

/// A Docker host Ruscker can schedule containers onto (Phase 6).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Host {
    /// Stable identifier — referenced by replicas, placement, and the
    /// dashboard. Required and unique.
    pub id: String,

    /// Daemon address. Supported schemes:
    /// - `ssh://user@host[:port]` — Docker over SSH (simplest to run);
    /// - `tcp://host:port` — Docker over TLS (set `tls`);
    /// - `http://host:port` — Docker over plain TCP (no TLS; trusted
    ///   networks only);
    /// - `unix:///path/to/docker.sock` — a local socket.
    pub address: String,

    /// Client TLS material for `tcp://` mutual TLS. Ignored for other
    /// schemes.
    #[serde(default)]
    pub tls: Option<HostTls>,

    /// Optional hard cap on the number of containers this host runs.
    /// Placement (Phase 6c) won't exceed it.
    #[serde(rename = "max-containers")]
    pub max_containers: Option<u32>,

    /// Relative weight for spread placement. Defaults to 1.
    pub weight: Option<u32>,
}

/// Client TLS paths for a `tcp://` Docker host (mutual TLS).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostTls {
    /// CA certificate that signed the daemon's server cert.
    pub ca: PathBuf,
    /// Client certificate presented to the daemon.
    pub cert: PathBuf,
    /// Client private key.
    pub key: PathBuf,
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
            shutdown_grace_ms: 30_000,
            max_body_size: None,
            container_log_path: None,
            port: 8080,
            bind_address: "0.0.0.0".to_string(),
            authentication: AuthScheme::None,
            specs: Vec::new(),
            landing_customization: LandingCustomization::default(),
            metrics_enabled: false,
            hosts: Vec::new(),
        }
    }
}

impl Proxy {
    /// Global default max request-body size in bytes, parsed from
    /// `proxy.max-body-size`. `None` (unset or malformed) means no
    /// global default; the validator flags a malformed value.
    pub fn max_body_bytes(&self) -> Option<i64> {
        self.max_body_size.as_deref().and_then(parse_memory_string)
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
    ///     pt: "Bem-vindo ao portal..."
    ///     en: "Welcome to the portal..."
    /// ```
    #[serde(default, rename = "intro-locales")]
    pub intro_locales: std::collections::HashMap<String, String>,

    /// SEO / social-share meta tags for the public landing `<head>`.
    /// Optional page-title override; falls back to `proxy.title`.
    #[serde(default, rename = "seo-title")]
    pub seo_title: Option<String>,

    /// `<meta name="description">` and `og:description`. When empty,
    /// the rendered head falls back to the resolved intro text.
    #[serde(default, rename = "seo-description")]
    pub seo_description: Option<String>,

    /// `og:image` / social-share image — a path (`/assets/img/og.png`)
    /// or an absolute URL. Omitted from the head when unset.
    #[serde(default, rename = "og-image")]
    pub og_image: Option<String>,

    /// Raw analytics snippet (e.g. a Plausible / Matomo / GA
    /// `<script>` tag) injected verbatim into the public landing
    /// `<head>`. Operator-authored and **admin-trusted** — rendered
    /// unescaped, so only set it from a trusted source. External
    /// scripts also need their origin listed in
    /// [`Self::analytics_origins`] (the landing CSP blocks others).
    #[serde(default, rename = "analytics-html")]
    pub analytics_html: Option<String>,

    /// Space-separated origins (e.g. `https://plausible.io`) added to
    /// the landing's CSP `script-src` / `connect-src` / `img-src` so
    /// the analytics script can load and report. Only widens the
    /// public landing's policy, not the admin's.
    #[serde(default, rename = "analytics-origins")]
    pub analytics_origins: Option<String>,

    /// Custom HTML blocks rendered in the landing slots (`top` /
    /// `bottom`). Admin-managed; this field exists so they survive
    /// `import` / `export` (YAML round-trip). Array order within a
    /// slot is the render order.
    #[serde(default)]
    pub blocks: Vec<LandingBlock>,

    /// Whether the public landing shows the "Sign in" entrance to
    /// `/admin/login` for **anonymous** visitors (#156). Defaults to
    /// `true` (the sign-in affordance from #155). Set `false` on a
    /// fully-public, no-login portal to hide the admin entrance
    /// entirely — logged-in users still see their panel link. A
    /// deploy-level policy, so it lives in YAML config rather than the
    /// live landing editor.
    #[serde(default, rename = "show-admin-link")]
    pub show_admin_link: Option<bool>,
}

impl LandingCustomization {
    /// Resolve [`Self::show_admin_link`] — defaults to `true` so the
    /// landing keeps the anonymous "Sign in" entrance unless an
    /// operator opts out.
    pub fn effective_show_admin_link(&self) -> bool {
        self.show_admin_link.unwrap_or(true)
    }
}

/// A custom HTML block on the public landing (admin-authored,
/// rendered verbatim). See [`LandingCustomization::blocks`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LandingBlock {
    /// Where it renders: `top` (after the header) or `bottom` (after
    /// the card grid).
    pub slot: String,
    /// Internal label shown in the admin list (not rendered publicly).
    pub title: String,
    /// The HTML, injected verbatim — admin-trusted.
    pub html: String,
    /// Space-separated CSP origins this block's content needs.
    #[serde(rename = "csp-origins")]
    pub csp_origins: String,
    /// Whether it renders. A block with no `enabled:` key defaults to
    /// enabled (the common case when authoring YAML).
    #[serde(default = "default_true")]
    pub enabled: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum AuthScheme {
    #[default]
    None,
    #[serde(rename = "openid")]
    OpenId,
    Ldap,
    Saml,
    Simple,
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

    /// Port the app listens on **inside** the container. Needed for
    /// frameworks that don't use Shiny's 3838 default — Streamlit
    /// (8501), Dash (8050), Bokeh/Panel (5006), Voilà (8866), … .
    /// Accepts ShinyProxy's `port:` as an alias for friction-free
    /// migration. Resolved via [`Spec::effective_inner_port`]; when
    /// unset the runtime falls back to `api.port`, then a kind default
    /// (8080 for APIs), then the backend's 3838.
    #[serde(rename = "container-port", alias = "port")]
    pub container_port: Option<u16>,

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

    /// Ruscker extension: name of a credential stored in the
    /// admin's encrypted credential store. When set, the runtime
    /// resolves username/password/registry from the DB (decrypted
    /// with the master key) at pull time, taking precedence over
    /// the inline `docker-registry-*` fields above. Lets operators
    /// keep secrets out of YAML entirely — reference a named
    /// credential instead.
    #[serde(rename = "docker-registry-credential")]
    pub docker_registry_credential: Option<String>,

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

    /// Per-spec override of [`Proxy::max_body_size`]. Same Docker-
    /// style size format (`"10m"`, plain bytes, …). Takes precedence
    /// over the global default for this spec's proxied requests.
    /// Resolved via [`Spec::effective_max_body_bytes`].
    #[serde(rename = "max-body-size")]
    pub max_body_size: Option<String>,

    /// Bind-mount volumes for the container, in Docker syntax —
    /// `"/host:/container"` or `"/host:/container:ro"`. ShinyProxy-
    /// compatible (`volumes:` is a list of such strings). The local
    /// Docker backend maps these to `HostConfig.binds`. Bind-mounting
    /// host paths is powerful and admin-only — see SECURITY.md.
    #[serde(default)]
    pub volumes: Option<Vec<String>>,

    /// Groups allowed to see and reach this app (ShinyProxy
    /// `access-groups`). A spec with neither `access-groups` nor
    /// `access-users` is **open** (visible to everyone, including
    /// anonymous visitors). Otherwise only matching logged-in users —
    /// and admins always — may see it on the landing and reach `/app`.
    #[serde(rename = "access-groups", default)]
    pub access_groups: Option<Vec<String>>,

    /// Individual usernames allowed to see and reach this app
    /// (ShinyProxy `access-users`). See [`Spec::access_groups`].
    #[serde(rename = "access-users", default)]
    pub access_users: Option<Vec<String>>,

    /// Free-form properties consumed by the landing page template.
    /// Common keys: `logo`, `icon`, `type`, `updated`, `state`, `link`.
    #[serde(rename = "template-properties", default)]
    pub template_properties: TemplateProperties,

    // -- Ruscker extensions: spec type --
    /// Spec type. When absent:
    ///
    /// - `App` if `container_image` is set
    /// - `External` otherwise
    ///
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

    // -- Ruscker extensions: multi-host placement (Phase 6) --
    /// How to place this spec's replicas across `proxy.hosts`:
    /// `spread` (default — distribute for fault isolation) or
    /// `bin-pack` (fill a host before using the next). Only matters
    /// with multiple hosts configured.
    pub placement: Option<Placement>,

    /// Keep replicas of this spec on **distinct** hosts when possible
    /// (anti-affinity). Defaults to `false`. Best-effort: if every
    /// eligible host already runs the spec, placement falls back to the
    /// strategy above rather than refusing to scale.
    #[serde(rename = "anti-affinity")]
    pub anti_affinity: Option<bool>,

    // -- Ruscker extensions: smart routing --
    /// Whether to inject `<base href>` and rewrite root-relative URLs
    /// in HTML responses on the `/app/{spec}` route — the transform
    /// that lets an app whose templates assume they live at `/`
    /// work behind Ruscker's sub-path mount. Defaults to `true`.
    ///
    /// Set `false` when the upstream app honours the smart-routing
    /// headers Ruscker always forwards (`X-Forwarded-Prefix` /
    /// `X-Script-Name`, plus `X-Forwarded-Proto` / `-Host`) and
    /// builds its own base path — then the HTML rewriting is
    /// redundant (and can interfere). Has no effect on `/api/{spec}`
    /// responses, which are never rewritten. Resolved via
    /// [`Spec::effective_inject_base_href`].
    #[serde(rename = "inject-base-href")]
    pub inject_base_href: Option<bool>,
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

    /// Whether this app is **open** — no access list, so visible and
    /// reachable by everyone (including anonymous visitors). True when
    /// both `access-groups` and `access-users` are absent or empty.
    pub fn is_open(&self) -> bool {
        self.access_groups.as_deref().is_none_or(<[String]>::is_empty)
            && self.access_users.as_deref().is_none_or(<[String]>::is_empty)
    }

    /// Whether a viewer may see and reach this app (Phase 8 per-app
    /// access). Pure rule, shared by the landing filter and the `/app`
    /// /`/api` enforcement guard so they can't drift:
    ///
    /// - **open** spec → always;
    /// - **admin** (`is_admin`) → always;
    /// - **logged-in** user → their `username` is in `access-users`, or
    ///   any of their `groups` is in `access-groups`;
    /// - **anonymous** (`username: None`) → only open specs.
    pub fn access_allows(&self, is_admin: bool, username: Option<&str>, groups: &[String]) -> bool {
        if is_admin || self.is_open() {
            return true;
        }
        if let Some(name) = username {
            if self
                .access_users
                .as_deref()
                .is_some_and(|us| us.iter().any(|u| u == name))
            {
                return true;
            }
        }
        self.access_groups
            .as_deref()
            .is_some_and(|gs| gs.iter().any(|g| groups.iter().any(|h| h == g)))
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

    /// Multi-host placement strategy (default [`Placement::Spread`]).
    pub fn effective_placement(&self) -> Placement {
        self.placement.unwrap_or_default()
    }

    /// Whether replicas should prefer distinct hosts (default false).
    pub fn effective_anti_affinity(&self) -> bool {
        self.anti_affinity.unwrap_or(false)
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

    /// Effective max request-body size in bytes for this spec's
    /// proxied requests: the spec's own `max-body-size` if set,
    /// otherwise the `global` default (already parsed from
    /// `proxy.max-body-size`). A malformed spec value falls back to
    /// the global — the validator flags the typo separately.
    /// `None` means no limit.
    pub fn effective_max_body_bytes(&self, global: Option<i64>) -> Option<i64> {
        self.max_body_size
            .as_deref()
            .and_then(parse_memory_string)
            .or(global)
    }

    /// Whether the `/app/{spec}` HTML transform (`<base href>` +
    /// root-relative URL rewriting) runs for this spec. Defaults to
    /// `true` — the historical behaviour, safe for apps that don't
    /// understand the forwarded-prefix headers. Operators opt out
    /// per spec once the upstream self-configures from
    /// `X-Forwarded-Prefix`.
    pub fn effective_inject_base_href(&self) -> bool {
        self.inject_base_href.unwrap_or(true)
    }

    /// The port to forward to **inside** the container, or `None` to
    /// let the backend use its default (3838, the Shiny Server port).
    ///
    /// Precedence: explicit `container-port` (or ShinyProxy `port`) →
    /// `api.port` → a per-kind default (8080 for APIs) → `None`
    /// (backend default). Centralises what the proxy + scaler used to
    /// resolve by hand.
    pub fn effective_inner_port(&self) -> Option<u16> {
        self.container_port
            .or_else(|| self.api.as_ref().and_then(|a| a.port))
            .or_else(|| match self.kind() {
                SpecKind::Api => Some(8080),
                _ => None,
            })
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
/// Whether `s` parses as a Docker-style memory size (`"512m"`, `"1.5g"`,
/// plain bytes). Public so the admin form can validate the field with
/// the same rules the runtime uses, instead of silently accepting a
/// typo like `512mb`.
pub fn is_valid_memory_size(s: &str) -> bool {
    parse_memory_string(s).is_some()
}

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

#[cfg(test)]
mod max_body_size_tests {
    use super::*;

    fn spec_with_max(max: Option<&str>) -> Spec {
        let mut s: Spec =
            serde_yaml_ng::from_str("id: x\ncontainer-image: a:1\n").expect("parse");
        s.max_body_size = max.map(|m| m.to_string());
        s
    }

    #[test]
    fn spec_override_wins_over_global() {
        let s = spec_with_max(Some("10m"));
        assert_eq!(
            s.effective_max_body_bytes(Some(1024)),
            Some(10 * 1024 * 1024)
        );
    }

    #[test]
    fn falls_back_to_global_when_spec_unset() {
        let s = spec_with_max(None);
        assert_eq!(s.effective_max_body_bytes(Some(2048)), Some(2048));
    }

    #[test]
    fn none_when_neither_set() {
        let s = spec_with_max(None);
        assert_eq!(s.effective_max_body_bytes(None), None);
    }

    #[test]
    fn malformed_spec_value_falls_back_to_global() {
        // A typo in the spec value isn't a hard error — it falls
        // back to the global default (and is flagged by validate).
        let s = spec_with_max(Some("500frogs"));
        assert_eq!(s.effective_max_body_bytes(Some(4096)), Some(4096));
    }

    #[test]
    fn proxy_global_parses() {
        let cfg: Config =
            serde_yaml_ng::from_str("proxy:\n  max-body-size: 1g\n").expect("parse");
        assert_eq!(cfg.proxy.max_body_bytes(), Some(1024 * 1024 * 1024));
    }

    #[test]
    fn proxy_global_malformed_is_none() {
        let cfg: Config =
            serde_yaml_ng::from_str("proxy:\n  max-body-size: huge\n").expect("parse");
        assert_eq!(cfg.proxy.max_body_bytes(), None);
        // ...but the field round-trips so the validator can flag it.
        assert_eq!(cfg.proxy.max_body_size.as_deref(), Some("huge"));
    }
}

#[cfg(test)]
mod parse_rate_limit_tests {
    use super::{parse_rate_limit, RatePolicy};
    use std::time::Duration;

    #[test]
    fn canonical_forms() {
        assert_eq!(
            parse_rate_limit("100/min"),
            Some(RatePolicy {
                max: 100,
                window: Duration::from_secs(60)
            })
        );
        assert_eq!(
            parse_rate_limit("5/s"),
            Some(RatePolicy {
                max: 5,
                window: Duration::from_secs(1)
            })
        );
        assert_eq!(
            parse_rate_limit("1000/hour"),
            Some(RatePolicy {
                max: 1000,
                window: Duration::from_secs(3600)
            })
        );
    }

    #[test]
    fn unit_aliases_and_case() {
        for unit in ["s", "sec", "secs", "second", "seconds", "S", "SEC"] {
            assert_eq!(
                parse_rate_limit(&format!("3/{unit}")).map(|p| p.window),
                Some(Duration::from_secs(1)),
                "unit `{unit}`"
            );
        }
        for unit in ["m", "min", "mins", "minute", "minutes", "Min"] {
            assert_eq!(
                parse_rate_limit(&format!("3/{unit}")).map(|p| p.window),
                Some(Duration::from_secs(60)),
                "unit `{unit}`"
            );
        }
        for unit in ["h", "hr", "hrs", "hour", "hours", "HOUR"] {
            assert_eq!(
                parse_rate_limit(&format!("3/{unit}")).map(|p| p.window),
                Some(Duration::from_secs(3600)),
                "unit `{unit}`"
            );
        }
    }

    #[test]
    fn whitespace_tolerated() {
        assert_eq!(
            parse_rate_limit("  100 / min  "),
            Some(RatePolicy {
                max: 100,
                window: Duration::from_secs(60)
            })
        );
    }

    #[test]
    fn rejects_garbage() {
        assert_eq!(parse_rate_limit(""), None);
        assert_eq!(parse_rate_limit("100"), None); // no unit
        assert_eq!(parse_rate_limit("/min"), None); // no count
        assert_eq!(parse_rate_limit("0/min"), None); // zero is not a limit
        assert_eq!(parse_rate_limit("-5/min"), None); // negative
        assert_eq!(parse_rate_limit("ten/min"), None); // non-numeric
        assert_eq!(parse_rate_limit("100/fortnight"), None); // unknown unit
        assert_eq!(parse_rate_limit("100/"), None); // empty unit
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

impl ApiSpec {
    /// Parsed `rate-limit` policy, or `None` if unset or malformed.
    ///
    /// Returning `None` for a malformed value means the proxy
    /// silently applies *no* limit rather than failing the request;
    /// the operator is told about the typo separately via
    /// [`crate::validate`] (`Warning::InvalidRateLimit`), which is
    /// the right place to surface authoring mistakes.
    pub fn rate_policy(&self) -> Option<RatePolicy> {
        self.rate_limit.as_deref().and_then(parse_rate_limit)
    }
}

/// A parsed proxy-side rate limit: at most `max` requests per
/// `window`. Produced from the `api.rate-limit` string (e.g.
/// `100/min`) by [`parse_rate_limit`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RatePolicy {
    /// Maximum number of requests allowed within `window`.
    pub max: u32,
    /// The sliding window the `max` count applies over.
    pub window: std::time::Duration,
}

/// Parse a `"N/unit"` rate-limit string into a [`RatePolicy`].
///
/// The format mirrors what operators expect from API gateways:
/// a positive integer, a slash, and a time unit. Accepted units
/// (case-insensitive, with common aliases):
///
/// - `s`, `sec`, `secs`, `second`, `seconds`
/// - `m`, `min`, `mins`, `minute`, `minutes`
/// - `h`, `hr`, `hrs`, `hour`, `hours`
///
/// Whitespace around each part is tolerated (`"100 / min"` works).
/// Returns `None` for anything malformed — an empty count, a zero
/// or negative count, an unknown unit, or a missing slash — so the
/// caller can treat "no valid limit" uniformly. The validator
/// re-checks and warns so the operator learns about the typo.
pub fn parse_rate_limit(raw: &str) -> Option<RatePolicy> {
    use std::time::Duration;
    let (count, unit) = raw.split_once('/')?;
    let max: u32 = count.trim().parse().ok()?;
    if max == 0 {
        return None;
    }
    let window = match unit.trim().to_ascii_lowercase().as_str() {
        "s" | "sec" | "secs" | "second" | "seconds" => Duration::from_secs(1),
        "m" | "min" | "mins" | "minute" | "minutes" => Duration::from_secs(60),
        "h" | "hr" | "hrs" | "hour" | "hours" => Duration::from_secs(3600),
        _ => return None,
    };
    Some(RatePolicy { max, window })
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

/// How to place a spec's replicas across multiple Docker hosts
/// (Phase 6). Default [`Placement::Spread`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Placement {
    /// Distribute replicas across hosts (weighted by `host.weight`) for
    /// fault isolation. The default.
    #[default]
    Spread,
    /// Fill one host before using the next (better density / cost).
    BinPack,
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
    fn parses_landing_seo_fields() {
        let yaml = r#"
proxy:
  landing-customization:
    seo-title: My Portal
    seo-description: A short summary
    og-image: /assets/img/og.png
"#;
        let config: Config = serde_yaml_ng::from_str(yaml).unwrap();
        let lc = &config.proxy.landing_customization;
        assert_eq!(lc.seo_title.as_deref(), Some("My Portal"));
        assert_eq!(lc.seo_description.as_deref(), Some("A short summary"));
        assert_eq!(lc.og_image.as_deref(), Some("/assets/img/og.png"));
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
            docker_registry_credential: None,
            container_cpu_limit: None,
            container_cpu_request: None,
            container_memory_limit: None,
            container_memory_request: None,
            max_body_size: None,
            volumes: None,
            access_groups: None,
            access_users: None,
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
            inject_base_href: None,
            container_port: None,
            placement: None,
            anti_affinity: None,
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
            docker_registry_credential: None,
            container_cpu_limit: None,
            container_cpu_request: None,
            container_memory_limit: None,
            container_memory_request: None,
            max_body_size: None,
            volumes: None,
            access_groups: None,
            access_users: None,
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
            inject_base_href: None,
            container_port: None,
            placement: None,
            anti_affinity: None,
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
            docker_registry_credential: None,
            container_cpu_limit: None,
            container_cpu_request: None,
            container_memory_limit: None,
            container_memory_request: None,
            max_body_size: None,
            volumes: None,
            access_groups: None,
            access_users: None,
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
            inject_base_href: None,
            container_port: None,
            placement: None,
            anti_affinity: None,
            concurrent_requests_per_replica: None,
        };
        spec.kind_override = Some(SpecKindOverride::Api);
        assert_eq!(spec.kind(), SpecKind::Api);
        assert_eq!(spec.effective_routing(), RoutingStrategy::RoundRobin);
        assert!(!spec.needs_sticky_sessions());
    }

    fn parse_spec(yaml: &str) -> Spec {
        serde_yaml_ng::from_str(yaml).expect("parse spec")
    }

    #[test]
    fn access_open_when_no_acl() {
        let s = parse_spec("id: a\ncontainer-image: x");
        assert!(s.is_open());
        // open → everyone, including anonymous.
        assert!(s.access_allows(false, None, &[]));
        assert!(s.access_allows(false, Some("u"), &["g".into()]));
        // empty lists are also "open".
        let e = parse_spec("id: a\ncontainer-image: x\naccess-groups: []");
        assert!(e.is_open());
    }

    #[test]
    fn access_restricted_by_group() {
        let s = parse_spec("id: a\ncontainer-image: x\naccess-groups: [staff, ops]");
        assert!(!s.is_open());
        assert!(!s.access_allows(false, None, &[]), "anonymous denied");
        assert!(
            !s.access_allows(false, Some("u"), &["other".into()]),
            "non-matching group denied"
        );
        assert!(
            s.access_allows(false, Some("u"), &["ops".into()]),
            "matching group allowed"
        );
        assert!(s.access_allows(true, None, &[]), "admin always allowed");
    }

    #[test]
    fn access_restricted_by_user() {
        let s = parse_spec("id: a\ncontainer-image: x\naccess-users: [alice]");
        assert!(!s.is_open());
        assert!(s.access_allows(false, Some("alice"), &[]), "named user allowed");
        assert!(!s.access_allows(false, Some("bob"), &[]), "other user denied");
        assert!(!s.access_allows(false, None, &[]), "anonymous denied");
        assert!(s.access_allows(true, Some("bob"), &[]), "admin always allowed");
    }

    #[test]
    fn container_port_explicit_wins() {
        let s = parse_spec("id: st\ncontainer-image: x:1\ntype: streamlit\ncontainer-port: 8501\n");
        assert_eq!(s.container_port, Some(8501));
        assert_eq!(s.effective_inner_port(), Some(8501));
    }

    #[test]
    fn shinyproxy_port_aliases_to_container_port() {
        // ShinyProxy's spec-level `port:` migrates cleanly.
        let s = parse_spec("id: legacy\ncontainer-image: rocker/shiny\nport: 3838\n");
        assert_eq!(s.container_port, Some(3838));
        assert_eq!(s.effective_inner_port(), Some(3838));
    }

    #[test]
    fn effective_inner_port_precedence_and_defaults() {
        // container-port beats api.port.
        let s = parse_spec(
            "id: a\ncontainer-image: x:1\ntype: api\ncontainer-port: 9000\napi:\n  port: 8000\n",
        );
        assert_eq!(s.effective_inner_port(), Some(9000));
        // api.port when no container-port.
        let s = parse_spec("id: a\ncontainer-image: x:1\ntype: api\napi:\n  port: 8000\n");
        assert_eq!(s.effective_inner_port(), Some(8000));
        // API kind with nothing set → 8080 default.
        let s = parse_spec("id: a\ncontainer-image: x:1\ntype: api\n");
        assert_eq!(s.effective_inner_port(), Some(8080));
        // A Shiny-ish app with nothing set → None (backend default 3838).
        let s = parse_spec("id: a\ncontainer-image: x:1\ntype: shiny\n");
        assert_eq!(s.effective_inner_port(), None);
    }
}
