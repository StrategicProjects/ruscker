# Active-active HA harness

A runnable two-instance Ruscker cluster: two app instances behind an
nginx load balancer, sharing one Postgres for the admin catalog **and**
the session store. It exercises the Phase 7 HA pieces end-to-end —
shared catalog, cross-instance sticky sessions, and scaler leader
election with failover.

```sh
docker compose -f examples/ha/docker-compose.yml up --build
```

`--build` compiles Ruscker from the repo (the published image predates
these flags; once a release ships with Phase 7, swap `build:` for
`image:`).

## What to check

**Both instances are live and share the database.** The LB is on
`localhost:8080`:

```sh
curl -s localhost:8080/readyz        # {"checks":{"db":"ok"},"status":"ready"}
curl -s localhost:8080/              # the landing page (read from shared Postgres)
```

**Exactly one instance scales.** Leader election logs on startup:

```sh
docker compose -f examples/ha/docker-compose.yml logs ruscker-1 ruscker-2 \
  | grep -i leadership
#  ruscker-1 … "auto-scaler: acquired leadership; scaling active"
#  ruscker-2 … "auto-scaler: standing by (another instance leads)"
```

**Failover.** Kill the leader; the survivor takes over within one
scaler tick (~10 s):

```sh
docker compose -f examples/ha/docker-compose.yml stop ruscker-1
docker compose -f examples/ha/docker-compose.yml logs ruscker-2 | grep -i leadership
#  ruscker-2 … "auto-scaler: acquired leadership; scaling active"
```

**Session continuity.** Open an app through the LB, then keep using it
as the LB round-robins between instances — the sticky cookie (minted
with a shared `RUSCKER_COOKIE_KEY`) validates on either instance, and
the session row lives in shared Postgres, so the binding survives the
hop.

## How it fits together

- **`--config-db-url`** — both instances read/write one Postgres admin
  catalog (specs, landing, users, audit). Edit on either; both see it.
- **`--session-store-url`** — one shared `proxy_sessions` table; each
  instance reconciles the cluster-wide per-replica session counts, so
  routing and scaling agree.
- **shared `RUSCKER_COOKIE_KEY`** — the sticky-session cookie is an
  HMAC; a common key lets either instance validate the other's cookies.
- **leader election** — a Postgres advisory lock picks one scaler
  leader; the rest serve traffic but don't spawn/reap. The leader's
  death releases the lock for automatic failover.

The keys in the compose file are throwaway dev values — generate real
ones for production and keep them out of YAML (see
`book/src/deploying.md`).
