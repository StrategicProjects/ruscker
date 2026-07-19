# Where Ruscker fits

Ruscker is a **portal-and-orchestrator for container-per-session and
container-per-API workloads** — interactive apps that want a fresh
container per visitor, and stateless HTTP services pooled per replica. If
your apps run in a container and speak HTTP or WebSocket, Ruscker wraps
them in a portal, a reverse proxy, sticky sessions, auto-scaling and an
admin panel. Service settings can live in `ruscker.yml`; the editable app
catalog and operational state live in SQLite, or Postgres for HA.

## What Ruscker is good at

- **Per-session isolation** — one container per visitor for stateful
  apps (Shiny, Streamlit, Dash, Voilà, notebooks), so sessions never
  share state.
- **Stateless APIs** — pooled per replica with an auto-scaler (Plumber2,
  FastAPI, and any HTTP service).
- **Mixed frameworks under one portal** — every app is a card on the
  landing page; anything in a container is first-class.
- **A real admin panel** — apps CRUD, a media library, an encrypted
  credentials store, a live monitoring dashboard, an audit log, and user
  roles — instead of editing a file and restarting.
- **Sensitive internal apps** — per-app step-up TOTP MFA is enforced before
  a container starts, while opt-in identity headers let the app consume the
  signed-in username, groups and selected profile claims.
- **Scheduled operations** — run the same app image/environment/volumes to
  completion on a cron for ETL, reports and maintenance, with history,
  timeouts and failure alerts.
- **Light to run** — **~14 MB idle**, a single static binary, no JVM and
  instant startup.

## Compared with ShinyProxy

Ruscker keeps the familiar spec schema and `/app/{spec}` shape, including
opt-in `add-default-http-headers: true` compatibility through
`X-SP-UserId` and `X-SP-UserGroups`. It adds per-app step-up MFA, selected
`X-Ruscker-User-*` profile claims, stateless API pools, scheduled jobs and
an editable database-backed admin catalog. Identity forwarding defaults
off, so enable it explicitly for apps that need and trust those headers.

## Self-hosting a single framework

Streamlit, Dash, Voilà and Gradio ship their own dev server but **no
multi-app portal, session isolation, auth, or scaling** — self-hosting
means rolling your own reverse proxy, container lifecycle and landing
page. That glue is exactly what Ruscker is. Point a spec at your image
and you get the portal, sticky sessions, WebSocket forwarding, scaling
and reaping for free. See [What Ruscker can serve](./use-cases.md).

## Sub-path handling (the strip model)

Ruscker uses a **strip** model: the proxy strips `/app/{id}` from the
request path before forwarding, so the container always receives a
root-relative path (e.g. `/lab/...` instead of `/app/jupyter/lab/...`).
The container does **not** need to know its mount path — the proxy injects
a `<base href>`, rewrites static URLs in HTML responses, and patches
runtime fetches via a small JS shim.

The practical consequence: serve apps at the root and don't hard-code a
mount path. Jupyter, for example, runs at `--ServerApp.base_url=/`:

```yaml
- id: jupyter
  container-image: quay.io/jupyter/minimal-notebook:latest
  container-port: 8888
  container-cmd:
    - start-notebook.py
    - --IdentityProvider.token=
    - --ServerApp.allow_origin=*
    - --ServerApp.base_url=/
```

If you're carrying over a config where the app reads a `*_PUBLIC_PATH`
environment variable to self-prefix its URLs, **drop it** under Ruscker —
the strip model already handles the mount path, and a self-prefixing app
would misconfigure itself and 404 on every request.

For the rare app that genuinely needs its external mount path — to build
absolute URLs the proxy can't rewrite — Ruscker exposes an explicit opt-in
token `#{publicPath}`, substituted at spawn with the spec's actual mount
path (including any `--base-path` prefix), for use in `container-cmd` or
`container-env`. Do **not** use it for Jupyter: under the strip model a
non-root `base_url` makes it 404 every path (see
[Troubleshooting](./troubleshooting.md)). Apps like Shiny, Streamlit, Dash
and Voilà never need it.

## Secrets via env-var interpolation

Ruscker supports `${VAR}` references in the YAML, resolved **at spawn**,
not at parse: the literal `${DB_PASSWORD}` is stored in the config and the
database; only when a container is actually started does Ruscker look up
the environment variable and inject the real value. This means **secrets
never land in the database** — the DB only ever sees the placeholder. Set
secrets in the process environment (or in `/etc/ruscker/ruscker.env` for
the systemd service) and reference them by name in the YAML.

## When Ruscker is *not* the right tool

- You need a **publishing / authoring workflow** — pushing source from an
  IDE, building content or versioning releases. Ruscker can schedule an
  existing image, but it does not replace your build and publishing system.
- You're **all-in on Kubernetes** and want a CRD-native operator today —
  Ruscker schedules onto Docker hosts (over ssh/tcp), not Kubernetes.
- You need **enterprise SSO gating app access per user** (OIDC/SAML/LDAP
  at the proxy level) — Ruscker's auth covers admin roles (Viewer /
  Editor / Admin) and per-app visibility (`access-groups` /
  `access-users`), but proxy-level SSO is not yet supported.

If none of those apply, start with the [Quickstart](./quickstart.md).
