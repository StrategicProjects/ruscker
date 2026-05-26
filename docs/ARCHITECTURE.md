# Architecture

Ruscker is a Rust-based proxy and orchestrator for containerized
interactive web apps and stateless HTTP APIs. This document describes
how the pieces fit together.

## High-level diagram

```
                    ┌────────────────────┐
                    │  Visitors (browsers)│
                    └──────────┬──────────┘
                               │ HTTPS
                               ▼
                    ┌─────────────────────┐
                    │   Ruscker process   │
                    │  ┌───────────────┐  │
                    │  │ Landing page  │  │ ◄── Askama templates
                    │  │ (Tailwind 4)  │  │
                    │  └───────────────┘  │
                    │  ┌───────────────┐  │
                    │  │ HTTP+WS proxy │  │ ◄── ruscker-proxy
                    │  │  - sticky     │  │
                    │  │  - rewrite    │  │
                    │  │  - WS upgrade │  │
                    │  └───────────────┘  │
                    │  ┌───────────────┐  │
                    │  │  Admin panel  │  │ ◄── ruscker-admin
                    │  │ (HTMX+Tailw.) │  │
                    │  └───────────────┘  │
                    │  ┌───────────────┐  │
                    │  │  Router /     │  │ ◄── ruscker-core
                    │  │  Auto-scaler  │  │
                    │  └───────┬───────┘  │
                    └──────────┼──────────┘
                               │ Docker API
                               ▼
                    ┌─────────────────────┐
                    │   Docker daemon     │
                    │  ┌─────┐ ┌─────┐    │
                    │  │ App │ │ App │    │
                    │  │  1  │ │  2  │ …  │
                    │  └─────┘ └─────┘    │
                    └─────────────────────┘
```

All of this is a single Rust process (single binary, ~20MB).

## Crate map

```
ruscker-config  (schema, parsing, validation)
       ▲
       │
ruscker-core  (traits, types, routing algorithms)
       ▲
       ├──── ruscker-docker  (ContainerBackend impl)
       │
       ├──── ruscker-proxy   (HTTP+WS reverse proxy)
       │
       └──── ruscker-admin   (Web UI for management)
                  ▲
                  │
            ruscker-cli   (binary entry point)
```

`ruscker-config` and `ruscker-core` are pure-domain crates. They do no
I/O and contain no async code (except for trait definitions in core
which need to be async-capable for impls).

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
3. Router.pick() with RoundRobin → R3
4. Forward request, stream response
5. No state, no follow-up — done.
```

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
  Admin paths require auth (when implemented).
- **Privileged**: admin users. Can mutate specs, view logs, restart
  containers.
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

### Single-node (default, MVP)

```
[load balancer]
      │
      ▼
[Ruscker (proxy + admin + Docker control)]
      │
      ▼
[Docker daemon, local socket]
      │
      ▼
[App containers]
```

This is what 99% of users will deploy. Simple, fast, easy to operate.

### Multi-node HA (future)

```
[L4 load balancer]
      │
      ├──► [Ruscker 1] ──┐
      │                  ├──► [Postgres]  ◄── shared state
      └──► [Ruscker 2] ──┘
                         │
                         ▼
                  [Docker Swarm / K8s]
```

Two Ruscker instances behind a real LB share session state via
Postgres. Either can serve any session.

## What's not covered here

- The admin UI internals — see `crates/ruscker-admin/CLAUDE.md`.
- The proxy's WebSocket handling — see
  `crates/ruscker-proxy/CLAUDE.md`.
- Specific algorithm choices — see `docs/adr/`.
- The YAML schema — see `docs/YAML_SCHEMA.md`.
