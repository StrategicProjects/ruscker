# Release notes

What changed in each release. Ruscker follows semantic versioning; while
on `0.x` the API and YAML schema stay backward-compatible (new fields are
optional), and breaking changes are called out here.

Downloads (binaries, `.deb`, container image — all cosign-signed) are on
the [GitHub releases page](https://github.com/StrategicProjects/ruscker/releases).

---

## v0.1.3 — 2026-05-29

Admin & UX polish, plus proxy fixes that unlock notebook-style apps.

**Admin**
- **The spec form now edits every container option.** The *Advanced*
  section gained inner port, platform, environment variables and command
  override, registry credentials, per-app access groups/users, CPU/memory
  requests, max body size, scaling thresholds, routing strategy,
  placement, and anti-affinity — each with a `?` help bubble that states
  the default a blank field inherits.
- **Self-service card images.** Pick a logo (or cover) from the media
  library or **upload one inline**, right in the spec form — no need to
  leave for the media page or type an `/assets/img/...` path. Pasting a
  custom path or external URL still works.

**Apps & proxy**
- **Per-app environment and command.** `container-env` (a `NAME: value`
  map) and `container-cmd` (an argument list) are honored, ShinyProxy-
  compatible. Values flow through `${VAR}` interpolation, so secrets stay
  in the environment. This is what lets you configure notebook servers.
- **Jupyter (and similar) now work behind the proxy.** The `/app/{id}`
  URL rewriter handles apps that own the `/api/` namespace — Jupyter's
  REST API and kernel WebSocket — and rewrites redirect `Location`
  headers, so a notebook loads and connects end-to-end under the mount.
- New **RStudio Server** showcase card; **R Markdown** is now a
  documentation link with a corrected logo.

**Fixes**
- `ruscker import` no longer deletes custom landing blocks.
- The admin **Logs** page renders only the most recent lines (fast on a
  long-lived server) with a **download-full-log** link; live follow is
  unchanged.
- A spec that keeps failing to start (typo'd image, registry down) is now
  logged **once**, then quieted, then re-surfaced if it persists —
  instead of one warning on every scaler tick.

## v0.1.2 — 2026-05-27

High-availability / multi-host hardening and sub-path mounting.

- **Mount under a sub-path.** `server.context-path` (ShinyProxy-
  compatible) or the `--base-path /box` flag serves the whole portal
  under a prefix, for reverse proxies that can't give Ruscker its own
  subdomain. Health probes stay at the root for load balancers.
- **Fully-public portals** can hide the sign-in entrance with
  `landing-customization.show-admin-link: false`.
- **Multi-host robustness**: authoritative placement pruning, idempotent
  stop, bracketed IPv6 host literals, and a degraded start when a Docker
  host is unreachable (fails only if none connect).
- **HA leader hardening**: timeouts on every step of the Postgres
  advisory-lock leader path so a degraded database can't freeze a scaler
  tick; idle-session eviction is leader-gated.
- **HA sign-in**: the deploy guide prescribes a sticky upstream for the
  session-bearing paths.

## v0.1.1 — 2026-05-26

Per-user visibility and HA session accounting.

- **Per-group / per-user app visibility.** `access-groups` and
  `access-users` (ShinyProxy-compatible) scope who can see and reach an
  app. The landing shows each viewer only the apps they may use, and
  `/app` + `/api` **enforce** it (an anonymous visitor is redirected to
  sign in; a restricted API returns 403) — not just hide the card. Users
  and their group memberships are managed in the admin panel.
- **HA Postgres session accounting** fixes so a load-balancer failover
  counts active sessions correctly and a graceful drain can complete.

## v0.1.0 — 2026-05-26

First stable release.

- **Public landing page** rendered from your config — cards with filters,
  search, theming, custom branding/SEO/analytics, custom HTML blocks, and
  full **i18n** (pt-BR / en-US / es-ES / fr-FR).
- **Admin panel**: spec CRUD with a live card preview, an image library
  (upload → WebP), an encrypted credentials store, a landing editor, and
  an audit log.
- **Reverse proxy + Docker backend**: on-demand container spawn, sticky
  sessions, WebSocket proxying, per-spec CPU/memory limits, auto-scaling
  with two-sided hysteresis, and session-heartbeat reaping.
- **Monitoring dashboard** with live (SSE) per-replica CPU/memory and
  sparklines, a logs viewer, and per-replica stop/restart.
- **Accounts & security**: user accounts with roles (Viewer / Editor /
  Admin), login rate-limiting, security headers, `/healthz` + `/readyz`
  probes, and graceful shutdown.
- **Migration-friendly**: ShinyProxy-compatible YAML with a
  `validate --strict-compat` pre-flight, and `import` / `export` that
  round-trip YAML ↔ the database.
- **Distribution**: multi-arch container image, `.deb` packages, and
  static musl tarballs — all **cosign-signed**.
