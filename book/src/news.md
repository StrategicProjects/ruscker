# Release notes

What changed in each release. Ruscker follows semantic versioning; while
on `0.x` the API and YAML schema stay backward-compatible (new fields are
optional), and breaking changes are called out here.

Downloads (binaries, `.deb`, container image — all cosign-signed) are on
the [GitHub releases page](https://github.com/StrategicProjects/ruscker/releases).

---

## v0.1.89 — 2026-06-07

Reliable cold starts for scale-to-zero interactive apps (#686).

**Fixes**
- A `min-replicas: 0` interactive app (e.g. an IDE with
  `seats-per-container: 1`) could fail to open: the scaler reaped the
  replica it had just spawned for the arriving visitor before they
  finished the cold-start splash and claimed a seat, leaving them on a
  dead/again-cold app. A freshly-ready replica is now exempt from idle
  scale-down for a short grace, so the visitor reliably lands on it.
  Single-user IDEs can still pin `min-replicas: 1` to stay warm.

---

## v0.1.88 — 2026-06-07

A spurious "upstream error" on the first open of an interactive app is
fixed (#683).

**Fixes**
- Opening RStudio Server (or any interactive app) could show a bare
  **"upstream error"** on the first navigation, then work on a retry.
  Cause was a hyper connection-pool race: app servers close idle
  keep-alive connections quickly, and the proxy could dispatch a request
  onto a socket the app had already closed. The proxy now evicts idle
  pooled connections promptly and retries an idempotent (GET/HEAD)
  forward once on a fresh connection, so the first open just works.

---

## v0.1.87 — 2026-06-07

The redesign's perceived-speed primitives are now live (#623).

**UI**
- **Top navigation progress bar** — a thin teal bar grows while a page
  navigation (or form submit) is in flight, across the admin and the
  public portal. Honors `prefers-reduced-motion`.
- **Content reveal** — admin pages fade in as they load, and the portal
  card grid cascades in with a small per-card delay.
- **Shimmer skeletons** — a replica's CPU/memory cell shows a shimmer
  while its first live reading is pending, instead of a bare dash, so a
  loading value reads as loading rather than empty.

These wire up the perceived-speed primitives the #623 handoff defined,
completing the Design System pass.

---

## v0.1.86 — 2026-06-07

Web apps pack more sessions per container by default.

**Behaviour**
- `seats-per-container` now defaults to **10** for web-framework apps
  (Shiny, Streamlit, Dash, Voilà) — they serve many concurrent sessions
  from one process, so a container per visitor was wasteful (the demo
  Shiny showed "1/1"). APIs keep 100. **Single-user IDEs** (RStudio,
  Jupyter) are the exception: set `seats-per-container: 1` on those so each
  visitor gets an isolated container, with concurrency from `max-replicas`.
- The app-editor's greyed hints now match the real defaults
  (sessions/container 10, min-replicas 1, max-replicas 5).

## v0.1.85 — 2026-06-07

Apps auto-scale to a few independent containers by default.

**Behaviour**
- A container app that doesn't set `max-replicas` now defaults to **5**
  (was effectively 1). So a single-seat interactive app (RStudio, Jupyter,
  Shiny) serves up to 5 concurrent visitors — one isolated container each,
  started on demand — instead of locking everyone out after the first.
  Set `max-replicas` per app to raise it (busier app) or lower it
  (constrained host); `External` apps are unaffected.

## v0.1.84 — 2026-06-07

Tell visitors when an app is full instead of an endless "Starting…".

**Interface**
- When an app is at its replica ceiling with every seat taken, a new
  visitor used to see the same "Starting…" page and wait forever, as if
  the container were perpetually booting. The waiting page now detects
  this: while the app can still scale it shows "Starting…" as before, but
  at capacity it says "<app> is full right now — this page opens
  automatically as soon as one frees up." Both keep polling, so the visitor
  is let in the moment a seat frees.

## v0.1.83 — 2026-06-07

Fix runaway session counts on single-seat interactive apps.

**Fixes**
- A single browser visit to a `seats: 1` app (RStudio, Jupyter) could
  inflate `sessions_active` to 7–9 and climbing, filling the seat and
  trapping the visitor (and any second visitor) on the starting splash.
  An app's `crossorigin` script bundles and credential-less requests
  arrive without the sticky cookie, and each was being counted as a new
  session. Now only a real visit — a top-level page navigation — opens a
  session and takes a seat; subresources ride the existing replica without
  counting. This also makes `max-replicas` scale-out behave: N concurrent
  visitors now map to N containers instead of one visit spawning several.

## v0.1.82 — 2026-06-07

Fix single-seat apps (RStudio, Jupyter) getting stuck on the starting
splash.

**Fixes**
- A `seats: 1` interactive app could trap the visitor on the "Starting…"
  splash forever, even with the container up: the first request reserved
  the app's only seat for that session, so the app's own follow-up
  navigation (RStudio → its sign-in page, Jupyter → its lab) re-entered
  the splash gate, found no *free* seat, and was shown the splash again —
  waiting on the seat it already held. The splash now lets a session that
  already holds a seat on a ready replica proxy straight through.

## v0.1.81 — 2026-06-06

The hi-fi design system reaches every admin screen, plus a new live YAML
import editor.

**Interface (#623)**
- **Disk** — a usage hero (host disk used / total with a stacked bar) over
  two side-by-side panels for the Ruscker-managed container images and
  containers, each with inline prune actions and an "in use" cross-check.
- **Apps** — a sticky filter toolbar (search + kind chips with live counts
  + a sort cycler). Access-group badges now use a fixed palette so the
  canonical roles always read the same colour across Apps, Users and
  Groups; `public` apps show in teal.
- **App editor** — boolean options became switch toggles; replicas a
  −/＋ stepper; CPU and memory sliders (with the text field still the
  source of truth); environment variables an add/remove `KEY = value`
  repeater; access groups a pill picker (custom names still allowed); and
  the live card preview gained a resources/scale summary.
- **Appearance editor** — flatter section headers and live character
  counters on the SEO title/description.
- **Dashboard** — a pulsing "Live" badge and a filter band (search +
  Ready/Starting/Draining/Stopped chips) over the grouped replica view.
- **Users / Groups** — the user list moved to the shared rounded table;
  Groups gained a "Public apps" rail listing every app open to everyone.
- **Media** — the gallery search is now the rounded search pill.
- **Audit log** — a sortable table with coloured actor avatars and
  colour-coded actions; the change diff stays available as an expandable
  row.
- **Logs** — a "Live" badge on the server log, and the per-replica
  container tail now shares the terminal styling.
- **Import YAML** — a live two-pane editor: edit or paste YAML on the
  left and watch the parsed apps (each marked new or update, selectable)
  refresh on the right as you type. Parse errors show inline; nothing is
  written until you confirm.

**Fixes**
- The Groups "Public apps" rail no longer lists apps that are gated to
  specific users as public.
- Restored a few status colours that weren't rendering (the warning/ok
  accents behind the SEO over-limit counter, the disk high-usage figure,
  reclaimable-row tints and in-use badges).
- The app-editor summary now shows the heartbeat timeout in minutes, and
  the CPU/memory sliders no longer write a spurious zero when dragged
  fully left.
- The import editor's empty-state hint no longer breaks under French.

## v0.1.80 — 2026-06-06

Fix interactive apps getting stuck on the starting splash.

**Fixes**
- A container that exited unexpectedly could leave a stale replica behind
  that blocked new launches of that app and trapped visitors on the
  "Starting…" page — most visibly on single-seat apps like RStudio and
  Jupyter. The scaler now prunes replicas whose container is no longer
  running each tick, freeing their seats.

## v0.1.79 — 2026-06-06

The redesign reaches the Apps table and the dashboard.

**Interface**
- The admin Apps table now shows each app's framework logo next to its
  name and an Access column with colour-coded group badges (or "public")
  (#623).
- The monitoring dashboard's collapsed app rows now summarise sessions, CPU
  and memory with little meters, and a toolbar adds an expand/collapse-all
  control (#623).

## v0.1.78 — 2026-06-06

Fix a cold-start splash that could loop.

**Fixes**
- A single-seat interactive app (e.g. RStudio) whose seat was already taken
  could trap a new visitor in a reloading "Starting…" splash. The readiness
  probe and the splash gate now use the same check, so the page advances
  exactly when the app can accept the visitor (#582 follow-up).

## v0.1.77 — 2026-06-06

The UX redesign reaches every admin screen.

**Interface**
- The Appearance live preview now shows a search bar and mock cards tinted
  with the configured accent colour, not empty boxes (#623).
- Each Groups card has a colour-coded accent bar (matching its badge colour
  elsewhere), and each Credentials row shows a key-icon tile (#623).

## v0.1.76 — 2026-06-06

Disk usage at a glance; a proper log viewer.

**Interface**
- The disk panel opens with a usage hero — total used / capacity, a
  percentage, and a stacked bar split into Ruscker images, other used and
  free (real host figures) (#623).
- The server-logs tab is now a colour-coded live terminal: lines are tinted
  by level, with level and free-text filters and a pause/resume control
  (#623).

## v0.1.75 — 2026-06-06

More UX-redesign polish.

**Interface**
- Admin action confirmations now appear as floating toasts that dismiss
  themselves (#623).
- The SEO editor shows a live Google-style search-result preview that
  updates as you edit the title and description (#623).

## v0.1.74 — 2026-06-06

More of the UX redesign.

**Interface**
- Users page: each account shows a coloured avatar with its initials, and
  its groups render as coloured badges (#623).
- Restyled form controls — range sliders (teal thumb), the featured-carousel
  toggle (an on/off switch) and the YAML-import checkboxes (#623).

## v0.1.73 — 2026-06-05

Cold-start apps spawn again; syntax-highlighted code editors.

**Fixes**
- An app with `min-replicas: 0` (cold start) and no `max-replicas` could
  never start a container — the default resolved to `max-replicas: 0`, so
  the on-demand spawn was a no-op and the booting splash hung. The default
  now floors at 1 for containerized apps (#623/#582).

**Interface**
- The Appearance custom-CSS / analytics-HTML editors and the custom HTML
  blocks editor now have VS Code-style live syntax highlighting (#623).

## v0.1.72 — 2026-06-05

Fix a cold-start splash that could hang.

**Fixes**
- The "container is booting…" page could poll forever on a busy
  single-seat app even after the container was up and serving, leaving the
  visitor stuck. The readiness probe now advances as soon as the app is
  ready, regardless of seat occupancy (regression from v0.1.66; #582).

## v0.1.71 — 2026-06-05

Dashboard redesign: replicas grouped by app.

**Interface**
- The monitoring dashboard now groups replicas into one expandable card per
  app instead of a flat table. Each card's header shows the app, its replica
  count, the worst replica's state, and total sessions; expand it to see the
  per-replica detail and the restart/stop/logs actions. The headline KPIs
  were restyled to match (#623).

## v0.1.70 — 2026-06-05

First slices of the UX redesign.

**Interface**
- The public portal's search and filters now stay pinned to the top while
  the catalog scrolls under them (#623).
- The monitoring dashboard's headline numbers count up to their value on
  load (honoring reduced-motion) (#623).
- Groundwork for the redesign: shared shadow tokens and
  perceived-performance primitives (top progress bar, shimmer skeletons,
  content reveal), and the high-fidelity design handoff vendored under
  `docs/design-handoff/` for reference (#623).

## v0.1.69 — 2026-06-05

Lighter image and asset serving.

**Performance**
- Card images: a warm thumbnail hit and an ETag revalidation no longer
  re-read the full source blob from the database/disk — the content hash
  is remembered per file (#592).
- The bundled CSS/JS are brotli/gzip-compressed once at startup instead of
  being re-encoded on every request; clients get the precompressed variant
  they accept (#593).

## v0.1.68 — 2026-06-05

Manage groups from the admin.

**Admin**
- The Groups page (`/admin/groups`) is now editable: rename or delete a
  group (the change propagates across user memberships and app
  access-groups), add or remove members, and create a group by adding its
  first member (#540).

## v0.1.67 — 2026-06-05

Close the seat over-admission race.

**Fix (from the code audit)**
- Completes #582: the proxy reserves a seat **atomically** when it picks
  a replica, so two concurrent first-requests can't both grab the last
  free seat of a `seats-per-container` replica. Combined with the
  scale-out in v0.1.66, a burst of new sessions now spawns up to
  `max-replicas` (one per seat) instead of over-packing a single one.

## v0.1.66 — 2026-06-04

Honor seats-per-container under load.

**Fix (from the code audit)**
- When every replica of a seat-based app is full, the proxy now spawns
  another replica (up to `max-replicas`) instead of oversubscribing a
  full one — so `seats-per-container` is honored under concurrent load.
  Only at the replica cap does it fall back to overloading the
  least-loaded replica (#582, structural part).

## v0.1.65 — 2026-06-04

Audit fixes, batch 6 (hot-path cache).

**Performance (from the code audit)**
- The proxy caches the resolved spec for each request for a short window
  (1s) instead of querying the database on every request — including
  every subresource of a page load (#587).

## v0.1.64 — 2026-06-04

Audit fixes, batch 5 (Docker backend).

**Fixes (from the code audit)**
- The Docker backend distinguishes a real image-not-found (404) from a
  daemon error, rebuilds replica uptime from the container's real
  creation time after a restart, and maps container state by matching
  the API enum directly (#586).
- The disk panel's "unused images" detection cross-references the real
  running containers instead of an unreliable per-image count, so an
  image in use is no longer flagged as reclaimable (#585).

## v0.1.63 — 2026-06-04

Audit fixes, batch 4.

**Reliability (from the code audit)**
- The auto-scaler now refuses to spawn past `max-replicas`, re-checked
  under the spawn lock — a defensive cap against races and split-brain
  HA leaders (#581).

## v0.1.62 — 2026-06-04

Audit fixes, batch 3.

**Performance & docs (from the code audit)**
- WebSocket binary/ping/pong frames forward zero-copy instead of copying
  each frame (#595).
- The HA deploy guide now documents that the scaler leader lock needs a
  direct (non-transaction-pooled) Postgres connection (#596).

## v0.1.61 — 2026-06-04

Audit fixes, batch 2 (DB performance).

**Performance (from the code audit)**
- The Apps-list trend query is now index-backed instead of full-scanning
  `spec_access` on every render (#589).
- `featured` is a real spec column, so the Apps list no longer
  deserializes every spec's `config_json` just to know which cards are
  featured (#588).

## v0.1.60 — 2026-06-04

Audit fixes, batch 1.

**Fixes (from the code audit)**
- **Config:** a `${VAR}` reference that appears only in a trailing inline
  comment (`port: 3838  # uses ${VAR}`) no longer hard-fails parsing
  (#584).
- **WebSocket:** close frames now forward the real close code and reason
  to the peer instead of an empty close (#583).

## v0.1.59 — 2026-06-04

FAQ cleanup + a Media spacing fix.

**Docs & admin**
- The FAQ questions were reworded to describe Ruscker directly rather
  than compare it to other systems (finishing the docs pass).
- The Media library's search/filter toolbar no longer touches the
  drag-drop upload zone above it.

## v0.1.58 — 2026-06-04

Docs: lead with Ruscker.

**Documentation**
- The introduction, README and the former "Ruscker vs. alternatives"
  page (now **"Where Ruscker fits"**) were rewritten to describe what
  Ruscker is and does rather than compare it against other systems — the
  feature-comparison table and competitor framing are gone, while the
  useful Ruscker-specific guidance (sub-path strip model, secrets via
  env-var) stays.

## v0.1.57 — 2026-06-04

Consistent Media filter styling.

**Admin**
- The Media library's search box and type filter now use the same look
  as the other admin controls (they were previously unstyled / a
  mismatched pill).

## v0.1.56 — 2026-06-04

Filter the Media library by type.

**Admin**
- The Media gallery gains a **type filter** next to the filename search.
  Its options come from the formats actually present (typically SVG and
  WebP — raster uploads are re-encoded to WebP), and it combines with the
  text search.

## v0.1.55 — 2026-06-04

Media filename management.

**Admin**
- Uploading an image whose name already exists no longer **silently
  overwrites** it — the upload is kept under a free name
  (`logo.webp` → `logo-2.webp`) and the toast says it was renamed.
- New **rename** action on each Media tile. The new name keeps the
  original extension, a taken name is refused, and every card logo/cover
  and landing logo that referenced the old name is **rewritten** to the
  new one — so nothing breaks.

## v0.1.54 — 2026-06-03

Clearer user-account form.

**Admin**
- Creating a user no longer fails silently. The username and password
  inputs now enforce their rules in the browser — **at least 8
  characters** for the password (create and reset), and **letters,
  digits and `_ . @ -` only** for the username — instead of letting a
  bad value through to a vague "invalid input" message. The field hints
  and the error message spell out the rules.

## v0.1.53 — 2026-06-03

Searchable, sortable admin tables.

**Admin**
- Every data table in the admin (Apps, Users, Credentials, both
  disk-panel tables) now has a **search box** that filters rows as you
  type and **clickable column headers** to sort ascending/descending
  (numeric-aware, so version/access/size sort as numbers). The Actions
  column stays inert.
- The spec form no longer shows the **Advanced** section for external
  **Link**/**Package** cards — there's no image or container to tune, so
  only the external-link card remains for those kinds.

## v0.1.52 — 2026-06-03

Friendlier YAML import.

**Admin**
- The import dialog now has a **drag-and-drop zone** in place of the bare
  file input. Drop an `application.yml` onto it or click to browse; the
  prompt is localized (the old native **"Choose file"** button always
  showed in the browser's language, never the panel's).
- After a selective import, the result message now reports **how many
  credentials and images** were pulled in alongside the apps — e.g.
  *"… 3 credential(s) and 12 image(s) imported"* — so the logo→Media and
  password→credential-store moves are visible, not silent.

## v0.1.51 — 2026-06-03

Media import + safe deletion.

**Admin**
- Importing a ShinyProxy `application.yml` now copies each selected app's
  **local logo into the Media library**, so it shows up in `/admin/media`
  and not just on the card. Logos that are URLs, data URIs, empty or
  traversal-looking are skipped.
- Inline Docker registry passwords in an imported config are moved into
  the **named credentials store** (encrypted at rest), de-duplicated, and
  the spec is rewired to reference the credential — the password never
  lands in the spec config.
- **Deleting a Media image that is in use no longer breaks the card.** The
  apps using it fall back to the default Ruscker logo (a cover image is
  cleared), and the confirm dialog spells this out before you delete.

## v0.1.50 — 2026-06-03

Selective YAML import.

**Admin**
- Importing a ShinyProxy `application.yml` now shows a **preview list of
  the apps** it contains — each marked **New** or **Updates**, with a
  checkbox — so you confirm **which to import** instead of taking the
  whole file. Only the checked apps are imported; the landing and
  settings are left untouched.

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
