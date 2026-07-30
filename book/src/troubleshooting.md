# Troubleshooting

## `/admin` returns 503 "RUSCKER_ADMIN_TOKEN is not set"
No admin token is configured. Set `RUSCKER_ADMIN_TOKEN` (the `.deb`
generates one — `sudo grep RUSCKER_ADMIN_TOKEN /etc/ruscker/ruscker.env`)
and restart. The admin pages also need `serve --db <file>`; without it
or `--config-db-url <postgres-url>` the editor/list screens return 503.

## Card logos don't show up
The images aren't being served at `/assets/img/`. Either pass
`--images-dir <dir>` pointing at the folder with the image files, or
keep the config next to its `template-path`'s `assets/img/` so Ruscker
auto-discovers it. Check: `curl -I http://localhost:8080/assets/img/<file>`.
With `--db`, you can also upload logos in **Media** and pick them in
the spec form.

## Apps don't start (proxy returns 503 / 502)
- `503 no container backend` — you started without `--docker`. Add it
  (and give the service Docker access).
- `502` — the container failed to start or pull. Check
  `docker logs` for the spawned `ruscker-<spec>-<id>` container, and
  verify registry credentials. Private images can use a named credential
  from **Credentials**, or `docker-registry-username` plus an env-backed
  `docker-registry-password: ${DOCKER_REGISTRY_PASSWORD}`.

## I removed or restarted a container with the Docker CLI

Ruscker self-heals. If you `docker rm -f` a managed container, or
`docker restart` it directly, Ruscker notices and reconciles on its own:

- a **removed** container is pruned from the registry, its stale sticky
  binding dropped, and a replacement is brought up (a browser visit lands on
  the "Starting…" splash instead of a persistent `upstream error`);
- a **restarted** container is re-adopted into the pool once it's serving
  again, without a duplicate being spawned.

A Docker events watcher makes this happen within about a second; the periodic
reconcile (~10 s) is the fallback. You may still see a brief `502` in the
window between the container going away and Ruscker reacting — retry and it
recovers. If it *doesn't* recover, the container is genuinely unreachable
(check `docker ps` and its logs). Recovery only acts on an authoritative
"gone" signal, so an app returning its own `5xx` or a momentary Docker daemon
hiccup never causes Ruscker to drop a healthy replica.

## 2FA enrolment returns `503`

`RUSCKER_MASTER_KEY` is missing (or an existing MFA secret cannot be
decrypted with the configured key). Set one stable 32-byte key, restart
Ruscker, and try **Account → 2FA** again:

```sh
openssl rand -hex 32
```

Put the result in the service environment, such as
`/etc/ruscker/ruscker.env`; do not regenerate it on each start. The `.deb`
generates it automatically when missing. MFA deliberately fails closed,
and the same key is also needed for encrypted registry credentials.

## My app no longer receives the logged-in user

Identity forwarding is off by default. Enable it on that spec:

```yaml
add-default-http-headers: true
identity-claims: [email, setor] # optional
```

The app then receives `X-SP-UserId` / `X-SP-UserGroups` and the selected
`X-Ruscker-User-*` claims for signed-in users, over HTTP and WebSocket. A
blank profile claim is omitted. Ruscker strips client-supplied reserved
identity headers, so test through Ruscker rather than by calling the
container directly.

## My app cannot set a cookie with a Ruscker-looking name

Apps share the portal origin, so response cookies using Ruscker's reserved
names are dropped to protect login, preference, sticky-session and MFA
state. The reserved set includes `ruscker_admin_session`, `ruscker_theme`,
`ruscker_locale`, `__ruscker_session*` and `__ruscker_mfa_*`; app-owned
cookie names are preserved. Rename the conflicting app cookie.

Ruscker also removes the cookie-clearing `Clear-Site-Data` directives
`"cookies"` and `"*"` from app responses, while preserving safe non-cookie
directives. An app must not try to clear all cookies on the shared origin.

## An app keeps failing with an old or broken image

You pushed a fixed image to the registry under the **same tag**, but
the app keeps crashing with the old error. That's not a Docker bug and
doesn't need a daemon restart — it's the pull-if-missing design: once
a tag is on the host, spawns use the local copy and never re-contact
the registry (a flaky network must not stop an app whose image is
already local).

The fix is one click: **Apps → the app's "Update image" button**. It
forces a pull, the daemon fetches the new manifest for the tag, and
the next start uses the fixed image — verified end to end, no
`systemctl restart docker` involved.

If a forced pull *doesn't* help and the registry digest is identical
to the local one (Docker answers "Image is up to date"), you may be
looking at genuine local-cache corruption — rare, and daemon-side. In
that order: `docker rmi <image>` then Update image again; a daemon
restart is the last resort, not the routine.

## A Shiny app loads but the page is broken / no live updates
Shiny needs WebSockets. Make sure your reverse proxy forwards the
upgrade headers (`Upgrade` / `Connection "upgrade"`) — see the nginx
snippet in [Deploying](./deploying.md).

