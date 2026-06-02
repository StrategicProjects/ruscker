# Configuration

Ruscker has **two layers** of configuration, and it helps to keep them
apart:

- **Portal content** — your apps (specs), the landing page, users,
  credentials, and media. This lives in the **database** and is managed
  from the [admin panel](./admin.md); you don't write any YAML for it.
  On a fresh `--db` the portal even seeds a set of showcase apps for you.
- **Runtime / deployment** — *how and where* Ruscker runs: the address it
  binds, whether it sits at the site root or under a subpath, the Docker
  backend, the database, secrets, and HA. These are **CLI flags and
  environment variables**, set once at startup.

The `application.yml` file is the **bootstrap + migration format**: it
must exist (it can be two lines), it carries the runtime `proxy:`
settings that aren't flags, and it's how you **import an existing
ShinyProxy config**. If you prefer to drive everything from YAML
(GitOps), you still can — but for a normal install the admin panel is
where you manage apps, and the YAML stays small.

> Secrets are never written in the YAML — use `${VAR}` interpolation and
> set the variables in the environment (or `/etc/ruscker/ruscker.env`).

## Where each setting lives

| You want to… | Set it in |
|---|---|
| Add / edit apps, APIs, links | **Admin panel → Apps** (or `proxy.specs` to import) |
| Customise the landing (title, colours, logos, SEO, blocks) | **Admin panel → Portal** |
| Manage users, roles, group membership | **Admin panel → Users** |
| Store registry credentials | **Admin panel → Credentials** |
| Bind address / port | `--bind` (or `proxy.bind-address` / `proxy.port`) |
| Serve at the root or a subpath | `--base-path` (or `proxy.server.context-path`) |
| Enable / disable the Docker backend | auto · `--docker` · `--no-docker` |
| Database (catalog, users, sessions) | `--db <file>` · `--config-db-url` (Postgres/HA) |
| Admin token + crypto keys | `RUSCKER_ADMIN_TOKEN`, `RUSCKER_MASTER_KEY`, `RUSCKER_COOKIE_KEY` |
| Log format | `--log-format text\|json` |

## Deployment settings

The decisions you make at startup. Most have both a CLI flag and an env
var; see [Deploying in production](./deploying.md) for the full systemd
+ nginx walkthrough.

### Served at the root, or under a subpath?

The most common deployment question. By default Ruscker serves the portal
at the **site root** (`https://apps.example.org/`). If you can't dedicate
a subdomain and need it under a path (`https://example.org/apps/`), set a
base path:

```sh
ruscker serve --config application.yml --bind 127.0.0.1:8080 --db ruscker.db \
  --base-path /apps
```

(Equivalently `proxy.server.context-path: /apps` in the YAML, or the
`RUSCKER_BASE_PATH` env var.) Ruscker then emits every URL — landing,
admin, assets, and the `/app` proxy — under `/apps`, and rewrites app
responses so unmodified Shiny / Streamlit / Jupyter apps work behind the
prefix. Point your reverse proxy's `/apps/` location at Ruscker. Full
nginx example: [Mounting under a base path][base-path].

### Bind address, Docker, database

- **`--bind <addr:port>`** — where Ruscker listens (overrides
  `proxy.bind-address` / `proxy.port`). Behind nginx, bind to localhost.
- **Docker backend** — auto-connects when the daemon socket is reachable.
  `--no-docker` runs landing-only (the `/app` proxy returns 503);
  `--docker` makes a failed connect fatal (e.g. a remote daemon you
  require).
- **`--db <file>`** — the SQLite catalog (apps, users, landing, audit,
  sessions); required for `/admin/*`. For active-active HA use
  **`--config-db-url postgres://…`** (a shared catalog) instead.

### Secrets

Set these in the environment (the `.deb` puts them in
`/etc/ruscker/ruscker.env`):

- **`RUSCKER_ADMIN_TOKEN`** — unlocks `/admin` and is the break-glass
  login. Without it, admin routes return 503.
- **`RUSCKER_MASTER_KEY`** — AES-256 key for the encrypted credentials
  store.
- **`RUSCKER_COOKIE_KEY`** — HMAC key for sticky-session cookies. Set it
  explicitly in production so sessions survive restarts (and are valid
  cross-instance in HA); without it a random key is generated per
  process.

### High availability

Running more than one instance behind a load balancer? Share the catalog
(`--config-db-url`), the proxy session store (`--session-store-url`), the
admin session store (`--admin-session-store-url`), and the same
`RUSCKER_COOKIE_KEY` across instances, so any node can serve any request.
See [Shared admin sessions][ha-sticky].

[base-path]: ./deploying.md#4b-mounting-under-a-base-path-subpath
[ha-sticky]: ./deploying.md#shared-admin-sessions-eliminate-the-sticky-upstream-caveat

## Per-user access

`access-groups` / `access-users` on a spec scope who can **see** the
card on the landing **and** reach the upstream at `/app` / `/api`. A spec
with neither key is **open** — visible to everyone, including anonymous
visitors. Otherwise:

- An **admin** session sees everything.
- A **signed-in user** sees a restricted spec when their username is in
  `access-users` *or* one of their groups is in `access-groups`.
- An **anonymous visitor** only sees open specs.

Enforcement is real — the `/app` and `/api` guards reject unauthorized
requests (anonymous on `/app` → redirected to login; otherwise 403), not
just hide the landing card.

You set both keys on the spec form (admin panel → Apps), and group
membership per user on the admin **Users** page. The same user record
drives both portal visibility and admin role (Admin / Editor / Viewer) —
see [The admin panel](./admin.md). In YAML the keys look like:

```yaml
proxy:
  specs:
    - id: open-app
      display-name: Open App
      container-image: demo/img        # no access keys ⇒ open
    - id: analysts-app
      display-name: Analysts App
      container-image: demo/img
      access-groups: [analysts]
    - id: vip-app
      display-name: VIP App
      container-image: demo/img
      access-users: [carol]
```

## The full YAML reference

Everything below is the complete `application.yml` schema (the same
`docs/YAML_SCHEMA.md` shipped in the repo). Reach for it to **migrate an
existing ShinyProxy config**, or to drive specs and landing from YAML
instead of the admin panel — not for a normal, admin-panel-managed
install.

{{#include ../../docs/YAML_SCHEMA.md}}
