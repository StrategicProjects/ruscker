# Release notes

What changed in each release. Ruscker follows semantic versioning; while
on `0.x` the API and YAML schema stay backward-compatible (new fields are
optional), and breaking changes are called out here.

Downloads (binaries, `.deb`, container image — all cosign-signed) are on
the [GitHub releases page](https://github.com/StrategicProjects/ruscker/releases).

---

## v0.1.49 — 2026-06-03

Access-counter follow-ups.

**Admin**
- The Accesses column now shows a small **daily-usage sparkline** (last
  14 days) next to each total.
- **API specs are counted too** — one access per call (they aren't
  session-based, so each request is the access).

---

## v0.1.48 — 2026-06-03

A built-in access counter.

**Admin**
- An **"Accesses" column** in the Apps table shows how many times each
  card/app has been used. App visits are counted once per session (not
  per request), and **external-link cards** are counted too — clicks now
  route through the portal so Ruscker can see them. Direct `/app/{id}`
  URLs that skip the landing still count. No external analytics needed.

---

## v0.1.47 — 2026-06-03

Better diagnostics when an app won't start.

**Proxy / Docker**
- When a spawned container **crashes on startup** (e.g. an app that halts
  because it can't reach its database), Ruscker now **fails fast** —
  reporting `exited (code N) during startup` instead of waiting out the
  full 60s readiness timeout — and **attaches the tail of the container's
  logs** to the failure. The real cause (a DB connection error, a missing
  env var, a crash) is visible in the warn log and the admin Logs tab
  without re-running the container by hand.

---

## v0.1.46 — 2026-06-02

Admin catalog on by default, plus a session-revocation fix.

**Packaging**
- The Debian/systemd unit now runs with **`--db` enabled by default**, so
  a fresh `.deb` install has the **admin panel live out of the box** and
  seeds the showcase apps on first boot — no YAML editing. The Docker
  backend stays opt-in (`sudo ruscker-enable-docker`).

**Security**
- Changing a user's role, **deleting** them, or resetting their password
  now **revokes their live admin sessions immediately**, instead of
  leaving the old (possibly elevated) role valid until the session
  expired.

**Docs**
- Configuration is reframed around **two layers** — portal content
  (managed in the admin panel) vs deployment settings (CLI flags / env),
  with the YAML schema as the migration reference — and the quickstart
  now leads with the `--db` showcase seed. Screenshots throughout the
  site and the README.

**Landing**
- The Featured carousel is now **centered on the page**, with the
  prev/next chevrons in the **side gutters outside the cards** (rather
  than overlaid on them), vertically centered.

---

## v0.1.44 — 2026-06-02

A refreshed Featured carousel.

**Landing**
- **New carousel controls.** The prev/next chevrons are now circular
  buttons overlaid on the card row, vertically centered on the left and
  right edges (Material-Tailwind style), instead of a pair of buttons in
  the section header. They stay pinned to the visible cards whether 1, 2
  or 3 fit, and disappear when everything fits on one page.
- **Fixed a hover clip.** A featured card's dark top border no longer
  gets shaved off when you hover it inside the carousel.

---

## v0.1.43 — 2026-06-02

Another featured-star placement fix.

**Admin**
- The featured star now lives **inside the Actions column**, alongside
  edit and duplicate, instead of in a separate column of its own.

---

## v0.1.42 — 2026-06-02

A follow-up fix for the featured star.

**Admin**
- **The featured star now fills in when toggled on.** The served icon
  font ships only the outline star, so the "featured" state rendered an
  empty glyph — the star appeared to vanish on click. It's now drawn as
  an inline SVG that toggles solid ↔ outline, so featuring an app shows a
  solid amber star as intended.

---

## v0.1.41 — 2026-06-02

Bug fixes for the admin Apps table plus Homebrew automation.

**Admin**
- **Featured star now works in the Apps table.** The list page never
  loaded Alpine, so the inline star rendered empty and didn't toggle;
  it's loaded now. The star also moved next to the **Actions** column,
  where featuring reads as a row action.

**Packaging**
- **Dropped the obsolete `welcome` starter spec** from the default
  `/etc/ruscker/application.yml`. It predated the first-run showcase
  seed, so on a fresh install it only duplicated a card and showed up
  as a stray read-only CONFIG row in the admin. Fresh installs are now
  clean (the showcase seed fills the landing).

**CI**
- The release workflow now **auto-publishes the Homebrew formula to the
  tap** on every release, so `brew install strategicprojects/tap/ruscker`
  tracks the latest version instead of drifting. (Requires a
  `HOMEBREW_TAP_TOKEN` secret; no-ops with a warning if absent.)

---

## v0.1.40 — 2026-06-02

Two admin UX touches for managing apps.

**Admin**
- **Inline featured star** in the Apps table: toggle an app's Featured
  flag straight from the list with a single click — solid star when on,
  outline when off — without opening each app's editor.
- **App form reorganized into three bands** so the layout maps to
  intent: Identity (the essentials), a visible **Metadata & visibility**
  band (Featured, access groups/users, updated date), and the
  **Advanced** collapse for runtime knobs. Per-session tuning
  (`seats-per-container`, `max-lifetime`) moved into Advanced; the
  Container card is now just "which image to run".

---

## v0.1.39 — 2026-06-02

Polish for the Featured carousel and subject pills.

**Landing**
- The **Featured carousel** is now paged: at most 3 cards with prev/next
  chevrons (shown only when there are more than three), and no horizontal
  scrollbar.
- The **subject pill** on a card fits its full text and uses a lighter,
  theme-matching style.

---

## v0.1.38 — 2026-06-02

A round of admin & landing UX polish.

**Landing**
- A **"Featured" carousel** of highlighted apps above the filters. Mark
  an app `featured` and toggle "Show Featured carousel" in the Portal
  editor; it only appears when both are set, and is a horizontal rail
  (1–3 cards per viewport, the rest scroll).
- Each card now shows its **subject** as a pill next to the type badge.

**Admin**
- The **Add/Edit App form** is reorganised into labelled section cards
  with a sticky Save bar, matching the Portal editor.
- The registry **credential** field is a real selector now, with a clear
  "no saved credentials" hint when the store is empty.
- A read-only **Groups** page derives each group's member users and the
  apps it gates (from `access-groups`), so you can spot typos and see who
  can use what.

---

## v0.1.37 — 2026-06-01

Catch a bad image in the editor, not at the first failed launch.

**Admin**
- The spec editor's **container image** field gains a **Check** button: it
  asks the backend whether the image is already on the server and shows
  ✓ on the server / ⬇ will be pulled on first launch (or flags a `${VAR}`
  that resolves at pull time / Docker not connected).
- When the image is absent, a **Pull** button fetches it right away and
  streams the daemon's progress live; on completion the indicator settles
  on present (success) or absent with the error line (failure). Private
  images use the spec's selected registry credential.

---

## v0.1.36 — 2026-06-01

A favicon fix for Safari and a cleaner uninstall.

**Admin**
- Every page now ships the **same favicon set** — the standalone login
  and setup screens previously linked only the SVG icon, which Safari can
  ignore (leaving a dark placeholder when moving between admin and the
  landing). The icon links live in one shared partial, and a dedicated
  monochrome `safari-pinned-tab.svg` backs the Safari pinned-tab icon.

**Packaging**
- `apt purge ruscker` now removes `/etc/ruscker` too (config + the admin
  token / keys in `ruscker.env`), so a purge leaves no trace. The
  installation chapter documents the full **uninstall & reset** matrix
  (remove vs purge vs purge+install vs a data-only DB wipe).

---

## v0.1.35 — 2026-06-01

Live dashboard fixes behind a reverse proxy, plus smarter share images.

**Admin**
- The **live dashboard** now streams through reverse proxies: SSE
  responses send `X-Accel-Buffering: no`, so new containers show up in
  real time even behind nginx on a subpath mount (no nginx change
  needed). Previously the table could appear frozen until a reload.
- **Social share image (`og:image`) auto-defaults**: when left blank it
  reuses the header (left) logo, else the Ruscker mark — so a shared
  link carries the portal's identity without setting it twice. The
  editor field also gets the gallery picker.
- The Safari pinned-tab `mask-icon` points at the monochrome mark
  (correct for a recoloured silhouette).

---

## v0.1.34 — 2026-06-01

Docker connects out of the box, per-theme colours, and a modernised
Portal editor.

**Runtime**
- Ruscker now **auto-connects to Docker** when the daemon socket is
  reachable — `serve` spawns app containers with no `--docker` flag.
  Pass `--no-docker` to run landing-only, or keep `--docker` to make a
  failed connect fatal (useful for a remote daemon).
- Showcase demos seed with `min-replicas: 0`, so a fresh install no
  longer pre-spawns every demo container at boot — they cold-start on
  first click.

**Portal**
- **Per-theme colours**: set the background, text and accent for the
  light and dark themes independently in the landing editor. Blank keeps
  the built-in default.
- **Logos integrate into the chrome**: a header-left logo replaces the
  Ruscker mark, header-right sits after the buttons, footer-right trails
  the version, and a center logo is centred within the header/footer bar
  itself. Each logo also takes an optional **margin**.

**Admin**
- The **landing editor** is reorganised into labelled section cards with
  a sticky Save bar; logos are edited as cards with segmented
  position/alignment pickers; the live preview now mirrors the real
  portal chrome (logos + footer). Theme colour swatches show the theme
  default instead of black when unset.

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