## The admin shows the wrong / old features after an upgrade
Templates are compiled into the binary, so changes need a **rebuild +
reinstall**, not just editing files on the server:
`sudo apt-get install -y --no-install-recommends
./ruscker_<version>-1_amd64.deb && sudo systemctl restart ruscker`.
Confirm both `ruscker --version` and the local `/readyz` endpoint before
putting the node back into rotation.

## `413 Payload Too Large` on a Media upload
There are two independent size limits in the upload path; the **nginx
limit fires first** and is the more common culprit.

**nginx (most common).** nginx's default `client_max_body_size` is 1 MB.
Any upload larger than that is rejected by nginx before Ruscker even sees
the request — the admin shows a generic "upload doesn't work" failure with
no obvious error. Ruscker accepts images up to 10 MB (the request
limit is 12 MB, allowing multipart overhead).
Set a higher limit in your nginx server block:

```nginx
client_max_body_size 16m;
```

See the full nginx snippet in [Deploying](./deploying.md).

**Ruscker `proxy.max-body-size`.** A separate cap applies to requests
forwarded to app containers. If a specific API spec rejects large POSTs
with 413, raise `proxy.max-body-size` globally or as a per-spec override.

## `429 Too Many Requests` from an API
The spec's `api.rate-limit` is throttling the caller. The `Retry-After`
header says when to retry. Behind a proxy, set
`server.useForwardHeaders: true` so the limiter keys on the real client
IP (`X-Forwarded-For`) instead of the proxy's.

## Building the `.deb` fails on a locked-down host
If the host can't reach crates.io / static.rust-lang.org (only Docker
Hub + GitHub), build the `.deb` off-box — e.g. in a `rust:<ver>`
container on a machine with full internet, or in CI — and copy the
artifact over. Docker pulls and the `build.rs` Tailwind download (from
GitHub) still work from a connected builder.

If crates and the Rust toolchain are already cached but GitHub is unavailable,
the admin build script cannot download its pinned standalone Tailwind CLI.
Choose one of these explicit offline modes:

```bash
# Production/UI build: supply a previously downloaded Tailwind executable.
TAILWIND_BIN=/opt/tailwindcss cargo build --release

# Backend-only development/tests: compile with placeholder, unstyled CSS.
TAILWIND_SKIP=1 cargo test --locked
```

`TAILWIND_SKIP` is not suitable for a production admin UI. Without either
setting, a missing download now fails with a concise error explaining these
options instead of a Rust panic.

## `perl: warning: Setting locale failed` during `apt`/`dpkg`
Cosmetic — the install still succeeds. It means a locale your SSH
session forwards (commonly `LC_CTYPE=UTF-8` from a macOS client via
`SendEnv LC_*`) isn't a valid locale name on the Linux host (which has
`C.UTF-8` / `en_US.UTF-8`, not bare `UTF-8`), so perl-based maintainer
scripts fall back to `C.UTF-8`. It comes from `apt`'s own machinery,
not Ruscker's package scripts. To silence it, either fix the host
locale and stop forwarding the bogus one:
```bash
sudo locale-gen en_US.UTF-8 && sudo update-locale LANG=en_US.UTF-8
# optionally drop `AcceptEnv LC_*` from the host's sshd_config,
# or remove `SendEnv LC_*` for that host in your local ~/.ssh/config
```
or just prefix the install in your deploy/auto-update script:
```bash
export LC_ALL=C.UTF-8 LANGUAGE=
sudo apt-get install -y ./ruscker_<version>-1_amd64.deb
```

## Cookies don't carry `Secure` behind my TLS proxy

Ruscker only honours `X-Forwarded-Proto` when
`server.useForwardHeaders: true` is set (otherwise any client could
spoof it). Behind a TLS-terminating reverse proxy you need **both**:
the proxy sending `proxy_set_header X-Forwarded-Proto https;` (or
`$scheme`) *and* the YAML flag. Check with
`curl -sI -H "Accept: text/html" https://your-host/app/<id>/ | grep -i set-cookie`
— the sticky cookie should list `Secure`.

## A Streamlit / Dash / Voilà app is unreachable (connection refused upstream)

The current defaults are 8501 / 8050 / 8866 for
`type: streamlit|dash|voila`. If the image listens elsewhere, set
`container-port` explicitly and confirm the process binds to `0.0.0.0`
inside the container rather than loopback.

## Users bounce between replicas / lose their session after a restart
The sticky-session cookie is signed with `RUSCKER_COOKIE_KEY`. If you
don't set it, Ruscker generates a random key on each start — so every
restart invalidates existing session cookies and can scatter users
across replicas. Set a stable `RUSCKER_COOKIE_KEY` (e.g.
`openssl rand -hex 32`) in `ruscker.env` and keep it constant.

## The dashboard doesn't update live (or lags badly)
The Containers dashboard polls `GET /admin/dashboard/snapshot`; it does not
use SSE. In browser developer tools, check that this request returns `200`
every few seconds. Authentication redirects, a wrong base-path mapping or a
slow/unreachable Docker daemon will stop or delay updates. nginx buffering
does not affect the dashboard poll; it matters only for the explicitly
enabled per-replica live log stream described in [Deploying](./deploying.md).

## Admin dates or scheduled-job times look wrong

