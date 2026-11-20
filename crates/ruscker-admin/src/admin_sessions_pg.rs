//! Postgres-backed [`crate::auth::AdminSessionStore`] — the HA seam
//! for sign-in sessions (#185).
//!
//! ## Why this exists
//!
//! The default [`crate::auth::InMemoryAdminSessionStore`] is
//! process-local: a load-balancer hop to another Ruscker instance
//! sees the cookie as unknown and bounces the user to login. The
//! deploy guide's "sticky upstream for the sign-in session"
//! workaround (#161) papers over it; this store removes the need by
//! moving the session table into shared Postgres.
//!
//! ## Hot path
//!
//! `validate(sid)` is called on **every** admin route AND every
//! `MaybeSession` resolution — the landing and the `/app` proxy
//! guard from #155. A naive PG-per-request adds a DB round-trip to
//! every authenticated request, which is unacceptable for a proxy
//! built for sub-millisecond hops.
//!
//! We keep a small node-local cache: a [`DashMap`] keyed by session
//! id holding the resolved [`SessionInfo`] plus a `valid_until`
//! [`Instant`]. Cache hit (fresh) returns immediately; cache miss
//! (or stale) does one `SELECT`, repopulates, and returns. `create`
//! and `remove` write through to PG and update the cache, so the
//! local node observes its own writes instantly.
//!
//! The cache TTL bounds cross-instance staleness: a logout on
//! instance A propagates to instance B within at most one TTL
//! window. Default is **5 seconds**, picked to keep the hot path
//! sub-ms while keeping the "logged out everywhere" UX snappy.
//!
//! ## Schema management
//!
//! Idempotent `CREATE TABLE IF NOT EXISTS` on connect — same pattern
//! as [`crate::sessions_pg::PostgresSessionStore`].

use crate::auth::{mint_session_id, AdminSessionStore, Role, SessionInfo};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::Row;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tracing::warn;

/// How many `validate` calls between cache sweeps (#738). The cache
/// writes an entry for EVERY sid looked up — including a negative one
/// for an attacker-chosen cookie value that was never in the DB — and
/// nothing else ever evicted stale entries, so an unauthenticated
/// client spraying random `ruscker_admin_session` values grew the map
/// one never-freed entry per request. The amortized sweep drops every
/// entry past its `valid_until`, bounding the map to the sids seen
/// within roughly one cache-TTL window (+ at most this many extras
/// between sweeps). Same shape as the ApiRateLimiter sweep (#737).
const SWEEP_EVERY: u32 = 512;

/// Default lifetime of a sign-in session in Postgres. Matches the
/// in-memory default (`24h`) and the cookie's `Max-Age`.
const DEFAULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Default node-local cache TTL. Picked small enough that a logout
/// propagates across instances within ~one window, large enough that
/// the hot path stays effectively DB-free for a long-running session.
const DEFAULT_CACHE_TTL: Duration = Duration::from_secs(5);

/// Postgres implementation of [`AdminSessionStore`]. Cheap to
/// `Arc`-share; `PgPool` is itself an `Arc` internally and the cache
/// uses `DashMap` shards.
#[derive(Debug)]
pub struct PostgresAdminSessionStore {
    pool: PgPool,
    /// How long a session stays valid after `create`.
    ttl: Duration,
    /// How long a cached `validate` result is considered fresh.
    cache_ttl: Duration,
    /// Local cache: sid → (info, valid_until). A `None` info means
    /// "we know the sid is invalid (deleted or expired)", which lets
    /// repeated lookups of a stale cookie also skip the DB. Swept of
    /// expired entries every [`SWEEP_EVERY`] validates (#738) so
    /// attacker-chosen sids can't grow it without bound.
    cache: DashMap<String, CachedEntry>,
    /// `validate` calls since the last cache sweep.
    since_sweep: AtomicU32,
    /// Single-session revocations (sign-outs) whose DELETE has not reached
    /// Postgres yet (security
    /// review 2026-11-19: they used to be logged and forgotten, so a
    /// "signed out" session came back once the database recovered).
    /// While here, this node denies them; a background task keeps
    /// retrying the DELETE and clears the entry once it lands.
    pending: Arc<PendingRevocations>,
}

/// See [`PostgresAdminSessionStore::pending`].
#[derive(Debug, Default)]
struct PendingRevocations {
    /// Session ids to deny.
    sids: DashMap<String, ()>,
    /// One background worker drains both lists; set while it runs.
    worker_running: std::sync::atomic::AtomicBool,
}

