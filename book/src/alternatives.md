# Ruscker vs. the alternatives

Ruscker is a **portal-and-orchestrator for container-per-session and
container-per-API workloads**. That puts it next to a handful of
established tools — some open source, some commercial, some R-only, some
notebook-only. This page is an honest map of where Ruscker fits and what
it replaces.

The short version: if you run **ShinyProxy** or **Shiny Server** today
and want the same model without the JVM weight (or without paying for a
commercial product), Ruscker is a drop-in-shaped fit — it even reads
ShinyProxy's `application.yml`.

## At a glance

| | **Ruscker** | **ShinyProxy** | **Shiny Server** (open source) | **Posit Connect** | **JupyterHub** |
|---|---|---|---|---|---|
| Runtime | Rust — single static binary | JVM (Java / Spring) | C++ / Node | proprietary | Python |
| Idle footprint | **~16 MB** | ~300–540 MB | moderate | heavy | moderate–heavy |
| Per-session isolation | ✅ container/session | ✅ container/session | ❌ shared R processes | ✅ | ✅ per-user |
| Frameworks | Shiny, Streamlit, Dash, Voilà, Plumber, FastAPI — any HTTP/WS container | any container | **R/Shiny only** | R, Python, Quarto, APIs | Jupyter/Python (+ images) |
| Admin UI | ✅ live dashboard + full CRUD | minimal | edit config + restart | ✅ rich | partial |
| Multi-host scheduling | ✅ Docker over ssh/tcp | ✅ (Kubernetes / operator) | ❌ | ✅ | ✅ (Kubernetes) |
| HA / active-active | ✅ (shared Postgres) | ✅ (operator) | ❌ | ✅ | ✅ |
| Reads ShinyProxy YAML | ✅ | — (native) | ❌ | ❌ | ❌ |
| License | open source | open source (Apache-2.0) | open source (free tier) | **commercial** | open source (BSD) |

"Footprint" is the idle resident memory of the orchestrator itself, not
the apps it runs. Feature checkmarks are deliberately coarse — see the
per-tool notes for the nuance.

## ShinyProxy

The closest comparison, and the one Ruscker is YAML-compatible with.
ShinyProxy is mature and capable, but it's a **Spring Boot application
on the JVM**: it idles at hundreds of megabytes, takes seconds to start,
and is configured by hand-editing `application.yml` and restarting.
Theming is Thymeleaf templates.

**What Ruscker changes:** the same container-per-session model and the
*same config file*, but as a single static binary that idles in the low
tens of MB, plus a real admin panel (apps CRUD, image library, encrypted
credentials, live dashboard, audit log, user roles) instead of "edit
YAML and restart". See [Migrating from ShinyProxy](./migrating.md).

**What ShinyProxy still does that Ruscker doesn't (yet):** a mature
Kubernetes operator and pluggable enterprise auth (OIDC/SAML/LDAP for
*app access*). Ruscker's multi-host is Docker-over-ssh/tcp today. Per-app
visibility and access control (`access-groups` / `access-users`,
ShinyProxy-compatible) are already supported; OIDC/SAML/LDAP gating at
the proxy level is not (apps can handle their own auth internally).

### Strip model vs. `SHINYPROXY_PUBLIC_PATH`

One migration detail worth knowing: ShinyProxy uses a **no-strip** model —
it passes the full public path to the container as the
`SHINYPROXY_PUBLIC_PATH` environment variable, and well-behaved
ShinyProxy demo images read that variable to self-prefix all their URLs.
Ruscker uses a **strip** model — the proxy strips `/app/{id}` from the
request path before forwarding, so the container always receives a
root-relative path (e.g. `/lab/...` instead of `/app/jupyter/lab/...`).
The container does **not** need to know its mount path; the proxy injects
a `<base href>`, rewrites static URLs in HTML responses, and patches
runtime fetches via a small JS shim.

This means **do not set `SHINYPROXY_PUBLIC_PATH` in `container-env`** for
Ruscker — the container would misconfigure itself and 404 on every request.
Most apps need no mount-path configuration at all; just serve them at the
root. Jupyter, for example, runs at `--ServerApp.base_url=/`:

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

