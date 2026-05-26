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

## Upgrading

Build the new `.deb`, copy it over, and reinstall keeping your config:

```sh
sudo dpkg -i --force-confold ruscker_<version>_amd64.deb
sudo systemctl restart ruscker
```

`--force-confold` preserves `ruscker.env` (your token) and the config;
the systemd drop-in is untouched. New DB migrations apply on the next
start.
