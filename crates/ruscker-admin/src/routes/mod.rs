//! HTTP route handlers, one module per resource family.

pub mod admin;
pub mod assets;
pub mod health;
pub mod landing;
pub mod metrics;
pub mod prefs;
pub mod proxy;
pub mod rewrite;

/// Reduce a `Referer` header to a safe **same-origin path** for
/// a post-action redirect, falling back to `fallback` when the
/// referer is absent or can't be trusted.
///
/// Browsers send `Referer` as an absolute URL
/// (`https://host:port/path?query`). We keep only the path +
/// query and discard the scheme + host entirely, so the redirect
/// always lands on Ruscker's own origin — a referer of
/// `https://evil.com/x` becomes `/x` here, not an open redirect.
///
/// Guards against protocol-relative (`//evil.com`) and backslash
/// (`/\evil.com`) tricks that some browsers/proxies normalize
/// into a cross-origin navigation.
pub(crate) fn same_origin_path(referer: Option<&str>, fallback: &'static str) -> String {
    let Some(referer) = referer else {
        return fallback.to_string();
    };
    // Drop `scheme://authority`, keep everything from the first
    // `/` of the path onward. A referer with no scheme is treated
    // as already path-like.
    let after_authority = match referer.split_once("://") {
        Some((_scheme, rest)) => match rest.find('/') {
            Some(i) => &rest[i..], // "/path?query"
            None => return fallback.to_string(),
        },
        None => referer,
    };
    // Must be a single-slash absolute path on our origin.
    let bytes = after_authority.as_bytes();
    let looks_cross_origin = !after_authority.starts_with('/')
        || after_authority.starts_with("//")
        || (bytes.len() >= 2 && bytes[0] == b'/' && bytes[1] == b'\\');
    if looks_cross_origin {
        return fallback.to_string();
    }
    after_authority.to_string()
}

/// Accept an explicit post-login destination only when it is already a
/// root-relative path on this origin. Unlike [`same_origin_path`], this does
/// not translate an absolute external URL into a local path: an operator- or
/// attacker-supplied `next=https://evil.example/x` is rejected outright.
pub(crate) fn local_next_path(value: Option<&str>) -> Option<&str> {
    let value = value?;
    let safe = value.starts_with('/')
        && !value.starts_with("//")
        && !value.contains('\\')
        && !value.bytes().any(|b| b < 0x20 || b == 0x7f);
    safe.then_some(value)
}

/// Append a percent-encoded `next` query parameter to a local path. The path
/// may already carry another query parameter (the token-login selector does),
/// in which case `next` is appended with `&`.
pub(crate) fn with_next_query(path: &str, next: &str) -> String {
    let separator = if path.contains('?') { '&' } else { '?' };
    let encoded: String = url::form_urlencoded::byte_serialize(next.as_bytes()).collect();
    format!("{path}{separator}next={encoded}")
}

#[cfg(test)]
mod same_origin_tests {
    use super::{local_next_path, same_origin_path, with_next_query};

    #[test]
    fn keeps_path_from_absolute_referer() {
        assert_eq!(
            same_origin_path(Some("http://127.0.0.1:8082/admin/specs"), "/"),
            "/admin/specs"
        );
        assert_eq!(
            same_origin_path(Some("https://portal.gov.br/app/x?q=1"), "/"),
            "/app/x?q=1"
        );
    }

    #[test]
    fn strips_cross_origin_host() {
        // The classic open-redirect payload: we keep only the
        // path, so the user stays on our origin.
        assert_eq!(same_origin_path(Some("https://evil.com/phish"), "/"), "/phish");
    }

    #[test]
    fn rejects_protocol_relative_and_backslash() {
        assert_eq!(same_origin_path(Some("//evil.com"), "/admin"), "/admin");
        assert_eq!(same_origin_path(Some("/\\evil.com"), "/admin"), "/admin");
    }

    #[test]
    fn falls_back_when_absent_or_pathless() {
        assert_eq!(same_origin_path(None, "/home"), "/home");
        assert_eq!(same_origin_path(Some("https://host-no-path"), "/home"), "/home");
    }

    #[test]
    fn accepts_already_relative_referer() {
        assert_eq!(same_origin_path(Some("/dashboard"), "/"), "/dashboard");
    }

    #[test]
    fn explicit_next_accepts_only_root_relative_paths() {
        assert_eq!(local_next_path(Some("/app/foo?q=1")), Some("/app/foo?q=1"));
        assert_eq!(local_next_path(Some("https://evil.example/x")), None);
        assert_eq!(local_next_path(Some("//evil.example/x")), None);
        assert_eq!(local_next_path(Some("/\\evil.example/x")), None);
        assert_eq!(local_next_path(Some("/app/foo\nset-cookie:x")), None);
    }

    #[test]
    fn next_query_is_encoded_and_appends_to_existing_query() {
        assert_eq!(
            with_next_query("/admin/login", "/app/foo?tab=1&sort=name"),
            "/admin/login?next=%2Fapp%2Ffoo%3Ftab%3D1%26sort%3Dname"
        );
        assert_eq!(
            with_next_query("/admin/login?token=1", "/app/foo/"),
            "/admin/login?token=1&next=%2Fapp%2Ffoo%2F"
        );
    }
}
