# Ruscker — Security & Threat Model

Status: living document. Tracks the Phase 5 security audit
(issue #14). Each control is marked **[implemented]**,
**[accepted limitation]**, or **[deferred]**. File references use
`crate/path:symbol` so they survive line-number drift.

> **Scope.** A single-operator install: Ruscker behind a
> TLS-terminating reverse proxy, Docker on one or more hosts. The model
> below reflects the current security audit.
> External identity-provider integration (OIDC, SAML, LDAP) remains
> Phase 8 work.

---

## 1. Threat model

### Assets

| Asset | Why it matters |
|-------|----------------|
| `RUSCKER_ADMIN_TOKEN` | break-glass Admin login + first-account bootstrap — full access |
| User passwords (DB) | per-user login; stored as argon2id hashes; policy on set/reset: ≥ 8 chars with upper + lower + digit + special |
| `RUSCKER_MASTER_KEY` | decrypts registry credentials and enrolled TOTP factors |
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
- External identity providers (OIDC/SAML/LDAP). Database-backed users,
  coarse RBAC, per-spec user/group ACLs, and per-app step-up MFA exist.
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
- **[implemented]** Sign-in throttling — `auth::LoginRateLimiter`,
  on the password and the break-glass token forms. Attempts are counted
  in layers, each refusing on its own with `429` + `Retry-After`:
  - **account + address**: after three failures in a row, each new
    attempt waits twice as long (1 s, 2 s, 4 s … up to 5 min). The
    streak is forgotten only after an hour without attempts, so sitting
    out the longest wait does not buy a fresh run of fast guesses:
    about 12 guesses an hour per account from one address;
  - **account**, from any address: 30 attempts per 10 minutes, so
    spreading guesses over many addresses does not help (at most about
    180 an hour against one account, however many addresses);
  - **address** (an IPv6 /64 counts as one): 30 attempts per minute,
    against one password tried on many accounts;
  - **global**: 300 attempts per minute, the last line.

  An attempt is counted **when it is admitted**, before the (slow)
  password check, and a correct sign-in gives its slot back — otherwise
  a burst of parallel requests would all get in before the first
  failure was recorded. A refused attempt is not counted, and an
  unknown username is throttled exactly like a real one.

  The first version was a single global window (10 failures per
  minute): it stopped guessing but let anyone pause sign-in for
  everybody by failing on purpose. The account, address and global
  ceilings are still collective, so a pair (account, address) that has
  signed in correctly in the last 30 days is **exempt from those
  three** and keeps only its own streak (at most 50 addresses are
  remembered per account). Strangers failing on purpose cannot keep a
  person out of the address they normally use.

  A correct sign-in gives back its own slot and halves the pair's
  earlier failures; it does not erase them, nor the account's window —
  otherwise the owner signing in would hand a guesser at the same
  address a clean slate each time. An attempt the portal could not
  judge (the user store failed) is uncounted. When a table is at its
  backstop size and an attempt cannot be counted, it is refused.

  **What remains**, because it would take something per browser (a
  device cookie) to tell two people at one address apart:
  - an account can be kept out *from an address it has never signed in
    from* for as long as someone keeps failing against it (30 attempts
    per 10 minutes), and with enough addresses (ten at full rate) the
    password form can be paused for people the portal does not know;
  - someone **at the same address** as a person — an office NAT — can
    keep that person's streak alive from there with one wrong password
    every few minutes;
  - a guesser at the owner's address gets, besides the steady 12 an
    hour, a few faster tries after each of the owner's sign-ins
    (halving, not resetting); and three free tries per new address
    until the account ceiling;
  - a co-located observer can tell a known pair from an unknown one by
    which attempts are refused.

  The list of known pairs is in memory, and rebuilt from the database
  at start-up and then **every 30 seconds** — so a restart does not
  forget it, and each node of an HA deployment hears about a sign-in or
  a password change made on another within about half a minute (nodes
  need clocks that agree to within two minutes). Two sources:
  - **the audit log**: the correct password sign-ins of the last 30
    days, account and address. Always used; limited by the audit
    retention, and it never has the break-glass token.
  - **the throttle's own table** (`signin_memory`), when
    `RUSCKER_COOKIE_KEY` is set: for each correct sign-in — password
    or token — the account, a **keyed hash** of the (account, address)
    pair, and the date. The key is derived from the cookie key, which
    lives outside the database, so an address cannot be read back from
    the table; the throttle can only recognise a pair it is shown.
    Rows are deleted after 30 days whatever the audit retention is.
    This is what makes the memory independent of a short audit
    retention and what keeps the operator's token address known across
    a restart. Without the variable the cookie key is random per
    process, there is nothing stable to hash with, and the table is
    not used.

  A **password change or reset, or deleting the account, forgets**
  where it was known — in memory and in the table at once on the node
  where it happens, within 30 seconds on the others — and sign-ins
  from before the change, or from before the account was created (a
  username can be reused), are not read back. A sign-in whose audit
  event was dropped, or whose table write failed, is simply not
  remembered. At most 50,000 rows are read from each source (the
  newest; a warning is logged if more exist). Without a database there
  is no memory across restarts.

  The break-glass **token** form has no account and its secret opens
  everything, so it keeps one low ceiling for all addresses together
  (10 attempts per minute). It shares no counter with the password
  form: flooding one does not close the other, from any address.
  Failing on purpose can pause the token form for strangers, but not
  for an address the token was used correctly from in the last 30
  days, which has a window of its own. That address is remembered
  across restarts when `RUSCKER_COOKIE_KEY` is set (see below);
  otherwise nobody is known after a restart, and an operator locked
  out of the form then still has the server itself.

  The address is the one resolved for auditing — from `X-Forwarded-For`
  only with `server.useForwardHeaders`. Behind a proxy **without** it
  every request has the proxy's address, so everyone is "at the same
  address": the address layer is one ceiling shared by all (30 per
  minute, where the first version had 10), and anyone can keep a
  chosen account's streak alive. **Set the flag behind a proxy.** Counters are in memory, per process (an HA
  deployment throttles per node). There is deliberately no CAPTCHA: a
  third-party one would put an outside script on the page that receives
  passwords and need outbound access from the server.
