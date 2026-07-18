//! Admin user accounts — per-user password login + roles.
//!
//! Passwords are stored only as argon2id PHC hashes; plaintext never
//! touches the disk. The [`crate::auth::Role`] of each user drives the
//! same route guards as the env-token model it replaces. The
//! `RUSCKER_ADMIN_TOKEN` env var stays a break-glass bootstrap (see
//! [`crate::auth::AdminAuth`]) — it always grants an Admin session and
//! seeds the first account.
//!
//! All write paths attach an `audit_log` row in the same transaction
//! (mirroring the rest of `db::*`), tagged with the acting username.

use anyhow::{Context, Result};
use argon2::password_hash::rand_core::OsRng;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use chrono::{DateTime, Utc};

use crate::auth::Role;
use crate::db::ConfigDb;

/// A user row without the password hash — safe to render.
#[derive(Debug, Clone)]
pub struct UserRow {
    pub id: String,
    pub username: String,
    pub role: Role,
    pub must_change_password: bool,
    /// Group memberships, matched against a spec's `access-groups` for
    /// app visibility (#155). Empty = belongs to no group.
    pub groups: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub created_by: Option<String>,
    /// Optional profile fields (#856): department, e-mail, mobile.
    /// `None`/blank when unset.
    pub setor: Option<String>,
    pub email: Option<String>,
    pub celular: Option<String>,
}

/// Normalize a username for storage/lookup: trimmed + lowercased.
/// Usernames are case-insensitive and unique.
pub fn normalize_username(raw: &str) -> String {
    raw.trim().to_lowercase()
}

/// A username is safe iff it's non-empty and made only of identifier-ish
/// chars (letters, digits, `_ . @ -` — `@` so e-mail logins work). It
/// lands un-encoded in the per-user admin action URLs
/// (`/admin/users/{username}/...`), so anything else (`/`, `?`, `#`,
/// space, …) would make the user impossible to edit/delete from the UI
/// (#429, same class as the credential-name fix #423).
pub fn is_valid_username(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '@' | '-'))
}

/// Parse the stored comma-separated `groups` column into a clean list:
/// trimmed, empties dropped, duplicates removed (first wins, order
/// preserved). Group names are compared case-sensitively against a
/// spec's `access-groups`, so we keep the operator's casing.
pub fn parse_groups(stored: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tok in stored.split(',') {
        let t = tok.trim();
        if !t.is_empty() && !out.iter().any(|g| g == t) {
            out.push(t.to_string());
        }
    }
    out
}

/// Canonicalize a group list for storage: same cleanup as
/// [`parse_groups`], joined with `,` (no spaces) so the round-trip is
/// stable.
pub fn join_groups(groups: &[String]) -> String {
    parse_groups(&groups.join(",")).join(",")
}

/// Hash a plaintext password with argon2id and a fresh random salt,
/// returning the PHC string to store.
pub fn hash_password(plain: &str) -> Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(plain.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| anyhow::anyhow!("hash password: {e}"))
}

/// Verify a plaintext password against a stored PHC hash. Returns
/// `false` on any parse/verify failure — argon2's verify is the
/// constant-time comparison.
pub fn verify_password(plain: &str, phc: &str) -> bool {
    match PasswordHash::new(phc) {
        Ok(parsed) => Argon2::default()
            .verify_password(plain.as_bytes(), &parsed)
            .is_ok(),
        Err(_) => false,
    }
}

/// Number of users with the Admin role. Used to block removing or
/// demoting the last admin (lockout protection).
pub async fn count_admins(db: &ConfigDb) -> Result<i64> {
    let sql = "SELECT COUNT(*) FROM users WHERE role = 'admin'";
    let (n,): (i64,) = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_one(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_one(pool).await,
    }
    .context("count admins")?;
    Ok(n)
}

/// Whether any admin account exists yet. Drives the bootstrap flow:
/// with no admin user, login falls back to the break-glass token and
/// the setup wizard.
pub async fn any_admin_exists(db: &ConfigDb) -> Result<bool> {
    Ok(count_admins(db).await? > 0)
}

/// Whether any user at all exists (used by the login page to decide
/// between the bootstrap and the normal form).
pub async fn any_user_exists(db: &ConfigDb) -> Result<bool> {
    let sql = "SELECT COUNT(*) FROM users";
    let (n,): (i64,) = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_one(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_one(pool).await,
    }
    .context("count users")?;
    Ok(n > 0)
}

/// Map a `(username, role, must_change, created_at, created_by)` tuple
/// to a [`UserRow`]. An unknown role string falls back to Viewer (the
/// least-privileged) so a hand-tampered DB can't escalate.
#[allow(clippy::too_many_arguments)]
fn row_from(
    id: String,
    username: String,
    role: String,
    must_change: bool,
    groups: String,
    created_at: DateTime<Utc>,
    created_by: Option<String>,
    setor: Option<String>,
    email: Option<String>,
    celular: Option<String>,
) -> UserRow {
    // Treat a stored empty string the same as NULL (the form may submit
    // blanks); keep only non-empty trimmed values.
    let clean = |s: Option<String>| s.map(|v| v.trim().to_string()).filter(|v| !v.is_empty());
    UserRow {
        id,
        username,
        role: Role::parse(&role).unwrap_or(Role::Viewer),
        // `must_change_password` reads as `bool` on both backends: sqlx
        // decodes SQLite's INTEGER 0/1 and Postgres' native BOOLEAN.
        must_change_password: must_change,
        groups: parse_groups(&groups),
        created_at,
        created_by,
        setor: clean(setor),
        email: clean(email),
        celular: clean(celular),
    }
}

