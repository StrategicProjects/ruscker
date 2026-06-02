# Roadmap

Ruscker is at **v0.1.39** — Phases 0 through 7 are done and the proxy is
production-ready and horizontally scalable. Phase 8 (external auth) is
the main optional, demand-driven work left. For what changed in each
release, see the [release notes](./news.md).

![Roadmap timeline: phases 0–7 are done — 0 to 5 shipped in v0.1.0 (scaffolding, landing page, persistence + admin CRUD, proxy + Docker backend, monitoring dashboard, production polish); phase 6 (multi-host scheduling, app visibility, sub-path mounting) across v0.1.1–v0.1.2; phase 7 (HA / multi-instance) in v0.1.1. The latest release v0.1.31 adds demo forks, URL-rewrite modernization, unified credentials, media library, portal logos, and security + perf fixes. Phase 8 (external auth) is planned and optional.](images/roadmap.svg)

## Shipped

### Phase 0 — Scaffolding
The Cargo workspace, the ShinyProxy-compatible YAML schema, env-var
interpolation, two-phase validation, and the `ruscker validate` /
`show` / `inspect` CLI.

### Phase 1 — Landing page
The public portal rendered from config with Askama + Tailwind 4 (no
Node toolchain), full i18n in **pt-BR / en-US / es-ES / fr-FR**, theme
and locale cookies, and the kind-tinted card grid with filters.

### Phase 2 — Persistence + admin CRUD
SQLite as the source of truth (sqlx, embedded migrations),
`ruscker import` / `export` to round-trip YAML, and the admin panel:
apps list + spec form, image/media library (WebP conversion), an
AES-256-GCM credentials store, the landing-page editor, and an audit
log.

### Phase 3 — Proxy + Docker backend
HTTP forwarding, **sticky sessions** (HMAC-signed cookie), **WebSocket
proxying**, the Docker backend (spawn / stop / stats / logs via
bollard), per-spec replica pools, the **auto-scaler** (scale-to-min,
scale-up on saturation, scale-down on idle with hysteresis), the
session tracker + heartbeat sweeper, absolute-URL rewriting so
unmodified Shiny/Streamlit apps work behind a sub-path, and
per-container CPU/memory limits.

### Phase 4 — Monitoring dashboard
A live dashboard over **Server-Sent Events**: aggregate cards,
per-replica state / uptime / sessions / **CPU + memory** (with
sparklines), one-shot and **live-follow logs**, per-replica
**stop / restart**, and a Prometheus `/metrics` endpoint.

### Phase 5 — Production polish → **v0.1.0**
`/healthz` + `/readyz` probes, graceful shutdown (session drain),
structured JSON logging, per-API rate limiting + CORS, request
body-size limits, `validate --strict-compat` migration pre-flight,
**role-based access control** (Viewer / Editor / Admin with per-user
password accounts), smart-routing headers (`X-Forwarded-Prefix` …),
and distribution: a **multi-arch Docker image**, a **Debian package**
with a hardened systemd unit, static **musl tarballs**, a `curl | sh`
installer, a **Homebrew tap**, and **cosign-signed** release artifacts.

> **Production milestone.** Ruscker replaced a JVM-based stack on the
> same machine serving the same apps, cutting idle memory from
> **~540 MB to ~14 MB** (~38×). A real 31-spec config migrated with no
> unsupported features.

