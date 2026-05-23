//! Internationalization via Fluent (`.ftl` files).
//!
//! Locale precedence at request time (see [`Locale::resolve`]):
//! 1. Cookie `ruscker_locale=<code>` set by the UI selector
//! 2. `Accept-Language` HTTP header
//! 3. Fallback to [`DEFAULT`] (pt-BR)
//!
//! Translation files live in `assets/i18n/<lang>/*.ftl`, embedded
//! into the binary via [`include_dir`] so there are no runtime
//! filesystem dependencies.

use anyhow::{Context, Result};
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use fluent_bundle::{concurrent::FluentBundle, FluentArgs, FluentResource};
use include_dir::{include_dir, Dir};
use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt;
use std::sync::Arc;
use tower_cookies::Cookies;
use unic_langid::LanguageIdentifier;

/// Embedded translation files. Anything under `assets/i18n/` ships
/// in the binary.
static I18N_DIR: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/assets/i18n");

/// Fallback locale when the user has not chosen one and
/// `Accept-Language` does not match any supported locale.
pub const DEFAULT: Locale = Locale::Pt;

/// Cookie key set by the language picker. Value is the short ISO
/// code (`pt`, `en`, `es`, `fr`).
pub const COOKIE_NAME: &str = "ruscker_locale";

/// Locales the UI exposes in the language selector. Add a variant
/// here AND a directory under `assets/i18n/<code>/` to ship a new
/// language.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash)]
pub enum Locale {
    Pt,
    En,
    Es,
    Fr,
}

impl Locale {
    pub const ALL: [Locale; 4] = [Locale::Pt, Locale::En, Locale::Es, Locale::Fr];

    /// Short ISO code as it appears in cookies, URLs, and the UI.
    pub fn code(self) -> &'static str {
        match self {
            Locale::Pt => "pt",
            Locale::En => "en",
            Locale::Es => "es",
            Locale::Fr => "fr",
        }
    }

    /// BCP-47 tag used by `fluent-bundle` and `Accept-Language`.
    pub fn tag(self) -> &'static str {
        match self {
            Locale::Pt => "pt-BR",
            Locale::En => "en-US",
            Locale::Es => "es-ES",
            Locale::Fr => "fr-FR",
        }
    }

    /// Native name shown in the language selector ("Português",
    /// "English", "Español", "Français").
    pub fn native_name(self) -> &'static str {
        match self {
            Locale::Pt => "Português",
            Locale::En => "English",
            Locale::Es => "Español",
            Locale::Fr => "Français",
        }
    }

    /// Parse an ISO short code. Returns `None` if unknown.
    pub fn parse(code: &str) -> Option<Locale> {
        let trimmed = code.split(['-', '_']).next().unwrap_or(code);
        match trimmed.to_ascii_lowercase().as_str() {
            "pt" => Some(Locale::Pt),
            "en" => Some(Locale::En),
            "es" => Some(Locale::Es),
            "fr" => Some(Locale::Fr),
            _ => None,
        }
    }
}

impl fmt::Display for Locale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// Axum extractor that resolves the active locale from the request.
///
/// Order of precedence:
/// 1. Cookie [`COOKIE_NAME`] (user's explicit pick via the UI)
/// 2. `Accept-Language` header (browser-supplied preference list)
/// 3. [`DEFAULT`] (pt-BR)
///
/// This is infallible — every request resolves to *some* locale.
impl<S: Send + Sync> FromRequestParts<S> for Locale {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        // 1. cookie
        if let Ok(cookies) = Cookies::from_request_parts(parts, state).await {
            if let Some(c) = cookies.get(COOKIE_NAME) {
                if let Some(loc) = Locale::parse(c.value()) {
                    return Ok(loc);
                }
            }
        }

        // 2. Accept-Language. Naive parsing: split on `,`, strip
        //    `;q=` weights, try each token. Good enough for the four
        //    locales we support; if we ever need full RFC 9110
        //    negotiation, swap in a crate.
        if let Some(header) = parts
            .headers
            .get(axum::http::header::ACCEPT_LANGUAGE)
            .and_then(|v| v.to_str().ok())
        {
            for token in header.split(',') {
                let lang = token.split(';').next().unwrap_or("").trim();
                if let Some(loc) = Locale::parse(lang) {
                    return Ok(loc);
                }
            }
        }

