//! Environment variable interpolation for YAML configurations.
//!
//! Supports `${VAR_NAME}` and `${VAR_NAME:-default}` syntax inside YAML
//! string values. This runs as a pre-processing step on the raw YAML
//! text before parsing, so it works regardless of where the variable
//! appears in the file structure.
//!
//! # Why pre-process raw text?
//!
//! An alternative would be a custom serde deserializer that interpolates
//! string values during deserialization. Two reasons we don't:
//!
//! 1. **Visibility for operators.** A pre-processing step can be tested
//!    and printed independently. `ruscker config render` could show the
//!    interpolated YAML with secrets visible (or redacted) for debugging.
//! 2. **No coupling to serde.** Any future format (TOML, JSON, etc.)
//!    benefits from the same logic.
//!
//! # Security
//!
//! Missing variables produce hard errors by default. Users who want
//! defaults must use the `:-` syntax. This makes accidental empty
//! values (e.g. blank Docker passwords) impossible.

use crate::error::{Error, Result};
use once_cell::sync::Lazy;
use regex::Regex;
use std::env;

static ENV_VAR_PATTERN: Lazy<Regex> = Lazy::new(|| {
    Regex::new(r"\$\{([A-Z_][A-Z0-9_]*)(?::-([^}]*))?\}").expect("env var regex is valid")
});

/// Interpolate all `${VAR}` and `${VAR:-default}` references in the input.
///
/// References inside YAML comments (lines starting with `#` after optional
/// whitespace) are left untouched. This preserves comment-out workflows
/// that operators use during debugging.
pub fn interpolate(input: &str) -> Result<String> {
    let mut output = String::with_capacity(input.len());

    for line in input.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            output.push_str(line);
            continue;
        }
        output.push_str(&interpolate_line(line)?);
    }

    Ok(output)
}

fn interpolate_line(line: &str) -> Result<String> {
    let mut result = String::with_capacity(line.len());
    let mut last_end = 0;

    for caps in ENV_VAR_PATTERN.captures_iter(line) {
        let full_match = caps.get(0).expect("regex always matches group 0");
        let var_name = caps
            .get(1)
            .expect("regex always captures var name")
            .as_str();
        let default = caps.get(2).map(|m| m.as_str());

        result.push_str(&line[last_end..full_match.start()]);

        let value = match env::var(var_name) {
            Ok(v) => v,
            Err(env::VarError::NotPresent) => {
                if let Some(d) = default {
                    d.to_string()
                } else {
                    return Err(Error::MissingEnvVar {
                        name: var_name.to_string(),
                    });
                }
            }
            Err(env::VarError::NotUnicode(_)) => {
                return Err(Error::InvalidEnvRef {
                    literal: var_name.to_string(),
                });
            }
        };

        result.push_str(&value);
        last_end = full_match.end();
    }

    result.push_str(&line[last_end..]);
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_env<F: FnOnce()>(key: &str, value: &str, f: F) {
        let original = env::var(key).ok();
        env::set_var(key, value);
        f();
        match original {
            Some(v) => env::set_var(key, v),
            None => env::remove_var(key),
        }
    }

    #[test]
    fn interpolates_simple_variable() {
        with_env("RUSCKER_TEST_VAR", "hello", || {
            let result = interpolate("value: ${RUSCKER_TEST_VAR}").unwrap();
            assert_eq!(result, "value: hello");
        });
    }

    #[test]
    fn interpolates_with_default() {
        let result = interpolate("value: ${RUSCKER_NEVER_SET:-fallback}").unwrap();
        assert_eq!(result, "value: fallback");
    }

    #[test]
    fn empty_default_is_valid() {
        let result = interpolate("value: ${RUSCKER_NEVER_SET:-}").unwrap();
        assert_eq!(result, "value: ");
    }

    #[test]
    fn missing_var_without_default_errors() {
        let result = interpolate("value: ${RUSCKER_DEFINITELY_NOT_SET}");
        match result {
            Err(Error::MissingEnvVar { name }) => {
                assert_eq!(name, "RUSCKER_DEFINITELY_NOT_SET");
            }
            other => panic!("expected MissingEnvVar, got {other:?}"),
        }
    }

    #[test]
    fn skips_comments() {
        let result = interpolate("# ${RUSCKER_NOT_SET}\nvalue: ok").unwrap();
        assert_eq!(result, "# ${RUSCKER_NOT_SET}\nvalue: ok");
    }

    #[test]
    fn multiple_vars_on_same_line() {
        with_env("RUSCKER_A", "alpha", || {
            with_env("RUSCKER_B", "beta", || {
                let result = interpolate("x: ${RUSCKER_A}/${RUSCKER_B}").unwrap();
                assert_eq!(result, "x: alpha/beta");
            });
        });
    }

    #[test]
    fn preserves_text_around_var() {
        with_env("RUSCKER_NAME", "world", || {
            let result = interpolate("greeting: hello ${RUSCKER_NAME}!").unwrap();
            assert_eq!(result, "greeting: hello world!");
        });
    }
}
