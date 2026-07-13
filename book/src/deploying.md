# Deploying in production

This walks through the production pattern Ruscker was designed for:
the `.deb` on a Docker host, behind nginx, optionally side-by-side
with an existing ShinyProxy.

## 1. Install and configure

```sh
sudo apt install ./ruscker_<version>-1_amd64.deb
```

Manage your **apps, landing page and users in the admin panel** at
`/admin` — the unit runs with `--db`, so the catalog is live out of the
box. `/etc/ruscker/ruscker.yml` only carries **service** settings (every
option is documented inline in the shipped file); put your **secrets** in
`/etc/ruscker/ruscker.env` (read by the unit):

```ini
RUSCKER_ADMIN_TOKEN=...        # openssl rand -hex 32
RUSCKER_MASTER_KEY=...         # for the credentials store
RUSCKER_COOKIE_KEY=...         # keep sticky sessions stable across restarts
DOCKER_REGISTRY_PASSWORD=...   # referenced as ${DOCKER_REGISTRY_PASSWORD} in the YAML
```

## 2. Enable the container backend

The shipped unit serves landing + admin + proxy but not the `--docker`
backend. The easy path is the helper, which writes the drop-in for you
(preserving your `--bind`/`--config`/`--db`):

```sh
sudo ruscker-enable-docker
```

