# Ruscker

**Ruscker** is a lightweight Rust alternative to **ShinyProxy** and
**Shiny Server Free**. It hosts and load-balances containerized
interactive web apps — R/Shiny, Streamlit, Dash, Voilà — and stateless
HTTP APIs (Plumber2, FastAPI) behind a single proxy, with a custom
landing page and a real admin panel.

It ships as a **single static binary, no JVM** — so the idle footprint
is megabytes, not hundreds of megabytes, and startup is instant.

## Why

ShinyProxy is mature but heavy: a JVM that idles at hundreds of MB,
slow to start, configured by hand-editing YAML and restarting. Shiny
Server Free doesn't isolate sessions or scale. Ruscker keeps
ShinyProxy's YAML schema (so migration is friction-free) and adds a
proper admin panel, a monitoring dashboard, and load balancing.

## In production

Ruscker runs in production at the Pernambuco state government (SEPE),
serving the **Monitoramento Estratégico** portal. Migrated side-by-side
with the existing ShinyProxy (now reachable at `/sp/` for comparison),
the idle footprint dropped from:

> **540 MB (ShinyProxy / JVM) → ~16 MB (Ruscker)** — roughly a 30×
> reduction, on the same machine, serving the same 31 apps.

The real 31-spec ShinyProxy 3.2.0 config parsed with **no unsupported
features** and apps spawn on demand.

## What's in the box

- **Reverse proxy + load balancer** with sticky sessions, WebSocket
  forwarding, per-spec replica pools and an auto-scaler.
- **Container backend** (Docker) that spawns app containers on demand
  and reaps idle ones.
- **Admin panel** — apps CRUD, image/media library, encrypted
  credentials store, landing-page editor (colors, intros, SEO, social
  meta, analytics, custom HTML blocks), audit log, and a live
  monitoring dashboard.
- **Operations**: `/healthz` + `/readyz` probes, graceful shutdown,
  structured (JSON) logging, per-API rate limiting + CORS, request
  body-size limits.
- **Distribution**: a multi-stage Docker image and a Debian package
  with a hardened `systemd` unit.

## Where to next

- [Installation](./installation.md) — Docker or the `.deb`.
- [Migrating from ShinyProxy](./migrating.md) — point Ruscker at your
  existing `application.yml`.
- [Configuration](./configuration.md) — the full YAML reference.
- [The admin panel](./admin.md) — what each screen does.
- [Deploying in production](./deploying.md) — systemd + nginx,
  side-by-side with ShinyProxy.
