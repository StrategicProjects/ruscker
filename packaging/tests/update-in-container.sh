#!/bin/bash
# Runs INSIDE the container started by update-test.sh.
#
# The stand-in packages carry a `ruscker` that behaves like the real one
# where the updater can tell: it answers `schema-version`, "migrates" the
# database on start (raising a `schema N` line, never lowering it — like
# the real one, an older binary does not undo a migration), and serves
# /healthz + /readyz. So a restore that did not happen shows.
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq >/dev/null && apt-get install -y -qq curl python3 >/dev/null 2>&1

W=/work; REL=$W/releases/StrategicProjects/ruscker/releases
mkdir -p "$REL" /var/lib/ruscker /etc/ruscker
useradd --system ruscker
ARCH="$(dpkg --print-architecture)"
DBF=/var/lib/ruscker/ruscker.db

# ── stand-ins for systemd ───────────────────────────────────────────
cat > /usr/bin/systemctl <<'SH'
#!/bin/bash
case "$1" in
  cat) echo "ExecStart=/usr/bin/ruscker serve --bind 127.0.0.1:8080 \\"
       echo "    --db /var/lib/ruscker/ruscker.db"
       cat /work/unit-extra 2>/dev/null ;;
  stop) [ -f /run/ruscker.pid ] && kill "$(cat /run/ruscker.pid)" 2>/dev/null; rm -f /run/ruscker.pid; sleep 0.3 ;;
  start) [ -f /run/ruscker.pid ] && kill -0 "$(cat /run/ruscker.pid)" 2>/dev/null && exit 0
         setsid /usr/bin/ruscker serve >/dev/null 2>&1 & echo $! > /run/ruscker.pid ;;
esac
exit 0
SH
printf '#!/bin/bash\nexit 0\n' > /usr/bin/journalctl
chmod +x /usr/bin/systemctl /usr/bin/journalctl

# ── a stand-in package ──────────────────────────────────────────────
# mkdeb <version> <schema, "" = too old to say> [how it misbehaves]
#   broken        exits instead of serving
#   wrongversion  serves, but /healthz names another version
#   notready      serves, /readyz answers 503 — and scribbles on the database
#   preinst       the package refuses to install
mkdeb() {
  local v="$1" schema="$2" how="${3:-ok}" d="$W/build-$1"
  local served="$v" ready=200
  [ "$how" = wrongversion ] && served=9.9.9
  [ "$how" = notready ] && ready=503
  rm -rf "$d"; mkdir -p "$d/DEBIAN" "$d/usr/bin" "$d/etc/ruscker" "$REL/download/v$v"
  printf 'Package: ruscker\nVersion: %s-1\nArchitecture: %s\nMaintainer: t <t@example.org>\nDescription: stand-in\n' "$v" "$ARCH" > "$d/DEBIAN/control"
  echo "shipped by $v" > "$d/etc/ruscker/ruscker.yml"
  echo /etc/ruscker/ruscker.yml > "$d/DEBIAN/conffiles"
  if [ "$how" = preinst ]; then printf '#!/bin/sh\nexit 1\n' > "$d/DEBIAN/preinst"; chmod 755 "$d/DEBIAN/preinst"; fi
  cat > "$d/usr/bin/ruscker" <<SH
#!/bin/bash
db=$DBF
case "\$1" in
  schema-version) [ -n "$schema" ] && echo "$schema" || exit 2 ;;
  serve)
    if [ -n "$schema" ]; then
      cur="\$(sed -n 's/^schema //p' "\$db" 2>/dev/null)"
      if [ -z "\$cur" ] || [ "$schema" -gt "\$cur" ]; then
        { grep -v '^schema ' "\$db" 2>/dev/null; echo "schema $schema"; } > "\$db.new"; mv "\$db.new" "\$db"
      fi
    fi
    [ "$how" = notready ] && echo "scribbled by $v" >> "\$db"
    [ "$how" = broken ] && exit 1
    exec python3 -c '
