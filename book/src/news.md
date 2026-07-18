# Release notes

What changed in each release. Ruscker follows semantic versioning; while
on `0.x` the API and YAML schema stay backward-compatible (new fields are
optional), and breaking changes are called out here.

Downloads (binaries, `.deb`, container image — all cosign-signed) are on
the [GitHub releases page](https://github.com/StrategicProjects/ruscker/releases).

---

## v0.2.45 — 2026-07-18

- **Two-factor authentication for selected apps (step-up MFA).** A spec
  can now require a second factor: set **`require-mfa: true`** (with an
  optional **`mfa-validity-days`**) on the app, or flip "Exigir 2FA" in
  its form. Users enrol a TOTP factor once — Google/Microsoft
  Authenticator, Authy, 1Password and the like — by scanning a QR code
  on their own **Account → 2FA** page, and get one-time recovery codes
  shown once. The factor belongs to the user, so one proof satisfies
  every protected app; each app decides how recent that proof must be
  (`mfa-validity-days`, default 7; `0` = only within the current login
  session). Enrolment establishes the first proof, so a user who just
  set up 2FA flows straight into the app.
  - **Enforced at the proxy, before anything starts:** an unenrolled or
    unproven visit to a protected `/app` is redirected to enrolment or
    the challenge **without waking or spawning a container**; a
    protected `/api` fails closed with `401`/`403`. Break-glass
    (`RUSCKER_ADMIN_TOKEN`) sessions bypass the factor so an operator
    can't be locked out — every bypass is logged and audited.
  - **Device trust with real revocation:** a successful challenge
    remembers the browser (an opaque, hash-only, `HttpOnly` cookie)
    within the app's window. Changing/resetting a password, resetting
    the factor, deleting the user, or "Forget devices" on the account
    page all revoke the remembered device immediately; a normal logout
    keeps it. TOTP replay is blocked, and the trusted-device cookies
    never reach an app container.
  - Secrets are encrypted at rest with `RUSCKER_MASTER_KEY` and never
    appear in logs, the audit trail, or config export; recovery codes
    are stored only as salted hashes. Admins see only "2FA configured"
    and an audited reset — never the QR code or secret. Works on SQLite
    and Postgres/HA. Migrations **0029** + **0030**.
- **Hardened cookie boundary for hosted apps.** Because apps share the
  portal's origin, a compromised app's HTTP **responses** could
  previously overwrite or clear the user's Ruscker cookies (session,
  sticky, the new MFA device cookie). The proxy now strips reserved
  `Set-Cookie` headers from app responses and neutralises
  cookie-clearing `Clear-Site-Data` directives, while leaving the app's
  own cookies and non-cookie directives untouched.

## v0.2.44 — 2026-07-17

