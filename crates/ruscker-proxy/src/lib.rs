//! # ruscker-proxy
//!
//! Building blocks used by `ruscker-admin`'s proxy routes:
//!
//! - [`sticky`] — HMAC-signed cookie that binds a visitor to one
//!   replica for the lifetime of their Shiny session.
//! - [`ws`] — WebSocket upgrade hijack + bidirectional pump.
//!
//! The routes themselves live in `ruscker-admin` because they
//! consume the shared `AppState`. This crate stays small and
//! reusable — every function here can be unit-tested without an
//! axum server.

pub mod sticky;
pub mod ws;