Or do it by hand with a drop-in (so upgrades don't clobber it):

```sh
sudo systemctl edit ruscker
```

```ini
[Service]
SupplementaryGroups=docker
ExecStart=
ExecStart=/usr/bin/ruscker serve --config /etc/ruscker/ruscker.yml \
  --docker --db /var/lib/ruscker/ruscker.db
```

```sh
sudo systemctl daemon-reload && sudo systemctl restart ruscker
```

> Adding `ruscker` to the `docker` group is effectively root on the
> host — the same trade-off ShinyProxy carries.

### On-demand vs. pre-warmed

By default an app **cold-starts**: `min-replicas` defaults to **0**, so
no container runs until the first visitor arrives (they see a brief
splash while it spawns), and the auto-scaler reaps it once idle — the
same on-demand behaviour as ShinyProxy and Shiny Server Free, and what
keeps a many-app portal light. To keep an app **warm** (no cold-start
latency on the first hit), set `min-replicas: 1` (or more) on that spec;
the scaler then keeps that many replicas running at all times.

### Multiple Docker hosts (Phase 6)

By default Ruscker drives the **local** Docker daemon. To spread app
containers across several hosts, list them under `proxy.hosts` — then
Ruscker schedules, routes, monitors and reaps containers on all of
them from the one process:

```yaml
proxy:
  hosts:
    - id: gpu-1
      address: ssh://ops@10.0.0.11        # Docker over SSH — simplest
    - id: gpu-2
      address: tcp://10.0.0.12:2376       # Docker over TLS
      tls: { ca: /etc/ruscker/ca.pem, cert: /etc/ruscker/cert.pem, key: /etc/ruscker/key.pem }
      max-containers: 40                  # cap; placement won't exceed it
      weight: 2                           # bigger host ⇒ more of the spread
  specs:
    - id: heavy-shiny
      container-image: org/shiny:latest
      placement: spread                   # spread (default) | bin-pack
      anti-affinity: true                 # replicas on distinct hosts
```

**Transports.** `ssh://user@host[:port]` (reuses your SSH keys — no
daemon TLS to set up), `tcp://host:port` + `tls` (mutual TLS), or
`http://host:port` (plain TCP — trusted networks only). SSH is the
least-effort option. Empty `hosts` keeps the single-local-daemon mode.

**Networking — the catch.** On a remote host Ruscker publishes each
container's port on the host's `0.0.0.0` and proxies to
`host:published-port`. So **the Ruscker box must be able to reach the
app hosts on the ephemeral published ports** (roughly 32768–60999).
Put Ruscker and the hosts on a private network and open that range
between them; **do not expose those ports to the public** — they're
unauthenticated app backends. (For SSH hosts, the SSH connection is
only the Docker *control* plane; the *data* plane is still this direct
TCP path.)

**Placement.** `spread` (default) distributes replicas (weighted by
`weight`) for fault isolation; `bin-pack` fills one host before the
next. `anti-affinity: true` keeps a spec's replicas on distinct hosts,
falling back gracefully when every eligible host already runs it.
`max-containers` caps a host; if all hosts are full a spawn fails and
the scaler retries. The dashboard shows the host inline next to each
replica's container id, so you can see where each
replica landed.

**Validating it.** The default crate tests cover weighted spread,
bin-pack, capacity caps, anti-affinity fallback, placement-cache
accounting and degraded-host reconciliation without a Docker daemon:

```sh
cargo test -p ruscker-docker
```

An additional gated integration test exercises spawn + spread + routed
stop against two real daemons:

```sh
RUSCKER_IT_HOST1=ssh://ops@10.0.0.11 \
RUSCKER_IT_HOST2=ssh://ops@10.0.0.12 \
cargo test -p ruscker-docker --features multihost-it -- --nocapture
```

## 3. nginx

Terminate TLS at your edge / load balancer and forward to nginx with
`X-Forwarded-Proto: https`. A minimal reverse proxy:

```nginx
server {
    listen 80;
    server_name portal.example.com;

    # Media-library / card-image uploads. nginx defaults to 1 MB, which
    # silently 413s any larger image before it reaches Ruscker (the admin
    # "upload doesn't work" symptom). Ruscker accepts images up to
    # 10 MB (12 MB request limit, allowing multipart overhead).
    client_max_body_size 16m;

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
`X-Forwarded-*`. **Since v0.2.5 this is required** for cookies to carry
the `Secure` flag behind a TLS-terminating proxy — without the flag,
forwarded headers from any client would be spoofable, so Ruscker
ignores them entirely (cookies are then minted as if on plain HTTP, and
per-client API rate limiting keys on the TCP peer, i.e. your proxy).
Both halves are needed: the `proxy_set_header X-Forwarded-Proto`
above **and** the YAML flag. ShinyProxy-migrated configs usually carry
the flag already.

> Two Ruscker endpoints stream over **Server-Sent Events** (SSE):
> the live dashboard (`/admin/dashboard/events`) and the Ruscker log
> tail (`/admin/logs/stream`). nginx's default response buffering
> delays or blocks these streams — disable it for both paths:
>
> ```nginx
> location = /admin/dashboard/events {
>     proxy_pass http://127.0.0.1:8090;
>     proxy_buffering off;
>     proxy_read_timeout 1h;
>     # + the same proxy_set_header lines as above
> }
> location = /admin/logs/stream {
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

## 4b. Mounting under a base path (subpath)

When you can't create a subdomain (no DNS governance), serve the whole
portal under a subpath like `example.org/apps/`. Start Ruscker with
`--base-path /apps` (or `server.context-path: /apps` in the config —
ShinyProxy's `server.servlet.context-path` is also accepted), and point
nginx at it **preserving the prefix** (no trailing path on `proxy_pass`,
so the `/apps` stays in the forwarded URL):

```nginx
# Forward both the bare /apps and everything under /apps/ — without an
# nginx-side redirect (Ruscker does its own canonicalization below).
location = /apps  { proxy_pass http://127.0.0.1:8080; /* + proxy_set_header from §3 */ }
location /apps/   { proxy_pass http://127.0.0.1:8080; /* + proxy_set_header from §3 */ }
```

Ruscker then nests every route under `/apps` (`/apps`, `/apps/admin/…`,
`/apps/app/{spec}/…`), rewrites the URLs/redirects it emits to carry the
prefix, and injects a small runtime shim so JS-built requests (the live
dashboard's SSE, `fetch`, etc.) resolve under `/apps` too. `/healthz` and
`/readyz` stay at the **root** for load-balancer probes — don't put them
behind the `/apps` locations. `--base-path` is empty by default, so
root-mounted deploys are unaffected.

> The canonical landing URL is `/apps` (no trailing slash); a request to
> `/apps/` 308-redirects to it. Keeping nginx as a plain `proxy_pass`
> (never redirecting) means that single hop can't loop.

## 5. Health checks

Point your load balancer / orchestrator at:

- `GET /healthz` — liveness, always `200` (no dependencies).
- `GET /readyz` — readiness; probes the DB (`SELECT 1`) and the Docker
  backend, returns `503` while draining or when a dependency is down.

On `SIGTERM` Ruscker flips `/readyz` to `draining`, lets in-flight
sessions wind down up to `proxy.shutdown-grace-ms`, then exits.

## Running active-active (HA, Phase 7)

For high availability you can run **several Ruscker instances behind a
load balancer**, all sharing one Postgres. There's a runnable harness
(two instances + nginx + Postgres) in [`examples/ha/`][ha] — start
there. The three things every instance must share:

- **`--config-db-url postgres://…`** — the admin catalog (specs,
  landing, users, credentials, audit) lives in Postgres instead of a
  per-node SQLite file, so an edit on any instance is seen by all. Runs
  the same migrations as SQLite; `--db` stays the single-node default.
- **`--session-store-url postgres://…`** — one shared `proxy_sessions`
  table. Each instance reconciles the cluster-wide per-replica session
  counts, so routing and the scaler agree across the fleet.
- **the same `RUSCKER_COOKIE_KEY`** on every instance — the sticky
  cookie is an HMAC, so a shared key lets any instance validate a cookie
  another minted. With that, the shared session table, and every
  instance pointed at the same Docker backend (so they reconcile the
  same replicas), a session survives the load balancer moving it
  between instances.

**Scaler leader election is automatic:** instances elect one leader via
a Postgres advisory lock; only the leader spawns/reaps replicas, the
rest serve traffic. If the leader dies its lock releases and another
takes over within one scaler tick — nothing to configure. Each instance
logs its role at startup (`acquired leadership` / `standing by`).

> **Use a direct Postgres connection for the leader lock.** The advisory
> lock is **session-scoped** — it lives on one specific backend
> connection and is held for the process's lifetime. Point
> `--config-db-url` (or `--session-store-url`) at Postgres **directly**, or
> through a pooler in **session** mode only. A transaction-pooling proxy
> (e.g. PgBouncer in transaction mode) hands the lock-holding connection
> to other clients between statements, which can break the single-leader
> guarantee and let two instances scale at once.

**App traffic** doesn't need sticky upstreams — proxy sessions are
shared (the sticky cookie is a shared-key HMAC and the `proxy_sessions`
table is shared), so round-robin is fine for `/app` and `/api`. Keep
`server.useForwardHeaders: true` and pass `X-Forwarded-*` as in the
nginx section above.

### Shared admin sessions (eliminate the sticky-upstream caveat)

By default the **sign-in session** (the cookie minted at `/admin/login`)
lives in an in-memory map per process. An admin — or, with per-group app
visibility ([access control](./configuration.md)), any signed-in user —
who logs into instance A and is then round-robined to instance B is
bounced back to the login screen (B doesn't know the session), and on
the proxy path B would treat them as anonymous (so a restricted app
could 403/redirect even though they're entitled to it).

**Recommended fix:** point Ruscker at a shared Postgres for the admin
session table (#185):

```bash
ruscker serve … --admin-session-store-url postgres://ruscker:pw@pg:5432/ruscker
```

(or `RUSCKER_ADMIN_SESSION_STORE_URL=…`). All instances then read and
write the same `admin_sessions` table. A short node-local TTL cache
(default **5 s**) means the hot path — every authenticated request and
every `/app` proxy guard — keeps serving from memory; the DB is only
hit on cache miss or after a write. Logout propagates to sibling
instances within one cache window (worst case 5 s of "still signed in"
on a peer). The table auto-creates on connect; no migration step is
needed.

This removes the need for a sticky LB. Round-robin (or least-conn) is
fine; any node can serve any session-bearing path.

> **One database, three tables.** `--config-db-url` (shared spec
> catalog), `--session-store-url` (proxy `proxy_sessions` table from
> Phase 7), and `--admin-session-store-url` (sign-in sessions, #185)
> can all point at the **same** Postgres URL — each store creates and
> owns its own table. No need to provision three databases.

#### Fallback: sticky upstream

If you can't host shared Postgres for admin sessions, pin the
session-bearing paths to one instance with a sticky upstream. The
simplest is `ip_hash` (route by client IP); a cookie-based sticky on
the `ruscker_admin_session` cookie is more precise if you have nginx
Plus or a similar LB.

```nginx
# Pin each client to one instance so its sign-in session is found.
upstream ruscker {
    ip_hash;                      # sticky by client IP
    server 10.0.0.11:8080;
    server 10.0.0.12:8080;
}
server {
    # … TLS, proxy headers (see above) …
    location / {                  # landing, /admin, /app, /api
        proxy_pass http://ruscker;
    }
}
```

Either path — shared store or sticky upstream — keeps **no data lost**
on a failover. The shared store skips the re-login; sticky upstream
re-logs in the user only when the pinned node falls over.

[#161]: https://github.com/StrategicProjects/ruscker/issues/161

> With `--config-db-url`, the proxy, landing and scaler all resolve
> specs **from the shared Postgres catalog first** (YAML is the union
> fallback for ids not in the DB) — an admin edit on one instance is
> served live by every instance. Seed a fresh Postgres catalog from
> YAML with `ruscker import --config-db-url postgres://…` (idempotent,
> same as the SQLite `--db` import).

[ha]: https://github.com/StrategicProjects/ruscker/tree/main/examples/ha

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
      serve --config /etc/ruscker/ruscker.yml
      --bind 0.0.0.0:8080 --docker --db /var/lib/ruscker/ruscker.db
    ports: ["127.0.0.1:8090:8080"]
    environment:
      RUSCKER_ADMIN_TOKEN: ${RUSCKER_ADMIN_TOKEN}
      RUSCKER_MASTER_KEY:  ${RUSCKER_MASTER_KEY}
      RUSCKER_COOKIE_KEY:  ${RUSCKER_COOKIE_KEY}
    volumes:
      - ./ruscker.yml:/etc/ruscker/ruscker.yml:ro
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

- **`/etc/ruscker/`** — `ruscker.yml` and `ruscker.env` (your
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
sudo dpkg -i --force-confold ruscker_<version>-1_amd64.deb
sudo systemctl restart ruscker
```

`--force-confold` preserves `ruscker.env` (your token) and the config;
the systemd drop-in is untouched. New DB migrations apply on the next
start.
