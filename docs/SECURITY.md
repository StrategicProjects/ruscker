# Ruscker — Security & Threat Model

Status: living document. Tracks the Phase 5 security audit
(issue #14). Each control is marked **[implemented]**,
**[accepted limitation]**, or **[deferred]**. File references use
`crate/path:symbol` so they survive line-number drift.

> **Scope.** A single-operator install: Ruscker behind a
> TLS-terminating reverse proxy, Docker on one or more hosts. The model
> below reflects the hardening done in the 2026-06 security audit.
> Multi-tenant / shared-team auth (OIDC, SAML) is out of scope until
> Phase 8.

---

## 1. Threat model

### Assets

| Asset | Why it matters |
|-------|----------------|
| `RUSCKER_ADMIN_TOKEN` | break-glass Admin login + first-account bootstrap — full access |
| User passwords (DB) | per-user login; stored as argon2id hashes; policy on set/reset: ≥ 8 chars with upper + lower + digit + special |
| `RUSCKER_MASTER_KEY` | decrypts the registry-credential store |
| `RUSCKER_COOKIE_KEY` | forges sticky-session cookies |
| Registry credentials (DB) | pull access to private images |
| Running app sessions | per-visitor app state inside containers |
| The Docker daemon | full host compromise if reachable |

### Attackers

- **Network attacker** — can reach the bound port. Mitigated by
  binding to localhost / private network + reverse proxy.
- **Malicious visitor** — hits `/app/*` / `/api/*` without admin
  rights. Should never reach admin surfaces or other visitors'
  sessions.
- **Curious operator-adjacent user** — has some network access,
  tries to brute-force the admin token or forge cookies.
- **Compromised app image** — a container Ruscker spawned that
  tries to escape its limits or reach the host/other containers.

### Non-goals (explicitly out of scope for MVP)

- Defending against a hostile operator (they own the host +
  Docker daemon + all keys).
- Per-app ACLs and external identity providers (OIDC/SAML/LDAP).
  Coarse RBAC (Viewer/Editor/Admin) exists (§2); fine-grained,
  per-spec authorization is Phase 8.
- Isolating **untrusted, third-party app code** from the admin on a
  shared origin. Apps are assumed first-party; multi-tenant hosting of
  hostile apps needs origin separation (§6.1, roadmap #878).
- TLS termination (delegated to the reverse proxy).

---

## 2. Authentication & authorization

- **[implemented]** User accounts (#107) — per-user login
  (`username` + `password`) backed by the `users` table; passwords
  stored only as **argon2id** PHC hashes (`db::users`, never
  plaintext). `verify_login` runs a decoy hash on unknown usernames
  so timing doesn't reveal whether an account exists. Roles
  (`viewer`/`editor`/`admin`) are per-user.
- **[implemented]** Break-glass admin token —
  `auth::AdminAuth::matches_token` → `ct_eq` (XOR-fold, length-
  checked; time depends only on the public length). `RUSCKER_ADMIN_TOKEN`
  always grants an Admin session and **bootstraps the first account**
  (token login on a fresh install → forced setup). It's the recovery
  path so an operator can never be locked out; treat it as a
  break-glass secret. The old `RUSCKER_EDITOR_TOKEN`/`RUSCKER_VIEWER_TOKEN`
  (the #101 MVP) are **removed** — Editor/Viewer are DB accounts now.
- **[implemented]** Login rate limiting —
  `auth::LoginRateLimiter` (global sliding window, default 10
  failures / 60 s). Saturated → `429` + `Retry-After`. Wired in
  `routes::admin::login_submit`. **Global, not per-IP**: behind a
  reverse proxy the peer IP is the proxy, and a per-IP key would
  trust a spoofable `X-Forwarded-For`. A global cap can't be
  evaded by rotating source addresses.
- **[implemented]** Self-service password change revokes the
  account's **other** sessions (#739) — changing your password is the
  natural response to a suspected compromise, so every other live
  session (including an attacker's) dies; the requester is re-issued a
  fresh session so they stay signed in. The admin-initiated reset has
  done this since #544.
- **[implemented]** Operational replica actions are audited (#745) —
  dashboard stop/restart write `replica.stop`/`replica.restart` rows
  (with the acting user) to the same `audit_log` config mutations use.
- **[implemented]** HA admin-session cache is bounded (#738) — the
  Postgres-backed session store caches a negative entry per looked-up
  session id (anti-hammer); an amortized sweep now evicts expired
  entries, so an unauthenticated client spraying random cookie values
  can no longer grow the node's memory without bound.
- **[implemented]** Admin cookie is `HttpOnly` + `SameSite=Strict`
  + `Secure` (under TLS, see §7) — `routes::admin::login_submit`.
- **[implemented]** Opaque server-side sessions (#77) — the cookie
  carries a random 244-bit session id (`auth::AdminSessionStore`),
  never the token. Logout and server restart revoke it; a stolen
  cookie never exposes the token. The store is in-memory by default
  (`InMemoryAdminSessionStore`); for HA, point Ruscker at a shared
  Postgres via `--admin-session-store-url` (#185) so sessions survive
  a load-balancer hop.
- **[implemented]** Role-based access control (#101/#107) — three
  roles (**Viewer** = dashboard read-only; **Editor** = apps + media +
  dashboard incl. stop/restart; **Admin** = everything, incl. user
  management). Enforcement is **server-side** via the `AdminSession` /
  `RequireEditor` / `RequireAdmin` extractors on each route group —
  the permission matrix lives in `Role::can_access_section` /
  `can_manage`, and the nav only *hides* links it can't reach (UX, not
  the boundary). Denied → `403`. Admins manage accounts at
  `/admin/users` (create/role/reset-password/delete) with a
  **last-admin guard** that refuses to delete or demote the only
  remaining admin. Audit entries record the acting username (or
  `token` for a break-glass session). Per-app ACLs and external IdPs
  (OIDC/SAML/LDAP) remain Phase 8.
- **[implemented]** Identifier charset validation (#429 / #423) — a
  username (`db::users::is_valid_username`) and a stored-credential name
  (`routes::admin::credentials::is_valid_credential_name`) must be
  non-empty and made only of identifier-ish chars (letters, digits,
  `_ . -`, plus `@` for e-mail logins). Both land **un-encoded** in a
  per-resource admin action URL path segment
  (`/admin/users/{username}/...`, `/admin/credentials/{name}/delete`), so
  a `/`, `?`, `#`, or space would make the account/credential impossible
  to edit or delete from the UI — the validation keeps every row
  manageable (and therefore deletable). Rejected → the form re-renders
  with an error, no row written.
- **[implemented]** Password fields are **write-only** in the admin
  forms (#430) — the user form and the spec-form Registry section never
  pre-fill or render a stored password; a blank field keeps the existing
  value, and the input is masked so a shoulder-surfer can't read a
  freshly-typed secret. Server-side, a blank password on edit is a no-op,
  not a wipe.
- **[implemented]** Bind-mount **`volumes` are Admin-only** (#302). A
  spec's `volumes` map to Docker `HostConfig.binds` — i.e. host
  filesystem / `docker.sock` access — so an Editor (who can otherwise
  create/edit apps) cannot set or change them: the spec-form field is
  hidden for non-Admins and, server-side, `into_spec` keeps the base
  spec's volumes when the actor isn't Admin. Treat **Admin** as
  host-trusted and **Editor** as app-config-trusted.
- **[accepted limitation]** Login lockout can be triggered by a
  flood of bad attempts (the global limiter's trade-off). Self-
  heals within the 60 s window.

## 3. Credentials & secrets

- **[implemented]** Registry passwords encrypted at rest with
  AES-256-GCM — `crypto::MasterKey::{encrypt,decrypt}`. A fresh
  random nonce per encryption, stored alongside the ciphertext;
  never reused (new nonce on every `upsert`).
- **[implemented]** Unified credential store, two storage modes (#351) —
  `db::credentials::upsert` accepts **either** a literal password
  (AES-256-GCM at rest, as above) **or** a pure `${VAR}` env-ref (stored
  **verbatim**, never encrypted, flagged by an empty nonce — a real GCM
  nonce is 12 bytes, never empty — and resolved from the environment only
  at pull time). The env-ref branch is gated on
  `ruscker_config::env::is_pure_env_ref`: the value must consist
  **entirely** of valid `${VAR}` / `${VAR:-default}` tokens (whole-token
  only). A value with any literal text — e.g. `prefix${VAR}` or a
  malformed `abc${def` — is **not** kept verbatim; it's treated as a
  literal secret and AES-encrypted. This is the #422 fix: a loose
  `contains("${")` test would have stored such a literal in cleartext at
  rest. Either way the DB never holds resolved cleartext.
- **[implemented]** Master key held in `Zeroizing<[u8; 32]>`
  inside an `Arc` — wiped on last drop. Cookie key likewise
  (`ruscker_proxy::sticky::CookieKey`).
- **[implemented]** DB credential store wired to image pulls —
  `db::credentials::resolve` decrypts only at pull time, in the
  spawn path, never echoed to the UI.
- **[implemented]** `${VAR}` secrets stay literal end-to-end (#260) —
  `docker-registry-password` (and any [`env::SECRET_KEYS`] key) is
  **not** interpolated at parse: the `${VAR}` placeholder is preserved
  through `import` into the DB and through `export` output, and resolved
  only at the point of use (`creds_from_spec`, right before a pull). So
  the resolved secret never lands in the config DB or an export. The
  admin spec form treats the password as **write-only** — never
  pre-filled or rendered; a blank field keeps the stored value.
  `docker-registry-credential` (the named store — AES literal **or** a
  verbatim `${VAR}` env-ref, see above) is preferred for new flows; the
  spec-form Registry section is now just the picker for a stored
  credential. **`container-env` values** get the same treatment
  (#272): a `${VAR}` in a `container-env` value is preserved literal at
  parse and resolved only at spawn (`Spec::resolved_env_pairs`), so an
  app secret passed via `${VAR}` never lands in the DB either. A missing
  env var fails the pull/spawn with a clear message rather than passing
  a literal `${VAR}` — both for the registry password (#273) and for
  `container-env` values (#300).
- **[legacy]** A spec imported by an older build may hold a *resolved*
  password in its `config_json`. Re-import the YAML (which now preserves
  the literal) or rotate the secret to the credentials store; the
  `scan_raw_text` validator still flags inline cleartext in YAML.
- **[implemented]** Plaintext secrets never logged: pull path
  logs `with_creds=<bool>` + registry host, not the password;
  audit-log inserts carry action/target, not secret values.
- **[accepted limitation]** Cookie key and master key are
  separate, undrived keys. Deriving both from a single
  `RUSCKER_ROOT_KEY` via HKDF is a possible ergonomic
  improvement, not a security need.
- **[deferred]** Confirm bollard never logs the auth header on
  pull at its own `debug` level (we run it at `info`+ in prod).

## 4. Image uploads

- **[implemented]** 10 MB pre-decode cap —
  `images::MAX_UPLOAD_BYTES`, checked before any decode (defends
  against decompression-bomb-style payloads).
- **[implemented]** MIME sniffing via `infer::get` — PNG/JPEG/
  WebP recognized by magic bytes, not the client-supplied
  filename/Content-Type.
- **[implemented]** `X-Content-Type-Options: nosniff` on served
  responses (§7) so a polyglot upload can't be reinterpreted as
  active content.
- **[implemented]** **SVG script neutralization at serve time.**
  Uploaded SVGs are still stored as-is, but `/assets/img/*`
  responses (`routes::assets::serve_dynamic`) carry
  `Content-Security-Policy: default-src 'none'; style-src
  'unsafe-inline'; sandbox` + `X-Content-Type-Options: nosniff`.
  Even if a malicious SVG is opened directly or embedded via
  `<object>`/`<iframe>`, its `<script>`/`<foreignObject>` can't
  execute. The common `<img src=…>` use is unaffected (scripts
  never run in `<img>` context). The global page-header
  middleware uses `entry().or_insert` so it does NOT clobber this
  stricter per-asset policy. **[deferred]** content-level
  sanitization (`usvg`) if we ever need SVG in an active context.
- **[implemented]** Path traversal guard on `/assets/img/{file}`
  rejects `/` and `..`, with tests for encoded variants
  (`%2F`, `%2e%2e`, backslash) — all 400 / 404, never a file
  read.

## 5. SQL & database

- **[implemented]** All queries parameterized — no string
  interpolation into SQL (`grep format!.*SELECT` across
  `db/` is empty). Dynamic filters in `db::audit::list` use
  `QueryBuilder::push_bind`, not concatenation.
- **[implemented]** `journal_mode = WAL` + `foreign_keys = ON`
  — `db::open` / `db::open_memory` (`db.rs`).
- **[accepted limitation]** No automated backups — the operator
  owns the SQLite file's backup schedule. Documented in §8.

## 6. Proxy

- **[implemented]** Hop-by-hop header strip — `routes::proxy`
  `HOP_BY_HOP` covers RFC 7230 §6.1 tokens + the dynamic
  `Connection:` token list. `X-Forwarded-Proto` / `-Port` are
  stripped before forwarding upstream.
- **[implemented]** Open-redirect closed — `routes::same_origin_path`
  reduces a `Referer` to a same-origin path; used by `/__set/*`
  and the login redirect.
- **[implemented]** CSRF defense — admin cookie is
  `SameSite=Strict`, so a cross-site POST can't carry it, **and** a
  server-side guard (`csrf_guard`, #259) rejects state-changing chrome
  requests that aren't same-origin: it trusts `Sec-Fetch-Site`
  (`same-origin`/`none` only) when present, else falls back to an
  `Origin` vs `Host` check. Requests with neither header (curl, the
  break-glass token POST) pass — they aren't browser CSRF.
- **[implemented]** Ruscker cookies are stripped before forwarding
  upstream (#258) — `strip_ruscker_cookies` removes the admin session,
  every sticky cookie (matched by the `__ruscker_session` prefix, so
  per-spec and legacy names are both covered), and the theme/locale
  prefs from the upstream-bound `Cookie` header so an app container
  never sees them (the admin session id is a bearer). The WebSocket
  handshake path applies the same filter.
- **[accepted limitation — needs origin separation]** Admin and apps
  share one origin by default. A script inside an **untrusted** app
  served at `/app/{spec}` is genuinely *same-origin* with `/admin`, so
  neither `SameSite=Strict` nor the same-origin CSRF guard can stop it
  from issuing credentialed `fetch('/admin/...')` calls. The cookie
  strip (#258) stops the app from *reading* the session, but a
  same-origin request from the browser still carries it. **Trusted,
  first-party apps on one origin are fine; for untrusted / third-party
  apps, see [§6.1 Hosting untrusted apps](#61-hosting-untrusted-apps-same-origin)**
  — the host-safety backstops below (#871/#889/#894) bound the blast
  radius regardless.
- **[implemented]** Per-app sticky cookies (#731, v0.2.5) — the
  sticky cookie is named `__ruscker_session_{spec}` and scoped with
  `Path={base}/app/{spec}`, so it is only ever sent to its own app:
  two apps in one browser can't fight over a session, and the cookie
  never travels cross-app at all. A lingering pre-v0.2.5 global
  cookie (`Path=/`) is actively expired. The embedded
  `session.spec_id == spec.id` check stays as defense-in-depth
  against a copied/forged cookie (`routes::proxy::resolve_replica`).
- **[implemented]** Sticky cookie integrity — HMAC-SHA256
  truncated to 16 bytes (128-bit forgery resistance) over the
  signed payload (`ruscker_proxy::sticky`). 128 bits is far past
  brute-forceable within a session window.
- **[accepted limitation]** The upstream is always a
  Ruscker-spawned container's published port — `127.0.0.1:<port>` on
  the local daemon, or `host:<port>` for a configured `proxy.hosts`
  entry — never an operator-typed URL, so it isn't an SSRF vector.
- **[accepted limitation]** Container labels (`ruscker.spec_id`,
  …) are trusted by `list()`. A manually-created container could
  forge them; acceptable because the operator owns the host.
- **[implemented]** `proxy-connection` (legacy HTTP/1.0) is in the
  hop-by-hop strip list.
- **[implemented]** WS pump isolation — each direction runs as an
  independent task (a slow client only backpressures its own
  producer), with an idle watchdog and a drain grace on close. A
  30-second per-frame send timeout closes a persistently blocked
  connection instead of dropping stateful frames; structured events
  identify the app, replica, close origin/code/reason, frame count,
  and timeout policy (#933).
- **[implemented]** `X-Forwarded-For` is normalized on the forward
  (#744, v0.2.5) — in trusted mode (§7) the real peer IP is appended
  to the inbound chain; untrusted, the spoofable client value is
  replaced with the peer. Upstream apps never see a forged chain.
  `X-Forwarded-Proto`/`-Port` are stripped and re-set authoritatively.
- **[implemented]** The per-client API rate limiter is bounded (#737)
  — an amortized sweep evicts `(spec, client)` windows whose newest
  hit aged past the largest configured window, so rotating source
  addresses (one IPv6 /64 is 2^64 of them) can't grow the map without
  bound.

### 6.1 Hosting untrusted apps (same-origin)

Ruscker routes by **path** on a single origin: the portal at `/`, the
admin at `/admin/*`, and proxied apps at `/app/{spec}/*` all share one
`scheme://host:port`. A container's web UI is **author-controlled code**;
when it runs in the browser it is the *same origin* as `/admin`, so its
JavaScript can issue credentialed `fetch('/admin/...')` requests (the
browser attaches the admin session cookie on same-origin requests) and
read same-origin responses. The `SameSite=Strict` cookie and the
`csrf_guard` only stop *cross*-origin requests, and a synchronizer token
wouldn't help either — same-origin script can simply read it. **Only a
separate origin truly isolates an app from the admin.**

**This matters only for untrusted apps.** If every app is first-party
(authored by your team), there is no hostile script and a shared origin
is fine — this is the supported posture and what the reference
deployments use.

**If you must host untrusted / third-party apps**, the options today,
strongest first:

1. **Don't.** Keep untrusted workloads on a *separate Ruscker instance*
   (its own host/origin, its own admin) from the one serving your
   first-party catalog. Two processes, two origins — full isolation, no
   new code.
2. **Operational hygiene** on the shared instance: administer from a
   **separate browser profile** (or after logging out of every app), so
   no live admin session exists in the browser that opens apps. The
   attack needs a logged-in admin session in the *same* browser.
3. **Rely on the backstops.** The Disk panel enforces Ruscker ownership
   and fails closed (#871/#889/#894), spec edits are audited, and
   sensitive chrome (credentials, users, volumes) is Admin-only — so a
   same-origin admin call has a bounded, logged blast radius. This is
   defense-in-depth, not isolation.

> **Not enough on its own:** simply pointing a second hostname at the
> same Ruscker via nginx does **not** separate the origins, because
> Ruscker (path-based) emits app links and `base href` relative to the
> request origin — the portal cards and the app's own rewritten URLs
> would still resolve back to a single origin. Real separation needs
> Ruscker to route apps by **Host** and emit absolute app-origin URLs.

**Roadmap (#878):** a first-class *app origin* — serve `/app/*` (or a
subdomain per app, `{spec}.apps.example.org`) on a distinct hostname,
routed by `Host`, so the browser's same-origin policy isolates apps from
the admin and from each other. The design must account for the
`context-path` / `--base-path` mount, the forwarded-header trust gate
(§7), and the per-spec sticky cookie scoping (§6) — which is why it's a
deliberate feature, not a config flip.

## 7. TLS, headers & network

- **[implemented]** Security response headers on Ruscker's own
  surfaces (landing/admin/prefs/assets), NOT on proxied
  `/app/*`,`/api/*` — `lib::security_headers`:
  `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
  `Referrer-Policy: same-origin`, and a `Content-Security-Policy`
  (`default-src 'self'; … frame-ancestors 'none'; base-uri 'self';
  form-action 'self'`).
- **[implemented]** `Secure` cookie flag under TLS — admin + sticky
  cookies set `Secure` when `auth::request_is_https` is true. Since
  v0.2.5 (#762) that reads `X-Forwarded-Proto` **only under the trust
  opt-in below**, and takes the **rightmost** entry of a chained list
  (the one appended by the proxy closest to Ruscker — the leftmost
  slot is client-controlled whenever a proxy appends). Off on
  plain-HTTP dev so the browser doesn't drop the cookie.

### Forwarded-header trust model

One switch — `server.useForwardHeaders: true` (or a
`forward-headers-strategy` other than `none`) — gates **every**
read of client-suppliable `X-Forwarded-*` headers:

| Surface | Trusted (`useForwardHeaders: true`) | Untrusted (default) |
|---|---|---|
| `X-Forwarded-Proto` → cookie `Secure` flag | honoured (rightmost entry) | ignored — cookies never carry `Secure` |
| `X-Forwarded-For` → API rate-limit client key | rightmost parseable address | TCP peer |
| `X-Forwarded-For` → forwarded upstream to apps | peer appended to the inbound chain | inbound value replaced with the peer |

The default is **untrusted** because honouring these headers from
arbitrary clients lets anyone spoof their identity (rate-limit
evasion) or flip cookie flags. **If a reverse proxy terminates TLS in
front of Ruscker, you must set `useForwardHeaders: true`** (and make
the proxy set `X-Forwarded-Proto`, §9) or cookies will be minted
without `Secure`. ShinyProxy-migrated configs already carry the flag.
- **[accepted limitation]** Ruscker does NOT terminate TLS —
  expects a reverse proxy (see §9).
- **[deferred]** CSP currently allows `'unsafe-inline'` for
  script/style because the landing + dashboard use inline
  `<script>`/`<style>`. A nonce-based CSP that drops
  `unsafe-inline` is the hardening follow-up.

## 8. Logging & observability

- **[implemented]** Default `tracing` level (`info`) logs paths,
  spec ids, replica ids — operational metadata, no secrets, no
  PII.
- **[accepted limitation]** No PII is collected today; revisit if
  auth/user features land.
- **[opt-in]** Prometheus `/metrics` (`proxy.metrics-enabled`, off by
  default). When enabled it's served **unauthenticated** — it exposes
  operational gauges (replica counts/states, per-spec sessions,
  per-replica CPU/memory), no secrets. Only enable it where the
  endpoint is reachable solely by your scraper (private network /
  firewall / bound behind the reverse proxy); don't expose it to the
  public alongside the landing page.

---

## 9. Recommended production configuration

### Reverse proxy (terminates TLS, forwards scheme)

Minimal Caddy:

```caddy
portal.example.org {
    reverse_proxy 127.0.0.1:8080 {
        header_up X-Forwarded-Proto {scheme}
    }
}
```

Minimal nginx:

```nginx
server {
    listen 443 ssl;
    server_name portal.example.org;
    # ssl_certificate ... ssl_certificate_key ...;
    location / {
        proxy_pass http://127.0.0.1:8080;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_set_header Host $host;
        proxy_http_version 1.1;            # WebSocket support
        proxy_set_header Upgrade $http_upgrade;
        proxy_set_header Connection "upgrade";
    }
}
```

`X-Forwarded-Proto: https` is what flips the `Secure` cookie flag on
(§7) — and since v0.2.5 it is only honoured when the YAML sets
`server.useForwardHeaders: true`. Behind a TLS-terminating proxy you
need **both** (the header on the proxy, the flag in the YAML);
without them Ruscker assumes plain HTTP and omits `Secure`.

### Binding

Bind Ruscker to localhost so only the reverse proxy reaches it:

```bash
ruscker serve --bind 127.0.0.1:8080 ...
```

### Secrets (env vars — never in YAML)

```bash
export RUSCKER_ADMIN_TOKEN=$(openssl rand -hex 32)   # 256-bit — break-glass admin
export RUSCKER_MASTER_KEY=$(openssl rand -hex 32)    # AES-256 key
export RUSCKER_COOKIE_KEY=$(openssl rand -hex 32)    # sticky HMAC key
```

On first run, log in with `RUSCKER_ADMIN_TOKEN` and you'll be prompted
to create the first admin **account** (username + password). After
that, everyone signs in with their account; the token stays as a
break-glass / recovery path. Manage further accounts (Viewer / Editor /
Admin) at `/admin/users`.

- Set `RUSCKER_COOKIE_KEY` explicitly in prod — without it the
  sticky key is randomized per process, invalidating all sessions
  on restart.
- Rotate `RUSCKER_ADMIN_TOKEN` if it may have leaked (it is a
  break-glass bearer). The admin *cookie* never contains the token —
  it carries an opaque server-side session id (§2), revocable by
  logout or restart.

### Backups

Snapshot the SQLite DB (the `--db` file) on your own schedule;
Ruscker does not back it up. With WAL, copy `*.db`, `*.db-wal`,
`*.db-shm` together, or use `sqlite3 .backup`.

---

## 10. Audit checklist status (issue #14)

Blocking-for-prod (all done):
- [x] CSP + security headers on admin (§7)
- [x] `Secure` cookie flag under TLS (§7)
- [x] Login rate limiting (§2)

Non-blocking follow-ups:
- [x] SVG script neutralization (CSP+sandbox at serve time) (§4)
- [x] Encoded path-traversal tests for `/assets/img` (§4)
- [x] Opaque server-side admin sessions — cookie no longer holds the
      token; logout revokes server-side (#77)
- [x] Operator CSP origins (blocks/analytics) sanitized before use (#82)
- [x] `proxy-connection` in hop-by-hop strip (§6, #84)
- [x] WS pump backpressure: independent tasks + idle/send watchdogs,
      structured close diagnostics, and no stateful frame drops (§6,
      #81, #933)
- [x] `audit_log.diff_json` verified to record metadata only — never a
      password/token/cookie (regression test in `db::credentials`)
- [x] Automated `cargo audit` in CI (`.github/workflows/security.yml`,
      weekly + on dependency changes) and as a blocking release gate.
      The remaining `RUSTSEC-2024-0436` unmaintained warning is visible but
      non-fatal: `paste` is only present through `image`'s optional `ravif`
      lockfile edge, while Ruscker enables only `png`, `jpeg`, `webp`, and
      `rayon`. Re-evaluate this exception whenever the `image` feature set
      changes.
- [ ] Nonce-based CSP, drop `unsafe-inline` (§7)
- [ ] `semgrep` in CI (cargo-audit is wired; semgrep deferred)
