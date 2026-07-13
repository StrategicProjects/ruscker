# Configuration

Ruscker's configuration lives in **four places**, each with its own
job — keeping them apart answers most "where do I set X?" questions:

- **`ruscker.yml`** — how the **service** runs: bind address and port,
  the subpath (`server.context-path`), forwarded-header trust, proxy
  timeouts, metrics. The `.deb` installs a fully self-documented file
  at `/etc/ruscker/ruscker.yml`; a few of these also have CLI-flag
  overrides for one-off runs.
- **`ruscker.env`** — **secrets** and environment: the admin token,
  the master/cookie keys, registry passwords. Secrets are never
  written in YAML — reference them as `${VAR}` and define them here.
- **The database** (`--db`) — your **portal**: apps (specs), the
  landing appearance, users, credentials, media. Managed from the
  [admin panel](./admin.md); on a fresh `--db` it even seeds a set of
  showcase apps for you.
- **`application.yml`** — the **ShinyProxy import format**:
  `ruscker import application.yml --db …` brings an existing config
  into the database. It parses with the same schema as `ruscker.yml`,
  so an existing file also still works as `--config` — but the
  canonical service file is `ruscker.yml` (`serve` without `--config`
  finds `ruscker.yml` first and falls back to `application.yml`).

If you prefer to drive everything from YAML (GitOps), you still can —
spec entries are accepted in the service file too. For a normal
install the admin panel is where you manage apps, and the YAML stays
small.

## Where each setting lives

| You want to… | Set it in |
|---|---|
| Add / edit apps, APIs, links | **Admin panel → Apps** (or `proxy.specs` to import) |
| Customise the landing (title, colours, logos, SEO, blocks) | **Admin panel → Portal** |
| Manage users, roles, group membership | **Admin panel → Users** |
| Store registry credentials | **Admin panel → Credentials** |
| Bind address / port | `ruscker.yml` (`proxy.bind-address` / `proxy.port`); `--bind` overrides |
| Serve at the root or a subpath | `ruscker.yml` (`server.context-path`); `--base-path` / `RUSCKER_BASE_PATH` override |
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

```yaml
# ruscker.yml
server:
  context-path: /apps
```

(Equivalently the `--base-path /apps` flag or the `RUSCKER_BASE_PATH`
env var — precedence: flag > env > file.) Ruscker then emits every URL — landing,
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

Everything below is the complete YAML schema (the same
`docs/YAML_SCHEMA.md` shipped in the repo) — `ruscker.yml` and the
ShinyProxy-compatible `application.yml` both parse with it. Reach for
it to **migrate an existing ShinyProxy config**, or to drive specs and
landing from YAML instead of the admin panel — not for a normal,
admin-panel-managed install.

{{#include ../../docs/YAML_SCHEMA.md}}
