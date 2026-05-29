# FAQ

### Is Ruscker really compatible with my ShinyProxy `application.yml`?

Yes — Ruscker reads the ShinyProxy YAML schema. Point it at your existing
file and run `ruscker validate application.yml --strict-compat` to get a
report of any ShinyProxy feature Ruscker doesn't support yet (it refuses
to silently ignore them). See [Migrating from ShinyProxy](./migrating.md).

### Do I need Java or the JVM?

No. Ruscker is a single static binary written in Rust. There's no JVM, no
Java toolchain, and no application server to manage — the idle footprint
is **~16 MB** instead of the hundreds of MB a JVM-based proxy idles at.

### Which app frameworks can it host?

Anything that runs in a container and speaks HTTP or WebSocket. R/Shiny is
the reference case, but Streamlit, Dash, Voilà, Gradio, Panel, Bokeh,
Plumber, FastAPI, Flask, and plain web services all work. The full list
is in [What Ruscker can serve](./use-cases.md).

### Does it isolate sessions like ShinyProxy (and unlike Shiny Server Free)?

Yes. Stateful apps get **one container per session** with sticky routing
and WebSocket forwarding by default. Stateless APIs are load-balanced
across replicas with no sticky cookie.

### Do I need Kubernetes?

No. Ruscker drives the **Docker** daemon. A single Docker host is the
common case; for more capacity it can schedule across several Docker
daemons over `ssh://` or `tcp://` (see
[multi-host scheduling](./deploying.md)). There's no Kubernetes
requirement — if you're all-in on k8s, see the
[alternatives](./alternatives.md).

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
interpolation; the credentials store is AES-encrypted at rest.

### Can I run more than one instance for high availability?

Yes. Several instances share one Postgres for the catalog and session
state, behind a load balancer; a Postgres advisory lock elects a single
auto-scaler leader with automatic failover. There's a runnable two-node
harness in `examples/ha/` — see the
[active-active section](./deploying.md) of the deploy guide.
One operational caveat: until a shared admin-session store ships, pin
the **sign-in session** paths to a single upstream — see
[Sticky upstream for the sign-in session][ha-sticky].

[ha-sticky]: ./deploying.md#sticky-upstream-for-the-sign-in-session-admin-app-api

### Is it production-ready?

Ruscker is on **v0.1.3** and runs in production. Releases are multi-arch
and cosign-signed; see the [release notes](./news.md) for what changed
in each version. The [Roadmap](./roadmap.md) tracks what's shipped
(Phases 0–7) and what's planned (Phase 8: external auth).

### What platforms does it run on?

Ruscker runs on a Linux Docker host — install via the multi-arch Docker
image, a Debian package with a `systemd` unit, or a static musl tarball.
A Homebrew tap builds it on macOS for local development. The apps
themselves are Linux containers.

### An app won't load behind Ruscker — what now?

Most issues are URL-rewriting, cookie-key, or backend-readiness related.
Start with [Troubleshooting](./troubleshooting.md).
