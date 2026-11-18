#!/bin/bash
# Exercise packaging/ruscker-update end to end, in a throwaway Debian
# container, against stand-in packages and a stand-in release server.
#
#   packaging/tests/update-test.sh        (needs Docker)
#
# The stand-in `ruscker` binary answers `schema-version` and serves
# /healthz + /readyz like the real one, so every branch of the updater
# runs for real: dpkg, the database copy, the health check, going back.
set -euo pipefail
HERE="$(cd "$(dirname "$0")/.." && pwd)"
exec docker run --rm -v "$HERE:/p:ro" debian:12-slim bash /p/tests/update-in-container.sh
