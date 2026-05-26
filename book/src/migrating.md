# Migrating from ShinyProxy

Ruscker reads the **same `application.yml` schema** as ShinyProxy, so
in most cases you point it at your existing config and it just works.

## 1. Pre-flight check

Before switching anything, ask Ruscker what your config uses:

```sh
ruscker validate application.yml                 # general report
ruscker validate --strict-compat application.yml # migration pre-flight
```

`--strict-compat` lists every ShinyProxy feature your config uses that
Ruscker does **not** honour (e.g. Kubernetes backend, per-spec
`volumes`/`environment`, non-`none` authentication) and exits non-zero
if it finds any. A clean run means a drop-in migration.

> In production, a real 31-spec ShinyProxy 3.2.0 config reported
> **"no unsupported features"**.

The validator also flags **plaintext credentials** in the YAML — move
any `docker-registry-password` to `${DOCKER_REGISTRY_PASSWORD}` and set
the variable in the environment (or `/etc/ruscker/ruscker.env`).

## 2. Card logos

ShinyProxy serves card logos from its `template-path`'s `assets/img/`
folder. When you run `serve` without `--images-dir`, Ruscker
auto-discovers them next to the config:

1. `<config-dir>/assets/img/`
2. `<config-dir>/<template-path>/assets/img/`

So a config left in place finds its logos with no extra flags.

## 3. Side-by-side cutover (recommended)

You don't have to flip everything at once. A safe pattern (proven in
production) keeps ShinyProxy reachable while Ruscker takes the root:

- Run Ruscker on a spare port (e.g. `127.0.0.1:8090`).
- In nginx, route `/` → Ruscker and `/sp/` → ShinyProxy (give
  ShinyProxy a `server.servlet.context-path: /sp`).
- Compare the two live, and roll back by restoring the nginx config if
  needed.

Because Ruscker uses the **same `/app/{spec}` URL scheme**, existing
bookmarks keep working after the cutover.

## What Ruscker adds

Beyond parity, you also get: a real admin panel (no more hand-editing
YAML), a monitoring dashboard, per-API rate-limiting/CORS, health
probes, graceful shutdown, and a tiny footprint. See
[The admin panel](./admin.md).

## Not supported (yet)

Authentication schemes other than `none`, the Kubernetes backend, and
a few per-spec fields (`volumes`, `environment`, `labels`, …) are
parsed but ignored — `validate --strict-compat` is the source of
truth. For apps that handle their own auth (a common case), `none` is
correct: Ruscker just routes traffic.