/// Attempts of a revoking DELETE before it is handed to the background
/// retry, and the pause between background retries.
const REVOKE_ATTEMPTS: u32 = 3;
/// Above this many waiting single-session revocations, only sessions this
/// node KNOWS are real (a positive cache entry) are still queued: a flood
/// of sign-outs with made-up ids during an outage cannot grow the list
/// without bound, and a real revocation is never dropped for it (Codex
/// review and the automated security review of #1123).
const MAX_PENDING_REVOCATIONS: usize = 4096;

/// Whether `id` has the shape [`mint_session_id`] gives every session (64
/// lowercase hex). Anything else cannot be a stored session, so it never
/// reaches Postgres — a cookie carrying, say, a NUL would make the query
/// fail every time (Codex review of #1123).
fn well_formed_sid(id: &str) -> bool {
    id.len() == 64 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}
const REVOKE_RETRY_EVERY: Duration = Duration::from_secs(2);

#[derive(Clone, Debug)]
struct CachedEntry {
    /// `None` ⇒ the sid is known-invalid (revoked or expired). Lets
    /// repeated lookups of a stale cookie also skip the DB.
    info: Option<SessionInfo>,
    /// When this cache entry becomes stale and must be re-resolved.
    valid_until: Instant,
}

impl PostgresAdminSessionStore {
    /// Connect to Postgres, ensure the schema, and return a store
    /// ready to back [`crate::AppState::admin_sessions`]. Uses the
    /// default lifetimes (24h sessions, 5s cache).
    pub async fn connect(url: &str) -> sqlx::Result<Self> {
        Self::connect_with(url, DEFAULT_TTL, DEFAULT_CACHE_TTL).await
    }

    /// Same as [`Self::connect`] but lets the operator pick a custom
    /// session TTL and cache TTL. Mostly used by tests.
    pub async fn connect_with(
        url: &str,
        ttl: Duration,
        cache_ttl: Duration,
    ) -> sqlx::Result<Self> {
        let pool = PgPoolOptions::new()
            .max_connections(5)
            .acquire_timeout(Duration::from_secs(5))
            .connect(url)
            .await?;
        ensure_schema(&pool).await?;
        Ok(Self {
            pool,
            ttl,
            cache_ttl,
            cache: DashMap::new(),
            since_sweep: AtomicU32::new(0),
            pending: Arc::default(),
        })
    }

    /// Returns the current node-local cache size. Used by the
    /// `postgres-it` tests to verify the hot path actually serves
    /// from cache.
    #[doc(hidden)]
    pub fn cache_len(&self) -> usize {
        self.cache.len()
    }

    /// Start the background worker that retries waiting revocations, if
    /// it is not running. One worker at most, however many are waiting.
    fn ensure_worker(&self) {
        use std::sync::atomic::Ordering as O;
        if self.pending.worker_running.swap(true, O::SeqCst) {
            return;
        }
        let (pool, pending) = (self.pool.clone(), self.pending.clone());
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(REVOKE_RETRY_EVERY).await;
                drain_pending(&pool, &pending).await;
                if pending.sids.is_empty() {
                    pending.worker_running.store(false, O::SeqCst);
                    // A revocation queued between the check and the store
                    // would find the flag still set and not start a worker:
                    // look once more, and carry on if so.
                    if (pending.sids.is_empty())
                        || pending.worker_running.swap(true, O::SeqCst)
                    {
                        return;
                    }
                }
            }
        });
    }
}

/// One pass of the worker: delete every waiting session in one statement.
/// Whatever lands is cleared; the rest waits.
async fn drain_pending(pool: &PgPool, pending: &PendingRevocations) {
    let sids: Vec<String> = pending.sids.iter().map(|e| e.key().clone()).collect();
    if !sids.is_empty() {
        match sqlx::query("DELETE FROM admin_sessions WHERE sid = ANY($1)")
            .bind(&sids)
            .execute(pool)
            .await
        {
            Ok(_) => {
                for sid in &sids {
                    pending.sids.remove(sid);
                }
                tracing::info!(count = sids.len(), "waiting admin session revocations reached the database");
            }
            Err(e) => warn!(error = ?e, "waiting admin session revocations still failing"),
        }
    }

}

