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
//! explicit cases and `<html>` (no attribute) for `Auto`, then a
//! tiny inline script reads `prefers-color-scheme` to set the
//! initial class. This avoids the FOUC of a server-side guess that
//! disagrees with the client OS.

use serde::{Deserialize, Serialize};

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
}
