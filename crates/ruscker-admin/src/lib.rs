//! # ruscker-admin
//!
//! Admin panel and public landing for Ruscker.
//!
//! ## Status (Phase 1)
//!
//! - Public landing page served from `Config` (read-only)
//! - i18n scaffold: pt-BR (100%), en/es/fr (placeholders)
//! - Theme: light/dark via cookie + `prefers-color-scheme`
//!
//! Admin CRUD (Phase 2), proxy (Phase 3), dashboard (Phase 4) come
//! later.

use anyhow::{Context, Result};
use axum::Router;
use ruscker_config::Config;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tokio::net::TcpListener;
use tower_cookies::CookieManagerLayer;
use tracing::info;

pub mod access_counter;
pub mod activity;
pub mod alerts;
pub mod auth;
pub mod catalog;
pub mod crypto;
pub mod db;
pub mod i18n;
pub mod images;
pub mod jobs;
pub mod leader;
pub mod logbuf;
pub mod markdown;
pub mod metrics_cache;
pub mod mfa;
pub mod ratelimit;
pub mod routes;
pub mod admin_sessions_pg;
pub mod scaler;
pub mod sessions;
pub mod sessions_pg;
pub mod theme;
pub mod view_model;

use sqlx::SqlitePool;

/// The running Ruscker version, from the workspace `Cargo.toml` at
/// compile time. Surfaced in the landing + admin footers (#241) and
/// reported by `/healthz`. Pathed (`crate::APP_VERSION`) from the
/// templates so it never drifts from a hardcoded string.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Tracing target for the one-shot startup banner (#452). The CLI's
/// default `EnvFilter` raises *just this target* to `info` so the boot
/// summary always lands in the admin "Logs" tab — even at the default
/// `warn` verbosity, where every other `ruscker*` info log is hidden.
/// `EnvFilter` prefix-matches and prefers the longest match, so
/// `ruscker=warn,ruscker_startup=info` shows the banner without
/// un-muting the rest of the app. Keep this in sync with the directive
/// in `ruscker-cli`'s `init_tracing`.
pub const STARTUP_LOG_TARGET: &str = "ruscker_startup";

/// Identity attributes cached together for the proxy hot path (#1001).
/// `celular`, roles, and authentication tokens are intentionally absent:
/// they are not supported upstream identity claims.
#[derive(Default)]
pub struct CachedIdentity {
    pub(crate) groups: Arc<Vec<String>>,
    pub(crate) email: Option<String>,
    pub(crate) setor: Option<String>,
}

/// Shared short-TTL cache of user identity attributes for the proxy hot
/// path (#1001).
///
/// The generation counter closes an invalidation race (codex review):
/// a proxied request that read the DB *before* an admin mutation must
/// not repopulate the cache *after* that mutation invalidated it —
/// else a revoked membership could survive locally for the full TTL.
/// Every entry records the generation that was current when its fill
/// STARTED (snapshotted before the DB read), and [`IdentityCache::get`]
/// rejects entries from an older generation. A stale fill may thus
/// physically land in the map after an invalidation, but it is
/// unreadable — no check-then-insert window exists by construction.
#[derive(Default)]
pub struct IdentityCache {
    map: dashmap::DashMap<String, (u64, std::time::Instant, Arc<CachedIdentity>)>,
    generation: std::sync::atomic::AtomicU64,
}

impl IdentityCache {
    /// Cached identity for `username`, if fresher than `ttl` AND filled
    /// under the current generation (i.e. not invalidated since).
    pub fn get(&self, username: &str, ttl: std::time::Duration) -> Option<Arc<CachedIdentity>> {
        let entry = self.map.get(username)?;
        let (generation, filled_at, identity) = entry.value();
        (*generation == self.generation() && filled_at.elapsed() < ttl)
            .then(|| identity.clone())
    }

    /// Snapshot to take BEFORE the DB read that will feed [`Self::store`].
    pub fn generation(&self) -> u64 {
        self.generation.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Cache a resolved identity, tagged with the generation that was
    /// current when the fill began — if an invalidation raced this fill,
    /// the entry lands already-expired and `get` never serves it.
    pub fn store(&self, generation: u64, username: &str, identity: Arc<CachedIdentity>) {
        self.map.insert(
            username.to_string(),
            (generation, std::time::Instant::now(), identity),
        );
    }

    /// Expire every entry — current AND in-flight (generation bump);
    /// the clear just reclaims memory early.
    pub fn invalidate(&self) {
        self.generation
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        self.map.clear();
    }
}

/// Shared state injected into every request.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub locales: Arc<i18n::Locales>,
    /// Normalized base path the portal is served under (#173): `""` for
    /// root (default) or e.g. `"/box"`. The router nests everything
    /// under it and the response rewriter prefixes generated URLs /
    /// redirects / cookie paths with it.
    pub base_path: Arc<str>,
    pub admin_auth: auth::AdminAuth,
    /// Opaque server-side admin session store. The cookie holds a
    /// random session id (not the token); shared so every cloned
    /// `AppState` sees the same live sessions. `Arc<dyn …>` so the
    /// in-memory default and the HA Postgres-backed store (#185) drop
    /// in identically.
    pub admin_sessions: Arc<dyn auth::AdminSessionStore>,
    /// Global rate limiter for `/admin/login` — bounds brute
    /// force against the admin token. Shared (Arc) so every
    /// cloned `AppState` sees the same window.
    pub login_limiter: Arc<auth::LoginRateLimiter>,
    /// Per-client, per-spec request limiter enforcing each API
    /// spec's `api.rate-limit`. Shared (Arc) so every cloned
    /// `AppState` sees the same sliding windows. Specs without a
    /// configured limit never touch it.
    pub api_limiter: Arc<ratelimit::ApiRateLimiter>,
    /// Optional admin config database (SQLite by default, Postgres in
    /// HA mode). `None` ⇒ admin CRUD routes 503 because they have no
    /// source of truth. Reach the SQLite pool for not-yet-ported
    /// repositories via [`AppState::sqlite`].
    pub db: Option<db::ConfigDb>,
    /// On-disk fallback directory for `/assets/img/<file>`. When
    /// the DB lookup misses (or `db` is `None`), the assets route
    /// falls through to this directory before 404'ing. `None`
    /// disables the disk fallback entirely.
    pub images_dir: Option<Arc<Path>>,
    /// Master key for the credentials store. When unset, the
    /// `/admin/credentials` route 503s with a hint to set
    /// `RUSCKER_MASTER_KEY`.
    pub master_key: crypto::MasterKey,

    /// Container backend used by the proxy to spawn/list/stop
    /// replicas. `None` ⇒ proxy routes (`/app/*`, `/api/*`) return
    /// 503 with a hint that no backend is wired (Phase 1/2 mode,
    /// landing-only).
    pub backend: Option<std::sync::Arc<dyn ruscker_core::ContainerBackend>>,

    /// Recent Ruscker log lines for the admin "Logs" tab. `None` when no
    /// log buffer was wired (e.g. tests, or non-`serve` commands).
    pub log_buffer: Option<logbuf::LogBuffer>,

    /// Live registry of running replicas, keyed by spec id.
    /// Shared across handlers via an RwLock; writes happen only
    /// on spawn / stop, reads happen per-request to pick a replica.
    pub replicas: std::sync::Arc<tokio::sync::RwLock<ruscker_core::ReplicaRegistry>>,

