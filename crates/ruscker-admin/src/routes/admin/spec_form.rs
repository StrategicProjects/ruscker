//! Admin > Add/Edit spec form.
//!
//! Single template (`templates/admin/spec_form.html`) used for both
//! New and Edit, parameterized by `mode`. On submit, the form posts
//! to `/admin/specs` (create) or `/admin/specs/:id` (update); on
//! delete, to `/admin/specs/:id/delete`.
//!
//! Field scope covers what the SEPE YAML actually uses today
//! (~80% of specs). Phase 2.5 adds advanced fields (replicas,
//! scaling, env vars, volumes) behind a collapsible section.

use anyhow::Result;
use askama::Template;
use axum::{
    extract::{Form, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use chrono::Utc;
use ruscker_config::{
    ApiSpec, OrderedFloat, Placement, RoutingStrategy, Spec, SpecKindOverride, TemplateProperties,
};
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value as YamlValue;
use std::collections::HashMap;

use crate::auth::{RequireEditor, Role};
use crate::db;
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::view_model::DisplayType;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/specs/new", get(new_form))
        .route("/admin/specs", post(create))
        .route(
            "/admin/specs/{id}/edit",
            get(edit_form),
        )
        .route("/admin/specs/{id}", post(update))
        .route("/admin/specs/{id}/delete", post(delete))
}

// ── Form payload ────────────────────────────────────────────────

/// Mirror of the form fields. Strings are unconditional so empty
/// inputs round-trip as `""` rather than disappearing; conversion
/// to [`Spec`] handles "empty means None".
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SpecForm {
    pub id: String,
    pub display_name: String,
    pub description: String,
    /// "app" | "talk" | "report" | "package" | "api" | "link"
    pub display_type: String,
    pub container_image: String,
    /// "active" | "inactive"
    pub state: String,
    /// "lock" (restricted) | "lock_open" (public)
    pub access: String,
    pub subject: String,
    pub logo: String,
    /// Card-cover CSS background (`template-properties.cover`):
    /// a solid color or a gradient string. Empty ⇒ fall back to
    /// the per-kind tint. Not validated server-side (browser
    /// fail-softs), same policy as `landing_customization.header_bg`.
    #[serde(default)]
    pub cover: String,
    /// Updated date in DD/MM/YYYY. Empty ⇒ stamp with today.
    pub updated: String,
    /// External link target (for type=link/package).
    pub link: String,
    pub seats_per_container: String,
    pub max_lifetime: String,

    // ── Advanced (collapsible). Empty string ⇒ keep the schema
    //    default; nothing here is required. ──────────────────────
    /// `heartbeat-timeout` override in ms; `-1` = never expire.
    pub heartbeat_timeout: String,
    /// Fractional CPUs, e.g. `0.5` (`container-cpu-limit`).
    pub container_cpu_limit: String,
    /// Memory cap, e.g. `512m` / `1.5g` (`container-memory-limit`).
    pub container_memory_limit: String,
    /// Replica pool floor / ceiling (`min`/`max-replicas`).
    pub min_replicas: String,
    pub max_replicas: String,
    /// API: concurrent requests a replica handles before scale-up.
    pub concurrent_requests_per_replica: String,
    /// Bind-mount volumes, one `"/host:/container[:ro]"` per line.
    pub volumes: String,
    /// API sub-fields (only meaningful for `type: api`).
    pub api_port: String,
    pub api_docs_path: String,
    pub api_health_path: String,
    pub api_rate_limit: String,
    /// Checkbox: non-empty ("on") ⇒ permissive CORS enabled.
    pub api_cors: String,
    /// Checkbox: non-empty ("on") ⇒ inject `<base href>` + rewrite
    /// root-relative URLs in `/app/{spec}` HTML (the default). Empty
    /// (unchecked) ⇒ the app self-routes from the forwarded-prefix
    /// headers, so the HTML transform is turned off. Defaults to
    /// checked on a new form.
    pub inject_base_href: String,

    // ── Advanced · Runtime ──────────────────────────────────────
    /// Inner port the app listens on (`container-port`). Blank ⇒
    /// per-kind default (3838 Shiny). Needed for Streamlit (8501),
    /// Dash (8050), Jupyter (8888), …
    pub container_port: String,
    /// Docker `--platform` (`linux/amd64`) for emulated images.
    pub platform: String,
    /// Soft lifetime cap in minutes (`container-lifetime`).
    pub container_lifetime: String,
    /// Checkbox ("on") ⇒ stop the container when the user logs out.
    pub stop_on_logout: String,

    // ── Advanced · Environment + command ────────────────────────
    /// `container-env`, one `NAME=value` per line.
    pub container_env: String,
    /// `container-cmd`, one argv token per line.
    pub container_cmd: String,

    // ── Advanced · Registry (private images) ────────────────────
    pub docker_registry_domain: String,
    pub docker_registry_username: String,
    pub docker_registry_password: String,
    /// Name of a stored credential (credentials store) to use instead
    /// of the inline username/password above.
    pub docker_registry_credential: String,

    // ── Advanced · Access (per-app, Phase 6 / #155) ─────────────
    /// Groups allowed to see + reach this app — comma- or newline-
    /// separated. Blank (and `access-users` blank) ⇒ open to everyone.
    pub access_groups: String,
    /// Usernames allowed to see + reach this app — comma/newline list.
    pub access_users: String,

    // ── Advanced · Resources (requests + body cap) ──────────────
    /// Soft CPU reservation in fractional cores (`container-cpu-request`).
    pub container_cpu_request: String,
    /// Soft memory reservation (`container-memory-request`), e.g. `256m`.
    pub container_memory_request: String,
    /// Per-spec proxied-body cap (`max-body-size`), e.g. `10m`.
    pub max_body_size: String,

    // ── Advanced · Scaling thresholds ───────────────────────────
    /// Utilization fraction (0–1) that triggers scale-up.
    pub scale_up_threshold: String,
    /// Utilization fraction (0–1) below which a replica is reaped.
    pub scale_down_threshold: String,
    /// Seconds below `scale-down-threshold` before reaping.
    pub scale_down_grace: String,
    /// Seconds to drain a replica's sessions before stopping it.
    pub drain_timeout: String,

    // ── Advanced · Routing + multi-host placement ───────────────
    /// `routing-strategy`: ""(default) | least-connections |
    /// round-robin | weighted-random | resource-aware.
    pub routing_strategy: String,
    /// `placement`: ""(default=spread) | spread | bin-pack.
    pub placement: String,
    /// Checkbox ("on") ⇒ prefer distinct hosts for this spec's replicas.
    pub anti_affinity: String,
}

impl SpecForm {
    /// Build a form pre-filled from an existing [`Spec`] for the
    /// edit view.
    pub fn from_spec(spec: &Spec) -> Self {
        let tp = &spec.template_properties;
        let dt = DisplayType::from_spec(spec);
        Self {
            id: spec.id.clone(),
            display_name: spec.display_name.clone().unwrap_or_default(),
            description: spec.description.clone().unwrap_or_default(),
            display_type: dt.key().to_string(),
            container_image: spec.container_image.clone().unwrap_or_default(),
            state: tp
                .get_str("state")
                .map(str::to_string)
                .unwrap_or_else(|| "active".into()),
            access: tp
                .get_str("icon")
                .map(str::to_string)
                .unwrap_or_else(|| "lock".into()),
            subject: tp.get_str("subject").map(str::to_string).unwrap_or_default(),
            logo: tp.get_str("logo").map(str::to_string).unwrap_or_default(),
            cover: tp.get_str("cover").map(str::to_string).unwrap_or_default(),
            updated: tp.get_str("updated").map(str::to_string).unwrap_or_default(),
            link: tp.get_str("link").map(str::to_string).unwrap_or_default(),
            seats_per_container: spec
                .seats_per_container
                .map(|n| n.to_string())
                .unwrap_or_default(),
            max_lifetime: spec.max_lifetime.map(|n| n.to_string()).unwrap_or_default(),
            heartbeat_timeout: spec
                .heartbeat_timeout
                .map(|n| n.to_string())
                .unwrap_or_default(),
            container_cpu_limit: spec
                .container_cpu_limit
                .map(|n| n.to_string())
                .unwrap_or_default(),
            container_memory_limit: spec.container_memory_limit.clone().unwrap_or_default(),
            min_replicas: spec.min_replicas.map(|n| n.to_string()).unwrap_or_default(),
            max_replicas: spec.max_replicas.map(|n| n.to_string()).unwrap_or_default(),
            concurrent_requests_per_replica: spec
                .concurrent_requests_per_replica
                .map(|n| n.to_string())
                .unwrap_or_default(),
            volumes: spec
                .volumes
                .as_ref()
                .map(|v| v.join("\n"))
                .unwrap_or_default(),
            api_port: spec
                .api
                .as_ref()
                .and_then(|a| a.port)
                .map(|n| n.to_string())
                .unwrap_or_default(),
            api_docs_path: spec
                .api
                .as_ref()
                .and_then(|a| a.docs_path.clone())
                .unwrap_or_default(),
            api_health_path: spec
                .api
                .as_ref()
                .and_then(|a| a.health_path.clone())
                .unwrap_or_default(),
            api_rate_limit: spec
                .api
                .as_ref()
                .and_then(|a| a.rate_limit.clone())
                .unwrap_or_default(),
            api_cors: if spec.api.as_ref().map(|a| a.cors).unwrap_or(false) {
                "on".into()
            } else {
                String::new()
            },
            inject_base_href: if spec.effective_inject_base_href() {
                "on".into()
            } else {
                String::new()
            },

            // ── Advanced (new) ──────────────────────────────────
            container_port: spec.container_port.map(|n| n.to_string()).unwrap_or_default(),
            platform: spec.platform.clone().unwrap_or_default(),
            container_lifetime: spec.container_lifetime.map(|n| n.to_string()).unwrap_or_default(),
            stop_on_logout: checkbox(spec.stop_on_logout.unwrap_or(false)),
            // `container-env` shown as sorted `NAME=value` lines.
            container_env: spec.env_pairs().join("\n"),
            container_cmd: spec
                .container_cmd
                .as_ref()
                .map(|v| v.join("\n"))
                .unwrap_or_default(),
            docker_registry_domain: spec.docker_registry_domain.clone().unwrap_or_default(),
            docker_registry_username: spec.docker_registry_username.clone().unwrap_or_default(),
            docker_registry_password: spec.docker_registry_password.clone().unwrap_or_default(),
            docker_registry_credential: spec.docker_registry_credential.clone().unwrap_or_default(),
            access_groups: spec.access_groups.as_ref().map(|v| v.join(", ")).unwrap_or_default(),
            access_users: spec.access_users.as_ref().map(|v| v.join(", ")).unwrap_or_default(),
            container_cpu_request: spec
                .container_cpu_request
                .map(|n| n.to_string())
                .unwrap_or_default(),
            container_memory_request: spec.container_memory_request.clone().unwrap_or_default(),
            max_body_size: spec.max_body_size.clone().unwrap_or_default(),
            scale_up_threshold: spec.scale_up_threshold.map(|f| f.0.to_string()).unwrap_or_default(),
            scale_down_threshold: spec
                .scale_down_threshold
                .map(|f| f.0.to_string())
                .unwrap_or_default(),
            scale_down_grace: spec.scale_down_grace.map(|n| n.to_string()).unwrap_or_default(),
            drain_timeout: spec.drain_timeout.map(|n| n.to_string()).unwrap_or_default(),
            routing_strategy: spec.routing_strategy.map(routing_to_key).unwrap_or_default(),
            placement: spec.placement.map(placement_to_key).unwrap_or_default(),
            anti_affinity: checkbox(spec.anti_affinity.unwrap_or(false)),
        }
    }

    /// Build a [`Spec`] from the submitted form, **merged onto `base`**
    /// (the existing spec, on edit). The form overwrites only the fields
    /// it owns; everything it doesn't model — `container-lifetime`,
    /// `docker-registry-*`, `*-request`, `max-body-size`, scaling
    /// thresholds, `routing-strategy`, `stop-on-logout`, and any custom
    /// `template-properties` keys — passes through from `base` instead of
    /// being silently dropped. `base` is `None` for a brand-new spec.
    /// Empty optional strings become `None`; numeric strings parse
    /// optimistically.
    pub fn into_spec(self, base: Option<&Spec>) -> Result<Spec> {
        let dt = DisplayType::parse(&self.display_type).unwrap_or(DisplayType::App);
        // App/API/External set an explicit kind; Talk/Report are purely
        // visual, so keep whatever run-kind override `base` carried.
        let kind_override = match dt {
            DisplayType::App => Some(SpecKindOverride::Shiny),
            DisplayType::Talk | DisplayType::Report => base.and_then(|b| b.kind_override),
            DisplayType::Package | DisplayType::Link => Some(SpecKindOverride::External),
            DisplayType::Api => Some(SpecKindOverride::Api),
        };

        let updated = if self.updated.trim().is_empty() {
            Utc::now().format("%d/%m/%Y").to_string()
        } else {
            self.updated.trim().to_string()
        };

        // Start from base so custom template-properties keys (anything
        // the form doesn't render) survive an edit, then overwrite the
        // managed keys. Empty managed fields are *removed* so the form
        // stays authoritative for the keys it owns.
        let mut tp_map: HashMap<String, YamlValue> = base
            .map(|b| b.template_properties.0.clone())
            .unwrap_or_default();
        // type, state, icon, updated are always set so chips/filters render
        tp_map.insert("type".into(), YamlValue::String(dt.key().to_string()));
        tp_map.insert("state".into(), YamlValue::String(self.state.clone()));
        tp_map.insert("icon".into(), YamlValue::String(self.access.clone()));
        tp_map.insert("updated".into(), YamlValue::String(updated));
        set_or_remove(&mut tp_map, "subject", &self.subject);
        set_or_remove(&mut tp_map, "logo", &self.logo);
        set_or_remove(&mut tp_map, "cover", &self.cover);
        set_or_remove(&mut tp_map, "link", &self.link);

        let container_image = match dt {
            DisplayType::Package | DisplayType::Link => None,
            _ => empty_to_none(&self.container_image),
        };

        // Advanced API block: built for API specs, or whenever any
        // API field was filled in (empty otherwise ⇒ schema defaults).
        let cors = !self.api_cors.trim().is_empty();
        let api_filled = cors
            || [
                &self.api_port,
                &self.api_docs_path,
                &self.api_health_path,
                &self.api_rate_limit,
            ]
            .iter()
            .any(|s| !s.trim().is_empty());
        let api = if matches!(dt, DisplayType::Api) || api_filled {
            Some(ApiSpec {
                port: parse_opt(&self.api_port),
                docs_path: empty_to_none(&self.api_docs_path),
                health_path: empty_to_none(&self.api_health_path),
                rate_limit: empty_to_none(&self.api_rate_limit),
                cors,
            })
        } else {
            None
        };

        Ok(Spec {
            id: self.id.trim().to_string(),
            display_name: empty_to_none(&self.display_name),
            description: empty_to_none(&self.description),
            container_image,
            seats_per_container: parse_opt(&self.seats_per_container),
            max_lifetime: parse_opt(&self.max_lifetime),
            heartbeat_timeout: parse_opt(&self.heartbeat_timeout),
            container_cpu_limit: parse_opt(&self.container_cpu_limit),
            container_memory_limit: empty_to_none(&self.container_memory_limit),
            template_properties: TemplateProperties(tp_map),
            kind_override,
            api,
            min_replicas: parse_opt(&self.min_replicas),
            max_replicas: parse_opt(&self.max_replicas),
            concurrent_requests_per_replica: parse_opt(&self.concurrent_requests_per_replica),
            volumes: lines_to_vec(&self.volumes),
            // Checked ⇒ leave unset (the `true` default keeps the
            // exported YAML clean); unchecked ⇒ explicit `false`.
            inject_base_href: if self.inject_base_href.trim().is_empty() {
                Some(false)
            } else {
                None
            },
            // ── Advanced fields: the form is authoritative now (it
            //    pre-fills from the spec in `from_spec`), so blank ⇒
            //    None ⇒ the schema/runtime default. Clearing a field
            //    clears it; an untouched field round-trips. ──────────
            container_port: parse_opt(&self.container_port),
            platform: empty_to_none(&self.platform),
            container_lifetime: parse_opt(&self.container_lifetime),
            stop_on_logout: checkbox_opt(&self.stop_on_logout),
            container_env: parse_env(&self.container_env),
            container_cmd: lines_to_vec(&self.container_cmd),
            docker_registry_domain: empty_to_none(&self.docker_registry_domain),
            docker_registry_username: empty_to_none(&self.docker_registry_username),
            docker_registry_password: empty_to_none(&self.docker_registry_password),
            docker_registry_credential: empty_to_none(&self.docker_registry_credential),
            access_groups: list_to_vec(&self.access_groups),
            access_users: list_to_vec(&self.access_users),
            container_cpu_request: parse_opt(&self.container_cpu_request),
            container_memory_request: empty_to_none(&self.container_memory_request),
            max_body_size: empty_to_none(&self.max_body_size),
            scale_up_threshold: parse_opt::<f64>(&self.scale_up_threshold).map(OrderedFloat),
            scale_down_threshold: parse_opt::<f64>(&self.scale_down_threshold).map(OrderedFloat),
            scale_down_grace: parse_opt(&self.scale_down_grace),
            drain_timeout: parse_opt(&self.drain_timeout),
            routing_strategy: routing_from_key(&self.routing_strategy),
            placement: placement_from_key(&self.placement),
            anti_affinity: checkbox_opt(&self.anti_affinity),
        })
    }

    /// Server-side validation. Returns a list of fluent message
    /// keys describing each problem; empty list = OK.
    pub fn validate(&self, mode: FormMode) -> Vec<&'static str> {
        let mut errs = Vec::new();
        if self.id.trim().is_empty() {
            errs.push("spec-form-error-id-required");
        } else if !is_kebab_id(self.id.trim()) {
            errs.push("spec-form-error-id-shape");
        }
        if self.display_name.trim().is_empty() {
            errs.push("spec-form-error-name-required");
        }
        if matches!(mode, FormMode::New) && self.id.trim().is_empty() {
            // duplicate-id check happens later (needs DB access)
        }

        // Numeric fields: a non-empty but unparseable value used to be
        // silently dropped to None (= schema default). Flag it instead,
        // so a pt-BR `0,5` CPU or a typo'd count doesn't quietly mean
        // "no limit / default".
        let int_fields = [
            &self.seats_per_container,
            &self.max_lifetime,
            &self.heartbeat_timeout,
            &self.min_replicas,
            &self.max_replicas,
            &self.concurrent_requests_per_replica,
            &self.api_port,
            &self.container_port,
            &self.container_lifetime,
            &self.scale_down_grace,
            &self.drain_timeout,
        ];
        if int_fields
            .iter()
            .any(|v| !v.trim().is_empty() && v.trim().parse::<i64>().is_err())
        {
            errs.push("spec-form-error-number");
        }
        // Ports are 1–65535.
        for p in [&self.container_port, &self.api_port] {
            if !p.trim().is_empty()
                && !matches!(p.trim().parse::<u32>(), Ok(n) if (1..=65535).contains(&n))
            {
                errs.push("spec-form-error-port");
                break;
            }
        }

        // CPU (limit + request) must be a positive, finite number of
        // cores (catches `0,5`).
        for cpu in [&self.container_cpu_limit, &self.container_cpu_request] {
            if !cpu.trim().is_empty()
                && !matches!(cpu.trim().parse::<f64>(), Ok(v) if v.is_finite() && v > 0.0)
            {
                errs.push("spec-form-error-cpu");
                break;
            }
        }

        // Memory-style sizes: limit, request, and the per-spec body cap.
        for mem in [
            &self.container_memory_limit,
            &self.container_memory_request,
            &self.max_body_size,
        ] {
            if !mem.trim().is_empty() && !ruscker_config::is_valid_memory_size(mem.trim()) {
                errs.push("spec-form-error-memory");
                break;
            }
        }

        // Scaling thresholds are utilization fractions in (0, 1].
        for th in [&self.scale_up_threshold, &self.scale_down_threshold] {
            if !th.trim().is_empty()
                && !matches!(th.trim().parse::<f64>(), Ok(v) if v.is_finite() && v > 0.0 && v <= 1.0)
            {
                errs.push("spec-form-error-threshold");
                break;
            }
        }

        // Replica pool: max must be >= min when both are given.
        if let (Ok(min), Ok(max)) = (
            self.min_replicas.trim().parse::<u32>(),
            self.max_replicas.trim().parse::<u32>(),
        ) {
            if max < min {
                errs.push("spec-form-error-replica-range");
            }
        }

        // Each volume line must be valid Docker bind syntax.
        if self
            .volumes
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .any(|l| !ruscker_config::is_valid_volume_bind(l))
        {
            errs.push("spec-form-error-volume");
        }

        errs
    }
}

