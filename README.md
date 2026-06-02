<picture>
  <source media="(prefers-color-scheme: dark)"
          srcset="crates/ruscker-admin/assets/brand/ruscker-lockup-horizontal-dark.svg">
  <img src="crates/ruscker-admin/assets/brand/ruscker-lockup-horizontal.svg"
       alt="Ruscker" height="56">
</picture>

A lightweight Rust alternative to **ShinyProxy** and **Shiny Server
Free**. Serves and load-balances containerized interactive web apps
(R/Shiny, Streamlit, Dash, Voilà) and stateless HTTP APIs (Plumber2,
FastAPI) behind a single proxy, with a custom landing page and an
admin panel.

It ships as a **single static binary, no JVM** — so the idle footprint
is megabytes, not hundreds of megabytes, and startup is instant.

📖 **Documentation: <https://strategicprojects.github.io/ruscker/>**
(install · migrating from ShinyProxy · configuration · admin · deploy).

<p align="center">
  <img src="book/src/images/landing.png" alt="The Ruscker landing page: a portal of app cards with a Featured carousel at the top, plus search and type/access filters." width="860">
</p>

## How it works

Visitors and API clients hit one Ruscker process. It serves the landing
page and admin UI, and reverse-proxies each request to the right app
container — picking a replica, keeping Shiny sessions sticky, upgrading
WebSockets, and rewriting URLs. When no replica can take the load (and
the spec allows it), Ruscker asks the Docker daemon to spawn one; idle
containers are reaped automatically.

<p align="center">
  <img src="docs/architecture.svg" alt="How Ruscker works: browsers and API clients reach one Ruscker binary, which reverse-proxies to app containers it spawns on demand through the Docker daemon." width="640">
</p>

## Why Ruscker

- **ShinyProxy** is mature but heavy: a JVM that idles at hundreds of
  MB, slow to start, configured by hand-editing YAML and restarting.
- **Shiny Server Free** doesn't isolate sessions or scale.
- **Both** have weak admin UIs ("edit YAML and restart").

Ruscker keeps ShinyProxy's `application.yml` schema (so migration is
friction-free) and adds a real admin panel, a live monitoring
dashboard, and load balancing on top.

|                        | ShinyProxy   | Shiny Server Free | **Ruscker**          |
|------------------------|--------------|-------------------|----------------------|
| Runtime                | JVM          | R process         | single Rust binary   |
| Idle memory            | 300–500 MB   | —                 | **~14 MB**           |
| Dependencies           | JVM          | R                 | none (static binary) |
| Web admin panel        | ✗            | ✗                 | ✓                    |
| Live monitoring dashboard | ✗         | ✗                 | ✓                    |
| Session isolation      | ✓            | ✗                 | ✓                    |
| Auto-scaling           | manual       | ✗                 | automatic            |
| Native HTTP APIs       | limited      | ✗                 | ✓ (Plumber/FastAPI)  |
| ShinyProxy YAML        | —            | —                 | 100% compatible      |

## What it can serve

Ruscker runs **one container per session** for stateful apps and **one
per replica** for stateless APIs — so anything that runs in a container
and speaks HTTP or WebSocket fits, not just Shiny:

- **Interactive apps** — Shiny, Streamlit, Dash, Gradio, Panel, Voilà,
  Marimo, Pluto.jl.
- **APIs & model serving** — Plumber/Plumber2, FastAPI, Flask, plus
  BentoML, MLflow, vLLM, Ollama, Text Generation Inference.
- **Notebooks & IDEs** — JupyterLab, RStudio Server, `code-server`.
- **BI & GenAI UIs** — Superset, Metabase, Datasette, Stable Diffusion
  WebUI, Open WebUI.