/// List all users (no hashes), most-recent first.
pub async fn list_all(db: &ConfigDb) -> Result<Vec<UserRow>> {
    type Row = (
        String,
        String,
        String,
        bool,
        String,
        DateTime<Utc>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let sql = "SELECT id, username, role, must_change_password, groups, created_at, created_by, setor, email, celular
           FROM users
          ORDER BY created_at DESC, username ASC";
    let rows: Vec<Row> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    }
    .context("list users")?;
    Ok(rows
        .into_iter()
        .map(|(id, u, r, m, g, c, by, se, em, ce)| row_from(id, u, r, m, g, c, by, se, em, ce))
        .collect())
}

/// Escape `%`, `_` and `\` in a user-typed search term so each matches
/// literally under SQL `LIKE`, then wrap it in `%…%` for a substring
/// match. Both dialects use `\` as the escape character (Postgres by
/// default; SQLite via an explicit `ESCAPE '\'` clause).
fn like_pattern(term: &str) -> String {
    let mut pat = String::with_capacity(term.len() + 2);
    pat.push('%');
    for c in term.chars() {
        if matches!(c, '%' | '_' | '\\') {
            pat.push('\\');
        }
        pat.push(c);
    }
    pat.push('%');
    pat
}

/// The two case variants of a search pattern for the SQLite arm.
///
/// SQLite's `LIKE` only case-folds ASCII, so `JOÃO` would not match a
/// stored `João` (the `Ã`/`ã` pair doesn't fold) while Postgres'
/// `ILIKE` folds full Unicode. Rust's `to_lowercase`/`to_uppercase`
/// ARE Unicode-aware, so matching against both variants covers an
/// accented query in either case against stored text in the usual
/// forms (lowercase, Capitalized, ALL CAPS). Text with mixed-case
/// accents mid-word (`GesTÃo`) can still evade — acceptable for a
/// search box, and impossible to fix in stock SQLite without ICU.
fn like_patterns_sqlite(term: &str) -> (String, String) {
    (
        like_pattern(&term.to_lowercase()),
        like_pattern(&term.to_uppercase()),
    )
}

/// Count the users matching a search term — the total behind
/// [`list_page`]'s pagination (#999). A blank `search` counts every
/// account; otherwise it's the same case-insensitive substring filter
/// over username, groups and the profile fields.
pub async fn count_filtered(db: &ConfigDb, search: &str) -> Result<i64> {
    let search = search.trim();
    let (n,): (i64,) = match db {
        ConfigDb::Sqlite(pool) => {
            if search.is_empty() {
                sqlx::query_as("SELECT COUNT(*) FROM users")
                    .fetch_one(pool)
                    .await
            } else {
                // Two case variants per column — see [`like_patterns_sqlite`].
                let (lo, up) = like_patterns_sqlite(search);
                sqlx::query_as(
                    "SELECT COUNT(*) FROM users
                      WHERE username LIKE ?1 ESCAPE '\\' OR username LIKE ?2 ESCAPE '\\'
                         OR groups LIKE ?1 ESCAPE '\\' OR groups LIKE ?2 ESCAPE '\\'
                         OR COALESCE(setor, '') LIKE ?1 ESCAPE '\\' OR COALESCE(setor, '') LIKE ?2 ESCAPE '\\'
                         OR COALESCE(email, '') LIKE ?1 ESCAPE '\\' OR COALESCE(email, '') LIKE ?2 ESCAPE '\\'
                         OR COALESCE(celular, '') LIKE ?1 ESCAPE '\\' OR COALESCE(celular, '') LIKE ?2 ESCAPE '\\'",
                )
                .bind(lo)
                .bind(up)
                .fetch_one(pool)
                .await
            }
        }
        ConfigDb::Postgres(pool) => {
            if search.is_empty() {
                sqlx::query_as("SELECT COUNT(*) FROM users")
                    .fetch_one(pool)
                    .await
            } else {
                // ILIKE for case-insensitivity; `\` is pg's default escape.
                sqlx::query_as(
                    "SELECT COUNT(*) FROM users
                      WHERE username ILIKE $1
                         OR groups ILIKE $1
                         OR COALESCE(setor, '') ILIKE $1
                         OR COALESCE(email, '') ILIKE $1
                         OR COALESCE(celular, '') ILIKE $1",
                )
                .bind(like_pattern(search))
                .fetch_one(pool)
                .await
            }
        }
    }
    .context("count users (filtered)")?;
    Ok(n)
}

