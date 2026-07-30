# Quickstart — a portal full of demos

From nothing to a portal of **live demos** in a couple of minutes. You
need **Docker** running locally and the `ruscker` binary (see
[Installation](./installation.md) — or just `docker run` the image,
shown below).

## 1. Run it (the portal seeds itself)

Ruscker reads a config file, but it can be almost empty — the **database
seeds the demos**. Save this two-line `ruscker.yml`:

```yaml
proxy:
  title: My Ruscker
```

Then start it with an admin token, a master key and `--db` (the admin
database):

```sh
export RUSCKER_ADMIN_TOKEN="$(openssl rand -hex 32)"
export RUSCKER_MASTER_KEY="$(openssl rand -hex 32)"
ruscker serve --bind 127.0.0.1:8080 --db ruscker.db
```

The master key encrypts saved registry credentials and TOTP secrets. Keep
it stable if you reuse this database; without it, 2FA enrolment fails with
`503`.

If you plan to use **Schedules**, choose the clock in which cron expressions
are written by adding an IANA timezone to the service config:

```yaml
server:
  timezone: America/Recife
```

Leave it out to retain the UTC default. This setting controls the scheduler
and its labelled next/last-run times; Activity, audit, Apps, Credentials,
Users and process Logs instead follow the timezone of each viewer's browser.

- Ruscker **auto-connects to Docker** when the daemon socket is
  reachable, so app containers spawn out of the box. Pass `--no-docker`
  to run landing-only (then `/app/*` returns 503); pass `--docker` to
  make a failed connect a fatal error instead of falling back to
  landing-only (useful for a remote daemon).
- **On first boot with `--db`, Ruscker seeds 13 showcase cards** — one
  live demo per supported framework (Shiny, Streamlit, Dash, Voilà,
  Jupyter, RStudio, …) plus external links for the rest — and seeds the
  framework logos into the Media library. The seed is idempotent; cards
  you delete stay deleted on subsequent boots.

Prefer the container image? Mount the Docker socket and a volume for the
DB (the image is cosign-signed; `:latest` tracks the current release):

```sh
docker run --rm -p 8080:8080 \
  -e RUSCKER_ADMIN_TOKEN \
  -e RUSCKER_MASTER_KEY \
  -v "$PWD/ruscker.yml:/etc/ruscker/ruscker.yml:ro" \
  -v "$PWD/ruscker.db:/data/ruscker.db" \
  -v /var/run/docker.sock:/var/run/docker.sock \
  ghcr.io/strategicprojects/ruscker:latest \
  serve --config /etc/ruscker/ruscker.yml --bind 0.0.0.0:8080 \
        --docker --db /data/ruscker.db
```

## 2. Open it

| URL | What you get |
|---|---|
| <http://127.0.0.1:8080/> | the portal — the seeded showcase cards |
| <http://127.0.0.1:8080/app/shiny/> | a live demo — Ruscker spawns the container on first hit |
| <http://127.0.0.1:8080/admin> | the admin panel (with `RUSCKER_ADMIN_TOKEN` set) |
| <http://127.0.0.1:8080/healthz> | liveness (always `200`) |

Click any live-demo card to see on-demand container spawn in action,
then watch the admin [Containers page](./admin.md) to see the replica start,
serve, and stop. The first request to an app spawns its container; it's
reaped automatically once idle.

## 3. Add your own app

Two ways, neither of which needs a restart for the admin route:

- **From the admin panel** (recommended) — go to `/admin` → **Apps** →
  **Add app**, pick a type, fill the form (there's a live card preview),
  and **Save**. Everything is editable here — image, ports, scaling,
  resource limits, access — without touching YAML.
- **In YAML** — add a spec to your config. A tiny stateless example
  (`traefik/whoami` is a public echo image, so there's nothing to
  build):

  ```yaml
  proxy:
    title: My Ruscker
    specs:
      - id: hello
        display-name: Hello
        description: A stateless echo server.
        container-image: traefik/whoami:latest
        port: 80
  ```

  Validate before (re)starting — it catches typos and unsupported
  features:

  ```sh
  ruscker validate ruscker.yml
  # add --strict-compat to flag any ShinyProxy feature Ruscker would ignore
  ```

The schema is ShinyProxy-compatible, so an existing `application.yml`
works here too — see [Migrating from ShinyProxy](./migrating.md).

## What just happened

Ruscker rendered the landing page, seeded the showcase catalogue into the
database, and on the first request to an app asked Docker to start its
container, routed you to it, and will reap it when idle. For interactive
apps — Shiny, Streamlit, Dash, Voilà, Jupyter, RStudio — Ruscker adds
sticky sessions and WebSocket forwarding automatically. For stateless
APIs (Plumber2, FastAPI) it load-balances across replicas with no sticky
overhead.

See [What Ruscker can serve](./use-cases.md) for the full framework
list and [Configuration](./configuration.md) for every spec field
(replica pools, CPU/memory limits, registry credentials, routing, rate
limits…).

## Next steps

- [Configuration](./configuration.md) — the full YAML reference. See
  [Per-user access](./configuration.md#per-user-access) to restrict
  apps by user / group.
- [The admin panel](./admin.md) — manage specs, images, users, and the
  live container dashboard, identity headers, per-app 2FA and schedules
  without editing YAML. Editors are delegated by group: they see open apps
  plus apps and non-Admin accounts in groups they belong to; Admin and the
  break-glass token remain global.
- [Deploying in production](./deploying.md) — systemd + nginx, TLS,
  multi-host, and active-active HA (including
  [mounting the portal under a subpath][base-path] and the
  [sticky-upstream requirement for sign-in sessions][ha-sticky] in
  HA).
- [Troubleshooting](./troubleshooting.md) — when an app won't load.

[base-path]: ./deploying.md#4b-mounting-under-a-base-path-subpath
[ha-sticky]: ./deploying.md#shared-admin-sessions-eliminate-the-sticky-upstream-caveat