    /// HMAC key used to sign sticky-session cookies. Auto-
    /// generated per-process unless `RUSCKER_COOKIE_KEY` is set.
    pub cookie_key: ruscker_proxy::sticky::CookieKey,

    /// Per-spec coalescer for first-request spawns. The fast path
    /// in `pick_or_spawn` only touches the read lock on
    /// `replicas`; on miss it acquires this spec's mutex, double-
    /// checks, and (if still empty) does the spawn. Concurrent
    /// first-requests for **different** specs go in parallel
    /// because the mutex is per-key. Concurrent first-requests
    /// for the **same** spec wait for one spawn instead of
    /// racing.
    pub spawn_locks: std::sync::Arc<
        dashmap::DashMap<String, std::sync::Arc<tokio::sync::Mutex<()>>>,
    >,

    /// Per-visitor session tracker. The proxy calls
    /// `touch_or_register` on every routed request so the
    /// replica's `sessions_active` reflects the real number of
    /// live visitors. A background sweeper (started from
    /// `AdminServer::run`) evicts idle sessions after
    /// `proxy.heartbeat_timeout` ms.
    pub sessions: std::sync::Arc<dyn sessions::SessionStore>,

    /// `stop-on-logout` index (#337): maps a signed-in username to the
    /// sticky app-session ids it has open **on specs with
    /// `stop-on-logout: true`**. The proxy records an entry when such a
    /// session is first registered for a known user; the logout handler
    /// drains the user's set and ends those sessions immediately (instead
    /// of waiting for the heartbeat sweep), so the replica goes idle and
    /// the scaler reaps it. Process-local — consistent with the
    /// process-local admin-session store (HA prescribes a sticky upstream
    /// for session-bearing paths, so a user's logout and app sessions
    /// land on the same instance). Best-effort: stale ids are harmless
    /// no-ops at logout.
    pub logout_index: std::sync::Arc<dashmap::DashMap<String, std::collections::HashSet<uuid::Uuid>>>,

    /// Decides whether this instance runs the auto-scaler. Single-node
    /// installs use [`leader::AlwaysLeader`]; HA installs inject a
    /// [`leader::PgLeaderLock`] so exactly one instance scales. The
    /// proxy and session tracking never consult this — only the scaler.
    pub leader: std::sync::Arc<dyn leader::LeaderElector>,

    /// Per-replica metrics cache. Filled by a background
    /// refresher that fans out `backend.metrics()` calls every
    /// [`metrics_cache::REFRESH_INTERVAL`]; the dashboard reads
    /// straight from here without ever waiting on Docker.
    pub metrics: metrics_cache::MetricsCache,

    /// Set to `true` when a graceful shutdown begins (SIGTERM /
    /// Ctrl-C). While set, `/readyz` reports `draining` (503) so
    /// load balancers stop routing new traffic before the listener
    /// closes. Shared (Arc) so the signal handler and every request
    /// handler observe the same flag.
    pub draining: Arc<std::sync::atomic::AtomicBool>,

    /// Hot-path spec cache (#587): `find_spec` runs on every proxied
    /// request (incl. every subresource), and a DB-first lookup parses
    /// `config_json` each time. This caches the resolved spec by id for a
    /// short TTL ([`crate::routes::proxy::SPEC_CACHE_TTL`]) so a page
    /// load's burst of requests hits memory, not the DB. Only positive
    /// results are cached (so the map stays bounded by the real catalog);
    /// the TTL bounds staleness so an admin edit takes effect without any
    /// explicit invalidation wiring. Shared so every cloned `AppState`
    /// sees the same cache.
    pub spec_cache: Arc<dashmap::DashMap<String, (Arc<ruscker_config::Spec>, std::time::Instant)>>,

    /// Short-lived username → group-membership cache for the proxy hot
    /// path (#1001). Identity-enabled pages fetch many assets; resolving
    /// the same signed-in user from the DB for each one would turn a page
    /// load into a burst of identical SELECTs. The TTL is a backstop —
    /// admin handlers that mutate membership call
    /// [`AppState::invalidate_identity_cache`] so revocations apply on
    /// the very next proxied request, matching the pre-cache semantics.
    pub identity_cache: Arc<IdentityCache>,

    /// Short-circuit cache of the **effective spec catalog** for admin
    /// pages (#902). The admin tabs (Apps/Disk/Media/Groups/System +
    /// landing) each rebuilt the full catalog — `list_all` + a
    /// `config_json` deserialize of every spec — on every navigation.
    /// This holds the last-built `Arc<Vec<Spec>>` keyed by a cheap catalog
    /// signature (`db::specs::catalog_signature`: count + Σversion + max
    /// updated_at); a cache hit skips the deserialize entirely, and any
    /// write moves the signature so it's never stale (HA-safe — a write on
    /// any node changes the signature every reader observes). `Arc` so all
    /// cloned `AppState`s share it in-process while each test gets its own.
    pub catalog_cache: catalog::CatalogCache,

    /// In-memory buffer for the per-spec access counter (#944). The
    /// proxy hot path bumps it synchronously; a single drain task
    /// (started from `AdminServer::run`) batches the deltas into the DB
    /// every couple of seconds, so writes track specs × flush windows —
    /// not the request rate. Shared so every cloned `AppState` and the
    /// drain task see the same buffer.
    pub access_counter: Arc<access_counter::AccessCounter>,

    /// Alert-webhook sink (#930). Emit sites (scaler, spawn paths)
    /// call `notify` — a cheap synchronous enqueue with per-(kind,
    /// spec) cooldown; one sender task (started in `AdminServer::run`
    /// when a DB is wired) POSTs the events to the operator-configured
    /// webhook URL.
    pub alerts: alerts::AlertSink,
    /// User-activity sink (#1021). Capture sites (login, new app session)
    /// call `record` — a non-blocking enqueue; one drain task (started in
    /// `AdminServer::run` when a DB is wired) batch-writes to `user_activity`.
    pub activity: activity::ActivitySink,
}

impl AppState {
    /// The SQLite pool when the config DB is SQLite, else `None`.
    /// Repositories not yet ported to Postgres call this; in Postgres
    /// mode they get `None` and 503. See [`db::ConfigDb`].
    pub fn sqlite(&self) -> Option<&SqlitePool> {
        self.db.as_ref().and_then(db::ConfigDb::as_sqlite)
    }

    /// Drop every cached identity resolution (#1001). Admin handlers
    /// call this after any mutation that can change group membership or
    /// remove an account (user edit/delete, group rename/membership,
    /// CSV import), so a revoked group loses app access on the very
    /// next proxied request — the 30s TTL is only a backstop for edits
    /// made outside this process (e.g. another HA node).
    pub fn invalidate_identity_cache(&self) {
        self.identity_cache.invalidate();
    }
}

/// HTTP server hosting the landing and (later) the admin panel.
pub struct AdminServer {
    addr: SocketAddr,
    state: AppState,
    /// Operator-provided directory served at `/assets/img/`. Cards
    /// reference logos via `template-properties.logo:
    /// "/assets/img/<file>"` (the ShinyProxy convention). `None`
    /// means no `/assets/img/` route is mounted — cards fall back
    /// to tint-only covers.
    images_dir: Option<PathBuf>,
}

