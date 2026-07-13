# YAML schema reference

Reference for every field Ruscker understands in `application.yml`.

For ShinyProxy users: this document marks which ShinyProxy features
are **supported**, **extended** by Ruscker, **deferred** to later
phases, or **not supported**.

## Top level

```yaml
server: { ... }      # Optional. Spring Boot-style server config
proxy: { ... }       # The main Ruscker config
logging: { ... }     # Optional. Logging config
```

## `server` block

Supported:

```yaml
server:
  useForwardHeaders: true             # Trust X-Forwarded-* headers (see below)
  forward-headers-strategy: native    # 'native' | 'framework' | 'none'
  context-path: /apps                  # Mount portal under a subpath (see below)
  # ShinyProxy's nested form is also accepted:
  servlet.context-path: /apps
```

Parsed for compatibility but **ignored** (each produces a startup
warning when set — see [Validation warnings](#validation-warnings)):

```yaml
server:
  secure-cookies: true                # IGNORED — see useForwardHeaders below
  servlet.session.timeout: 3600       # IGNORED (admin sessions are 24h fixed)
  # OR equivalently:
  servlet:
    session:
      timeout: 3600
```

### `server.useForwardHeaders` — forwarded-header trust

Since v0.2.5 this single switch gates **every** read of
client-suppliable `X-Forwarded-*` headers: the cookie `Secure` flag
(`X-Forwarded-Proto`), the per-client API rate-limit key, and the
`X-Forwarded-For` chain forwarded to your apps (trusted → the real
peer is appended; untrusted → the spoofable inbound value is
replaced). **If a reverse proxy terminates TLS in front of Ruscker,
set it to `true`** — otherwise cookies are minted without `Secure`.
ShinyProxy's `secure-cookies` flag is *not* what does this in Ruscker;
it's ignored. Full rationale in `docs/SECURITY.md` §7.

### `server.context-path` — subpath mounting

Serves the whole portal under a URL prefix when you can't dedicate a
subdomain (e.g. `example.org/apps/` instead of `box.example.org/`).
Normalized form: leading slash, no trailing slash (`"box"`,
`"/apps/"`, and `"/apps"` all become `/apps`). The CLI flag
`--base-path /apps` overrides the YAML.

ShinyProxy emits this as `server.servlet.context-path`; the flat
`server.context-path` form is also accepted. Empty / absent ⇒ served
at the root (the default).

Operational notes:

- The health probes `/healthz` and `/readyz` stay at the root
  regardless of the prefix — load balancers don't need to know it.
- The chrome's root-absolute URLs (`/admin/...`, `/assets/...`,
  redirects) are rewritten on the response so they all carry the
  prefix; runtime fetches (`fetch`/`XMLHttpRequest`/`WebSocket`/
  `EventSource`) get a small shim that prefixes them too.
- Cookies set `Path=/` (already sent by browsers for `{base}/...`).
- Configure your reverse proxy to pass requests through under the
  same prefix — see the **Mounting under a base path** section of
  `book/src/deploying.md`.

Other `server.*` fields from Spring Boot are accepted by serde but
ignored by Ruscker.

## `proxy` block

### Top-level proxy fields

| Field | Type | Default | Notes |
|---|---|---|---|
| `title` | string | `"Ruscker"` | Browser tab title |
| `landing-page` | string | `"/"` | **Ignored** (warned when set) — the portal is always at the root / base path |
| `hide-navbar` | bool | `false` | **Ignored** (warned when set) |
| `template-path` | path | none | Override template directory |
| `heartbeat-rate` | ms | `10000` | **Ignored** (warned when set) — the landing's heartbeat is fixed |
| `heartbeat-timeout` | ms | `3600000` | Session expiry; `-1` = never. Per-spec override supported |
| `container-log-path` | path | none | **Ignored** (warned when set) — use `docker logs` / the admin Logs tab |
| `port` | u16 | `8080` | HTTP listener port |
| `bind-address` | string | `"0.0.0.0"` | Listener interface |
| `authentication` | enum | `none` | `none` (only `none` is implemented today) / `openid` / `ldap` / `saml` / `simple` |
| `landing-customization` | block | `{}` | Branding, SEO/social meta, analytics, custom HTML blocks, sign-in visibility — see [§ `proxy.landing-customization`](#proxylanding-customization). Ruscker extension |
| `specs` | array | `[]` | List of apps/links/APIs |
| `container-wait-time` | ms | `60000` | **Not consumed today** — the spawn readiness wait is a fixed 60 s; wiring the field is tracked in issue #970 |
| `shutdown-grace-ms` | ms | `30000` | Drain window on SIGTERM/Ctrl-C before forced exit; `/readyz` reports `draining` during it. Ruscker extension |
| `max-body-size` | size | none | Global cap on proxied request bodies (`"10m"`, `"1g"`, bytes); over → `413`. Per-spec `max-body-size` overrides. Ruscker extension |
| `metrics-enabled` | bool | `false` | Expose a Prometheus `/metrics` endpoint (**unauthenticated** when on — firewall it). Ruscker extension |
| `metrics-interval` | s | `5` | How often the dashboard polls the backend for per-replica CPU/mem. A busy host can slow it (`10`–`15`) to ease the Docker daemon. `0` ⇒ default. Ruscker extension |
| `hosts` | list | `[]` | Docker hosts for multi-host scheduling (Phase 6). Empty ⇒ the local daemon. Ruscker extension |

### `proxy.hosts` — multi-host scheduling (Phase 6)

Empty (the default) means the single local Docker daemon — today's
behaviour. List hosts to spawn app containers across several daemons:

```yaml
proxy:
  hosts:
    - id: ssh-1
      address: ssh://ops@10.0.0.11      # Docker over SSH (simplest)
    - id: tcp-1
      address: tcp://10.0.0.12:2376     # Docker over TLS
      tls: { ca: /etc/ruscker/ca.pem, cert: /etc/ruscker/cert.pem, key: /etc/ruscker/key.pem }
      max-containers: 40                # optional cap
      weight: 2                         # optional, for spread placement
    - id: local
      address: unix:///var/run/docker.sock
```

Address schemes: `ssh://user@host[:port]`, `tcp://host:port` (needs
`tls`), `http://host:port` (plain TCP — trusted networks only),
`unix:///path`. `validate` flags empty/duplicate ids, unknown schemes,
and `tls` mismatched with the scheme.

**Per-host:** `max-containers` caps how many containers a host runs;
`weight` (default 1) biases spread placement toward bigger hosts.

**Per-spec placement** controls how a spec's replicas land across hosts:

```yaml
- id: heavy-shiny
  placement: spread          # spread (default) | bin-pack
  anti-affinity: true        # keep replicas on distinct hosts (best-effort)
```

- `spread` distributes replicas (weighted least-loaded) for fault
  isolation; `bin-pack` fills one host before using the next.
- `anti-affinity: true` prefers hosts not already running the spec,
  falling back to the strategy above if every eligible host does
  (so scaling never stalls). Hosts at `max-containers` are skipped; if
  all are full, the spawn fails (the scaler retries).

### Authentication

**Ruscker currently supports only `none`.** The other variants are
accepted by the parser (so existing YAML doesn't fail) but Ruscker will
warn at boot that auth is unimplemented and continue as if `none`. An
external identity provider (OIDC / SAML / LDAP) is the Phase 8 roadmap
item; today, apps that handle their own auth are the common case, and
`none` is correct — Ruscker just routes traffic.

For applications that handle their own auth internally (a common
case), `none` is the correct choice — Ruscker just routes traffic.

### `proxy.landing-customization`

Branding, SEO, analytics, and custom-HTML overrides for the public
landing. Every subfield is optional; an empty `landing-customization`
block (the default) renders the stock landing.

```yaml
proxy:
  landing-customization:
    # Branding — CSS colors applied to the landing header
    header-bg: "#0f6e56"               # any CSS color
    header-fg: "#ffffff"               # contrast override when bg is dark
    header-bg-dark: "#10312b"          # dark-theme header bg (blank = inherit header-bg)
    header-fg-dark: "#f0f0f0"          # dark-theme header text (blank = inherit header-fg)

    # Intro paragraph between header and filters
    intro: "Welcome to the portal."    # single-language fallback
    intro-locales:                     # per-language overrides; locale code → text
      pt: "Bem-vindo ao portal."
      en: "Welcome to the portal."
      es: "Bienvenido al portal."
      fr: "Bienvenue sur le portail."

    # SEO / social-share meta tags injected into the landing `<head>`
    seo-title: "Portal — Org Name"     # overrides `proxy.title` for <title>
    seo-description: "Internal apps."  # <meta name="description"> + og:description
    og-image: /assets/img/og.png       # path or absolute URL for og:image

    # Analytics — admin-trusted, injected verbatim into landing <head>
    analytics-html: |
      <script defer src="https://plausible.io/js/script.js"
              data-domain="example.org"></script>
    analytics-origins: "https://plausible.io"   # space-separated; widens landing CSP

    # Operator CSS — injected as a <style> late in the landing <head>,
    # so it can override the built-in styles
    custom-css: ".rk-card { border-radius: 0; }"

    # Header / footer logos (admin-managed in the live editor; see below)
    logos:
      - url: /assets/img/org-mark.png   # uploaded, built-in, or absolute URL
        slot: header                    # `header` | `footer`
        align: left                     # `left` | `center` | `right`
        link: https://org.example       # optional click-through
        height: 40                      # render height in px
        margin: 12                      # optional outer margin in px

    # Sign-in visibility (anonymous viewers only)
    show-admin-link: true              # default true; false hides the entrance

    # Custom HTML blocks (admin-managed in the live editor; see note below)
    blocks:
      - slot: top                       # `top` (after header) | `bottom` (after grid)
        title: "Maintenance banner"     # internal label, not shown publicly
        html: '<div class="...">Scheduled downtime Sunday 02:00 UTC.</div>'
        csp-origins: ""                 # space-separated origins this block needs
        enabled: true                   # default true
```

Field reference:

| Field | Type | Default | Notes |
|---|---|---|---|
| `header-bg` | CSS color | none | Header background. Match your brand's primary color. |
| `header-fg` | CSS color | none | Header text — set when `header-bg` is dark and the default loses contrast. |
| `header-bg-dark` | CSS color | none | Dark-theme header background. Unset inherits `header-bg`. |
| `header-fg-dark` | CSS color | none | Dark-theme header text. Unset inherits `header-fg`. |
| `card-cover-default` | CSS value | none | Default cover (solid/gradient) painted behind catalog cards that have no per-app `cover`/`accent`. Unset keeps the per-kind tint. |
| `card-cover-default-dark` | CSS value | none | Dark-theme default card cover. Unset inherits `card-cover-default`. |
| `intro` | string | none | Single-language fallback. Inline Markdown only — `**bold**`, `*italic*`, `[links](https://…)`; no HTML. |
| `intro-locales` | map | `{}` | Locale code → intro string (same inline Markdown). Wins over `intro` for matching locales. |
| `seo-title` | string | `proxy.title` | Override for `<title>`. |
| `seo-description` | string | resolved intro | `<meta name="description">` + `og:description`. |
| `og-image` | path / URL | none | `og:image` for social-share. |
| `analytics-html` | string | none | **Trusted** raw HTML, injected verbatim into landing `<head>`. |
| `analytics-origins` | string | none | Space-separated origins added to the landing CSP (`script-src`/`connect-src`/`img-src`). |
| `custom-css` | string | none | **Trusted** raw CSS, injected as a `<style>` late in the landing `<head>` so it overrides the built-in styles. |
| `show-admin-link` | bool | `true` | When `false`, anonymous visitors don't see the "Sign in" entrance. Logged-in users still see their panel link. |
| `show-highlights` | bool | `true` | Show the "Featured" carousel above the filters. The carousel still only renders when at least one spec is `featured`. |
| `logos[]` | list | `[]` | Header/footer logos (see below). |
| `blocks[]` | list | `[]` | Custom HTML blocks (see below). |

`logos[]` subfields:

| Field | Type | Default | Notes |
|---|---|---|---|
| `url` | string | required | Image URL — `/assets/img/...` (uploaded), a built-in (`/assets/showcase/...`, `/assets/brand/...`), or an absolute URL. |
| `slot` | enum | required | `header` or `footer` — where the logo renders. |
| `align` | enum | required | `left`, `center`, or `right` within the slot. `left`/`right` integrate into the chrome: header-`left` replaces the Ruscker mark, header-`right` trails the buttons, footer-`left` sits far-left, footer-`right` trails the version+mark. `center` renders in a separate bar. Logos sharing a slot+alignment render side by side. |
| `link` | URL | none | Optional click-through — when set, the logo becomes an `<a>`. |
| `height` | px | default | Per-logo render height in pixels; falls back to a built-in default when unset. |
| `margin` | px | none | Optional outer margin in pixels around the logo, for spacing from adjacent chrome or a neighbouring logo. |

`blocks[]` subfields:

| Field | Type | Default | Notes |
|---|---|---|---|
| `slot` | enum | required | `top` or `bottom` — render position on the landing. |
| `title` | string | required | Admin-only label; not rendered publicly. |
| `html` | string | required | **Trusted** raw HTML, injected verbatim into the chosen slot. |
| `csp-origins` | string | `""` | Space-separated origins this block's content needs, folded into the landing CSP. |
| `enabled` | bool | `true` | Toggle without deleting. |

> **Trust model:** `analytics-html`, `custom-css`, and `blocks[].html`
> are rendered unescaped. Only set them from a trusted source. Anything
> the snippet loads from outside Ruscker's origin must also be listed in
> the matching `*-origins` field, otherwise the landing's CSP blocks
> it.

`custom-css`, `logos[]`, and `blocks[]` are **admin-managed in the live
landing editor** (the logos picker and blocks editor live on the Portal
page). Blocks are stored in their own DB table; the `blocks[]` slot in
this YAML schema exists so a future `ruscker export` round-trip can
serialize them, but at the moment the import/export path does **not**
populate it — operators edit blocks from the admin UI. `logos[]` does
round-trip through import/export. SEO, analytics, and `show-admin-link`
are deploy-level policy and live only in this block.

## Specs

A spec describes one app, API, or external link. Every spec has an
`id` and lives in `proxy.specs[]`.

### Common fields (all spec types)

```yaml
- id: my_app                          # required, kebab-case
  display-name: "My App"              # shown on the card
  description: "What it does"         # HTML inline allowed
  template-properties:                # free-form bag for the landing
    logo: "/assets/img/myapp.png"
    icon: lock                        # 'lock' | 'lock_open'
    type: app                         # 'app' | 'package' | 'talk' | 'report' | 'api'
    updated: "18/05/2025"
    state: active                     # 'active' | 'inactive'
    link: https://external.example    # optional explicit URL
```

### Containerized specs (Shiny, Streamlit, Dash, Voilà, API)

```yaml
- id: my_app
  container-image: org/repo:tag       # required for containerized
  type: shiny                         # optional, default 'shiny' if image set
  container-port: 8501                # port the app listens on inside the
                                      #   container; default 3838 (Shiny).
                                      #   `type: streamlit|dash|voila`
                                      #   defaults to the framework's
                                      #   well-known port (8501 / 8050 /
                                      #   8866) when unset. ShinyProxy
                                      #   `port:` is accepted as an alias.
  seats-per-container: 10             # sessions per replica
  max-lifetime: 360                   # minutes — hard recycle (enforced, #334)
  container-lifetime: 360             # minutes — soft recycle when idle (enforced, #334)
  heartbeat-timeout: 3600000          # ms — per-spec override (enforced)
  stop-on-logout: false               # end a user's sessions on logout (enforced, #337)
  docker-registry-username: acme
  docker-registry-password: ${DOCKER_REGISTRY_PASSWORD}   # use env vars!
  docker-registry-domain: docker.io
  docker-registry-credential: dockerhub-acme   # OR reference a stored
                                      #   credential by name (Ruscker
                                      #   extension; see below)
  container-volumes:                  # bind mounts (ShinyProxy key; `volumes` also accepted)
    - /srv/myapp/data:/data           #   persistent data
    - /srv/myapp/www:/www:ro          #   static assets, read-only
  container-env:                      # env vars injected into the container
    DB_HOST: db.internal              #   (ShinyProxy-compatible)
    DB_PASSWORD: ${DB_PASSWORD}       #   ${VAR} resolved at spawn, not at parse —
                                      #   the literal is stored; secret never hits the DB
  container-cmd:                      # override the image's CMD (argv list)
    - R
    - -e
    - shiny::runApp('/app', port=3838, host='0.0.0.0')
  container-network: ruscker_net      # attach to this Docker network (created if missing)
  labels:                             # extra Docker labels stamped on the container
    team: data                        #   (ShinyProxy-compatible)
    cost-center: GAPE
  access-groups: [staff, ops]         # who may see/reach this app
  access-users: [alice]               #   (ShinyProxy-compatible)
```

`container-volumes` is a list of Docker bind specs (`/host:/container`,
optionally `:ro`/`:rw`), mapped to the container's `HostConfig.binds`.
This is ShinyProxy's key; the bare `volumes` is still accepted as an
alias for older Ruscker configs. Add as many as needed; editable in the
admin **Advanced** form (one per line).
**Bind-mounting host paths is root-equivalent and admin-only** — see
`SECURITY.md`.

`container-env` is a `NAME: value` map injected into the container as
environment variables (Docker `Config.Env`). `${VAR}` /
`${VAR:-default}` references in the values are **resolved at spawn**, not
at parse: the `${VAR}` literal is what gets stored (config / DB / export),
and the real value is only ever materialized when the container is
created — so an app secret passed this way never lands in the database.
`container-cmd`
is an argv list that overrides the image's baked `CMD` (Docker
`Config.Cmd`); omit it to keep the image default. Both are
ShinyProxy-compatible and editable per spec.

`container-network` (Ruscker extension) attaches the spec's containers to
a named Docker network, mapped to the container's `HostConfig.NetworkMode`.
The backend **creates the network** (a plain user-defined bridge) if it
doesn't exist yet, so you don't have to pre-create it. Omit it (or leave
it blank) to use the daemon's default bridge. Use it to isolate Ruscker's
app containers on their own L2 segment — the published port still binds on
loopback (`127.0.0.1`), so the proxy reaches the app exactly as before; the
network only segments app-to-app traffic. ShinyProxy's per-spec
`network-connections` list maps straight to this single field (Ruscker
attaches one network; multi-network attach is still deferred).

`labels` (ShinyProxy-compatible) is a `NAME: value` map of extra Docker
labels stamped on the container — useful for external tooling
(monitoring, log routing, cost attribution). Ruscker merges them onto the
container's labels at spawn; its own `ruscker.*` labels always win on a
key collision (they're load-bearing for the registry, reconcile and disk
panel), so you can't override them. Blank keys are dropped.

`access-groups` / `access-users` (ShinyProxy-compatible) scope who can
**see** an app on the landing and **reach** it at `/app` / `/api`. A spec
with neither is **open** — visible to everyone, including anonymous
visitors. Otherwise: a logged-in user sees it when their username is in
`access-users` or one of their groups is in `access-groups`; an admin
always sees everything; an anonymous visitor sees only open apps. Group
membership is set per user in the admin panel. Enforcement is real (not
just hiding the card). See the [Roadmap](ROADMAP.md) Phase 8.

#### Registry credentials — inline vs. named (`docker-registry-credential`)

A spec can authenticate to a private registry two ways:

1. **Inline** `docker-registry-username` / `docker-registry-password` /
   `docker-registry-domain` — ShinyProxy-compatible. Always use
   `${ENV_VAR}` for the password (never a literal in YAML).
2. **Named** `docker-registry-credential: <name>` — a **Ruscker
   extension** that references an entry in the admin's encrypted
   credential store (managed under **Credentials** in the admin panel).
   The username, password, and registry are resolved from the store at
   **pull time** (decrypted with `RUSCKER_MASTER_KEY`), so no secret —
   not even a `${VAR}` reference — needs to live in the spec at all.

When `docker-registry-credential` is set, it **takes precedence** over
the inline `docker-registry-*` fields for that spec. Leave it unset to
use the inline fields. The spec form's **Registry** section is a picker
over stored credential names.

### Smart routing — sub-path context for the upstream

Ruscker mounts each app under a sub-path (`/app/{id}`, `/api/{id}`) but
the container usually assumes it lives at the server root. Two
mechanisms bridge that gap:

1. **Forwarded-prefix headers (always on).** Every proxied request
   carries the public mount context to the upstream so a
   prefix-aware app can self-route and emit correct absolute links:

   | Header | Value | Consumed by |
   |---|---|---|
   | `X-Forwarded-Prefix` | `/app/{id}` (no trailing slash) | Spring, Traefik, FastAPI `root_path` |
   | `X-Script-Name` | same mount path | WSGI, Dash, Plumber |
   | `X-Forwarded-Proto` | `http` / `https` as seen by the client | absolute-URL builders behind TLS |
   | `X-Forwarded-Host` | the public `Host` | absolute-URL builders |

   Apps that ignore these headers are unaffected. To honour them you
   typically point the framework at the prefix — e.g. uvicorn
   `--root-path /app/my-api`, or Dash `requests_pathname_prefix`.

2. **HTML rewriting (`inject-base-href`, default `true`).** For apps
   that *can't* be told their prefix, Ruscker injects `<base href>`
   and rewrites root-relative URLs in `/app/{id}` HTML responses (see
   the rewriter in `routes::rewrite`). This is the safe default and
   covers Shiny out of the box. Set it to `false` per spec once the
   app self-routes from the headers above — then the rewriting is
   redundant and best disabled:

   ```yaml
   - id: my_api
     container-image: org/fastapi:tag
     type: api
     inject-base-href: false   # app reads X-Forwarded-Prefix itself
   ```

   `inject-base-href` only affects `/app/{id}` responses; `/api/{id}`
   responses are never rewritten. Editable in the admin **Advanced**
   form under **Routing**.

3. **The `#{publicPath}` token (opt-in).** For an app that needs its
   *own* base-url told to it as a config value — Jupyter's
   `--ServerApp.base_url`, for instance — drop the literal token
   `#{publicPath}` into any `container-cmd` argument or `container-env`
   value. At spawn it's substituted with the spec's public mount path
   **with a trailing slash** (e.g. `/app/{id}/`, or `/apps/app/{id}/`
   under a base path; `/api/{id}/` for APIs). This is Ruscker's analog
   of ShinyProxy's `SHINYPROXY_PUBLIC_PATH`, but it is **never injected
   automatically** — Ruscker strips the mount prefix before forwarding,
   so most apps should serve at root and rely on the rewriting above.
   Reach for the token only when an app genuinely needs the public path
   in its own configuration:

   ```yaml
   - id: jupyter
     container-image: org/jupyter:tag
     type: app
     container-cmd:
       - jupyter
       - lab
       - --ServerApp.base_url=#{publicPath}
   ```

   Note `#{publicPath}` is resolved at **spawn**, distinct from the
   parse-time `${VAR}` env interpolation, which never sees this runtime
   value.

### External link specs (no container)

```yaml
- id: my_pkg
  display-name: "My Package"
  description: "An R package"
  template-properties:
    type: package
    link: https://pkg.example          # the destination URL
    logo: "/assets/img/mypkg.png"
    icon: lock_open
    state: active
```

Just omit `container-image` and provide `template-properties.link`.
Ruscker won't try to orchestrate anything — clicking the card
navigates to the link.

### API specs (Plumber2 / FastAPI / etc.)

```yaml
- id: my_api
  type: api                            # explicit type, overrides auto-detect
  container-image: org/my-api:latest
  api:
    port: 8080                         # container port
    docs-path: /__docs__               # OpenAPI/Swagger UI
    health-path: /__healthz__          # readiness check
    rate-limit: 100/min                # per-IP rate limit at proxy
    cors: true                         # permissive CORS headers
  min-replicas: 1
  max-replicas: 3
  concurrent-requests-per-replica: 100
  routing-strategy: round-robin        # APIs don't need sticky
```

#### `api.rate-limit` — per-client throttling

Enforced at the proxy, **before** any container is spawned or woken,
so a throttled caller costs nothing downstream. Format is
`N/unit` where `unit` is one of `s`/`sec`/`second(s)`,
`m`/`min`/`minute(s)`, or `h`/`hr`/`hour(s)` (case-insensitive):

```yaml
rate-limit: 100/min      # at most 100 requests per client per minute
rate-limit: 5/s
rate-limit: 1000/hour
```

A request over the limit gets `429 Too Many Requests` with a
`Retry-After` header. The window is a sliding one, per
`(spec, client)`.

**Client identity.** The "client" is the caller's IP. When the
operator opts into forwarded headers
(`server.useForwardHeaders: true`, or a `forward-headers-strategy`
other than `none`), the **right-most** parseable `X-Forwarded-For`
address is used — the entry appended by the trusted proxy, which a
client can't spoof (the left-most slot is client-controlled).
Otherwise the real TCP peer is used: `X-Forwarded-For` is **not**
trusted unless opted in, since a direct client could otherwise spoof
it to dodge the limit.

A malformed `rate-limit` value is ignored (no limit applied) and
flagged by `ruscker validate`.

#### `api.cors` — permissive CORS headers

`cors: true` makes the proxy add permissive CORS headers
(`Access-Control-Allow-Origin: *`, common methods, `*` headers) to
every response for that API spec, and answer `OPTIONS` preflight
requests itself (`204`) without touching the container. Headers an
upstream app already set are never overwritten — an API that does
its own CORS wins. CORS applies only to the `/api/` route family.

### `max-body-size` — cap proxied request bodies

Limits how large a request body the proxy will forward, for both
`/app/` and `/api/` routes. Set it globally on `proxy.max-body-size`
and/or override it per spec:

```yaml
proxy:
  max-body-size: 10m          # global default
  specs:
    - id: upload_api
      container-image: org/api:1
      type: api
      max-body-size: 100m     # this spec accepts larger uploads
```

Format is the Docker-style size string used elsewhere (`"512"` bytes,
`"10m"`, `"1g"`; binary units). The effective limit is the spec's own
value if set, otherwise the global default; unset everywhere means **no
limit** (the default, preserving prior behaviour).

A request whose `Content-Length` exceeds the limit is rejected with
`413 Payload Too Large` before any container is touched. A chunked or
under-declared body that grows past the cap mid-stream is also stopped
(it surfaces as a `502`). A malformed size string is ignored (no limit
applied) and flagged by `ruscker validate`.

### Load-balancing fields (any containerized spec)

| Field | Type | Default | Notes |
|---|---|---|---|
| `min-replicas` | u32 | `0` | Always-warm replicas. Default `0` = **cold-start**: no container runs until the first visitor (they see a brief splash), reaped when idle — like ShinyProxy. Set `1`+ to keep an app hot (e.g. a single-user RStudio/Jupyter you don't want cold) |
| `max-replicas` | u32 | `5` (≥ `min-replicas`) | Per-container apps auto-scale to up to 5 independent replicas by default. With `min-replicas` unset/`0` the app still scales up to 5 on demand (cold-start); set explicitly to raise/lower |
| `scale-up-threshold` | float | unset | scale up when pool utilization exceeds this; unset ⇒ the built-in saturation rule (enforced, #333) |
| `scale-down-threshold` | float | unset | only retire idle replicas while utilization is below this; unset ⇒ the built-in idle rule (enforced, #333) |
| `scale-down-grace` | s | unset | idle-grace before retiring a replica; unset ⇒ the global ~30 s grace (enforced, #333) |
| `scale-down-cooldown-secs` | s | `60` | suppress saturation-driven scale-up after this app scales down, preventing immediate respawn flaps; `0` disables. Ruscker extension (#936) |
| `drain-timeout` | s | `60` | grace for in-flight sessions on a `max-lifetime` recycle (enforced, #335) |
| `routing-strategy` | enum | varies | See below |
| `concurrent-requests-per-replica` | u32 | `100` | API-only — per-replica in-flight cap the scaler scales on (enforced, #336) |

> **Autoscaling knobs (#326).** By default the scaler scales on **seat
> saturation** (`sessions_active` vs `sessions_max`) with built-in grace
> ticks. All the per-spec scaling/lifecycle knobs are now enforced
> (opt-in where noted): `scale-up-threshold` / `scale-down-threshold` /
> `scale-down-grace` (#333 — pool-utilization-driven scale-up + a
> conservative scale-down gate + per-spec idle grace; unset ⇒ the
> default rules), `max-lifetime` / `container-lifetime` (#334 — recycle
> past the age cap), `drain-timeout` (#335 — grace for a busy
> `max-lifetime` recycle), `stop-on-logout` (#337 — a signed-in user's
> sticky sessions end immediately on logout), and
> `concurrent-requests-per-replica` (#336 — API specs have no sticky
> sessions, so the scaler meters their capacity by in-flight requests
> against this per-replica cap). `ruscker validate --strict-compat` no
> longer flags any of them.

### Routing strategies

- `least-connections` — pick replica with most free seats. Default
  for Shiny, Streamlit, Dash, Voilà.
- `round-robin` — cycle through replicas. Default for API.
- `weighted-random` — random with weights = remaining seats. Not yet
  implemented (falls back to least-connections).
- `resource-aware` — pick based on CPU/mem load. Requires phase 4
  metrics. Falls back to least-connections.

### Spec kind dispatch

The effective kind drives runtime behavior:

- **Explicit `type` field** wins if set
- Otherwise: `container-image` set → `shiny`, unset → `external`

The kind controls:
- Whether sticky session cookies are issued (`shiny`, `streamlit`,
  `dash`, `voila` — yes; `api`, `external` — no)
- Default routing strategy
- Default `seats-per-container` (10 for web-framework apps — Shiny,
  Streamlit, Dash, Voilà — which serve many sessions per process; 100 for
  API; 0 for external). A single-user IDE (RStudio, Jupyter) should set
  `seats-per-container: 1` so each visitor gets an isolated container.
- Whether WebSocket forwarding is attempted

## Environment variable interpolation

Any string value can use `${VAR_NAME}` or `${VAR_NAME:-default}`:

```yaml
docker-registry-password: ${DOCKER_REGISTRY_PASSWORD}
heartbeat-rate: ${HEARTBEAT_RATE:-10000}
```

Rules:

- Variable names: `[A-Za-z_][A-Za-z0-9_]*` (lower/mixed case accepted)
- Missing variable without default: hard error at parse time
- Missing variable with default: substituted with the default
- Comments (lines starting with `#`, or trailing `# …`) are not
  interpolated
- A **nested** reference in a default (`${A:-${B}}`) is refused with a
  hard error (v0.2.5) — it used to silently corrupt the value
- `docker-registry-password` and everything under `container-env` are
  **preserved verbatim** and resolved only at use (spawn/pull), so
  secrets never land in the database on `import`. This holds wherever
  `container-env` appears in the spec — including as the first key of
  a list item (fixed in v0.2.5)

This applies to the whole YAML file, not just credentials. Use it for
any value that varies between environments.

## `template-properties`

Free-form key-value bag. The current landing template uses:

| Key | Type | Notes |
|---|---|---|
| `logo` | string | Path or URL to card image |
| `cover` | string (CSS) | Card-cover background — a solid color or gradient. Empty ⇒ a per-kind tint |
| `icon` | `lock` \| `lock_open` | Access level |
| `type` | `app` \| `package` \| `talk` \| `report` \| `api` | Badge category |
| `subject` | string | Subject/topic of the app — drives the **Subject** filter facet on the landing |

> **`featured`** (a top-level spec field, not a template-property): set
> `featured: true` to highlight the app in the landing's **Featured**
> carousel above the filters. The carousel shows only when
> `landing-customization.show-highlights` is on (the default) and at least
> one spec is featured. Default `false`.
| `updated` | string | Display date (DD/MM/YYYY) |
| `state` | `active` \| `inactive` | Whether to enable card |
| `link` | URL | External URL for non-container specs |

You can add custom keys — they're ignored unless the template uses
them. Useful for future custom templates.

## `logging` block

```yaml
logging:
  file:
    name: logs/ruscker.log
```

Accepted for ShinyProxy compat. Ruscker uses `tracing` for logging
and respects the `RUST_LOG` env var as well.

## Validation warnings

`ruscker validate <config>` — and, since v0.2.5, `ruscker serve` at
startup — reports these non-fatal findings. None of them stops the
server; each one means some configured intent is **not** taking
effect, so treat warnings in production logs as action items.

| Warning | Meaning |
|---|---|
| duplicate spec id | Two specs share an `id`; only the last parsed wins |
| no display-name / no description | Cosmetic: the landing card falls back to the `id` / renders empty |
| embedded credential | A sensitive field (`docker-registry-password`, …) carries a literal value instead of a pure `${VAR}` reference — including a partial one like `${VAR}-suffix` (v0.2.5) |
| unknown template-properties type | `template-properties.type` isn't one of the known card types |
| invalid replica range | `max-replicas < min-replicas` — the scaler can't satisfy both |
| replica ceiling zero (v0.2.5) | Explicit `max-replicas: 0` on a containerized spec — every spawn is refused; the app can never start |
| missing container-image (v0.2.5) | A containerized `type:` with no `container-image` — fails only when first visited |
| external with container-image (v0.2.5) | `type: external` + `container-image` — the image is silently ignored |
| invalid scale threshold | `scale-up`/`scale-down` thresholds inverted or out of `0..1` |
| container fields without image | `seats-per-container` etc. on a spec with no image |
| invalid rate-limit / max-body-size / cpu / memory / volume | The value doesn't parse, so the intended cap or mount is **not enforced** |
| invalid label key | A `labels` key uses characters outside `[A-Za-z0-9._-]` — Docker may reject the container create at spawn |
| reserved label key | A `labels` key in Ruscker's `ruscker.*` namespace — the backend stamps those itself and overrides the value; rename it |
| invalid container-network | `container-network` isn't a valid Docker network name (`[a-zA-Z0-9][a-zA-Z0-9_.-]*`) — the create fails at spawn |
| zero seats | `seats-per-container: 0` confuses the auto-scaler |
| invalid docker host | A `proxy.hosts` entry that will fail to connect at startup |
| ignored compat field (v0.2.5) | A modeled ShinyProxy field with no runtime effect is set (`server.secure-cookies`, `server.servlet.session.timeout`, `proxy.heartbeat-rate`, `proxy.hide-navbar`, `proxy.landing-page`, `proxy.container-log-path`, `logging.file`) |

`ruscker validate --strict-compat` additionally lists every
*unsupported* ShinyProxy feature a migrated config uses and exits
non-zero — the recommended pre-flight when migrating.

## Not supported (MVP)

These ShinyProxy fields are accepted by the parser but currently
ignored:

- `proxy.specs[*].kubernetes-*` — Kubernetes backend (out of scope)
- `proxy.specs[*].minimum-seats-available` — pre-warm pool (planned)
- `proxy.specs[*].network-connections` — multi-network attach (phase
  3.5); map it to the single `container-network` field, which Ruscker
  creates + attaches
- `proxy.docker.*` — global docker config (use defaults or env vars)

(`proxy.specs[*].volumes`, `container-env` / `container-cmd`,
`container-network` and `labels` are now supported — see "Containerized
specs" above.)

Setting any of these will produce a startup warning but not an error
(`serve` runs the same validation `ruscker validate` does and logs
each finding at startup). The same applies to fields that *parse* but
have no runtime effect yet: `server.secure-cookies`,
`server.servlet.session.timeout`, `proxy.heartbeat-rate`,
`proxy.hide-navbar`, `proxy.landing-page`, `proxy.container-log-path`
and `logging.file` each produce a warning when set, so a migrated
config never silently loses configured behaviour.
Run `ruscker validate --strict-compat <config>` to list every
unsupported feature a config uses (and exit non-zero if any are
found) — the recommended pre-flight check when migrating from
ShinyProxy.

For a field-by-field ShinyProxy → Ruscker reference — every documented
ShinyProxy 3.x key with its status here (supported /
warned-and-ignored / planned / out of scope) and the Ruscker
equivalent where one exists — see the **ShinyProxy → Ruscker field
map** page on the docs site (`book/src/shinyproxy-fieldmap.md`).
