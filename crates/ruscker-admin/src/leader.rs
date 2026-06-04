//! Scaler leader election (Phase 7d).
//!
//! ## Why this exists
//!
//! The auto-scaler spawns and reaps replicas to track demand. With one
//! Ruscker process that's fine, but in active-active HA every instance
//! runs its own scaler against the *same* Docker backend(s) — so they'd
//! all independently spawn-to-min and fight over the replica count,
//! thrashing containers.
//!
//! The fix is single-writer: exactly one instance — the **leader** —
//! runs the scaler loop; the others serve traffic and track sessions
//! normally but skip the scaling work. The proxy, session tracking and
//! manual dashboard actions (stop / restart a replica) are **not**
//! gated — only the automatic scaler is.
//!
//! ## How leadership is decided
//!
//! Via a Postgres **session-level advisory lock** on the shared
//! database. `pg_try_advisory_lock(key)` succeeds for whichever
//! instance grabs it first and stays held for the life of that
//! connection; the others get `false`. If the leader crashes, its
//! connection drops, Postgres releases the lock, and the next instance
//! acquires it on its following check — automatic failover, no
//! heartbeat table to reap.
//!
//! Failover isn't instantaneous: a leader whose connection goes
//! half-open (Postgres released the lock, a standby acquired it, but the
//! old leader's TCP hasn't reset yet) keeps believing it leads until its
//! next liveness check fails. That check is bounded by
//! [`LEADER_OP_TIMEOUT`], so the window in which two instances could
//! both run a tick is at most one timeout — and a scaler tick is
//! idempotent (it reconciles to the same target), so a transient overlap
//! only risks a brief double-spawn-to-min, never divergence.
//!
//! Single-node (SQLite, or no config DB) installs use [`AlwaysLeader`]:
//! there's only one process, so it's always the leader and the scaler
//! runs unconditionally — zero added dependencies.
//!
//! **Operational requirement:** because the lock is session-scoped, the
//! Postgres URL (`--config-db-url` / `--session-store-url`) must reach the
//! server **directly** or through a pooler in *session* mode only. A
//! transaction-pooling proxy (PgBouncer transaction mode) reassigns the
//! lock-holding backend connection between statements, which can break the
//! single-leader guarantee. See the HA section of the deploy guide.

use std::time::Duration;
use sqlx::postgres::PgConnection;
use sqlx::Connection;
use tokio::sync::Mutex;
use tokio::time::timeout;
use tracing::warn;

/// Wall-clock cap for any single leader-election round-trip (connect /
/// ping / lock query). It must stay well under the scaler tick interval
/// so a degraded-but-not-dead Postgres (e.g. a half-open TCP that hasn't
/// reset yet) can't freeze a tick — and a standby's takeover — by
/// blocking under the state mutex (#159 C1). A held leader whose
/// liveness check exceeds this relinquishes, which also bounds any
/// split-brain to a single tick (#159 C2).
const LEADER_OP_TIMEOUT: Duration = Duration::from_secs(2);

/// Decides whether *this* instance may run the auto-scaler. Consulted
/// once per scaler tick; see the module docs.
#[async_trait::async_trait]
pub trait LeaderElector: Send + Sync {
    /// `true` if this instance currently holds leadership and should run
    /// the scaler this tick. Implementations must be cheap (one round-
    /// trip at most) and never panic — a transient failure returns
    /// `false` so a non-leader never accidentally scales.
    async fn is_leader(&self) -> bool;
}

/// The single-node default: this process is always the leader. No I/O,
/// no dependency — the scaler runs every tick.
#[derive(Debug, Default)]
pub struct AlwaysLeader;

#[async_trait::async_trait]
impl LeaderElector for AlwaysLeader {
    async fn is_leader(&self) -> bool {
        true
    }
}

/// The advisory-lock key for the scaler leader. A fixed application-
/// chosen `bigint`; unrelated subsystems must pick different keys if
/// they ever add their own advisory locks. (ASCII `"RUSCKR\x01"`.)
const SCALER_LOCK_KEY: i64 = 0x0052_5553_434b_5201;

struct LockState {
    /// Dedicated connection that holds the advisory lock. `None` until
    /// the first attempt or after the connection is found dead.
    conn: Option<PgConnection>,
    /// Whether we currently hold the lock on `conn`.
    holds: bool,
}

