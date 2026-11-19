#!/bin/bash
# Start a throwaway Ruscker with the OIDC provider on, seeded with one
# person and one SSO client, for testing the client packages end to end.
#
#   clients/testing/provider.sh /path/to/ruscker [workdir]
#
# It stays in the foreground; stop it with Ctrl-C. Everything a test
# needs is written to <workdir>/env (source it):
#
#   RUSCKER_ISSUER         http://127.0.0.1:18200/box
#   RUSCKER_CLIENT_ID      demo
#   RUSCKER_CLIENT_SECRET  (generated)
#   RUSCKER_REDIRECT_URI   http://127.0.0.1:18299/cb   (nothing listens there)
#   RUSCKER_TEST_COOKIES   cookie jar of the signed-in test person
#
# The person is `ana` — full name "Ana Maria de Souza", e-mail
# ana@example.org, setor GAPE, groups fiscal and outro, roles
# analista+gestor in fiscal and leitor in outro. The client `demo` is
# restricted to the group `fiscal`, so tokens name only that group.
#
# `authorize.sh` plays the browser: give it an authorization URL and it
# prints where the provider redirects the signed-in person.
set -euo pipefail
BIN="${1:?usage: provider.sh /path/to/ruscker [workdir]}"
DIR="${2:-$(mktemp -d)}"
PORT="${RUSCKER_TEST_PORT:-18200}"
mkdir -p "$DIR"; rm -f "$DIR"/dev.db* "$DIR/env"
O="http://127.0.0.1:$PORT"; B="$O/box"; A="$DIR/admin.cookies"; U="$DIR/ana.cookies"
REDIRECT="http://127.0.0.1:18299/cb"
# A return address after sign-out (since 0.2.0 of the packages).
SIGNED_OUT="${REDIRECT%/*}/signed-out"
cat > "$DIR/dev.yml" <<YML
server:
  oidc-provider:
    enabled: true
    issuer: $B
proxy:
  title: SSO test portal
  port: $PORT
  specs: []
YML
export RUSCKER_ADMIN_TOKEN=devtoken DOCKER_REGISTRY_PASSWORD=unused
export RUSCKER_COOKIE_KEY="$(openssl rand -hex 32)" RUSCKER_MASTER_KEY="$(openssl rand -hex 32)"
"$BIN" serve --config "$DIR/dev.yml" --db "$DIR/dev.db" --bind "127.0.0.1:$PORT" --base-path /box > "$DIR/server.log" 2>&1 &
PID=$!
trap 'kill $PID 2>/dev/null || true' EXIT INT TERM
for _ in $(seq 1 30); do curl -s -o /dev/null "$B/healthz" && break; sleep 0.5; done
H=(-H "Origin: $O" -H "Sec-Fetch-Site: same-origin")
post() { curl -s -o /dev/null -b "$A" -c "$A" "${H[@]}" "$@"; }
PW1='CorrectPass9!'; PW2='Another#Pass7'
post -d 'token=devtoken' "$B/admin/login/token"
post --data-urlencode username=root --data-urlencode "password=$PW1" --data-urlencode "confirm=$PW1" "$B/admin/setup"
post --data-urlencode username=ana --data-urlencode "password=$PW1" --data-urlencode role=viewer \
     --data-urlencode 'groups=fiscal,outro' "$B/admin/users"
post --data-urlencode 'full_name=Ana Maria de Souza' --data-urlencode role=viewer --data-urlencode 'groups=fiscal,outro' \
     --data-urlencode setor=GAPE --data-urlencode email=ana@example.org --data-urlencode celular= "$B/admin/users/ana/edit"
for gr in fiscal:analista fiscal:gestor outro:leitor; do
  post --data-urlencode "group=${gr%%:*}" --data-urlencode "role=${gr##*:}" "$B/admin/groups/roles/define"
  post --data-urlencode "group=${gr%%:*}" --data-urlencode "role=${gr##*:}" --data-urlencode username=ana "$B/admin/groups/roles/assign"
done
SECRET=$(curl -s -b "$A" "${H[@]}" --data-urlencode client_id=demo --data-urlencode name=Demo \
  --data-urlencode "redirect_uris=$REDIRECT" --data-urlencode access_groups=fiscal \
  --data-urlencode "post_logout_redirect_uris=$SIGNED_OUT" "$B/admin/sso-clients" \
  | grep -o 'data-sso-secret-value[^<]*<' | sed -E 's/.*>([0-9a-f]{64})</\1/')
[ ${#SECRET} = 64 ] || { echo "could not register the client; see $DIR/server.log" >&2; exit 1; }
# Sign the person in and get the forced first password change out of the way.
curl -s -o /dev/null -c "$U" "${H[@]}" --data-urlencode username=ana --data-urlencode "password=$PW1" "$B/admin/login"
curl -s -o /dev/null -b "$U" -c "$U" "${H[@]}" --data-urlencode "current=$PW1" --data-urlencode "new_password=$PW2" \
     --data-urlencode "confirm=$PW2" "$B/admin/account/password"
cat > "$DIR/env" <<ENV
export RUSCKER_ISSUER=$B
export RUSCKER_CLIENT_ID=demo
export RUSCKER_CLIENT_SECRET=$SECRET
export RUSCKER_REDIRECT_URI=$REDIRECT
export RUSCKER_POST_LOGOUT_URI=$SIGNED_OUT
export RUSCKER_TEST_COOKIES=$U
ENV
echo "provider ready at $B — source $DIR/env"
wait $PID