fn empty_to_none(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}
/// Set a managed template-property to the trimmed value, or remove the
/// key entirely when the form left it blank (so clearing a field clears
/// it, while unmanaged keys merged from the base stay put).
fn set_or_remove(map: &mut HashMap<String, YamlValue>, key: &str, val: &str) {
    let t = val.trim();
    if t.is_empty() {
        map.remove(key);
    } else {
        map.insert(key.to_string(), YamlValue::String(t.to_string()));
    }
}
fn parse_opt<T: std::str::FromStr>(s: &str) -> Option<T> {
    s.trim().parse().ok()
}
/// Split a textarea into trimmed, non-empty lines — `None` if all blank.
/// Used for the volumes field (one `host:container[:ro]` per line).
fn lines_to_vec(s: &str) -> Option<Vec<String>> {
    let v: Vec<String> = s
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}
/// Checkbox value for `from_spec`: `"on"` when set, else empty.
fn checkbox(on: bool) -> String {
    if on { "on".into() } else { String::new() }
}
/// Checkbox parse: non-empty (`"on"`) ⇒ `Some(true)`; blank ⇒ `None`
/// (the default-`false` semantics, kept out of the exported YAML).
fn checkbox_opt(s: &str) -> Option<bool> {
    if s.trim().is_empty() { None } else { Some(true) }
}
/// Parse `container-env` from a textarea: one `NAME=value` per line.
/// The first `=` splits; blank lines and lines without `=` are skipped.
/// `BTreeMap` so the resulting env list is deterministically ordered.
/// `None` when nothing valid is present.
fn parse_env(s: &str) -> Option<BTreeMap<String, String>> {
    let map: BTreeMap<String, String> = s
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            let k = k.trim();
            if k.is_empty() {
                return None;
            }
            Some((k.to_string(), v.trim().to_string()))
        })
        .collect();
    if map.is_empty() { None } else { Some(map) }
}
/// Parse an access list (`access-groups` / `access-users`) from a field
/// that accepts commas and/or newlines. `None` when all blank.
fn list_to_vec(s: &str) -> Option<Vec<String>> {
    let v: Vec<String> = s
        .split([',', '\n'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect();
    if v.is_empty() { None } else { Some(v) }
}
/// `routing-strategy` ↔ form key (the enum's kebab serde value).
fn routing_from_key(s: &str) -> Option<RoutingStrategy> {
    match s.trim() {
        "least-connections" => Some(RoutingStrategy::LeastConnections),
        "round-robin" => Some(RoutingStrategy::RoundRobin),
        "weighted-random" => Some(RoutingStrategy::WeightedRandom),
        "resource-aware" => Some(RoutingStrategy::ResourceAware),
        _ => None,
    }
}
fn routing_to_key(r: RoutingStrategy) -> String {
    match r {
        RoutingStrategy::LeastConnections => "least-connections",
        RoutingStrategy::RoundRobin => "round-robin",
        RoutingStrategy::WeightedRandom => "weighted-random",
        RoutingStrategy::ResourceAware => "resource-aware",
    }
    .into()
}
/// `placement` ↔ form key (the enum's kebab serde value).
fn placement_from_key(s: &str) -> Option<Placement> {
    match s.trim() {
        "spread" => Some(Placement::Spread),
        "bin-pack" => Some(Placement::BinPack),
        _ => None,
    }
}
fn placement_to_key(p: Placement) -> String {
    match p {
        Placement::Spread => "spread",
        Placement::BinPack => "bin-pack",
    }
    .into()
}
fn is_kebab_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && s.chars()
            .next()
            .map(|c| c.is_ascii_alphabetic())
            .unwrap_or(false)
}

// ── Template ────────────────────────────────────────────────────

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FormMode {
    New,
    Edit,
}
impl FormMode {
    pub fn is_new(self) -> bool {
        matches!(self, FormMode::New)
    }
    pub fn is_edit(self) -> bool {
        matches!(self, FormMode::Edit)
    }
}

#[derive(Template)]
#[template(path = "admin/spec_form.html")]
struct SpecFormPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    nav_section: &'static str,
    /// Current session role (Editor or Admin) — drives nav gating.
    role: Role,
    mode: FormMode,
    form: SpecForm,
    /// Pre-validation errors (Fluent keys) shown above the form.
    errors: Vec<&'static str>,
    /// Filenames in the media library, for the logo picker. Empty
    /// when no DB is wired or the listing fails.
    logo_images: Vec<String>,
}

