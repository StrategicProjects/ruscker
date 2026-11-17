#!/bin/bash
# Play the browser of the signed-in test person: request an authorization
# URL and print the URL the provider redirects to (the client's callback,
# with `code`, `state` and `iss` — or with `error`).
#
#   source <workdir>/env
#   clients/testing/authorize.sh 'http://127.0.0.1:18200/box/oauth/authorize?…'
set -euo pipefail
: "${RUSCKER_TEST_COOKIES:?source the env file written by provider.sh first}"
curl -s -o /dev/null -w '%{redirect_url}\n' -b "$RUSCKER_TEST_COOKIES" "${1:?usage: authorize.sh <authorization URL>}"