/// One page of users (no hashes), most-recent first — the paginated
/// sibling of [`list_all`] (#999), so the users admin page stays fast
/// with thousands of accounts. `search` filters by a case-insensitive
/// substring over username, groups, setor, email and celular; blank
/// means no filter. The caller derives `limit`/`offset` from its page
/// number.
pub async fn list_page(
    db: &ConfigDb,
    search: &str,
    limit: i64,
    offset: i64,
) -> Result<Vec<UserRow>> {
    type Row = (
        String,
        String,
        String,
        bool,
        String,
        DateTime<Utc>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let search = search.trim();
    let rows: Vec<Row> = match db {
        ConfigDb::Sqlite(pool) => {
            if search.is_empty() {
                sqlx::query_as(
                    "SELECT id, username, role, must_change_password, groups, created_at, created_by, setor, email, celular
                       FROM users
                      ORDER BY created_at DESC, username ASC
                      LIMIT ?1 OFFSET ?2",
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await
            } else {
                // Two case variants per column — see [`like_patterns_sqlite`].
                let (lo, up) = like_patterns_sqlite(search);
                sqlx::query_as(
                    "SELECT id, username, role, must_change_password, groups, created_at, created_by, setor, email, celular
                       FROM users
                      WHERE username LIKE ?1 ESCAPE '\\' OR username LIKE ?2 ESCAPE '\\'
                         OR groups LIKE ?1 ESCAPE '\\' OR groups LIKE ?2 ESCAPE '\\'
                         OR COALESCE(setor, '') LIKE ?1 ESCAPE '\\' OR COALESCE(setor, '') LIKE ?2 ESCAPE '\\'
                         OR COALESCE(email, '') LIKE ?1 ESCAPE '\\' OR COALESCE(email, '') LIKE ?2 ESCAPE '\\'
                         OR COALESCE(celular, '') LIKE ?1 ESCAPE '\\' OR COALESCE(celular, '') LIKE ?2 ESCAPE '\\'
                      ORDER BY created_at DESC, username ASC
                      LIMIT ?3 OFFSET ?4",
                )
                .bind(lo)
                .bind(up)
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await
            }
        }
        ConfigDb::Postgres(pool) => {
            if search.is_empty() {
                sqlx::query_as(
                    "SELECT id, username, role, must_change_password, groups, created_at, created_by, setor, email, celular
                       FROM users
                      ORDER BY created_at DESC, username ASC
                      LIMIT $1 OFFSET $2",
                )
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await
            } else {
                sqlx::query_as(
                    "SELECT id, username, role, must_change_password, groups, created_at, created_by, setor, email, celular
                       FROM users
                      WHERE username ILIKE $1
                         OR groups ILIKE $1
                         OR COALESCE(setor, '') ILIKE $1
                         OR COALESCE(email, '') ILIKE $1
                         OR COALESCE(celular, '') ILIKE $1
                      ORDER BY created_at DESC, username ASC
                      LIMIT $2 OFFSET $3",
                )
                .bind(like_pattern(search))
                .bind(limit)
                .bind(offset)
                .fetch_all(pool)
                .await
            }
        }
    }
    .context("list users (page)")?;
    Ok(rows
        .into_iter()
        .map(|(id, u, r, m, g, c, by, se, em, ce)| row_from(id, u, r, m, g, c, by, se, em, ce))
        .collect())
}

/// Fetch one user by (normalized) username, without the hash.
pub async fn fetch(db: &ConfigDb, username: &str) -> Result<Option<UserRow>> {
    type Row = (
        String,
        String,
        String,
        bool,
        String,
        DateTime<Utc>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
    );
    let username = normalize_username(username);
    let row: Option<Row> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(
            "SELECT id, username, role, must_change_password, groups, created_at, created_by, setor, email, celular
               FROM users WHERE username = ?",
        )
        .bind(&username)
        .fetch_optional(pool)
        .await,
        ConfigDb::Postgres(pool) => sqlx::query_as(
            "SELECT id, username, role, must_change_password, groups, created_at, created_by, setor, email, celular
               FROM users WHERE username = $1",
        )
        .bind(&username)
        .fetch_optional(pool)
        .await,
    }
    .with_context(|| format!("fetch user {username}"))?;
    Ok(row.map(|(id, u, r, m, g, c, by, se, em, ce)| row_from(id, u, r, m, g, c, by, se, em, ce)))
}

/// Verify a login. Returns the [`UserRow`] on a correct password,
/// `None` on unknown username or wrong password (indistinguishable to
/// the caller). Always runs an argon2 verify — even for an unknown
/// user, against a dummy hash — so response time doesn't reveal
/// whether the username exists.
pub async fn verify_login(
    db: &ConfigDb,
    username: &str,
    password: &str,
) -> Result<Option<UserRow>> {
    type Row = (
        String,
        String,
        String,
        bool,
        String,
        DateTime<Utc>,
        Option<String>,
        Option<String>,
        Option<String>,
        Option<String>,
        String,
    );
    let username = normalize_username(username);
    let row: Option<Row> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(
            "SELECT id, username, role, must_change_password, groups, created_at, created_by, setor, email, celular, password_hash
               FROM users WHERE username = ?",
        )
        .bind(&username)
        .fetch_optional(pool)
        .await,
        ConfigDb::Postgres(pool) => sqlx::query_as(
            "SELECT id, username, role, must_change_password, groups, created_at, created_by, setor, email, celular, password_hash
               FROM users WHERE username = $1",
        )
        .bind(&username)
        .fetch_optional(pool)
        .await,
    }
    .with_context(|| format!("login lookup {username}"))?;

    // argon2id is ~tens of ms of pure CPU — run it on the blocking
    // pool so it can't stall a tokio worker (and every proxied request
    // sharing it) during a login burst (#744). The unknown-username
    // branch spends the same verify time on a decoy hash so response
    // timing doesn't reveal whether the account exists.
    let pw = password.to_string();
    let (hash_to_check, found) = match &row {
        Some((.., hash)) => (hash.clone(), true),
        None => (dummy_hash().to_string(), false),
    };
    let verified = tokio::task::spawn_blocking(move || verify_password(&pw, &hash_to_check))
        .await
        .context("password verify task")?;
    match row {
        Some((id, u, r, m, g, c, by, se, em, ce, _)) if found && verified => {
            Ok(Some(row_from(id, u, r, m, g, c, by, se, em, ce)))
        }
        _ => Ok(None),
    }
}

/// A real argon2id hash of a throwaway password, computed once, used
/// only to keep unknown-username logins as slow as real ones.
fn dummy_hash() -> &'static str {
    use std::sync::OnceLock;
    static H: OnceLock<String> = OnceLock::new();
    H.get_or_init(|| hash_password("ruscker-timing-decoy").expect("hash dummy"))
}

