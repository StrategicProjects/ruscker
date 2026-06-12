//! Inline-Markdown rendering for the landing intro (#812).
//!
//! A deliberately tiny subset — `**bold**`, `*italic*` and
//! `[label](https://url)` — NOT a CommonMark engine. The input is
//! HTML-escaped *first*, then the three constructs are rewritten, so
//! raw HTML can never pass through: the intro renders on the public
//! landing. Anything that doesn't match a construct stays literal
//! (a lone `*` is just an asterisk), which keeps every existing
//! plain-text intro rendering byte-for-byte as before.
//!
//! Lives in its own module (not `routes::landing`) because the admin
//! preview mirrors the same subset client-side — keeping the server
//! half small and visible makes it practical to keep the two in sync.

use regex::Regex;
use std::sync::OnceLock;

fn link_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    // Label without `]`, URL restricted to http(s) and stopped at
    // whitespace/`)` — the escape pass already neutralized quotes, so
    // the URL can't break out of the href attribute.
    RE.get_or_init(|| Regex::new(r"\[([^\]]+)\]\((https?://[^\s)]+)\)").unwrap())
}

fn bold_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\*\*([^*]+)\*\*").unwrap())
}

fn italic_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"\*([^*]+)\*").unwrap())
}

fn escape_html(src: &str) -> String {
    src.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render the inline subset to HTML. Escape-first: the output is safe
/// to emit with `|safe` into the landing template.
pub fn render_inline(src: &str) -> String {
    let escaped = escape_html(src);
    // Links first (so a `*` inside a URL can't be eaten by emphasis),
    // then bold before italic (`**` would otherwise read as two `*`).
    let s = link_re().replace_all(
        &escaped,
        "<a href=\"$2\" target=\"_blank\" rel=\"noopener\">$1</a>",
    );
    let s = bold_re().replace_all(&s, "<strong>$1</strong>");
    let s = italic_re().replace_all(&s, "<em>$1</em>");
    s.into_owned()
}

/// The plain-text reading of the same subset — markers dropped, link
/// labels kept. Used where the intro feeds plain-text contexts (the
/// meta-description fallback), so `**` never leaks into a `<meta>`.
pub fn strip_inline(src: &str) -> String {
    let s = link_re().replace_all(src, "$1");
    let s = bold_re().replace_all(&s, "$1");
    let s = italic_re().replace_all(&s, "$1");
    s.into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_text_is_untouched_but_escaped() {
        assert_eq!(render_inline("hello world"), "hello world");
        assert_eq!(
            render_inline("a <script>alert(1)</script> & \"quote\""),
            "a &lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;quote&quot;"
        );
    }

    #[test]
    fn bold_italic_and_nesting() {
        assert_eq!(render_inline("**b**"), "<strong>b</strong>");
        assert_eq!(render_inline("*i*"), "<em>i</em>");
        assert_eq!(
            render_inline("***both***"),
            "<em><strong>both</strong></em>"
        );
        assert_eq!(
            render_inline("mix **b** and *i*."),
            "mix <strong>b</strong> and <em>i</em>."
        );
    }

    #[test]
    fn lone_or_unpaired_markers_stay_literal() {
        assert_eq!(render_inline("2 * 3 = 6"), "2 * 3 = 6");
        assert_eq!(render_inline("a ** b"), "a ** b");
        assert_eq!(render_inline("*open"), "*open");
    }

    #[test]
    fn links_are_scheme_restricted_and_attribute_safe() {
        assert_eq!(
            render_inline("[docs](https://example.com/a?b=1&c=2)"),
            "<a href=\"https://example.com/a?b=1&amp;c=2\" target=\"_blank\" rel=\"noopener\">docs</a>"
        );
        // Non-http(s) schemes don't linkify.
        assert_eq!(
            render_inline("[x](javascript:alert(1))"),
            "[x](javascript:alert(1))"
        );
        // A quote can't escape the href — it was escaped first.
        assert_eq!(
            render_inline("[x](https://e.com/\"onmouseover=1)"),
            "<a href=\"https://e.com/&quot;onmouseover=1\" target=\"_blank\" rel=\"noopener\">x</a>"
        );
    }

    #[test]
    fn emphasis_inside_link_labels() {
        assert_eq!(
            render_inline("[**bold** label](https://e.com)"),
            "<a href=\"https://e.com\" target=\"_blank\" rel=\"noopener\"><strong>bold</strong> label</a>"
        );
    }

    #[test]
    fn strip_drops_markers_and_keeps_labels() {
        assert_eq!(
            strip_inline("**Welcome** to *the* [portal](https://e.com)."),
            "Welcome to the portal."
        );
        assert_eq!(strip_inline("2 * 3"), "2 * 3");
    }
}
