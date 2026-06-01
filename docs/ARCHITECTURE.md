# Architecture

Ruscker is a Rust-based proxy and orchestrator for containerized
interactive web apps and stateless HTTP APIs. This document describes
how the pieces fit together.

## High-level diagram

![How Ruscker works: browsers and API clients hit a single Ruscker binary, which serves the landing page + admin and reverse-proxies to app containers it spawns on demand via the Docker daemon.](images/architecture.svg)

All of this is a single Rust process — one static binary, ~16 MB idle,
no JVM. Visitors and API clients reach it on one port; it serves the
landing page and admin UI, reverse-proxies `/app/{spec}` and
`/api/{spec}` to the right replica (keeping Shiny sessions sticky and
upgrading WebSockets), and drives the Docker daemon to spawn and reap
containers. SQLite is the source of truth for configuration; the live
replica registry and session store live in memory.

## Crate map

The workspace is six crates. `ruscker-config` and `ruscker-core` are
**pure-domain** — no I/O, no async (bar the async trait *definitions*
in core). Everything that touches the network or Docker layers on top,
and the `ruscker-cli` binary stitches them together.

![Crate dependency map: ruscker-cli builds on the I/O crates (docker, proxy, admin), which build on ruscker-core, which builds on ruscker-config.](images/crate-map.svg)

Keeping the backend behind the `ContainerBackend` trait in
`ruscker-core` means a future Kubernetes or multi-host backend is a new
impl, not a rewrite — see [Deployment shapes](#deployment-shapes) and
`docs/adr/`.

## Request flow

### A Shiny session lifecycle

```
1. Visitor hits  https://portal/app/sales-dashboard/
2. Proxy reads cookie  __ruscker_session
3. Cookie missing → Proxy.create_session:
     a. Look up spec 'sales-dashboard' in config
     b. Ask ContainerBackend.list() for current replicas
     c. Router.pick(replicas) → ReplicaDecision::Use(R2)  (least-conn)
     d. If Saturated:
          - Check spec.max_replicas
          - If room, ContainerBackend.spawn() → wait for Ready → retry
          - Else 503
     e. SessionStore.create(Session { spec, replica: R2 })
     f. Sign and set cookie  __ruscker_session
4. Forward GET /  to  http://127.0.0.1:<R2_port>/   (path rewrite)
5. Stream response back
6. Browser opens WebSocket  ws://portal/app/sales-dashboard/websocket
7. Proxy upgrades, opens parallel WS to  ws://127.0.0.1:<R2_port>/websocket
8. Bidirectional frame pump
9. On heartbeat: SessionStore.touch()
10. Idle timeout reached → Session purged → if last seat, container drained
```

### An API request lifecycle

```
1. Client hits  https://portal/api/data-api/v1/data
2. Spec.kind() == Api  → no sticky cookie path
3. Router.pick() balances by in-flight request count → R3
4. Bump R3's in-flight gauge, forward request, stream response
5. In-flight gauge drops only after the full body has streamed out
6. No session state, no follow-up — done.
```

An `Api` spec has no sticky sessions, so its replicas have no seat
notion to balance on. Instead the proxy keeps a per-replica **in-flight
request gauge** (`routes::proxy::INFLIGHT`, a process-global `DashMap`)
and least-connections routing picks the replica with the **fewest
in-flight requests**, not the most free seats. An RAII
`routes::proxy::InflightGuard` bumps the gauge when the forward starts;
crucially it is **moved into the streaming response body**, so it only
drops once the whole (possibly long) download has been sent to the
client — a large file transfer keeps counting against the replica for
its full duration, and the scaler sees real concurrency rather than a
spike that vanishes the instant headers are written.

### Proxying an app under `/app/{spec}/` — the strip-and-rewrite model

A containerised app expects to live at the host root: it emits
`/lib/jquery.js`, opens `WebSocket('/websocket')`, redirects to `/lab`.
Ruscker serves it from a sub-path (`/app/sales-dashboard/`). Two halves
reconcile that gap.

**On the way in, the proxy *strips* the mount prefix.** `forward()`
matches `/app/{spec}/{*rest}` and forwards only the `*rest` portion to
the container, so a request for `/app/sales-dashboard/lib/x` reaches the
upstream as `/lib/x` — the container believes it is at the root and
never has to know its public path. (This is the opposite of ShinyProxy's
no-strip model; apps should be configured to serve at root, not to
self-prefix.) The proxy also stamps `X-Forwarded-Prefix` /
`X-Script-Name` / `X-RStudio-Root-Path` with the public mount so apps
that *do* build their own absolute URLs (RStudio, Jupyter) emit correct
links — see `routes::proxy::apply_smart_routing_headers`.

**On the way out, the proxy *rewrites* the response so the browser sends
follow-up requests back under the mount.** This lives in
`routes::rewrite` (`inject_base_href`) and runs only on the
`/app/` route family, only for HTML responses:

- **`<base href="/app/{spec}/">`** is injected at the top of `<head>`,
  so relative URLs (`foo.css`, `./img/x.png`) resolve under the mount.
- **Root-absolute attribute URLs** (`<script src="/lib/x">`,
  `<link href="/...">`, `<form action="/...">`, …) are prefixed with the
  mount via a streaming `lol_html` pass over a narrow selector set. A
  skip-list (`/admin/`, `/assets/`, `/app/`, …) avoids double-prefixing
  Ruscker's own chrome; notably `/api/` is **not** skipped, because under
  the mount it is the *app's* own namespace (Jupyter's REST + kernel
  WebSocket live there).
