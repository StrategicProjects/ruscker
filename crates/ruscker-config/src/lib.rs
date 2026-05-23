//! # ruscker-config
//!
//! ShinyProxy-compatible YAML configuration parsing for Ruscker.
//!
//! This crate is the entry point for loading and validating Ruscker
//! configurations. It supports a subset of ShinyProxy's `application.yml`
//! schema plus Ruscker-specific extensions (API specs, replica pools,
//! load-balancing strategies).
//!
//! ## Quick start
//!
//! ```no_run
//! use ruscker_config::Config;
//!
//! let config = Config::from_file("application.yml")?;
//! println!("Loaded {} specs", config.proxy.specs.len());
//! # Ok::<(), ruscker_config::Error>(())
//! ```
//!
//! ## Environment variable interpolation
//!
//! String values may reference environment variables using `${VAR_NAME}`
//! syntax. This is essential for keeping secrets like Docker registry
//! credentials out of YAML files:
//!
//! ```yaml
//! docker-registry-password: ${DOCKER_REGISTRY_PASSWORD}
//! ```
//!
//! See [`env::interpolate`] for the interpolation rules.
//!
//! ## Compatibility
//!
//! See `docs/YAML_SCHEMA.md` in the repository root for the full schema
//! reference and notes on which ShinyProxy features are supported,
//! deprecated, or extended.

pub mod env;
pub mod error;
pub mod schema;
pub mod validate;

pub use error::{Error, Result};
pub use schema::{Config, Proxy, RoutingStrategy, Server, Spec, SpecKind, SpecKindOverride, TemplateProperties};
pub use validate::{ValidationReport, Warning};

use std::path::Path;

impl Config {
    /// Load and parse a configuration file from disk.
    ///
    /// This reads the file, interpolates environment variables, parses the
    /// YAML, and runs validation. Returns a fully-validated `Config` or an
    /// error describing what went wrong.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref();
        let raw = std::fs::read_to_string(path).map_err(|e| Error::Io {
            path: path.display().to_string(),
            source: e,
        })?;
        Self::from_yaml(&raw)
    }

    /// Parse a configuration from a YAML string.
    ///
    /// Mostly useful for tests and programmatic construction. Production
    /// code should prefer [`Config::from_file`].
    pub fn from_yaml(raw: &str) -> Result<Self> {
        let pre_warnings = validate::scan_raw_text(raw);
        let interpolated = env::interpolate(raw)?;
        let mut config: Config = serde_yaml::from_str(&interpolated)?;
        config.raw_warnings = pre_warnings;
        Ok(config)
    }

    /// Run non-fatal validation, returning warnings.
    ///
    /// Validation that produces *fatal* errors happens during parsing.
    /// This step surfaces issues that don't prevent operation but should
    /// be flagged to the operator (duplicate IDs, empty descriptions,
    /// suspicious credentials, etc).
    pub fn validate(&self) -> ValidationReport {
        let mut report = validate::run(self);
        report.warnings.extend(self.raw_warnings.iter().cloned());
        report
    }
}
