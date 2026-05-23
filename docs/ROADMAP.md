# Roadmap

Phased plan from "config parsing only" (today) to "production-ready
ShinyProxy replacement" (~8-10 weeks of focused work).

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
- [x] CLAUDE.md memory for every crate
- [x] Mockups in `docs/mockups/`
- [x] Architecture, roadmap, ADRs

**Deliverable**: `ruscker validate examples/application.yml` produces
a useful report.

## Phase 1 — Landing page (1 week)

**Goal**: replace the Thymeleaf-based ShinyProxy landing with Askama +
Tailwind 4. Specs and template properties drive what gets rendered.

- [ ] Add a new `ruscker-web` crate (or fold into `ruscker-admin`)
- [ ] Askama templates matching `docs/mockups/landing-page.html`
- [ ] Type badge colors and access icons driven by template properties
- [ ] Filter chips with counts (matching
  `docs/mockups/landing-filters-cards-refined.html`)
- [ ] Light/dark theme toggle (cookie + system preference)
- [ ] Image library: serve static assets from a configurable
  directory
- [ ] Tailwind 4 build script (no Node required, uses standalone CLI)

**Deliverable**: visit `localhost:8080` and see the SEPE portal
rendered from the YAML, looking visually equivalent to today but in
Tailwind. Cards are not yet clickable (no proxy yet).

## Phase 2 — Persistence + admin CRUD (3 weeks)

**Goal**: edit specs via web UI, no YAML editing required.

- [ ] SQLite schema + sqlx migrations
- [ ] Importer: `ruscker import application.yml --db ruscker.db`
- [ ] Exporter: `ruscker export --db ruscker.db > application.yml`
- [ ] Admin login (basic auth or env-var token for MVP; OIDC later)
- [ ] Apps list page (filter, search, bulk actions)
- [ ] Add/Edit spec form with type-driven fields (mockup:
  `admin-add-edit-app.html`)
- [ ] Image library: upload, convert to WebP, optimize
- [ ] Credentials store: named entries, encrypted at rest
- [ ] Landing-page section editor (mockup:
  `admin-landing-editor.html`)
- [ ] Audit log of all admin actions

**Deliverable**: the operator can fully manage Ruscker via web UI
without touching files.

## Phase 3 — Proxy + Docker backend (3 weeks)

**Goal**: actually run apps. The technically hardest phase.

- [ ] `LocalDockerBackend` via bollard
  - [ ] Image pulling with registry credentials
  - [ ] Container spawn with resource limits, env, volumes
  - [ ] Health-check polling
  - [ ] Graceful stop (drain → SIGTERM → SIGKILL)
- [ ] HTTP request forwarding via hyper
- [ ] WebSocket upgrade hijack + bidirectional pump
- [ ] Path rewriting for Shiny's relative URLs
- [ ] Sticky session cookie signing (HMAC-SHA256 via `ring`)
- [ ] Session store (`DashMap` impl of `SessionStore`)
- [ ] Replica pool per spec with `Router` plumbed in
- [ ] Auto-scaler: spawn when utilization > threshold, retire when
  under threshold for grace period
- [ ] Heartbeat handling (per-spec timeout overrides, `-1` = never)
- [ ] API spec branch: HTTP-only, round-robin, no cookie

**Deliverable**: end-to-end Shiny session works. Open Aurora Prime
card on the landing, app loads, reactivity works, session survives
page refresh.

## Phase 4 — Monitoring dashboard (2 weeks)

**Goal**: visibility into running containers.

- [ ] `/admin/dashboard` page (mockup: `admin-dashboard.html`)
- [ ] Metric cards: containers, sessions, memory, CPU
- [ ] Live container table with utilization bars (SSE updates)
- [ ] Sessions-over-24h sparkline chart
- [ ] Recent events feed
- [ ] Logs streaming page with filter + regex + follow mode
- [ ] Container detail view: live CPU/mem charts, environment, logs
- [ ] Configurable alert thresholds (notification webhooks deferred)
- [ ] Prometheus metrics endpoint at `/metrics`

**Deliverable**: open the dashboard, see everything that's running,
drill into any container.

## Phase 5 — Production polish (1 week)

- [ ] Graceful shutdown (drain active sessions before exit)
- [ ] structured logging via `tracing` + `tracing-subscriber`
- [ ] Rate limiting on API specs (token bucket)
- [ ] CORS support for API specs
- [ ] Configurable max body sizes
- [ ] Health endpoint `/healthz`
- [ ] Readiness endpoint `/readyz`
- [ ] Multi-stage Dockerfile for Ruscker itself
- [ ] systemd unit file
- [ ] Documentation: deployment guide, migration guide from
  ShinyProxy, troubleshooting

**Deliverable**: ready to drop into production.

## Phase 6 (optional) — Multi-host scheduling

- [ ] `MultiHostDockerBackend` (talks to multiple Docker hosts)
- [ ] Bin-pack vs spread strategies
- [ ] Per-spec anti-affinity ("replicas must be on different hosts")

## Phase 7 (optional) — HA / multi-instance

- [ ] Postgres `SessionStore` implementation
- [ ] Postgres replication for SQLite admin DB
- [ ] Coordination via Postgres advisory locks
- [ ] Failover testing

## Phase 8 (optional) — Auth

- [ ] OIDC integration (Keycloak, Auth0, Google)
- [ ] SAML for enterprise
- [ ] Role-based access control (viewer / operator / admin)
- [ ] Per-spec access lists (only group X can use this app)

## What we explicitly defer / don't do

- **Kubernetes backend** — possible, but not until there's demand.
  `KubernetesBackend` is a long-term aspiration, not a phase.
- **App ecosystem features** — pause/resume, snapshots, etc. These
  are ShinyProxy Pro features. Not in MVP.
- **Multi-tenancy** — separate organizations / billing. Possibly a
  commercial extension; not in open source.
- **Public marketplace of apps** — outside scope.

## Definition of "ready to replace ShinyProxy"

After Phase 5, Ruscker can be deployed where SEPE currently runs
ShinyProxy. The migration path:

1. `ruscker import application.yml --db /var/ruscker/ruscker.db`
2. Stop ShinyProxy
3. Start Ruscker on the same port
4. Verify with the same browser URL

Phases 6+ are for organizations with more demanding needs.