/// Create a new user. `must_change` marks the optional first-login
/// password-change prompt. Errors if the username already exists.
pub async fn create(
    db: &ConfigDb,
    username: &str,
    password: &str,
    role: Role,
    must_change: bool,
    groups: &[String],
    actor: Option<&str>,
) -> Result<()> {
    let username = normalize_username(username);
    // argon2id hashing is CPU-bound — blocking pool, not a tokio
    // worker (#744).
    let hash = {
        let pw = password.to_string();
        tokio::task::spawn_blocking(move || hash_password(&pw))
            .await
            .context("password hash task")??
    };
    let groups = join_groups(groups);
    let now = Utc::now();
    let id = uuid::Uuid::new_v4().to_string();

    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin user tx")?;
            sqlx::query(
                "INSERT INTO users
                   (id, username, password_hash, role, must_change_password,
                    groups, created_at, updated_at, created_by)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(&id)
            .bind(&username)
            .bind(&hash)
            .bind(role.as_str())
            .bind(must_change as i64)
            .bind(&groups)
            .bind(now)
            .bind(now)
            .bind(actor)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("insert user {username}"))?;
            audit(&mut tx, actor, "user.create", &username, &role, now).await?;
            tx.commit().await.context("commit user create")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin user tx")?;
            sqlx::query(
                "INSERT INTO users
                   (id, username, password_hash, role, must_change_password,
                    groups, created_at, updated_at, created_by)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            )
            .bind(&id)
            .bind(&username)
            .bind(&hash)
            .bind(role.as_str())
            .bind(must_change)
            .bind(&groups)
            .bind(now)
            .bind(now)
            .bind(actor)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("insert user {username}"))?;
            audit_pg(&mut tx, actor, "user.create", &username, &role, now).await?;
            tx.commit().await.context("commit user create")?;
        }
    }
    Ok(())
}

/// Replace a user's password and set/clear the must-change prompt.
pub async fn set_password(
    db: &ConfigDb,
    username: &str,
    new_password: &str,
    must_change: bool,
    actor: Option<&str>,
) -> Result<()> {
    let username = normalize_username(username);
    let hash = {
        let pw = new_password.to_string();
        tokio::task::spawn_blocking(move || hash_password(&pw))
            .await
            .context("password hash task")??
    };
    let now = Utc::now();
    let target = format!("user:{username}");

    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin password tx")?;
            let res = sqlx::query(
                "UPDATE users
                    SET password_hash = ?, must_change_password = ?, updated_at = ?
                  WHERE username = ?",
            )
            .bind(&hash)
            .bind(must_change as i64)
            .bind(now)
            .bind(&username)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("set password for {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("user {username} not found");
            }
            crate::db::mfa_grants::delete_all_sqlite(&mut tx, &username).await?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'user.password', ?, NULL, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit password change")?;
            tx.commit().await.context("commit password change")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin password tx")?;
            let res = sqlx::query(
                "UPDATE users
                    SET password_hash = $1, must_change_password = $2, updated_at = $3
                  WHERE username = $4",
            )
            .bind(&hash)
            .bind(must_change)
            .bind(now)
            .bind(&username)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("set password for {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("user {username} not found");
            }
            crate::db::mfa_grants::delete_all_postgres(&mut tx, &username).await?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'user.password', $2, NULL, $3)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit password change")?;
            tx.commit().await.context("commit password change")?;
        }
    }
    Ok(())
}

/// Change a user's role.
/// Postgres advisory-lock key serializing admin-count mutations
/// (`set_role`/`delete`) so the last-admin guard can't be raced (#872).
/// SQLite needs no equivalent — its writes already serialize.
const ADMIN_MUTATION_LOCK: i64 = 0x7275_7363_6b72_0001;

/// Editable account fields saved together by the consolidated user form.
/// Password resets remain a separate, deliberately explicit operation.
pub struct AccountUpdate<'a> {
    pub role: Role,
    pub groups: &'a [String],
    pub setor: Option<&'a str>,
    pub email: Option<&'a str>,
    pub celular: Option<&'a str>,
}

/// Update role, groups and profile in one transaction. The conditional
/// role predicate is the same last-admin backstop used by [`set_role`],
/// so a concurrent demotion cannot leave the portal without an admin.
pub async fn update_account(
    db: &ConfigDb,
    username: &str,
    update: AccountUpdate<'_>,
    actor: Option<&str>,
) -> Result<()> {
    fn norm(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|v| !v.is_empty())
    }

    let username = normalize_username(username);
    let groups = join_groups(update.groups);
    let setor = norm(update.setor);
    let email = norm(update.email);
    let celular = norm(update.celular);
    let now = Utc::now();
    let target = format!("user:{username}");
    let diff = serde_json::json!({
        "role": update.role.as_str(),
        "groups": groups,
        "setor": setor,
        "email": email,
        "celular": celular,
    })
    .to_string();

    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin account update")?;
            let res = sqlx::query(
                "UPDATE users
                    SET role = ?, groups = ?, setor = ?, email = ?, celular = ?, updated_at = ?
                  WHERE username = ?
                    AND (role <> 'admin' OR ? = 'admin'
                         OR (SELECT count(*) FROM users WHERE role = 'admin') > 1)",
            )
            .bind(update.role.as_str())
            .bind(&groups)
            .bind(setor)
            .bind(email)
            .bind(celular)
            .bind(now)
            .bind(&username)
            .bind(update.role.as_str())
            .execute(&mut *tx)
            .await
            .with_context(|| format!("update account {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!(
                    "update account {username}: no-op (not found, or would strip the last admin)"
                );
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'user.update', ?, ?, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit account update")?;
            tx.commit().await.context("commit account update")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin account update")?;
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(ADMIN_MUTATION_LOCK)
                .execute(&mut *tx)
                .await
                .context("acquire admin-mutation lock")?;
            let res = sqlx::query(
                "UPDATE users
                    SET role = $1, groups = $2, setor = $3, email = $4, celular = $5, updated_at = $6
                  WHERE username = $7
                    AND (role <> 'admin' OR $1 = 'admin'
                         OR (SELECT count(*) FROM users WHERE role = 'admin') > 1)",
            )
            .bind(update.role.as_str())
            .bind(&groups)
            .bind(setor)
            .bind(email)
            .bind(celular)
            .bind(now)
            .bind(&username)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("update account {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!(
                    "update account {username}: no-op (not found, or would strip the last admin)"
                );
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'user.update', $2, $3, $4)",
            )
            .bind(actor)
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit account update")?;
            tx.commit().await.context("commit account update")?;
        }
    }
    Ok(())
}

