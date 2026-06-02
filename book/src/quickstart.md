# Quickstart — your first app

From nothing to a running app behind Ruscker in a few minutes. You need
**Docker** running locally and the `ruscker` binary (see
[Installation](./installation.md) — or just `docker run` the image,
shown below).

## 1. Write a config

One spec is enough. Save this as `application.yml` — it serves
`traefik/whoami`, a tiny stateless echo image, so there's nothing to
build:

```yaml
proxy:
  title: My Ruscker
  bind-address: 0.0.0.0
  port: 8080
  specs:
    - id: hello
      display-name: Hello
      description: A stateless echo server.
      container-image: traefik/whoami:latest
      port: 80
```

The schema is ShinyProxy-compatible, so an existing `application.yml`
works here too — see [Migrating from ShinyProxy](./migrating.md).

## 2. Validate it

Catch typos and unsupported features before starting:

```sh
ruscker validate application.yml
# add --strict-compat to flag any ShinyProxy feature Ruscker would ignore
```

## 3. Run it

```sh
ruscker serve --config application.yml --bind 127.0.0.1:8080
```

Ruscker **auto-connects to Docker** when the daemon socket is reachable, so
app containers spawn out of the box. Pass `--no-docker` to run landing-only
(then `/app/*` returns 503); pass `--docker` to make a failed connect a fatal
error instead of falling back to landing-only (useful for a remote daemon). To
unlock the admin panel, also pass an admin token and a DB:

```sh
RUSCKER_ADMIN_TOKEN=$(openssl rand -hex 32) \
ruscker serve --config application.yml --bind 127.0.0.1:8080 \
  --docker --db ruscker.db
```

On first boot with `--db`, Ruscker seeds 13 showcase cards into the
portal automatically — one live demo per supported framework (Shiny,
Streamlit, Dash, Voilà, Jupyter, RStudio, …) plus external links for
the rest. The seed is idempotent; cards you delete stay deleted on
subsequent boots. Framework logos are also seeded into the Media
library so they appear in the image picker alongside your own uploads.

Prefer the container image? Mount your config and the Docker socket
(the image is cosign-signed; `:latest` tracks the current release):

```sh
docker run --rm -p 8080:8080 \
  -v "$PWD/application.yml:/etc/ruscker/application.yml:ro" \
  -v /var/run/docker.sock:/var/run/docker.sock \
  ghcr.io/strategicprojects/ruscker:latest \
  serve --config /etc/ruscker/application.yml --bind 0.0.0.0:8080 --docker
```

## 4. Open it

| URL | What you get |
|---|---|
| <http://127.0.0.1:8080/> | the landing page — one card per spec |
| <http://127.0.0.1:8080/app/hello/> | the app — Ruscker spawns the container on first hit |
| <http://127.0.0.1:8080/admin> | the admin panel (with `RUSCKER_ADMIN_TOKEN` set) |
| <http://127.0.0.1:8080/healthz> | liveness (always `200`) |

The first request to `/app/hello/` spawns a container on demand; it's
reaped automatically once idle.

## What just happened

Ruscker rendered the landing page from your config, and on the first
request to the app it asked Docker to start `traefik/whoami`, routed you
to it, and will reap it when idle. For a real interactive app — Shiny,
Streamlit, Dash, Voilà, Jupyter, RStudio — the model is the same:
Ruscker adds sticky sessions and WebSocket forwarding automatically.
For stateless APIs (Plumber2, FastAPI) it load-balances across replicas
with no sticky overhead.

If you started with `--db`, the landing page already shows the seeded
showcase cards. Click any live-demo card to see on-demand container
spawn in action, then watch the [admin dashboard](./admin.md) to see
the replica start, serve, and stop.

![The seeded landing page on first boot: a Featured carousel of highlighted apps above a filterable grid of showcase cards (Shiny, Streamlit, Dash, Jupyter, RStudio, …).](images/landing.png)

See [What Ruscker can serve](./use-cases.md) for the full framework
list and [Configuration](./configuration.md) for every spec field
(replica pools, CPU/memory limits, registry credentials, routing, rate
limits…).

## Next steps

- [Configuration](./configuration.md) — the full YAML reference. See
  [Per-user access](./configuration.md#per-user-access) to restrict
  apps by user / group.
- [The admin panel](./admin.md) — manage specs, images, users, and the
  live dashboard without editing YAML.
- [Deploying in production](./deploying.md) — systemd + nginx, TLS,
  multi-host, and active-active HA (including
  [mounting the portal under a subpath][base-path] and the
  [sticky-upstream requirement for sign-in sessions][ha-sticky] in
  HA).
- [Troubleshooting](./troubleshooting.md) — when an app won't load.

[base-path]: ./deploying.md#4b-mounting-under-a-base-path-subpath
[ha-sticky]: ./deploying.md#shared-admin-sessions-eliminate-the-sticky-upstream-caveat
