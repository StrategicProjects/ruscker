# Installation

Ruscker is a single binary. Pick the packaging that fits your host.

## Debian / Ubuntu (`.deb`)

The most ShinyProxy-like install: a systemd service on your Docker
host. Packages for **amd64** and **arm64** are attached to every
[GitHub release](https://github.com/StrategicProjects/ruscker/releases).

```sh
# amd64
sudo apt install ./ruscker_<version>-1_amd64.deb

# arm64
sudo apt install ./ruscker_<version>-1_arm64.deb
```

This:

- installs `/usr/bin/ruscker`,
- creates a `ruscker` system user,
- installs a hardened `ruscker.service` unit and enables + starts it,
- drops a minimal config at `/etc/ruscker/ruscker.yml` and a
  secrets file at `/etc/ruscker/ruscker.env`,
- **generates a unique admin token on first install and prints it
  once** (there is no default password),
- generates stable master and cookie keys without overwriting existing
  values, so encrypted credentials, MFA and sticky sessions survive restarts,
- runs with the **admin catalog enabled** (`--db
  /var/lib/ruscker/ruscker.db`), so the admin panel works out of the box
  and a set of showcase apps seeds on first boot.

```sh
systemctl status ruscker
curl http://localhost:8080/healthz
sudo grep RUSCKER_ADMIN_TOKEN /etc/ruscker/ruscker.env   # your admin token
```

Log in at `/admin` with the printed token to manage **apps, the landing
page and users** — that's where day-to-day configuration lives (stored
in the catalog DB, not the YAML). `ruscker.yml` holds **deployment**
settings; put secrets in `/etc/ruscker/ruscker.env`. After editing either
file, `sudo systemctl restart ruscker`.

To actually run the demo apps (and your own containers) enable the Docker
backend: `sudo ruscker-enable-docker`. See [Deploying in
production](./deploying.md) for nginx + TLS, and
[Configuration](./configuration.md) for what lives where.

### Uninstall & reset

Pick the scope you want:

| Goal | Command | What's left |
|---|---|---|
| **Remove the software**, keep config + data | `sudo apt remove ruscker` | `/etc/ruscker` (config, admin token, keys) and `/var/lib/ruscker` (catalog DB) — a reinstall resumes where you left off. |
| **Remove everything** (no trace) | `sudo apt purge ruscker` | Nothing. Drops the catalog DB **and** `/etc/ruscker` — including `ruscker.yml` and the admin token / master key in `ruscker.env`. |
| **Reset to a brand-new install** | `sudo apt purge ruscker && sudo apt install ./ruscker_<version>-1_amd64.deb` | A pristine box: the default `ruscker.yml` (no custom title/specs) and freshly generated keys and admin token, exactly like a first install. |
| **Wipe the data only**, keep config | stop, remove the DB, start (below) | Config + token unchanged; the catalog is empty, so migrations re-run and the showcase cards re-seed on next boot. |

To wipe just the catalog (a "fresh portal" without uninstalling):

```sh
sudo systemctl stop ruscker
sudo rm -f /var/lib/ruscker/ruscker.db          # + -wal/-shm if present
sudo systemctl start ruscker
```

> The catalog DB lives in `/var/lib/ruscker`; your config and secrets
> live in `/etc/ruscker`. `purge` clears both; removing the DB clears
> only the catalog. The portal **title** comes from `proxy.title` in
> `ruscker.yml`, so it survives a data-only wipe — only a `purge` (or
> editing the file) changes it.

## Static musl tarball