pub async fn set_role(
    db: &ConfigDb,
    username: &str,
    role: Role,
    actor: Option<&str>,
) -> Result<()> {
    let username = normalize_username(username);
    let now = Utc::now();

    // The UPDATE is **conditional** so it can never demote the last admin
    // (#872): it no-ops when the row is the sole admin and the new role
    // isn't admin. Combined with SQLite write-serialization / the Postgres
    // advisory lock below, two concurrent demotions can't both pass.
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin role tx")?;
            let res = sqlx::query(
                "UPDATE users SET role = ?, updated_at = ? WHERE username = ?
                   AND (role <> 'admin' OR ? = 'admin'
                        OR (SELECT count(*) FROM users WHERE role = 'admin') > 1)",
            )
            .bind(role.as_str())
            .bind(now)
            .bind(&username)
            .bind(role.as_str())
            .execute(&mut *tx)
            .await
            .with_context(|| format!("set role for {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("set role for {username}: no-op (not found, or would strip the last admin)");
            }
            audit(&mut tx, actor, "user.role", &username, &role, now).await?;
            tx.commit().await.context("commit role change")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin role tx")?;
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(ADMIN_MUTATION_LOCK)
                .execute(&mut *tx)
                .await
                .context("acquire admin-mutation lock")?;
            let res = sqlx::query(
                "UPDATE users SET role = $1, updated_at = $2 WHERE username = $3
                   AND (role <> 'admin' OR $1 = 'admin'
                        OR (SELECT count(*) FROM users WHERE role = 'admin') > 1)",
            )
            .bind(role.as_str())
            .bind(now)
            .bind(&username)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("set role for {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("set role for {username}: no-op (not found, or would strip the last admin)");
            }
            audit_pg(&mut tx, actor, "user.role", &username, &role, now).await?;
            tx.commit().await.context("commit role change")?;
        }
    }
    Ok(())
}

/// Replace a user's group memberships. The list is canonicalized
/// ([`join_groups`]) before storage. Records the new list in the audit
/// log's `diff_json`.
pub async fn set_groups(
    db: &ConfigDb,
    username: &str,
    groups: &[String],
    actor: Option<&str>,
) -> Result<()> {
    let username = normalize_username(username);
    let groups = join_groups(groups);
    let now = Utc::now();
    let target = format!("user:{username}");
    let diff = serde_json::json!({ "groups": groups }).to_string();

    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin groups tx")?;
            let res = sqlx::query("UPDATE users SET groups = ?, updated_at = ? WHERE username = ?")
                .bind(&groups)
                .bind(now)
                .bind(&username)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("set groups for {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("user {username} not found");
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'user.groups', ?, ?, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit groups change")?;
            tx.commit().await.context("commit groups change")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin groups tx")?;
            let res =
                sqlx::query("UPDATE users SET groups = $1, updated_at = $2 WHERE username = $3")
                    .bind(&groups)
                    .bind(now)
                    .bind(&username)
                    .execute(&mut *tx)
                    .await
                    .with_context(|| format!("set groups for {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("user {username} not found");
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'user.groups', $2, $3, $4)",
            )
            .bind(actor)
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit groups change")?;
            tx.commit().await.context("commit groups change")?;
        }
    }
    Ok(())
}

/// Replace a user's optional profile fields (#856): setor, e-mail,
/// celular. A blank value is stored as `NULL` (see [`row_from`]'s clean).
/// Records the change in the audit log.
pub async fn update_profile(
    db: &ConfigDb,
    username: &str,
    setor: Option<&str>,
    email: Option<&str>,
    celular: Option<&str>,
    actor: Option<&str>,
) -> Result<()> {
    // Normalize "" → NULL so a cleared field round-trips as absent.
    fn norm(s: Option<&str>) -> Option<&str> {
        s.map(str::trim).filter(|v| !v.is_empty())
    }
    let (setor, email, celular) = (norm(setor), norm(email), norm(celular));
    let username = normalize_username(username);
    let now = Utc::now();
    let target = format!("user:{username}");
    let diff = serde_json::json!({
        "setor": setor, "email": email, "celular": celular,
    })
    .to_string();

    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin profile tx")?;
            let res = sqlx::query(
                "UPDATE users SET setor = ?, email = ?, celular = ?, updated_at = ? WHERE username = ?",
            )
            .bind(setor)
            .bind(email)
            .bind(celular)
            .bind(now)
            .bind(&username)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("set profile for {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("user {username} not found");
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'user.profile', ?, ?, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit profile change")?;
            tx.commit().await.context("commit profile change")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin profile tx")?;
            let res = sqlx::query(
                "UPDATE users SET setor = $1, email = $2, celular = $3, updated_at = $4 WHERE username = $5",
            )
            .bind(setor)
            .bind(email)
            .bind(celular)
            .bind(now)
            .bind(&username)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("set profile for {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("user {username} not found");
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'user.profile', $2, $3, $4)",
            )
            .bind(actor)
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit profile change")?;
            tx.commit().await.context("commit profile change")?;
        }
    }
    Ok(())
}

/// Clear the first-login password-change prompt once it's been
/// answered (whether or not the user actually changed it). No audit —
/// it's a benign per-user UI flag. `= FALSE` is valid on both backends.
pub async fn clear_must_change(db: &ConfigDb, username: &str) -> Result<()> {
    let username = normalize_username(username);
    let ctx = || format!("clear must-change for {username}");
    match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query("UPDATE users SET must_change_password = FALSE WHERE username = ?")
                .bind(&username)
                .execute(pool)
                .await
                .with_context(ctx)?;
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query("UPDATE users SET must_change_password = FALSE WHERE username = $1")
                .bind(&username)
                .execute(pool)
                .await
                .with_context(ctx)?;
        }
    }
    Ok(())
}

