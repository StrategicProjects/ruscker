//! # ruscker-proxy
//!
//! HTTP and WebSocket reverse proxy for Ruscker. The trickiest crate in
//! the workspace because of WebSocket handling for Shiny apps.
//!
//! ## Status: stub
//!
//! See `CLAUDE.md` in this crate for the exact sequence of work to do
//! when implementing Phase 3.
//!
//! ## Key constraints
//!
//! 1. **Sticky sessions for Shiny.** Once a session is created on
//!    replica X, every subsequent HTTP request and WebSocket frame
//!    must reach replica X — never X+1. Implemented via a signed
//!    cookie (`__ruscker_session`) containing the replica ID.
//!
//! 2. **WebSocket upgrade hijack.** Shiny's reactive layer uses WS.
//!    We need to: (a) accept the upgrade request, (b) open a parallel
//!    WS to the upstream container, (c) bidirectionally pump frames
//!    with backpressure.
//!
//! 3. **Path rewriting.** Client requests `/app/<id>/...`. Upstream
//!    container expects `/...` (its app root). Rewrite path prefix
//!    and the `Host` header.
//!
//! 4. **API specs are easier.** `type: api` specs skip the WS path
//!    entirely and use round-robin with no cookie.

#![allow(dead_code)]

use anyhow::Result;
use std::net::SocketAddr;

pub struct ProxyServer {
    addr: SocketAddr,
}

impl ProxyServer {
    pub fn new(addr: SocketAddr) -> Self {
        Self { addr }
    }

    pub async fn run(self) -> Result<()> {
        anyhow::bail!("ProxyServer::run not yet implemented (phase 3)")
    }
}
