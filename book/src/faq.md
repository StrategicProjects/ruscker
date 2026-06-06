# FAQ

### Can I import my existing `application.yml`?

Yes — Ruscker reads the familiar YAML schema. Point it at your existing
file and run `ruscker validate application.yml --strict-compat` to get a
report of any feature Ruscker doesn't support yet (it refuses to silently
ignore them). See [Migrate an existing config](./migrating.md).

### What does Ruscker need to run?

Just the binary and a Docker host. Ruscker is a single static binary
written in Rust — no language runtime, no toolchain, and no application
server to manage alongside it. The idle footprint is **~14 MB**.

### Which app frameworks can it host?

Anything that runs in a container and speaks HTTP or WebSocket. R/Shiny is
the reference case, but Streamlit, Dash, Voilà, Gradio, Panel, Bokeh,
Plumber, FastAPI, Flask, and plain web services all work. The full list
is in [What Ruscker can serve](./use-cases.md).

### Does it isolate sessions?

Yes. Stateful apps get **one container per session** with sticky routing
and WebSocket forwarding by default. Stateless APIs are load-balanced
across replicas with no sticky cookie.

### Do I need Kubernetes?

No. Ruscker drives the **Docker** daemon. A single Docker host is the
common case; for more capacity it can schedule across several Docker
daemons over `ssh://` or `tcp://` (see
[multi-host scheduling](./deploying.md)). There's no Kubernetes
requirement — if you're all-in on k8s, see
[where Ruscker fits](./alternatives.md).

### How does scaling work?

Each spec has a replica pool with `min`/`max` bounds. An auto-scaler keeps
`min` replicas warm, spawns more on sustained saturation, and reaps idle
ones after a grace period. Routing is least-connections (interactive) or
round-robin (APIs). It's all in [Configuration](./configuration.md).

### How does authentication work?

The **admin panel** has user accounts with Viewer / Editor / Admin
roles (plus a break-glass token). The same accounts gate **per-app
visibility**: every spec can declare `access-groups` / `access-users`
and only matching users see the card and reach `/app` / `/api`
(specs with no access keys remain open to anyone) —
see [Per-user access](./configuration.md#per-user-access).
External identity providers (OIDC / SAML / LDAP) for end-user
sign-in are [Phase 8](./roadmap.md); user accounts are managed in
the admin **Users** page until then.

### Where is configuration and state stored?

By default in a local **SQLite** database (`--db`), with YAML as the
import/export format. For multi-instance HA the same admin catalog and the
session store live in shared **Postgres** (`--config-db-url` /
`--session-store-url`). Secrets never go in YAML — use `${ENV_VAR}`
interpolation. The named credentials store is AES-encrypted at rest; it
also accepts a pure `${VAR}` env-ref as the password (stored verbatim,
resolved at pull time, so the decryption key is never needed for env-based
credentials).

### Can I run more than one instance for high availability?

Yes. Several instances share one Postgres for the catalog and session
state, behind a load balancer; a Postgres advisory lock elects a single
auto-scaler leader with automatic failover. There's a runnable two-node
harness in `examples/ha/` — see the
[active-active section](./deploying.md) of the deploy guide.
One operational caveat: until a shared admin-session store ships, pin
the **sign-in session** paths to a single upstream — see
[Sticky upstream for the sign-in session][ha-sticky].

[ha-sticky]: ./deploying.md#fallback-sticky-upstream

### Is it production-ready?

Yes. The current release is **v0.1.77** — Phases 0–7 are complete and
the proxy is production-ready and horizontally scalable.
Releases are multi-arch and cosign-signed; see the
[release notes](./news.md) and the [Roadmap](./roadmap.md).
Phase 8 (external auth: OIDC / SAML / LDAP) is the main remaining
optional work.

### What platforms does it run on?

Ruscker runs on a Linux Docker host — install via the multi-arch Docker
image, a Debian package with a `systemd` unit, or a static musl tarball.
A Homebrew tap builds it on macOS for local development. The apps
themselves are Linux containers.

### An app won't load behind Ruscker — what now?

Most issues are URL-rewriting, cookie-key, or backend-readiness related.
Start with [Troubleshooting](./troubleshooting.md).