These use two intentionally different clocks:

- Activity, audit, Apps, Credentials and Users table dates render in the
  viewer's browser timezone. Process Logs also show the viewer-local clock.
  Check the operating system/browser timezone if they are wrong. Hover a
  table date for the full localized timestamp and zone; process-log hover
  keeps the original UTC token for correlation with downloaded logs.
- The scheduler evaluates cron in `server.timezone`, and the Schedules page
  labels next/last runs in that zone. With no setting, it uses UTC. Use an
  IANA name such as `America/Recife`, not an abbreviation such as `BRT`.

Run `ruscker validate <config>` after changing the setting. An invalid name
produces a warning and falls back to UTC without blocking boot. The startup
banner's `timezone` field shows the effective scheduler zone.

## An Editor cannot find an app or user

This is normally the group scope working as designed. An Editor sees and
administers:

- apps with no `access-groups` or `access-users`, plus restricted apps whose
  `access-groups` overlap the Editor's groups;
- non-Admin users who share at least one of those groups, and memberships in
  the Editor's own groups;
- the shared Media library.

An app restricted only by `access-users` has no group boundary and is
Admin-only. Direct requests for an out-of-scope app or user return `404`,
not `403`, so a guessed id cannot reveal another team's resource. Editors
may create Viewer/Editor accounts in their groups and reset their passwords,
but deleting accounts, importing users by CSV, resetting MFA, and
creating/renaming/deleting groups require Admin or break-glass access.

## Admin navigation hangs after visiting Logs

Upgrade to a current release. Older builds automatically opened
`/admin/logs/stream` as an infinite SSE response. On a deployment with an
HTTP/1.1 load-balancer or reverse-proxy hop, an intermediary could retain
that response after browser navigation and then reuse the same backend
connection for another request behind it, causing head-of-line blocking.

Current releases use finite cursor polling. In browser developer tools,
`GET /admin/logs/poll?cursor=…` should return JSON promptly and repeat while
the page is visible. `/admin/logs/stream` should return `204` and must not be
configured in nginx as a persistent stream. The per-replica log viewer still
uses SSE only after an operator clicks **Live**; that endpoint is unrelated
to ordinary admin navigation.

## The favicon doesn't appear in Safari (or shows a stale icon)
Safari caches favicons aggressively and sometimes keeps serving a stale
or broken icon long after you upgrade Ruscker. To force a refresh:

1. In Safari, go to **Settings → Advanced** and enable the **Develop**
   menu.
2. Open **Develop → Empty Caches**, then reload the page.
3. If that isn't enough, close all tabs pointing at the site and reopen
   them.

For iOS Safari, a full Safari data clear (**Settings → Safari → Clear
History and Website Data**) removes the icon cache.

This is a browser-side caching behaviour, not a Ruscker bug. Ruscker's
favicon markup avoids the `sizes="any"` attribute that can confuse Safari's
icon selection.

## `docker pull ghcr.io/strategicprojects/ruscker` is denied
A freshly-published image package starts **private**. Either make the
package public (Packages → the package → *Package settings* →
visibility), or authenticate: `docker login ghcr.io` with a token that
has `read:packages`.

## Jupyter or Voilà loads a blank page / 404s on assets
Ruscker uses a **strip model**: `/app/{id}` is stripped from the request
path before forwarding, so the container always sees a root-relative path.
The proxy injects a `<base href>`, rewrites static URLs in HTML responses,
and patches runtime JavaScript via a shim — most apps (Shiny, Streamlit,
Dash, Voilà) need no special configuration.

**Voilà** — no special setup is required; the generalized runtime shim
handles Voilà's RequireJS bootstrap.

**JupyterLab / Jupyter Notebook** — the proxy also rewrites the
`jupyter-config-data` JSON block (where Lab stores `baseUrl`,
`fullStaticUrl`, and related paths) so the browser loads its chunks from
under the mount. Because Ruscker strips the mount prefix before
forwarding, the container should serve at **root** (`base_url=/`) and let
the proxy do the prefixing. Configure the spec like this:

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

`--IdentityProvider.token=` disables the login token so the proxy can
forward requests without authentication, and `--ServerApp.allow_origin=*`
lets the kernel WebSocket connect. Do **not** set
`--ServerApp.base_url=#{publicPath}`: under Ruscker's strip model the
container never sees the mount prefix, so a non-root `base_url` makes
Jupyter 404 every path. See
[Sub-path handling (the strip model)](./alternatives.md#sub-path-handling-the-strip-model)
for why.

**Do not set `SHINYPROXY_PUBLIC_PATH`** in `container-env` either. That
variable is a ShinyProxy convention; Ruscker does not use it, and if a
container reads it to self-prefix URLs it will 404 on every request.

If assets still 404 after configuring the above, check
`docker logs <ruscker-container-id>` for startup errors and verify the
image's server is listening on the port you set in `container-port`
(Jupyter uses 8888; the Ruscker default is 3838, the Shiny Server port).

## Inspecting what's running
```sh
systemctl status ruscker
journalctl -u ruscker -f
curl -s localhost:8080/readyz
docker ps --filter label=ruscker.replica_id
```
