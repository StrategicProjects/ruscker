# ADR 0003 — Sticky session affinity is mandatory for interactive apps

Status: accepted (amended 2026-06, v0.2.5 — see Amendment below)

## Context

Shiny apps maintain state on the server inside a WebSocket-backed
reactive context. That state lives in a specific R process inside a
specific container. If the proxy load-balances individual HTTP
requests across multiple containers running the same app, the user's
session breaks the moment a request lands on the wrong container.

This is true for any framework that holds session state server-side:
Shiny, Streamlit, Dash, Voilà.

## Decision

For specs with kind `Shiny`, `InteractiveApp`, every session is
permanently bound to one replica at session creation. Subsequent
requests follow the binding regardless of load.

For specs with kind `Api`, no binding — every request can go to any
replica (round-robin by default).

## How affinity is encoded

A signed cookie `__ruscker_session` carries:

```rust
struct StickySession {
    session_id: SessionId,
    spec_id: String,
    replica_id: ReplicaId,
}
```

Signed with HMAC-SHA256 using a key from `RUSCKER_COOKIE_KEY` env var
(auto-generated and persisted on first run if missing).

On every incoming request:

1. Read and verify the cookie.
2. If valid and `replica_id` still alive: forward there.
3. If invalid or replica gone: create a new session, pick a new
   replica via the routing algorithm, set new cookie.

## Consequences

### Load distribution

For Shiny, load distribution happens **only at session start**, not
per request. Once a user is on replica R2, all their traffic goes to
R2 until the session ends. This is fine because:

- New sessions arrive often enough (many users per minute) for load
  to distribute over time.
- The `least-connections` strategy at session-start time naturally
  fills empty replicas first.

For APIs, true per-request distribution works.

### What happens when a replica dies mid-session

The user loses state. Their cookie still references R2 (now gone).
On their next request, the proxy detects R2 is missing and creates a
new session on a fresh replica. The user sees the Shiny app start
fresh — same as if they'd reopened the browser tab.

We accept this. The alternative (replicating session state across
replicas) is huge engineering complexity for a rare event.

### Scale-down requires drain

We cannot kill a replica mid-session — that's the same "loses state"
event but caused by us instead of by a crash. The auto-scaler's
retirement flow is:

1. Mark replica as `Draining` (no new sessions)
2. Wait for `sessions_active` to reach zero, OR `drain_timeout`
   elapses (default 60s)
3. SIGTERM
4. SIGKILL after another 10s

This means scale-down is slow. Acceptable.

### Cookie size and security

The cookie carries an HMAC signature + serialized session state. We
keep it under 200 bytes by using compact UUIDs and short field names.
Signed but **not** encrypted — the contents (session ID, replica ID)
are not sensitive (they're not credentials), and avoiding encryption
keeps the cookie cheap to verify on every request.

## Alternatives considered

### Server-side session table only, no cookie

Use a session ID in a path component instead of cookie:
`/app/auroraprime/sess-{uuid}/...`. Rejected because:

- Shiny clients can't easily generate this; you'd need a redirect
  on entry.
- Hard to integrate with Shiny's own client-side URL handling.
- Bookmarks would be bound to specific sessions, which is wrong.

### Sticky load balancer outside Ruscker (HAProxy etc.)

A real LB can do source-IP affinity. Rejected because:

- Source IP isn't reliable behind corporate NAT.
- Pushes a dependency outside Ruscker that operators don't want.
- We need session-aware decisions (e.g. "this replica is draining,
  send new users elsewhere but keep existing ones"), which is not
  expressible to a generic LB.

### Replicating state across replicas

Rejected as massively out of scope. Frameworks like Shiny aren't
built for it and adding it externally requires deep R-runtime
intrusion.


## Amendment (2026-06, v0.2.5 — #731)

The original design used **one** cookie named `__ruscker_session` with
`Path=/`. In practice that meant the whole browser held a single
sticky session: opening app B overwrote app A's cookie, orphaning A's
seat (its user landed on the "full" splash for a seat they themselves
held) and breaking stickiness for multi-replica apps whenever a second
app's cookie was present.

Since v0.2.5 the cookie is **per spec**:

- **Name:** `__ruscker_session_{spec_id}` — the distinct name (not just
  the path) is what avoids read-ambiguity with a lingering legacy
  `Path=/` cookie, since cookies are keyed by name+domain+path.
- **Path:** `{base}/app/{spec_id}` — the cookie is only ever *sent* to
  its own app, so it cannot collide and never travels cross-app.
- **Legacy cleanup:** a pre-v0.2.5 global cookie is actively expired
  (`Max-Age=0`, `Path=/`) when seen.
- The embedded `spec_id` check from the original design is kept as
  defense-in-depth against a copied or forged cookie.

The payload, signing scheme and replica-gone fallback are unchanged.
