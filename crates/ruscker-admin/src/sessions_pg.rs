//! Postgres-backed [`SessionStore`] — the HA (active-active) seam.
//!
//! ## Why this exists
//!
//! The single-node [`InMemorySessionStore`](crate::sessions::InMemorySessionStore)
//! keeps every session in a process-local `DashMap` and owns
//! `Replica::sessions_active` directly. That's perfect for one
//! Ruscker process but invisible to a second one: run two instances
//! behind a load balancer and each only sees the sessions it
//! personally routed, so their seat accounting and load balancing
//! drift apart.
//!
//! `PostgresSessionStore` moves the session table into shared
//! Postgres so every instance reads the *same* truth. It implements
//! the exact same [`SessionStore`] trait, so the proxy, sweeper and
//! graceful-drain code don't change — the CLI just injects this store
//! instead of the in-memory one when a session-store URL is set.
//!
//! ## How the two counters stay honest
//!
//! There are two distinct numbers, and they come from different
//! places on purpose:
//!
//! - **`Replica::sessions_active`** (drives routing + the scaler) is
//!   the *cluster-wide* count. Each [`sweep`](SessionStore::sweep)
//!   reconciles it from a `GROUP BY replica_id` over the shared table
//!   via [`ReplicaRegistry::set_session_counts`], so within one sweep
//!   interval every node converges on the same per-replica totals —
//!   including sessions opened by sibling nodes. A fresh registration
//!   also bumps the local counter immediately (`inc_sessions`) so a
//!   burst of concurrent first-requests can't over-pack a seats=1
//!   replica in the window before the next reconcile.
//! - **[`len`](SessionStore::len)** (drives graceful-drain + the
//!   dashboard "tracked sessions" card) is *this node's* count. It's a
//!   cached `AtomicUsize`, refreshed each sweep to `count(*) WHERE
//!   instance_id = me`. Drain wants to wait for the sessions this node
//!   is serving, not the whole cluster's, so it must stay node-local.
//!
//! "Ownership" is last-toucher-wins: every `touch_or_register` stamps
//! the row's `instance_id` with this node. When a node drains it stops
//! touching, so its sessions either get taken over by whichever node
//! the load balancer fails them over to, or expire — either way this
//! node's `len()` drains toward zero.
//!
//! ## Schema management
//!
//! The store creates its one table with `CREATE TABLE IF NOT EXISTS`
//! on connect. A single idempotent table doesn't justify a migration
//! framework yet; Phase 7c (config in Postgres) introduces proper
//! Postgres migrations and this can fold into them.

use crate::sessions::{SessionStore, TouchOutcome};
use ruscker_core::{ReplicaId, ReplicaRegistry};
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::RwLock;
use tracing::{error, warn};
use uuid::Uuid;

/// Postgres implementation of [`SessionStore`]. Cheap to `Arc`-share;
/// `PgPool` is itself an `Arc` internally and multiplexes connections.
#[derive(Debug)]
pub struct PostgresSessionStore {
    pool: PgPool,
    /// Identifies this Ruscker process among the cluster. Stamped on
    /// every row this node touches so `len()` can count node-local
    /// sessions for graceful drain.
    instance_id: Uuid,
    /// Cached node-local session count, refreshed every `sweep`.
    /// Kept as an atomic so the sync `len()` never blocks on a query.
    local_len: AtomicUsize,
}

/// Outcome of one upsert — see [`PostgresSessionStore::try_touch`].
struct TouchRow {
    inserted: bool,
    took_over: bool,
}

