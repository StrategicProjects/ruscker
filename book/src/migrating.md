# Migrating from ShinyProxy

Ruscker reads the **same `application.yml` schema** as ShinyProxy, so
in most cases you point it at your existing config and it just works.
(Your file keeps working as `--config` too — Ruscker's canonical
service config is `ruscker.yml`, same schema; see
[Configuration](./configuration.md) for how the pieces fit.)

## 1. Pre-flight check

Before switching anything, ask Ruscker what your config uses:

```sh
ruscker validate application.yml                 # general report
ruscker validate application.yml --strict-compat # migration pre-flight
```

`--strict-compat` lists every ShinyProxy feature your config uses that
Ruscker does **not** honour (e.g. Kubernetes backend, `minimum-seats-available`,
non-`none` authentication) and exits non-zero if it finds any. A clean
run means a drop-in migration.

> In production, a real 31-spec ShinyProxy 3.2.0 config reported
> **"no unsupported features"**.

The validator also flags **plaintext credentials** in the YAML — move
any `docker-registry-password` to `${DOCKER_REGISTRY_PASSWORD}` and set
the variable in the environment (or `/etc/ruscker/ruscker.env`).

## 2. Credentials and env-var interpolation

Any string value in `application.yml` can reference an environment
variable with `${VAR}` or `${VAR:-default}`:

```yaml
docker-registry-password: ${DOCKER_REGISTRY_PASSWORD}
```

The literal `${VAR}` token is what gets stored (in the config file, the
database, and exports) — it is resolved to the real value only when a
container is actually spawned. This means registry passwords and
per-spec `container-env` secrets **never land in the database**.

For teams managing several apps that share a registry credential,
Ruscker also has a **named credential store** in the admin panel.
Store the credential there once, then reference it by name in the spec:

```yaml
- id: my_app
  container-image: registry.example.com/team/app:latest
  docker-registry-credential: my-registry-cred   # name from the store
```

When `docker-registry-credential` is set, it takes precedence over the
inline `docker-registry-username` / `docker-registry-password` fields.
The credential store accepts either an encrypted password or a pure
`${VAR}` env-ref (resolved at pull time, not stored in cleartext).

> The inline fields are still valid and kept for back-compat — use
> whichever fits your workflow.

## 3. Identity headers

ShinyProxy's per-spec `add-default-http-headers: true` is supported. It
forwards the signed-in account as `X-SP-UserId` and its comma-separated
groups as `X-SP-UserGroups` over HTTP and WebSocket.

Check specs whose applications read those headers: ShinyProxy defaults the
setting on, while Ruscker deliberately defaults it off. If the field was
omitted from the old config, add it explicitly where the application needs
the identity:

```yaml
- id: internal-app
  container-image: example/internal-app:latest
  add-default-http-headers: true
  identity-claims: [email, setor] # optional Ruscker profile headers
```

The optional claims become `X-Ruscker-User-Email` and
`X-Ruscker-User-Setor`; missing values are omitted. Ruscker strips
client-supplied `X-SP-*` and `X-Ruscker-User-*` headers before adding its
own, but the app should still be reachable only through the proxy if it
trusts them.

## 4. Sub-path mounting (`context-path`)

If you run Ruscker on a path prefix rather than a dedicated subdomain
(e.g. `example.org/apps/` instead of `apps.example.org`), use
`server.context-path`:

```yaml
server:
  context-path: /apps    # normalized: leading slash, no trailing slash
```

ShinyProxy's nested form is also accepted without changes:

```yaml
server:
  servlet.context-path: /apps
```

Or override it at startup with the CLI flag (wins over YAML):

```sh
ruscker serve --base-path /apps --config application.yml ...
```

The portal and admin routes are all mounted under the prefix; the
health probes (`/healthz`, `/readyz`) stay at the root so your load
balancer does not need to know the prefix. Your reverse proxy just
needs to forward requests under the same path through to Ruscker.

## 5. Card logos

ShinyProxy serves card logos from its `template-path`'s `assets/img/`
folder. When you run `serve` without `--images-dir`, Ruscker
auto-discovers them next to the config:

