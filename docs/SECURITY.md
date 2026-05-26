# Ruscker — Security & Threat Model

Status: living document. Tracks the Phase 5 security audit
(issue #14). Each control is marked **[implemented]**,
**[accepted limitation]**, or **[deferred]**. File references use
`crate/path:symbol` so they survive line-number drift.

> **Scope.** This covers the MVP (v0.1.0): single-operator
> install, Ruscker behind a TLS-terminating reverse proxy, Docker
> on the same host. Multi-tenant / shared-team auth (OIDC, RBAC)
> is out of scope until Phase 8.

---

## 1. Threat model

### Assets

| Asset | Why it matters |
|-------|----------------|
| `RUSCKER_ADMIN_TOKEN` | full admin access — create/edit specs, read audit log, stop containers |
| `RUSCKER_EDITOR_TOKEN` | (optional) Editor role — apps + media + dashboard actions |
| `RUSCKER_VIEWER_TOKEN` | (optional) Viewer role — dashboard read-only |
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
- TLS termination (delegated to the reverse proxy).

---

## 2. Authentication & authorization

- **[implemented]** Constant-time token compare —
  `auth::AdminAuth::role_for` → `ct_eq` (XOR-fold, length-checked).
  Time depends only on the public length, not the bytes. The
  candidate is compared against each configured role token
  (most-privileged first) and resolves to the matched [`Role`].
- **[implemented]** Login rate limiting —
  `auth::LoginRateLimiter` (global sliding window, default 10
  failures / 60 s). Saturated → `429` + `Retry-After`. Wired in
  `routes::admin::login_submit`. **Global, not per-IP**: behind a
  reverse proxy the peer IP is the proxy, and a per-IP key would
  trust a spoofable `X-Forwarded-For`. A global cap can't be
  evaded by rotating source addresses.
- **[implemented]** Admin cookie is `HttpOnly` + `SameSite=Strict`
  + `Secure` (under TLS, see §7) — `routes::admin::login_submit`.
- **[implemented]** Opaque server-side sessions (#77) — the cookie
  carries a random 244-bit session id (`auth::AdminSessions`), never
  the token. Logout and server restart revoke it; a stolen cookie
  never exposes the token.
- **[implemented]** Role-based access control (#101) — three roles
  with separate env tokens: `RUSCKER_ADMIN_TOKEN` (required ⇒
  **Admin**, full access), and optional `RUSCKER_EDITOR_TOKEN`
  (**Editor**: apps + media + dashboard incl. stop/restart) and
  `RUSCKER_VIEWER_TOKEN` (**Viewer**: dashboard read-only). The
  matched role rides in the session. Enforcement is **server-side**
  via the `AdminSession` / `RequireEditor` / `RequireAdmin`
  extractors on each route group — the permission matrix lives in
  `Role::can_access_section` / `can_manage`, and the nav only *hides*
  links it can't reach (UX, not the boundary). Denied → `403`. With
  only the admin token set, behaviour is identical to the previous
  single-token model (backward-compatible). Per-app ACLs and external
  IdPs (OIDC/SAML/LDAP) remain Phase 8.
- **[accepted limitation]** Login lockout can be triggered by a
  flood of bad attempts (the global limiter's trade-off). Self-
  heals within the 60 s window.

## 3. Credentials & secrets

- **[implemented]** Registry passwords encrypted at rest with
  AES-256-GCM — `crypto::MasterKey::{encrypt,decrypt}`. A fresh
  random nonce per encryption, stored alongside the ciphertext;
  never reused (new nonce on every `upsert`).
- **[implemented]** Master key held in `Zeroizing<[u8; 32]>`
  inside an `Arc` — wiped on last drop. Cookie key likewise
  (`ruscker_proxy::sticky::CookieKey`).
- **[implemented]** DB credential store wired to image pulls —
  `db::credentials::resolve` decrypts only at pull time, in the
  spawn path, never echoed to the UI.
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
  `SameSite=Strict`, so a cross-site POST can't carry it.
- **[implemented]** Sticky-cookie cross-app defense — the handler
  checks `session.spec_id == spec.id` before honoring a sticky
  cookie, even though its `Path=/` spans apps.
  (`routes::proxy::resolve_replica`.)
- **[implemented]** Sticky cookie integrity — HMAC-SHA256
  truncated to 16 bytes (128-bit forgery resistance) over the
  signed payload (`ruscker_proxy::sticky`). 128 bits is far past
  brute-forceable within a session window.
- **[accepted limitation]** Upstream is always `127.0.0.1:<port>`
  — not an SSRF vector while the operator can't point a spec at
  an external host.
- **[accepted limitation]** Container labels (`ruscker.spec_id`,
  …) are trusted by `list()`. A manually-created container could
  forge them; acceptable because the operator owns the host.
- **[deferred]** Add `proxy-connection` (legacy HTTP/1.0) to the
  hop-by-hop strip list.
- **[deferred]** WS pump backpressure — a slow client can
  accumulate frames. Bound the channel with a drop policy.
- **[deferred]** Whitelist (rather than only strip a couple of)
  client-supplied `X-Forwarded-*` headers before forwarding
  upstream.

## 7. TLS, headers & network

- **[implemented]** Security response headers on Ruscker's own
  surfaces (landing/admin/prefs/assets), NOT on proxied
  `/app/*`,`/api/*` — `lib::security_headers`:
  `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
  `Referrer-Policy: same-origin`, and a `Content-Security-Policy`
  (`default-src 'self'; … frame-ancestors 'none'; base-uri 'self';
  form-action 'self'`).
- **[implemented]** `Secure` cookie flag under TLS — admin +
  sticky cookies set `Secure` when `auth::request_is_https`
  (reads `X-Forwarded-Proto`) is true. Off on plain-HTTP dev so
  the browser doesn't drop the cookie.
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

---

## 9. Recommended production configuration

### Reverse proxy (terminates TLS, forwards scheme)

Minimal Caddy:

```caddy
portal.example.gov.br {
    reverse_proxy 127.0.0.1:8080 {
        header_up X-Forwarded-Proto {scheme}
    }
}
```

Minimal nginx:

```nginx
server {
    listen 443 ssl;
    server_name portal.example.gov.br;
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

`X-Forwarded-Proto: https` is what flips the `Secure` cookie flag
on (§7) — without it Ruscker assumes plain HTTP and omits `Secure`.

### Binding

Bind Ruscker to localhost so only the reverse proxy reaches it:

```bash
ruscker serve --bind 127.0.0.1:8080 ...
```

### Secrets (env vars — never in YAML)

```bash
export RUSCKER_ADMIN_TOKEN=$(openssl rand -hex 32)   # 256-bit — Admin
export RUSCKER_MASTER_KEY=$(openssl rand -hex 32)    # AES-256 key
export RUSCKER_COOKIE_KEY=$(openssl rand -hex 32)    # sticky HMAC key
# Optional extra roles (RBAC, §2) — omit for the single-admin model:
export RUSCKER_EDITOR_TOKEN=$(openssl rand -hex 32)  # Editor: apps + media
export RUSCKER_VIEWER_TOKEN=$(openssl rand -hex 32)  # Viewer: dashboard only
```

Use **distinct** tokens per role; sharing one across roles resolves
to the highest privilege.

- Set `RUSCKER_COOKIE_KEY` explicitly in prod — without it the
  sticky key is randomized per process, invalidating all sessions
  on restart.
- Rotate `RUSCKER_ADMIN_TOKEN` if you suspect cookie exfiltration
  (the cookie holds the token literally — §2).

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
- [x] WS pump backpressure: independent tasks + idle watchdog (§6, #81)
- [x] `audit_log.diff_json` verified to record metadata only — never a
      password/token/cookie (regression test in `db::credentials`)
- [x] Automated `cargo audit` in CI (`.github/workflows/security.yml`,
      weekly + on dependency changes)
- [ ] Nonce-based CSP, drop `unsafe-inline` (§7)
- [ ] `semgrep` in CI (cargo-audit is wired; semgrep deferred)