- **A runtime JS shim** is prepended before any page script. It
  monkey-patches `fetch`, `XMLHttpRequest.open`, and `WebSocket` to
  prefix absolute paths built at runtime. The shim was **generalized**
  to also patch the resource-loading property setters
  `HTMLScriptElement.prototype.src`,
  `HTMLLinkElement.prototype.href`, and `HTMLImageElement.prototype.src`
  (plus `iframe`/`audio`/`video`/`source` and `Element.setAttribute`).
  Those are the browser's *own* fetches — never visible to the fetch/XHR
  wrappers — so patching them covers RequireJS/webpack chunk loading and
  runtime-set images generically.
- **A redirect `Location` header** that points at a root-absolute path
  (an app's `302 → /lab`) is prefixed the same way, so the redirect stays
  inside the app instead of escaping to a Ruscker 404.

The generalized shim **retired the old Voilà-specific rewrite**: Voilà's
RequireJS bootstrap assigns its static URLs to `script.src` at runtime,
which the patched `src` setter now prefixes without a bespoke pass.

**JupyterLab is the one app that still needs a special case**
(`rewrite::rewrite_jupyter_config`). Lab is served with `base_url=/` and
reports `baseUrl: "/"` in its `jupyter-config-data` JSON; its bootstrap
then builds **absolute, same-origin** API and static URLs from that
config and injects `<script src=…>` for its lazy chunks. Because those
URLs are absolute strings baked into a config object — not relative paths
the browser resolves against `<base href>`, and not paths a root-relative
shim can intercept — Ruscker rewrites the `baseUrl` and `full*Url` fields
of that JSON to carry the mount before the HTML pass.

The base-path mount (Ruscker itself served under, e.g., `/apps`) is the
inverse rewrite and is handled separately: templates emit `{{ base }}`-
prefixed URLs directly, so the chrome no longer needs a per-request body
rewrite — only the redirect `Location` header (`prefix_base_path`).

## Module boundaries

### Pure layer (no I/O, no async)

- `ruscker-config::schema`
- `ruscker-config::env`
- `ruscker-config::validate`
- `ruscker-core::routing`
- `ruscker-core::replica` (types only)
- `ruscker-core::session` (types only — `SessionStore` trait is async,
  but the trait def is pure)

### I/O layer (async + tokio)

- `ruscker-docker` — talks to Docker
- `ruscker-proxy` — listens on a TCP socket
- `ruscker-admin` — listens on another TCP socket
- `ruscker-cli` — synchronous main, spawns tokio runtime for I/O
  commands

## State and persistence

### Three sources of state, ranked by authority

1. **SQLite (admin DB)** — source of truth for spec configurations,
   images, credentials, landing-page sections, audit log. Always
   write here first.
2. **Live in-memory** — `ReplicaRegistry` (in proxy), `SessionStore`
   (in proxy, in-memory by default). Reflects the *running* state of
   containers and sessions.
3. **Docker** — actual containers and their state. Source of truth for
   "is this thing alive". The proxy queries Docker on startup to
   rebuild the registry.

The YAML file is **NOT** a source of truth in production — it's an
import/export format. Ruscker can be configured to auto-export to YAML
for git versioning, but the running config lives in SQLite.

### State transitions

- **First boot, no DB**: Bootstrap from `application.yml` if present;
  otherwise create empty DB.
- **Subsequent boots**: Load from DB. The YAML is optional.
- **YAML changes detected** (via inotify/polling): Show diff in admin,
  let operator apply.

## Concurrency model

- One tokio runtime, multi-threaded by default.
- The proxy accepts connections on one task per connection, handlers
  use `tower` middleware stack.
- Container lifecycle (`ContainerBackend::spawn`, `stop`) runs in a
  dedicated task; admin/proxy request it via a channel and await the
  result.
- The auto-scaler runs as a periodic task (every 10s).
- The session-purger runs as a periodic task (every 60s).
- `DashMap` for in-memory state (lock-free reads, sharded writes).

## Security boundary

### Trust levels

- **Untrusted**: visitors. They can hit `/app/*` and `/api/*` only.
  Admin paths require an authenticated session.
- **Privileged**: admin users. `/admin/*` is gated by per-user
  password login with three roles — **Viewer** (read-only dashboard),
  **Editor** (apps + media), **Admin** (everything, incl. user
  management) — enforced server-side. A break-glass `RUSCKER_ADMIN_TOKEN`
  bootstraps the first account. See `docs/SECURITY.md` §2.
- **Operator**: filesystem access (the person running Ruscker). Can
  edit YAML, restart the process.

### Secrets at rest

- Docker registry passwords: stored encrypted in
  `credentials.password_enc` via AES-GCM with a master key from
  `RUSCKER_MASTER_KEY` env var.
- Session cookie signing: HMAC-SHA256 with key from
  `RUSCKER_COOKIE_KEY` env var (auto-generated on first run if
  missing).
- TLS: rustls with cert paths in config. Optional but recommended.

## Deployment shapes

![Two deployment shapes. Single-node (default): a reverse proxy in front of one Ruscker driving the local Docker daemon and its app containers. Multi-node HA (active-active): an L4 load balancer fans to two Ruscker instances sharing config and session state in Postgres, with one scaler leader at a time.](images/deployment.svg)

### Single-node (default)

A reverse proxy terminates TLS in front of a single Ruscker, which
talks to the local Docker daemon over its socket. This is what 99% of
installs run — simple, fast, easy to operate.

### Multi-node HA (active-active, since Phase 7)

Two or more Ruscker instances behind an L4 load balancer share a Postgres
config catalog and session store, so either can serve any session.
Exactly one instance holds the scaler leadership at a time via a Postgres
advisory lock; standbys serve traffic and reconcile counts but skip the
spawn/reap loop. The sticky cookie is an HMAC over a shared key, so any
instance can validate any other's cookie. The `ContainerBackend` /
`SessionStore` traits leave room for a multi-host or Kubernetes backend
without touching proxy code. See the deployment guide's
"Running active-active" section for the runnable example.

## What's not covered here

- The admin UI internals — see the `ruscker-admin` crate
  (`cargo doc --open`).
- The proxy's WebSocket handling — see the `ruscker-proxy` crate.
- Specific algorithm choices — see `docs/adr/`.
- The YAML schema — see `docs/YAML_SCHEMA.md`.
