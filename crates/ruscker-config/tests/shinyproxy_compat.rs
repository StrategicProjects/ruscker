//! Integration test: load the example `application.yml` from
//! `examples/` and verify it parses correctly.
//!
//! This is the most important test in the codebase — it's the
//! ShinyProxy-compatibility contract. The example is a generic but
//! comprehensive config exercising every spec kind, env-var
//! interpolation, template properties, and the Ruscker extensions. If
//! this breaks, existing ShinyProxy users can't migrate.

use ruscker_config::{Config, SpecKind};
use std::path::PathBuf;

fn fixture_path() -> PathBuf {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop();
    p.pop();
    p.push("examples");
    p.push("application.yml");
    p
}

fn load_with_env() -> Config {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test-pat-not-real");
    let path = fixture_path();
    Config::from_file(&path).unwrap_or_else(|e| panic!("failed to load {path:?}: {e}"))
}

#[test]
fn parses_example_config() {
    let config = load_with_env();
    assert_eq!(config.proxy.title.trim(), "Ruscker Demo Portal");
    assert_eq!(config.proxy.landing_page, "/");
    assert!(config.proxy.hide_navbar);
    assert_eq!(config.proxy.heartbeat_rate, 10_000);
    assert_eq!(config.proxy.heartbeat_timeout, 3_600_000);
    // `shutdown-grace-ms` is a Ruscker extension absent from the
    // example — it must fall back to the 30s default.
    assert_eq!(config.proxy.shutdown_grace_ms, 30_000);
    assert_eq!(config.proxy.port, 8080);
    assert_eq!(config.proxy.bind_address, "127.0.0.1");
}

#[test]
fn loads_all_specs() {
    let config = load_with_env();
    assert_eq!(
        config.proxy.specs.len(),
        8,
        "expected 8 specs, found {}",
        config.proxy.specs.len()
    );
}

#[test]
fn classifies_specs_correctly() {
    let config = load_with_env();
    let count = |k: SpecKind| config.proxy.specs.iter().filter(|s| s.kind() == k).count();

    assert_eq!(count(SpecKind::Shiny), 5, "expected 5 Shiny specs");
    assert_eq!(count(SpecKind::Api), 1, "expected 1 API spec");
    assert_eq!(count(SpecKind::External), 2, "expected 2 external specs");

    // Interactive apps need sticky sessions; APIs and external links don't.
    for spec in &config.proxy.specs {
        match spec.kind() {
            SpecKind::Shiny => assert!(
                spec.needs_sticky_sessions(),
                "spec {} should need sticky sessions",
                spec.id
            ),
            SpecKind::External => {
                assert!(
                    spec.container_image.is_none(),
                    "external spec {} should have no container-image",
                    spec.id
                );
                assert!(!spec.needs_sticky_sessions(), "spec {} should not be sticky", spec.id);
            }
            _ => {}
        }
    }
}

#[test]
fn env_interpolation_works_on_credentials() {
    let config = load_with_env();
    let ops = config
        .proxy
        .specs
        .iter()
        .find(|s| s.id == "ops-report")
        .expect("ops-report spec must exist");
    assert_eq!(
        ops.docker_registry_password.as_deref(),
        Some("test-pat-not-real"),
        "${{DOCKER_REGISTRY_PASSWORD}} should have been interpolated"
    );
    assert_eq!(ops.docker_registry_username.as_deref(), Some("acme"));
}

#[test]
fn template_properties_preserved_with_html() {
    let config = load_with_env();
    let app = config
        .proxy
        .specs
        .iter()
        .find(|s| s.id == "sales-dashboard")
        .expect("sales-dashboard spec must exist");

    assert_eq!(
        app.template_properties.get_str("logo"),
        Some("/assets/img/sales.png")
    );
    assert_eq!(app.template_properties.type_field(), Some("app"));
    assert_eq!(app.template_properties.state(), "active");
    // Inline HTML in the description is preserved verbatim.
    assert!(app.description.as_deref().unwrap().contains("<a href="));
}

#[test]
fn external_link_spec_carries_a_link() {
    let config = load_with_env();
    let handbook = config
        .proxy
        .specs
        .iter()
        .find(|s| s.id == "handbook")
        .expect("handbook spec must exist");
    assert_eq!(handbook.kind(), SpecKind::External);
    assert!(handbook.template_properties.get_str("link").is_some());
}

#[test]
fn heartbeat_override_negative_one_preserved() {
    let config = load_with_env();
    let longrun = config
        .proxy
        .specs
        .iter()
        .find(|s| s.id == "longrun-model")
        .expect("longrun-model spec must exist");
    assert_eq!(longrun.heartbeat_timeout, Some(-1));
}

#[test]
fn validation_finds_no_embedded_credentials() {
    let config = load_with_env();
    let report = config.validate();

    let embedded: Vec<_> = report
        .warnings
        .iter()
        .filter(|w| matches!(w, ruscker_config::Warning::EmbeddedCredential { .. }))
        .collect();
    assert!(
        embedded.is_empty(),
        "the example uses ${{VAR}} form throughout, expected no \
        EmbeddedCredential warnings, found: {:?}",
        embedded
    );
}

#[test]
fn stats_report_matches_yaml() {
    let config = load_with_env();
    let report = config.validate();
    assert_eq!(report.stats.total_specs, 8);
    assert_eq!(*report.stats.by_kind.get("shiny").unwrap_or(&0), 5);
    assert_eq!(*report.stats.by_kind.get("external").unwrap_or(&0), 2);
}

#[test]
fn session_timeout_resolves_from_flat_notation() {
    let config = load_with_env();
    assert_eq!(config.server.session_timeout_secs(), Some(3600));
}
