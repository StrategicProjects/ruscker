# Deploying in production

This walks through the production pattern Ruscker was designed for:
the `.deb` on a Docker host, behind nginx, optionally side-by-side
with an existing ShinyProxy.

## 1. Install and configure

```sh
sudo apt install ./ruscker_<version>_amd64.deb
```

Put your apps in `/etc/ruscker/application.yml` and your secrets in
`/etc/ruscker/ruscker.env` (read by the unit):

```ini
RUSCKER_ADMIN_TOKEN=...        # openssl rand -hex 32
RUSCKER_MASTER_KEY=...         # for the credentials store
RUSCKER_COOKIE_KEY=...         # keep sticky sessions stable across restarts
DOCKER_REGISTRY_PASSWORD=...   # referenced as ${DOCKER_REGISTRY_PASSWORD} in the YAML
```

## 2. Enable the container backend

The shipped unit serves landing + admin + proxy but not the `--docker`
backend. Enable it with a drop-in (so upgrades don't clobber it):

```sh
sudo systemctl edit ruscker
```

```ini
[Service]
SupplementaryGroups=docker
ExecStart=
ExecStart=/usr/bin/ruscker serve --config /etc/ruscker/application.yml \
  --bind 127.0.0.1:8090 --docker --db /var/lib/ruscker/ruscker.db
```

```sh
sudo systemctl daemon-reload && sudo systemctl restart ruscker
```

> Adding `ruscker` to the `docker` group is effectively root on the
> host — the same trade-off ShinyProxy carries.

### On-demand vs. pre-warmed

Ruscker's auto-scaler keeps `min-replicas` (default **1**) warm per
spec. With many specs that's a lot of idle containers. To match
ShinyProxy's on-demand behaviour (spawn on first request, reap when
idle), set `min-replicas: 0` on the specs.

## 3. nginx

Terminate TLS at your edge / load balancer and forward to nginx with
`X-Forwarded-Proto: https`. A minimal reverse proxy:

```nginx
server {
    listen 80;
    server_name portal.example.gov;

    location / {
        proxy_pass http://127.0.0.1:8090/;
        proxy_http_version 1.1;
        proxy_set_header Upgrade $http_upgrade;       # WebSocket (Shiny)
        proxy_set_header Connection "upgrade";
        proxy_read_timeout 600s;
        proxy_set_header Host              $http_host;
        proxy_set_header X-Real-IP         $remote_addr;
        proxy_set_header X-Forwarded-For   $proxy_add_x_forwarded_for;
        proxy_set_header X-Forwarded-Proto https;
    }
}
```

Set `server.useForwardHeaders: true` in the YAML so Ruscker trusts
`X-Forwarded-*` (needed for `Secure` cookies and per-client API rate
limiting).

> The live dashboard streams over Server-Sent Events
> (`/admin/dashboard/events`). If its updates lag, nginx is buffering
> the stream — disable it for that path:
>
> ```nginx
> location = /admin/dashboard/events {
>     proxy_pass http://127.0.0.1:8090;
>     proxy_buffering off;
>     proxy_read_timeout 1h;
>     # + the same proxy_set_header lines as above
> }
> ```

## 4. Side-by-side with ShinyProxy

To run both during a migration, give ShinyProxy a context path and
route by prefix:

```nginx
    # ShinyProxy under /sp/ (set server.servlet.context-path: /sp in its YAML)
    location /sp/ { proxy_pass http://127.0.0.1:8080; /* + the proxy_set_header lines */ }

    # Ruscker at the root
    location /    { proxy_pass http://127.0.0.1:8090/; /* + the proxy_set_header lines */ }
```

Cut over by reloading nginx; roll back by restoring the previous
config. Keep a backup of the site file and run `nginx -t` before every
reload.

## 5. Health checks

Point your load balancer / orchestrator at:

- `GET /healthz` — liveness, always `200` (no dependencies).
- `GET /readyz` — readiness; probes the DB (`SELECT 1`) and the Docker
  backend, returns `503` while draining or when a dependency is down.

On `SIGTERM` Ruscker flips `/readyz` to `draining`, lets in-flight
sessions wind down up to `proxy.shutdown-grace-ms`, then exits.

## Running with Docker instead of the `.deb`

If you'd rather run the container image, mount your config and the
Docker socket (the `--docker` backend talks to the host daemon) and
persist the SQLite DB on a volume:

```yaml
# docker-compose.yml
services:
  ruscker:
    image: ghcr.io/strategicprojects/ruscker:latest
    command: >
      serve --config /etc/ruscker/application.yml
      --bind 0.0.0.0:8080 --docker --db /var/lib/ruscker/ruscker.db
    ports: ["127.0.0.1:8090:8080"]
    environment:
      RUSCKER_ADMIN_TOKEN: ${RUSCKER_ADMIN_TOKEN}
      RUSCKER_MASTER_KEY:  ${RUSCKER_MASTER_KEY}
      RUSCKER_COOKIE_KEY:  ${RUSCKER_COOKIE_KEY}
    volumes:
      - ./application.yml:/etc/ruscker/application.yml:ro
      - /var/run/docker.sock:/var/run/docker.sock
      - ruscker-data:/var/lib/ruscker
    restart: unless-stopped
volumes:
  ruscker-data:
```

nginx sits in front exactly as above. Mounting the Docker socket grants
the container control of the host daemon — the same root-equivalent
trade-off as the `docker` group with the `.deb`.

## Backups

State lives in two places:

- **`/etc/ruscker/`** — `application.yml` and `ruscker.env` (your
  tokens). Back these up with the rest of `/etc`.
- **The SQLite DB** (`/var/lib/ruscker/ruscker.db`) — specs, the image
  library, encrypted credentials, landing customization and the audit
  log when you run with `--db`. Snapshot it consistently with:

  ```sh
  sqlite3 /var/lib/ruscker/ruscker.db ".backup '/backup/ruscker.db'"
  ```

  The encrypted credentials are useless without `RUSCKER_MASTER_KEY`, so
  back up the key too — separately. `ruscker export --db <file>` also
  writes a YAML snapshot (everything except the encrypted secrets).

## Upgrading

Build the new `.deb`, copy it over, and reinstall keeping your config:

```sh
sudo dpkg -i --force-confold ruscker_<version>_amd64.deb
sudo systemctl restart ruscker
```

`--force-confold` preserves `ruscker.env` (your token) and the config;
the systemd drop-in is untouched. New DB migrations apply on the next
start.
