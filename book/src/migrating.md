# Migrating from ShinyProxy

Ruscker reads the **same `application.yml` schema** as ShinyProxy, so
in most cases you point it at your existing config and it just works.

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

## 3. Sub-path mounting (`context-path`)

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

## 4. Card logos

ShinyProxy serves card logos from its `template-path`'s `assets/img/`
folder. When you run `serve` without `--images-dir`, Ruscker
auto-discovers them next to the config:

1. `<config-dir>/assets/img/`
2. `<config-dir>/<template-path>/assets/img/`

So a config left in place finds its logos with no extra flags.

You can also upload images through the admin **Media** panel and
reference them by filename in the spec's `logo` field.

## 5. Side-by-side cutover (recommended)

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

Since v0.2.5 `ruscker serve` runs the same validation as
`ruscker validate` at boot and logs every finding. A migrated config
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

## What Ruscker adds

Beyond parity, you also get: a real admin panel (no more hand-editing
YAML), a live monitoring dashboard, per-spec `container-env` /
`container-cmd` injection, per-API rate-limiting and CORS, per-user
and per-group app visibility, health probes, graceful shutdown, and a
tiny footprint — **~14 MB** idle, against the ~540 MB of the JVM-based
proxy it replaced on the same machine. See
[The admin panel](./admin.md).

## Not supported (yet)

Authentication schemes other than `none` and the Kubernetes backend
are the main gaps — `validate --strict-compat` is the authoritative
source of truth for your specific config. For apps that handle their
own auth (a common case), `none` is correct: Ruscker just routes
traffic.

The following ShinyProxy per-spec fields are accepted by the parser
but currently ignored:

- `minimum-seats-available` — use `min-replicas` instead
- `labels` — custom container labels (phase 3.5)
- `network-connections` — container network wiring (phase 3.5)
- `kubernetes-*` — Kubernetes backend

Fields that **are** now fully supported (and no longer flagged by
`--strict-compat`): `volumes`, `container-env`, `container-cmd`,
`port` (maps to `container-port`), and the scaling and lifecycle knobs
`scale-up-threshold`, `scale-down-threshold`, `scale-down-grace`,
`max-lifetime`, `container-lifetime`, `drain-timeout`,
`stop-on-logout`, and `concurrent-requests-per-replica`.