For hosts without a package manager, or for quick installs without a
systemd unit, download the static musl binary directly. Tarballs for
**amd64** and **arm64** are on the
[releases page](https://github.com/StrategicProjects/ruscker/releases).

```sh
tar -xzf ruscker-<version>-linux-amd64.tar.gz
sudo install -m 755 ruscker-<version>-linux-amd64/ruscker /usr/local/bin/ruscker
ruscker --version
```

The binary has no shared-library dependencies and runs on any
glibc-free or glibc Linux system.

## Homebrew (macOS / Linux)

For a local install — handy on a macOS workstation for development — use
the tap:

```sh
brew install strategicprojects/tap/ruscker
ruscker --version
```

On Linux the formula pulls the static musl binary from the matching
release; on macOS it builds from source (so a Rust toolchain is fetched
as a build dependency). Each release auto-publishes its formula to the
tap, so `brew upgrade` tracks the latest version.

> Ruscker spawns **Linux containers**, so the `--docker` backend needs a
> Linux Docker host. A macOS Homebrew install is meant for running the
> portal locally and for development, not for hosting app containers.

## Docker

```sh
docker run --rm -p 8080:8080 \
  -v "$PWD/ruscker.yml:/etc/ruscker/ruscker.yml:ro" \
  ghcr.io/strategicprojects/ruscker:latest \
  serve --config /etc/ruscker/ruscker.yml --bind 0.0.0.0:8080
```

To let Ruscker spawn app containers (the `--docker` backend), also
mount the Docker socket and add `--docker`:

```sh
docker run --rm -p 8080:8080 \
  -v "$PWD/ruscker.yml:/etc/ruscker/ruscker.yml:ro" \
  -v /var/run/docker.sock:/var/run/docker.sock \
  ghcr.io/strategicprojects/ruscker:latest \
  serve --config /etc/ruscker/ruscker.yml --bind 0.0.0.0:8080 --docker
```

> Mounting the Docker socket grants control of the host's Docker
> daemon — the same trade-off ShinyProxy carries. Prefer the `.deb`
> on the Docker host when you can.

## From source

Requires Rust (the pinned toolchain installs automatically via
`rust-toolchain.toml`):

```sh
cargo build --release --bin ruscker
./target/release/ruscker --help
```

Development and release builds use Rust 1.96.0 from
`rust-toolchain.toml`. The MSRV source of truth is
`workspace.package.rust-version` in `Cargo.toml` (currently 1.94.0); the
dedicated `msrv` workflow reads that field and checks the locked graph.

## Verifying release artifacts

Every tagged release is signed with [cosign](https://docs.sigstore.dev/)
using GitHub Actions OIDC (keyless — no public key to fetch). Each
asset ships a `.sha256` plus a `.sig` + `.pem` (signing certificate);
the container image is signed by digest.

```sh
# Container image
cosign verify ghcr.io/strategicprojects/ruscker:<version> \
  --certificate-identity-regexp '^https://github.com/StrategicProjects/ruscker/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com

# A downloaded asset (tarball or .deb)
cosign verify-blob ruscker-<version>-linux-amd64.tar.gz \
  --signature ruscker-<version>-linux-amd64.tar.gz.sig \
  --certificate ruscker-<version>-linux-amd64.tar.gz.pem \
  --certificate-identity-regexp '^https://github.com/StrategicProjects/ruscker/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

The exact commands are also printed in each release's notes.

## The `serve` command

```text
ruscker serve [--config <path>] [--bind 0.0.0.0:8080] [--docker|--no-docker]
              [--db <file>] [--config-db-url <postgres-url>]
              [--images-dir <dir>] [--log-format json]
              [--base-path <prefix>]
```

| Flag | What it does |
|---|---|
| `--config` | Path to the service config. When omitted, Ruscker prefers `ruscker.yml` in the working directory and falls back to `application.yml` for compatibility. |
| `--bind` | Listen address (defaults to the YAML's `proxy.port`). |
| `--docker` / `--no-docker` | By default Ruscker auto-connects when the local daemon socket is reachable. `--docker` makes connection failure fatal; `--no-docker` forces landing-only mode. |
| `--db` | SQLite file backing the admin panel. Without it or `--config-db-url`, `/admin/*` returns 503. |
| `--config-db-url` | PostgreSQL URL backing the admin panel and shared catalog in HA deployments. |
| `--images-dir` | Directory served at `/assets/img/`. Auto-discovered from the config / ShinyProxy `template-path` when omitted. |
| `--log-format` | `text` (default) or `json`. |
| `--base-path` | Mount the whole portal under a URL prefix (e.g. `--base-path /apps`). Overrides `server.context-path` in the YAML. Health probes (`/healthz`, `/readyz`) stay at the root. |

Secrets come from the environment: `RUSCKER_ADMIN_TOKEN`,
`RUSCKER_MASTER_KEY`, `RUSCKER_COOKIE_KEY`,
`DOCKER_REGISTRY_PASSWORD`. `RUSCKER_MASTER_KEY` is required for encrypted
credentials and 2FA enrolment; keep it stable and back it up.
