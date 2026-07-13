# Roadmap

> **Status: Phases 0–7 shipped and running in production**, followed by
> the v0.2.x polish series (the v0.2.5 audit release and the
> admin design-handoff sprint). Phase 8 is optional and demand-driven.
> For the narrative, up-to-date version with a timeline diagram, see
> the [Roadmap chapter](https://strategicprojects.github.io/ruscker/roadmap.html)
> on the docs site. The phase checklists below are the original
> planning detail (they predate v0.1.0); every item in Phases 0–7 has
> since shipped, so their boxes are ticked. Phase 8 shows what remains.

Phased plan from "config parsing only" to "production-ready ShinyProxy
replacement".

Each phase has a clear deliverable. Skip ahead if a phase doesn't
apply to your use case (e.g. skip phase 4 if you don't need a
dashboard).

## Phase 0 — Scaffolding (DONE)

**Status: ✅ shipped.**

- [x] Cargo workspace with 6 crates
- [x] YAML schema covering ShinyProxy + Ruscker extensions
- [x] Env var interpolation (`${VAR}`, `${VAR:-default}`)
- [x] Two-phase validation (raw text scan + typed model checks)
- [x] CLI binary with `validate`, `show`, `inspect`
- [x] Integration tests against the real `application.yml`
- [x] Per-crate developer docs (module-level)
- [x] Mockups in `docs/mockups/`
- [x] Architecture, roadmap, ADRs

**Deliverable**: `ruscker validate examples/application.yml` produces
a useful report.

## Phase 1 — Landing page (1 week)

**Goal**: replace the Thymeleaf-based ShinyProxy landing with Askama +
Tailwind 4. Specs and template properties drive what gets rendered.

- [x] Add a new `ruscker-web` crate (or fold into `ruscker-admin`)
- [x] Askama templates matching `docs/mockups/landing-page.html`
- [x] Type badge colors and access icons driven by template properties
- [x] Filter chips with counts (matching
  `docs/mockups/landing-filters-cards-refined.html`)
- [x] Light/dark theme toggle (cookie + system preference)
- [x] Image library: serve static assets from a configurable
  directory
- [x] Tailwind 4 build script (no Node required, uses standalone CLI)

**Deliverable**: visit `localhost:8080` and see the bundled
`examples/application.yml` portal rendered, looking visually
equivalent to today but in Tailwind. Cards are not yet clickable
(no proxy yet).

## Phase 2 — Persistence + admin CRUD (3 weeks)

**Goal**: edit specs via web UI, no YAML editing required.

- [x] SQLite schema + sqlx migrations
- [x] Importer: `ruscker import application.yml --db ruscker.db`
- [x] Exporter: `ruscker export --db ruscker.db > application.yml`
- [x] Admin login (basic auth or env-var token for MVP; OIDC later)
- [x] Apps list page (filter, search, bulk actions)
- [x] Add/Edit spec form with type-driven fields (mockup:
  `admin-add-edit-app.html`)
- [x] Image library: upload, convert to WebP, optimize
- [x] Credentials store: named entries, encrypted at rest
- [x] Landing-page section editor (mockup:
  `admin-landing-editor.html`)
- [x] Audit log of all admin actions

**Deliverable**: the operator can fully manage Ruscker via web UI
without touching files.

## Phase 3 — Proxy + Docker backend (3 weeks)

**Goal**: actually run apps. The technically hardest phase.

- [x] `LocalDockerBackend` via bollard
  - [ ] Image pulling with registry credentials
  - [ ] Container spawn with resource limits, env, volumes
  - [ ] Health-check polling
  - [ ] Graceful stop (drain → SIGTERM → SIGKILL)
- [x] HTTP request forwarding via hyper
- [x] WebSocket upgrade hijack + bidirectional pump
- [x] Path rewriting for Shiny's relative URLs
- [x] Sticky session cookie signing (HMAC-SHA256 via `ring`)
- [x] Session store (`DashMap` impl of `SessionStore`)
- [x] Replica pool per spec with `Router` plumbed in
- [x] Auto-scaler: spawn when utilization > threshold, retire when
  under threshold for grace period
- [x] Heartbeat handling (per-spec timeout overrides, `-1` = never)
- [x] API spec branch: HTTP-only, round-robin, no cookie

**Deliverable**: end-to-end Shiny session works. Open Aurora Prime
card on the landing, app loads, reactivity works, session survives
page refresh.

## Phase 4 — Monitoring dashboard (2 weeks)

**Goal**: visibility into running containers.

- [x] `/admin/dashboard` page (mockup: `admin-dashboard.html`)
- [x] Metric cards: containers, sessions, memory, CPU
- [x] Live container table with utilization bars (SSE updates)
- [x] Sessions-over-24h sparkline chart
- [x] Recent events feed
- [x] Logs streaming page with filter + regex + follow mode
- [x] Container detail view: live CPU/mem charts, environment, logs
- [x] Alert notification webhook (#930) — spawn-failed / replica-down /
      saturated events POST JSON to an operator-configured URL
      (admin System tab), with retry, cooldown dedup and a test button
- [x] Prometheus metrics endpoint at `/metrics`

**Deliverable**: open the dashboard, see everything that's running,
drill into any container.

## Phase 5 — Production polish (1 week)

- [x] Graceful shutdown (drain active sessions before exit)
- [x] structured logging via `tracing` + `tracing-subscriber`
- [x] Rate limiting on API specs (token bucket)
- [x] CORS support for API specs
- [x] Configurable max body sizes
- [x] Health endpoint `/healthz`
- [x] Readiness endpoint `/readyz`
- [x] Multi-stage Dockerfile for Ruscker itself
- [x] systemd unit file
- [x] Documentation: deployment guide, migration guide from
  ShinyProxy, troubleshooting

**Deliverable**: ready to drop into production.

## Phase 6 (optional) — Multi-host scheduling

- [x] `MultiHostDockerBackend` (talks to multiple Docker hosts)
- [x] Bin-pack vs spread strategies
- [x] Per-spec anti-affinity ("replicas must be on different hosts")

## Phase 7 (optional) — HA / multi-instance

- [x] Postgres `SessionStore` implementation
- [x] Postgres replication for SQLite admin DB
- [x] Coordination via Postgres advisory locks
- [x] Failover testing

## Phase 8 (optional) — Auth

- [ ] OIDC integration (Keycloak, Auth0, Google)
- [ ] SAML for enterprise
- [x] Role-based access control (Viewer / Editor / Admin) — shipped
- [x] Per-spec access lists (only group X can use this app) — shipped in Phase 6

> Only the external identity-provider items (OIDC / SAML) remain; the
> coarse role model and per-spec access lists already ship.

## What we explicitly defer / don't do

- **Kubernetes backend** — possible, but not until there's demand.
  `KubernetesBackend` is a long-term aspiration, not a phase.
- **App ecosystem features** — pause/resume, snapshots, etc. These
  are ShinyProxy Pro features. Not in MVP.
- **Multi-tenancy** — separate organizations / billing. Possibly a
  commercial extension; not in open source.
- **Public marketplace of apps** — outside scope.

## Definition of "ready to replace ShinyProxy"

After Phase 5, Ruscker can be deployed as a drop-in replacement for
existing ShinyProxy installations. The migration path:

1. `ruscker import application.yml --db /var/ruscker/ruscker.db`
2. Stop ShinyProxy
3. Start Ruscker on the same port
4. Verify with the same browser URL

Phases 6+ are for organizations with more demanding needs.
