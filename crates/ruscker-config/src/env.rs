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

    // Indentation of an open `container-env:` block, if any. Its child
    // lines (more indented) keep their `${VAR}` verbatim — app secrets
    // are resolved at spawn, not at parse (#272), so they never land in
    // the DB on `import`. Same "literal in the DB, resolve at use"
    // model as the secret scalar keys below (#260).
    let mut env_block_indent: Option<usize> = None;

    for line in input.split_inclusive('\n') {
        let content = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = content.trim_start();
        let indent = content.len() - trimmed.len();

        // Comments / blank lines: emit verbatim, don't change state.
        if trimmed.is_empty() || trimmed.starts_with('#') {
            output.push_str(line);
            continue;
        }
        // A line at/below the block key's indent closes the block.
        if env_block_indent.is_some_and(|bi| indent <= bi) {
            env_block_indent = None;
        }
        // Inside a `container-env:` block → preserve the child verbatim.
        if env_block_indent.is_some() {
            output.push_str(line);
            continue;
        }
        let key = trimmed
            .split(':')
            .next()
            .unwrap_or("")
            .trim_start_matches('-')
            .trim();
        // `container-env:` opens a block whose child values are secrets;
        // a secret scalar key (`docker-registry-password`) is preserved
        // directly. Both are resolved at use, not parse.
        if key == "container-env" {
            env_block_indent = Some(indent);
            output.push_str(line);
            continue;
        }
        if SECRET_KEYS.contains(&key) {
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

/// `true` when `value` (trimmed) consists ONLY of valid `${VAR}` /
/// `${VAR:-default}` tokens (plus inter-token whitespace) — i.e. nothing
/// literal remains that would be stored or leak.
///
/// Used to decide whether a value is safe to keep VERBATIM (resolved from
/// the environment at use time) vs. must be treated as a literal secret.
/// A loose `contains("${")` is unsafe: a literal password like `abc${def`
/// or `p@ss${word` has no valid token, so keeping it verbatim would store
/// the cleartext (see the credential-store regression #422).
pub fn is_pure_env_ref(value: &str) -> bool {
    let t = value.trim();
    // Strip every valid token; if only whitespace is left, the whole value
    // was env-refs. Empty (no token at all) ⇒ not an env-ref.
    !t.is_empty() && ENV_VAR_PATTERN.replace_all(t, "").trim().is_empty()
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
    use std::sync::Mutex;

    /// Serializes every test that mutates the process environment.
    /// `set_var` / `remove_var` poke global C state that is not
    /// thread-safe, and two tests sharing a var name (e.g. the
    /// `RUSCKER_REG_PASS` pair below) would otherwise race under the
    /// default parallel test runner — a real, observed flake (#295).
    /// One module-wide lock keeps env-mutating tests effectively serial
    /// without forcing `--test-threads=1` on the whole suite.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Set each `(key, value)` for the duration of `f`, then restore the
    /// prior value (or unset it). Holds [`ENV_LOCK`] so no two
    /// env-mutating tests run at once. Takes a slice so a test needing
    /// several vars makes a single, non-nested call — a nested call would
    /// deadlock on the non-reentrant lock.
    fn with_env<F: FnOnce()>(vars: &[(&str, &str)], f: F) {
        // Recover from a poisoned lock: a prior test panicking inside the
        // guard must not wedge every other env test.
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let originals: Vec<(String, Option<String>)> = vars
            .iter()
            .map(|(k, v)| {
                let original = env::var(k).ok();
                env::set_var(k, v);
                ((*k).to_string(), original)
            })
            .collect();
        f();
        for (k, original) in originals {
            match original {
                Some(v) => env::set_var(&k, v),
                None => env::remove_var(&k),
            }
        }
    }

    #[test]
    fn interpolates_simple_variable() {
        with_env(&[("RUSCKER_TEST_VAR", "hello")], || {
            let result = interpolate("value: ${RUSCKER_TEST_VAR}").unwrap();
            assert_eq!(result, "value: hello");
        });
    }

    #[test]
    fn preserves_secret_keyed_lines() {
        // #260: a secret-keyed line keeps its `${VAR}` verbatim (resolved
        // only at use), while ordinary lines interpolate as usual.
        with_env(&[("RUSCKER_REG_PASS", "s3cret")], || {
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
    fn preserves_container_env_block() {
        // #272: every `${VAR}` *under* container-env is preserved at
        // parse; a sibling key after the block interpolates normally.
        with_env(&[("RUSCKER_DB_PW", "s3cret")], || {
            let yaml = "\
proxy:
  specs:
  - id: db
    container-image: nginx:${RUSCKER_NEVER_SET:-latest}
    container-env:
      DB_PASSWORD: ${RUSCKER_DB_PW}
      API_KEY: ${RUSCKER_DB_PW}
    description: tag-${RUSCKER_NEVER_SET:-x}
";
            let out = interpolate(yaml).unwrap();
            assert!(out.contains("DB_PASSWORD: ${RUSCKER_DB_PW}"), "env value preserved");
            assert!(out.contains("API_KEY: ${RUSCKER_DB_PW}"), "all env values preserved");
            assert!(!out.contains("s3cret"), "no env secret resolved at parse");
            // The image (before the block) and the sibling key (after the
            // block, dedented) still interpolate.
            assert!(out.contains("nginx:latest"));
            assert!(out.contains("description: tag-x"));
        });
    }

    #[test]
    fn interpolate_value_resolves_and_is_idempotent() {
        with_env(&[("RUSCKER_REG_PASS", "s3cret")], || {
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
        with_env(&[("RUSCKER_A", "alpha"), ("RUSCKER_B", "beta")], || {
            let result = interpolate("x: ${RUSCKER_A}/${RUSCKER_B}").unwrap();
            assert_eq!(result, "x: alpha/beta");
        });
    }

    #[test]
    fn preserves_text_around_var() {
        with_env(&[("RUSCKER_NAME", "world")], || {
            let result = interpolate("greeting: hello ${RUSCKER_NAME}!").unwrap();
            assert_eq!(result, "greeting: hello world!");
        });
    }

    #[test]
    fn is_pure_env_ref_only_for_whole_token_values() {
        // Pure env-refs (kept verbatim by the credential store, #422).
        assert!(is_pure_env_ref("${VAR}"));
        assert!(is_pure_env_ref("  ${VAR}  "));
        assert!(is_pure_env_ref("${VAR:-default}"));
        assert!(is_pure_env_ref("${A} ${B}"));
        // NOT pure — literal text that would be stored / leak.
        assert!(!is_pure_env_ref("abc${def")); // malformed, no token
        assert!(!is_pure_env_ref("p@ss${word")); // no closing brace
        assert!(!is_pure_env_ref("prefix${VAR}")); // literal prefix
        assert!(!is_pure_env_ref("${VAR}suffix"));
        assert!(!is_pure_env_ref("plainsecret"));
        assert!(!is_pure_env_ref(""));
        assert!(!is_pure_env_ref("${}")); // empty braces, no valid name
    }
}
