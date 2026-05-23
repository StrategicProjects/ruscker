//! Integration test: load the real (sanitized) `application.yml` from
//! `examples/` and verify it parses correctly.
//!
//! This is the most important test in the codebase — if this breaks,
//! existing ShinyProxy users can't migrate.

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
fn parses_full_sepe_config() {
    let config = load_with_env();
    assert_eq!(config.proxy.title.trim(), "Monitoramento Estratégico");
    assert_eq!(config.proxy.landing_page, "/");
    assert!(config.proxy.hide_navbar);
    assert_eq!(config.proxy.heartbeat_rate, 10_000);
    assert_eq!(config.proxy.heartbeat_timeout, 3_600_000);
    assert_eq!(config.proxy.port, 8080);
    assert_eq!(config.proxy.bind_address, "127.0.0.1");
}

#[test]
fn loads_all_31_specs() {
    let config = load_with_env();
    assert_eq!(
        config.proxy.specs.len(),
        31,
        "expected 31 specs, found {}",
        config.proxy.specs.len()
    );
}

#[test]
fn classifies_specs_correctly() {
    let config = load_with_env();

    let containerized: Vec<_> = config
        .proxy
        .specs
        .iter()
        .filter(|s| s.container_image.is_some())
        .collect();
    let external: Vec<_> = config
        .proxy
        .specs
        .iter()
        .filter(|s| s.container_image.is_none())
        .collect();

    assert_eq!(
        containerized.len(),
        17,
        "expected 17 specs with container-image"
    );
    assert_eq!(external.len(), 14, "expected 14 specs without container");

    for spec in containerized {
        assert_eq!(spec.kind(), SpecKind::Shiny, "spec {} should be Shiny", spec.id);
        assert!(
            spec.needs_sticky_sessions(),
            "spec {} should need sticky sessions",
            spec.id
        );
    }
    for spec in external {
        assert_eq!(spec.kind(), SpecKind::External, "spec {} should be External", spec.id);
    }
}

#[test]
fn env_interpolation_works_on_credentials() {
    let config = load_with_env();
    let creches = config
        .proxy
        .specs
        .iter()
        .find(|s| s.id == "creches")
        .expect("creches spec must exist");
    assert_eq!(
        creches.docker_registry_password.as_deref(),
        Some("test-pat-not-real"),
        "${{DOCKER_REGISTRY_PASSWORD}} should have been interpolated"
    );
    assert_eq!(creches.docker_registry_username.as_deref(), Some("milkway"));
}

#[test]
fn template_properties_preserved_with_html() {
    let config = load_with_env();
    let aurora = config
        .proxy
        .specs
        .iter()
        .find(|s| s.id == "auroraprime")
        .expect("auroraprime spec must exist");

    assert_eq!(
        aurora.template_properties.get_str("logo"),
        Some("/assets/img/auroraprime.png")
    );
    assert_eq!(aurora.template_properties.type_field(), Some("app"));
    assert_eq!(aurora.template_properties.state(), "active");
    assert!(aurora.template_properties.get_str("link").is_some());
    assert!(aurora
        .description
        .as_deref()
        .unwrap()
        .contains("Governo de Pernambuco"));
}

#[test]
fn heartbeat_override_negative_one_preserved() {
    let config = load_with_env();
    let hortensias = config
        .proxy
        .specs
        .iter()
        .find(|s| s.id == "hortensias")
        .expect("hortensias spec must exist");
    assert_eq!(hortensias.heartbeat_timeout, Some(-1));
}

#[test]
fn validation_finds_no_embedded_credentials_after_sanitization() {
    let config = load_with_env();
    let report = config.validate();

    let embedded: Vec<_> = report
        .warnings
        .iter()
        .filter(|w| {
            matches!(
                w,
                ruscker_config::Warning::EmbeddedCredential { .. }
            )
        })
        .collect();
    assert!(
        embedded.is_empty(),
        "the sanitized YAML uses ${{VAR}} form throughout, expected no \
        EmbeddedCredential warnings, found: {:?}",
        embedded
    );
}

#[test]
fn stats_report_matches_yaml() {
    let config = load_with_env();
    let report = config.validate();
    assert_eq!(report.stats.total_specs, 31);
    assert_eq!(*report.stats.by_kind.get("shiny").unwrap_or(&0), 17);
    assert_eq!(*report.stats.by_kind.get("external").unwrap_or(&0), 14);
}

#[test]
fn session_timeout_resolves_from_flat_notation() {
    let config = load_with_env();
    assert_eq!(config.server.session_timeout_secs(), Some(3600));
}