impl AdminServer {
    /// Build a server bound to `addr`, serving the given config.
    ///
    /// The config is what the landing page reads to render cards.
    /// In Phase 2 this is replaced with a SQLite-backed source of
    /// truth and the YAML becomes import/export only.
    pub fn new(addr: SocketAddr, config: Config) -> Result<Self> {
        let locales = i18n::Locales::load().context("load locale bundles")?;
        let state = AppState {
            config: Arc::new(config),
            locales: Arc::new(locales),
            base_path: Arc::from(""),
            admin_auth: auth::AdminAuth::from_env(),
            admin_sessions: Arc::new(auth::InMemoryAdminSessionStore::default_policy()),
            login_limiter: Arc::new(auth::LoginRateLimiter::default_policy()),
            api_limiter: Arc::new(ratelimit::ApiRateLimiter::new()),
            db: None,
            images_dir: None,
            master_key: crypto::MasterKey::from_env().context("load master key")?,
            backend: None,
            log_buffer: None,
            replicas: std::sync::Arc::new(tokio::sync::RwLock::new(
                ruscker_core::ReplicaRegistry::new(),
            )),
            cookie_key: ruscker_proxy::sticky::CookieKey::from_env_or_random()
                .context("load sticky cookie key")?,
            spawn_locks: std::sync::Arc::new(dashmap::DashMap::new()),
            sessions: std::sync::Arc::new(sessions::InMemorySessionStore::new()),
            logout_index: std::sync::Arc::new(dashmap::DashMap::new()),
            leader: std::sync::Arc::new(leader::AlwaysLeader),
            metrics: metrics_cache::MetricsCache::new(),
            draining: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            spec_cache: Arc::new(dashmap::DashMap::new()),
            identity_cache: Default::default(),
            catalog_cache: Arc::new(tokio::sync::RwLock::new(None)),
            access_counter: Arc::new(access_counter::AccessCounter::default()),
            alerts: alerts::AlertSink::default(),
            activity: activity::ActivitySink::default(),
        };
        Ok(Self {
            addr,
            state,
            images_dir: None,
        })
    }

    /// Set the on-disk fallback directory for `/assets/img/<file>`.
    /// When the DB image library has the filename it wins; otherwise
    /// the handler reads from this directory. `None` disables the
    /// fallback (DB-only mode).
    pub fn with_images_dir(mut self, dir: impl AsRef<Path>) -> Self {
        let pathbuf = dir.as_ref().to_path_buf();
        let arc: Arc<Path> = Arc::from(pathbuf.clone().into_boxed_path());
        self.images_dir = Some(pathbuf);
        self.state.images_dir = Some(arc);
        self
    }

    /// Override the **admin** token (default: pulled from
    /// `RUSCKER_ADMIN_TOKEN` env var). Useful for tests that need
    /// a known token without touching the process environment.
    ///
    /// Only the admin token is replaced — the optional `editor` /
    /// `viewer` tokens that [`auth::AdminAuth::from_env`] already read
    /// stay intact, so the CLI's `--admin-token` flag (which clap also
    /// feeds from `RUSCKER_ADMIN_TOKEN`) doesn't wipe the other roles.
    pub fn with_admin_token(mut self, token: impl Into<String>) -> Self {
        self.state.admin_auth.admin = Some(Arc::from(token.into()));
        self
    }

    /// Override the credentials master key (default: pulled from
    /// `RUSCKER_MASTER_KEY`). Accepts hex (64ch) or base64 (44ch).
    pub fn with_master_key(mut self, raw: impl AsRef<str>) -> Result<Self> {
        self.state.master_key = crypto::MasterKey::parse(raw.as_ref())?;
        Ok(self)
    }

    /// Wire the in-memory log buffer feeding the admin "Logs" tab.
    /// Created and populated by a `tracing` layer in `ruscker-cli`.
    pub fn with_log_buffer(mut self, buffer: logbuf::LogBuffer) -> Self {
        self.state.log_buffer = Some(buffer);
        self
    }

    /// Serve the whole portal under a base path (#173), e.g. `/box` for
    /// mounting at `example.org/box/`. Normalized via
    /// [`ruscker_config::normalize_base_path`]; empty ⇒ root (default).
    pub fn with_base_path(mut self, path: impl AsRef<str>) -> Self {
        let norm = ruscker_config::normalize_base_path(path.as_ref());
        self.state.base_path = Arc::from(norm.as_str());
        self
    }

    /// Attach a container backend (e.g. `LocalDockerBackend`).
    /// Without this, the proxy routes `/app/*` and `/api/*` reply
    /// 503 with a hint that no backend is wired.
    pub fn with_backend(
        mut self,
        backend: std::sync::Arc<dyn ruscker_core::ContainerBackend>,
    ) -> Self {
        self.state.backend = Some(backend);
        self
    }

    /// Swap in a different session store. The default is the
    /// single-node [`sessions::InMemorySessionStore`]; HA deployments
    /// pass a [`sessions_pg::PostgresSessionStore`] so several Ruscker
    /// instances share one session table. The proxy and sweeper only
    /// ever see the `dyn SessionStore`, so nothing else changes.
    pub fn with_session_store(
        mut self,
        store: std::sync::Arc<dyn sessions::SessionStore>,
    ) -> Self {
        self.state.sessions = store;
        self
    }

    /// Replace the admin sign-in session store (#185). Default is
    /// the in-memory [`auth::InMemoryAdminSessionStore`]; pass an
    /// [`admin_sessions_pg::PostgresAdminSessionStore`] to make
    /// sign-in sessions survive a load-balancer hop between HA
    /// instances.
    pub fn with_admin_session_store(
        mut self,
        store: std::sync::Arc<dyn auth::AdminSessionStore>,
    ) -> Self {
        self.state.admin_sessions = store;
        self
    }

    /// Attach a SQLite pool. Required for the `/admin/*` routes
    /// that read or write the spec catalog. The pool is shared
    /// across all requests — sqlx handles the connection
    /// multiplexing.
    pub fn with_db(mut self, pool: SqlitePool) -> Self {
        self.state.db = Some(db::ConfigDb::Sqlite(pool));
        self
    }

    /// Attach a config database directly as a [`db::ConfigDb`] — used to
    /// select the shared Postgres catalog for HA. `with_db` is the
    /// SQLite shorthand for `with_config_db(ConfigDb::Sqlite(pool))`.
    pub fn with_config_db(mut self, db: db::ConfigDb) -> Self {
        self.state.db = Some(db);
        self
    }

    /// Replace the leader elector. Default is [`leader::AlwaysLeader`]
    /// (single node). HA installs pass a [`leader::PgLeaderLock`] so
    /// only one instance runs the auto-scaler.
    pub fn with_leader_elector(
        mut self,
        elector: std::sync::Arc<dyn leader::LeaderElector>,
    ) -> Self {
        self.state.leader = elector;
        self
    }

