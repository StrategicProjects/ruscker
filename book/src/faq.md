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
ones after a grace period. `min` defaults to **0** — apps cold-start on
the first request and are reaped when idle (set `min-replicas: 1` to keep
one warm). Routing is least-connections (interactive) or round-robin
(APIs). It's all in [Configuration](./configuration.md).

### How does authentication work?

The **admin panel** has user accounts with Viewer / Editor / Admin
roles (plus a break-glass token). An Editor's own group memberships are
also their administrative boundary: they can manage open apps, apps whose
`access-groups` overlap their groups, and non-Admin Viewer/Editor accounts
and memberships in those groups. They can create those accounts and reset
their passwords; account deletion, CSV import, MFA reset, and group
create/rename/delete remain Admin-only. The Media library is shared.
Out-of-scope app and user ids return `404` so guessed identifiers do not
reveal another team's resources. Admin and break-glass sessions remain
unrestricted.

The same accounts gate **per-app visibility**: every spec can declare
`access-groups` / `access-users` and only matching users see the card and
reach `/app` / `/api` (specs with no access keys remain open to anyone) —
see [Per-user access](./configuration.md#per-user-access).
An app can also set `require-mfa: true` for a TOTP step-up before its
container starts; each user enrols one factor and each app chooses how
recent the proof must be.
External identity providers (OIDC / SAML / LDAP) for end-user
sign-in are [Phase 8](./roadmap.md); user accounts are managed in
the admin **Users** page until then.

### How does my app receive the signed-in user?

Identity forwarding is opt-in per spec. Set
`add-default-http-headers: true` for `X-SP-UserId` and
`X-SP-UserGroups`. Independently, select `identity-claims: [email, setor]`
for `X-Ruscker-User-Email` / `X-Ruscker-User-Setor`. Ruscker sends values
only for signed-in users, on HTTP and WebSocket, and strips client-supplied
reserved identity headers first. It defaults off; if the app appears to
have “lost” the user after migrating from ShinyProxy, enable it explicitly.

### Can proxied apps set their own cookies?

Yes. App-owned cookies pass through. Because apps share the portal origin,
Ruscker drops response cookies that use its reserved session, preference,
sticky or MFA names (including `ruscker_admin_session`, `ruscker_theme`,
`ruscker_locale`, `__ruscker_session*` and `__ruscker_mfa_*`). It also
neutralises `Clear-Site-Data` cookie-clearing directives. Rename a
conflicting app cookie; this boundary prevents an app from overwriting the
portal's login or MFA state.

### Where is configuration and state stored?

By default in a local **SQLite** database (`--db`), with YAML as the
import/export format. For multi-instance HA the same admin catalog and the
session store live in shared **Postgres** (`--config-db-url` /
`--session-store-url`). Secrets never go in YAML — use `${ENV_VAR}`
interpolation. The named credentials store is AES-encrypted at rest; it
also accepts a pure `${VAR}` env-ref as the password (stored verbatim,
resolved at pull time, so the decryption key is never needed for env-based
credentials).

### Which timezone does Ruscker use?

Stored instants remain UTC. Historical timestamps in Activity, audit, Apps,
Credentials, Users and process Logs render in the viewer's browser timezone;
sortable table dates compare the underlying instant, not the formatted text.
Hover a table timestamp for its full localized date and zone.

Cron has no viewer, so the scheduler uses the IANA zone in
`server.timezone`. The Schedules page labels next/last runs in that zone.
Leaving it unset preserves the historical UTC behavior, so an upgrade does
not move existing jobs. An invalid name warns and also falls back to UTC;
the effective zone is included in the startup banner.

### Can I run more than one instance for high availability?

Yes. Several instances share one Postgres for the catalog and session
state, behind a load balancer; a Postgres advisory lock elects a single
auto-scaler leader with automatic failover. There's a runnable two-node
harness in `examples/ha/` — see the
[active-active section](./deploying.md) of the deploy guide.
For the **sign-in session**, point every instance at a shared
admin-session store (`--admin-session-store-url postgres://…`); a
sticky upstream for the login paths remains the fallback — see
[Sticky upstream for the sign-in session][ha-sticky].

[ha-sticky]: ./deploying.md#fallback-sticky-upstream

### Is it production-ready?

The current release includes health probes, graceful shutdown, structured
logging, signed multi-arch artifacts and active-active operation with
Postgres. Review the [release notes](./news.md) and
[Roadmap](./roadmap.md), then stage your own workload and failure tests.
External identity providers (OIDC / SAML / LDAP) are not yet available;
use the built-in accounts when that limitation fits your deployment.

### What platforms does it run on?

Ruscker runs on a Linux Docker host — install via the multi-arch Docker
image, a Debian package with a `systemd` unit, or a static musl tarball.
A Homebrew tap builds it on macOS for local development. The apps
themselves are Linux containers.

### An app won't load behind Ruscker — what now?

Most issues are URL-rewriting, cookie-key, or backend-readiness related.
Start with [Troubleshooting](./troubleshooting.md).