impl<'a> SpecFormPage<'a> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// JSON-encoded initial form values, ready to drop into the
    /// `x-data` attribute of the live-preview Alpine component.
    fn form_initial_json(&self) -> String {
        serde_json::to_string(&self.form).unwrap_or_else(|_| "{}".into())
    }

    /// Options for the kind picker: (key, label-fluent-key, tabler-icon).
    /// Order intentional — mirrors the public landing chip order.
    fn display_type_options(&self) -> &'static [(&'static str, &'static str, &'static str)] {
        &[
            ("app", "spec-form-kind-app", "app-window"),
            ("talk", "spec-form-kind-talk", "presentation"),
            ("report", "spec-form-kind-report", "file-text"),
            ("package", "spec-form-kind-package", "package"),
            ("api", "spec-form-kind-api", "api"),
            ("link", "spec-form-kind-link", "external-link"),
        ]
    }
}

// ── Handlers ─────────────────────────────────────────────────────

/// Media-library filenames for the logo picker. Empty when no DB is
/// wired or the query fails — the picker degrades to the text field.
async fn logo_filenames(state: &AppState) -> Vec<String> {
    match state.db.as_ref() {
        Some(pool) => db::images::list_all(pool)
            .await
            .map(|imgs| imgs.into_iter().map(|i| i.filename).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

async fn new_form(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    let page = SpecFormPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        nav_section: "specs",
        role: editor.role,
        mode: FormMode::New,
        form: SpecForm {
            // Sensible defaults for a new app
            display_type: "app".into(),
            state: "active".into(),
            access: "lock".into(),
            // The HTML base-href transform is on by default — the
            // safe behaviour for apps that don't self-route.
            inject_base_href: "on".into(),
            ..Default::default()
        },
        errors: Vec::new(),
        logo_images: logo_filenames(&state).await,
    };
    super::render(&page)
}

async fn edit_form(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Path(id): Path<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let spec = match db::specs::fetch_one(pool, &id).await {
        Ok(Some(s)) => s,
        Ok(None) => return (StatusCode::NOT_FOUND, "spec not found").into_response(),
        Err(e) => {
            tracing::error!(error = ?e, id, "fetch spec failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let page = SpecFormPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        nav_section: "specs",
        role: editor.role,
        mode: FormMode::Edit,
        form: SpecForm::from_spec(&spec),
        errors: Vec::new(),
        logo_images: logo_filenames(&state).await,
    };
    super::render(&page)
}

async fn create(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Form(form): Form<SpecForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };

    let mut errors = form.validate(FormMode::New);

    // Uniqueness check
    if errors.is_empty() {
        match db::specs::fetch_one(pool, form.id.trim()).await {
            Ok(Some(_)) => errors.push("spec-form-error-id-duplicate"),
            Ok(None) => {}
            Err(e) => {
                tracing::error!(error = ?e, "duplicate-check failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        }
    }

    if !errors.is_empty() {
        return render_form_with_errors(
            &state,
            loc,
            theme,
            editor.role,
            FormMode::New,
            form,
            errors,
        )
        .await;
    }

    let id = form.id.trim().to_string();
    let spec = match form.into_spec(None) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = ?e, "form → spec failed");
            return (StatusCode::BAD_REQUEST, "invalid form data").into_response();
        }
    };

    match db::specs::upsert_one(pool, &spec, Some(editor.actor())).await {
        Ok(_) => Redirect::to(&format!("/admin/specs/{}/edit", id)).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "save failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "save failed").into_response()
        }
    }
}

async fn update(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Path(id): Path<String>,
    Form(mut form): Form<SpecForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };

    // The URL id wins over the form id — operators don't get to
    // rename specs through the form (it would orphan the audit
    // log target). Renaming is a separate planned action.
    form.id = id.clone();

    let errors = form.validate(FormMode::Edit);
    if !errors.is_empty() {
        return render_form_with_errors(
            &state,
            loc,
            theme,
            editor.role,
            FormMode::Edit,
            form,
            errors,
        )
        .await;
    }

    // Load the existing spec as the merge base so fields the form
    // doesn't model (registry creds, lifetimes, limits, scaling, custom
    // template-properties) survive the edit instead of being wiped.
    let base = match db::specs::fetch_one(pool, &id).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = ?e, id, "load base spec failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };

    let spec = match form.into_spec(base.as_ref()) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = ?e, "form → spec failed");
            return (StatusCode::BAD_REQUEST, "invalid form data").into_response();
        }
    };

    match db::specs::upsert_one(pool, &spec, Some(editor.actor())).await {
        Ok(_) => Redirect::to(&format!("/admin/specs/{}/edit", id)).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "save failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "save failed").into_response()
        }
    }
}