    /// Start listening. Blocks until the process is shut down.
    pub async fn run(self) -> Result<()> {
        // Reconcile the in-memory replica registry with whatever
        // the backend reports as already running. This lets
        // Ruscker survive its own restart without losing track
        // of containers spawned in a prior incarnation — the
        // `ruscker.replica_id` label on each container is the
        // durable identifier.
        if let Some(backend) = self.state.backend.as_ref() {
            match backend.list().await {
                Ok(mut existing) => {
                    // The backend can't know each replica's seat cap —
                    // that lives in the spec config. Enrich each reconciled
                    // replica with its spec's `effective_seats` so
                    // `sessions_max` is accurate and the scaler's saturation
                    // / available-seats math works on first tick.
                    //
                    // Resolve from the EFFECTIVE catalog (DB ∪ YAML), not
                    // just `proxy.specs`: a spec created in the admin is
                    // DB-only, so a YAML-only lookup left its reconciled
                    // replica at `sessions_max = 0` → `available_seats() == 0`
                    // → never accepting, so after a restart an already-running
                    // DB-only app looked permanently full (#907).
                    let catalog =
                        catalog::effective_specs(self.state.db.as_ref(), &self.state.config).await;
                    apply_seat_caps(&mut existing, &catalog);
                    let n = existing.len();
                    self.state.replicas.write().await.reset(existing);
                    if n > 0 {
                        info!(
                            target: STARTUP_LOG_TARGET,
                            replicas = n,
                            "reconciled existing replicas from backend"
                        );
                    }
                }
                Err(err) => {
                    tracing::warn!(error = ?err, "backend.list() failed at startup; starting with empty registry");
                }
            }
        }

        // Start the auto-scaler. With no backend wired the task is
        // still spawned but its loop short-circuits on every tick,
        // so this is safe in landing-only mode. The JoinHandle is
        // deliberately dropped — there's no graceful shutdown
        // protocol for the scaler because every tick is idempotent.
        if let Some(backend) = self.state.backend.clone() {
            // These `spawn`s return a `JoinHandle` (itself a future) we
            // deliberately detach — each loop is idempotent and has no
            // shutdown protocol — so the `let_underscore_future` lint is
            // a false positive here.
            #[allow(clippy::let_underscore_future)]
            let _ = scaler::spawn(self.state.clone(), scaler::DEFAULT_INTERVAL);
            // Docker events watcher (#1018 slice B): reconciles within ~1 s of
            // an external `docker rm -f` / `docker restart` instead of waiting
            // for the periodic scaler tick. The periodic reconcile above stays
            // as the fallback; backends without event support park on an empty
            // stream. Detached like the scaler — every reconcile is idempotent.
            #[allow(clippy::let_underscore_future)]
            let _ = scaler::spawn_event_watcher(self.state.clone());
            // Session sweeper: evicts idle sessions per the
            // global `heartbeat-timeout`. `-1` (the ShinyProxy
            // idiom for "never expire") becomes a no-op loop
            // inside `sessions::spawn` so the call shape stays
            // uniform.
            #[allow(clippy::let_underscore_future)]
            let _ = sessions::spawn(
                self.state.sessions.clone(),
                self.state.replicas.clone(),
                self.state.config.clone(),
                self.state.leader.clone(),
                self.state.db.clone(),
            );
            // Dashboard metrics: keep `state.metrics` fresh in
            // the background so dashboard renders are read-only
            // and never block on a Docker stats call.
            #[allow(clippy::let_underscore_future)]
            let _ = metrics_cache::spawn(
                self.state.metrics.clone(),
                backend,
                self.state.replicas.clone(),
                // `proxy.metrics-interval` (seconds; 0 ⇒ 5 s default) —
                // a busy host can poll Docker stats less often (#288).
                std::time::Duration::from_secs(self.state.config.proxy.effective_metrics_interval_secs()),
            );
            info!(
                target: STARTUP_LOG_TARGET,
                "background workers started (scaler, session sweeper, metrics)"
            );
        }

        // Access-counter drain (#944): the proxy hot path only bumps an
        // in-memory buffer; this single supervised task batches the
        // deltas into the DB. Needs only the DB, not a backend (external
        // specs count clicks even in landing-only mode). Detached like
        // the loops above — every flush is idempotent-by-delta, and the
        // final flush on shutdown runs explicitly below.
        if let Some(db) = self.state.db.clone() {
            #[allow(clippy::let_underscore_future)]
            let _ = access_counter::spawn(self.state.access_counter.clone(), db);
        }

        // Job scheduler (#986 slice B): leader-only cron runner for
        // run-to-completion jobs. Needs BOTH a DB (schedules live
        // there) and a backend (something must run the container).
        if self.state.db.is_some() && self.state.backend.is_some() {
            #[allow(clippy::let_underscore_future)]
            let _ = jobs::spawn(self.state.clone());
        }

        // Alert-webhook sender (#930): one task drains the alert queue
        // and POSTs to the operator-configured URL (settings table).
        // Detached like the loops above; no shutdown flush — alerts
        // about a server that is going down on purpose are noise.
        if let Some(db) = self.state.db.clone() {
            #[allow(clippy::let_underscore_future)]
            let _ = alerts::spawn(&self.state.alerts, db);
        }

        // User-activity drain (#1021): one task batch-writes login/app-access
        // events to `user_activity`. Capture sites (login, new app session)
        // enqueue non-blocking; without a DB there's nowhere to write, so the
        // task isn't started and `record` just drops into a never-drained
        // queue (bounded — no leak).
        if let Some(db) = self.state.db.clone() {
            #[allow(clippy::let_underscore_future)]
            let _ = activity::spawn(&self.state.activity, db);
        }

        let app = router_with_images(self.state.clone(), self.images_dir.as_deref());
        let listener = TcpListener::bind(self.addr)
            .await
            .with_context(|| format!("bind {}", self.addr))?;
        // One-shot startup banner (#452): the single line operators can
        // count on seeing in the admin "Logs" tab right after boot,
        // confirming the build, where it bound, and which subsystems are
        // wired. Emitted on `STARTUP_LOG_TARGET` so the default filter
        // surfaces it even at `warn`.
        let db_kind = match self.state.db.as_ref() {
            None => "none",
            Some(db::ConfigDb::Sqlite(_)) => "sqlite",
            Some(db::ConfigDb::Postgres(_)) => "postgres",
        };
        let base = if self.state.base_path.is_empty() {
            "/"
        } else {
            &self.state.base_path
        };
        info!(
            target: STARTUP_LOG_TARGET,
            version = APP_VERSION,
            addr = %self.addr,
            base_path = base,
            docker = self.state.backend.is_some(),
            db = db_kind,
            specs = self.state.config.proxy.specs.len(),
            images_dir = ?self.images_dir,
            // The scheduler's clock (#1042). Worth a line in the banner:
            // an operator who set `timezone` in the wrong file or section
            // gets a silent UTC fallback otherwise, and would only find
            // out when a nightly job fires hours off.
            timezone = self.state.config.server.effective_timezone().name(),
            "ruscker started — listening"
        );
        // `into_make_service_with_connect_info` exposes the TCP peer
        // address to handlers via `ConnectInfo<SocketAddr>` — the
        // proxy uses it as the per-client key for API rate limiting
        // when no trusted `X-Forwarded-For` is present.
        let make_service =
            app.into_make_service_with_connect_info::<SocketAddr>();
        axum::serve(listener, make_service)
            .with_graceful_shutdown(shutdown_signal(self.state.clone()))
            .await
            .context("axum serve")?;
        // Final access-counter flush (#944): in-flight requests have
        // finished, so their bumps are in the buffer. Bounded — a dead
        // DB must not hold up shutdown past the watchdog.
        if let Some(db) = self.state.db.as_ref() {
            let flush = self.state.access_counter.flush(db);
            match tokio::time::timeout(std::time::Duration::from_secs(5), flush).await {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => tracing::warn!(
                    error = ?e,
                    lost = self.state.access_counter.backlog(),
                    "final access-counter flush failed; pending counts lost"
                ),
                Err(_) => tracing::warn!(
                    lost = self.state.access_counter.backlog(),
                    "final access-counter flush timed out; pending counts lost"
                ),
            }
        }
        info!("shutdown complete");
        Ok(())
    }
}

