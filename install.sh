#!/bin/sh
# Ruscker installer — fetches the right artifact from the latest GitHub
# release and installs it. Debian/Ubuntu hosts get the .deb (systemd
# unit + auto-generated admin token); everything else gets the static
# musl binary into PREFIX (default /usr/local/bin).
#
#   curl -fsSL https://raw.githubusercontent.com/StrategicProjects/ruscker/main/install.sh | sh
#   # or pin a version / target dir:
#   ./install.sh v0.1.0
#   PREFIX="$HOME/.local/bin" ./install.sh
set -eu

REPO="StrategicProjects/ruscker"

[ "$(uname -s)" = "Linux" ] || {
    echo "Ruscker ships Linux binaries only (got $(uname -s))." >&2
    exit 1
}
case "$(uname -m)" in
x86_64 | amd64) ARCH=amd64 ;;
aarch64 | arm64) ARCH=arm64 ;;
*)
    echo "unsupported architecture: $(uname -m)" >&2
    exit 1
    ;;
esac

# Version: first argument, else the latest release tag.
VERSION="${1:-}"
if [ -z "$VERSION" ]; then
    VERSION=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" |
        grep -oE '"tag_name"[[:space:]]*:[[:space:]]*"[^"]+"' | head -1 |
        sed -E 's/.*"([^"]+)"$/\1/')
fi
[ -n "$VERSION" ] || {
    echo "could not determine the latest version — pass one, e.g. ./install.sh v0.1.0" >&2
    exit 1
}
VER="${VERSION#v}"
BASE="https://github.com/$REPO/releases/download/$VERSION"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fetch() {
    echo "↓ $1" >&2
    curl -fSL "$1" -o "$2"
}
verify() { (cd "$(dirname "$1")" && sha256sum -c "$(basename "$1").sha256"); }

maybe_sudo() {
    if [ "$(id -u)" -eq 0 ]; then "$@"; else sudo "$@"; fi
}

if command -v dpkg >/dev/null 2>&1; then
    DEB="ruscker_${VER}-1_${ARCH}.deb"
    fetch "$BASE/$DEB" "$TMP/$DEB"
    fetch "$BASE/$DEB.sha256" "$TMP/$DEB.sha256"
    verify "$TMP/$DEB"
    echo "Installing $DEB (may prompt for sudo)…" >&2
    maybe_sudo apt-get install -y "$TMP/$DEB" || maybe_sudo dpkg -i "$TMP/$DEB"
    echo "✓ ruscker installed. Your admin token:" >&2
    echo "    sudo grep RUSCKER_ADMIN_TOKEN /etc/ruscker/ruscker.env" >&2
else
    TGZ="ruscker-${VER}-linux-${ARCH}.tar.gz"
    fetch "$BASE/$TGZ" "$TMP/$TGZ"
    fetch "$BASE/$TGZ.sha256" "$TMP/$TGZ.sha256"
    verify "$TMP/$TGZ"
    tar -C "$TMP" -xzf "$TMP/$TGZ"
    BIN="$TMP/ruscker-${VER}-linux-${ARCH}/ruscker"
    DEST="${PREFIX:-/usr/local/bin}"
    echo "Installing ruscker → $DEST/ruscker (may prompt for sudo)…" >&2
    if [ -w "$DEST" ]; then
        install -m 0755 "$BIN" "$DEST/ruscker"
    else
        maybe_sudo install -m 0755 "$BIN" "$DEST/ruscker"
    fi
    echo "✓ ruscker installed: $("$DEST/ruscker" --version 2>/dev/null || echo "$DEST/ruscker")" >&2
fi