        Ok(DEFAULT)
    }
}

/// All bundles, keyed by [`Locale`]. Loaded once at server startup.
pub struct Locales {
    bundles: HashMap<Locale, Arc<FluentBundle<FluentResource>>>,
}

impl Locales {
    /// Load every locale present under `assets/i18n/`. A missing
    /// directory for one of the four supported locales is **not** an
    /// error — that locale just falls back to [`DEFAULT`] on lookup.
    pub fn load() -> Result<Self> {
        let mut bundles = HashMap::with_capacity(Locale::ALL.len());
        for &loc in &Locale::ALL {
            if let Some(bundle) = load_bundle(loc)? {
                bundles.insert(loc, Arc::new(bundle));
            }
        }
        if !bundles.contains_key(&DEFAULT) {
            anyhow::bail!(
                "default locale `{}` has no translation files at assets/i18n/{}/",
                DEFAULT.code(),
                DEFAULT.code()
            );
        }
        Ok(Self { bundles })
    }

    /// Look up `key` in `locale`. Falls back to [`DEFAULT`] when the
    /// locale is missing the key (logged once per key via tracing).
    pub fn t(&self, locale: Locale, key: &str, args: Option<&FluentArgs<'_>>) -> String {
        if let Some(s) = self.try_format(locale, key, args) {
            return s;
        }
        if locale != DEFAULT {
            if let Some(s) = self.try_format(DEFAULT, key, args) {
                tracing::debug!(missing = key, locale = locale.code(), "falling back to default");
                return s;
            }
        }
        tracing::warn!(missing = key, "translation key not found in any locale");
        format!("⟦{key}⟧")
    }

    fn try_format(
        &self,
        locale: Locale,
        key: &str,
        args: Option<&FluentArgs<'_>>,
    ) -> Option<String> {
        let bundle = self.bundles.get(&locale)?;
        let msg = bundle.get_message(key)?;
        let pattern = msg.value()?;
        let mut errs = Vec::new();
        let out = bundle.format_pattern(pattern, args, &mut errs);
        if !errs.is_empty() {
            tracing::warn!(?errs, key, locale = locale.code(), "fluent format errors");
        }
        // strip Fluent's Unicode isolation marks for plain HTML output
        Some(out.replace(['\u{2068}', '\u{2069}'], ""))
    }
}

fn load_bundle(locale: Locale) -> Result<Option<FluentBundle<FluentResource>>> {
    let Some(dir) = I18N_DIR.get_dir(locale.code()) else {
        return Ok(None);
    };
    let lang: LanguageIdentifier = locale
        .tag()
        .parse()
        .with_context(|| format!("parse {} as BCP-47", locale.tag()))?;
    let mut bundle = FluentBundle::new_concurrent(vec![lang]);
    bundle.set_use_isolating(false);
    for file in dir.files() {
        let text = file
            .contents_utf8()
            .with_context(|| format!("{} is not utf-8", file.path().display()))?;
        let res = FluentResource::try_new(text.to_string()).map_err(|(_, errs)| {
            anyhow::anyhow!("parse {}: {:?}", file.path().display(), errs)
        })?;
        bundle
            .add_resource(res)
            .map_err(|errs| anyhow::anyhow!("add {}: {:?}", file.path().display(), errs))?;
    }
    Ok(Some(bundle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_short_codes() {
        assert_eq!(Locale::parse("pt"), Some(Locale::Pt));
        assert_eq!(Locale::parse("PT-br"), Some(Locale::Pt));
        assert_eq!(Locale::parse("en_US"), Some(Locale::En));
        assert_eq!(Locale::parse("zz"), None);
    }

    #[test]
    fn load_default_succeeds() {
        let locales = Locales::load().expect("load locales");
        // landing.title is the canary key shipped in Phase 1
        let s = locales.t(Locale::Pt, "landing-title", None);
        assert!(!s.starts_with("⟦"), "got placeholder: {s:?}");
    }

    #[test]
    fn missing_locale_falls_back() {
        let locales = Locales::load().expect("load locales");
        // Even if EN bundle is incomplete, default fallback kicks in
        let s = locales.t(Locale::En, "landing-title", None);
        assert!(!s.starts_with("⟦"), "got placeholder: {s:?}");
    }
}