/// Resolves when the process receives a termination signal, then
/// runs the graceful-drain sequence. Returning from this future is
/// what tells `axum::serve` to stop accepting new connections and
/// finish the in-flight ones.
///
/// Sequence on signal:
/// 1. Flip [`AppState::draining`] so `/readyz` starts replying
///    `503 draining` — load balancers deregister this instance.
/// 2. Arm a hard-deadline watchdog that force-exits the process if
///    draining overruns. Long-lived WebSocket sessions (Shiny,
///    Streamlit) never close on their own, so without this the
///    in-flight wait would hang forever.
/// 3. Wait for active sessions to drain, polling
///    [`sessions::SessionStore::len`], up to
///    `proxy.shutdown-grace-ms`. Exits the wait early the moment
///    the last session ends.
async fn shutdown_signal(state: AppState) {
    use std::sync::atomic::Ordering;
    use tokio::time::{sleep, Duration, Instant};

    // SIGINT (Ctrl-C) on every platform; SIGTERM too on Unix
    // (what `systemctl stop` / `docker stop` / k8s send).
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            // If we can't install the handler, never resolve this
            // arm — Ctrl-C still works.
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }

    let grace = Duration::from_millis(state.config.proxy.shutdown_grace_ms);
    let active = state.sessions.len();
    info!(
        grace_ms = state.config.proxy.shutdown_grace_ms,
        active_sessions = active,
        "shutdown signal received; draining"
    );
    state.draining.store(true, Ordering::SeqCst);

    // Watchdog: guarantee the process exits even if in-flight
    // connections (notably long-lived WebSockets) never close. The
    // extra slack covers the time axum spends finishing in-flight
    // HTTP requests after this future resolves.
    let watchdog = grace + Duration::from_secs(5);
    tokio::spawn(async move {
        sleep(watchdog).await;
        tracing::warn!(
            deadline_ms = watchdog.as_millis() as u64,
            "graceful-shutdown deadline exceeded; forcing exit"
        );
        std::process::exit(0);
    });

    // Drain loop: wait for sessions to wind down, but never longer
    // than the grace window. Idle instances (no sessions) fall
    // through immediately.
    let deadline = Instant::now() + grace;
    while !state.sessions.is_empty() && Instant::now() < deadline {
        sleep(Duration::from_millis(250)).await;
    }
    info!(
        remaining_sessions = state.sessions.len(),
        "drain window elapsed; closing listener"
    );
}

/// Compose the axum router. Pulled out so tests can hit it via
/// `Router::oneshot` without a real socket.
pub fn router(state: AppState) -> Router {
    router_with_images(state, None)
}

/// Deprecated alias kept for the CLI's call site — the
/// `images_dir` argument is now ignored because state carries the
/// fallback directory itself (set via [`AdminServer::with_images_dir`]).
pub fn router_with_images(state: AppState, _images_dir: Option<&Path>) -> Router {
    // Precompress the bundled CSS/JS up front so the first request already
    // has the brotli/gzip variants ready, and the CompressionLayer never
    // re-encodes these immutable bytes per request (#593).
    routes::assets::warm_precompression();

    // Ruscker's own surfaces (landing, admin, prefs, assets) get
    // security response headers. The proxy routes do NOT — those
    // forward upstream app responses verbatim, and injecting our
    // X-Frame-Options / CSP into a Shiny page would interfere
    // with how the app expects to be served. Apps own their own
    // security headers.
    let own = Router::new()
        .merge(routes::landing::routes())
        .merge(routes::assets::routes())
        .merge(routes::prefs::routes())
        .merge(routes::admin::routes())
        .layer(axum::middleware::from_fn(security_headers))
        // CSRF defense for the chrome's mutations (#259). Layered on the
        // chrome only — NOT the proxy routes (apps legitimately take
        // cross-origin POSTs). Safe methods pass through untouched.
        // `X-Forwarded-Host` is only trusted in the Origin/Host fallback
        // when the operator opted into forwarded headers (#328).
        .layer(axum::middleware::from_fn({
            let trust = routes::proxy::forward_headers_trusted(&state.config.server);
            move |req: axum::extract::Request, next: axum::middleware::Next| {
                csrf_guard(req, next, trust)
            }
        }))
        // Force a first-login password change (#454): a logged-in account
        // still carrying `must_change_password` is pinned to the password
        // page on every other `/admin/*` route. Layered on the chrome so a
        // redirect's `Location` rides the base-path rewriter below.
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            must_change_password_guard,
        ));

    // Base-path mounting (#173): when served under a subpath, rewrite the
    // chrome's root-absolute URLs / redirects to carry the prefix. Only
    // the chrome needs it — the proxy's `/app` responses get the prefix
    // through their own rewriter, and `/api`/metrics carry no chrome URLs.
    // No-op layer cost when there's no base path (we skip adding it).
    let own = {
        let base = state.base_path.clone();
        if base.is_empty() {
            own
        } else {
            own.layer(axum::middleware::from_fn(
                move |req: axum::extract::Request, next: axum::middleware::Next| {
                    let base = base.clone();
                    async move { routes::rewrite::prefix_base_path(next.run(req).await, &base).await }
                },
            ))
        }
    };

    // gzip/br compression for the chrome's text responses (HTML + the
    // bundled CSS/JS) (#287). Outermost layer, so it compresses the
    // final body — after the base-path rewrite has read it uncompressed,
    // and only for the chrome routes (the proxy forwards upstream bodies
    // verbatim and must not be re-compressed). Honors `Accept-Encoding`.
    // The default predicate already skips images and tiny responses; we
    // also skip `font/*` — woff2 is already compressed, so re-encoding it
    // only burns CPU.
    use tower_http::compression::predicate::{DefaultPredicate, NotForContentType, Predicate};
    let own = own.layer(
        tower_http::compression::CompressionLayer::new()
            .compress_when(DefaultPredicate::new().and(NotForContentType::const_new("font/"))),
    );

    // Health probes (`/healthz`, `/readyz`) sit outside the
    // `security_headers` layer: they return JSON for orchestrators,
    // not HTML for browsers, so CSP / X-Frame-Options are
    // irrelevant. Like the proxy routes, they're merged at the
    // outer level.
    // The full portal surface (chrome + proxy + metrics). Health is
    // kept separate so it can stay at the root even under a base path —
    // load-balancer probes shouldn't have to know the prefix.
    let portal = own
        .merge(routes::proxy::routes())
        // `/metrics` (opt-in via proxy.metrics-enabled) is likewise
        // unauthenticated and outside `security_headers` — it serves
        // Prometheus text for a scraper, not HTML for a browser.
        .merge(routes::metrics::routes());

    // Mount under the configured base path (#173). `""` ⇒ root (the
    // default), so single-host deploys are unchanged. With e.g. `/box`,
    // every portal route matches under `/box/...`; the response rewriter
    // (added in a later slice) prefixes the URLs the handlers emit.
    let base = state.base_path.clone();
    let portal = if base.is_empty() {
        portal
    } else {
        // axum 0.8 `nest` maps the inner `/` route to exactly `/box`
        // (no trailing slash) and does NOT match `/box/` — but that's
        // the URL a browser / nginx sends for the landing root. Redirect
        // `/box/` → `/box` so both forms work (the deeper routes like
        // `/box/admin/...` are matched by nest directly).
        let canonical = base.to_string();
        let trailing = format!("{base}/");
        Router::new().nest(&base, portal).route(
            &trailing,
            axum::routing::get(move || {
                let to = canonical.clone();
                async move { axum::response::Redirect::permanent(&to) }
            }),
        )
    };

    portal
        .merge(routes::health::routes())
        .layer(CookieManagerLayer::new())
        .with_state(state)
}

