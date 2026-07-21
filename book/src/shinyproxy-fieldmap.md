# ShinyProxy → Ruscker field map

A field-by-field reference for migrating a ShinyProxy 3.x
`application.yml` to Ruscker. For the step-by-step migration guide, see
[Migrating from ShinyProxy](./migrating.md); this page answers the
narrower question *"what happens to each key in my config?"*

> Context: in Ruscker your `application.yml` is the **import format**
> (`ruscker import` brings the specs into the database); the service's
> own config is `ruscker.yml` — same schema. See
> [Configuration](./configuration.md) for the four-layer model.

Statuses:

| | Meaning |
|---|---|
| ✅ | **Supported** — Ruscker honours the key (same name unless noted) |
| ⚠️ | **Accepted but ignored** — parses fine, does nothing; Ruscker **warns at startup** when it's set |
| 🚧 | **Planned** — not implemented yet; tracked on the roadmap / an issue |
| ❌ | **Not planned** — out of scope, with the reasoning and (where one exists) the Ruscker way to get the same outcome |

> **How to check *your* config:** run
> `ruscker validate application.yml --strict-compat`. It flags a
> non-`none` `proxy.authentication`, the whole `proxy.docker.*` block,
> and the per-spec `kubernetes-*` / `minimum-seats-available` /
> `network-connections` keys, and exits non-zero if any are present.
> The ⚠️ rows below additionally warn at every startup. **Any other
> unknown key is dropped silently by the parser** — that's standard
> serde behaviour, and it's exactly why this table exists.

## `server.*` and Spring-level keys

| ShinyProxy key | Status | In Ruscker |
|---|---|---|
| `server.servlet.context-path` | ✅ | Same key, or the flat `server.context-path`; the `--base-path` CLI flag overrides both. See [Configuration](./configuration.md) |
| `server.forward-headers-strategy` | ✅ | Same key — any value other than `none` trusts `X-Forwarded-*`. The pre-3.x `server.useForwardHeaders: true` is also accepted. **Set one of them whenever a reverse proxy terminates TLS** |
| `server.secure-cookies` | ⚠️ | Warned and ignored. The `Secure` cookie flag comes from the forwarded-headers trust above + your proxy's `X-Forwarded-Proto` |
| `server.servlet.session.timeout` | ⚠️ | Warned and ignored — admin sessions have a fixed 24 h TTL |
| `server.compression.*` | ❌ | Not needed: Ruscker always compresses responses (gzip/brotli built in) |
| `server.tomcat.accesslog.*` | ❌ | Tomcat-specific. Use structured logging (`--log-format json`) and your process manager's log routing |
| `logging.file` / `logging.file.name` | ⚠️ | `logging.file` is warned and ignored — Ruscker logs to stdout/stderr; let systemd/journald or Docker capture it |
| `logging.level.*`, `logging.pattern.*`, `logging.logback.*`, `logging.json.*` | ❌ | Spring/Logback-specific. Use `-v`/`-vv` for verbosity and `--log-format json` for JSON logs |
| `spring.session.store-type`, `spring.redis.*` | ❌ | Ruscker's HA story uses **Postgres**, not Redis: `--config-db-url`, `--session-store-url`, `--admin-session-store-url`. See [Deploying](./deploying.md) |
| `management.prometheus.metrics.export.enabled` | ✅ | Equivalent: `proxy.metrics-enabled: true` exposes a Prometheus `/metrics` endpoint (unauthenticated — firewall it) |
| `spring.application.name` | ❌ | Cosmetic; `proxy.title` names the portal |

## `proxy.*` top-level keys

