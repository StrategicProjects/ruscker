# Troubleshooting

## `/admin` returns 503 "RUSCKER_ADMIN_TOKEN is not set"
No admin token is configured. Set `RUSCKER_ADMIN_TOKEN` (the `.deb`
generates one — `sudo grep RUSCKER_ADMIN_TOKEN /etc/ruscker/ruscker.env`)
and restart. The admin pages also need `serve --db <file>`; without it
the editor/list screens return 503.

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
  verify registry credentials. Private images need
  `docker-registry-username` + `docker-registry-password` (the latter
  via `${DOCKER_REGISTRY_PASSWORD}`).

## A Shiny app loads but the page is broken / no live updates
Shiny needs WebSockets. Make sure your reverse proxy forwards the
upgrade headers (`Upgrade` / `Connection "upgrade"`) — see the nginx
snippet in [Deploying](./deploying.md).

## The admin shows the wrong / old features after an upgrade
Templates are compiled into the binary, so changes need a **rebuild +
reinstall**, not just editing files on the server:
`sudo dpkg -i --force-confold ruscker_<version>-1_amd64.deb && sudo
systemctl restart ruscker`.

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

Since v0.2.5 Ruscker only honours `X-Forwarded-Proto` when
`server.useForwardHeaders: true` is set (otherwise any client could
spoof it). Behind a TLS-terminating reverse proxy you need **both**:
the proxy sending `proxy_set_header X-Forwarded-Proto https;` (or
`$scheme`) *and* the YAML flag. Check with
`curl -sI -H "Accept: text/html" https://your-host/app/<id>/ | grep -i set-cookie`
— the sticky cookie should list `Secure`.

## A Streamlit / Dash / Voilà app is unreachable (connection refused upstream)

Before v0.2.5, `type: streamlit|dash|voila` without an explicit
`container-port` forwarded to Shiny's 3838 — these frameworks listen on
8501 / 8050 / 8866, so the proxy hit a closed port. v0.2.5 defaults to
the framework's port; on older versions (or for a non-standard port)
set `container-port` explicitly.

## Users bounce between replicas / lose their session after a restart
The sticky-session cookie is signed with `RUSCKER_COOKIE_KEY`. If you
don't set it, Ruscker generates a random key on each start — so every
restart invalidates existing session cookies and can scatter users
across replicas. Set a stable `RUSCKER_COOKIE_KEY` (e.g.
`openssl rand -hex 32`) in `ruscker.env` and keep it constant.

## The dashboard doesn't update live (or lags badly)
The dashboard streams over Server-Sent Events. A reverse proxy that
buffers the response will hold the stream back. Disable buffering for
`/admin/dashboard/events` — see the nginx note in
[Deploying](./deploying.md).

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

This is a browser-side caching behaviour, not a Ruscker bug. The favicon
markup was hardened in v0.1.18 (the `sizes="any"` attribute that confused
Safari's icon selection was removed).

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