/// Delete a user.
pub async fn delete(db: &ConfigDb, username: &str, actor: Option<&str>) -> Result<()> {
    let username = normalize_username(username);
    let now = Utc::now();
    let target = format!("user:{username}");

    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin user delete")?;
            // Conditional so it can never delete the last admin (#872).
            // `user_mfa_grants.username` has ON DELETE CASCADE, so this
            // already-audited user deletion revokes grants atomically without
            // a duplicate trusted-device audit row.
            let res = sqlx::query(
                "DELETE FROM users WHERE username = ?
                   AND (role <> 'admin'
                        OR (SELECT count(*) FROM users WHERE role = 'admin') > 1)",
            )
            .bind(&username)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("delete user {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("delete user {username}: no-op (not found, or would strip the last admin)");
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, 'user.delete', ?, NULL, ?)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit user delete")?;
            tx.commit().await.context("commit user delete")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin user delete")?;
            sqlx::query("SELECT pg_advisory_xact_lock($1)")
                .bind(ADMIN_MUTATION_LOCK)
                .execute(&mut *tx)
                .await
                .context("acquire admin-mutation lock")?;
            let res = sqlx::query(
                "DELETE FROM users WHERE username = $1
                   AND (role <> 'admin'
                        OR (SELECT count(*) FROM users WHERE role = 'admin') > 1)",
            )
            .bind(&username)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("delete user {username}"))?;
            if res.rows_affected() == 0 {
                anyhow::bail!("delete user {username}: no-op (not found, or would strip the last admin)");
            }
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, 'user.delete', $2, NULL, $3)",
            )
            .bind(actor)
            .bind(&target)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit user delete")?;
            tx.commit().await.context("commit user delete")?;
        }
    }
    Ok(())
}

/// Shared audit insert for create/role changes (records the new role
/// in `diff_json`).
async fn audit(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor: Option<&str>,
    action: &str,
    username: &str,
    role: &Role,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(actor)
    .bind(action)
    .bind(format!("user:{username}"))
    .bind(serde_json::json!({ "role": role.as_str() }).to_string())
    .bind(now)
    .execute(&mut **tx)
    .await
    .context("audit user op")?;
    Ok(())
}