import http.server
class H(http.server.BaseHTTPRequestHandler):
    def do_GET(self):
        ready = self.path.startswith("/readyz")
        self.send_response($ready if ready else 200); self.end_headers()
        self.wfile.write(b"{\"status\":\"ok\",\"version\":\"$served\"}")
    def log_message(self,*a): pass
http.server.HTTPServer(("127.0.0.1",8080),H).serve_forever()' ;;
esac
SH
  chmod 755 "$d/usr/bin/ruscker"
  local f="ruscker_$v-1_$ARCH.deb"
  dpkg-deb -b "$d" "$REL/download/v$v/$f" >/dev/null
  (cd "$REL/download/v$v" && sha256sum "$f" > "$f.sha256")
}
mkdeb 1.0.0 ""            # too old to know `schema-version`
mkdeb 1.0.1 10
mkdeb 1.0.2 10            # no database change
mkdeb 1.0.3 11            # migrates
mkdeb 1.0.4 12 broken     # migrates and does not come up
mkdeb 1.0.5 12 wrongversion
mkdeb 1.0.6 11 notready   # says it does not migrate, yet touches the database
mkdeb 1.0.7 11 preinst
mkdeb 1.0.8 12            # a good one, migrating
mkdeb 1.0.9 12

# ── a stand-in release server: /releases/latest redirects to the tag ─
echo 1.0.1 > $W/latest
cat > $W/server.py <<'PY'
import http.server, os
class H(http.server.SimpleHTTPRequestHandler):
    def do_GET(self):
        if self.path.endswith("/releases/latest"):
            v = open("/work/latest").read().strip()
            self.send_response(302)
            self.send_header("Location", f"http://127.0.0.1:9000/StrategicProjects/ruscker/releases/tag/v{v}")
            self.end_headers(); return
        if "/releases/tag/" in self.path:
            self.send_response(200); self.end_headers(); return
        super().do_GET()
    def log_message(self,*a): pass
os.chdir("/work/releases")
http.server.HTTPServer(("127.0.0.1",9000),H).serve_forever()
PY
python3 $W/server.py & sleep 1
conf() { { printf 'SOURCE=http://127.0.0.1:9000\nHEALTH_WAIT=5\n'; printf '%s\n' "$@"; } > $W/update.conf; }
conf
U="bash /p/ruscker-update --conf $W/update.conf"

pass=0; fail=0
check() { # description, expected, actual
  if [ "$2" = "$3" ]; then pass=$((pass+1)); echo "  ok    $1"; else fail=$((fail+1)); echo "  FAIL  $1 — expected '$2', got '$3'"; fi
}
installed() { dpkg-query -W -f='${Version}' ruscker; }
field() { grep "^$1=" /var/lib/ruscker-update/status | cut -d= -f2-; }
result() { field result; }
serving() { # the version being served; allows a just-started service a few seconds
  local i v
  for i in 1 2 3 4 5 6; do
    v="$(curl -s -m 3 http://127.0.0.1:8080/healthz | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' || true)"
    if [ -n "$v" ]; then echo "$v"; return; fi
    sleep 1
  done
  echo none
}
set_latest() { echo "$1" > $W/latest; }
db() { tr '\n' '|' < $DBF; }
copies() { ls -1d /var/backups/ruscker/pre-* 2>/dev/null | wc -l | tr -d ' '; }
run() { rc=0; "$@" >$W/out 2>&1 || rc=$?; }

dpkg -i "$REL/download/v1.0.0/ruscker_1.0.0-1_$ARCH.deb" >/dev/null
echo "edited by the operator" > /etc/ruscker/ruscker.yml
echo "the operator's data" > $DBF
systemctl start ruscker; sleep 1

echo "1. --check changes nothing"
run $U --check; check "still on 1.0.0" 1.0.0-1 "$(installed)"; check "recorded as available" available "$(result)"