impl PostgresSessionStore {
    /// Connect to `url` (a `postgres://…` DSN), create the session
    /// table if missing, and return a ready store.
    pub async fn connect(url: &str) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new().max_connections(5).connect(url).await?;
        Self::from_pool(pool).await
    }

    /// Build from an existing pool (used by tests). Runs the same
    /// idempotent schema bootstrap as [`connect`](Self::connect).
    pub async fn from_pool(pool: PgPool) -> anyhow::Result<Self> {
        let store = Self {
            pool,
            instance_id: Uuid::new_v4(),
            local_len: AtomicUsize::new(0),
        };
        store.ensure_schema().await?;
        Ok(store)
    }

    /// `CREATE TABLE IF NOT EXISTS` for the session table + its
    /// idle-sweep / reconcile indexes. Idempotent.
    async fn ensure_schema(&self) -> sqlx::Result<()> {
        sqlx::query(
            "CREATE TABLE IF NOT EXISTS proxy_sessions (
                 session_id  UUID PRIMARY KEY,
                 spec_id     TEXT NOT NULL,
                 replica_id  UUID NOT NULL,
                 instance_id UUID NOT NULL,
                 created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
                 last_seen   TIMESTAMPTZ NOT NULL DEFAULT now()
             )",
        )
        .execute(&self.pool)
        .await?;
        // `created_at` discriminates insert from update on the upsert
        // (see `try_touch`). Backfill it for any table created before
        // this column existed; `ADD COLUMN IF NOT EXISTS` is idempotent.
        sqlx::query(
            "ALTER TABLE proxy_sessions
                 ADD COLUMN IF NOT EXISTS created_at TIMESTAMPTZ NOT NULL DEFAULT now()",
        )
        .execute(&self.pool)
        .await?;
        // last_seen powers the idle sweep; replica_id powers the
        // per-replica reconcile GROUP BY.
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS proxy_sessions_last_seen_idx
                 ON proxy_sessions (last_seen)",
        )
        .execute(&self.pool)
        .await?;
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS proxy_sessions_replica_idx
                 ON proxy_sessions (replica_id)",
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Result of one [`try_touch`](Self::try_touch) upsert.
    ///
    /// `inserted` ⇒ a brand-new session (mint the sticky cookie + admit
    /// a seat). `took_over` ⇒ an *existing* session whose ownership just
    /// moved to this node (a load-balancer failover) — it was already
    /// counted cluster-wide, but it's newly part of *this* node's
    /// `len()` for graceful drain.
    async fn try_touch(
        &self,
        session_id: Uuid,
        spec_id: &str,
        replica_id: &ReplicaId,
    ) -> sqlx::Result<TouchRow> {
        // One UPSERT, two facts pulled back in the same round-trip:
        //
        //  - `inserted`: `created_at = last_seen`. On a fresh INSERT both
        //    are the statement's `now()`, so they're equal. On the
        //    ON CONFLICT update path `created_at` keeps the row's
        //    original (earlier) insert time while `last_seen` becomes a
        //    strictly-later `now()` — so they differ. This replaces the
        //    officially-unsupported `xmax = 0` trick with plain,
        //    supported SQL (an update's transaction always starts after
        //    the insert committed, so `created_at < last_seen` holds).
        //
        //  - `prev_instance`: the row's owner *before* this touch,
        //    captured by a CTE evaluated against the pre-statement
        //    snapshot. `NULL` on a fresh insert. Lets the caller detect
        //    a takeover (owner changed to me) and adjust node-local
        //    accounting without waiting for the next sweep.
        let row = sqlx::query(
            "WITH prev AS (
                 SELECT instance_id FROM proxy_sessions WHERE session_id = $1
             )
             INSERT INTO proxy_sessions
                 (session_id, spec_id, replica_id, instance_id, created_at, last_seen)
             VALUES ($1, $2, $3, $4, now(), now())
             ON CONFLICT (session_id) DO UPDATE
                 SET last_seen = now(), instance_id = $4
             RETURNING
                 (created_at = last_seen) AS inserted,
                 (SELECT instance_id FROM prev) AS prev_instance",
        )
        .bind(session_id)
        .bind(spec_id)
        .bind(replica_id.0)
        .bind(self.instance_id)
        .fetch_one(&self.pool)
        .await?;
        let inserted: bool = row.try_get("inserted")?;
        let prev_instance: Option<Uuid> = row.try_get("prev_instance")?;
        // Took over iff the row already existed under a *different*
        // owner. (A repeat touch by this same node is neither an insert
        // nor a takeover.)
        let took_over = !inserted && prev_instance != Some(self.instance_id);
        Ok(TouchRow {
            inserted,
            took_over,
        })
    }

    /// `count(*)` of live sessions on one replica, read straight from
    /// the shared table. Used on the register path to set the replica's
    /// count to committed truth (B2) instead of a blind `+1` a
    /// concurrent reconcile could clobber.
    async fn count_for_replica(&self, replica_id: &ReplicaId) -> sqlx::Result<u32> {
        let n: i64 =
            sqlx::query("SELECT count(*)::bigint AS n FROM proxy_sessions WHERE replica_id = $1")
                .bind(replica_id.0)
                .fetch_one(&self.pool)
                .await?
                .try_get("n")?;
        Ok(n.max(0) as u32)
    }

    /// Inner `sweep`. Returns the number of rows evicted, or a DB
    /// error. Deletes idle rows (honouring per-spec / never-expire
    /// timeouts), then reconciles the registry's per-replica counts
    /// and this node's cached `len()` from the surviving rows.
    async fn try_sweep(
        &self,
        registry: &RwLock<ReplicaRegistry>,
        global_ms: i64,
        overrides: &HashMap<String, i64>,
    ) -> sqlx::Result<usize> {
        let mut evicted: u64 = 0;

        // Specs with any override are excluded from the global pass —
        // each is handled (or skipped, if never-expire) below.
        let overridden: Vec<String> = overrides.keys().cloned().collect();

        // Global pass: every non-overridden spec, unless the global
        // timeout itself is the never-expire sentinel (< 0).
        if global_ms >= 0 {
            let res = sqlx::query(
                "DELETE FROM proxy_sessions
                 WHERE last_seen < now() - ($1::float8 * interval '1 millisecond')
                   AND spec_id <> ALL($2)",
            )
            .bind(global_ms as f64)
            .bind(&overridden)
            .execute(&self.pool)
            .await?;
            evicted += res.rows_affected();
        }

        // Per-spec overrides: a non-negative override reaps that
        // spec's idle rows on its own clock; a negative one (-1) means
        // "never expire", so we leave it alone.
        for (spec_id, ms) in overrides {
            if *ms < 0 {
                continue;
            }
            let res = sqlx::query(
                "DELETE FROM proxy_sessions
                 WHERE last_seen < now() - ($1::float8 * interval '1 millisecond')
                   AND spec_id = $2",
            )
            .bind(*ms as f64)
            .bind(spec_id)
            .execute(&self.pool)
            .await?;
            evicted += res.rows_affected();
        }

        // Reconcile: push the cluster-wide per-replica totals into the
        // local registry so routing + scaling see sibling nodes' load.
        let rows = sqlx::query(
            "SELECT replica_id, count(*)::bigint AS n
             FROM proxy_sessions GROUP BY replica_id",
        )
        .fetch_all(&self.pool)
        .await?;
        let mut counts: HashMap<ReplicaId, u32> = HashMap::with_capacity(rows.len());
        for row in rows {
            let rid: Uuid = row.try_get("replica_id")?;
            let n: i64 = row.try_get("n")?;
            counts.insert(ReplicaId(rid), n.max(0) as u32);
        }
        registry.write().await.set_session_counts(&counts);

        // Refresh this node's cached count for len()/drain.
        let mine: i64 =
            sqlx::query("SELECT count(*)::bigint AS n FROM proxy_sessions WHERE instance_id = $1")
                .bind(self.instance_id)
                .fetch_one(&self.pool)
                .await?
                .try_get("n")?;
        self.local_len
            .store(mine.max(0) as usize, Ordering::Relaxed);

        Ok(evicted as usize)
    }
}