For the rare app that genuinely needs to know its external mount path — to
build absolute URLs the proxy can't rewrite — Ruscker exposes an explicit
opt-in token `#{publicPath}`, substituted at spawn with the spec's actual
mount path (including any `--base-path` prefix), for use in `container-cmd`
or `container-env`. Do **not** use it for Jupyter: under the strip model a
non-root `base_url` makes it 404 every path (see
[Troubleshooting](./troubleshooting.md)). Apps like Shiny, Streamlit, Dash
and Voilà never need it.

### Secrets via env-var interpolation

Both Ruscker and ShinyProxy support `${VAR}` references in the YAML. The
difference is where resolution happens. In Ruscker, `${VAR}` references
in `container-env` values are resolved **at spawn**, not at parse: the
literal `${DB_PASSWORD}` is stored in the config and the database; only
when a container is actually started does Ruscker look up the environment
variable and inject the real value. This means **secrets never land in
the database** — the DB only ever sees the placeholder. Set secrets in
the process environment (or in `/etc/ruscker/ruscker.env` for the systemd
service) and reference them by name in the YAML.

## Shiny Server (open source / "Free")

Shiny Server's free edition runs R/Shiny apps but **does not isolate
sessions** — users share R processes per app — and it **doesn't scale or
load-balance**. Authentication, per-user resources and monitoring are
Pro (commercial) features. Administration is "edit the config file and
restart".

**What Ruscker changes:** real per-session isolation (one container per
visitor), an auto-scaler with replica pools, a monitoring dashboard, and
it isn't limited to R — Streamlit, Dash, Voilà, FastAPI and anything in
a container are first-class.

## Posit Connect (formerly RStudio Connect)

The polished commercial option: a publishing platform for R, Python,
Quarto and APIs with push-button deploys, scheduling, and fine-grained
access control. It does far more than Ruscker — but it's **commercial,
per-seat licensed, and heavy**.

**Where Ruscker fits:** when you want a self-hosted, open-source,
lightweight orchestrator and don't need Connect's publishing workflow or
are unwilling to pay per-seat. Ruscker is a proxy/orchestrator, not a
publishing platform — you bring your own container images.

## JupyterHub

JupyterHub spawns a **per-user** server (notebook/Lab), typically via a
Docker or Kubernetes spawner. It's excellent for interactive Python/data
science, but it's Python-centric, leans on Kubernetes for real scale, and
is organized around *users with notebooks* rather than *apps with
sessions*.

**Where Ruscker fits:** a per-**app** portal (each app a card, each
visitor a container) across mixed frameworks, lighter to run, configured
by one YAML file. You can still run a Jupyter/Lab image as a Ruscker spec.

## Streamlit, Dash, Voilà, Gradio (self-hosted)

These frameworks ship their own dev server but **no multi-app portal,
session isolation, auth, or scaling** — self-hosting means rolling your
own reverse proxy, container lifecycle and landing page. That glue is
exactly what Ruscker is. Point a spec at your Streamlit/Dash/Voilà/Gradio
image and you get the portal, sticky sessions, WebSocket forwarding,
scaling and reaping for free. See [What Ruscker can serve](./use-cases.md).

## When Ruscker is *not* the right tool

- You need a **publishing/authoring workflow** (push from an IDE,
  scheduled reports, content versioning) — that's Posit Connect.
- You're **all-in on Kubernetes** and want a CRD-native operator today —
  ShinyProxy's operator or a k8s-native platform is a better match
  (Ruscker schedules onto Docker hosts, not k8s).
- You need **enterprise SSO gating app access per user** (OIDC/SAML/LDAP
  at the proxy level) — Ruscker's auth covers admin roles (Viewer /
  Editor / Admin) and per-app visibility (`access-groups` / `access-users`),
  but proxy-level SSO is not yet supported.

If none of those apply and you want ShinyProxy's model without the JVM,
start with the [Quickstart](./quickstart.md).