echo "2. a binary too old to state its schema counts as a migration: held"
run $U --apply; check "not installed" 1.0.0-1 "$(installed)"; check "recorded as held" held "$(result)"

echo "3. …and installs when allowed"
run $U --apply --allow-migrations; check "exit code 0" 0 "$rc"; check "on 1.0.1" 1.0.1-1 "$(installed)"; check "serving 1.0.1" 1.0.1 "$(serving)"
check "recorded" updated "$(result)"; check "the record names the new version" 1.0.1-1 "$(field installed)"

echo "4. up to date"
run $U --apply; check "recorded" up-to-date "$(result)"

echo "5. the timer in MODE=check only looks"
set_latest 1.0.2; run $U --timer; check "still on 1.0.1" 1.0.1-1 "$(installed)"; check "recorded" available "$(result)"

echo "6. the timer in MODE=auto, outside its window, waits"
h=$((10#$(date +%H)))
conf MODE=auto "WINDOW=$(( (h + 2) % 24 ))-$(( (h + 3) % 24 ))"
run $U --timer; check "still on 1.0.1" 1.0.1-1 "$(installed)"; check "recorded" available "$(result)"

echo "7. a window that cannot be read stops the update instead of opening it"
for w in abc-def -05 01-0x 5 05-05 1-5-9; do
  conf MODE=auto "WINDOW=$w"; run $U --timer
  check "WINDOW=$w: exit 1, not installed, recorded" "1 1.0.1-1 error" "$rc $(installed) $(result)"
done

echo "8. the timer in MODE=auto installs a release without database changes"
conf MODE=auto "WINDOW=$h-$(( h + 1 ))"
run $U --timer; check "on 1.0.2" 1.0.2-1 "$(installed)"; check "serving 1.0.2" 1.0.2 "$(serving)"

echo "9. …but holds one that migrates"
conf MODE=auto
set_latest 1.0.3; run $U --timer; check "still on 1.0.2" 1.0.2-1 "$(installed)"; check "recorded as held" held "$(result)"

echo "10. …unless APPLY_MIGRATIONS=yes"
conf MODE=auto APPLY_MIGRATIONS=yes
run $U --timer; check "on 1.0.3" 1.0.3-1 "$(installed)"; check "database migrated, data kept" "the operator's data|schema 11|" "$(db)"

echo "11. a release that does not come up: back to the previous one, database restored"
set_latest 1.0.4; run $U --timer
check "exit code 1" 1 "$rc"; check "back on 1.0.3" 1.0.3-1 "$(installed)"; check "serving 1.0.3" 1.0.3 "$(serving)"
check "database as it was before" "the operator's data|schema 11|" "$(db)"; check "recorded" rolled-back "$(result)"

echo "12. the timer does not try that release again"
n="$(copies)"; run $U --timer
check "exit code 0" 0 "$rc"; check "recorded as held" held "$(result)"; check "no new database copy" "$n" "$(copies)"; check "still serving" 1.0.3 "$(serving)"

echo "13. a release that comes up as another version is undone"
conf; set_latest 1.0.5; run $U --apply --allow-migrations
check "exit code 1" 1 "$rc"; check "back on 1.0.3" 1.0.3-1 "$(installed)"; check "database as it was before" "the operator's data|schema 11|" "$(db)"

echo "14. a release that is up but not ready is undone — and what it wrote goes with it"
set_latest 1.0.6; run $U --apply
check "exit code 1" 1 "$rc"; check "back on 1.0.3" 1.0.3-1 "$(installed)"; check "serving 1.0.3" 1.0.3 "$(serving)"
check "database as it was before" "the operator's data|schema 11|" "$(db)"

echo "15. dpkg refusing to install: nothing changed, service running"
set_latest 1.0.7; run $U --apply
check "exit code 1" 1 "$rc"; check "still on 1.0.3" 1.0.3-1 "$(installed)"; check "serving 1.0.3" 1.0.3 "$(serving)"; check "recorded" error "$(result)"

echo "16. the database copy cannot be made: nothing changed, service running"
set_latest 1.0.8; mv /var/backups/ruscker /var/backups/aside; touch /var/backups/ruscker
run $U --apply --allow-migrations
check "exit code 1" 1 "$rc"; check "still on 1.0.3" 1.0.3-1 "$(installed)"; check "serving 1.0.3" 1.0.3 "$(serving)"; check "recorded" error "$(result)"
rm /var/backups/ruscker; mv /var/backups/aside /var/backups/ruscker

echo "17. a catalog in a shared database (address in the environment file): a migrating release is refused"
echo "EnvironmentFile=-/etc/ruscker/ruscker.env" > $W/unit-extra; echo "RUSCKER_CONFIG_DB_URL=postgres://db/ruscker" > /etc/ruscker/ruscker.env
run $U --apply --allow-migrations
check "exit code 1" 1 "$rc"; check "still on 1.0.3" 1.0.3-1 "$(installed)"; check "recorded" error "$(result)"
rm $W/unit-extra /etc/ruscker/ruscker.env

echo "18. the service down before the update: the timer does not touch it"
conf MODE=auto APPLY_MIGRATIONS=yes; systemctl stop ruscker
run $U --timer; check "exit code 1" 1 "$rc"; check "still on 1.0.3" 1.0.3-1 "$(installed)"; check "recorded" error "$(result)"
systemctl start ruscker; sleep 1

echo "19. PIN holds the server at a version"
conf PIN=1.0.3
run $U --apply --allow-migrations; check "still on 1.0.3" 1.0.3-1 "$(installed)"; check "recorded" up-to-date "$(result)"

echo "20. a package that does not match its published sha256 is refused"
conf; set_latest 1.0.9
echo tampered >> "$REL/download/v1.0.9/ruscker_1.0.9-1_$ARCH.deb"
run $U --apply --allow-migrations
check "exit code 1" 1 "$rc"; check "still on 1.0.3" 1.0.3-1 "$(installed)"; check "still serving" 1.0.3 "$(serving)"; check "recorded" error "$(result)"

echo "21. the release server unreachable: an error on the record, nothing changed"
conf; sed -i 's|^SOURCE=.*|SOURCE=http://127.0.0.1:9|' $W/update.conf
run $U --check; check "exit code 1" 1 "$rc"; check "still on 1.0.3" 1.0.3-1 "$(installed)"; check "recorded" error "$(result)"

echo "22. settings that cannot be read are an error on the record"
printf 'MODE=auto\nWINDOW=01 - 05\n' > $W/update.conf; run $U --timer; check "exit code 1, recorded" "1 error" "$rc $(result)"
conf MODE=sometimes; run $U --timer; check "bad MODE: exit code 1, recorded" "1 error" "$rc $(result)"

echo "23. going backwards is refused"
conf; run $U --apply --version 1.0.1; check "exit code 1" 1 "$rc"; check "still on 1.0.3" 1.0.3-1 "$(installed)"

echo "24. and after all that, a good release installs"
set_latest 1.0.8; run $U --apply --allow-migrations
check "exit code 0" 0 "$rc"; check "on 1.0.8" 1.0.8-1 "$(installed)"; check "serving 1.0.8" 1.0.8 "$(serving)"
check "database migrated, data kept" "the operator's data|schema 12|" "$(db)"
check "the operator's configuration kept" "edited by the operator" "$(cat /etc/ruscker/ruscker.yml)"
check "at most five database copies" yes "$([ "$(copies)" -le 5 ] && echo yes || echo "no: $(copies)")"
check "no leftover self-copies in /tmp" 0 "$(ls /tmp/ruscker-update.* 2>/dev/null | wc -l | tr -d ' ')"

echo; echo "$pass passed, $fail failed"
[ "$fail" = 0 ]
