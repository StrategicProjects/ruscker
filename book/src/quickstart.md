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
ruscker serve --config application.yml --bind 127.0.0.1:8080 --docker
```

`--docker` enables the backend that spawns app containers; without it the
landing page and admin still work but `/app/*` returns 503. To unlock the
admin panel, also pass an admin token and a DB:

```sh
RUSCKER_ADMIN_TOKEN=$(openssl rand -hex 32) \
ruscker serve --config application.yml --bind 127.0.0.1:8080 \
  --docker --db ruscker.db
```

Prefer the container image? Mount your config and the Docker socket:

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
reaped automatically once idle. Refresh the [admin dashboard](./admin.md)
to watch the replica start, serve, and stop.

## What just happened

Ruscker rendered the landing page from your config, and on the first
request to the app it asked Docker to start `traefik/whoami`, routed you
to it, and will reap it when idle. For a real interactive app (Shiny,
Streamlit, Dash, Voilà) the model is the same — Ruscker adds sticky
sessions and WebSocket forwarding automatically. See
[What Ruscker can serve](./use-cases.md) for the framework list and
[Configuration](./configuration.md) for every spec field (replica pools,
CPU/memory limits, registry credentials, routing, rate limits…).

## Next steps

- [Configuration](./configuration.md) — the full YAML reference.
- [The admin panel](./admin.md) — manage specs, images, users, and the
  live dashboard without editing YAML.
- [Deploying in production](./deploying.md) — systemd + nginx, TLS,
  multi-host, and active-active HA.
- [Troubleshooting](./troubleshooting.md) — when an app won't load.
