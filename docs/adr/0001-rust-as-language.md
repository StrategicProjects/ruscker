# ADR 0001 — Rust as the implementation language

Status: accepted

## Context

We need to build a replacement for ShinyProxy and Shiny Server Free.
The candidates are:

- Java/Kotlin (matching ShinyProxy's stack)
- Go (used by Docker, Caddy, similar tools)
- Rust
- Python (matching the R/Plumber community's neighbour language)
- Node.js

## Decision

Rust.

## Consequences

### What we gain

- **Footprint**: a static binary in the low tens of MB, idle memory
  around ~14 MB for the proxy itself (measured in production). The
  JVM-based proxy it replaced, on the same machine serving the same
  apps, sat at ~540 MB idle. For a portal serving 10-20 containers,
  this can make Ruscker fit on machines where ShinyProxy can't.
- **Startup**: milliseconds vs seconds. Important for restarts during
  config changes.
- **WebSocket performance**: Rust's async ecosystem (tokio + axum +
  hyper) is among the best at sustaining many idle connections — which
  is exactly what Shiny apps create.
- **Single binary deploy**: no JRE to install, no `pip install`. The
  `target/release/ruscker` binary is the entire artifact.
- **Type safety end-to-end**: from YAML schema to admin templates
  (Askama is compile-time typed against Rust structs). Many bugs that
  would surface at runtime in dynamic stacks are caught at build time.

### What we lose

- **Smaller talent pool** in R/data-science world. The target users
  (Brazilian government IT teams, data scientists at universities) are
  unlikely to have Rust experience. Mitigated by:
  - Heavy documentation (per-crate developer guides, mockups, ADRs)
  - Conservative use of language features (no macros heavy magic, no
    fancy generic constraints)
  - Clear separation between domain code (pure Rust) and I/O
- **Build times** compared to Go. ~30s for a cold release build on
  modern hardware. Acceptable; not in the CI critical path most days.
- **Library maturity** for some niches. Async WebSocket proxying with
  proper backpressure isn't a beaten path — we'll have to be careful.
  Mitigated by Cloudflare's Pingora demonstrating that production-grade
  Rust proxying works.

## Alternatives considered

**Go** was the close runner-up. Same single-binary story, slightly
larger memory footprint, simpler concurrency model (goroutines vs
async/await). We chose Rust because:

1. Type system is stronger (sum types, exhaustive matches), which
   matters a lot for a system with many spec kinds and routing
   strategies that must dispatch correctly.
2. Performance ceiling is higher for the WebSocket proxy case.
3. Compile-time templates (Askama) have no Go equivalent that's as
   ergonomic.

**Java/Kotlin** was rejected because the whole point is to escape the
JVM footprint.

**Python** was rejected because performance under concurrent WS load
is the weak point we need to beat.

**Node** was considered for the natural alignment with HTMX
ecosystem, but the runtime memory cost is the same problem we have
with the JVM.

## Stability constraints

> Amendment (2026-07): the current **MSRV is Rust 1.94.0**. The workspace
> declares it once via `workspace.package.rust-version`; CI verifies the
> locked graph with that toolchain. Development, release builds, and the
> Docker builder use the pinned Rust 1.96.0 toolchain from
> `rust-toolchain.toml`. The original 1.75/no-rustup goal is now historical:
> the current latest-stable dependency policy (notably sqlx 0.9) made that
> floor impossible.

- **Current MSRV** (Minimum Supported Rust Version): 1.94.0.
- **Development/release toolchain**: 1.96.0, pinned separately from MSRV so
  contributors get deterministic rustfmt and clippy behavior.
- **Edition**: 2021. Edition changes remain independent from MSRV changes.
