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

- **Footprint**: a static binary around 15-25 MB, runtime memory in
  the 20-50 MB range for the proxy itself. ShinyProxy (JVM) sits at
  300-500 MB ocioso. For a portal serving 10-20 containers, this can
  make Ruscker fit on machines where ShinyProxy can't.
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

- **MSRV** (Minimum Supported Rust Version): 1.75. This is what
  Ubuntu 24.04 ships, and we want a no-rustup install path.
- **Edition**: 2021. Edition 2024 is not yet stable on MSRV.
- Several transitive deps need pinning (`getrandom 0.2.15`,
  `indexmap 2.7.0`, `clap 4.5.20`, `uuid 1.10.0`) because their
  newer versions require Edition 2024. This is documented in the
  workspace developer notes.

If MSRV ever bumps, we can lift the pins.