async fn ensure_schema(pool: &PgPool) -> sqlx::Result<()> {
    // sqlx prepares each call as a single statement, so split the two
    // DDLs across two `execute`s rather than `;`-joining them.
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS admin_sessions (
            sid        TEXT PRIMARY KEY,
            role       TEXT NOT NULL,
            actor      TEXT,
            expires_at TIMESTAMPTZ NOT NULL
        )"#,
    )
    .execute(pool)
    .await?;
    // The authentication generation each session was issued under
    // (security review 2026-11-19); older tables gain it here.
    sqlx::query("ALTER TABLE admin_sessions ADD COLUMN IF NOT EXISTS auth_epoch BIGINT NOT NULL DEFAULT 0")
        .execute(pool)
        .await?;
    sqlx::query("ALTER TABLE admin_sessions ADD COLUMN IF NOT EXISTS account_id TEXT")
        .execute(pool)
        .await?;
    sqlx::query(
        "CREATE INDEX IF NOT EXISTS admin_sessions_expires_at_idx
            ON admin_sessions (expires_at)",
    )
    .execute(pool)
    .await
    .map(|_| ())
}

/// Clip a cache `valid_until` so it never outlives the DB-recorded
/// `expires_at`. Returns the earlier of the two, so a positive
/// cached answer is invalidated as soon as PG would refuse it.
///
/// Converts `expires_at` (calendar time) to a monotonic [`Instant`]
/// by anchoring it against `Utc::now()` and `Instant::now()` — same
/// pair sampled close together so monotonic-vs-wall-clock drift is
/// negligible.
fn clip_to_expiry(cache_until: Instant, expires_at: DateTime<Utc>) -> Instant {
    let wall_now = Utc::now();
    if expires_at <= wall_now {
        // Already expired by wall clock — clamp to "now" so the cache
        // entry is stale immediately on the next lookup.
        return Instant::now();
    }
    // expires_at > wall_now ⇒ the delta is positive.
    let delta = (expires_at - wall_now).to_std().unwrap_or_default();
    let expiry_instant = Instant::now() + delta;
    cache_until.min(expiry_instant)
}

fn role_to_str(role: Role) -> &'static str {
    match role {
        Role::Admin => "admin",
        Role::Editor => "editor",
        Role::Viewer => "viewer",
    }
}

fn role_from_str(s: &str) -> Option<Role> {
    match s {
        "admin" => Some(Role::Admin),
        "editor" => Some(Role::Editor),
        "viewer" => Some(Role::Viewer),
        _ => None,
    }
}

