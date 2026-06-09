//! Template lints for the inline-JS i18n trap (#741, #740, #532).
//!
//! Askama entity-escapes interpolated text, but the browser DECODES
//! entities in attribute values BEFORE compiling an inline event
//! handler — so a localized message interpolated into an inline JS
//! string breaks the handler at the message's first quote/apostrophe
//! (FR "d'exécution" shipped a confirm-less destructive action). The
//! sanctioned patterns are `data-confirm="…"` (consumed by the
//! layout's global submit guard) or routing the string through
//! `data-*` + `dataset`; comments in `images.html` used to be the only
//! enforcement, and it regressed. These lints are the teeth.

use std::path::{Path, PathBuf};

fn template_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        for entry in std::fs::read_dir(dir).expect("read templates dir") {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "html") {
                out.push(path);
            }
        }
    }
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("templates");
    let mut files = Vec::new();
    walk(&root, &mut files);
    assert!(!files.is_empty(), "no templates found under {root:?}");
    files
}

/// No Askama interpolation inside an inline `confirm('…')` /
/// `prompt('…')` string — that's the exact shape that breaks on a
/// translated apostrophe. (A *static* inline handler reading from
/// `this.dataset` is fine.)
#[test]
fn no_interpolation_inside_inline_confirm_or_prompt() {
    let offenders: Vec<String> = template_files()
        .iter()
        .flat_map(|path| {
            let body = std::fs::read_to_string(path).expect("read template");
            body.lines()
                .enumerate()
                .filter(|(_, line)| {
                    // Direct interpolation into a confirm/prompt string…
                    let direct = ["confirm('{", "confirm(\"{", "prompt('{", "prompt(\"{"]
                        .iter()
                        .any(|pat| line.contains(pat));
                    // …or a LOCALIZED string anywhere inside an event-handler
                    // attribute's single-quoted JS (e.g. `@submit="… ?
                    // '{{ self.t(...) }}' : …"`). Non-localized interpolations
                    // (hex colours, enum keys) are server-controlled and can't
                    // grow an apostrophe, so they pass.
                    let handler = ["@submit", "@click", "onsubmit=", "onclick="]
                        .iter()
                        .any(|pat| line.contains(pat))
                        && line.contains("'{{ self.t(");
                    direct || handler
                })
                .map(|(n, line)| format!("{}:{}: {}", path.display(), n + 1, line.trim()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "localized text interpolated into an inline JS string — use \
         data-confirm / data-* + dataset instead (#741):\n{}",
        offenders.join("\n")
    );
}

/// The `onsubmit="return confirm(…)"` idiom is retired wholesale in
/// favour of `data-confirm` + the layout's global capture-phase guard —
/// even a today-safe message regresses the moment a translator adds an
/// apostrophe.
#[test]
fn no_inline_onsubmit_confirm_handlers() {
    let offenders: Vec<String> = template_files()
        .iter()
        .flat_map(|path| {
            let body = std::fs::read_to_string(path).expect("read template");
            body.lines()
                .enumerate()
                .filter(|(_, line)| line.contains("onsubmit=\"return confirm"))
                .map(|(n, line)| format!("{}:{}: {}", path.display(), n + 1, line.trim()))
                .collect::<Vec<_>>()
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "inline onsubmit confirm — use data-confirm + the layout guard (#741):\n{}",
        offenders.join("\n")
    );
}
