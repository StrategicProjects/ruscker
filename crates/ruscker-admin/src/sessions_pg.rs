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
                 last_seen   TIMESTAMPTZ NOT NULL DEFAULT now()
             )",
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

    /// Inner `touch_or_register` returning the DB error so the trait
    /// method can decide how to degrade.
    async fn try_touch(
        &self,
        session_id: Uuid,
        spec_id: &str,
        replica_id: &ReplicaId,
    ) -> sqlx::Result<bool> {
        // One UPSERT. `xmax = 0` is Postgres' idiom for "this row was
        // freshly inserted" (vs. updated) — true on insert, false on
        // the ON CONFLICT update path. Lets us tell a brand-new
        // session (mint the sticky cookie) from a returning one (just
        // refresh) in a single round-trip on the hot path.
        let row = sqlx::query(
            "INSERT INTO proxy_sessions
                 (session_id, spec_id, replica_id, instance_id, last_seen)
             VALUES ($1, $2, $3, $4, now())
             ON CONFLICT (session_id) DO UPDATE
                 SET last_seen = now(), instance_id = $4
             RETURNING (xmax = 0) AS inserted",
        )
        .bind(session_id)
        .bind(spec_id)
        .bind(replica_id.0)
        .bind(self.instance_id)
        .fetch_one(&self.pool)
        .await?;
        row.try_get::<bool, _>("inserted")
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
            Ok(true) => {
                // Fresh session: bump the local registry counter for
                // immediacy (the next sweep reconciles it anyway).
                registry.write().await.inc_sessions(replica_id);
                self.local_len.fetch_add(1, Ordering::Relaxed);
                TouchOutcome::Registered
            }
            Ok(false) => TouchOutcome::Touched,
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
}