#[async_trait::async_trait]
impl AdminSessionStore for PostgresAdminSessionStore {
    async fn create_at_epoch(
        &self,
        role: Role,
        actor: Option<String>,
        epoch: i64,
        account_id: Option<String>,
    ) -> String {
        let id = mint_session_id();
        let expires_at: DateTime<Utc> = Utc::now() + chrono::Duration::from_std(self.ttl).unwrap();
        let result = sqlx::query(
            "INSERT INTO admin_sessions (sid, role, actor, expires_at, auth_epoch, account_id)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&id)
        .bind(role_to_str(role))
        .bind(&actor)
        .bind(expires_at)
        .bind(epoch)
        .bind(&account_id)
        .execute(&self.pool)
        .await;
        if let Err(e) = result {
            // The store-level contract is "best-effort" — if the DB is
            // unreachable we still return an id so the cookie path
            // doesn't 500. The local cache lets the just-signed-in user
            // browse THIS node; a failover lands them at login because
            // sibling instances never see the row. Log loud so ops
            // notices the divergence in their dashboards.
            tracing::error!(
                error = ?e,
                "admin session INSERT failed; cookie works on this node only — sibling instances will treat it as unknown until the DB recovers"
            );
        }
        // Write-through to the cache so this node sees its own writes
        // instantly without re-querying. Clip the cache window to the
        // session's `expires_at` so a cached positive answer never
        // outlives the DB-recorded expiry (review on #196 finding #1).
        // At default TTLs (24h session, 5s cache) the clip is a no-op;
        // it matters at the boundary and in test configurations with
        // short session TTLs.
        let valid_until = clip_to_expiry(Instant::now() + self.cache_ttl, expires_at);
        self.cache.insert(
            id.clone(),
            CachedEntry {
                info: Some(SessionInfo {
                    role,
                    actor,
                    epoch,
                    issued_at: expires_at - ttl_as_chrono(self.ttl),
                    account_id,
                }),
                valid_until,
            },
        );
        id
    }

    async fn validate(&self, id: &str) -> Option<SessionInfo> {
        let now = Instant::now();

        // Amortized sweep (#738): every SWEEP_EVERY validates, drop every
        // entry past its `valid_until` — they'd be re-resolved on their
        // next touch anyway, so they only pin memory for sids (often
        // attacker-chosen ones) that may never come back.
        if self.since_sweep.fetch_add(1, Ordering::Relaxed) + 1 >= SWEEP_EVERY {
            self.since_sweep.store(0, Ordering::Relaxed);
            self.cache.retain(|_, e| e.valid_until > now);
        }

        // Not a session id at all: nothing to look up.
        if !well_formed_sid(id) {
            return None;
        }
        // A revocation of this very session that has not reached the
        // database yet: denied, whatever the cache or the row says.
        if self.pending.sids.contains_key(id) {
            return None;
        }

        // Cache hit (fresh) — the hot path.
        if let Some(entry) = self.cache.get(id) {
            if entry.valid_until > now {
                return entry.info.clone();
            }
        }

        // Cache miss / stale — query Postgres.
        let row = sqlx::query(
            "SELECT role, actor, expires_at, auth_epoch, account_id
               FROM admin_sessions
              WHERE sid = $1",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await;

        // (info, expires_at-from-row) — the second slot is only used
        // to clip the cache window on the positive branch.
        let (info, expires_at): (Option<SessionInfo>, Option<DateTime<Utc>>) = match row {
            Ok(Some(r)) => {
                let role_str: String = r.get("role");
                let actor: Option<String> = r.get("actor");
                let expires_at: DateTime<Utc> = r.get("expires_at");
                let epoch: i64 = r.get("auth_epoch");
                let account_id: Option<String> = r.get("account_id");
                let role = match role_from_str(&role_str) {
                    Some(r) => r,
                    None => {
                        warn!(role = %role_str, sid_short = &id[..8], "unknown role in admin_sessions row — treating as invalid");
                        // Cache the negative answer too.
                        self.cache.insert(
                            id.to_string(),
                            CachedEntry {
                                info: None,
                                valid_until: now + self.cache_ttl,
                            },
                        );
                        return None;
                    }
                };
                if expires_at <= Utc::now() {
                    // Expired — lazy GC, then cache the negative answer.
                    let _ = sqlx::query("DELETE FROM admin_sessions WHERE sid = $1")
                        .bind(id)
                        .execute(&self.pool)
                        .await;
                    (None, None)
                } else {
                    (
                        Some(SessionInfo {
                            role,
                            actor,
                            epoch,
                            // Issued = expiry − lifetime (sessions are never extended).
                            issued_at: expires_at - ttl_as_chrono(self.ttl),
                            account_id,
                        }),
                        Some(expires_at),
                    )
                }
            }
            Ok(None) => (None, None),
            Err(e) => {
                // DB hiccup — log and fall back to "unknown". Don't poison
                // the cache, so the next request retries instead of
                // stalling on a stale "invalid" answer.
                warn!(error = ?e, "admin session SELECT failed");
                return None;
            }
        };

        // Cache the result (positive or negative) so a flood of the
        // same sid stays single-DB-hit per TTL window. For positive
        // answers, clip the cache window to the row's `expires_at` so
        // a cached entry never outlives the DB-recorded expiry (review
        // on #196 finding #1).
        let valid_until = match expires_at {
            Some(exp) => clip_to_expiry(now + self.cache_ttl, exp),
            None => now + self.cache_ttl,
        };
        self.cache.insert(
            id.to_string(),
            CachedEntry {
                info: info.clone(),
                valid_until,
            },
        );
        info
    }

    async fn remove(&self, id: &str) -> bool {
        // Delete + cache eviction (write-through). If the DB delete fails
        // the session is denied on this node and queued for the retry
        // worker — and the caller is told (`false`), so the person is not
        // shown a sign-out the database never recorded (Codex review of
        // #1123: a bounded local queue cannot guarantee it on its own).
        if !well_formed_sid(id) {
            // Cannot be a stored session: nothing to delete.
            return true;
        }
        let deleted = delete_with_retries(&self.pool, "DELETE FROM admin_sessions WHERE sid = $1", id).await;
        if !deleted {
            if self.pending.sids.len() < MAX_PENDING_REVOCATIONS
                || self.cache.get(id).is_some_and(|e| e.info.is_some())
            {
                self.pending.sids.insert(id.to_string(), ());
                self.ensure_worker();
            } else {
                tracing::error!(
                    "admin session revocation not queued: {MAX_PENDING_REVOCATIONS} already waiting"
                );
            }
        }
        self.cache.remove(id);
        // Drop a tombstone so a sibling instance with a still-cached
        // positive answer doesn't keep serving the deleted session for
        // up to its own cache_ttl on this node specifically. Cross-node
        // staleness still rides the cache_ttl window.
        self.cache.insert(
            id.to_string(),
            CachedEntry {
                info: None,
                valid_until: Instant::now() + self.cache_ttl,
            },
        );
        deleted
    }

    async fn revoke_by_actor(&self, actor: &str) {
        // Cleanup of the account's rows. What actually ends those sessions
        // is the account's authentication generation, moved forward in the
        // same transaction as the change that called for the revocation
        // (`users.auth_epoch`, checked by `auth::confirm_session`): durable,
        // seen by every node, and unaffected by this DELETE failing. So a
        // failure here is only logged — and, unlike a retry loop, can never
        // delete a session issued after the change (Codex review of #1123).
        if !delete_with_retries(&self.pool, "DELETE FROM admin_sessions WHERE actor = $1", actor).await {
            warn!("admin sessions of an account were not deleted; its generation already ended them");
        }
        // Evict this node's positive cache entries for the actor so it
        // stops serving them immediately; negative tombstones and other
        // users' entries stay. Sibling nodes ride the cache_ttl window,
        // same as logout.
        self.cache
            .retain(|_, e| e.info.as_ref().and_then(|i| i.actor.as_deref()) != Some(actor));
    }

    async fn revoke_before_epoch(&self, actor: &str, epoch: i64) {
        // Same cleanup, sparing sessions already issued under the new
        // generation (see the trait).
        let mut done = false;
        for attempt in 1..=REVOKE_ATTEMPTS {
            match sqlx::query("DELETE FROM admin_sessions WHERE actor = $1 AND auth_epoch < $2")
                .bind(actor)
                .bind(epoch)
                .execute(&self.pool)
                .await
            {
                Ok(_) => {
                    done = true;
                    break;
                }
                Err(e) => {
                    warn!(error = ?e, attempt, "admin session cleanup DELETE failed");
                    if attempt < REVOKE_ATTEMPTS {
                        tokio::time::sleep(Duration::from_millis(100 * u64::from(attempt))).await;
                    }
                }
            }
        }
        if !done {
            warn!("admin sessions of an account were not deleted; its generation already ended them");
        }
        self.cache.retain(|_, e| match e.info.as_ref() {
            Some(i) => i.actor.as_deref() != Some(actor) || i.epoch >= epoch,
            None => true,
        });
    }
}

/// The session lifetime as a chrono duration (for issued = expiry − ttl).
fn ttl_as_chrono(ttl: Duration) -> chrono::Duration {
    chrono::Duration::from_std(ttl).unwrap_or_else(|_| chrono::Duration::zero())
}

/// Run a revoking DELETE (one `$1` bind) up to [`REVOKE_ATTEMPTS`] times.
/// `true` once it succeeded.
async fn delete_with_retries(pool: &PgPool, sql: &'static str, key: &str) -> bool {
    for attempt in 1..=REVOKE_ATTEMPTS {
        match sqlx::query(sql).bind(key).execute(pool).await {
            Ok(_) => return true,
            Err(e) => {
                warn!(error = ?e, attempt, "admin session revocation DELETE failed");
                if attempt < REVOKE_ATTEMPTS {
                    tokio::time::sleep(Duration::from_millis(100 * u64::from(attempt))).await;
                }
            }
        }
    }
    false
}


// ── Tests ─────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "postgres-it"))]
mod postgres_it {
    use super::*;

    /// `RUSCKER_TEST_PG_URL` points at a throwaway Postgres for these
    /// tests. Same convention as the `sessions_pg` integration tests.
    fn pg_url() -> String {
        std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN")
    }

    /// Wipe the table contents so each test starts clean. Uses
    /// `DELETE` not `DROP/CREATE` — concurrent `CREATE TABLE` races
    /// on `pg_type` and `cargo test` runs in parallel by default.
    async fn reset_rows(pool: &PgPool) {
        sqlx::query("DELETE FROM admin_sessions")
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_then_validate_roundtrips() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let store = PostgresAdminSessionStore::connect(&pg_url()).await.unwrap();
        reset_rows(&store.pool).await;
        let sid = store
            .create(Role::Editor, Some("alice".to_string()))
            .await;
        let info = store.validate(&sid).await.expect("freshly created is live");
        assert_eq!(info.role, Role::Editor);
        assert_eq!(info.actor.as_deref(), Some("alice"));
    }

    #[tokio::test]
    async fn validate_hits_cache_after_first_lookup() {
        // After one validate, subsequent validates within the cache
        // window must NOT round-trip to the DB. We can't easily count
        // PG round trips from here; instead we DELETE the row out
        // from under the cache and verify the cached lookup still
        // succeeds.
        let _guard = crate::db::pg_test_lock().lock().await;
        let store = PostgresAdminSessionStore::connect_with(
            &pg_url(),
            Duration::from_secs(60),
            Duration::from_secs(30), // wide cache window
        )
        .await
        .unwrap();
        reset_rows(&store.pool).await;
        let sid = store.create(Role::Admin, None).await;
        // First validate populates the cache from the just-written row.
        assert!(store.validate(&sid).await.is_some());
        // Now make the DB row invisible — if validate still returns
        // Some, it MUST have served from cache.
        sqlx::query("DELETE FROM admin_sessions WHERE sid = $1")
            .bind(&sid)
            .execute(&store.pool)
            .await
            .unwrap();
        assert!(
            store.validate(&sid).await.is_some(),
            "fresh cache entry should skip the DB"
        );
    }

    #[tokio::test]
    async fn cross_instance_visibility_via_shared_table() {
        // Two store handles sharing the same Postgres → emulates two
        // Ruscker instances behind a load balancer. A session created on
        // A must validate on B without re-login.
        let _guard = crate::db::pg_test_lock().lock().await;
        let a = PostgresAdminSessionStore::connect_with(
            &pg_url(),
            Duration::from_secs(60),
            // Tiny cache window on B so we hit the DB; A's cache is
            // irrelevant (it never validates).
            Duration::from_millis(50),
        )
        .await
        .unwrap();
        reset_rows(&a.pool).await;
        let b = PostgresAdminSessionStore::connect_with(
            &pg_url(),
            Duration::from_secs(60),
            Duration::from_millis(50),
        )
        .await
        .unwrap();

        let sid = a.create(Role::Viewer, Some("bob".to_string())).await;
        let info = b.validate(&sid).await.expect("session survives LB hop");
        assert_eq!(info.role, Role::Viewer);
        assert_eq!(info.actor.as_deref(), Some("bob"));
    }

    #[tokio::test]
    async fn cleanup_spares_sessions_of_the_new_generation() {
        // Final security review (2026-11-20), finding 7: in the shared
        // store too, the cleanup after a change to generation 3 keeps a
        // session already issued at 3, also on a node that cached it.
        let _guard = crate::db::pg_test_lock().lock().await;
        let store = PostgresAdminSessionStore::connect_with(
            &pg_url(),
            Duration::from_secs(60),
            Duration::from_secs(30),
        )
        .await
        .unwrap();
        reset_rows(&store.pool).await;
        let old = store.create_at_epoch(Role::Admin, Some("alice".into()), 2, Some("a".into())).await;
        let new = store.create_at_epoch(Role::Admin, Some("alice".into()), 3, Some("a".into())).await;
        let bob = store.create_at_epoch(Role::Editor, Some("bob".into()), 0, Some("b".into())).await;
        assert!(store.validate(&old).await.is_some(), "cached");

        store.revoke_before_epoch("alice", 3).await;

        assert!(store.validate(&old).await.is_none(), "earlier generation removed, cache too");
        assert!(store.validate(&new).await.is_some(), "the new generation stays");
        assert!(store.validate(&bob).await.is_some());
        let rows: Vec<(String,)> = sqlx::query_as("SELECT sid FROM admin_sessions WHERE actor = 'alice'")
            .fetch_all(&store.pool)
            .await
            .unwrap();
        assert_eq!(rows, vec![(new.clone(),)], "only the new row is left");
    }

    #[tokio::test]
    async fn remove_revokes_within_cache_ttl_on_other_node() {
        // Logout propagation: A revokes, B sees revoked within
        // `cache_ttl` (B's cache window bounds the staleness).
        let _guard = crate::db::pg_test_lock().lock().await;
        let cache_ttl = Duration::from_millis(50);
        let a =
            PostgresAdminSessionStore::connect_with(&pg_url(), Duration::from_secs(60), cache_ttl)
                .await
                .unwrap();
        reset_rows(&a.pool).await;
        let b =
            PostgresAdminSessionStore::connect_with(&pg_url(), Duration::from_secs(60), cache_ttl)
                .await
                .unwrap();

        let sid = a.create(Role::Viewer, None).await;
        // Warm B's cache with a live answer.
        assert!(b.validate(&sid).await.is_some());

        // A logs the user out.
        a.remove(&sid).await;

        // Within the TTL, B's cache may still serve "live" — that's
        // acceptable per the design. After the TTL expires, B MUST
        // observe the revocation.
        tokio::time::sleep(cache_ttl + Duration::from_millis(20)).await;
        assert!(
            b.validate(&sid).await.is_none(),
            "logout must propagate within cache_ttl"
        );
    }

    #[tokio::test]
    async fn cache_window_is_clipped_to_session_expiry() {
        // #196 review finding #1: if the session is about to expire
        // *before* the cache window ends, the cached entry must NOT
        // outlive the row's `expires_at`. Set session_ttl smaller than
        // cache_ttl so the clip actually fires.
        let _guard = crate::db::pg_test_lock().lock().await;
        let session_ttl = Duration::from_millis(100);
        let cache_ttl = Duration::from_secs(60); // would otherwise dominate
        let store =
            PostgresAdminSessionStore::connect_with(&pg_url(), session_ttl, cache_ttl)
                .await
                .unwrap();
        reset_rows(&store.pool).await;
        let sid = store.create(Role::Admin, None).await;
        // Sleep past `session_ttl` but well inside `cache_ttl`. With
        // the clip the cache window stopped at `expires_at`, so the
        // next validate must NOT return the (now-expired) session.
        tokio::time::sleep(session_ttl + Duration::from_millis(30)).await;
        assert!(
            store.validate(&sid).await.is_none(),
            "cache window must be clipped to expires_at"
        );
    }

    #[tokio::test]
    async fn cache_sweep_bounds_a_flood_of_unknown_sids() {
        // #738: an unauthenticated client spraying random session-cookie
        // values caches one negative entry per distinct sid, and nothing
        // ever evicted them — unbounded memory growth. The amortized
        // sweep must reap them once they age past the cache TTL.
        let _guard = crate::db::pg_test_lock().lock().await;
        let cache_ttl = Duration::from_millis(50);
        let store = PostgresAdminSessionStore::connect_with(
            &pg_url(),
            Duration::from_secs(60),
            cache_ttl,
        )
        .await
        .unwrap();
        reset_rows(&store.pool).await;

        // The spray: distinct well-formed but unknown sids (malformed ones
        // are refused before the cache), one cached negative each.
        for i in 0..200 {
            assert!(store.validate(&format!("{i:064x}")).await.is_none());
        }
        assert!(
            store.cache_len() >= 200,
            "negatives are cached (the anti-hammer behaviour is kept)"
        );

        // Age them past the cache TTL, then cross the sweep threshold.
        tokio::time::sleep(cache_ttl + Duration::from_millis(20)).await;
        for _ in 0..(SWEEP_EVERY as usize + 8) {
            let _ = store.validate("bogus-fixed").await;
        }
        assert!(
            store.cache_len() < 16,
            "expired negative entries must be swept; cache still holds {}",
            store.cache_len()
        );
    }

    #[tokio::test]
    async fn expired_session_returns_none_and_is_pruned() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let store =
            PostgresAdminSessionStore::connect_with(&pg_url(), Duration::from_millis(0), Duration::from_millis(50))
                .await
                .unwrap();
        reset_rows(&store.pool).await;
        let sid = store.create(Role::Viewer, None).await;
        // Sleep past the cache window so validate hits PG.
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(
            store.validate(&sid).await.is_none(),
            "expired session is invalid"
        );
        // Lazy GC removed the row.
        let row = sqlx::query("SELECT 1 FROM admin_sessions WHERE sid = $1")
            .bind(&sid)
            .fetch_optional(&store.pool)
            .await
            .unwrap();
        assert!(row.is_none(), "expired row should be GC'd on lookup");
    }

    /// Security review 2026-11-19 (finding 4): a sign-out whose DELETE
    /// fails used to be logged and forgotten, so the session came back
    /// once the database recovered. Now this node denies it at once, the
    /// caller is told it was not recorded, and the worker deletes it when
    /// it can. (Account-wide revocation no longer depends on its DELETE:
    /// the account's authentication generation ends those sessions.)
    /// Simulated by renaming the table for the duration of the sign-out.
    #[tokio::test]
    async fn a_failed_revocation_is_denied_and_retried() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let store = PostgresAdminSessionStore::connect_with(&pg_url(), DEFAULT_TTL, Duration::from_millis(50))
            .await
            .unwrap();
        reset_rows(&store.pool).await;
        let signed_out = store.create(Role::Viewer, Some("mallory".into())).await;
        let other = store.create(Role::Viewer, Some("mallory".into())).await;
        sqlx::query("ALTER TABLE admin_sessions RENAME TO admin_sessions_away")
            .execute(&store.pool)
            .await
            .unwrap();
        assert!(!store.remove(&signed_out).await, "the caller learns it was not recorded");
        sqlx::query("ALTER TABLE admin_sessions_away RENAME TO admin_sessions")
            .execute(&store.pool)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
        assert!(store.validate(&signed_out).await.is_none(), "denied although the row survived");
        assert!(store.validate(&other).await.is_some(), "the account's other session stands");
        // The worker deletes the row once the database answers.
        tokio::time::sleep(REVOKE_RETRY_EVERY + Duration::from_millis(500)).await;
        let row = sqlx::query("SELECT 1 FROM admin_sessions WHERE sid = $1")
            .bind(&signed_out)
            .fetch_optional(&store.pool)
            .await
            .unwrap();
        assert!(row.is_none(), "the retry reached the database");
        assert!(store.pending.sids.is_empty());
        assert!(store.remove(&other).await, "a working database records the sign-out");
    }

