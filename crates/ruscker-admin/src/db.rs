//! SQLite persistence — connection pool, migrations, repositories.
//!
//! Phase 2 introduces SQLite as the source of truth for spec
//! configurations, images, credentials, landing customization,
//! and the audit log. YAML stays around as an import/export
//! format only (see `ruscker import` / `ruscker export`).
//!
//! Conventions:
//!
//! - All write paths go through transactions, even single-row
//!   updates, so we can attach an `audit_log` insert atomically.
//! - The application is the single source of "now()"; no DB-level
//!   `DEFAULT CURRENT_TIMESTAMP` triggers. Keeps tests
//!   deterministic and lets imports preserve historical
//!   `created_at` values.
//! - Foreign keys are enabled per-connection via PRAGMA — SQLite
//!   defaults to off for legacy reasons.

use anyhow::{Context, Result};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use std::path::Path;
use std::str::FromStr;

/// Built-in migrations directory. The `sqlx::migrate!` macro
/// embeds every `.sql` file under `migrations/` at compile time —
/// no separate deployment needed.
pub static MIGRATIONS: sqlx::migrate::Migrator = sqlx::migrate!("./migrations");

/// Postgres twin of [`MIGRATIONS`], embedded from `migrations-pg/`.
///
/// Phase 7 (HA / active-active) lets the admin catalog live in shared
/// Postgres instead of per-node SQLite. The two directories are kept
/// in lockstep — same migration numbers, same intent, different
/// dialect — so the schema is identical whichever backend an install
/// runs. SQLite stays the zero-config single-node default; Postgres is
/// opt-in for clustering.
pub static MIGRATIONS_PG: sqlx::migrate::Migrator = sqlx::migrate!("./migrations-pg");

/// A handle to the admin config database.
///
/// SQLite is the zero-config single-node default; Postgres backs the
/// shared catalog in HA mode (Phase 7). The repository functions are
/// ported to dual-dialect **one module at a time** — until a function
/// is ported it runs in SQLite mode only, reached through
/// [`ConfigDb::as_sqlite`]. In Postgres mode an un-ported path gets
/// `None` and the caller surfaces a clear "not yet supported" response
/// rather than silently misbehaving.
#[derive(Debug, Clone)]
pub enum ConfigDb {
    Sqlite(SqlitePool),
    Postgres(sqlx::PgPool),
}

impl ConfigDb {
    /// The SQLite pool when running in SQLite mode, else `None`.
    /// Un-ported repositories go through this; a ported one matches on
    /// the enum directly.
    pub fn as_sqlite(&self) -> Option<&SqlitePool> {
        match self {
            ConfigDb::Sqlite(pool) => Some(pool),
            ConfigDb::Postgres(_) => None,
        }
    }
}

/// Open the SQLite database at `path`, creating the file if
/// missing, and apply any pending migrations. Returns the pool
/// ready for use.
pub async fn open(path: impl AsRef<Path>) -> Result<SqlitePool> {
    let path = path.as_ref();
    let url = format!("sqlite://{}", path.display());
    let opts = SqliteConnectOptions::from_str(&url)
        .with_context(|| format!("invalid SQLite URL `{url}`"))?
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        // BUSY_TIMEOUT prevents `database is locked` errors under
        // concurrent writes (importer + admin write paths).
        .busy_timeout(std::time::Duration::from_secs(5));

    let pool = SqlitePoolOptions::new()
        // SQLite is single-writer; more than a handful of conns
        // just serializes longer. 5 readers is plenty for the
        // admin's expected load.
        .max_connections(5)
        .connect_with(opts)
        .await
        .with_context(|| format!("open SQLite at {}", path.display()))?;

    MIGRATIONS
        .run(&pool)
        .await
        .context("apply migrations")?;

    // First-install showcase cards (idempotent via config_meta).
    let cfg = ConfigDb::Sqlite(pool.clone());
    if let Err(err) = showcase::seed_if_unseeded(&cfg).await {
        tracing::warn!(error = ?err, "showcase seed skipped");
    }
    // First-install built-in logos in the Media library (#433).
    if let Err(err) = builtin_logos::seed_if_unseeded(&cfg).await {
        tracing::warn!(error = ?err, "builtin logos seed skipped");
    }

    Ok(pool)
}

/// Open the shared Postgres admin database at `url` (a `postgres://…`
/// DSN) and apply any pending [`MIGRATIONS_PG`]. Returns the pool ready
/// for use.
///
/// This is the HA counterpart to [`open`]: several Ruscker instances
/// point at the same Postgres so they share one editable spec catalog,
/// landing customization, credential store, user table and audit log.
/// The per-statement query port that lets the existing repositories
/// run against this pool lands in the following Phase 7c slices; this
/// function plus `migrations-pg/` is the foundation it builds on.
pub async fn open_pg(url: &str) -> Result<sqlx::PgPool> {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await
        .context("open Postgres admin database")?;

    MIGRATIONS_PG
        .run(&pool)
        .await
        .context("apply Postgres migrations")?;

    Ok(pool)
}

