#!/usr/bin/env bash
#
# End-to-end validation of Ruscker's interactive-app support
# against a REAL container that serves HTML, static assets, a
# JSON endpoint, AND a WebSocket — the four things a Shiny /
# Streamlit / Dash app exercises behind the proxy.
#
# What it proves:
#   1. HTML transform — <base href> + JS shim + absolute-attr
#      rewriting land in the served page (#21/#27/#28).
#   2. Asset round-trip — a rewritten /app/{spec}/static/x path
#      routes back through the proxy and serves the asset.
#   3. JSON endpoint — a rewritten /app/{spec}/api/ping returns.
#   4. WebSocket proxy — ws::pump bidirectional echo works with
#      a genuine upstream WebSocket server (Phase 3).
#
# What it does NOT prove: the JS shim executing in a real
# browser (runtime monkey-patching). That needs a headless
# browser; out of scope for a shell harness.
#
# Requires: Docker daemon, a built `ruscker` binary, python3.
# Usage:    ./scripts/e2e-interactive.sh
# Exit 0 = all four checks passed.

set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/.." && pwd)"
APPDIR="$ROOT/examples/e2e-interactive-app"
IMAGE="ruscker-wsapp:test"
BIND="127.0.0.1:8082"
BASEURL="http://$BIND"
PIDFILE=""
FAILED=0

log()  { printf '\033[36m==>\033[0m %s\n' "$*"; }
pass() { printf '  \033[32mPASS\033[0m %s\n' "$*"; }
fail() { printf '  \033[31mFAIL\033[0m %s\n' "$*"; FAILED=1; }

cleanup() {
  [ -n "$PIDFILE" ] && kill -9 "$PIDFILE" 2>/dev/null || true
  docker ps -a --filter "label=ruscker.replica_id" --format '{{.ID}}' \
    | xargs -r docker rm -f >/dev/null 2>&1 || true
}
trap cleanup EXIT

# ── locate the ruscker binary ──────────────────────────────
RUSCKER="$ROOT/target/debug/ruscker"
[ -x "$RUSCKER" ] || RUSCKER="$ROOT/target/release/ruscker"
if [ ! -x "$RUSCKER" ]; then
  echo "ruscker binary not found — run 'cargo build --bin ruscker' first" >&2
  exit 2
fi

log "Building test image $IMAGE"
docker build -q -t "$IMAGE" "$APPDIR" >/dev/null

log "Starting ruscker (--docker) on $BIND"
DOCKER_REGISTRY_PASSWORD=test \
  "$RUSCKER" serve --config "$APPDIR/wsapp.yml" --bind "$BIND" --docker \
  >/tmp/e2e-interactive.log 2>&1 &
PIDFILE=$!

# Wait for the landing page to answer.
for _ in $(seq 1 10); do
  curl -s -o /dev/null --max-time 1 "$BASEURL/" && break
  sleep 1
done
# Wait for the scaler to bring the replica up and ready.
for _ in $(seq 1 15); do
  if docker ps --filter "label=ruscker.replica_id" --format '{{.Status}}' | grep -q Up; then
    break
  fi
  sleep 1
done
sleep 3

# ── 1. HTML transform ──────────────────────────────────────
log "1. HTML transform"
PAGE="$(curl -s "$BASEURL/app/wsapp/")"
for needle in \
  '<base href="/app/wsapp/">' \
  'src="/app/wsapp/static/app.js"' \
  'href="/app/wsapp/static/app.css"' \
  'window.WebSocket' \
  'XMLHttpRequest.prototype.open'; do
  if grep -qF "$needle" <<<"$PAGE"; then pass "$needle"; else fail "$needle"; fi
done

# ── 2. asset round-trip ────────────────────────────────────
log "2. Asset round-trip"
CODE="$(curl -s -o /dev/null -w '%{http_code}' "$BASEURL/app/wsapp/static/app.js")"
if [ "$CODE" = "200" ]; then pass "GET /app/wsapp/static/app.js -> 200"; else fail "app.js -> $CODE"; fi

# ── 3. JSON endpoint ───────────────────────────────────────
log "3. JSON endpoint"
PING="$(curl -s "$BASEURL/app/wsapp/api/ping")"
if grep -qF '"pong"' <<<"$PING"; then pass "api/ping -> $PING"; else fail "api/ping -> $PING"; fi

# ── 4. WebSocket proxy ─────────────────────────────────────
log "4. WebSocket proxy"
WS="$(python3 "$APPDIR/wsclient.py" 127.0.0.1 8082 /app/wsapp/ws hello)"
if grep -qF 'echo:hello' <<<"$WS"; then pass "ws echo -> $WS"; else fail "ws echo -> $WS"; fi

echo
if [ "$FAILED" -eq 0 ]; then
  printf '\033[32mALL CHECKS PASSED\033[0m\n'
else
  printf '\033[31mSOME CHECKS FAILED\033[0m (see /tmp/e2e-interactive.log)\n'
fi
exit "$FAILED"
