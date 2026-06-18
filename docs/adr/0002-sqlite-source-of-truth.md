# ADR 0002 — SQLite as the source of truth, YAML as I/O

Status: accepted

## Context

ShinyProxy's source of truth is `application.yml` on disk. Operators
edit the file, restart the daemon. This is simple but has problems:

- Concurrent edits can clobber each other (no file locking).
- No history / undo.
- Hot reload is impractical without elaborate file-watcher logic.
- Admin UI editing requires arbitrating between "what's on disk" and
  "what I just changed".
- Bulk operations on 30+ specs are painful (one text file, no
  transactions).

Ruscker introduces an admin UI with all of those needs. We have to
decide: does the admin UI edit the YAML directly, or does it edit
something else?

## Decision

**SQLite is the source of truth at runtime. YAML is a first-class
import/export format.**

- Operators can `ruscker import application.yml --db ruscker.db` to
  bootstrap.
- Operators can `ruscker export --db ruscker.db > application.yml` to
  produce a YAML representation (for git versioning, backup, sharing).
- Optionally, Ruscker could watch a YAML file on disk and offer to apply
  detected changes via the admin (with a diff view). *(Not implemented:
  `import` is a one-shot command and `export` is manual; there is no
  live file watcher today.)*
- The running proxy reads from SQLite. Period.

## Consequences

### What we gain

- **Transactions**: changing 17 specs at once is atomic.
- **History**: `spec_versions` table tracks every change. Easy
  rollback.
- **Audit trail**: `audit_log` table records who did what when.
- **Concurrent edits**: SQLite's locking handles multiple admin
  sessions correctly.
- **Fast queries** for the monitoring dashboard (joins, aggregations
  that would be miserable on parsed YAML).
- **Schema migrations**: sqlx's migration tooling lets us evolve the
  schema cleanly.

### What we lose

- **One more thing to back up**: the DB file. Mitigated by export to
  YAML — operators can `cron` a daily export to a git repo for
  belt-and-suspenders durability.
- **Drift between disk YAML and DB**: if an operator edits YAML
  expecting it to apply, it won't until they re-run `ruscker import`.
  (The YAML watcher + diff UI that would have auto-applied changes was
  never built — see the proposed solution above.)

### What we explicitly don't do

- **No "YAML is source of truth, DB is a cache"** — that creates a
  consistency mess. The arrow goes one way.
- **No "always write back to YAML on any DB change"** — we don't want
  the admin UI silently rewriting operator-curated YAML files. The
  export step is explicit.

## Alternatives considered

### YAML as source of truth, file-watcher syncs DB

Rejected: bidirectional sync is a classic mistake. Inevitable race
conditions when two writers exist. The admin would have to acquire a
file lock for the whole edit duration, which is brittle.

### Postgres instead of SQLite

Rejected for the MVP because SQLite is zero-config and zero-deps.
Single-node deploys (the 99% case) don't need a separate database
server. Phase 7 (HA) introduces Postgres for shared state — at that
point SQLite is still the local cache for each Ruscker instance, with
Postgres as the cluster source of truth.

### Pure in-memory state + periodic YAML snapshot

Rejected because the snapshot interval introduces a window of data
loss on crash. Operators expect "I clicked save, it's persisted."

## Migration path

1. Phase 0 (today): YAML is everything. `ruscker validate` reads it.
2. Phase 2: SQLite arrives. `ruscker import` populates it. From this
   point, the admin reads/writes the DB; running proxy reads the DB.
3. Phase 2 polish: optional YAML watcher in the admin.
4. Phase 7: Postgres replaces SQLite as cluster-shared store for HA
   deployments. Single-node deploys keep using SQLite.
