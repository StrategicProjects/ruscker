#!/usr/bin/env bash
# Pin transitive dependencies that require edition2024 (Rust 1.80+)
# down to versions compatible with Rust 1.75 MSRV.
#
# Run this once after `cargo generate-lockfile` or whenever cargo
# regenerates Cargo.lock and pulls in newer versions.

set -euo pipefail

cd "$(dirname "$0")/.."

# Make sure the lockfile exists
if [ ! -f Cargo.lock ]; then
    echo "→ generating lockfile"
    cargo generate-lockfile
fi

echo "→ pinning uuid to 1.10.0"
cargo update -p uuid --precise 1.10.0 || true

echo "→ pinning getrandom to 0.2.15"
cargo update -p getrandom --precise 0.2.15 || true
# If two versions resolve, also pin the 0.3 line:
cargo update -p getrandom@0.3.4 --precise 0.2.15 2>/dev/null || true

echo "→ pinning indexmap to 2.7.0"
cargo update -p indexmap@2.14.0 --precise 2.7.0 2>/dev/null || true

echo "→ pinning clap_derive to 4.5.18"
cargo update -p clap_derive --precise 4.5.18 || true

echo "→ pinning clap to 4.5.20"
cargo update -p clap --precise 4.5.20 || true

echo "✓ done. You can now run: cargo build"