/// Serializes the `postgres-it` integration tests. They all run against
/// one shared database, so cargo's default parallel test execution would
/// let them clobber each other's rows (e.g. two tests that
/// `DELETE FROM landing_blocks`, or a default-singleton read racing a
/// singleton write). Every gated test takes this lock first, so they run
/// one at a time regardless of `--test-threads`.
#[cfg(all(test, feature = "postgres-it"))]
pub(crate) fn pg_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Open an in-memory database for tests. Migrations applied.
#[cfg(test)]
pub async fn open_memory() -> Result<SqlitePool> {
    let opts = SqliteConnectOptions::from_str("sqlite::memory:")?
        .create_if_missing(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await?;
    MIGRATIONS.run(&pool).await?;
    Ok(pool)
}

pub mod audit;
pub mod builtin_logos;
pub mod credentials;
pub mod export;
pub mod images;
pub mod landing;
pub mod landing_blocks;
pub mod mfa;
pub mod mfa_grants;
pub mod ruscker_images;
pub mod schedules;
pub mod settings;
pub mod showcase;
pub mod spec_access;
pub mod specs;
pub mod user_favorites;
pub mod users;

#[cfg(test)]
mod migration_parity {
    // #758: `migrations-pg/` silently stopped at 0016 while `migrations/`
    // reached 0021 — every Postgres-catalog deployment's landing fetch
    // broke from v0.1.90 until the drift was found, because the only
    // schema-parity test was gated behind `postgres-it` and hadn't run.
    // Filename parity needs no database, so it runs in the default gate;
    // the file *contents* may differ (dialects), the SET may not.
    #[test]
    fn sqlite_and_postgres_migration_sets_match() {
        fn names(dir: &str) -> Vec<String> {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(dir);
            let mut v: Vec<String> = std::fs::read_dir(&root)
                .unwrap_or_else(|e| panic!("read {root:?}: {e}"))
                .map(|e| e.expect("dir entry").file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".sql"))
                .collect();
            v.sort();
            v
        }
        assert_eq!(
            names("migrations"),
            names("migrations-pg"),
            "migrations/ and migrations-pg/ must carry the same migration \
             files — port the missing twin before shipping (#758)"
        );
    }
}

#[cfg(all(test, feature = "postgres-it"))]
mod pg_tests {
    use super::*;
    use sqlx::Row;

    // Proves the Postgres migrations apply cleanly to a real daemon
    // and produce the same table set as the SQLite schema. Gated:
    //   docker run --rm -e POSTGRES_PASSWORD=pg -p 5433:5432 postgres:16-alpine
    //   RUSCKER_TEST_PG_URL=postgres://postgres:pg@127.0.0.1:5433/postgres \
    //     cargo test -p ruscker-admin --features postgres-it -- --nocapture
    #[tokio::test]
    async fn pg_migrations_apply_and_match_sqlite_tables() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");

        // open_pg runs every migration. A second open_pg must be a
        // no-op (the migrator tracks applied rows) — exercise
        // idempotency. Non-destructive throughout so this can share a
        // database with the 7b session-store test without racing it.
        let pool = open_pg(&url).await.unwrap();
        let pool = {
            drop(pool);
            open_pg(&url).await.unwrap()
        };

        // Every admin table the SQLite schema defines must be present
        // (subset check — a sibling test's `proxy_sessions` may also
        // exist; we don't require an exact match).
        let present: std::collections::HashSet<String> = sqlx::query(
            "SELECT table_name FROM information_schema.tables
             WHERE table_schema = 'public' AND table_type = 'BASE TABLE'",
        )
        .fetch_all(&pool)
        .await
        .unwrap()
        .into_iter()
        .map(|r| r.get::<String, _>("table_name"))
        .collect();
        for expected in [
            "audit_log",
            "config_meta",
            "credentials",
            "images",
            "landing_blocks",
            "landing_customization",
            "spec_versions",
            "specs",
            "user_mfa",
            "user_mfa_recovery",
            "users",
        ] {
            assert!(present.contains(expected), "missing table: {expected}");
        }

        // The singleton landing row was seeded by 0001.
        let n: i64 = sqlx::query("SELECT count(*) AS n FROM landing_customization")
            .fetch_one(&pool)
            .await
            .unwrap()
            .get("n");
        assert_eq!(n, 1);

        // The 0003/0004 ALTERs landed (seo + analytics columns).
        let cols: i64 = sqlx::query(
            "SELECT count(*) AS n FROM information_schema.columns
             WHERE table_name = 'landing_customization'
               AND column_name IN ('seo_title', 'analytics_html', 'og_image')",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
        .get("n");
        assert_eq!(cols, 3);
    }
}