### Phase 6 — Multi-host scheduling → **v0.1.1 / v0.1.2**
A `MultiHostDockerBackend` that talks to several Docker hosts, with
bin-pack vs spread placement and per-spec anti-affinity ("replicas on
different hosts") — behind the existing `ContainerBackend` trait, no
proxy changes. Shipped alongside per-group / per-user **app visibility**
(`access-groups` / `access-users`) and **sub-path mounting**
(`server.context-path` / `--base-path`).

### Phase 7 — HA / multi-instance → **v0.1.1**
A Postgres `SessionStore` and a shared config catalog so several Ruscker
instances behind an L4 load balancer can share state and any instance
can serve any session; one scaler leader via Postgres advisory locks,
with failover. A runnable 2-instance compose harness lives in
`examples/ha/`. See [Deployment shapes](./architecture.md#deployment-shapes).

### Post-phase polish → **v0.1.4 – v0.1.39**

Incremental improvements shipped after Phase 7.

**Demo app images.** The Dash, FastAPI, and Quarto showcase cards now
point to dedicated **`milkway/ruscker-*-demo`** images on Docker Hub.
The Quarto demo is a static nginx image (~67 MB); Dash and FastAPI serve
at the root without needing `SHINYPROXY_PUBLIC_PATH`. Demo forks for
Shiny, Streamlit, and Voilà are **backlog** — those cards still point to
upstream images.

**URL-rewrite modernization.** The runtime shim that rewrites relative
asset paths under `/app/{id}/` is now generalized to patch
`script.src`, `link.href`, `img.src`, and `setAttribute`, which retired
the Voilà-specific rewriter. The Jupyter-config rewriter (`rewrite_jupyter_config`)
is kept: JupyterLab builds absolute same-origin API URLs from
`PageConfig.baseUrl` that a root-relative shim cannot intercept. A
full absolute-URL Path B rewrite (handling apps that hard-code
`window.location.origin`) is **deferred** — the current shim covers all
validated app types.

**Credentials.** The named-credential store now accepts a pure
`${VAR}` env-ref in addition to an AES-256-GCM literal, resolved only
at container pull time. The spec form's Registry section is now a
credential picker; inline domain/user/password fields remain as
back-compat.

**Media library.** Built-in logos are seeded into the unified media
library (deletable, with an "in use" badge). The spec-form image picker
supports inline upload. Gallery pages are paginated with search.

**Portal logos.** The landing editor supports per-slot logos (header /
footer) with alignment (left / center / right), an optional link, and a
per-logo height.

**Performance.** gzip/br compression on admin and landing responses;
`?v=<version>` immutable cache headers on bundled CSS/JS; ETag on media
assets; WebP thumbnails; bounded Docker stats fan-out with a
configurable `metrics-interval`.

**Security fixes.** API routing uses in-flight count (not seats);
charset validation on usernames and credential names; admin password
fields are write-only in the UI; `${VAR}` resolution returns an error
when the variable is unset (names the missing variable).

**Disk management & admin polish (v0.1.32–v0.1.33).** A new **Disk**
panel reclaims space: remove containers, prune every stopped one
(label-scoped, never touching a non-Ruscker container), and remove
unused images — individually or all at once. Deleting an app now reaps
its containers instead of leaving orphans. New accounts must change
their password on first login. A one-line startup banner (version,
bind, base path, Docker, database, spec count) shows in the admin Logs
tab at the default log level. The Portal logos editor reuses the spec
form's image gallery picker.

**Docker-by-default & Portal branding (v0.1.34–v0.1.35).** `serve`
**auto-connects to Docker** when the daemon socket is reachable (pass
`--no-docker` for landing-only); showcase demos seed with
`min-replicas: 0` so a fresh install no longer pre-spawns every demo
container. The Portal gains **per-theme colours** (independent
background/text/accent for light and dark) and **logos that integrate
into the chrome** — a header-left logo replaces the Ruscker mark,
header-right trails the buttons, footer-right trails the version, and a
center logo is centred within the bar; each takes an optional margin.
The **landing editor** is reorganised into labelled section cards with a
sticky Save bar, logos are edited as cards with segmented
position/alignment pickers, and the live preview mirrors the real portal
chrome. The social-share **`og:image` auto-defaults** to the header logo
(else the Ruscker mark) and gets a gallery picker. Finally, the **live
dashboard streams through reverse proxies** (`X-Accel-Buffering: no`), so
new containers appear in real time even behind nginx on a subpath mount.

## Planned (optional)

Demand-driven — Ruscker is complete and useful without it.

### Phase 8 — External auth
OIDC (Keycloak / Auth0 / Google), SAML, and LDAP, plus per-app access
lists ("only group X can use this app"). The coarse Viewer / Editor /
Admin RBAC already shipped in Phase 5; this is the federated-identity
and fine-grained-ACL layer on top.

## Explicitly out of scope

- **Kubernetes backend** — possible as a future `ContainerBackend`
  impl, but not a committed phase until there's demand.
- **App-ecosystem features** (pause/resume, snapshots) — these are
  ShinyProxy Pro territory.
- **Multi-tenancy / billing** and a **public app marketplace**.

## What "done" means

After Phase 5, Ruscker can drop in where ShinyProxy runs today:

1. `ruscker import application.yml --db /var/ruscker/ruscker.db`
2. Stop ShinyProxy.
3. Start Ruscker on the same port.
4. Verify with the same browser URL.

Phase 8 is for organisations with federated-identity or
fine-grained per-app access needs. Progress is tracked in the
GitHub issues.