1. `<config-dir>/assets/img/`
2. `<config-dir>/<template-path>/assets/img/`

So a config left in place finds its logos with no extra flags.

You can also upload images through the admin **Media** panel and
reference them by filename in the spec's `logo` field.

## 6. Side-by-side cutover (recommended)

You don't have to flip everything at once. A safe pattern (proven in
production) keeps ShinyProxy reachable while Ruscker takes the root:

- Run Ruscker on a spare port (e.g. `127.0.0.1:8090`).
- In nginx, route `/` → Ruscker and `/sp/` → ShinyProxy (give
  ShinyProxy a `server.servlet.context-path: /sp`).
- Compare the two live, and roll back by restoring the nginx config if
  needed.

Because Ruscker uses the **same `/app/{spec}` URL scheme**, existing
bookmarks keep working after the cutover.

## After the cutover: read the startup warnings

`ruscker serve` runs the same validation as `ruscker validate` at boot and
logs every finding. A migrated config
typically produces a few `is set but has no effect` warnings —
ShinyProxy fields Ruscker parses but doesn't honour
(`server.secure-cookies`, `proxy.heartbeat-rate`, `hide-navbar`, …).
They're harmless, but each one is configured intent that is *not*
happening, so review them once: the
[validation-warnings reference](configuration.md#validation-warnings)
says what to do for each. Two to know about:

- `server.secure-cookies` does nothing — the `Secure` flag comes from
  `server.useForwardHeaders` + your proxy's `X-Forwarded-Proto`
  (see [Deploying](deploying.md)).
- `type: streamlit | dash | voila` specs without a `container-port`
  now default to the framework's well-known port (8501 / 8050 / 8866)
  instead of Shiny's 3838 — apps that previously needed an explicit
  port "just work"; an explicit `container-port`/`port:` still wins.

### Preserve or choose the scheduler clock

An existing schedule keeps its historical behavior after migration:
without `server.timezone`, Ruscker evaluates cron in UTC. To write cron in
a local clock, opt in with an IANA name:

```yaml
server:
  timezone: America/Recife
```

The Schedules page then labels next and last runs in that zone. Invalid
names produce a validation warning and fall back to UTC instead of blocking
startup; the startup banner reports the effective zone. This does not
control Activity, audit, Apps, Credentials, Users or process Logs, whose
stored UTC timestamps render in each viewer's browser timezone.

## What Ruscker adds

Beyond parity, you also get: a real admin panel (no more hand-editing
YAML), a live monitoring dashboard, per-spec `container-env` /
`container-cmd` injection, per-API rate-limiting and CORS, per-user
and per-group app visibility, group-scoped Editor delegation, viewer-local
admin timestamps, per-app step-up MFA, timezone-aware scheduled jobs and
named volume management (local Docker backend), health probes, graceful
shutdown, and **~14 MB idle**.
The JVM-based proxy it replaced on the same machine used about 540 MB. See
[The admin panel](./admin.md).

## Not supported (yet)

Authentication schemes other than `none` and the Kubernetes backend
are the main gaps — `validate --strict-compat` is the authoritative
source of truth for your specific config. For apps that handle their
own auth (a common case), `none` is correct: Ruscker just routes
traffic.

For the full field-by-field picture — every ShinyProxy key with its
status in Ruscker (supported / warned-and-ignored / planned / out of
scope, and the Ruscker way to get the same outcome) — see the
**[ShinyProxy → Ruscker field map](./shinyproxy-fieldmap.md)**.

The short version: the container keys (`container-env` / `-cmd` /
`-volumes` / `-network`, `labels`, the CPU/memory requests and limits,
`port`), the access lists, and the lifecycle knobs all map straight
across; `minimum-seats-available` (use `min-replicas`),
`network-connections` (use the single `container-network`) and
`kubernetes-*` are flagged by `--strict-compat`; per-user-instance
knobs (`max-instances` family) don't apply — Ruscker pools
seats × replicas instead.
