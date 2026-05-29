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
use std::sync::LazyLock;
use regex::Regex;
use std::env;

static ENV_VAR_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    // Allow lower- and mixed-case names too (Spring/ShinyProxy do), so a
    // `${db_password}` reference is interpolated rather than silently
    // left as literal text.
    Regex::new(r"\$\{([A-Za-z_][A-Za-z0-9_]*)(?::-([^}]*))?\}").expect("env var regex is valid")
});

/// Interpolate all `${VAR}` and `${VAR:-default}` references in the input.
///
/// References inside YAML comments (lines starting with `#` after optional
/// whitespace) are left untouched. This preserves comment-out workflows
/// that operators use during debugging.
/// YAML keys whose values are secrets: their `${VAR}` placeholders are
/// **preserved verbatim** at parse and resolved only at the point of use
/// (e.g. `creds_from_spec` right before a registry pull). This keeps the
/// resolved secret out of the DB on `import`, out of `export` output,
/// and off the admin form — "the DB never sees the secret in clear"
/// (#260). The credentials store (AES-encrypted) is the other path.
pub const SECRET_KEYS: &[&str] = &["docker-registry-password"];

pub fn interpolate(input: &str) -> Result<String> {
    let mut output = String::with_capacity(input.len());

    for line in input.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            output.push_str(line);
            continue;
        }
        // Leave secret-keyed lines (e.g. `docker-registry-password:
        // ${VAR}`) untouched — resolved at use, not at parse (#260).
        if is_secret_key_line(trimmed) {
            output.push_str(line);
            continue;
        }
        output.push_str(&interpolate_line(line)?);
    }

    Ok(output)
}

/// Resolve `${VAR}` / `${VAR:-default}` in a single scalar value (e.g. a
/// secret read back from the DB at use time). Idempotent: a value with
/// no `${...}` comes back unchanged.
pub fn interpolate_value(value: &str) -> Result<String> {
    interpolate_line(value)
}

/// Does this (already left-trimmed) YAML line assign a [`SECRET_KEYS`]
/// key? Matches the key before the first `:`, tolerating a leading `- `
/// for a list-item mapping's first key.
fn is_secret_key_line(trimmed: &str) -> bool {
    let key = trimmed
        .split(':')
        .next()
        .unwrap_or("")
        .trim_start_matches('-')
        .trim();
    SECRET_KEYS.contains(&key)
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
    fn preserves_secret_keyed_lines() {
        // #260: a secret-keyed line keeps its `${VAR}` verbatim (resolved
        // only at use), while ordinary lines interpolate as usual.
        with_env("RUSCKER_REG_PASS", "s3cret", || {
            let yaml = "container-image: nginx:${RUSCKER_NEVER_SET:-latest}\n\
                        docker-registry-password: ${RUSCKER_REG_PASS}\n";
            let out = interpolate(yaml).unwrap();
            assert!(out.contains("nginx:latest"), "non-secret line interpolated");
            assert!(
                out.contains("docker-registry-password: ${RUSCKER_REG_PASS}"),
                "secret line preserved verbatim: {out}"
            );
            assert!(!out.contains("s3cret"), "secret never resolved at parse");
        });
    }

    #[test]
    fn interpolate_value_resolves_and_is_idempotent() {
        with_env("RUSCKER_REG_PASS", "s3cret", || {
            assert_eq!(interpolate_value("${RUSCKER_REG_PASS}").unwrap(), "s3cret");
            // A value with no placeholder comes back unchanged.
            assert_eq!(interpolate_value("already-plain").unwrap(), "already-plain");
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