The full catalogue (with the "works-with-caveats" and "not the right
tool" cases) is in
[**What Ruscker can serve**](https://strategicprojects.github.io/ruscker/use-cases.html).

## Status

**Production-ready and running in production.** Releases are
multi-arch (amd64 + arm64) and [cosign-signed]; the
[releases page](https://github.com/StrategicProjects/ruscker/releases)
has the current version. Where the JVM-based
stack it replaced idled at hundreds of megabytes, Ruscker idles in the
low tens:

[cosign-signed]: https://strategicprojects.github.io/ruscker/installation.html#verifying-release-artifacts

> **~540 MB → ~14 MB idle** — roughly a 38× cut, on the same machine
> serving the same apps. A real 31-spec config migrated with **no
> unsupported features**, and apps spawn on demand.

What's in the box:

- **Reverse proxy + load balancer** — sticky sessions, WebSocket
  forwarding, per-spec replica pools, an auto-scaler, and absolute-URL
  rewriting so unmodified Shiny/Streamlit apps work behind a subpath.
- **Container backend** (Docker) that spawns app containers on demand,
  applies per-container CPU/memory limits, and reaps idle ones.
- **Admin panel** — full apps CRUD (every spec field editable from the
  form, including API/scaling/resource/lifecycle settings, with inline
  help), image/media library, encrypted credentials store, a
  landing-page editor (colors, intros, SEO, social meta, analytics,
  custom HTML blocks), an audit log, **user accounts with Viewer /
  Editor / Admin roles**, and a live SSE dashboard (CPU/memory
  sparklines, live-follow logs, stop/restart).
- **Operations** — `/healthz` + `/readyz` probes, graceful shutdown,
  structured (JSON) logging, per-API rate limiting + CORS, request
  body-size limits, an opt-in Prometheus `/metrics` endpoint, and
  `validate --strict-compat` migration pre-flight.
- **Distribution** — a multi-arch Docker image, a Debian package with a
  hardened `systemd` unit, static musl tarballs, a Homebrew tap, and
  cosign-signed release artifacts.

300+ unit + integration tests run green on `cargo test` (no Docker
required); extra feature-gated suites exercise a real Docker daemon
(`docker-it`) and a full proxy + WebSocket end-to-end run (`e2e`).

<p align="center">
  <img src="book/src/images/admin-apps.png" alt="The admin Apps table: each spec row with kind/state, and a featured star in the Actions column to toggle landing-page highlighting inline." width="860">
  <br><em>Apps catalogue — every spec field editable from the web UI, no YAML.</em>
</p>

<p align="center">
  <img src="book/src/images/admin-dashboard.png" alt="The live monitoring dashboard: aggregate cards plus a per-replica table with CPU sparklines, memory, sessions and stop/restart/logs actions, streamed over SSE." width="860">
  <br><em>Live monitoring dashboard — per-replica CPU/memory over Server-Sent Events.</em>
</p>

## Install

Every tagged release publishes a multi-arch Docker image, `.deb`
packages, and static `musl` tarballs (amd64 + arm64).

### Quick install (Linux, amd64/arm64)

```bash
curl -fsSL https://raw.githubusercontent.com/StrategicProjects/ruscker/main/install.sh | sh
```

Picks the right artifact from the latest release: the `.deb` (with the
`systemd` unit + an auto-generated admin token) on Debian/Ubuntu, or the
static binary into `/usr/local/bin` elsewhere. Pin a version or target
dir with `./install.sh vX.Y.Z` (any tag from the releases page) or
`PREFIX=~/.local/bin ./install.sh`.

### Homebrew (macOS / Linux)

```bash
brew install strategicprojects/tap/ruscker
```

Linux pulls the prebuilt musl binary; macOS builds from source.

### Docker

```bash
docker run --rm -p 8080:8080 \
  -v "$PWD/application.yml:/etc/ruscker/application.yml:ro" \
  -v /var/run/docker.sock:/var/run/docker.sock \
  ghcr.io/strategicprojects/ruscker:latest \
  serve --config /etc/ruscker/application.yml --bind 0.0.0.0:8080 --docker
```

### Debian / Ubuntu

```bash
sudo apt install ./ruscker_<version>_amd64.deb
# Creates a `ruscker` user, installs a systemd unit, and prints a
# freshly-generated admin token on first install.
```

### From source

```bash
git clone https://github.com/StrategicProjects/ruscker
cd ruscker
cargo build --release          # rustup fetches the pinned toolchain on first run
./target/release/ruscker --help
```

## Quickstart

```bash
# Validate a config (and check ShinyProxy-feature compatibility)
ruscker validate examples/application.yml --strict-compat

# Run the portal + admin + proxy with the Docker backend
RUSCKER_ADMIN_TOKEN=$(openssl rand -hex 16) \
RUSCKER_MASTER_KEY=$(openssl rand -hex 32) \
ruscker serve \
  --config examples/application.yml \
  --bind 127.0.0.1:8080 \
  --docker
```

`ruscker validate` on the bundled example prints something like:

```
  Ruscker config validation
  ─────────────────────────
  file: examples/application.yml
  title: Ruscker Demo Portal

  Specs: 8 total      shiny 5 · api 1 · external 2
  State: active 7 · inactive 1

  ✓ no warnings
  ShinyProxy compatibility: ✓ no unsupported features
```

### CLI subcommands

```bash
ruscker validate <path>              # parse + validate + report
ruscker validate <path> --strict-compat   # flag ShinyProxy features Ruscker ignores
ruscker serve   --config <path> [--docker] [--db <file>]   # run the portal
ruscker import  <path> --db <file>   # populate SQLite from YAML
ruscker export  --db <file>          # round-trip back to YAML
ruscker show    <path>               # render YAML with env vars interpolated
```

## YAML compatibility

Ruscker reads ShinyProxy's `application.yml` schema unchanged and adds
Ruscker-specific fields (API specs, replica pools, landing
customization) as you go. Migration is typically a one-line change in
your reverse proxy — point it at Ruscker on the same port and paths.
See [`docs/YAML_SCHEMA.md`](docs/YAML_SCHEMA.md) and the
[migration guide](https://strategicprojects.github.io/ruscker/migrating.html).

Credentials never go in YAML — use `${ENV_VAR}` interpolation; the
validator flags any plaintext secret it finds.

## Project layout

```
ruscker/
├── crates/
│   ├── ruscker-config/   # YAML schema + parsing + validation (pure)
│   ├── ruscker-core/     # traits, types, routing, replica registry (pure)
│   ├── ruscker-docker/   # Docker backend (bollard)
│   ├── ruscker-proxy/    # sticky cookie + WebSocket pump
│   ├── ruscker-admin/    # landing + admin + proxy routes (axum/askama)
│   └── ruscker-cli/      # the `ruscker` binary
├── book/                 # the documentation site (mdBook)
├── docs/                 # ARCHITECTURE, YAML_SCHEMA, SECURITY, ADRs, mockups
└── examples/
    └── application.yml   # a generic, comprehensive reference config
```

Each crate carries module-level docs (`cargo doc --open`) describing its
scope and how to extend it.

## Documentation

| Doc | Purpose |
|---|---|
| [Documentation site](https://strategicprojects.github.io/ruscker/) | Install, migrate, configure, deploy |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | System design + request flow |
| [`docs/YAML_SCHEMA.md`](docs/YAML_SCHEMA.md) | Full YAML reference |
| [`docs/SECURITY.md`](docs/SECURITY.md) | Threat model + hardening |
| [`docs/ROADMAP.md`](docs/ROADMAP.md) | Phased plan |
| [`docs/adr/`](docs/adr/) | Architecture decision records |
| [`docs/BRAND.md`](docs/BRAND.md) | Brand, teal palette, lockups |

## Development

```bash
cargo build
cargo test                                   # 300+ tests, no Docker needed
cargo test -p ruscker-docker --features docker-it   # + real Docker daemon
cargo test -p ruscker-cli --features e2e            # + full proxy/WS e2e
cargo clippy --all-targets -- -D warnings
./scripts/i18n-check.sh                      # enforce locale key parity
```

> Hand-format only the lines you touch — don't run `cargo fmt`
> crate-wide. `main` isn't fmt-clean under the current rustfmt, so a
> workspace `cargo fmt` produces a large unrelated diff; `cargo fmt
> --check` is intentionally **not** part of the gate.

## License

Apache-2.0