/// Per-session cache of the `must_change_password` answer for
/// [`must_change_password_guard`] (#903). Keyed by the opaque session id,
/// whose `must_change` state is immutable for its lifetime, so an entry is
/// authoritative until its TTL evicts it — turning a per-navigation
/// `users` SELECT into at most one lookup per session per TTL.
static MUST_CHANGE_CACHE: std::sync::LazyLock<
    dashmap::DashMap<String, (std::time::Instant, bool)>,
> = std::sync::LazyLock::new(dashmap::DashMap::new);

const MUST_CHANGE_TTL: std::time::Duration = std::time::Duration::from_secs(300);

/// Soft cap on [`MUST_CHANGE_CACHE`] entries; above it an insert first
/// drops stale entries so churned-through session ids can't grow the map
/// without bound (same spirit as the rate-limiter sweep, #737).
const MUST_CHANGE_CACHE_CAP: usize = 256;

/// Force a first-login password change (#454).
///
/// A user account created in the admin gets `must_change_password = true`
/// (see `db::users::create`). Login already redirects such a user to the
/// self-service password page, but nothing stopped them from navigating
/// straight to `/admin/dashboard` and keeping the admin-assigned initial
/// password. This guard closes that gap: on any `/admin/*` route other
/// than the password page / login / logout / first-admin setup, a session
/// whose account still needs a change is bounced back to
/// `/admin/account/password`.
///
/// Break-glass token sessions (no `actor`) and anonymous requests pass
/// through untouched — they have no account to rotate, and the routes'
/// own guards handle authentication.
///
/// The `must_change` flag is **immutable for the life of a session id**:
/// changing the password revokes the session and re-mints a fresh id
/// (#544/#555), so a given id never flips from needing-a-change to not.
/// We therefore cache the per-session answer ([`MUST_CHANGE_CACHE`]) and
/// do the indexed `users` lookup at most once per session per TTL, instead
/// of on every gated admin navigation (#903). The panel is low-traffic
/// and the proxy hot path (`/app`, `/api`) never reaches this layer.
async fn must_change_password_guard(
    axum::extract::State(state): axum::extract::State<AppState>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let path = req.uri().path();
    let gated = path.starts_with("/admin/")
        && path != "/admin/account/password"
        && !path.starts_with("/admin/login")
        && !path.starts_with("/admin/logout")
        && !path.starts_with("/admin/setup");
    if gated {
        if let Some(pool) = state.db.as_ref() {
            // Cookies are populated by the outer CookieManagerLayer, so the
            // extension is always present here; read the opaque session id
            // straight from it to avoid re-running the full extractor.
            let session_id = req
                .extensions()
                .get::<tower_cookies::Cookies>()
                .and_then(|c| c.get(auth::COOKIE_NAME).map(|c| c.value().to_string()));
            if let Some(id) = session_id {
                // Fast path: a recent answer for this exact session id.
                // `must_change` can't change under a live id (see above), so
                // a cache hit is authoritative until the TTL evicts it.
                let cached = MUST_CHANGE_CACHE.get(&id).and_then(|e| {
                    (e.0.elapsed() < MUST_CHANGE_TTL).then_some(e.1)
                });
                let must = match cached {
                    Some(v) => v,
                    None => {
                        // Resolve the session, then the account, once.
                        let actor = state
                            .admin_sessions
                            .validate(&id)
                            .await
                            .and_then(|info| info.actor);
                        // `(answer, cacheable)`. A DB **error** is NOT
                        // cacheable: caching its `false` would let a real
                        // must-change user skip the prompt for the whole TTL
                        // on one transient blip (#903 follow-up). We still
                        // let the request through this once (matching the
                        // pre-cache fail-open), but re-check next request so
                        // the user is caught as soon as the DB recovers.
                        let (v, cacheable) = match actor {
                            Some(actor) => match db::users::fetch(pool, &actor).await {
                                Ok(u) => (u.map(|u| u.must_change_password).unwrap_or(false), true),
                                Err(e) => {
                                    tracing::warn!(error = ?e, "must-change lookup failed; not caching");
                                    (false, false)
                                }
                            },
                            // No actor (break-glass token / unknown id):
                            // nothing to rotate — a definitive `false`.
                            None => (false, true),
                        };
                        if cacheable {
                            if MUST_CHANGE_CACHE.len() >= MUST_CHANGE_CACHE_CAP {
                                MUST_CHANGE_CACHE.retain(|_, e| e.0.elapsed() < MUST_CHANGE_TTL);
                            }
                            MUST_CHANGE_CACHE.insert(id.clone(), (std::time::Instant::now(), v));
                        }
                        v
                    }
                };
                if must {
                    return axum::response::Redirect::to("/admin/account/password")
                        .into_response();
                }
            }
        }
    }
    next.run(req).await
}

/// Baseline security response headers for Ruscker's own pages.
///
/// - `X-Content-Type-Options: nosniff` — stop MIME sniffing
///   (defends against polyglot uploads being interpreted as
///   active content).
/// - `X-Frame-Options: DENY` — the portal/admin is not meant to
///   be embedded; blocks clickjacking.
/// - `Referrer-Policy: same-origin` — don't leak admin URLs to
///   third parties.
/// - `Content-Security-Policy` — restricts resource origins to
///   self. `'unsafe-inline'` is currently required because the
///   landing + dashboard use inline `<script>`/`<style>`; a
///   nonce-based CSP that drops `unsafe-inline` is a tracked
///   follow-up. Even with it, this still blocks loading scripts
///   / frames / objects from other origins.
async fn security_headers(
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    use axum::http::header::HeaderName;
    use axum::http::HeaderValue;
    let mut resp = next.run(req).await;
    let h = resp.headers_mut();
    // Provide defaults only — never clobber a header the handler
    // already set. The `/assets/img/*` route sets a STRICTER CSP
    // (`default-src 'none'; … sandbox`) for operator-uploaded
    // SVGs; an unconditional `insert` here would overwrite it
    // with the looser page policy. `entry().or_insert` leaves any
    // handler-set value intact.
    let defaults: &[(&str, &str)] = &[
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "DENY"),
        ("referrer-policy", "same-origin"),
    ];
    for (name, value) in defaults {
        if let Ok(hn) = HeaderName::from_bytes(name.as_bytes()) {
            h.entry(hn).or_insert(HeaderValue::from_static(value));
        }
    }
    // CSP as a default too — but the landing route may set its own
    // (widened for an operator's analytics origins), so `or_insert`
    // must leave a handler-set value intact.
    if let Ok(v) = HeaderValue::from_str(&content_security_policy("")) {
        h.entry(axum::http::header::CONTENT_SECURITY_POLICY)
            .or_insert(v);
    }
    resp
}

