//! # ruscker-admin
//!
//! Admin panel for Ruscker — CRUD over specs, image gallery,
//! credentials store, live monitoring dashboard, landing page editor.
//!
//! ## Stack
//!
//! - `axum` for routing
//! - `askama` for compile-time templates (typed, fast, no runtime IO)
//! - `sqlx` over SQLite for source-of-truth state
//! - HTMX + Alpine.js for client interactivity (no SPA build step)
//! - Tailwind 4 for styling (CLI binary, no Node required)
//!
//! ## Status: stub
//!
//! See `CLAUDE.md` in this crate for the implementation roadmap.
//! Mockups for every screen live in `docs/mockups/`.

#![allow(dead_code)]

use anyhow::Result;
use std::net::SocketAddr;

pub struct AdminServer {
    addr: SocketAddr,
}

impl AdminServer {
    pub fn new(addr: SocketAddr) -> Self {
        Self { addr }
    }

    pub async fn run(self) -> Result<()> {
        anyhow::bail!("AdminServer::run not yet implemented (phase 2)")
    }
}
