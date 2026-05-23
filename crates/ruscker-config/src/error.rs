//! Error types for configuration loading.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("failed to read config file {path}: {source}")]
    Io {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("YAML parse error: {0}")]
    Yaml(#[from] serde_yaml_ng::Error),

    #[error("environment variable {name} referenced in config but not set")]
    MissingEnvVar { name: String },

    #[error("invalid environment variable reference: {literal}")]
    InvalidEnvRef { literal: String },

    #[error("invalid configuration: {0}")]
    Invalid(String),
}
