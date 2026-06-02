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
use std::time::{Duration, Instant};
use tracing::warn;

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
    /// repeated lookups of a stale cookie also skip the DB.
    cache: DashMap<String, CachedEntry>,
}

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
        })
    }

    /// Returns the current node-local cache size. Used by the
    /// `postgres-it` tests to verify the hot path actually serves
    /// from cache.
    #[doc(hidden)]
    pub fn cache_len(&self) -> usize {
        self.cache.len()
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
    async fn create(&self, role: Role, actor: Option<String>) -> String {
        let id = mint_session_id();
        let expires_at: DateTime<Utc> = Utc::now() + chrono::Duration::from_std(self.ttl).unwrap();
        let result = sqlx::query(
            "INSERT INTO admin_sessions (sid, role, actor, expires_at)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(&id)
        .bind(role_to_str(role))
        .bind(&actor)
        .bind(expires_at)
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
                info: Some(SessionInfo { role, actor }),
                valid_until,
            },
        );
        id
    }

    async fn validate(&self, id: &str) -> Option<SessionInfo> {
        let now = Instant::now();

        // Cache hit (fresh) — the hot path.
        if let Some(entry) = self.cache.get(id) {
            if entry.valid_until > now {
                return entry.info.clone();
            }
        }

        // Cache miss / stale — query Postgres.
        let row = sqlx::query(
            "SELECT role, actor, expires_at
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
                    (Some(SessionInfo { role, actor }), Some(expires_at))
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

    async fn remove(&self, id: &str) {
        // Best-effort delete + cache eviction (write-through). If the
        // DB delete fails the cache still drops the entry, so the
        // signed-out user can't bounce on this node.
        if let Err(e) = sqlx::query("DELETE FROM admin_sessions WHERE sid = $1")
            .bind(id)
            .execute(&self.pool)
            .await
        {
            warn!(error = ?e, "admin session DELETE failed");
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
    }

    async fn revoke_by_actor(&self, actor: &str) {
        // Authoritative delete of every session for this user (#544).
        if let Err(e) = sqlx::query("DELETE FROM admin_sessions WHERE actor = $1")
            .bind(actor)
            .execute(&self.pool)
            .await
        {
            warn!(error = ?e, "admin session revoke-by-actor DELETE failed");
        }
        // Evict this node's positive cache entries for the actor so it
        // stops serving them immediately; negative tombstones and other
        // users' entries stay. Sibling nodes ride the cache_ttl window,
        // same as logout.
        self.cache
            .retain(|_, e| e.info.as_ref().and_then(|i| i.actor.as_deref()) != Some(actor));
    }
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
}
