# Roadmap

Ruscker is at **v0.1.3** — Phases 0 through 7 are done and the proxy is
production-ready and horizontally scalable. Phase 8 (external auth) is
the main optional, demand-driven work left. For what changed in each
release, see the [release notes](./news.md).

![Roadmap timeline: phases 0–7 are done — 0 to 5 shipped in v0.1.0 (scaffolding, landing page, persistence + admin CRUD, proxy + Docker backend, monitoring dashboard, production polish), and 6 (multi-host scheduling) and 7 (HA / multi-instance) followed in v0.1.1. Phase 8 (external auth) is planned and optional.](images/roadmap.svg)

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
> **~540 MB to ~16 MB** (~30×). A real 31-spec config migrated with no
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