/// CSRF defense for the chrome's state-changing requests (#259).
///
/// Belt-and-suspenders with the `SameSite=Strict` session cookie:
/// rejects POST/PUT/PATCH/DELETE that aren't same-origin. Uses **Fetch
/// Metadata** when the browser sends it (`Sec-Fetch-Site`: only
/// `same-origin` / `none` may mutate), falling back to an **Origin vs
/// Host** check for clients that don't. Requests with neither header
/// (curl, the break-glass token POST) pass — they aren't browser CSRF.
///
/// IMPORTANT — this does NOT isolate *untrusted apps* hosted on the same
/// origin: a script in an app at `/app/{spec}` is genuinely same-origin
/// with `/admin`, so it passes this check. Hosting third-party apps
/// requires a **separate origin/hostname for the admin** (see
/// `docs/SECURITY.md`).
async fn csrf_guard(
    req: axum::extract::Request,
    next: axum::middleware::Next,
    // Whether `X-Forwarded-Host` may be trusted in the Origin/Host
    // fallback. Only true when the operator opted into forwarded headers
    // (#328) — otherwise a client could spoof it to pass the check.
    trust_forwarded: bool,
) -> axum::response::Response {
    use axum::http::Method;
    use axum::response::IntoResponse;
    let state_changing = matches!(
        *req.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    if state_changing && request_is_cross_origin(req.headers(), trust_forwarded) {
        tracing::warn!(
            method = %req.method(), uri = %req.uri(),
            "CSRF guard refused a cross-origin state-changing request"
        );
        return (
            axum::http::StatusCode::FORBIDDEN,
            "cross-origin request refused",
        )
            .into_response();
    }
    next.run(req).await
}

/// `true` when a state-changing request looks cross-origin. See
/// [`csrf_guard`] for the policy. `trust_forwarded` gates whether a
/// client-supplied `X-Forwarded-Host` may stand in for the real `Host`
/// in the Origin/Host fallback (#328).
fn request_is_cross_origin(h: &axum::http::HeaderMap, trust_forwarded: bool) -> bool {
    use axum::http::header;
    // Modern browsers: trust Fetch Metadata.
    if let Some(site) = h.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        return !matches!(site, "same-origin" | "none");
    }
    // Fallback: reject only when an Origin is present and disagrees with
    // the request host. No Origin ⇒ not a browser form/fetch ⇒ allow.
    let Some(origin) = h.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
        return false;
    };
    // Prefer the real `Host`; only consult `X-Forwarded-Host` when the
    // operator trusts forwarded headers — otherwise a non-browser client
    // could spoof it to match its own Origin and pass the check.
    let host = trust_forwarded
        .then(|| h.get("x-forwarded-host"))
        .flatten()
        .or_else(|| h.get(header::HOST))
        .and_then(|v| v.to_str().ok());
    match host {
        // Strip the scheme from the Origin and compare host[:port].
        Some(host) => origin.split("://").nth(1).unwrap_or(origin) != host,
        None => false,
    }
}

/// Build the Content-Security-Policy for Ruscker's own pages.
///
/// `extra_origins` (space-separated, may be empty) is appended to the
/// directives that fetch third-party resources (`script-src`,
/// `img-src`, `connect-src`). The landing uses this to allow an
/// operator-configured analytics provider without loosening the
/// policy for every other page. `'unsafe-inline'` on script/style is
/// still required by the inline landing/dashboard scripts, and
/// `'unsafe-eval'` by Alpine.js (its default evaluator builds
/// functions via `new Function`, which a CSP without `'unsafe-eval'`
/// blocks with an `EvalError` — breaking every Alpine directive:
/// the card filters, help popovers, cover editor, spec form, etc.).
/// The strict path (Alpine's CSP build + a nonce, dropping both
/// `unsafe-*`) is a tracked follow-up.
pub(crate) fn content_security_policy(extra_origins: &str) -> String {
    // Sanitize operator-supplied origins before they reach the header:
    // a stray `*`, `'unsafe-eval'`, or a `;`-smuggled directive would
    // otherwise neuter the whole policy for every visitor.
    let e = sanitize_csp_origins(extra_origins);
    let extra = if e.is_empty() {
        String::new()
    } else {
        format!(" {e}")
    };
    format!(
        "default-src 'self'; \
         script-src 'self' 'unsafe-inline' 'unsafe-eval'{extra}; \
         style-src 'self' 'unsafe-inline'; \
         img-src 'self' data:{extra}; \
         font-src 'self'; \
         connect-src 'self'{extra}; \
         frame-ancestors 'none'; \
         base-uri 'self'; \
         form-action 'self'"
    )
}

/// Keep only space-separated tokens that are safe CSP *host/scheme
/// sources*, dropping anything that could subvert the policy.
fn sanitize_csp_origins(raw: &str) -> String {
    raw.split_whitespace()
        .filter(|t| is_safe_csp_source(t))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A token is accepted only if it's a plain host source
/// (`[scheme://][*.]host[:port][/path]`, must contain a `.` or `:`) or a
/// scheme-only source (`https:`, `data:`). Rejected: the bare wildcard
/// `*`, any `'...'` keyword (`'unsafe-inline'`/`'unsafe-eval'`/…), and
/// anything carrying `;`/`,`/quotes/whitespace — i.e. directive
/// smuggling or policy-loosening keywords.
fn is_safe_csp_source(t: &str) -> bool {
    if t.is_empty()
        || t == "*"
        || t.contains(|c: char| c == ';' || c == ',' || c == '\'' || c.is_whitespace())
    {
        return false;
    }
    if let Some(scheme) = t.strip_suffix(':') {
        return !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'));
    }
    let body = t
        .strip_prefix("https://")
        .or_else(|| t.strip_prefix("http://"))
        .unwrap_or(t);
    let body = body.strip_prefix("*.").unwrap_or(body);
    let valid = !body.is_empty()
        && body
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '/' | '_'))
        && body
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric());
    // Must look like a host (has a dot or a port) — rejects bare words.
    valid && (body.contains('.') || body.contains(':'))
}

/// Set each reconciled replica's `sessions_max` from its spec's
/// `effective_seats`. `specs` must be the EFFECTIVE catalog (DB ∪ YAML),
/// not just `proxy.specs`, so a spec created in the admin (DB-only) is
/// found — otherwise its reconciled replica keeps `list()`'s
/// `sessions_max = 0` and reads as permanently full after a restart
/// (#907). A replica whose spec is no longer in the catalog is left as-is.
fn apply_seat_caps(replicas: &mut [ruscker_core::Replica], specs: &[ruscker_config::Spec]) {
    let caps: std::collections::HashMap<&str, u32> = specs
        .iter()
        .map(|s| (s.id.as_str(), s.effective_seats()))
        .collect();
    for r in replicas {
        if let Some(max) = caps.get(r.spec_id.as_str()) {
            r.sessions_max = *max;
        }
    }
}

