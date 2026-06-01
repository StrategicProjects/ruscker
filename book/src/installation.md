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
- drops an example config at `/etc/ruscker/application.yml` and a
  secrets file at `/etc/ruscker/ruscker.env`,
- **generates a unique admin token on first install and prints it
  once** (there is no default password).

```sh
systemctl status ruscker
curl http://localhost:8080/healthz
sudo grep RUSCKER_ADMIN_TOKEN /etc/ruscker/ruscker.env   # your admin token
```

Edit `/etc/ruscker/application.yml`, put secrets in
`/etc/ruscker/ruscker.env`, then `sudo systemctl restart ruscker`. See
[Deploying in production](./deploying.md) to enable the `--docker`
backend and put it behind nginx.

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

## Docker

```sh
docker run --rm -p 8080:8080 \
  -v "$PWD/application.yml:/etc/ruscker/application.yml:ro" \
  ghcr.io/strategicprojects/ruscker:latest \
  serve --config /etc/ruscker/application.yml --bind 0.0.0.0:8080
```

To let Ruscker spawn app containers (the `--docker` backend), also
mount the Docker socket and add `--docker`:

```sh
docker run --rm -p 8080:8080 \
  -v "$PWD/application.yml:/etc/ruscker/application.yml:ro" \
  -v /var/run/docker.sock:/var/run/docker.sock \
  ghcr.io/strategicprojects/ruscker:latest \
  serve --config /etc/ruscker/application.yml --bind 0.0.0.0:8080 --docker
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
ruscker serve --config <path> [--bind 0.0.0.0:8080] [--docker]
              [--db <file>] [--images-dir <dir>] [--log-format json]
              [--base-path <prefix>]
```

| Flag | What it does |
|---|---|
| `--config` | Path to the `application.yml`. |
| `--bind` | Listen address (defaults to the YAML's `proxy.port`). |
| `--docker` | Enable the container backend (spawn app containers). |
| `--db` | SQLite file backing the admin panel. Without it, `/admin/*` is read-only-ish and the editor returns 503. |
| `--images-dir` | Directory served at `/assets/img/`. Auto-discovered from the config / ShinyProxy `template-path` when omitted. |
| `--log-format` | `text` (default) or `json`. |
| `--base-path` | Mount the whole portal under a URL prefix (e.g. `--base-path /apps`). Overrides `server.context-path` in the YAML. Health probes (`/healthz`, `/readyz`) stay at the root. |

Secrets come from the environment: `RUSCKER_ADMIN_TOKEN`,
`RUSCKER_MASTER_KEY`, `RUSCKER_COOKIE_KEY`,
`DOCKER_REGISTRY_PASSWORD`.