#[async_trait::async_trait]
impl SessionStore for PostgresSessionStore {
    async fn touch_or_register(
        &self,
        registry: &RwLock<ReplicaRegistry>,
        session_id: Uuid,
        spec_id: &str,
        replica_id: &ReplicaId,
    ) -> TouchOutcome {
        match self.try_touch(session_id, spec_id, replica_id).await {
            Ok(TouchRow { inserted: true, .. }) => {
                // Fresh session. For immediacy (before the next sweep
                // reconciles) set this replica's count to committed
                // truth read back from the shared table — an absolute
                // value, so a concurrent reconcile can't lose a blind
                // `+1` (B2). On a count read error, fall back to the
                // old blind bump rather than skip the admission.
                match self.count_for_replica(replica_id).await {
                    Ok(n) => registry.write().await.set_session_count(replica_id, n),
                    Err(e) => {
                        warn!(error = %e, "replica count read failed; using blind bump");
                        registry.write().await.inc_sessions(replica_id);
                    }
                }
                self.local_len.fetch_add(1, Ordering::Relaxed);
                TouchOutcome::Registered
            }
            Ok(TouchRow { took_over: true, .. }) => {
                // Existing session failed over to this node. It's
                // already counted cluster-wide, so don't touch the
                // registry — but it now counts toward *this* node's
                // `len()` for drain, which otherwise wouldn't see it
                // until the next sweep (B1).
                self.local_len.fetch_add(1, Ordering::Relaxed);
                TouchOutcome::Touched
            }
            Ok(_) => TouchOutcome::Touched,
            Err(e) => {
                // Degrade gracefully: the request still proxies. We
                // can't tell new from returning, so report Touched —
                // worst case the sticky cookie isn't minted this once
                // and the next request retries.
                warn!(error = %e, "postgres session touch failed; treating as touch");
                TouchOutcome::Touched
            }
        }
    }

