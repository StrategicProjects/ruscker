<picture>
  <source media="(prefers-color-scheme: dark)"
          srcset="crates/ruscker-admin/assets/brand/ruscker-mark-knockout.svg">
  <img src="crates/ruscker-admin/assets/brand/ruscker-lockup-horizontal.svg"
       alt="Ruscker" height="56">
</picture>

A lightweight Rust alternative to **ShinyProxy** and **Shiny Server
Free**. Serves and load-balances containerized interactive web apps
(R/Shiny, Streamlit, Dash, Voilà) and stateless HTTP APIs (Plumber2,
FastAPI) behind a single proxy, with a custom landing page and an
admin panel.

```
                ┌──────────────────────┐
                │       Visitors       │
                └──────────┬───────────┘
                           │ HTTPS
                           ▼
                ┌──────────────────────┐
                │ Ruscker (single .bin)│
                │  ┌────────────────┐  │
                │  │  Landing page  │  │
                │  │   Admin panel  │  │
                │  │   HTTP+WS proxy│  │
                │  │   Auto-scaler  │  │
                │  └────────┬───────┘  │
                └───────────┼──────────┘
                            │ Docker API
                            ▼
                ┌──────────────────────┐
                │ App containers (3x)  │
                └──────────────────────┘
```

## Status

**Phase 0: scaffolding complete.** Parser and CLI are functional;
proxy, admin, and Docker backend are documented stubs.

What works **today**:

```bash
$ DOCKER_REGISTRY_PASSWORD=test cargo run --bin ruscker -- \
        validate examples/application.yml

  Ruscker config validation
  ─────────────────────────
  file: examples/application.yml
  title: Monitoramento Estratégico
  bind: 127.0.0.1:8080
  authentication: None

  Specs: 31 total
    external       14
    shiny          17

  State:
    active         28
    inactive       3

  ⚠ 1 warning(s):
    - spec hortensias_conseplan has no container-image but uses
      container-only fields
```

See [docs/ROADMAP.md](docs/ROADMAP.md) for the path from here to a
production-ready release.

## Why Ruscker

- **ShinyProxy** is mature but heavy (JVM, 300-500 MB ocioso, slow
  startup, old Thymeleaf templates).
- **Shiny Server Free** doesn't isolate sessions or scale.
- **Both** have weak admin UIs ("edit YAML and restart").

Ruscker is a single static binary (~20 MB), keeps ShinyProxy's YAML
schema for migration friction, and adds a real admin panel +
dashboard + load balancing on top.

## Project layout

```
ruscker/
├── CLAUDE.md                   # Project memory for Claude Code
├── README.md                   # ← you are here
├── Cargo.toml                  # Workspace root
├── crates/
│   ├── ruscker-config/         # ✅ YAML schema + parsing + validation
│   ├── ruscker-core/           # ✅ Traits, types, routing algorithms
│   ├── ruscker-docker/         # 🚧 Docker backend (stub, phase 3)
│   ├── ruscker-proxy/          # 🚧 HTTP+WS reverse proxy (stub, phase 3)
│   ├── ruscker-admin/          # 🚧 Admin web UI (stub, phase 2+4)
│   └── ruscker-cli/            # ✅ `ruscker` binary
├── docs/
│   ├── ARCHITECTURE.md
│   ├── ROADMAP.md
│   ├── YAML_SCHEMA.md
│   ├── adr/                    # Architecture decision records
│   └── mockups/                # HTML mockups of every UI screen
└── examples/
    └── application.yml         # Real-world ShinyProxy config (sanitized)
```

Every crate has its own `CLAUDE.md` documenting scope, conventions,
and how to extend it. Read those before touching code in that crate.

## Quickstart

### Requirements

- Rust **1.95** stable or newer (install via [rustup](https://rustup.rs/);
  the repo's `rust-toolchain.toml` pins the exact version)
- For phase 3+: Docker daemon access

### First build

```bash
git clone <this-repo> ruscker
cd ruscker

# Build everything (rustup will fetch the pinned toolchain on first run)
cargo build

# Run tests (24 unit + 9 integration against real YAML)
cargo test

# Validate a config
DOCKER_REGISTRY_PASSWORD=anything-for-now \
    cargo run --bin ruscker -- validate examples/application.yml
```

### CLI subcommands (current)

```bash
ruscker validate <path>              # parse + validate + report
ruscker validate <path> --json       # machine-readable
ruscker validate <path> --strict     # exit non-zero on warnings
ruscker show <path>                  # render YAML with envs interpolated
ruscker inspect <path>               # parsed Config as JSON
```

## YAML compatibility

Ruscker reads ShinyProxy's `application.yml` schema unchanged. Add
Ruscker-specific fields (API specs, replica pools) as you go. See
[`docs/YAML_SCHEMA.md`](docs/YAML_SCHEMA.md) for the full reference.

Migration is a one-line change in your reverse proxy config:

```diff
-  proxy_pass http://localhost:8080/;  # ShinyProxy
+  proxy_pass http://localhost:8080/;  # Ruscker (same port, same paths)
```

## Mockups

The visual direction is preserved in [`docs/mockups/`](docs/mockups/).
Open `docs/mockups/index.html` in a browser to see every admin and
landing screen with light/dark theme support. These are the source of
truth for design decisions — when implementing a screen, refer to its
mockup first.

## Documentation

| Doc | Purpose |
|---|---|
| [`CLAUDE.md`](CLAUDE.md) | Project memory for Claude Code |
| [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) | System design |
| [`docs/ROADMAP.md`](docs/ROADMAP.md) | Phased plan (8 phases) |
| [`docs/YAML_SCHEMA.md`](docs/YAML_SCHEMA.md) | YAML reference |
| [`docs/adr/0001-rust-as-language.md`](docs/adr/0001-rust-as-language.md) | Why Rust |
| [`docs/adr/0002-sqlite-source-of-truth.md`](docs/adr/0002-sqlite-source-of-truth.md) | Why SQLite over YAML at runtime |
| [`docs/adr/0003-sticky-sessions.md`](docs/adr/0003-sticky-sessions.md) | Session affinity rationale |
| [`docs/adr/0004-ui-stack.md`](docs/adr/0004-ui-stack.md) | Askama + HTMX + Tailwind 4 |
| [`docs/BRAND.md`](docs/BRAND.md) | Marca, paleta teal, lockups e regras de uso |
| [`docs/IMAGES.md`](docs/IMAGES.md) | Regras para imagens de cards |

## Continuing development

This codebase is set up to be continued with **Claude Code**. The
`CLAUDE.md` files (one at root + one per crate) explain status,
conventions, and the exact next steps. Open the repo in Claude Code
and ask, for example:

> "Begin phase 1 by implementing the landing page in `ruscker-admin`
>  using the mockup at `docs/mockups/landing-page.html`."

Phase 0 (this scaffolding) was built by Claude over a planning session
with hand-curated mockups. Phase 1+ can be done autonomously.

## License

Apache-2.0