#[cfg(test)]
mod csrf_tests {
    use super::request_is_cross_origin;
    use axum::http::{header, HeaderMap, HeaderValue};

    fn hm(pairs: &[(&str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(
                header::HeaderName::from_bytes(k.as_bytes()).unwrap(),
                HeaderValue::from_str(v).unwrap(),
            );
        }
        h
    }

    #[test]
    fn fetch_metadata_decides_when_present() {
        // Sec-Fetch-Site short-circuits before the host check, so the
        // trust flag is irrelevant here.
        assert!(!request_is_cross_origin(&hm(&[("sec-fetch-site", "same-origin")]), false));
        assert!(!request_is_cross_origin(&hm(&[("sec-fetch-site", "none")]), false));
        assert!(request_is_cross_origin(&hm(&[("sec-fetch-site", "cross-site")]), false));
        assert!(request_is_cross_origin(&hm(&[("sec-fetch-site", "same-site")]), false));
    }

    #[test]
    fn no_headers_is_allowed() {
        // curl / the break-glass token POST send neither header.
        assert!(!request_is_cross_origin(&HeaderMap::new(), false));
    }

    #[test]
    fn origin_fallback_compares_host() {
        // Same host (scheme stripped) → allowed.
        assert!(!request_is_cross_origin(
            &hm(&[
                ("origin", "https://portal.example.org"),
                ("host", "portal.example.org"),
            ]),
            false
        ));
        // Behind a trusted proxy, compare against X-Forwarded-Host.
        assert!(!request_is_cross_origin(
            &hm(&[
                ("origin", "https://portal.example.org"),
                ("x-forwarded-host", "portal.example.org"),
                ("host", "127.0.0.1:8080"),
            ]),
            true
        ));
        // Different host → cross-origin.
        assert!(request_is_cross_origin(
            &hm(&[
                ("origin", "https://evil.example.com"),
                ("host", "portal.example.org"),
            ]),
            false
        ));
    }

    #[test]
    fn xforwarded_host_ignored_when_untrusted() {
        // #328: with forwarded headers NOT trusted, a spoofed
        // X-Forwarded-Host matching the attacker's Origin must NOT pass
        // — the check falls back to the real Host and sees a mismatch.
        assert!(request_is_cross_origin(
            &hm(&[
                ("origin", "https://evil.example.com"),
                ("x-forwarded-host", "evil.example.com"),
                ("host", "portal.example.org"),
            ]),
            false
        ));
        // The same request IS treated same-origin when forwarded headers
        // are trusted (operator put Ruscker behind a real proxy).
        assert!(!request_is_cross_origin(
            &hm(&[
                ("origin", "https://evil.example.com"),
                ("x-forwarded-host", "evil.example.com"),
                ("host", "portal.example.org"),
            ]),
            true
        ));
    }
}

#[cfg(test)]
mod csp_tests {
    use super::content_security_policy;

    #[test]
    fn base_policy_is_self_only() {
        let csp = content_security_policy("");
        assert!(csp.contains("script-src 'self' 'unsafe-inline' 'unsafe-eval';"));
        assert!(csp.contains("connect-src 'self';"));
        assert!(!csp.contains("plausible"));
    }

    #[test]
    fn extra_origins_widen_script_img_connect() {
        let csp = content_security_policy("https://plausible.io");
        assert!(csp.contains("script-src 'self' 'unsafe-inline' 'unsafe-eval' https://plausible.io;"));
        assert!(csp.contains("img-src 'self' data: https://plausible.io;"));
        assert!(csp.contains("connect-src 'self' https://plausible.io;"));
        // Directives that must NOT be widened.
        assert!(csp.contains("style-src 'self' 'unsafe-inline';"));
        assert!(csp.contains("frame-ancestors 'none';"));
    }

    // #82: unsafe origin tokens must be stripped before reaching the
    // header so a careless/hostile entry can't neuter the landing CSP.
    #[test]
    fn dangerous_origins_are_dropped() {
        let csp = content_security_policy(
            "* 'unsafe-eval' 'unsafe-inline' evil.com;script-src https://ok.example.com",
        );
        // Only the one clean host source survives.
        assert!(csp.contains("https://ok.example.com"));
        assert!(!csp.contains('*'));
        // The base `script-src` carries exactly one `'unsafe-eval'`
        // (for Alpine). The operator's injected copy must be stripped,
        // so it appears once — not duplicated into img-src/connect-src.
        assert_eq!(csp.matches("unsafe-eval").count(), 1);
        // The `evil.com;script-src` token (directive smuggling) is gone.
        assert!(!csp.contains("evil.com"));
        // The base policy is intact.
        assert!(csp.contains("default-src 'self';"));
    }

    #[test]
    fn safe_sources_are_kept() {
        let csp = content_security_policy("https://a.example.com data: *.cdn.example.org");
        assert!(csp.contains("https://a.example.com"));
        assert!(csp.contains("data:"));
        assert!(csp.contains("*.cdn.example.org"));
    }
}

#[cfg(test)]
mod reconcile_tests {
    use super::apply_seat_caps;
    use ruscker_config::Spec;
    use ruscker_core::{Replica, ReplicaId, ReplicaState};

    fn yaml_spec(s: &str) -> Spec {
        std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
        serde_yaml_ng::from_str(s).unwrap()
    }

    // #907: a reconciled replica of a DB-only spec must get its seat cap
    // from the effective catalog, or it reads as permanently full.
    #[test]
    fn seat_caps_cover_db_only_specs() {
        // The backend's list() can't know seat caps → sessions_max = 0.
        let mut replicas = vec![Replica {
            id: ReplicaId::new(),
            spec_id: "db-only".into(),
            container_id: "c1".into(),
            upstream: "127.0.0.1:8000".parse().unwrap(),
            state: ReplicaState::Ready,
            started_at: chrono::Utc::now(),
            sessions_active: 0,
            sessions_max: 0,
            host: None,
        }];
        // The effective catalog (DB ∪ YAML) carries the DB-only spec.
        let specs = vec![yaml_spec(
            "id: db-only\ncontainer-image: nginx\nseats-per-container: 3",
        )];
        apply_seat_caps(&mut replicas, &specs);
        assert_eq!(replicas[0].sessions_max, 3, "seat cap applied");
        assert!(replicas[0].available_seats() > 0, "no longer reads as full");
    }

    // A replica whose spec vanished from the catalog is left untouched.
    #[test]
    fn seat_caps_leave_unknown_spec_alone() {
        let mut replicas = vec![Replica {
            id: ReplicaId::new(),
            spec_id: "gone".into(),
            container_id: "c1".into(),
            upstream: "127.0.0.1:8000".parse().unwrap(),
            state: ReplicaState::Ready,
            started_at: chrono::Utc::now(),
            sessions_active: 1,
            sessions_max: 7,
            host: None,
        }];
        apply_seat_caps(&mut replicas, &[]);
        assert_eq!(replicas[0].sessions_max, 7, "untouched when spec is absent");
    }
}