    /// The generation each session was issued under round-trips through
    /// the table (the check itself is `auth::confirm_session`).
    #[tokio::test]
    async fn the_generation_is_stored_with_the_session() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let store = PostgresAdminSessionStore::connect_with(&pg_url(), DEFAULT_TTL, Duration::from_millis(1))
            .await
            .unwrap();
        reset_rows(&store.pool).await;
        let sid = store.create_at_epoch(Role::Editor, Some("quinn".into()), 7, None).await;
        tokio::time::sleep(Duration::from_millis(20)).await; // past the cache: read the row
        assert_eq!(store.validate(&sid).await.unwrap().epoch, 7);
    }

    #[tokio::test]
    async fn malformed_ids_never_reach_postgres_and_retries_are_bounded() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let store = PostgresAdminSessionStore::connect(&pg_url()).await.unwrap();
        reset_rows(&store.pool).await;
        for bad in ["\0", "x", &"g".repeat(64), &"A".repeat(64)] {
            assert!(!well_formed_sid(bad));
            store.remove(bad).await;
            assert!(store.validate(bad).await.is_none());
        }
        assert!(store.pending.sids.is_empty(), "nothing queued for a malformed id");
        assert!(well_formed_sid(&mint_session_id()));
    }

    /// Automated security review of #1123: above the bound, a revocation
    /// of a session this node knows is real must still be denied and
    /// retried — only made-up ids are dropped.
    #[tokio::test]
    async fn a_known_session_is_revoked_even_over_the_bound() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let store = PostgresAdminSessionStore::connect(&pg_url()).await.unwrap();
        reset_rows(&store.pool).await;
        let real = store.create(Role::Admin, Some("nina".into())).await; // cached positive
        for i in 0..MAX_PENDING_REVOCATIONS {
            store.pending.sids.insert(format!("{i:064x}"), ());
        }
        sqlx::query("ALTER TABLE admin_sessions RENAME TO admin_sessions_away")
            .execute(&store.pool)
            .await
            .unwrap();
        store.remove(&real).await;
        let unknown = format!("{:064x}", u64::MAX);
        store.remove(&unknown).await;
        sqlx::query("ALTER TABLE admin_sessions_away RENAME TO admin_sessions")
            .execute(&store.pool)
            .await
            .unwrap();
        assert!(store.pending.sids.contains_key(&real), "the real session is queued");
        assert!(!store.pending.sids.contains_key(&unknown), "a made-up id over the bound is not");
        assert!(store.validate(&real).await.is_none(), "and denied meanwhile");
        // The single worker clears the queue once the database answers.
        tokio::time::sleep(REVOKE_RETRY_EVERY + Duration::from_millis(800)).await;
        assert!(store.pending.sids.is_empty(), "drained in one pass");
        let row = sqlx::query("SELECT 1 FROM admin_sessions WHERE sid = $1")
            .bind(&real)
            .fetch_optional(&store.pool)
            .await
            .unwrap();
        assert!(row.is_none());
    }
}
