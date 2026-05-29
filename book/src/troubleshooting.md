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
auto-discovers it. Check: `curl -I http://localhost:8090/assets/img/<file>`.
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
`sudo dpkg -i --force-confold ruscker_<version>_amd64.deb && sudo
systemctl restart ruscker`.

## `413 Payload Too Large` on an upload
A `max-body-size` cap is in effect (global `proxy.max-body-size` or a
per-spec override). Raise it for that spec or globally.

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
sudo apt-get install -y ./ruscker_<version>_amd64.deb
```

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

## `docker pull ghcr.io/strategicprojects/ruscker` is denied
A freshly-published image package starts **private**. Either make the
package public (Packages → the package → *Package settings* →
visibility), or authenticate: `docker login ghcr.io` with a token that
has `read:packages`.

## Inspecting what's running
```sh
systemctl status ruscker
journalctl -u ruscker -f
curl -s localhost:8090/readyz
docker ps --filter label=ruscker.replica_id
```