/// Postgres advisory-lock leader elector for HA. Holds its own
/// dedicated connection (separate from the shared pool) so the lock's
/// lifetime is exactly this elector's — dropping it, or the process
/// dying, releases leadership.
pub struct PgLeaderLock {
    url: String,
    state: Mutex<LockState>,
}

impl PgLeaderLock {
    /// Build an elector that competes for leadership on the Postgres at
    /// `url`. The connection is established lazily on the first
    /// [`is_leader`](LeaderElector::is_leader) call.
    pub fn new(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            state: Mutex::new(LockState {
                conn: None,
                holds: false,
            }),
        }
    }
}

#[async_trait::async_trait]
impl LeaderElector for PgLeaderLock {
    async fn is_leader(&self) -> bool {
        let mut st = self.state.lock().await;

        // Reconnect if we have no live connection. A fresh connection
        // never holds the lock yet. Bounded so a stuck connect can't
        // freeze the tick (#159 C1).
        if st.conn.is_none() {
            match timeout(LEADER_OP_TIMEOUT, PgConnection::connect(&self.url)).await {
                Ok(Ok(c)) => {
                    st.conn = Some(c);
                    st.holds = false;
                }
                Ok(Err(e)) => {
                    warn!(error = %e, "leader: connect to Postgres failed; not leading this tick");
                    return false;
                }
                Err(_) => {
                    warn!("leader: connect to Postgres timed out; not leading this tick");
                    return false;
                }
            }
        }
        if st.holds {
            // Already leader — re-validate by a *bounded* liveness check.
            // The advisory lock is session-scoped and we never unlock it,
            // so an alive session still holds it; a severed or half-open
            // connection is caught within `LEADER_OP_TIMEOUT` and we
            // relinquish, so any split-brain lasts at most one tick
            // (#159 C2). We deliberately do NOT re-run
            // `pg_try_advisory_lock` here: it *stacks* (each acquire
            // needs a matching unlock), so re-acquiring every tick would
            // leak lock depth.
            let alive = {
                let conn = st.conn.as_mut().expect("connection present");
                matches!(timeout(LEADER_OP_TIMEOUT, conn.ping()).await, Ok(Ok(())))
            };
            if alive {
                return true;
            }
            warn!("leader: lock connection lost or timed out; relinquishing leadership");
            st.conn = None;
            st.holds = false;
            return false;
        }

        // Not yet leader — try to acquire (bounded). `pg_try_advisory_lock`
        // is non-blocking: `true` ⇒ we got it, `false` ⇒ another instance
        // holds it.
        let result = {
            let conn = st.conn.as_mut().expect("connection present");
            timeout(
                LEADER_OP_TIMEOUT,
                sqlx::query_scalar::<_, bool>("SELECT pg_try_advisory_lock($1)")
                    .bind(SCALER_LOCK_KEY)
                    .fetch_one(&mut *conn),
            )
            .await
        };
        match result {
            Ok(Ok(got)) => {
                st.holds = got;
                got
            }
            Ok(Err(e)) => {
                warn!(error = %e, "leader: advisory-lock query failed; not leading this tick");
                st.conn = None;
                false
            }
            Err(_) => {
                warn!("leader: advisory-lock query timed out; not leading this tick");
                st.conn = None;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn always_leader_is_always_true() {
        assert!(AlwaysLeader.is_leader().await);
        // Through the trait object, as the scaler uses it.
        let e: std::sync::Arc<dyn LeaderElector> = std::sync::Arc::new(AlwaysLeader);
        assert!(e.is_leader().await);
    }

    // Two electors contend for the same lock against a real Postgres:
    // the first wins, the second waits, and dropping the first hands
    // leadership over (failover). Gated on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn pg_lock_elects_one_leader_and_fails_over() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");

        let a = PgLeaderLock::new(&url);
        let b = PgLeaderLock::new(&url);

        assert!(a.is_leader().await, "first contender becomes leader");
        assert!(!b.is_leader().await, "second contender is not leader");
        // Leadership is sticky for the holder.
        assert!(a.is_leader().await);
        assert!(!b.is_leader().await);

        // Leader goes away → its connection closes → lock releases →
        // the other contender takes over on a subsequent check.
        drop(a);
        let mut won = false;
        for _ in 0..50 {
            if b.is_leader().await {
                won = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(won, "surviving contender takes over leadership");
    }
}