    async fn sweep(
        &self,
        registry: &RwLock<ReplicaRegistry>,
        global_ms: i64,
        overrides: &HashMap<String, i64>,
    ) -> usize {
        match self.try_sweep(registry, global_ms, overrides).await {
            Ok(n) => n,
            Err(e) => {
                // Leave the registry counts as last reconciled rather
                // than zeroing them on a transient DB blip.
                error!(error = %e, "postgres session sweep failed");
                0
            }
        }
    }

    fn len(&self) -> usize {
        self.local_len.load(Ordering::Relaxed)
    }
}

// Only the gated Postgres test lives here; the pure logic is covered
// by the in-memory store's tests. Gating the whole module on the
// feature keeps `use super::*` from being unused in the default build.
#[cfg(all(test, feature = "postgres-it"))]
mod tests {
    use super::*;

    // The pure timeout/never-expire logic is shared with the
    // in-memory store and covered there. The behaviour unique to this
    // store — UPSERT insert-vs-update detection, the reconcile
    // GROUP BY, and node-local `len()` — only has meaning against a
    // real Postgres, so it lives in the `postgres-it` gated test
    // below.
    //
    // Run it with a throwaway daemon:
    //   docker run --rm -e POSTGRES_PASSWORD=pg -p 5433:5432 postgres:16-alpine
    //   RUSCKER_TEST_PG_URL=postgres://postgres:pg@127.0.0.1:5433/postgres \
    //     cargo test -p ruscker-admin --features postgres-it -- --nocapture
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn end_to_end_against_real_postgres() {
        use ruscker_core::{Replica, ReplicaState};
        use std::net::SocketAddr;
        use std::sync::Arc;

        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let store = PostgresSessionStore::connect(&url).await.unwrap();
        // Isolate from any prior run.
        sqlx::query("DELETE FROM proxy_sessions")
            .execute(&store.pool)
            .await
            .unwrap();

        fn replica(spec: &str) -> Replica {
            Replica {
                id: ReplicaId(Uuid::new_v4()),
                spec_id: spec.to_string(),
                container_id: "c".into(),
                upstream: "127.0.0.1:1".parse::<SocketAddr>().unwrap(),
                state: ReplicaState::Ready,
                started_at: chrono::Utc::now(),
                sessions_active: 0,
                sessions_max: 5,
                host: None,
            }
        }

        let reg = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let r = replica("alpha");
        let rid = r.id.clone();
        reg.write().await.add(r);

        // First touch registers + increments immediately.
        let sid = Uuid::new_v4();
        assert_eq!(
            store.touch_or_register(&reg, sid, "alpha", &rid).await,
            TouchOutcome::Registered
        );
        assert_eq!(store.len(), 1);
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 1);

        // Second touch only refreshes.
        assert_eq!(
            store.touch_or_register(&reg, sid, "alpha", &rid).await,
            TouchOutcome::Touched
        );