- **[implemented]** Self-service password change revokes the
  account's **other** sessions (#739) — changing your password is the
  natural response to a suspected compromise, so every other live
  session (including an attacker's) dies; the requester is re-issued a
  fresh session so they stay signed in. The admin-initiated reset has
  done this since #544.
- **[implemented]** Operational replica actions are audited (#745) —
  dashboard stop/restart write `replica.stop`/`replica.restart` rows
  (with the acting user) to the same `audit_log` config mutations use.
- **[implemented]** User activity is recorded (#1021) — a `user_activity`
  table logs successful password logins and new interactive app sessions
  (username or "anonymous", app id, timestamp) for the Admin-only Activity
  page. Since #1079 it also records **refused** password sign-ins, SSO
  sign-ins and SSO refusals, and every row — **app accesses included,
  so also anonymous visitors of public apps** — carries the **client
  address** and the browser's **`User-Agent`** (sanitized, at most 256
  characters) and, for a refusal, the reason. Rules that keep this an
  audit and not a liability:
  - the address follows the forwarded-header trust rule of the rate
    limiter: when the operator opted into forwarded headers, the
    right-most entry of the **last** `X-Forwarded-For` line (the one the
    reverse proxy in front wrote); the TCP peer otherwise. This is
    trustworthy with a single reverse proxy in front that sets or
    appends the header, which is the documented deployment;
  - events an outsider can cause at will are **budgeted**: refused
    sign-ins to ten rows a minute per address (per /64 for IPv6), SSO
    refusals to one a minute per user, system and reason, and each of
    the two classes to 240 a minute overall — so they can neither fill
    the table nor push real events out of the queue. The classes have
    separate ceilings, so anonymous traffic against the login cannot
    silence the record of SSO refusals; and when a ceiling is reached a
    marker row ("recording limit reached") is written for that minute —
    typed `sso.suppressed` for SSO refusals, so it appears on the SSO
    Access tab too — and both markers can be filtered for, so the gap is
    visible rather than looking like nothing happened;
  - a refused sign-in keeps the typed username **only if it names an
    existing account**; anything else (people type passwords in the wrong
    field) is not stored;
  - no device fingerprinting — only what the request itself states;
  - rows are **purged after `server.activity-retention-days`** (default
    180) because address and browser are personal data;
  - API calls are not logged per-request (only the aggregate counter), the
    table has no foreign keys so deleting a user or app is not blocked by
    it, and read access is Admin-only.
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
- **[implemented]** Per-app TOTP step-up MFA (#1005) — a factor belongs to
  the user, not an app. The user enrolls it once under **Account → 2FA** by
  scanning a local QR code with Google Authenticator, Microsoft Authenticator,
  Authy, 1Password, or another compatible app, then receives one-time
  recovery codes that are shown once. Each spec independently opts
  in with `require-mfa` and selects the acceptable proof age with
  `mfa-validity-days` (seven days by default, session-only at `0`, capped at
  30). One successful proof satisfies every protected app whose freshness
  policy accepts it.
- **[implemented]** Trusted-device grants use an opaque, `HttpOnly` cookie;
  only a salted token hash and a one-way login-session binding are stored in
  `user_mfa_grants`. Grants are bound to the factor confirmation time and
  security epoch. Password change/reset, MFA reset, user deletion, **Forget
  this device**, and **Forget all devices** revoke the applicable grants; a
  zero-day policy also requires the current opaque login session.
- **[implemented]** The MFA guard (`mfa::evaluate`) runs on `/app` and `/api`
  before backend access, cold-start handling, replica selection, or spawn.
  Unenrolled/unproven interactive-app visits redirect to enrollment or a
  challenge without starting a container; protected APIs return `401` when
  the caller is not signed in and `403` when the factor proof is unsatisfied.
  WebSocket upgrades are guarded before the upstream handshake.
- **[implemented]** TOTP secrets are AES-256-GCM encrypted at rest with
  `RUSCKER_MASTER_KEY`; enrollment and TOTP verification fail closed with
  `503` when the key is unavailable. Secrets and recovery codes never appear
  in logs, audit diffs, or configuration exports. The factor, recovery-code,
  and grant tables work with SQLite and Postgres/HA. Admins see only whether
  2FA is configured and may perform an audited reset; they cannot retrieve a
  factor secret or recovery code.
- **[implemented]** `RUSCKER_ADMIN_TOKEN` break-glass sessions bypass the
  user factor so recovery cannot deadlock. Every bypass emits a warning and
  a cooldown-deduplicated `mfa.break_glass_bypass` audit row. Reserved
  `__ruscker_mfa_*` cookies are consumed by Ruscker and stripped before HTTP
  or WebSocket proxying, so device and enrollment bearers never reach an app.
- **[implemented]** Role-based access control (#101/#107/#990). The nav
  reflects the matrix below, but hiding a link is only UX. Coarse section
  access is enforced server-side by `AdminSession` / `RequireEditor` /
  `RequireAdmin`; handlers for group-scoped Apps, Containers, Users and
  Groups then apply `EditorScope` to the authoritative database row.

  | Session | Admin-panel trust boundary |
  |---------|----------------------------|
  | **Viewer** | Portal user only; no admin-panel section. |
  | **Editor** | App-config-trusted but **group-scoped**. May operate open apps and apps/replicas sharing one of the Editor's exact, case-sensitive groups; may administer non-Admin users and membership within those groups. Media remains a shared Editor resource. |
  | **Admin account** | Unrestricted across the panel and all catalog rows. |
  | **`RUSCKER_ADMIN_TOKEN` break-glass** | Unrestricted Admin session even when the account database is absent or damaged. |

  A role denied at the section guard receives `403`. Once an Editor is
  admitted to Apps, Containers, Users or Groups, a missing **or
  out-of-scope** target receives the same `404`; this is deliberate
  non-disclosure, so guessed identifiers cannot reveal that another team's
  app, account, group or replica exists. Scope is fetched on every request
  rather than copied into the session, so membership removal takes effect on
  the next request and lookup failure fails closed.

  The #990 integration tests lock down the privilege-escalation invariants:
  an Editor cannot grant a group they do not have, cannot view or mutate an
  Admin account, cannot assign the Admin role, and cannot change their own
  role or groups through delegated user management. An Editor update replaces
  only memberships inside their scope; out-of-scope groups survive unchanged.
  Account deletion, CSV user import, MFA reset, and group
  create/rename/delete remain Admin-only. The **last-admin guard** still
  refuses to delete or demote the only remaining Admin. Audit entries record
  the acting username (or `token` for break-glass).
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
- **[implemented]** Reserved-cookie response filtering (#1010) — apps share
  the portal origin, so the proxy inspects every upstream `Set-Cookie` and
  drops cookies whose names belong to Ruscker (admin session, preferences,
  sticky sessions, and MFA), while preserving the app's own cookies. It also
  neutralizes the `cookies` and `*` directives in app-supplied
  `Clear-Site-Data`; safe non-cookie directives such as `cache` and `storage`
  may remain. A same-origin app therefore cannot overwrite or bulk-clear the
  user's portal, sticky, or trusted-device cookies through its response.
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
- **[implemented]** Per-app sticky cookies (#731) — the
  sticky cookie is named `__ruscker_session_{spec}` and scoped with
  `Path={base}/app/{spec}`, so it is only ever sent to its own app:
  two apps in one browser can't fight over a session, and the cookie
  never travels cross-app at all. A lingering legacy global
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
  (#744) — in trusted mode (§7) the real peer IP is appended
  to the inbound chain; untrusted, the spoofable client value is
  replaced with the peer. Upstream apps never see a forged chain.
  `X-Forwarded-Proto`/`-Port` are stripped and re-set authoritatively.
- **[implemented]** The per-client API rate limiter is bounded (#737)
  — an amortized sweep evicts `(spec, client)` windows whose newest
  hit aged past the largest configured window, so rotating source
  addresses (one IPv6 /64 is 2^64 of them) can't grow the map without
  bound.

### 2.1 OpenID Connect provider (optional)

When `server.oidc-provider` is enabled, Ruscker signs users in for
systems outside the portal (design and decisions:
[ADR 0005](https://github.com/StrategicProjects/ruscker/blob/main/docs/adr/0005-oidc-provider.md)). Off by default; with it off,
none of its endpoints exist.

| Boundary | Rule |
|---|---|
| Who may be a client | Only what an Admin registered. No dynamic registration, no public clients. |
| Where a user may be sent | A registered redirect URI, matched byte for byte. Until client and URI check out nothing is redirected — the user sees a 400. |
| Which flow | Authorization code with PKCE `S256` only. A repeated request parameter is refused. |
| Who gets a code | A signed-in **account** (never the break-glass token) that is in the client's allowed groups, read from the database on every request, with MFA proved in the same login session when the client requires it. No bypass for Admins. |
| The code | 256 random bits, stored as a SHA-256, valid 60 seconds, redeemed by one conditional `UPDATE`. |
| Who may redeem it | The same client, authenticated with its secret (stored as a salted hash), repeating the redirect URI and presenting the PKCE verifier. Client authentication comes first: a failed one does not spend the code. |
| Tokens | ES256. ID token: audience = the client. Access token: audience = the issuer, a different `typ`, accepted only by `/oauth/userinfo`. Five minutes each. |
| Signing key | Generated with `ring`, stored AES-256-GCM-encrypted under `RUSCKER_MASTER_KEY`, published by RFC 7638 thumbprint. Never replaced automatically. |
| Issuer | Configured, never derived from the request's `Host`; its path must equal the mount point. |
| Unauthenticated cost | The token request body is capped at 4 KiB and 32 parameters; client ids are shape-checked before any database read. |

The session cookie stays `SameSite=Strict`. A browser arriving from a
client on another site therefore sends none; that request is answered
with a minimal page that re-requests the same URL from Ruscker's own
origin, so an existing session is recognised without weakening the
cookie.

What a client must do for its part — validate signature, `iss`, `aud`,
`exp` and `nonce`; bind `state` and the PKCE verifier to its browser
session; identify people by `sub` — is spelled out in the
authentication guide. The provider cannot enforce it.

**Interaction with §6.1.** Hosted apps share the origin of the sign-in
page. With the provider enabled that page yields tokens for other
systems as well, so the same-origin caveat below weighs more: do not
enable the provider on a portal that hosts code you do not control
unless the apps are served from a separate origin.

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
  cookies set `Secure` when `auth::request_is_https` is true. That reads
  `X-Forwarded-Proto` **only under the trust
  opt-in below**, and takes the **rightmost** entry of a chained list
  (the one appended by the proxy closest to Ruscker — the leftmost
  slot is client-controlled whenever a proxy appends). Off on
  plain-HTTP dev so the browser doesn't drop the cookie.

### Data plane: proxy → app containers

Where the traffic between Ruscker and the containers it spawns
actually flows, and why it is **deliberately not TLS**:

- **Single host (the default and every current deployment):** each
  container publishes its port on **`127.0.0.1`** and the proxy
  connects over loopback — the bytes never leave the machine, so
  there is no network path for an eavesdropper that doesn't already
  own the host. End-to-end encryption with the *user* is complete:
  browser —TLS→ reverse proxy —loopback→ Ruscker —loopback→ container.
- **Multi host (`proxy.hosts`):** the data plane crosses the network
  (`host:published-port`), so the deployment requirement is a
  **private network between the Ruscker box and the app hosts**, with
  the ephemeral published-port range opened only between them and
  **never exposed publicly** (the backends are unauthenticated). If
  the link between hosts isn't trusted, encrypt at the network layer
  (WireGuard / an encrypted overlay) — that protects every port at
  once and needs no per-container certificates.
- **[accepted limitation / by design]** Ruscker does not do TLS or
  mTLS *to* the app containers: apps (Shiny, Streamlit, Jupyter,
  Plumber) serve plain HTTP by default, and per-container certificate
  issuance/rotation would add real complexity against a threat the
  loopback / private-network requirement already removes. This
  mirrors ShinyProxy and the rest of this class of tool.
- **Identity-header trust boundary (#1001):** an app may trust the
  `X-SP-*` / reserved `X-Ruscker-User-*` identity namespaces only when
  its container port is reachable exclusively through Ruscker — loopback
  on a single host, or the private app network in a multi-host deployment.
  Exposing a container's published port lets a caller bypass the proxy and
  forge those headers directly. Ruscker strips inbound claims and injects
  authoritative `X-SP-UserId` / `X-SP-UserGroups` and explicitly selected
  `X-Ruscker-User-Email` / `X-Ruscker-User-Setor` / `X-Ruscker-User-Roles`
  claims only on opted-in specs and only for signed-in users. The same stripping and injection rules
  apply to HTTP and WebSocket handshakes; a selected claim with no value is
  omitted. Stripping reads `_` as `-`: `X_SP_UserId` is a different header
  on the wire but the same variable (`HTTP_X_SP_USERID`) inside a
  Shiny, WSGI or Rack app, where it could otherwise override the injected
  one. Ruscker cannot protect a separate network path around the proxy.
  E-mail and department/unit values are PII; operators should enable each
  claim only for apps that need and are trusted to process it. Group roles
  (#1067) are minimized per app: a spec restricted by `access-groups`
  receives only the roles of those groups, and the group names are
  percent-encoded so a crafted name cannot inject an extra `GROUP:ROLE`
  entry into the header.

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
- **[implemented]** User identity data is minimized at the app boundary:
  identity forwarding is per-spec and off by default, optional profile claims
  are individually selected, and blank claims are omitted. Usernames, groups,
  e-mail addresses, and department/unit values are PII and should not be
  enabled for an app without a need.
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
(§7) — and it is only honoured when the YAML sets
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