- **Identity headers for apps (ShinyProxy compat).** Apps that need to
  know *who* is using them can now receive the authenticated identity
  per request. Setting **`add-default-http-headers: true`** on a spec
  (or its toggle in the app form's Access section) forwards
  `X-SP-UserId` and `X-SP-UserGroups` — the ShinyProxy contract, so
  migrated apps that attributed writes to a user work again. Unlike
  ShinyProxy, Ruscker defaults this **off**: upgrading never silently
  discloses identity to an app that wasn't already trusted with it.
- **Extra identity claims, opt-in per app.** A Ruscker-native
  **`identity-claims: [email, setor]`** list (checkboxes in the same
  form section) additionally forwards `X-Ruscker-User-Email` /
  `X-Ruscker-User-Setor`. Data minimization throughout: each app gets
  only the claims it declared, a claim with no stored value is omitted
  (never sent empty), and the claims work with or without the X-SP
  pair. Anonymous visitors and token sessions carry no identity.
- **Spoofing-proof by construction.** The whole `X-SP-*` and
  `X-Ruscker-User-*` namespaces are stripped from every incoming
  request — HTTP and WebSocket alike — before Ruscker injects its own
  authoritative values, so an app can trust what it receives (see the
  trust-boundary note in the security guide: the container port must
  only be reachable through Ruscker). Accented values (`Gestão`)
  arrive as clean UTF-8. No extra database work per asset: identity is
  resolved once per request through a short-lived cache that admin
  edits invalidate immediately.
- **Users page scales.** `/admin/users` now paginates server-side (50
  per page) with a server-side search over username, groups and the
  profile fields — a large user base (big CSV imports) no longer
  renders thousands of rows per view. Search is accent-tolerant on
  both SQLite and Postgres.
- **Clearer module titles.** Every admin screen's title/subtitle was
  standardized to state what the module is for ("Gestão de X" +
  purpose line, per the design doc): the nav now says **Containers**
  (was Painel) and **Atividades** (was Auditoria), the Logs tab is
  titled "Auditoria de Logs", and technical notes (media formats,
  CSV import details) moved from subtitles into in-context helper
  text. The Groups subtitle no longer claims the page is read-only —
  it hasn't been since groups became editable.

## v0.2.43 — 2026-07-13

- **Scheduled jobs (cron).** A new **Schedules** page in the admin runs
  a spec's image to completion on a cron — the ETL/report case: same
  image, environment, volumes and credentials as the app, with an
  optional command override per schedule. Semantics built for ETL:
  a new schedule waits for its first occurrence (no fire-on-create),
  downtime over several occurrences collapses to one firing, and in HA
  only the leader fires (with an atomic claim so nothing double-runs).
  Each run lands in a history with status, exit code, duration and the
  log tail; a failing job raises the **`job-failed`** alert through the
  notification webhook. Per-schedule timeout (default 1 h). Job
  containers never linger — removed on every exit path, and invisible
  to the replica machinery.
- **Named-volume management.** The Disk panel gained a **Volumes**
  card: list named Docker volumes with live reference counts (any
  container on the host counts), create them (labelled as
  Ruscker-created), and remove — offered only for volumes Ruscker
  created, with zero references and no catalog spec mentioning them;
  third-party data is never removable from the panel, and the daemon's
  no-force refusal backstops everything.
- **Refresh button on the Apps list** — reload rows, states and counts
  without hunting for F5.
- **Docs.** The admin guide caught up with the recent releases
  (consolidated user editing, password policy and generator, the
  corrected Viewer role, sections following the new tab order); a new
  **Data plane** section in Security states the proxy→container
  encryption story explicitly (loopback on a single host; private
  network / WireGuard for multi-host); and Troubleshooting explains
  the one-click fix when an app keeps failing with an old cached image
  (the **Update image** button — no Docker restart involved).

---

## v0.2.42 — 2026-07-13

- **Password policy.** New passwords (creating a user, admin reset,
  first-admin setup, changing your own, CSV import) must now have at
  least **8 characters with 1 uppercase, 1 lowercase, 1 digit and
  1 special character**. Existing passwords are untouched — the rule
  applies when a password is set or reset (and admin-assigned
  passwords already force a change at first login). Error messages and
  form hints state the full rule in all four languages.
- **Random password generator.** A new button beside the password
  field (user create and reset forms) fills in a strong 14-character
  password — generated in the browser with cryptographic randomness,
  policy-compliant by construction, without look-alike characters
  (no `0`/`O`, `1`/`l`), and revealed so you can read what you're
  about to hand over. No more `teste123`.
- **Admin tabs reordered by workflow.** Daily drivers first
  (Dashboard, Apps, Media, Credentials, Appearance), then people
  (Users, Groups), with diagnostics and maintenance at the end
  (Logs, Disk, Audit, System) — Disk no longer sits between Dashboard
  and Apps. An Editor's visible tabs now form one contiguous block, so
  Editor and Admin share the same nav geography.

---

## v0.2.41 — 2026-07-13

A large batch: ten issues closed, including the whole config-model epic.

- **Alert notification webhooks.** Set a URL in the admin **System** tab
  and Ruscker `POST`s a JSON payload when something an operator should
  know about happens: an app's container **failing to start**, a running
  replica **dying outside Ruscker's control**, or an app **saturated at
  `max-replicas`** (visitors being turned away). Delivery is best-effort
  with retries and a per-(event, app) cooldown so a stuck condition
  doesn't storm the channel; a *Send test alert* button checks the
  wiring. Payload contract documented in the guide (§ admin/System).
- **`ruscker.yml` is the canonical service config.** The service's own
  settings (bind, subpath, forwarded-header trust, timeouts, metrics)
  now live in a fully **self-documented `/etc/ruscker/ruscker.yml`** —
  every option commented with its default. `application.yml` remains as
  the **ShinyProxy import format** (same schema, and still accepted as
  `--config`); `serve` without `--config` finds `ruscker.yml` first and
  falls back silently. **Upgrades migrate automatically**: an edited
  `application.yml` from an older package is copied over `ruscker.yml`
  once, while the latter is still the pristine example — nothing to do
  by hand. Changing the port or the subpath no longer ever requires
  editing the systemd unit (the `--bind` flag left `ExecStart`; the
  README gained a complete no-systemd subpath recipe).
- **Consolidated user editing.** Each row in **Users** now has an
  *Edit* button opening a dedicated page — role, groups and profile in
  one form with a single save (atomic, with the last-admin guard rolling
  back *all* fields), and password reset alongside.
- **`container-wait-time` now works.** The field parsed but was never
  consumed — the readiness wait was hardcoded at 60 s. It now drives the
  spawn readiness budget (both single and multi-host), and failure
  messages name the configured value.
- **Per-spec access counting no longer writes per request.** API access
  counts aggregate in memory and flush in batches (one UPSERT per
  spec/day bucket every couple of seconds, with retry and a final flush
  on shutdown) — under load, thousands of calls become one write.
- **Browser-tab title honours your title.** The page `<title>`/`og:title`
  used to skip `proxy.title`; it now follows the same chain as the
  header (SEO title → Appearance title → `proxy.title` → default).
- **Docs: ShinyProxy → Ruscker field map.** A new guide page lists every
  documented ShinyProxy 3.x option with its status here (supported /
  warned-and-ignored / planned / out of scope) and the Ruscker
  equivalent — the field-by-field companion to the migration guide. The
  Configuration chapter now opens with the four-layer model (service
  YAML × secrets env × portal DB/admin × ShinyProxy import).
- **Live WebSocket e2e for Shiny + Streamlit.** A new gated test suite
  drives the real demo containers through the proxy — cold spawn, URL
  rewriting, and both directions of the WebSocket pump — closing a
  validation gap open since the proxy phase.

---

## v0.2.40 — 2026-06-25

- **Citation metadata + DOI.** Added a `CITATION.cff` at the repo root so
  GitHub shows a **"Cite this repository"** button (APA/BibTeX) and
  [Zenodo](https://zenodo.org) archives each release with a permanent
  **DOI**. No code changes — this is the first release archived to
  Zenodo. Cite Ruscker with the concept DOI shown on the repository page.

---

## v0.2.39 — 2026-06-18

- **Apps no longer pre-warm by default.** An app with no `min-replicas`
  set now defaults to **0 (cold-start)** — the container starts when the
  first visitor arrives and is reaped once idle, matching ShinyProxy and
  Shiny Server Free. Previously every containerized app defaulted to 1,
  so importing a large config (e.g. a 24-app ShinyProxy setup) lit one
  idle container per app at boot. To keep an app hot, set
  `min-replicas: 1` (or more) on it explicitly. The autoscaling ceiling
  is unchanged, so cold-start apps still scale on demand. No re-import is
  needed — only the effective default changed; existing specs keep their
  saved values. (The spec form's "Initial replicas" hint and summary now
  read 0 to match.)

---

## v0.2.38 — 2026-06-18

- **Favoriting a card updates the Featured rail instantly.** Clicking the
  star on a portal card now adds it to (or removes it from) the
  "Destaques" rail live — no page reload. The rail appears the moment you
  favorite your first card and hides again when you clear the last one.

---

## v0.2.37 — 2026-06-18

- **The live dashboard no longer holds a connection open.** It used a
  persistent SSE stream; behind a reverse proxy that serves Ruscker and a
  side-by-side app (e.g. ShinyProxy) on **one hostname** over HTTP/1.1,
  that long-lived connection could starve the browser's ~6-per-origin
  pool and stall every request to that host — freezing both apps until
  the stream dropped. The dashboard now **polls** a JSON snapshot every
  few seconds (each request returns and frees the connection), so opening
  the monitoring panel can't wedge the origin. The live logs view and the
  image-pull progress keep their short-lived streams. (HTTP/2 at the TLS
  edge remains the broader fix and also helps any other app on the host.)

---

## v0.2.36 — 2026-06-18

Portal/admin UX + a big icon-loading win.

- **Icons appear instantly.** The bundled Tabler icon font shipped the
  full set — ~447 KB woff2 + ~209 KB CSS — even though the app uses ~95
  glyphs, so on a cold visit the icons "popped in" only after that
  download. Subsetted to just the used glyphs (**10.7 KB woff2 + 4 KB
  CSS**), and fonts now carry an ETag + a day-long revalidated cache, so
  repeat visits don't re-download them.
- **Favorite star tidied.** Logged-out visitors no longer see an empty
  white circle on cards; the "Destaques" carousel drops the star
  entirely; the un-favorited star is a softer grey.
- **CSV user import is clearer.** Instead of two look-alike buttons, you
  see "Choose CSV file" first; once a file is picked it becomes a single
  primary "Review import: «file»" button (plus a discreet "change file"),
  so the choose → review flow is obvious.
- **Fixes (#910):** a transient DB error no longer lets a must-change user
  skip the password prompt, and duplicating an app won't suggest an id
  that shadows a YAML-defined one.

---

## v0.2.35 — 2026-06-17

Two small correctness fixes (#910).

- **A DB blip no longer lets a must-change user skip the prompt.** The
  forced-password-change guard's per-session cache (v0.2.34) treated a
  transient DB error as "no change needed" and pinned it for the cache
  window. A DB error is now never cached — the next request re-checks, so
  the user is caught as soon as the database recovers.
- **Duplicating an app won't shadow a YAML-defined one.** The suggested
  copy id now checks the full effective catalog (DB + YAML), so it can't
  land on an id already used by a config-only spec.

---

## v0.2.34 — 2026-06-17

Admin correctness + performance, from a navigation-focused audit.

- **DB-only apps survive a restart (#907).** A spec created in the admin
  (not in the YAML) had its running container reconciled after a restart
  with a seat cap of 0, so it read as permanently full — wrong routing
  and scaling until it was re-touched. Reconcile now resolves seat caps
  from the effective catalog (DB + YAML), not the YAML alone. Same root
  cause fixed for **Duplicate**: duplicating a YAML/config-only app no
  longer 404s.
- **Admin tab navigation is lighter.** The effective spec catalog is now
  cached behind a cheap signature (#902) instead of being rebuilt — and
  re-deserialized — on every Apps/Disk/Media/Groups/System page load; the
  Disk panel fetches its Docker/DB inputs concurrently instead of one
  after another (#904); and the forced-password-change guard no longer
  does a DB lookup on every admin request (#903). The cache is never
  stale (any spec write moves the signature) and HA-safe.

> If admin navigation felt slow on an older build, the bigger win is
> v0.2.31's dashboard-SSE fix (#852) — it stopped a live monitoring tab
> from holding an HTTP/1.1 connection and stalling other tabs.

---

## v0.2.33 — 2026-06-17

Admin & portal UX polish, all operator-requested.

- **Apps action column — grouped buttons.** The action icons are now
  clustered into logical groups (marker · manage · operate) with thin
  dividers, and the destructive **Delete** is set apart so it's never
  clicked by mistake.
- **"Update image" shows live progress.** The per-row re-pull now shows
  the current Docker pull step inline (Pulling → Downloading → Extracting
  → done) instead of just a spinner, so you can follow it.
- **Users CSV import — localized file picker.** The native file input
  rendered the browser's own "Choose file" text in the browser language;
  it's now a styled, localized control matching the YAML import, with the
  chosen filename shown and the submit gated until a file is picked.
- **Dashboard replica logos fall back to the monogram.** When a logo file
  isn't present on the host, the Active Replicas grid now shows the app's
  monogram initial instead of a blank tile.
- **Portal public/private filter removed.** With the decorative access
  lock gone (v0.2.31), the filter no longer distinguished anything a
  visitor could see (restricted apps are hidden from non-members), so the
  chips were dropped. Real access enforcement and visibility are
  unchanged.

---

## v0.2.32 — 2026-06-17

Disk host-safety hardening — the follow-ups that make "remove unused
images" safe to click on a host that runs other containers (a
side-by-side ShinyProxy) and can't re-pull from the registry.

- **Image removal is now provenance-aware (#894).** The Disk panel only
  ever removes images **Ruscker has managed** — a ref recorded when a
  spec referencing it is saved, or when it's explicitly pulled, in a
  durable table that survives spec deletion. A neighbour's idle image
  (e.g. ShinyProxy's) shows a "not managed" badge and is never deletable;
  the per-row remove also enforces this server-side. Dangling images stay
  the job of the host-safe "Reclaim space" button. Backfilled from the
  current catalog on upgrade.
- **Multi-host fails closed (#897).** When a clustered host is
  unreachable, the in-use signal now fails the call instead of returning
  a partial inventory — so the panel enters its usage-unknown mode rather
  than mislabelling an image backing a container on the down host as
  "unused".
- **Multi-host disk fan-out is parallel with a per-host timeout (#895).**
  Several hosts or one slow daemon no longer sum their latencies onto the
  admin page; a timed-out host is skipped where tolerant and fails closed
  for the in-use signal.
- **`ruscker import --images-dir` matches its help (#891 follow-up):** an
  empty value or a missing directory skips media import instead of
  auto-discovering or aborting; now covered by tests.
- **Docs:** `SECURITY.md` gained actionable guidance on hosting untrusted
  apps on a shared origin (#878) — what isolates them and what doesn't.

---

## v0.2.31 — 2026-06-17

A migration-and-operations release: the bits that mattered when moving a
real ShinyProxy install onto Ruscker side by side, plus the host-safety
hardening that a shared host (ShinyProxy next door) demands.

**ShinyProxy migration fidelity.**

- **Import reads the `container-volumes` key (#886).** A real ShinyProxy
  config authors bind mounts under `container-volumes` (like
  `container-env` / `container-cmd` / `container-network`); the schema had
  named it `volumes` with no rename, so importing silently dropped every
  mount. The ShinyProxy key is now read; the bare `volumes` stays as an
  alias for older Ruscker configs.
- **Import card images into the Media library (#887).** A spec's logo is a
  reference like `/assets/img/snap_aurora.png`; the YAML never carried the
  bytes, so a migrated catalog had no logos in the library.
  `ruscker import --images-dir <dir>` (auto-discovered beside the config
  like `serve`) now ingests every image, keeping each file's **original
  name** so the references resolve. Idempotent; `--images-dir ""` skips,
  a missing dir warns and skips (#891).
- **Per-spec `container-network` (#850) and custom `labels` (#851)** —
  create-and-attach a Docker network, and stamp extra labels on the
  container.

**Host-safety on a shared host.** The Disk panel can now run next to a
ShinyProxy (or anything else) without ever touching its containers or
images:

- **Backend enforces Ruscker ownership (#871).** `remove_container`
  re-inspects and refuses anything without the `ruscker.replica_id`
  label; the in-use cross-reference is computed against **every** host
  container, so a non-Ruscker image is never flagged "unused".
- **Fail closed when Docker can't be queried (#889).** If the container
  listing fails, every image now reads as in-use (no remove, no prune,
  warning banner) instead of assuming the host runs nothing — which could
  otherwise delete an in-use image that can't be re-pulled.
- **"Reclaim space" button (#869)** — prunes only dangling images + the
  build cache, never a tagged image or any container.

**Admin features.**

- **Read-only System diagnostics tab (#766)** — version, Docker, DB, paths
  at a glance (restart shown as a command, not a button).
- **Bulk user import from CSV with a preview (#862)**, all-or-nothing per
  row (#875); **optional profile fields** — sector / e-mail / phone (#856).
- **Viewer is a portal user, not a panel operator (#857)** — Viewers land
  on the portal and see the cards their groups allow; **per-user favorite
  star** on cards (#858).
- **`max-containers` is an always-visible field (#854)** and rejects `0`
  at save (#877); **"Update image" re-pull** in the Apps list (#855), with
  the pull stream now bounded and concurrent pulls capped (#874).
- **Validation warnings for labels + network (#892)** — an invalid label
  key, a reserved `ruscker.*` key, or a malformed `container-network` are
  flagged by `validate` and the spec form.

**Fixes.** Dashboard shows the app logo in the replica grid (#870); the
last-admin guard is atomic at the DB (#872); an image rename rolls back if
the image vanished (#873); link cards are labelled "Links" (#876); the
admin lands on Apps and pauses dashboard SSE when the tab is hidden (#852).

---

## v0.2.30 — 2026-06-15

- **Dashboard mobile: the app name no longer overlaps the replicas column
  (#847).** In the Active Replicas grid on a phone a long app name spilled
  over the next column; it now ellipsizes cleanly and the columns keep
  their room and alignment as the grid scrolls inside its card.

---

## v0.2.29 — 2026-06-15

- **Mobile fixes: icon-only landing header and aligned table columns
  (#845).** The public landing header now shows the sign-in control as an
  icon (no text label) on phones, matching the rest of the chrome. And the
  admin tables now scroll inside a wrapper while staying real tables, so
  their columns always line up instead of the first column overlapping the
  second on narrow screens.

---

## v0.2.28 — 2026-06-15

- **Mobile portal cards: full width and full description (#843).** On
  phones the app cards were a narrow fixed-width column with empty space
  either side, and the description stayed clamped to two lines because the
  reveal is hover-only. Now the cards fill the screen width and the whole
  description shows up-front on touch devices. Desktop is unchanged.

---

## v0.2.27 — 2026-06-15

- **Mobile polish: icon-only nav and no sideways scroll (#841).** On
  phones the admin top nav now shows just the icons (labels stay
  available to screen readers) on their own row that scrolls
  horizontally, and the wide data tables + dashboard replica grid scroll
  inside their own boxes — so no admin page (or the public portal) pushes
  the page wider than the screen. Desktop is unchanged.

---

## v0.2.26 — 2026-06-15

- **A decorative "requires login" lock, independent of access lists
  (#839).** The restricted-access toggle only stuck when a group or user
  was listed, so turning it on while leaving the app public silently
  reverted on save. There's now a separate **Access lock** toggle that
  closes the card's padlock to signal that the app authenticates on its
  own — without restricting anything in Ruscker (the app stays visible to
  everyone and reachable; its own sign-in does the gating). The real
  group/user restriction stays as its own control. Localized in
  pt/en/es/fr.

---

## v0.2.25 — 2026-06-14

- **The post-create confirmation is centred on screen (#835).** v0.2.24's
  success dialog rendered low and partly hidden — the admin shell's
  animated `<main>` was acting as its positioning container. It now
  teleports to the page body, so it sits centred in the viewport.

---

## v0.2.24 — 2026-06-14

- **Creating an app now confirms the save and asks where to go (#835).**
  Saving a brand-new app used to drop you silently onto its edit form,
  with no sign the save took. It now shows a success dialog that
  confirms the app was created and offers a clear choice: stay on the
  form to keep editing, or jump to the apps list. Localized in
  pt/en/es/fr.

---

## v0.2.23 — 2026-06-13

- **The card description also collapses smoothly (#834).** The hover
  reveal eased open but snapped shut; now it eases closed too, so the
  open and close are symmetric.

---

## v0.2.22 — 2026-06-13

- **The card hover reveal is smooth again (#833).** v0.2.21 fixed the
  clipping but, while measuring the text height, made the expansion
  snap open. It now eases open over its full, comfortable curve — full
  text, any length, smoothly revealed.

---

## v0.2.21 — 2026-06-13

- **The hover reveal shows the whole description (#832).** v0.2.20's
  smoother animation capped the height, so longer descriptions were
  cut off. The card now expands to the exact length of its text — any
  size, no clipping — while keeping the smooth ease.

---

## v0.2.20 — 2026-06-13

- **Smoother card-description reveal on hover (#831).** The expansion
  used to snap open; it now eases in and out over a comfortable
  duration, with a small delay so a quick pass of the mouse doesn't
  trigger it.

---

## v0.2.19 — 2026-06-13

Fixes for two v0.2.18 features that didn't quite land in real use.

- **Featured carousel: hover expands only the card under the cursor
  (#828).** A description expanding on hover used to stretch every card
  in the rail to match; now just the hovered one grows.
- **Stop/restart progress feedback now actually shows (#828).** The
  busy state (dimmed row, disabled buttons, spinner) was applied but
  the navigating form POST made the browser skip painting it. The
  action now runs without a page navigation, so the feedback appears
  and stays for the whole operation.

---

## v0.2.18 — 2026-06-13

**Landing**

- **Card descriptions expand on hover (#825).** A description clamped to
  two lines now reveals its full text when you hover the card — it
  grows downward without disturbing its neighbours.

**Dashboard**

- **Stop/restart now show progress (#827).** These actions take a few
  seconds (drain, signal, and a respawn for restart) and the page used
  to look frozen with no feedback. The replica row now dims, its action
  buttons disable (no accidental double-fire) and the clicked one shows
  a spinner until the action completes.

---

## v0.2.17 — 2026-06-12

**Images**

- **Force a re-pull of an already-present image (#824).** The app
  form's Pull button only appeared when the image was missing, so a
  locally-cached image couldn't be refreshed from the UI. It now also
  shows when the image is present, as **"Update image"** — re-pulling
  from the registry. Use it after re-publishing the same tag with new
  bytes (or a corrected CPU architecture); the next launch runs the
  freshly-pulled image.

---

## v0.2.16 — 2026-06-12

**Diagnostics**

- **A container that crashes on boot now says why (#823).** When an app
  container dies during startup — a bad config, an unreachable
  database, a missing mounted file, or an image built for the wrong
  CPU architecture — Docker drops its port bindings, and the spawn used
  to fail with the cryptic "no port binding for 8000/tcp". The error
  now names the exit code and appends the container's own log tail (the
  app's stack trace), so the cause is right there in the dashboard logs;
  a container that's still running but isn't listening on the expected
  port is reported as exactly that.

---

## v0.2.15 — 2026-06-12

**Private images**

- **A picked registry credential now applies to the on-demand Pull
  immediately (#822).** The credential selector wasn't bound to the
  form's live state, so the editor's "Pull" button used an empty
  credential until the app was saved and reloaded — a private image
  pulled anonymously and failed with "404: pull access denied", then
  worked after a save. The selector is now bound, so the chosen
  credential is used right away. The Pull error line also names how it
  authenticated (anonymous vs. the user/registry), matching the
  launch-time pull.

---

## v0.2.14 — 2026-06-12

**Private images**

- **Docker Hub credentials now apply reliably (#820).** The stored
  credential's registry field was handed to the Docker daemon verbatim
  (`docker.io`, or empty) — but daemons match credentials against the
  canonical `https://index.docker.io/v1/` address, and on a mismatch
  silently pulled anonymously, so a private image failed with
  "404: pull access denied" despite a valid credential. Hub aliases are
  now normalized to the canonical address on every pull; other
  registries pass through unchanged.
- **Pull errors say how they authenticated.** A failed spawn pull now
  reads "pull image (authenticated as user @ registry)" — or
  "(anonymous)" — so a missing, ignored or wrong credential is
  diagnosable straight from the error message.
- **The registry credential picker moved next to the Docker image
  field** in the app form (it used to hide under Advanced).

---

## v0.2.13 — 2026-06-12

**Media & pickers**

- **Same-name uploads keep both images, visibly (#815).** Uploading a
  file whose name already exists stores it under a free name
  (`logo.webp` → `logo-2.webp`) — the existing image and everything
  referencing it stay untouched. That was already the behaviour; what
  was missing was telling them apart afterwards: the image-picker
  tiles (app form and Appearance) now show a **filename caption**, so
  the renamed upload is unmistakable next to its look-alike. Inline
  uploads keep auto-selecting the renamed file for you.

---

## v0.2.12 — 2026-06-12

**Public landing**

- **The intro paragraph understands inline Markdown (#812):**
  `**bold**`, `*italic*` and `[links](https://…)` — in the default
  text and in every per-locale variant. It is intentionally tiny and
  safe: input is HTML-escaped before the rewrites, so raw HTML always
  renders as visible text; unmatched markers stay literal, which keeps
  every existing plain-text intro rendering exactly as before; links
  are restricted to http(s). The Appearance preview renders the same
  subset live as you type, and the page meta description uses the
  marker-free reading.

---

## v0.2.11 — 2026-06-11

**Groups page**

- **Public apps render as logo chips (#809).** Each open app shows as a
  rounded chip — its logo on a square tinted with the catalog's
  per-type colour (the same palette the portal cards use), the app
  name and a green globe. Clicking still opens the app's editor.

---

## v0.2.10 — 2026-06-11

Live-testing round on the portal content tools.

**Public landing**

- **The intro paragraph now fills the row (#805)** — justified, with
  per-language hyphenation — instead of capping at a narrow column and
  ragging right. It also gained a stable `.landing-intro` class as the
  hook for operator CSS.

**Appearance editor**

- **HTML blocks reach the handoff layout, with inline editing (#806).**
  The blocks card matches the other cards: slot headings with count
  badges, rows with drag handle / active pill / arrows / pencil /
  trash, dashed empty states — and the pencil (or "+ New block") opens
  the editor right there: name, a Top|Bottom position segment, the dark
  HTML editor, CSP origins, an active switch, delete and Done. No more
  navigating away.
- **Block actions return you to the blocks section (#808)** instead of
  the top of the page, and the card sits in the form column at the same
  width as its neighbours.
- **Custom CSS card** follows its mockup: the caution note above the
  dark editor.

---

## v0.2.9 — 2026-06-11

The Appearance editor reaches the design handoff, end to end — built
card-by-card with the operator reviewing each screen live.

**Appearance editor, handoff layouts**

- **Catalog layout (#794):** Grid/List/Sections as large icon tiles
  (active outlined in the brand teal) with the density control as a
  full-width high-contrast bar.
- **Catalog cards (#796, #800):** one high-contrast mode bar — Auto
  (type colour) | Solid | Gradient. Auto explains itself and shows the
  real per-type tints; Solid/Gradient open per-theme panels with a live
  cover preview, a draggable angle dial and handoff-style colour stops.
  The dark panel carries an explicit **Inherited ⇄ Own** pill — inherit
  shows just the preview and a note; own opens the controls (your work
  is kept when toggling).
- **Theme & colors (#798):** full-width default-theme bar, large brand
  swatches (check on the active one, a "+" tile for a custom accent
  applied to both themes) and per-theme palette panels with a live
  mini-preview of background, text and accent.
- **The Featured-carousel toggle moved into Visible sections** — it is
  a portal section like search and filters.

**Fixed**

- **Initial colour states now tell the truth (#792):** empty pickers no
  longer contradict their placeholders; dark-side pickers show the
  light value they inherit; the brand swatch matching the effective
  accent starts selected. Also fixes colour pickers that could render
  black when a value was cleared.

---

## v0.2.8 — 2026-06-11

Second round of v0.2.7 field-testing feedback.

**Apps list**

- **Archiving no longer reloads the page (#787).** The archive toggle
  now updates the row in place (state pill + icon) — no navigation, so
  the page keeps its scroll position.

**Appearance editor**

- **Per-theme default card cover (#790).** The catalog's default cover
  (solid or gradient) can now differ between the light and dark themes,
  dark inheriting light when unset. The portal switches covers
  instantly with the visitor's theme toggle.
- **Theme switch on the Portal preview (#790).** A ☀️/🌙 toggle on the
  preview pane lets you inspect both themes without changing the saved
  default theme — every preview control follows it.
- **The "Card covers: Tinted | Gradient" control was removed (#789).**
  It only toggled a subtle overlay on tinted covers and was confusing
  next to the default-cover builder. Saved values keep rendering as
  before; "Restore defaults" clears them.

---

## v0.2.7 — 2026-06-11

Polish from v0.2.6 field testing — four operator reports, all fixed.

**Apps list**

- **Archiving no longer reorders the table (#780).** The archive toggle
  used to stamp the app's "updated" time, so the row jumped to the top
  of the list (which sorts by recency). Archiving is a visibility flip,
  not an edit — the row now stays exactly where it was. Version history
  and auditing are kept (with distinct archive/unarchive audit actions).

**Appearance editor**

- **Header background is now one explicit choice (#782): Preset, Solid
  or Gradient.** Previously a custom background silently overrode the
  flat/soft/bold presets, making them look broken. Picking Preset
  clears the custom background (and remembers it, in case you switch
  back); the preview reacts immediately.
- **Per-theme header colours (#784).** The light and dark themes can
  now each have their own header background (solid or gradient) and
  text colour — before, one value served both. Leaving the dark side
  blank inherits the light values, so existing setups render unchanged.
  The portal switches instantly with the visitor's theme toggle.
- **"Restore defaults" button (#783).** One click (with confirmation)
  returns the portal to its original look — colours, theme, header
  style, covers and layout. Titles, logos, intro texts, SEO, custom CSS
  and HTML blocks are preserved.

---

## v0.2.6 — 2026-06-11

Admin usability release: faster app management from the Apps list and a
clearer Appearance editor.

**Apps list (#775)**

- **Archive / unarchive an app in one click.** The Actions column gained
  an archive toggle: an archived (inactive) app keeps its configuration,
  history and audit trail, but its card leaves the public portal until
  it's reactivated. No more opening the editor just to hide an app.
- **Delete from the list**, with a confirmation dialog. Same behaviour as
  the editor's delete: the app's containers are stopped and the action is
  audited. Apps defined in the YAML config remain read-only (the file is
  their source) — deleting a database app whose id also exists in the
  YAML brings it back as a read-only config row, by design.

**Appearance editor (#776)**

- **One "Logos" section.** The main header logo (mark / symbol-only /
  custom) now carries its own inline image picker — previously the
  "Custom" mode had no picker and the image had to be configured in a
  separate "Header/footer logos" section that silently fed it. The modes
  are now truthful: switching away from Custom really brings the built-in
  mark back, and switching back restores the picked image. Additional
  logos (header center/right, footer) live right below.
- **Header style in one place**: preset (flat/soft/bold), background
  colour or gradient, and text colour together — they were split across
  two sections.
- **Catalog cards in one place**: cover style + the default cover builder.
- **Theme & colors in one place**: default theme, brand-colour quick
  picks and the light/dark palettes together.

**Fixed**

- **Duplicated logo in the Portal preview (#777).** With a custom header
  logo set, moving the logo-size slider made the built-in mark reappear
  next to it ("two logos in the top-left"). The preview now removes the
  mark outright when a logo is shown, and mirrors the real portal's
  rendering exactly.

---

## v0.2.5 — 2026-06-10

**Audit release.** A full bug / security / UX audit of the codebase
(issues #730–#746 + #758) — every finding fixed, 18 PRs. Highlights, by
area:

**Proxy correctness**

- **One sticky-session cookie per app.** A single global cookie meant two
  interactive apps open in the same browser fought over one session:
  opening app B silently dropped app A's session (orphaning its seat —
  users landed on the "full" splash for a seat they held) and broke
  stickiness for multi-replica apps. The cookie is now per-spec and
  scoped to the app's own URL path; lingering old cookies are expired
  automatically.
- **Compressed upstream HTML no longer bypasses the URL rewriter.** Apps
  that gzip their HTML (Dash behind flask-compress, nginx-fronted apps)
  served pages with no `<base href>` and broken assets; the proxy now
  asks such upstreams for uncompressed HTML, ShinyProxy-style.
- **WebSocket upgrades** keep the request's query string (Jupyter kernel
  channels use `?session_id=`), echo the negotiated subprotocol on the
  101 (required by RFC 6455 — newer jupyter-server broke without it),
  and a dead replica now yields a real 502 instead of a silent drop.

**Reliability**

- A spawn that fails after the container was created (slow boot, crash)
  now removes that container — it used to linger, spawn duplicates and
  even be adopted as `Ready` after a restart.
- The scaler's crash cleanup also releases the dead replica's tracked
  sessions (graceful shutdown no longer waits out the full grace window).
- **Multi-host:** the disk panel, image indicator / Pull button and the
  scaler's crash cleanup now work across all hosts (they silently did
  nothing on multi-host deployments before).
- **HA / shared Postgres catalog:** migrations 0017–0021 were never
  ported to Postgres, so the landing and the whole Appearance editor
  were broken on `--config-db-url` deployments since v0.1.90. Ported,
  plus a guard test so the two migration sets can't drift again.

**Security**

- **Forwarded-header trust unified** behind `server.useForwardHeaders`:
  `X-Forwarded-Proto` is no longer trusted from arbitrary clients (it
  could flip a cookie's `Secure` flag), and the real client IP is now
  appended to `X-Forwarded-For` for upstream apps. **Deployments that
  terminate TLS in a reverse proxy must set `useForwardHeaders: true`**
  for cookies to carry `Secure` — ShinyProxy-migrated configs already do.
- Changing your own password now signs out every other session for the
  account (the admin-initiated reset already did).
- Two unbounded-memory fixes: the per-client API rate limiter and the
  HA admin-session cache now sweep stale entries (the latter was a
  pre-auth memory-DoS vector).

**Admin & UX**

- **Dashboard stop/restart confirmation actually works again** — the
  live-updated rows carried a broken inline handler, so destructive
  actions fired with no confirmation at all. Confirmations across the
  whole admin now use a single robust mechanism that survives any
  translation (the French strings used to break it), enforced by a new
  template lint.
- The dashboard's live updates no longer steal keyboard focus every
  second; sortable tables and audit rows are keyboard-accessible.
- Editing the same app in two tabs no longer silently overwrites the
  other editor's changes (a conflict banner asks to re-submit); creating
  an app with a taken id fails cleanly under concurrency; replica
  stop/restart actions are written to the audit log.
- The landing editor's image picker got the same dialog accessibility as
  the app form's; assorted i18n fixes (subject suggestions now come from
  your own catalog instead of a fixed list).

**Configuration**

- `serve` now runs validation at startup and logs every warning — and
  ShinyProxy fields that parse but have no effect in Ruscker
  (`server.secure-cookies`, `proxy.hide-navbar`, …) each warn instead of
  being silently ignored.
- `type: streamlit | dash | voila` without `container-port` now forwards
  to the framework's well-known port (8501 / 8050 / 8866) instead of
  Shiny's 3838.
- `${VAR}` edge cases: a nested default (`${A:-${B}}`) is refused loudly
  instead of corrupting the value, and `container-env` placed as the
  first key of a spec no longer exempts the spec's other fields from
  interpolation.

---

## v0.2.4 — 2026-06-09

**Logo picker is now a searchable modal.** The app form's inline logo
thumbnail grid didn't scale — with a large media library the tiles
overflowed and overlapped (an SVG with a big viewBox got no height from
`aspect-ratio` and blew up its cell). It's now a compact control (current
logo thumbnail + "Choose from library") that opens the shared image-picker
modal with search, the full library, and inline upload. Tiles got a
`min-height` fallback so a failed `aspect-ratio` can't overflow them, and
logos render `contain` (no crop).

---

## v0.2.3 — 2026-06-09

**Gradient card-cover preview fix.** A gradient default card cover didn't
show in the Appearance preview (the mock cards stayed grey) when the
**Card covers: Gradient** toggle was on — the preview emitted two
`background-image` declarations, so the subtle overlay clobbered the
colour gradient. The preview now shows an explicit cover as-is, mirroring
the public landing. Also: the gradient builders now seed from the saved
value (`gradientParse`), so reopening shows the saved stops and editing a
saved gradient modifies it in place instead of silently resetting it to
the default palette.

---

## v0.2.2 — 2026-06-09

**Card-cover preview fix.** Selecting the **Solid** card-cover mode only
flipped the editing mode and left the value empty, so the live preview
didn't change until the colour picker was dragged — it read as "the
cover adjustment isn't reflecting". Switching to Solid now seeds a
starting brand colour, so the mock card updates immediately and the
picker edits it live. Applies to both the Appearance default-cover
builder and the per-app cover builder in the spec form.

---

## v0.2.1 — 2026-06-09

**Apps editor + Appearance polish.** Four operator-reported fixes:

- **Current logo reads as selected** in the inline logo gallery. The
  match was an exact path compare, which missed a stored value carrying
  a base-path prefix (`/box/assets/img/…`); it now compares by filename.
- **Card cover drops the "Image" mode.** A logo renders *on top* of the
  cover, so an image cover + a logo painted two overlapping pictures on
  one card. Cover is now tint / colour / gradient only; a legacy image
  cover degrades to the kind-tint/accent fallback.
- **Environment-variable rows** are laid out one clean line each
  (`KEY · = · value · ✕`) instead of the inputs stacking full-width.
- **Default card cover in Appearance.** A new Auto / Solid / Gradient
  builder in the Background section sets one catalog-wide default cover
  for cards without their own `cover`/`accent` (which still win), so the
  default is no longer editable only per app (migration 0021).

---

## v0.2.0 — 2026-06-09

**Apps editor redesign complete + per-app accent & monogram.** The
final piece of the editor rework: each app can now set an **accent
colour** (tints the card cover when no cover is set) and a **monogram**
(1–2 chars shown on the cover when there's no logo) — both stored in
`template-properties`, no migration. The editor's Appearance section
gains a swatch row and a monogram field, and the live preview reflects
them. This closes the editor redesign that also brought the handoff
section structure, the inline logo gallery, and the Access & scale
section (v0.1.98–0.1.99).

---

## v0.1.99 — 2026-06-08

**Apps editor — Access & scale.** The old "Metadata" section is now
**Access & scale**, matching the design: a **Restricted access** toggle
(off = public, and turning it off clears the group/user lists), the
**Initial replicas** stepper surfaced alongside access, and an
**Autoscaling** toggle (in Advanced) that gates the replica ceiling and
thresholds. (Still to come: accent colour + monogram.)

---

## v0.1.98 — 2026-06-08

**Apps editor — closer to the design.** The Edit-application form now
follows the handoff structure: **Identity** (ID + Name side-by-side,
Subject) → **Kind** → **Description** (its own section) → **Appearance**
→ Container → Metadata. The Appearance section gets an inline **logo
gallery** — pick an app logo straight from the media library (or upload
via the last tile) instead of opening a modal. (More of the editor —
access/scale toggles, accent colour, monogram — lands next.)

---

## v0.1.97 — 2026-06-08

**Logs spacing fix.** The log lines gave the app column a fixed width, so
lines without an app (most infra events) showed a large empty gap between
the level and the message. The column now collapses when empty, so the
message sits right after the level.

---

## v0.1.96 — 2026-06-08

**Logs tab — colour-coded event stream.** The Logs view rendered every
line in flat grey because the parser expected a log format the server
doesn't emit. It now colours each level (INFO blue, WARN amber, ERROR
red), shows the app name and a millisecond timestamp, and the toolbar
matches the design: a Pause button, Info/Warn/Error level chips, an
"All apps" filter, a live line count, clear, and download — in one card.

---

## v0.1.95 — 2026-06-08

**Logo controls behave as expected.** Three appearance-editor logo
behaviors that read as bugs are fixed:

- The **Logo size / margin** sliders now resize a custom header logo too,
  not just the built-in mark.
- A custom header logo in **any** position (left/center/right) now hides
  the built-in Ruscker mark — no more mark-plus-logo "two logos".
- The mark is **always brand-colored**; a custom header background no
  longer turns it grey.

---

## v0.1.94 — 2026-06-08

**Portal no longer cached + clearer header labels.**

- The public portal is now served `Cache-Control: private, no-cache`, so
  appearance changes (catalog layout, colors, …) show on the next load
  instead of being masked by a browser or proxy cache — and a shared
  cache can no longer replay one visitor's access-filtered view to
  another. Bundled assets stay cached as before.
- Renamed two header controls that read alike: the preset is now **Header
  style** (was "Portal header") and the explicit color is **Custom
  background color** (was "Background color"), with a note that the
  custom color overrides the preset.

---

## v0.1.93 — 2026-06-08

**Appearance editor fixes.** Two follow-ups from testing the editor:

- **Image picker on screen.** The "Choose image" modal could open
  centered in the (tall) editor page instead of the viewport — often
  below the fold, hidden. It now teleports to `<body>` so it always
  centers on screen.
- **Live preview reflects every control.** The editor's portal preview
  used to mirror only a few fields; it now reacts to the per-theme
  palette and default theme (the whole frame repaints light/dark), logo
  mode/size, header preset, card-cover style, catalog layout
  (grid/list/sections) and density, and the visible-section toggles.

---

## v0.1.92 — 2026-06-08

**Appearance — catalog "Sections" layout + editor card order.** The
catalog-layout picker gains a third option, **Sections**: the portal
catalog grouped by app type, each group under a heading that hides itself
when the live filters (search / access / status / type) empty it. Grid
and List are unchanged. The appearance editor's cards are also reordered
to match the design handoff (logo controls grouped together, default
theme ahead of catalog layout).

---

## v0.1.91 — 2026-06-08

**Appearance — analytics provider picker.** The appearance editor's
Analytics section now offers a provider picker (Google Analytics 4,
Plausible, or Matomo) plus a site-key field; the portal builds the
standard snippet from provider + key and opens the matching CSP origins.
The raw analytics-HTML field stays as an escape hatch for anything else.

---

## v0.1.90 — 2026-06-08

Appearance editor rebuilt toward the design handoff, plus a Disk-tab
polish.

**Appearance editor** (the admin "Portal" tab is now **Appearance**, to
free "Portal" for the back-to-portal link)
- **Footer** text is editable; blank keeps the version + wordmark lockup.
- **Default theme** (light/dark/auto) for a first-time visitor; their own
  toggle still overrides it.
- **Visible sections**: toggle the portal search bar and access filters.
- **Brand color** swatch row sets the accent in one click.
- **Logo**: header brand mode (mark+name / symbol-only / custom) with size
  and margin.
- **Background**: header preset (flat/soft/bold) and card cover style
  (tinted/gradient).
- **Catalog layout**: grid or list, comfortable or compact density.

**Disk tab**
- The table search boxes get a proper inset + search glyph, and the images
  / containers panels size to their own content (no blank space under the
  shorter table).

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
  at capacity it says "`<app>` is full right now — this page opens
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
