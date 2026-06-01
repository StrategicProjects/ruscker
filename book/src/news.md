# Release notes

What changed in each release. Ruscker follows semantic versioning; while
on `0.x` the API and YAML schema stay backward-compatible (new fields are
optional), and breaking changes are called out here.

Downloads (binaries, `.deb`, container image — all cosign-signed) are on
the [GitHub releases page](https://github.com/StrategicProjects/ruscker/releases).

---

## v0.1.33 — 2026-06-01

A bulk image cleanup on the disk panel, plus a documentation fix.

**Admin**
- The **Disk** panel gains a **"Remove unused" images** button: reclaim
  every image no container uses and no spec references, in one click.
  It complements the existing one-click "remove stopped containers" —
  and, like everything on the panel, it only touches that exact unused
  subset (never a host-wide `docker image prune`, never `--force`).

**Docs**
- The documented idle footprint is now ~14 MB (the measured value),
  down from the rounded ~16 MB.

---

## v0.1.32 — 2026-06-01

Admin disk management, a forced first-login password change, and a more
visible process log.

**Admin**
- New **Disk** panel (`/admin/disk`, Admin-only): list and remove
  Ruscker-managed containers, prune every stopped one in a click
  (label-scoped — it never touches a non-Ruscker container on the host),
  and remove images no container or spec uses. Reclaims the space left
  behind by scaled-down or crashed replicas and by apps you've deleted.
- Deleting an app now **reaps its containers** instead of leaving them
  running or stopped as orphans.
- New accounts must **change their password on first login** — the prompt
  can no longer be skipped, and a guard re-routes to it on every admin
  page until the change is done. The user-admin password fields are
  masked, with a reveal toggle.
- The Portal logos editor uses the same **image gallery picker** as the
  spec form — search, thumbnails, and inline upload, instead of a bare
  path field.

**Operations**
- A one-line **startup banner** (version, bind address, base path, Docker
  on/off, database, spec count) now appears in the admin Logs tab even at
  the default log level — so a fresh boot is visible without `-v`. The
  Logs tab also distinguishes "nothing logged yet" from "no log buffer".

**Docs**
- The handbook was refreshed to match the current release.

---

## v0.1.18–v0.1.31 — 2026-05-31

Demo images, credential unification, a redesigned media library, and portal logo support.

**Demo app images**
- **Dash**, **FastAPI**, and **Quarto** showcase cards now use dedicated
  fork images on Docker Hub (`milkway/ruscker-dash-demo`,
  `milkway/ruscker-fastapi-demo`, `milkway/ruscker-quarto-demo`).
  Dash and FastAPI serve at the container root (no
  `SHINYPROXY_PUBLIC_PATH` configuration needed). The Quarto demo is a
  static nginx image (~67 MB).

**Credentials store**
- The named-credential store now accepts a **pure `${VAR}` env-ref** as
  a password (stored verbatim, resolved only at pull time), in addition
  to the existing AES-encrypted literal. "Pure" means a whole-token
  `${VAR}` — a value with a literal prefix like `prefix${VAR}` is not
  stored verbatim; it is treated as a literal and AES-encrypted (security
  fix).
- The spec-form Registry section is now a **credential picker**; the
  inline domain/user/password fields are hidden back-compat fallbacks.

**Media library**
- Built-in logos are **seeded into the Media library** on first start
  (idempotent) — one unified gallery, no separate "Built-in logos"
  group. Each logo is **deletable** and shows an **"in use" badge**
  (cross-references spec logo/cover and landing logos).
- A **modal picker** in the spec form provides search, uploads, and
  inline upload without leaving the form; drag-and-drop is supported on
  both the modal and the media page.

**Portal header/footer logos**
- The landing editor supports logos in the **header and footer slots**,
  each with alignment (left/center/right), an optional click-through
  link, and a per-logo height.

**Security fixes**
- Username charset and credential-name charset are now validated,
  making credentials safely deletable.
- Admin password fields in the spec and user forms are now
  **masked / write-only**.

**Proxy**
- API requests (kind `Api`) are now routed by **in-flight request
  count** instead of seat count, and the in-flight guard spans the
  full streaming response body.

---

## v0.1.4–v0.1.17 — 2026-05-29

Live UX fixes, security hardening, and a performance pass.

**Live UX fixes**
- **Cold-start splash**: a loading screen appears on first navigation to
  an app while its container is starting.
- **RStudio Server**: the proxy injects the `X-RStudio-Root-Path` header
  so RStudio rewrites its own internal links correctly behind the mount.
- **`App` kind** added for notebook-style apps (Jupyter, RStudio) that
  don't fit the Shiny or plain API model.
- Relative font URLs fixed so icons resolve correctly when served under
  a sub-path.
- Alpine.js CSP flag corrected (`'unsafe-eval'`) so popovers and dynamic
  filters work.
- Version number shown in the admin footer.
- The admin **Blocks editor** is folded into the Portal settings page.

**Security hardening**
- Ruscker's own session cookies are stripped before forwarding requests
  to app containers — the admin session no longer leaks upstream.
- CSRF guard (Fetch-Metadata / Origin check) on all chrome-mutating
  actions.
- `${VAR}` secrets in `container-env` and registry passwords are
  preserved verbatim through import/export and resolved only at spawn
  or pull time — they never appear in cleartext in the database.
- Container log access is gated to Editor-and-above accounts.

**Performance**
- **gzip / Brotli compression** on all HTML, CSS, and JS chrome
  responses.
- **`?v={version}`** appended to bundled CSS/JS URLs for
  cache-busting on upgrade.
- **ETag** validation on `/assets/img` image responses — revalidation
  returns a cheap 304.
- **WebP thumbnails** in media galleries.
- Configurable **`proxy.metrics-interval`** (seconds) for dashboard
  stats polling; Docker stats fan-out is now bounded.
- Dashboard snapshot is memoized per locale across SSE tabs; the SSE
  patcher updates individual cells instead of replacing whole rows.

**Admin**
- Spec editing is now fully gated: specs that exist only in YAML (not
  the database) are shown read-only in `/admin/specs`.
- The media gallery at `/admin/media` is a client-side Alpine page with
  filename search and paginated "show more" (24 per page).

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
  compatible) or the `--base-path /portal` flag serves the whole portal
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
