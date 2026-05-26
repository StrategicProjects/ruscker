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
  useForwardHeaders: true             # Honor X-Forwarded-* headers
  forward-headers-strategy: native    # 'native' | 'framework' | 'none'
  secure-cookies: true                # Set Secure flag on cookies
  servlet.session.timeout: 3600       # Spring flat-key form
  # OR equivalently:
  servlet:
    session:
      timeout: 3600
```

Resolved via `Server::session_timeout_secs()` — either form works.

Other `server.*` fields from Spring Boot are accepted by serde but
ignored by Ruscker.

## `proxy` block

### Top-level proxy fields

| Field | Type | Default | Notes |
|---|---|---|---|
| `title` | string | `"Ruscker"` | Browser tab title |
| `landing-page` | string | `"/"` | Where the portal is served |
| `hide-navbar` | bool | `false` | Suppress default navbar |
| `template-path` | path | none | Override template directory |
| `heartbeat-rate` | ms | `10000` | Client heartbeat interval |
| `heartbeat-timeout` | ms | `3600000` | Session expiry; `-1` = never |
| `container-wait-time` | ms | `60000` | Max wait for container Ready |
| `shutdown-grace-ms` | ms | `30000` | Drain window on SIGTERM/Ctrl-C before forced exit; `/readyz` reports `draining` during it. Ruscker extension |
| `max-body-size` | size | none | Global cap on proxied request bodies (`"10m"`, `"1g"`, bytes); over → `413`. Per-spec `max-body-size` overrides. Ruscker extension |
| `container-log-path` | path | none | Directory for per-container logs |
| `port` | u16 | `8080` | HTTP listener port |
| `bind-address` | string | `"0.0.0.0"` | Listener interface |
| `authentication` | enum | `none` | `none` (MVP) / `openid` / `ldap` / `saml` / `simple` |
| `specs` | array | `[]` | List of apps/links/APIs |

### Authentication

**MVP only supports `none`.** The other variants are accepted by the
parser (so existing YAML doesn't fail) but Ruscker will warn at boot
that auth is unimplemented and continue as if `none`. Phase 8 adds
real auth support.

For applications that handle their own auth internally (a common
case), `none` is the correct choice — Ruscker just routes traffic.

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
  seats-per-container: 10             # sessions per replica
  max-lifetime: 360                   # minutes — hard cap
  container-lifetime: 360             # minutes — soft cap
  heartbeat-timeout: 3600000          # ms — per-spec override
  stop-on-logout: false               # auth-related
  docker-registry-username: milkway
  docker-registry-password: ${DOCKER_REGISTRY_PASSWORD}   # use env vars!
  docker-registry-domain: docker.io
```

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
other than `none`), the left-most `X-Forwarded-For` address is used
— the right choice when Ruscker sits behind a reverse proxy.
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
| `min-replicas` | u32 | `1` | Always running |
| `max-replicas` | u32 | = `min-replicas` | Set higher to enable auto-scale |
| `scale-up-threshold` | float | `0.8` | Spawn when utilization > this |
| `scale-down-threshold` | float | `0.3` | Retire when < this for grace |
| `scale-down-grace` | s | `300` | Seconds below threshold before retiring |
| `drain-timeout` | s | `60` | Seconds to wait for sessions to end |
| `routing-strategy` | enum | varies | See below |
| `concurrent-requests-per-replica` | u32 | `100` | API-only |

### Routing strategies

- `least-connections` — pick replica with most free seats. Default
  for Shiny, Streamlit, Dash, Voilà.
- `round-robin` — cycle through replicas. Default for API.
- `weighted-random` — random with weights = remaining seats. Not yet
  implemented (falls back to round-robin).
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
- Default `seats-per-container` (1 for interactive, 100 for API,
  0 for external)
- Whether WebSocket forwarding is attempted

## Environment variable interpolation

Any string value can use `${VAR_NAME}` or `${VAR_NAME:-default}`:

```yaml
docker-registry-password: ${DOCKER_REGISTRY_PASSWORD}
heartbeat-rate: ${HEARTBEAT_RATE:-10000}
```

Rules:

- Variable names: `[A-Z_][A-Z0-9_]*`
- Missing variable without default: hard error at parse time
- Missing variable with default: substituted with the default
- Comments (lines starting with `#`) are not interpolated

This applies to the whole YAML file, not just credentials. Use it for
any value that varies between environments.

## `template-properties`

Free-form key-value bag. The current landing template uses:

| Key | Type | Notes |
|---|---|---|
| `logo` | string | Path or URL to card image |
| `icon` | `lock` \| `lock_open` | Access level |
| `type` | `app` \| `package` \| `talk` \| `report` \| `api` | Badge category |
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

## Not supported (MVP)

These ShinyProxy fields are accepted by the parser but currently
ignored:

- `proxy.specs[*].kubernetes-*` — Kubernetes backend, phase 6
- `proxy.specs[*].port` — explicit upstream port (Ruscker uses
  `api.port` for APIs, auto-detects for Shiny)
- `proxy.specs[*].minimum-seats-available` — pre-warm pool (planned)
- `proxy.specs[*].labels`, `proxy.specs[*].network-connections` — phase
  3.5
- `proxy.specs[*].volumes`, `proxy.specs[*].environment` — phase 3
- `proxy.docker.*` — global docker config (use defaults or env vars)

Setting any of these will produce a startup warning but not an error.
Run `ruscker validate --strict-compat <config>` to list every
unsupported feature a config uses (and exit non-zero if any are
found) — the recommended pre-flight check when migrating from
ShinyProxy.
