//! Light/dark theme selection.
//!
//! Three states because we want the user's explicit choice to
//! survive across visits, but new visitors should follow their OS
//! preference without any cookie at all:
//!
//! - [`Theme::Light`] / [`Theme::Dark`] — explicit choice in cookie
//! - [`Theme::Auto`] — no cookie; browser picks via
//!   `prefers-color-scheme`
//!
//! The render path emits `<html data-theme="...">` for the two
//! explicit cases and `<html>` (no attribute) for `Auto`. CSS in
//! `assets/tailwind/input.css` then applies dark vars either when
//! `data-theme="dark"` is set or when the OS reports dark.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde::{Deserialize, Serialize};
use std::convert::Infallible;
use tower_cookies::Cookies;

pub const COOKIE_NAME: &str = "ruscker_theme";

#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
    Auto,
}

impl Theme {
    pub fn parse(value: &str) -> Theme {
        match value {
            "light" => Theme::Light,
            "dark" => Theme::Dark,
            _ => Theme::Auto,
        }
    }

    /// Value to set in the cookie. `Auto` clears the cookie so the
    /// caller knows to issue a removal instead of a set.
    pub fn cookie_value(self) -> Option<&'static str> {
        match self {
            Theme::Light => Some("light"),
            Theme::Dark => Some("dark"),
            Theme::Auto => None,
        }
    }

    /// `data-theme` attribute value, or `None` if the client
    /// should decide via `prefers-color-scheme`.
    pub fn data_attr(self) -> Option<&'static str> {
        match self {
            Theme::Light => Some("light"),
            Theme::Dark => Some("dark"),
            Theme::Auto => None,
        }
    }

    pub fn is_light(self) -> bool {
        matches!(self, Theme::Light)
    }
    pub fn is_dark(self) -> bool {
        matches!(self, Theme::Dark)
    }
    pub fn is_auto(self) -> bool {
        matches!(self, Theme::Auto)
    }
}

/// Axum extractor: read the theme from the cookie, default to
/// `Auto` when absent or unrecognized.
impl<S: Send + Sync> FromRequestParts<S> for Theme {
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        if let Ok(cookies) = Cookies::from_request_parts(parts, state).await {
            if let Some(c) = cookies.get(COOKIE_NAME) {
                return Ok(Theme::parse(c.value()));
            }
        }
        Ok(Theme::Auto)
    }
}