        // A no-op sweep keeps the fresh session and reconciles the
        // count to exactly 1 from the shared table.
        let no_overrides = HashMap::new();
        let evicted = store.sweep(&reg, 3_600_000, &no_overrides).await;
        assert_eq!(evicted, 0);
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 1);

        // Age the row past a tiny timeout and sweep again: evicted,
        // count reconciled back to 0, len() drained.
        sqlx::query("UPDATE proxy_sessions SET last_seen = now() - interval '1 hour'")
            .execute(&store.pool)
            .await
            .unwrap();
        let evicted = store.sweep(&reg, 1_000, &no_overrides).await;
        assert_eq!(evicted, 1);
        assert_eq!(reg.read().await.replicas_of("alpha")[0].sessions_active, 0);
        assert_eq!(store.len(), 0);
    }

    // Session continuity across instances (the HA core, Phase 7e): a
    // session registered by instance A is visible to instance B once B
    // reconciles from the shared table — so B's router/scaler see A's
    // load. Two stores (distinct instance_ids) over one Postgres, with
    // separate registries (as each node reconciles the same replica
    // from the shared Docker backend). Gated on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn session_registered_on_a_is_seen_by_b() {
        use ruscker_core::{Replica, ReplicaState};
        use std::net::SocketAddr;
        use std::sync::Arc;

        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let a = PostgresSessionStore::connect(&url).await.unwrap();
        let b = PostgresSessionStore::connect(&url).await.unwrap();
        sqlx::query("DELETE FROM proxy_sessions")
            .execute(&a.pool)
            .await
            .unwrap();

        // Same replica id on both nodes (each reconciles it from the
        // shared backend) — but separate registries.
        let rid = ReplicaId(Uuid::new_v4());
        let mk = || Replica {
            id: rid.clone(),
            spec_id: "alpha".into(),
            container_id: "c".into(),
            upstream: "127.0.0.1:1".parse::<SocketAddr>().unwrap(),
            state: ReplicaState::Ready,
            started_at: chrono::Utc::now(),
            sessions_active: 0,
            sessions_max: 5,
            host: None,
        };
        let reg_a = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let reg_b = Arc::new(RwLock::new(ReplicaRegistry::new()));
        reg_a.write().await.add(mk());
        reg_b.write().await.add(mk());

        // A registers a session.
        let sid = Uuid::new_v4();
        assert_eq!(
            a.touch_or_register(&reg_a, sid, "alpha", &rid).await,
            TouchOutcome::Registered
        );
        // B hasn't reconciled yet — its registry shows nothing.
        assert_eq!(reg_b.read().await.replicas_of("alpha")[0].sessions_active, 0);

        // B's sweep reconciles the cluster-wide count from the shared
        // table and now sees A's session.
        let no_overrides = HashMap::new();
        b.sweep(&reg_b, 3_600_000, &no_overrides).await;
        assert_eq!(reg_b.read().await.replicas_of("alpha")[0].sessions_active, 1);
        // …and the session is node-local to A, not B.
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 0);
    }

    // B1: a load-balancer failover hands A's live session to B. B's
    // touch is an UPDATE (the row exists) — so `Touched`, not
    // `Registered` — but ownership moves to B, and B's node-local
    // `len()` must reflect it *immediately* (not only after the next
    // sweep), or a graceful drain on B could finish while it's still
    // serving the failed-over session. Gated on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn takeover_bumps_node_local_len_immediately() {
        use ruscker_core::{Replica, ReplicaState};
        use std::net::SocketAddr;
        use std::sync::Arc;

        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let a = PostgresSessionStore::connect(&url).await.unwrap();
        let b = PostgresSessionStore::connect(&url).await.unwrap();
        sqlx::query("DELETE FROM proxy_sessions")
            .execute(&a.pool)
            .await
            .unwrap();

        let rid = ReplicaId(Uuid::new_v4());
        let mk = || Replica {
            id: rid.clone(),
            spec_id: "alpha".into(),
            container_id: "c".into(),
            upstream: "127.0.0.1:1".parse::<SocketAddr>().unwrap(),
            state: ReplicaState::Ready,
            started_at: chrono::Utc::now(),
            sessions_active: 0,
            sessions_max: 5,
            host: None,
        };
        let reg_a = Arc::new(RwLock::new(ReplicaRegistry::new()));
        let reg_b = Arc::new(RwLock::new(ReplicaRegistry::new()));
        reg_a.write().await.add(mk());
        reg_b.write().await.add(mk());

        // A registers; A owns it, A.len() == 1, B.len() == 0.
        let sid = Uuid::new_v4();
        assert_eq!(
            a.touch_or_register(&reg_a, sid, "alpha", &rid).await,
            TouchOutcome::Registered
        );
        assert_eq!(a.len(), 1);
        assert_eq!(b.len(), 0);

        // Failover: the LB routes the *same* session to B. The row
        // exists, so this is a Touched (not Registered) — but B now
        // owns it and must count it in len() right away (B1 fix).
        assert_eq!(
            b.touch_or_register(&reg_b, sid, "alpha", &rid).await,
            TouchOutcome::Touched
        );
        assert_eq!(b.len(), 1, "B counts the failed-over session immediately");

        // The cluster-wide count is still exactly 1 (one session, now on
        // B) — the takeover must not have double-counted the seat.
        b.sweep(&reg_b, 3_600_000, &HashMap::new()).await;
        assert_eq!(reg_b.read().await.replicas_of("alpha")[0].sessions_active, 1);

        // After A sweeps, the row's instance_id is B's, so A's len()
        // drains to 0 (A is no longer serving it).
        a.sweep(&reg_a, 3_600_000, &HashMap::new()).await;
        assert_eq!(a.len(), 0, "A no longer owns the session");

        // A repeat touch by the new owner is neither insert nor takeover
        // — len() stays 1, no double count.
        assert_eq!(
            b.touch_or_register(&reg_b, sid, "alpha", &rid).await,
            TouchOutcome::Touched
        );
        assert_eq!(b.len(), 1);
    }
}