/// Postgres twin of [`audit`] — `$n` placeholders.
async fn audit_pg(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Option<&str>,
    action: &str,
    username: &str,
    role: &Role,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(actor)
    .bind(action)
    .bind(format!("user:{username}"))
    .bind(serde_json::json!({ "role": role.as_str() }).to_string())
    .bind(now)
    .execute(&mut **tx)
    .await
    .context("audit user op")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pool() -> ConfigDb {
        ConfigDb::Sqlite(crate::db::open_memory().await.expect("memory db"))
    }

    #[test]
    fn hash_and_verify_roundtrip() {
        let h = hash_password("s3cret-pw").unwrap();
        assert!(verify_password("s3cret-pw", &h));
        assert!(!verify_password("wrong", &h));
        assert!(!verify_password("s3cret-pw", "not-a-phc-string"));
    }

    #[test]
    fn username_charset_is_restricted() {
        assert!(is_valid_username("jane"));
        assert!(is_valid_username("jane.doe_99"));
        assert!(is_valid_username("ops@de.ufpe.br")); // e-mail logins
        // Unsafe in the per-user action URL path segment (#429).
        assert!(!is_valid_username("foo/bar"));
        assert!(!is_valid_username("a?b"));
        assert!(!is_valid_username("a#b"));
        assert!(!is_valid_username("with space"));
        assert!(!is_valid_username(""));
    }

    #[tokio::test]
    async fn create_then_verify_login() {
        let p = pool().await;
        create(
            &p,
            "Alice",
            "pw-alice",
            Role::Editor,
            true,
            &["analysts".to_string()],
            Some("admin"),
        )
        .await
        .unwrap();
        // Username is case-insensitive.
        let u = verify_login(&p, "alice", "pw-alice").await.unwrap();
        let u = u.expect("login ok");
        assert_eq!(u.username, "alice");
        assert_eq!(u.role, Role::Editor);
        assert!(u.must_change_password);
        // Groups round-trip through the column.
        assert_eq!(u.groups, vec!["analysts".to_string()]);
        // Wrong password / unknown user ⇒ None.
        assert!(verify_login(&p, "alice", "nope").await.unwrap().is_none());
        assert!(verify_login(&p, "ghost", "pw").await.unwrap().is_none());
    }

    #[test]
    fn parse_and_join_groups_canonicalize() {
        // Trim, drop empties, dedupe (first wins), preserve order.
        assert_eq!(
            parse_groups(" analysts , ,leads, analysts ,viewers"),
            vec![
                "analysts".to_string(),
                "leads".to_string(),
                "viewers".to_string()
            ]
        );
        assert!(parse_groups("").is_empty());
        assert!(parse_groups("  ,  , ").is_empty());
        assert_eq!(
            join_groups(&["a".to_string(), " a ".to_string(), "b".to_string()]),
            "a,b"
        );
    }

    #[tokio::test]
    async fn set_groups_replaces_membership() {
        let p = pool().await;
        create(&p, "carol", "pw-carol1", Role::Viewer, false, &[], Some("a"))
            .await
            .unwrap();
        assert!(fetch(&p, "carol").await.unwrap().unwrap().groups.is_empty());
        set_groups(
            &p,
            "carol",
            &["leads".to_string(), "leads".to_string(), "ops".to_string()],
            Some("a"),
        )
        .await
        .unwrap();
        let u = fetch(&p, "carol").await.unwrap().unwrap();
        assert_eq!(u.groups, vec!["leads".to_string(), "ops".to_string()]);
        // Clearing groups writes an empty list back.
        set_groups(&p, "carol", &[], Some("a")).await.unwrap();
        assert!(fetch(&p, "carol").await.unwrap().unwrap().groups.is_empty());
        // Unknown user ⇒ error.
        assert!(set_groups(&p, "ghost", &[], Some("a")).await.is_err());
    }

    #[tokio::test]
    async fn update_account_saves_role_groups_and_profile_together() {
        let p = pool().await;
        create(&p, "alice", "pw-alice1", Role::Viewer, false, &[], Some("root"))
            .await
            .unwrap();
        let groups = vec!["analysts".to_string(), "managers".to_string()];

        update_account(
            &p,
            "alice",
            AccountUpdate {
                role: Role::Editor,
                groups: &groups,
                setor: Some(" Data "),
                email: Some(" alice@example.com "),
                celular: Some(" 555-0100 "),
            },
            Some("root"),
        )
        .await
        .unwrap();

        let user = fetch(&p, "alice").await.unwrap().unwrap();
        assert_eq!(user.role, Role::Editor);
        assert_eq!(user.groups, groups);
        assert_eq!(user.setor.as_deref(), Some("Data"));
        assert_eq!(user.email.as_deref(), Some("alice@example.com"));
        assert_eq!(user.celular.as_deref(), Some("555-0100"));
    }

    #[tokio::test]
    async fn update_account_rolls_back_fields_when_demoting_last_admin() {
        let p = pool().await;
        create(&p, "root", "pw-root12", Role::Admin, false, &[], None)
            .await
            .unwrap();
        let groups = vec!["changed".to_string()];

        let result = update_account(
            &p,
            "root",
            AccountUpdate {
                role: Role::Viewer,
                groups: &groups,
                setor: Some("Changed"),
                email: Some("changed@example.com"),
                celular: Some("555"),
            },
            Some("root"),
        )
        .await;
        assert!(result.is_err());

        let user = fetch(&p, "root").await.unwrap().unwrap();
        assert_eq!(user.role, Role::Admin);
        assert!(user.groups.is_empty());
        assert!(user.setor.is_none());
        assert!(user.email.is_none());
        assert!(user.celular.is_none());
    }

    #[tokio::test]
    async fn profile_fields_roundtrip() {
        let p = pool().await;
        create(&p, "dora", "pw-dora123", Role::Viewer, false, &[], Some("a"))
            .await
            .unwrap();
        // Absent by default.
        let u = fetch(&p, "dora").await.unwrap().unwrap();
        assert_eq!((u.setor, u.email, u.celular), (None, None, None));

        // Set them; blanks/whitespace normalize to None.
        update_profile(
            &p,
            "dora",
            Some("  GAPE  "),
            Some("dora@orgao.gov.br"),
            Some("   "),
            Some("a"),
        )
        .await
        .unwrap();
        let u = fetch(&p, "dora").await.unwrap().unwrap();
        assert_eq!(u.setor.as_deref(), Some("GAPE")); // trimmed
        assert_eq!(u.email.as_deref(), Some("dora@orgao.gov.br"));
        assert_eq!(u.celular, None); // whitespace ⇒ NULL

        // Clearing wipes back to None.
        update_profile(&p, "dora", Some(""), None, None, Some("a"))
            .await
            .unwrap();
        let u = fetch(&p, "dora").await.unwrap().unwrap();
        assert_eq!((u.setor, u.email, u.celular), (None, None, None));

        // Unknown user ⇒ error.
        assert!(update_profile(&p, "ghost", Some("x"), None, None, None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn admin_count_and_roles() {
        let p = pool().await;
        assert!(!any_admin_exists(&p).await.unwrap());
        create(&p, "root", "pw", Role::Admin, false, &[], None)
            .await
            .unwrap();
        create(&p, "ed", "pw", Role::Editor, false, &[], None)
            .await
            .unwrap();
        assert_eq!(count_admins(&p).await.unwrap(), 1);
        assert!(any_admin_exists(&p).await.unwrap());
        // Promote editor → admin.
        set_role(&p, "ed", Role::Admin, Some("root")).await.unwrap();
        assert_eq!(count_admins(&p).await.unwrap(), 2);
    }

    #[tokio::test]
    async fn last_admin_guard_is_atomic_at_the_db() {
        // #872: the conditional UPDATE/DELETE refuses to strip the last
        // admin even if a handler-level pre-check were raced away.
        let p = pool().await;
        create(&p, "root", "rootpw12", Role::Admin, false, &[], None)
            .await
            .unwrap();
        // Sole admin: demote is refused and the row stays admin.
        assert!(set_role(&p, "root", Role::Viewer, None).await.is_err());
        assert_eq!(fetch(&p, "root").await.unwrap().unwrap().role, Role::Admin);
        // Sole admin: delete is refused and the row stays.
        assert!(delete(&p, "root", None).await.is_err());
        assert!(fetch(&p, "root").await.unwrap().is_some());

        // With two admins, demoting one works and leaves exactly one…
        create(&p, "root2", "rootpw34", Role::Admin, false, &[], None)
            .await
            .unwrap();
        set_role(&p, "root2", Role::Editor, None).await.unwrap();
        assert_eq!(count_admins(&p).await.unwrap(), 1);
        // …and the remaining sole admin still can't be stripped.
        assert!(set_role(&p, "root", Role::Viewer, None).await.is_err());
        assert!(delete(&p, "root", None).await.is_err());
    }

    #[tokio::test]
    async fn duplicate_username_rejected() {
        let p = pool().await;
        create(&p, "dup", "pw", Role::Viewer, false, &[], None)
            .await
            .unwrap();
        assert!(create(&p, "DUP", "pw2", Role::Viewer, false, &[], None)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn set_password_clears_must_change() {
        let p = pool().await;
        create(&p, "bob", "init-pw", Role::Viewer, true, &[], Some("admin"))
            .await
            .unwrap();
        set_password(&p, "bob", "new-pw", false, Some("bob"))
            .await
            .unwrap();
        let u = verify_login(&p, "bob", "new-pw").await.unwrap().unwrap();
        assert!(!u.must_change_password);
        assert!(verify_login(&p, "bob", "init-pw").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn list_page_and_count_filtered() {
        let p = pool().await;
        create(&p, "alice", "pw-a", Role::Admin, false, &[], None)
            .await
            .unwrap();
        create(&p, "bob", "pw-b", Role::Viewer, false, &["ops".to_string()], None)
            .await
            .unwrap();
        create(&p, "carol", "pw-c", Role::Viewer, false, &[], None)
            .await
            .unwrap();
        update_profile(&p, "carol", None, Some("carol@example.com"), None, None)
            .await
            .unwrap();

        // Unfiltered: pages partition the full set with no overlap.
        assert_eq!(count_filtered(&p, "").await.unwrap(), 3);
        let first = list_page(&p, "", 2, 0).await.unwrap();
        let rest = list_page(&p, "", 2, 2).await.unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(rest.len(), 1);
        let mut all: Vec<String> = first
            .iter()
            .chain(rest.iter())
            .map(|u| u.username.clone())
            .collect();
        all.sort();
        assert_eq!(all, vec!["alice", "bob", "carol"]);
        // Past the end ⇒ empty page, not an error.
        assert!(list_page(&p, "", 2, 4).await.unwrap().is_empty());

        // Search is a case-insensitive substring over username, groups
        // and profile fields.
        assert_eq!(count_filtered(&p, "ALI").await.unwrap(), 1);
        let hit = list_page(&p, "ALI", 50, 0).await.unwrap();
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].username, "alice");
        assert_eq!(list_page(&p, "ops", 50, 0).await.unwrap()[0].username, "bob");
        assert_eq!(
            list_page(&p, "@example", 50, 0).await.unwrap()[0].username,
            "carol"
        );
        // LIKE wildcards in the term match literally, not as patterns.
        assert_eq!(count_filtered(&p, "%").await.unwrap(), 0);
        assert_eq!(count_filtered(&p, "a_i").await.unwrap(), 0);
        assert_eq!(count_filtered(&p, "nobody").await.unwrap(), 0);

        // Accented text: SQLite's LIKE only folds ASCII, so the sqlite
        // arm matches both Rust-cased variants of the term (codex
        // review, PR #1000). `GESTÃO` must find a stored `Gestão` and
        // vice-versa.
        update_profile(&p, "bob", Some("Gestão"), None, None, None)
            .await
            .unwrap();
        assert_eq!(count_filtered(&p, "GESTÃO").await.unwrap(), 1);
        assert_eq!(count_filtered(&p, "gestão").await.unwrap(), 1);
        assert_eq!(
            list_page(&p, "GESTÃO", 50, 0).await.unwrap()[0].username,
            "bob"
        );
        update_profile(&p, "bob", Some("GESTÃO"), None, None, None)
            .await
            .unwrap();
        assert_eq!(count_filtered(&p, "gestão").await.unwrap(), 1);
    }

    // Full user lifecycle through the `ConfigDb::Postgres` arm against a
    // real daemon (BOOLEAN `must_change_password`, argon2 verify, audit,
    // last-admin count). Gated on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn users_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pg = crate::db::open_pg(&url).await.unwrap();
        sqlx::query("DELETE FROM users")
            .execute(&pg)
            .await
            .unwrap();
        let db = ConfigDb::Postgres(pg);

        assert!(!any_admin_exists(&db).await.unwrap());
        create(&db, "Root", "rootpw12", Role::Admin, false, &[], None)
            .await
            .unwrap();
        create(&db, "Ed", "edpw1234", Role::Editor, true, &["leads".to_string()], Some("root"))
            .await
            .unwrap();
        assert_eq!(count_admins(&db).await.unwrap(), 1);
        assert!(any_user_exists(&db).await.unwrap());
        assert_eq!(list_all(&db).await.unwrap().len(), 2);

        // Pagination + ILIKE search through the Postgres arm (#999).
        assert_eq!(count_filtered(&db, "").await.unwrap(), 2);
        assert_eq!(list_page(&db, "", 1, 0).await.unwrap().len(), 1);
        assert_eq!(list_page(&db, "", 10, 2).await.unwrap().len(), 0);
        let hit = list_page(&db, "ED", 10, 0).await.unwrap();
        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].username, "ed");
        assert_eq!(count_filtered(&db, "%").await.unwrap(), 0);

        // Case-insensitive login + must_change BOOLEAN round-trip.
        let u = verify_login(&db, "ed", "edpw1234")
            .await
            .unwrap()
            .expect("login ok");
        assert_eq!(u.role, Role::Editor);
        assert!(u.must_change_password);
        // Groups column round-trips through the Postgres TEXT arm.
        assert_eq!(u.groups, vec!["leads".to_string()]);
        assert!(verify_login(&db, "ed", "wrong").await.unwrap().is_none());
        assert!(verify_login(&db, "ghost", "x").await.unwrap().is_none());

        // Replace group membership, then clear it.
        set_groups(
            &db,
            "ed",
            &["leads".to_string(), "ops".to_string()],
            Some("root"),
        )
        .await
        .unwrap();
        assert_eq!(
            fetch(&db, "ed").await.unwrap().unwrap().groups,
            vec!["leads".to_string(), "ops".to_string()]
        );
        set_groups(&db, "ed", &[], Some("root")).await.unwrap();
        assert!(fetch(&db, "ed").await.unwrap().unwrap().groups.is_empty());

        // Promote, re-set password (clears must_change), then delete.
        set_role(&db, "ed", Role::Admin, Some("root")).await.unwrap();
        assert_eq!(count_admins(&db).await.unwrap(), 2);
        set_password(&db, "ed", "newpw123", false, Some("ed"))
            .await
            .unwrap();
        assert!(!verify_login(&db, "ed", "newpw123")
            .await
            .unwrap()
            .unwrap()
            .must_change_password);
        clear_must_change(&db, "root").await.unwrap();
        delete(&db, "ed", Some("root")).await.unwrap();
        assert!(fetch(&db, "ed").await.unwrap().is_none());
        assert_eq!(count_admins(&db).await.unwrap(), 1);
    }
}
