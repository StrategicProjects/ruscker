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

    Ok(pool)
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

pub mod specs;
