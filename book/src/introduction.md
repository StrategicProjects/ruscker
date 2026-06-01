<p align="center">
  <img class="ruscker-logo ruscker-logo-light" src="images/ruscker-lockup-horizontal.svg" alt="Ruscker">
  <img class="ruscker-logo ruscker-logo-dark" src="images/ruscker-lockup-horizontal-dark.svg" alt="Ruscker">
</p>

# Ruscker

**Ruscker** is a **portal and orchestrator** for containerized web
workloads behind one proxy. It handles two shapes:

- **Container-per-session** interactive apps — R/Shiny, Streamlit,
  Dash, Voilà, Jupyter, RStudio.
- **Container-per-API** stateless HTTP services — Plumber2, FastAPI.

It keeps a ShinyProxy-compatible YAML schema for low-friction migration,
ships as a **single static binary, no JVM**, and adds a real admin
panel, a live monitoring dashboard, and load balancing. Idle footprint
is megabytes, not hundreds of megabytes, and startup is instant.

## How it works

Visitors and API clients hit one Ruscker process. It serves the landing
page and admin UI, and reverse-proxies each request to the right app
container — picking a replica, keeping Shiny sessions sticky, upgrading
WebSockets, and rewriting URLs. When no replica can take the load (and
the spec allows it), Ruscker asks the Docker daemon to spawn one; idle
containers are reaped automatically.

<p align="center">
  <img src="images/architecture.svg" alt="How Ruscker works: browsers and API clients reach one Ruscker binary, which reverse-proxies to app containers it spawns on demand through the Docker daemon." width="640">
</p>

## Why

ShinyProxy is mature but heavy: a JVM that idles at hundreds of MB,
slow to start, configured by hand-editing YAML and restarting. Shiny
Server Free doesn't isolate sessions or scale. Ruscker keeps
ShinyProxy's YAML schema (so migration is friction-free) and adds a
proper admin panel, a monitoring dashboard, and load balancing.

## In production

Ruscker is on **v0.1.31** and runs in production today. Where the
JVM-based stack it replaced idled at hundreds of megabytes, Ruscker
idles in the low tens:

> **~540 MB → ~16 MB idle** — roughly a 30× cut.

A real 31-spec config migrated with **no unsupported features**, and
apps spawn on demand. Releases are multi-arch and **cosign-signed**;
the [Roadmap](./roadmap.md) tracks what's shipped and what's next.

## What's in the box

- **Reverse proxy + load balancer** with sticky sessions, WebSocket
  forwarding, per-spec replica pools, an auto-scaler, and URL rewriting
  (a generalized runtime shim patches `fetch`, `XMLHttpRequest`,
  `WebSocket`, `script.src`, `link.href`, and more) so unmodified apps
  work behind a sub-path.
- **Container backend** (Docker) that spawns app containers on demand,
  applies per-container CPU/memory limits, and reaps idle ones. Per-spec
  `container-env` and `container-cmd` let you configure notebook servers
  (Jupyter, RStudio) without custom images.
- **Admin panel** — apps CRUD with a full advanced form, a unified
  media library (built-in logos, uploads, drag-and-drop, "in use"
  badges), an encrypted credentials store (AES literal or `${VAR}`
  env-ref, resolved only at pull time), a landing-page editor (colors,
  intros, SEO, social meta, analytics, custom HTML blocks, header/footer
  logos with alignment and links), audit log, **user accounts with
  Viewer / Editor / Admin roles**, and a live monitoring dashboard
  (CPU/memory, live-follow logs, stop/restart).
- **Sub-path mounting**: serve the whole portal under a prefix via
  `server.context-path` (ShinyProxy-compatible) or `--base-path`. Health
  probes (`/healthz`, `/readyz`) stay at the root for load balancers.
- **Operations**: graceful shutdown, structured (JSON) logging, per-API
  rate limiting + CORS, request body-size limits, gzip/br compression,
  immutable-versioned static assets, and an opt-in Prometheus `/metrics`
  endpoint.
- **Distribution**: a cosign-signed multi-arch container image
  (`ghcr.io/strategicprojects/ruscker`), a Debian package with a
  hardened `systemd` unit, static musl tarballs, and a Homebrew tap.

## Where to next

- [Quickstart](./quickstart.md) — from zero to a running app in minutes.
- [What Ruscker can serve](./use-cases.md) — Shiny, Streamlit, Dash,
  FastAPI, JupyterLab, LLM UIs, BI tools, and more.
- [Ruscker vs. alternatives](./alternatives.md) — how it compares to
  ShinyProxy, Shiny Server, Posit Connect, and JupyterHub.
- [Installation](./installation.md) — Docker, the `.deb`, or `brew`.
- [Migrating from ShinyProxy](./migrating.md) — point Ruscker at your
  existing `application.yml`.
- [Configuration](./configuration.md) — the full YAML reference.
- [The admin panel](./admin.md) — what each screen does.
- [Deploying in production](./deploying.md) — systemd + nginx,
  side-by-side with ShinyProxy.
- [Roadmap](./roadmap.md) — shipped phases and what's planned.