| ShinyProxy key | Status | In Ruscker |
|---|---|---|
| `proxy.title` | ✅ | Same key (browser-tab / portal title) |
| `proxy.port` | ✅ | Same key |
| `proxy.bind-address` | ✅ | Same key |
| `proxy.heartbeat-timeout` | ✅ | Same key (ms; `-1` = never); per-spec override supported |
| `proxy.container-wait-time` | ✅ | Same key (ms) — max wait for a spawned container to become ready. Note the default differs: ShinyProxy 20 s, Ruscker 60 s |
| `proxy.template-path` | ✅ | Read for one purpose: auto-discovering card logos in `<template-path>/assets/img/` next to the config. Thymeleaf templates themselves don't apply — the portal UI is Ruscker's own, themed in the admin **Appearance** tab |
| `proxy.specs` | ✅ | Same key — see the per-spec table below |
| `proxy.authentication` | ✅/🚧 | Parsed; **only `none` is implemented today** (apps that do their own auth are the common case). `openid` / `saml` are the Phase 8 roadmap (issue #934). Anything non-`none` is flagged by `--strict-compat` and warned at boot |
| `proxy.admin-groups`, `proxy.admin-users`, `proxy.users` | ❌ | Ruscker has its own user store with roles (Admin / Editor / Viewer), managed in the admin **Users** tab (CSV import available) — not driven from YAML |
| `proxy.heartbeat-rate` | ⚠️ | Warned and ignored — the landing's heartbeat is fixed |
| `proxy.landing-page` | ⚠️ | Warned and ignored — the portal is always at the root / base path |
| `proxy.hide-navbar` | ⚠️ | Warned and ignored |
| `proxy.container-log-path` | ⚠️ | Warned and ignored — use `docker logs`, or the admin **Logs** tab |
| `proxy.container-log-storage`, `proxy.s3-log-*` | ❌ | No app-log shipping; capture container logs with Docker's own log drivers |
| `proxy.container-backend` | ❌ | Docker only (local daemon by default; several daemons via the `proxy.hosts` extension). Kubernetes/Swarm/ECS are out of scope |
| `proxy.container-wait-timeout` | ❌ | Single readiness knob: `container-wait-time` |
| `proxy.stop-proxies-on-shutdown` | ✅ | Built-in behaviour, not a flag: Ruscker **never stops app containers on shutdown** (equivalent to `false`) |
| `proxy.recover-running-proxies` (+ `…-from-different-config`) | ✅ | Built-in: at boot Ruscker reconciles running containers it owns (label-scoped) back into the registry — no Redis, no flag |
| `proxy.default-stop-proxy-on-logout` | ✅ | Per-spec `stop-on-logout` (no global default — set it on the specs that need it) |
| `proxy.default-proxy-max-lifetime` | ✅ | Per-spec `max-lifetime` (no global default) |
| `proxy.default-max-instances`, `proxy.max-total-instances`, per-spec `max-instances` | ❌ | Different concurrency model: ShinyProxy counts **instances per user**; Ruscker pools **seats × replicas** (`seats-per-container`, `min-replicas` / `max-replicas`). See [Configuration § load balancing](./configuration.md) |
| `proxy.seat-wait-time` | ❌ | No seat queue — a cold-start visitor sees a splash while the replica spawns; a saturated pool scales up to `max-replicas` |
| `proxy.default-webSocket-reconnection-mode` | ❌ | Ruscker pumps WebSockets transparently and leaves reconnection to the app framework |
| `proxy.default-cache-headers-mode` | ❌ | App responses pass through untouched |
| `proxy.default-max-session-time` | ❌ | Sessions expire by `heartbeat-timeout` (idle), not wall-clock |
| `proxy.notification-message` | ✅ | Equivalent: the landing **intro text** and custom HTML blocks, edited in the admin **Appearance** tab |
| `proxy.username-case-sensitive` | ❌ | Usernames are normalized (lowercase) throughout |
| `proxy.secure-cookies`, `proxy.same-site-cookie` | ❌ | Cookie flags are managed by Ruscker (`Secure` via forwarded-header trust; sticky cookies are per-spec, `SameSite` set appropriately) |
| `proxy.usage-stats-*`, `proxy.usage-stats` | ❌ | No InfluxDB/JDBC usage-stats pipeline. Built in instead: a per-spec **access counter** with sparklines in the admin Apps table, the live dashboard, and the optional Prometheus `/metrics` endpoint |
| `proxy.monitoring.grafana-url` | ❌ | The monitoring dashboard is built in; scrape `/metrics` for Grafana |
| `proxy.enable-app-persistence` | ❌ | Not needed: containers keep running across Ruscker restarts and are reconciled at boot |
| `proxy.logo-url`, `proxy.favicon-path`, `proxy.template-groups`, `proxy.body-classes`, `proxy.my-apps-mode` | ❌ | Thymeleaf-template UI knobs. The equivalents live in the admin **Appearance** tab (header logos, presets, catalog layout); the catalog groups cards by app type automatically |
| `proxy.ldap.*`, `proxy.openid.*`, `proxy.saml.*`, `proxy.ms-graph.*`, `proxy.webservice.*`, `proxy.custom-header.*` | 🚧/❌ | Provider config for the unimplemented auth schemes. OIDC/SAML config will come with Phase 8 (#934); LDAP / webservice / customHeader have no plans |
| `proxy.docker.*` (url, cert-path, port-range, internal-networking, image-pull-policy, …) | ❌ | Flagged by `--strict-compat`. Ruscker talks to the daemon via the standard socket/env (`DOCKER_HOST`); published ports bind on `127.0.0.1` by design; re-pull is a button in the admin. Multiple daemons: the `proxy.hosts` extension |
| `proxy.kubernetes.*`, `proxy.ecs.*` | ❌ | Kubernetes / ECS backends are out of scope |

## Per-spec keys (`proxy.specs[]`)

| ShinyProxy key | Status | In Ruscker |
|---|---|---|
| `id` | ✅ | Same key (also the `/app/{id}` URL segment) |
| `display-name` | ✅ | Same key |
| `description` | ✅ | Same key (inline HTML allowed) |
| `container-image` | ✅ | Same key |
| `port` | ✅ | Accepted as an alias of `container-port`. Unset, `type: streamlit\|dash\|voila` default to 8501 / 8050 / 8866; Shiny to 3838 |
| `container-cmd` | ✅ | Same key (argv list overriding the image `CMD`) |
| `container-env` | ✅ | Same key; `${VAR}` values resolve at spawn and never land in the DB |
| `container-volumes` | ✅ | Same key (bare `volumes` also accepted). Bind mounts are admin-only — see `SECURITY.md` |
| `container-network` | ✅ | Same key; Ruscker also **creates** the network if missing |
| `labels` | ✅ | Same key; Ruscker's own `ruscker.*` labels win on collision |
| `container-memory-request` / `container-memory-limit` | ✅ | Same keys |
| `container-cpu-request` / `container-cpu-limit` | ✅ | Same keys |
| `access-groups` / `access-users` | ✅ | Same keys — enforced at `/app`–`/api` and on landing visibility, against Ruscker's own user store |
| `heartbeat-timeout` | ✅ | Same key (per-spec override, ms) |
| `stop-on-logout` | ✅ | Same key |
| `max-lifetime` | ✅ | Same key (minutes, hard recycle; in-flight sessions get `drain-timeout`) |
| `seats-per-container` | ✅ | Same key — but **defaults differ**: ShinyProxy defaults to 1; Ruscker to 10 for web apps (Shiny/Streamlit/Dash/Voilà) and 100 for APIs. Single-user IDEs (RStudio, Jupyter) should set `1` explicitly |
| `docker-registry-username` / `-password` / `-domain` | ✅ | Same keys. Use `${ENV_VAR}` for the password; or the named-credential extension `docker-registry-credential` |
| `template-properties` | ✅ | Same key — Ruscker reads `logo`, `type`, `state`, `link`, and its own extensions (`locked`, `accent`, `monogram`) |
| `minimum-seats-available` | 🚧 | Flagged by `--strict-compat`. Pre-warm pool not implemented — `min-replicas: 1` keeps an app warm today |
| `container-network-connections` / `network-connections` | ❌ | Flagged by `--strict-compat`. Multi-network attach is deferred — map it to the single `container-network` |
| `kubernetes-*` (pod-patches, manifests, probes, …) | ❌ | Flagged by `--strict-compat`. Kubernetes backend is out of scope |
| `ecs-*` | ❌ | ECS backend is out of scope |
| `container-privileged`, `docker-user`, `docker-ipc`, `docker-group-add`, `docker-runtime`, `docker-device-requests`, `resource-name`, `container-dns`, `additional-port-mappings` | ❌ | Host-level container knobs, deliberately not exposed (privileged containers are root-equivalent; see `SECURITY.md`) |
| `container-env-file` | ❌ | Use `container-env` with `${VAR}` references instead — same secrecy, no file to mount |
| `target-path` | ❌ | Ruscker mounts every app at `/app/{id}/` and rewrites URLs itself (see [Architecture](./architecture.md)); apps that self-route get the forwarded-prefix headers |
| `access-expression`, `access-strict-expression` | ❌ | SpEL is JVM-specific; use `access-groups` / `access-users` |
| `max-instances`, `max-total-instances`, `always-show-switch-instance`, `allow-container-re-use` | ❌ | Per-user-instance model — see the concurrency note in the proxy table above |
| `scale-down-delay` | ✅ | Equivalents: `scale-down-grace` (idle grace before retiring a replica) and `scale-down-cooldown-secs` (anti-flap) |
| `websocket-reconnection-mode`, `shiny-force-full-reload`, `track-app-url` | ❌ | WebSockets are pumped transparently; reconnection/reload is the app framework's business |
| `add-default-http-headers` | ⚠️ | Supported for `X-SP-UserId` / `X-SP-UserGroups`, but Ruscker defaults it **off** (ShinyProxy defaults it on). Enable it explicitly per trusted spec |
| `http-headers` | ❌ | Static custom headers remain unsupported; Ruscker forwards the standard `X-Forwarded-*` family and its sub-path context headers |
| `cache-headers-mode` | ❌ | App responses pass through untouched |
| `logo-url`, `logo-height/-width/-classes/-style`, `favicon-path` | ❌ | Card art comes from `template-properties.logo` (the `/assets/img/<file>` convention — `ruscker import --images-dir` ingests the files) or the admin **Media** picker; sizing is the card design's |
| `template-group` | ❌ | The catalog groups cards by app **type** automatically (`template-properties.type`) |
| `custom-app-details`, `hide-navbar-on-main-page-link`, `support-mail-to-address`, `support-mail-subject` | ❌ | Thymeleaf-template features with no Ruscker counterpart |
| `parameters` (app parameters) | ❌ | No parameterized-launch form; make variants explicit as separate specs with different `container-env` |
| `external-url` | ✅ | Equivalent: an **external link spec** — omit `container-image` and set `template-properties.link` |

## Ruscker extensions (no ShinyProxy counterpart)

- `identity-claims` opts a trusted spec in to selected additional profile
  headers (`email` and/or `setor`); it is independent of
  `add-default-http-headers` and defaults to none.

These are additions, not compat concerns — listed so a reviewed config
is fully accounted for. Details in the
[YAML schema reference](./configuration.md): per-spec `type`,
`min-replicas` / `max-replicas`, `scale-up-threshold` /
`scale-down-threshold` / `scale-down-grace` / `scale-down-cooldown-secs`,
`drain-timeout`, `routing-strategy`,
`concurrent-requests-per-replica`, `container-lifetime`, `platform`,
per-spec `container-wait-time` (the ShinyProxy counterpart is global-only),
`inject-base-href`, `max-body-size` (global + per-spec),
`docker-registry-credential`, `placement` / `anti-affinity`, the
`api.*` block (`docs-path`, `health-path`, `rate-limit`, `cors`),
`proxy.hosts` (multi-host), `proxy.landing-customization`,
`proxy.metrics-enabled` / `metrics-interval`, `proxy.shutdown-grace-ms`
and `proxy.csp-origins`.

## Keeping this page honest

This table is a living contract. When a field's status changes
(a 🚧 ships, a ⚠️ gains a consumer), the PR that changes the behaviour
should update this page, `docs/YAML_SCHEMA.md`, and — when the
detection surface changes — the `--strict-compat` scan
(`ruscker-config::validate`). The sources of truth, in order:
`crates/ruscker-config/src/schema.rs` (what parses),
`validate.rs`'s `check_ignored_compat_fields` (the ⚠️ set) and
`compat_scan` (what `--strict-compat` flags), and
`docs/YAML_SCHEMA.md` § *Not supported*.