async fn delete(
    editor: RequireEditor,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    match db::specs::delete_one(pool, &id, Some(editor.actor())).await {
        Ok(_) => Redirect::to("/admin/specs").into_response(),
        Err(e) => {
            tracing::error!(error = ?e, id, "delete failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response()
        }
    }
}

async fn render_form_with_errors(
    state: &AppState,
    loc: Locale,
    theme: Theme,
    role: Role,
    mode: FormMode,
    form: SpecForm,
    errors: Vec<&'static str>,
) -> Response {
    let page = SpecFormPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        nav_section: "specs",
        role,
        mode,
        form,
        errors,
        logo_images: logo_filenames(state).await,
    };
    let body = match page.render() {
        Ok(s) => s,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "render").into_response(),
    };
    (StatusCode::UNPROCESSABLE_ENTITY, axum::response::Html(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruscker_config::Config;

    /// A spec carrying fields the form doesn't model must survive an
    /// edit: `from_spec` (load into the form) → `into_spec(Some(base))`
    /// (save) must preserve registry creds, lifetimes, limits, scaling
    /// thresholds and custom template-properties. Regression for #74.
    #[test]
    fn edit_preserves_unmodelled_fields() {
        let yaml = r#"
proxy:
  specs:
    - id: ops
      display-name: Ops
      container-image: registry.example.com/acme/ops:latest
      container-lifetime: 360
      stop-on-logout: true
      docker-registry-username: acme
      docker-registry-domain: registry.example.com
      docker-registry-credential: dh-creds
      container-cpu-request: 0.25
      container-memory-request: 128m
      max-body-size: 25m
      routing-strategy: round-robin
      min-replicas: 1
      max-replicas: 4
      template-properties:
        type: app
        state: active
        custom-key: keep-me
"#;
        let cfg = Config::from_yaml(yaml).expect("parse fixture");
        let original = &cfg.proxy.specs[0];

        // Round-trip: load into the form, change a managed field, save.
        let mut form = SpecForm::from_spec(original);
        form.display_name = "Ops (edited)".into();
        let merged = form.into_spec(Some(original)).expect("into_spec");

        // Managed field changed.
        assert_eq!(merged.display_name.as_deref(), Some("Ops (edited)"));
        // Unmodelled fields preserved (the #74 bug would None these).
        assert_eq!(merged.container_lifetime, Some(360));
        assert_eq!(merged.stop_on_logout, Some(true));
        assert_eq!(merged.docker_registry_username.as_deref(), Some("acme"));
        assert_eq!(
            merged.docker_registry_domain.as_deref(),
            Some("registry.example.com")
        );
        assert_eq!(
            merged.docker_registry_credential.as_deref(),
            Some("dh-creds")
        );
        assert_eq!(merged.container_cpu_request, Some(0.25));
        assert_eq!(merged.container_memory_request.as_deref(), Some("128m"));
        assert_eq!(merged.max_body_size.as_deref(), Some("25m"));
        assert!(merged.routing_strategy.is_some());
        // Custom template-property survives.
        assert_eq!(
            merged.template_properties.get_str("custom-key"),
            Some("keep-me")
        );
        // Form-managed advanced fields still round-trip.
        assert_eq!(merged.min_replicas, Some(1));
        assert_eq!(merged.max_replicas, Some(4));
    }

    /// A brand-new spec (no base) has no unmodelled fields to carry.
    #[test]
    fn create_without_base_leaves_unmodelled_none() {
        let form = SpecForm {
            id: "fresh".into(),
            display_name: "Fresh".into(),
            display_type: "app".into(),
            state: "active".into(),
            access: "lock".into(),
            ..Default::default()
        };
        let spec = form.into_spec(None).expect("into_spec");
        assert_eq!(spec.id, "fresh");
        assert_eq!(spec.container_lifetime, None);
        assert_eq!(spec.docker_registry_username, None);
        assert_eq!(spec.max_body_size, None);
    }

    fn valid_form() -> SpecForm {
        SpecForm {
            id: "ok".into(),
            display_name: "Ok".into(),
            display_type: "app".into(),
            state: "active".into(),
            access: "lock".into(),
            ..Default::default()
        }
    }

    // #79/#83: malformed numbers used to silently default; now they're
    // form errors instead of "no limit / default".
    #[test]
    fn validate_rejects_malformed_numbers() {
        let mut f = valid_form();
        f.container_cpu_limit = "0,5".into(); // pt-BR comma
        assert!(f.validate(FormMode::New).contains(&"spec-form-error-cpu"));

        let mut f = valid_form();
        f.container_memory_limit = "512mb".into(); // typo
        assert!(f
            .validate(FormMode::New)
            .contains(&"spec-form-error-memory"));

        let mut f = valid_form();
        f.seats_per_container = "ten".into();
        assert!(f
            .validate(FormMode::New)
            .contains(&"spec-form-error-number"));

        let mut f = valid_form();
        f.min_replicas = "5".into();
        f.max_replicas = "2".into();
        assert!(f
            .validate(FormMode::New)
            .contains(&"spec-form-error-replica-range"));
    }

    #[test]
    fn volumes_round_trip_and_validate() {
        let mut f = valid_form();
        f.volumes = "/srv/data:/data\n/srv/www:/www:ro\n".into();
        let spec = f.into_spec(None).expect("into_spec");
        assert_eq!(
            spec.volumes,
            Some(vec![
                "/srv/data:/data".to_string(),
                "/srv/www:/www:ro".to_string()
            ])
        );
        // Round-trips back into the textarea (newline-joined).
        let back = SpecForm::from_spec(&spec);
        assert_eq!(back.volumes, "/srv/data:/data\n/srv/www:/www:ro");

        // A malformed bind is a form error.
        let mut bad = valid_form();
        bad.volumes = "not-a-bind".into();
        assert!(bad
            .validate(FormMode::New)
            .contains(&"spec-form-error-volume"));
    }

    #[test]
    fn validate_accepts_good_numbers() {
        let mut f = valid_form();
        f.container_cpu_limit = "0.5".into();
        f.container_memory_limit = "512m".into();
        f.seats_per_container = "10".into();
        f.min_replicas = "1".into();
        f.max_replicas = "3".into();
        f.heartbeat_timeout = "-1".into();
        let errs = f.validate(FormMode::New);
        assert!(errs.is_empty(), "expected no errors, got {errs:?}");
    }

    // ── #211: newly-modelled advanced fields ────────────────────

    #[test]
    fn new_advanced_fields_round_trip() {
        // Build a spec carrying every newly-modelled field, load it into
        // the form, and save it back unchanged — values must survive.
        let yaml = r#"
proxy:
  specs:
    - id: nb
      display-name: Notebook
      container-image: quay.io/jupyter/minimal-notebook:latest
      container-port: 8888
      platform: linux/amd64
      container-env:
        JUPYTER_TOKEN: ""
        GRANT_SUDO: "yes"
      container-cmd:
        - start-notebook.py
        - --ServerApp.base_url=/
      access-groups: [staff, ops]
      access-users: [alice]
      placement: bin-pack
      anti-affinity: true
      routing-strategy: round-robin
      scale-up-threshold: 0.8
      scale-down-threshold: 0.2
      scale-down-grace: 45
      drain-timeout: 20
      template-properties:
        type: app
        state: active
"#;
        let cfg = Config::from_yaml(yaml).expect("parse fixture");
        let original = &cfg.proxy.specs[0];
        let merged = SpecForm::from_spec(original)
            .into_spec(Some(original))
            .expect("into_spec");

        assert_eq!(merged.container_port, Some(8888));
        assert_eq!(merged.platform.as_deref(), Some("linux/amd64"));
        assert_eq!(merged.env_pairs(), vec!["GRANT_SUDO=yes", "JUPYTER_TOKEN="]);
        assert_eq!(
            merged.container_cmd.as_deref(),
            Some(&["start-notebook.py".into(), "--ServerApp.base_url=/".into()][..])
        );
        assert_eq!(merged.access_groups.as_deref(), Some(&["staff".into(), "ops".into()][..]));
        assert_eq!(merged.access_users.as_deref(), Some(&["alice".into()][..]));
        assert_eq!(merged.placement, Some(Placement::BinPack));
        assert_eq!(merged.anti_affinity, Some(true));
        assert_eq!(merged.routing_strategy, Some(RoutingStrategy::RoundRobin));
        assert_eq!(merged.scale_up_threshold.map(|f| f.0), Some(0.8));
        assert_eq!(merged.scale_down_grace, Some(45));
        assert_eq!(merged.drain_timeout, Some(20));
    }

    #[test]
    fn clearing_an_advanced_field_clears_it() {
        // The form is authoritative: blanking a field that the base had
        // set drops it to None (= inherit the default), not preserve.
        let yaml = "proxy:\n  specs:\n    - id: a\n      container-image: x\n      platform: linux/amd64\n";
        let cfg = Config::from_yaml(yaml).unwrap();
        let base = &cfg.proxy.specs[0];
        let mut form = SpecForm::from_spec(base);
        assert_eq!(form.platform, "linux/amd64");
        form.platform = "  ".into(); // operator clears it
        let merged = form.into_spec(Some(base)).unwrap();
        assert_eq!(merged.platform, None);
    }

    #[test]
    fn parse_env_splits_on_first_equals_and_sorts() {
        let m = parse_env("FOO=bar\n  BAZ = qux \n\nDSN=postgres://u:p@h/db?x=1\nnoequals\n=noval")
            .expect("some");
        assert_eq!(m.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(m.get("BAZ").map(String::as_str), Some("qux"));
        // value may itself contain '=' (only the first splits)
        assert_eq!(m.get("DSN").map(String::as_str), Some("postgres://u:p@h/db?x=1"));
        // lines without a key, or with an empty key, are skipped
        assert!(!m.contains_key("noequals"));
        assert_eq!(m.len(), 3);
        assert!(parse_env("   \n\n").is_none());
    }

    #[test]
    fn list_to_vec_accepts_commas_and_newlines() {
        assert_eq!(
            list_to_vec("staff, ops\nresearch").as_deref(),
            Some(&["staff".into(), "ops".into(), "research".into()][..])
        );
        assert!(list_to_vec("  ,  \n").is_none());
    }

    #[test]
    fn validate_flags_bad_port_and_threshold() {
        let mut f = valid_form();
        f.container_port = "70000".into();
        assert!(f.validate(FormMode::New).contains(&"spec-form-error-port"));

        let mut f = valid_form();
        f.scale_up_threshold = "1.5".into();
        assert!(f.validate(FormMode::New).contains(&"spec-form-error-threshold"));

        let mut f = valid_form();
        f.container_port = "8501".into();
        f.scale_up_threshold = "0.8".into();
        f.max_body_size = "10m".into();
        f.container_cpu_request = "0.25".into();
        assert!(f.validate(FormMode::New).is_empty());
    }
}
