# syntax=docker/dockerfile:1
#
# Multi-stage build for the `ruscker` binary.
#
#   docker build -t ruscker:latest .
#   docker run --rm -p 8080:8080 \
#     -v "$PWD/application.yml:/etc/ruscker/application.yml:ro" \
#     ruscker:latest serve --config /etc/ruscker/application.yml --bind 0.0.0.0:8080
#
# To let Ruscker orchestrate containers (the --docker backend), also
# mount the Docker socket:  -v /var/run/docker.sock:/var/run/docker.sock
#
# Health probes for orchestrators: GET /healthz (liveness) and
# /readyz (readiness) — see docs/ROADMAP.md / CLAUDE.md.

# ----------------------------------------------------------------------
# Builder — compiles the release binary. The full (non-slim) rust
# image ships curl + a C toolchain, which build.rs needs: it downloads
# the standalone Tailwind CLI via curl and compiles the admin
# stylesheet, and sqlx bundles SQLite (needs cc). The pinned tag
# matches rust-toolchain.toml so the image's toolchain is used
# directly (no extra rustup download).
# ----------------------------------------------------------------------
FROM rust:1.96-bookworm AS builder

WORKDIR /build
COPY . .

# Build only the `ruscker` binary in release mode.
RUN cargo build --release --bin ruscker \
 && /build/target/release/ruscker --help >/dev/null

# ----------------------------------------------------------------------
# Runtime — a slim Debian with just the binary and the CA bundle.
# debian-slim (glibc) matches the builder's dynamic linkage; sqlite is
# statically bundled, so the only runtime dependency is
# ca-certificates (TLS when pulling app images from registries).
# ----------------------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/*

# Run as an unprivileged user. Note: using the --docker backend
# requires access to the Docker socket, which usually means adding
# this user to the host's docker group (or running with that gid).
RUN useradd --system --no-create-home --uid 10001 ruscker

COPY --from=builder /build/target/release/ruscker /usr/local/bin/ruscker

USER ruscker
EXPOSE 8080

ENTRYPOINT ["ruscker"]
CMD ["--help"]
