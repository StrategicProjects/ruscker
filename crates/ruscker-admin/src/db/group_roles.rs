//! Group roles (#1067) — named roles defined inside a group and held by
//! that group's members.
//!
//! **Not the account role.** `auth::Role` (Admin / Editor / Viewer) says
//! what someone may do in the Ruscker admin panel. A *group role* is a
//! free label (`gestor`, `analista`, …) that a hosted app reads through the
//! opt-in `X-Ruscker-User-Roles` header to decide what the user may do
//! inside the app. It grants nothing in Ruscker itself.
//!
//! The model:
//!
//! * Each group has its own list of roles (no global catalogue). The pair
//!   `(group, role)` is the identity of a role; `gestor` in two groups is
//!   two different roles.
//! * A user may hold several roles in one group, but only roles defined in
//!   that group and only while they belong to it.
//!
//! Groups are still derived — there is no `groups` table — so the second
//! invariant cannot be a foreign key. It is kept here, in the storage
//! layer, at every place that can break it:
//!
//! | change | where it is enforced |
//! |---|---|
//! | user leaves a group (any writer of `users.groups`) | [`prune_memberships_sqlite`] / [`prune_memberships_postgres`], called inside the SAME transaction by `users::set_groups` and `users::update_account` |
//! | user deleted | FK `ON DELETE CASCADE` + an explicit delete in `users::delete` |
//! | role removed from the group | FK `ON DELETE CASCADE` from `group_roles` |
//! | group renamed | [`rename_group`] (definitions and assignments move together) |
//! | group deleted | `users::set_groups` prunes each member, then [`delete_group`] drops the definitions |
//! | assigning to a non-member / undefined role | [`assign`] checks both against the authoritative rows, in the transaction that inserts |
//!
//! The proxy additionally filters by current membership when it builds the
//! header, so even a row that slipped past (a partially failed group
//! rename) never reaches an app for a group the user is no longer in.
//!
//! Every mutation writes its `audit_log` row in the same transaction.

use std::collections::BTreeMap;

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};

use crate::db::users::{normalize_username, parse_groups};
use crate::db::ConfigDb;

/// Longest accepted role name. Roles travel in an HTTP header, one entry
/// per role, so keep them short.
pub const MAX_ROLE_LEN: usize = 40;

/// Validate and normalize a role name: trimmed, 1–[`MAX_ROLE_LEN`] bytes of
/// `[A-Za-z0-9_.-]`. Spaces, `:` and `,` are refused because they are the
/// separators of the header encoding; anything non-ASCII is refused so a
/// role never needs escaping. Case is kept (and compared exactly), like
/// group names.
pub fn clean_role(raw: &str) -> Option<String> {
    let role = raw.trim();
    let ok = !role.is_empty()
        && role.len() <= MAX_ROLE_LEN
        && role
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'.' | b'-'));
    ok.then(|| role.to_string())
}

/// A group name as stored in `users.groups`: non-empty, no comma (the CSV
/// separator). Same rule as the Groups page's `clean_group`.
fn valid_group(group: &str) -> bool {
    !group.is_empty() && group == group.trim() && !group.contains(',')
}

/// One role held by one user.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Assignment {
    pub username: String,
    pub group: String,
    pub role: String,
}

/// Result of [`assign`]. Only `Assigned` changed anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssignOutcome {
    Assigned,
    /// The user already held this role — nothing written, nothing audited.
    AlreadyHeld,
    /// The user does not exist or does not belong to the group.
    NotMember,
    /// The role is not defined in that group.
    UnknownRole,
}

// ── Reads ───────────────────────────────────────────────────────────────

/// Every defined role, grouped by group name. Both levels are sorted.
pub async fn list_definitions(db: &ConfigDb) -> Result<BTreeMap<String, Vec<String>>> {
    let sql = "SELECT group_name, role FROM group_roles ORDER BY group_name, role";
    let rows: Vec<(String, String)> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    }
    .context("list group roles")?;
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (group, role) in rows {
        map.entry(group).or_default().push(role);
    }
    Ok(map)
}

/// Every assignment, sorted by group, user, role.
pub async fn list_assignments(db: &ConfigDb) -> Result<Vec<Assignment>> {
    let sql = "SELECT username, group_name, role FROM user_group_roles
                ORDER BY group_name, username, role";
    let rows: Vec<(String, String, String)> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    }
    .context("list group role assignments")?;
    Ok(rows
        .into_iter()
        .map(|(username, group, role)| Assignment { username, group, role })
        .collect())
}

/// The `(group, role)` pairs one user holds, sorted. This is what the
/// proxy caches per user; the per-spec filtering happens later, per
/// request, so the cached value never depends on which app asked.
pub async fn roles_of_user(db: &ConfigDb, username: &str) -> Result<Vec<(String, String)>> {
    let username = normalize_username(username);
    let rows: Vec<(String, String)> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as(
                "SELECT group_name, role FROM user_group_roles
                  WHERE username = ? ORDER BY group_name, role",
            )
            .bind(&username)
            .fetch_all(pool)
            .await
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as(
                "SELECT group_name, role FROM user_group_roles
                  WHERE username = $1 ORDER BY group_name, role",
            )
            .bind(&username)
            .fetch_all(pool)
            .await
        }
    }
    .with_context(|| format!("list group roles of {username}"))?;
    Ok(rows)
}

// ── Audit helpers ───────────────────────────────────────────────────────

async fn audit_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    actor: Option<&str>,
    action: &str,
    target: &str,
    diff: &serde_json::Value,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(actor)
    .bind(action)
    .bind(target)
    .bind(diff.to_string())
    .bind(now)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("audit {action}"))?;
    Ok(())
}

async fn audit_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    actor: Option<&str>,
    action: &str,
    target: &str,
    diff: &serde_json::Value,
    now: DateTime<Utc>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(actor)
    .bind(action)
    .bind(target)
    .bind(diff.to_string())
    .bind(now)
    .execute(&mut **tx)
    .await
    .with_context(|| format!("audit {action}"))?;
    Ok(())
}

// ── Definitions ─────────────────────────────────────────────────────────

/// Define `role` in `group`. Returns `false` (and writes nothing) when the
/// role already exists there. Errors on an invalid name — callers validate
/// with [`clean_role`] first; this is the backstop.
pub async fn define(db: &ConfigDb, group: &str, role: &str, actor: Option<&str>) -> Result<bool> {
    if !valid_group(group) || clean_role(role).as_deref() != Some(role) {
        bail!("invalid group role {group:?}/{role:?}");
    }
    let now = Utc::now();
    let target = format!("group:{group}");
    let diff = serde_json::json!({ "role": role });
    let inserted = match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin group role define")?;
            let n = sqlx::query(
                "INSERT INTO group_roles (group_name, role, created_at, created_by)
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT(group_name, role) DO NOTHING",
            )
            .bind(group)
            .bind(role)
            .bind(now)
            .bind(actor)
            .execute(&mut *tx)
            .await
            .context("insert group role")?
            .rows_affected();
            if n == 1 {
                audit_sqlite(&mut tx, actor, "group-role.define", &target, &diff, now).await?;
            }
            tx.commit().await.context("commit group role define")?;
            n == 1
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin group role define")?;
            let n = sqlx::query(
                "INSERT INTO group_roles (group_name, role, created_at, created_by)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT(group_name, role) DO NOTHING",
            )
            .bind(group)
            .bind(role)
            .bind(now)
            .bind(actor)
            .execute(&mut *tx)
            .await
            .context("insert group role")?
            .rows_affected();
            if n == 1 {
                audit_postgres(&mut tx, actor, "group-role.define", &target, &diff, now).await?;
            }
            tx.commit().await.context("commit group role define")?;
            n == 1
        }
    };
    Ok(inserted)
}

/// Remove `role` from `group`. Every assignment of it goes with it (FK
/// cascade, plus an explicit delete so the audit row can say how many).
/// Returns `false` when the role was not defined.
pub async fn remove(db: &ConfigDb, group: &str, role: &str, actor: Option<&str>) -> Result<bool> {
    let now = Utc::now();
    let target = format!("group:{group}");
    let removed = match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin group role remove")?;
            let holders = sqlx::query(
                "DELETE FROM user_group_roles WHERE group_name = ? AND role = ?",
            )
            .bind(group)
            .bind(role)
            .execute(&mut *tx)
            .await
            .context("delete group role assignments")?
            .rows_affected();
            let n = sqlx::query("DELETE FROM group_roles WHERE group_name = ? AND role = ?")
                .bind(group)
                .bind(role)
                .execute(&mut *tx)
                .await
                .context("delete group role")?
                .rows_affected();
            if n == 1 {
                let diff = serde_json::json!({ "role": role, "holders_removed": holders });
                audit_sqlite(&mut tx, actor, "group-role.remove", &target, &diff, now).await?;
            }
            tx.commit().await.context("commit group role remove")?;
            n == 1
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin group role remove")?;
            let holders = sqlx::query(
                "DELETE FROM user_group_roles WHERE group_name = $1 AND role = $2",
            )
            .bind(group)
            .bind(role)
            .execute(&mut *tx)
            .await
            .context("delete group role assignments")?
            .rows_affected();
            let n = sqlx::query("DELETE FROM group_roles WHERE group_name = $1 AND role = $2")
                .bind(group)
                .bind(role)
                .execute(&mut *tx)
                .await
                .context("delete group role")?
                .rows_affected();
            if n == 1 {
                let diff = serde_json::json!({ "role": role, "holders_removed": holders });
                audit_postgres(&mut tx, actor, "group-role.remove", &target, &diff, now).await?;
            }
            tx.commit().await.context("commit group role remove")?;
            n == 1
        }
    };
    Ok(removed)
}

// ── Assignments ─────────────────────────────────────────────────────────

/// Give `username` the role `role` of `group`.
///
/// Both preconditions are read from the authoritative rows inside the
/// transaction that inserts: the user's `users.groups` must contain
/// `group`, and `(group, role)` must be defined. On Postgres both rows are
/// read `FOR SHARE`, so a concurrent `set_groups` (which `UPDATE`s the user
/// row and then prunes) or role removal (which `DELETE`s the definition)
/// serializes with this insert instead of racing past it. SQLite already
/// serializes writers.
pub async fn assign(
    db: &ConfigDb,
    username: &str,
    group: &str,
    role: &str,
    actor: Option<&str>,
) -> Result<AssignOutcome> {
    let username = normalize_username(username);
    let now = Utc::now();
    let target = format!("user:{username}");
    let diff = serde_json::json!({ "group": group, "role": role });
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin group role assign")?;
            let groups: Option<String> =
                sqlx::query_scalar("SELECT groups FROM users WHERE username = ?")
                    .bind(&username)
                    .fetch_optional(&mut *tx)
                    .await
                    .context("read memberships")?;
            if !groups.is_some_and(|g| parse_groups(&g).iter().any(|g| g == group)) {
                return Ok(AssignOutcome::NotMember);
            }
            let defined: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM group_roles WHERE group_name = ? AND role = ?",
            )
            .bind(group)
            .bind(role)
            .fetch_optional(&mut *tx)
            .await
            .context("read group role")?;
            if defined.is_none() {
                return Ok(AssignOutcome::UnknownRole);
            }
            let n = sqlx::query(
                "INSERT INTO user_group_roles (username, group_name, role, created_at)
                 VALUES (?, ?, ?, ?)
                 ON CONFLICT(username, group_name, role) DO NOTHING",
            )
            .bind(&username)
            .bind(group)
            .bind(role)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("insert group role assignment")?
            .rows_affected();
            if n == 0 {
                return Ok(AssignOutcome::AlreadyHeld);
            }
            audit_sqlite(&mut tx, actor, "group-role.assign", &target, &diff, now).await?;
            tx.commit().await.context("commit group role assign")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin group role assign")?;
            let groups: Option<String> =
                sqlx::query_scalar("SELECT groups FROM users WHERE username = $1 FOR SHARE")
                    .bind(&username)
                    .fetch_optional(&mut *tx)
                    .await
                    .context("read memberships")?;
            if !groups.is_some_and(|g| parse_groups(&g).iter().any(|g| g == group)) {
                return Ok(AssignOutcome::NotMember);
            }
            let defined: Option<i32> = sqlx::query_scalar(
                "SELECT 1 FROM group_roles WHERE group_name = $1 AND role = $2 FOR SHARE",
            )
            .bind(group)
            .bind(role)
            .fetch_optional(&mut *tx)
            .await
            .context("read group role")?;
            if defined.is_none() {
                return Ok(AssignOutcome::UnknownRole);
            }
            let n = sqlx::query(
                "INSERT INTO user_group_roles (username, group_name, role, created_at)
                 VALUES ($1, $2, $3, $4)
                 ON CONFLICT(username, group_name, role) DO NOTHING",
            )
            .bind(&username)
            .bind(group)
            .bind(role)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("insert group role assignment")?
            .rows_affected();
            if n == 0 {
                return Ok(AssignOutcome::AlreadyHeld);
            }
            audit_postgres(&mut tx, actor, "group-role.assign", &target, &diff, now).await?;
            tx.commit().await.context("commit group role assign")?;
        }
    }
    Ok(AssignOutcome::Assigned)
}

/// Take a role away from a user. Returns `false` when they did not hold it.
pub async fn unassign(
    db: &ConfigDb,
    username: &str,
    group: &str,
    role: &str,
    actor: Option<&str>,
) -> Result<bool> {
    let username = normalize_username(username);
    let now = Utc::now();
    let target = format!("user:{username}");
    let diff = serde_json::json!({ "group": group, "role": role });
    let removed = match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin group role unassign")?;
            let n = sqlx::query(
                "DELETE FROM user_group_roles
                  WHERE username = ? AND group_name = ? AND role = ?",
            )
            .bind(&username)
            .bind(group)
            .bind(role)
            .execute(&mut *tx)
            .await
            .context("delete group role assignment")?
            .rows_affected();
            if n == 1 {
                audit_sqlite(&mut tx, actor, "group-role.unassign", &target, &diff, now).await?;
            }
            tx.commit().await.context("commit group role unassign")?;
            n == 1
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin group role unassign")?;
            let n = sqlx::query(
                "DELETE FROM user_group_roles
                  WHERE username = $1 AND group_name = $2 AND role = $3",
            )
            .bind(&username)
            .bind(group)
            .bind(role)
            .execute(&mut *tx)
            .await
            .context("delete group role assignment")?
            .rows_affected();
            if n == 1 {
                audit_postgres(&mut tx, actor, "group-role.unassign", &target, &diff, now).await?;
            }
            tx.commit().await.context("commit group role unassign")?;
            n == 1
        }
    };
    Ok(removed)
}

// ── Group lifecycle ─────────────────────────────────────────────────────

/// Move every role of `old` (definitions AND assignments) to `new`.
///
/// When `new` already has a role of the same name the two fold into one
/// (`ON CONFLICT DO NOTHING`), mirroring how the group rename folds
/// memberships. Call this BEFORE rewriting the users' memberships: the
/// membership rewrite prunes assignments of groups a user leaves, and at
/// that point the assignments must already live under the new name.
pub async fn rename_group(db: &ConfigDb, old: &str, new: &str, actor: Option<&str>) -> Result<()> {
    if old == new {
        return Ok(());
    }
    if !valid_group(new) {
        bail!("invalid group name {new:?}");
    }
    let now = Utc::now();
    let target = format!("group:{old}");
    // The SELECT carries a WHERE, which SQLite needs to parse
    // `INSERT … SELECT … ON CONFLICT` unambiguously.
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin group role rename")?;
            let defs = sqlx::query(
                "INSERT INTO group_roles (group_name, role, created_at, created_by)
                 SELECT ?, role, created_at, created_by FROM group_roles WHERE group_name = ?
                 ON CONFLICT(group_name, role) DO NOTHING",
            )
            .bind(new)
            .bind(old)
            .execute(&mut *tx)
            .await
            .context("copy group roles")?;
            sqlx::query(
                "INSERT INTO user_group_roles (username, group_name, role, created_at)
                 SELECT username, ?, role, created_at FROM user_group_roles WHERE group_name = ?
                 ON CONFLICT(username, group_name, role) DO NOTHING",
            )
            .bind(new)
            .bind(old)
            .execute(&mut *tx)
            .await
            .context("copy group role assignments")?;
            sqlx::query("DELETE FROM user_group_roles WHERE group_name = ?")
                .bind(old)
                .execute(&mut *tx)
                .await
                .context("drop old group role assignments")?;
            let moved = sqlx::query("DELETE FROM group_roles WHERE group_name = ?")
                .bind(old)
                .execute(&mut *tx)
                .await
                .context("drop old group roles")?
                .rows_affected();
            if moved > 0 || defs.rows_affected() > 0 {
                let diff = serde_json::json!({ "renamed_to": new, "roles": moved });
                audit_sqlite(&mut tx, actor, "group-role.rename-group", &target, &diff, now)
                    .await?;
            }
            tx.commit().await.context("commit group role rename")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin group role rename")?;
            let defs = sqlx::query(
                "INSERT INTO group_roles (group_name, role, created_at, created_by)
                 SELECT $1, role, created_at, created_by FROM group_roles WHERE group_name = $2
                 ON CONFLICT(group_name, role) DO NOTHING",
            )
            .bind(new)
            .bind(old)
            .execute(&mut *tx)
            .await
            .context("copy group roles")?;
            sqlx::query(
                "INSERT INTO user_group_roles (username, group_name, role, created_at)
                 SELECT username, $1, role, created_at FROM user_group_roles WHERE group_name = $2
                 ON CONFLICT(username, group_name, role) DO NOTHING",
            )
            .bind(new)
            .bind(old)
            .execute(&mut *tx)
            .await
            .context("copy group role assignments")?;
            sqlx::query("DELETE FROM user_group_roles WHERE group_name = $1")
                .bind(old)
                .execute(&mut *tx)
                .await
                .context("drop old group role assignments")?;
            let moved = sqlx::query("DELETE FROM group_roles WHERE group_name = $1")
                .bind(old)
                .execute(&mut *tx)
                .await
                .context("drop old group roles")?
                .rows_affected();
            if moved > 0 || defs.rows_affected() > 0 {
                let diff = serde_json::json!({ "renamed_to": new, "roles": moved });
                audit_postgres(&mut tx, actor, "group-role.rename-group", &target, &diff, now)
                    .await?;
            }
            tx.commit().await.context("commit group role rename")?;
        }
    }
    Ok(())
}

/// Drop every role definition (and any assignment left) of a deleted group.
pub async fn delete_group(db: &ConfigDb, group: &str, actor: Option<&str>) -> Result<()> {
    let now = Utc::now();
    let target = format!("group:{group}");
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin group role delete")?;
            sqlx::query("DELETE FROM user_group_roles WHERE group_name = ?")
                .bind(group)
                .execute(&mut *tx)
                .await
                .context("drop group role assignments")?;
            let n = sqlx::query("DELETE FROM group_roles WHERE group_name = ?")
                .bind(group)
                .execute(&mut *tx)
                .await
                .context("drop group roles")?
                .rows_affected();
            if n > 0 {
                let diff = serde_json::json!({ "roles": n });
                audit_sqlite(&mut tx, actor, "group-role.delete-group", &target, &diff, now)
                    .await?;
            }
            tx.commit().await.context("commit group role delete")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin group role delete")?;
            sqlx::query("DELETE FROM user_group_roles WHERE group_name = $1")
                .bind(group)
                .execute(&mut *tx)
                .await
                .context("drop group role assignments")?;
            let n = sqlx::query("DELETE FROM group_roles WHERE group_name = $1")
                .bind(group)
                .execute(&mut *tx)
                .await
                .context("drop group roles")?
                .rows_affected();
            if n > 0 {
                let diff = serde_json::json!({ "roles": n });
                audit_postgres(&mut tx, actor, "group-role.delete-group", &target, &diff, now)
                    .await?;
            }
            tx.commit().await.context("commit group role delete")?;
        }
    }
    Ok(())
}

// ── Membership pruning (called by every writer of `users.groups`) ───────

/// Drop the assignments of `username` in every group not in `kept_csv`
/// (the canonical `users.groups` value just written). Runs INSIDE the
/// caller's transaction so the membership change and the pruning commit
/// or roll back together. Audited as `group-role.prune` only when it
/// removed something.
pub(crate) async fn prune_memberships_sqlite(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    username: &str,
    kept_csv: &str,
    actor: Option<&str>,
    now: DateTime<Utc>,
) -> Result<()> {
    let kept = parse_groups(kept_csv);
    let held: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT group_name FROM user_group_roles WHERE username = ?",
    )
    .bind(username)
    .fetch_all(&mut **tx)
    .await
    .context("read held group roles")?;
    let mut dropped = Vec::new();
    for group in held.into_iter().filter(|g| !kept.contains(g)) {
        sqlx::query("DELETE FROM user_group_roles WHERE username = ? AND group_name = ?")
            .bind(username)
            .bind(&group)
            .execute(&mut **tx)
            .await
            .context("prune group roles")?;
        dropped.push(group);
    }
    if !dropped.is_empty() {
        dropped.sort();
        let diff = serde_json::json!({ "left_groups": dropped });
        let target = format!("user:{username}");
        audit_sqlite(tx, actor, "group-role.prune", &target, &diff, now).await?;
    }
    Ok(())
}

/// Postgres twin of [`prune_memberships_sqlite`]. The caller has already
/// `UPDATE`d the user row, so a concurrent [`assign`] (which reads that row
/// `FOR SHARE`) either committed before — and its row is seen and pruned
/// here, READ COMMITTED reading fresh per statement — or waits and then
/// sees the new membership.
pub(crate) async fn prune_memberships_postgres(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    username: &str,
    kept_csv: &str,
    actor: Option<&str>,
    now: DateTime<Utc>,
) -> Result<()> {
    let kept = parse_groups(kept_csv);
    let held: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT group_name FROM user_group_roles WHERE username = $1",
    )
    .bind(username)
    .fetch_all(&mut **tx)
    .await
    .context("read held group roles")?;
    let mut dropped = Vec::new();
    for group in held.into_iter().filter(|g| !kept.contains(g)) {
        sqlx::query("DELETE FROM user_group_roles WHERE username = $1 AND group_name = $2")
            .bind(username)
            .bind(&group)
            .execute(&mut **tx)
            .await
            .context("prune group roles")?;
        dropped.push(group);
    }
    if !dropped.is_empty() {
        dropped.sort();
        let diff = serde_json::json!({ "left_groups": dropped });
        let target = format!("user:{username}");
        audit_postgres(tx, actor, "group-role.prune", &target, &diff, now).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;
    use crate::db::users;

    async fn mem_db() -> ConfigDb {
        ConfigDb::Sqlite(crate::db::open_memory().await.expect("open in-memory"))
    }

    async fn user(db: &ConfigDb, name: &str, groups: &[&str]) {
        let groups: Vec<String> = groups.iter().map(|g| g.to_string()).collect();
        users::create(db, name, "Correct#Pass9", Role::Viewer, false, &groups, Some("t"))
            .await
            .unwrap();
    }

    fn pairs(v: &[(&str, &str)]) -> Vec<(String, String)> {
        v.iter().map(|(g, r)| (g.to_string(), r.to_string())).collect()
    }

    async fn audit_actions(db: &ConfigDb) -> Vec<(String, Option<String>)> {
        crate::db::audit::list(db, &crate::db::audit::AuditFilter::new())
            .await
            .unwrap()
            .into_iter()
            .filter(|e| e.action.starts_with("group-role."))
            .map(|e| (e.action, e.target))
            .collect()
    }

    #[test]
    fn role_names_are_header_safe() {
        assert_eq!(clean_role("  gestor ").as_deref(), Some("gestor"));
        assert_eq!(clean_role("Ana.lista_2-b").as_deref(), Some("Ana.lista_2-b"));
        assert_eq!(clean_role(&"a".repeat(40)).as_deref(), Some(&*"a".repeat(40)));
        for bad in ["", "   ", "two words", "a:b", "a,b", "gestão", "a/b", "a%20"] {
            assert!(clean_role(bad).is_none(), "{bad:?} must be refused");
        }
        assert!(clean_role(&"a".repeat(41)).is_none(), "too long");
    }

    #[tokio::test]
    async fn define_remove_assign_unassign_round_trip_with_audit() {
        let db = mem_db().await;
        user(&db, "ana", &["fazenda"]).await;

        assert!(define(&db, "fazenda", "gestor", Some("root")).await.unwrap());
        assert!(!define(&db, "fazenda", "gestor", Some("root")).await.unwrap(), "idempotent");
        assert!(define(&db, "fazenda", "analista", Some("root")).await.unwrap());
        assert!(define(&db, "fazenda", "bad role", None).await.is_err());
        assert!(define(&db, "a,b", "gestor", None).await.is_err());
        assert_eq!(
            list_definitions(&db).await.unwrap().get("fazenda").cloned(),
            Some(vec!["analista".to_string(), "gestor".to_string()])
        );

        assert_eq!(
            assign(&db, "ana", "fazenda", "gestor", Some("ed")).await.unwrap(),
            AssignOutcome::Assigned
        );
        assert_eq!(
            assign(&db, "ANA", "fazenda", "gestor", Some("ed")).await.unwrap(),
            AssignOutcome::AlreadyHeld,
            "usernames are normalized"
        );
        assert_eq!(
            assign(&db, "ana", "fazenda", "analista", Some("ed")).await.unwrap(),
            AssignOutcome::Assigned
        );
        assert_eq!(
            roles_of_user(&db, "ana").await.unwrap(),
            pairs(&[("fazenda", "analista"), ("fazenda", "gestor")])
        );

        assert!(unassign(&db, "ana", "fazenda", "analista", Some("ed")).await.unwrap());
        assert!(!unassign(&db, "ana", "fazenda", "analista", Some("ed")).await.unwrap());
        assert!(remove(&db, "fazenda", "analista", Some("root")).await.unwrap());
        assert!(!remove(&db, "fazenda", "analista", Some("root")).await.unwrap());

        let audits = audit_actions(&db).await;
        let count = |a: &str| audits.iter().filter(|(x, _)| x == a).count();
        assert_eq!(count("group-role.define"), 2, "{audits:?}");
        assert_eq!(count("group-role.assign"), 2, "no audit for the no-op");
        assert_eq!(count("group-role.unassign"), 1);
        assert_eq!(count("group-role.remove"), 1);
        assert!(audits
            .iter()
            .any(|(a, t)| a == "group-role.assign" && t.as_deref() == Some("user:ana")));
        assert!(audits
            .iter()
            .any(|(a, t)| a == "group-role.define" && t.as_deref() == Some("group:fazenda")));
    }

    #[tokio::test]
    async fn assign_refuses_non_members_and_undefined_roles() {
        let db = mem_db().await;
        user(&db, "ana", &["fazenda"]).await;
        user(&db, "bia", &["saude"]).await;
        define(&db, "fazenda", "gestor", None).await.unwrap();
        define(&db, "saude", "medico", None).await.unwrap();

        // Not a member of the group.
        assert_eq!(
            assign(&db, "bia", "fazenda", "gestor", None).await.unwrap(),
            AssignOutcome::NotMember
        );
        // Unknown user.
        assert_eq!(
            assign(&db, "ghost", "fazenda", "gestor", None).await.unwrap(),
            AssignOutcome::NotMember
        );
        // Role defined, but in ANOTHER group.
        assert_eq!(
            assign(&db, "ana", "fazenda", "medico", None).await.unwrap(),
            AssignOutcome::UnknownRole
        );
        // Group names are case-sensitive, like access-groups.
        assert_eq!(
            assign(&db, "ana", "Fazenda", "gestor", None).await.unwrap(),
            AssignOutcome::NotMember
        );
        assert!(list_assignments(&db).await.unwrap().is_empty());
        assert!(audit_actions(&db)
            .await
            .iter()
            .all(|(a, _)| a != "group-role.assign"));
    }

    #[tokio::test]
    async fn leaving_a_group_drops_its_roles_via_set_groups() {
        let db = mem_db().await;
        user(&db, "ana", &["fazenda", "saude"]).await;
        define(&db, "fazenda", "gestor", None).await.unwrap();
        define(&db, "saude", "medico", None).await.unwrap();
        assign(&db, "ana", "fazenda", "gestor", None).await.unwrap();
        assign(&db, "ana", "saude", "medico", None).await.unwrap();

        users::set_groups(&db, "ana", &["saude".to_string()], Some("ed")).await.unwrap();
        assert_eq!(roles_of_user(&db, "ana").await.unwrap(), pairs(&[("saude", "medico")]));
        // Re-joining does NOT resurrect the old role.
        users::set_groups(&db, "ana", &["saude".into(), "fazenda".into()], None).await.unwrap();
        assert_eq!(roles_of_user(&db, "ana").await.unwrap(), pairs(&[("saude", "medico")]));
        // The definition itself survives a member leaving.
        assert!(list_definitions(&db).await.unwrap().contains_key("fazenda"));
        assert!(audit_actions(&db)
            .await
            .iter()
            .any(|(a, t)| a == "group-role.prune" && t.as_deref() == Some("user:ana")));
    }

    #[tokio::test]
    async fn leaving_a_group_drops_its_roles_via_update_account() {
        let db = mem_db().await;
        user(&db, "ana", &["fazenda", "saude"]).await;
        define(&db, "fazenda", "gestor", None).await.unwrap();
        define(&db, "saude", "medico", None).await.unwrap();
        assign(&db, "ana", "fazenda", "gestor", None).await.unwrap();
        assign(&db, "ana", "saude", "medico", None).await.unwrap();

        let groups = vec!["fazenda".to_string()];
        users::update_account(
            &db,
            "ana",
            users::AccountUpdate {
                role: Role::Viewer,
                groups: &groups,
                setor: None,
                email: None,
                celular: None,
            },
            Some("root"),
        )
        .await
        .unwrap();
        assert_eq!(roles_of_user(&db, "ana").await.unwrap(), pairs(&[("fazenda", "gestor")]));
    }

    #[tokio::test]
    async fn a_refused_membership_write_keeps_the_roles() {
        // update_account refuses to demote the last admin; the prune runs in
        // the same transaction, so the refusal must leave the roles alone.
        let db = mem_db().await;
        users::create(&db, "root", "Correct#Pass9", Role::Admin, false, &["ops".into()], None)
            .await
            .unwrap();
        define(&db, "ops", "plantao", None).await.unwrap();
        assign(&db, "root", "ops", "plantao", None).await.unwrap();
        let err = users::update_account(
            &db,
            "root",
            users::AccountUpdate {
                role: Role::Viewer,
                groups: &[],
                setor: None,
                email: None,
                celular: None,
            },
            None,
        )
        .await;
        assert!(err.is_err());
        assert_eq!(roles_of_user(&db, "root").await.unwrap(), pairs(&[("ops", "plantao")]));
    }

    #[tokio::test]
    async fn deleting_the_user_drops_their_roles() {
        let db = mem_db().await;
        user(&db, "ana", &["fazenda"]).await;
        define(&db, "fazenda", "gestor", None).await.unwrap();
        assign(&db, "ana", "fazenda", "gestor", None).await.unwrap();
        users::delete(&db, "ana", Some("root")).await.unwrap();
        assert!(list_assignments(&db).await.unwrap().is_empty());
        // A new account with the same name starts with no roles.
        user(&db, "ana", &["fazenda"]).await;
        assert!(roles_of_user(&db, "ana").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn removing_a_role_drops_every_holder() {
        let db = mem_db().await;
        user(&db, "ana", &["fazenda"]).await;
        user(&db, "bia", &["fazenda"]).await;
        define(&db, "fazenda", "gestor", None).await.unwrap();
        define(&db, "fazenda", "analista", None).await.unwrap();
        assign(&db, "ana", "fazenda", "gestor", None).await.unwrap();
        assign(&db, "bia", "fazenda", "gestor", None).await.unwrap();
        assign(&db, "bia", "fazenda", "analista", None).await.unwrap();
        remove(&db, "fazenda", "gestor", Some("root")).await.unwrap();
        assert_eq!(
            list_assignments(&db).await.unwrap(),
            vec![Assignment {
                username: "bia".into(),
                group: "fazenda".into(),
                role: "analista".into()
            }]
        );
        // Re-defining it does not bring the holders back.
        define(&db, "fazenda", "gestor", None).await.unwrap();
        assert!(roles_of_user(&db, "ana").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn renaming_a_group_moves_roles_and_holders_and_folds_duplicates() {
        let db = mem_db().await;
        user(&db, "ana", &["old"]).await;
        user(&db, "bia", &["old", "new"]).await;
        define(&db, "old", "gestor", None).await.unwrap();
        define(&db, "old", "analista", None).await.unwrap();
        define(&db, "new", "gestor", None).await.unwrap();
        assign(&db, "ana", "old", "analista", None).await.unwrap();
        assign(&db, "bia", "old", "gestor", None).await.unwrap();
        assign(&db, "bia", "new", "gestor", None).await.unwrap();

        // Same order as the Groups page: roles first, then memberships.
        rename_group(&db, "old", "new", Some("root")).await.unwrap();
        users::set_groups(&db, "ana", &["new".into()], Some("root")).await.unwrap();
        users::set_groups(&db, "bia", &["new".into()], Some("root")).await.unwrap();

        let defs = list_definitions(&db).await.unwrap();
        assert!(!defs.contains_key("old"));
        assert_eq!(defs["new"], vec!["analista".to_string(), "gestor".to_string()]);
        assert_eq!(roles_of_user(&db, "ana").await.unwrap(), pairs(&[("new", "analista")]));
        assert_eq!(roles_of_user(&db, "bia").await.unwrap(), pairs(&[("new", "gestor")]));
        assert!(audit_actions(&db).await.iter().any(|(a, t)| {
            a == "group-role.rename-group" && t.as_deref() == Some("group:old")
        }));
        // Nothing was pruned: the assignments were already under the new name.
        assert!(audit_actions(&db).await.iter().all(|(a, _)| a != "group-role.prune"));
    }

    #[tokio::test]
    async fn deleting_a_group_drops_definitions_and_assignments() {
        let db = mem_db().await;
        user(&db, "ana", &["fazenda", "saude"]).await;
        define(&db, "fazenda", "gestor", None).await.unwrap();
        define(&db, "saude", "medico", None).await.unwrap();
        assign(&db, "ana", "fazenda", "gestor", None).await.unwrap();
        assign(&db, "ana", "saude", "medico", None).await.unwrap();

        users::set_groups(&db, "ana", &["saude".into()], Some("root")).await.unwrap();
        delete_group(&db, "fazenda", Some("root")).await.unwrap();
        assert!(!list_definitions(&db).await.unwrap().contains_key("fazenda"));
        assert_eq!(roles_of_user(&db, "ana").await.unwrap(), pairs(&[("saude", "medico")]));
        // Even with an assignment left (e.g. a member whose rewrite failed),
        // delete_group leaves nothing behind.
        define(&db, "saude", "enfermeiro", None).await.unwrap();
        assign(&db, "ana", "saude", "enfermeiro", None).await.unwrap();
        delete_group(&db, "saude", None).await.unwrap();
        assert!(list_assignments(&db).await.unwrap().is_empty());
        assert!(audit_actions(&db)
            .await
            .iter()
            .any(|(a, _)| a == "group-role.delete-group"));
    }

    // The dual-dialect statements against a real Postgres. Gated on
    // `postgres-it` (see the workspace CLAUDE.md for the one-liner).
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn group_roles_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let db = ConfigDb::Postgres(crate::db::open_pg(&url).await.expect("open pg"));
        let tag = std::process::id();
        let (u, g, g2) = (
            format!("gr-it-{tag}"),
            format!("gr-it-group-{tag}"),
            format!("gr-it-renamed-{tag}"),
        );
        let _ = users::delete(&db, &u, None).await;
        let _ = delete_group(&db, &g, None).await;
        let _ = delete_group(&db, &g2, None).await;

        users::create(&db, &u, "Correct#Pass9", Role::Viewer, false, &[g.clone()], None)
            .await
            .unwrap();
        assert!(define(&db, &g, "gestor", Some("root")).await.unwrap());
        assert!(!define(&db, &g, "gestor", Some("root")).await.unwrap());
        assert!(define(&db, &g, "analista", Some("root")).await.unwrap());
        assert_eq!(assign(&db, &u, &g, "gestor", None).await.unwrap(), AssignOutcome::Assigned);
        assert_eq!(assign(&db, &u, &g, "gestor", None).await.unwrap(), AssignOutcome::AlreadyHeld);
        assert_eq!(assign(&db, &u, &g, "nope", None).await.unwrap(), AssignOutcome::UnknownRole);
        assert_eq!(assign(&db, &u, &g2, "gestor", None).await.unwrap(), AssignOutcome::NotMember);
        assert_eq!(assign(&db, &u, &g, "analista", None).await.unwrap(), AssignOutcome::Assigned);
        assert!(unassign(&db, &u, &g, "analista", None).await.unwrap());
        assert!(remove(&db, &g, "analista", None).await.unwrap());

        rename_group(&db, &g, &g2, Some("root")).await.unwrap();
        users::set_groups(&db, &u, &[g2.clone()], None).await.unwrap();
        assert_eq!(roles_of_user(&db, &u).await.unwrap(), vec![(g2.clone(), "gestor".to_string())]);

        // update_account prunes too.
        users::update_account(
            &db,
            &u,
            users::AccountUpdate { role: Role::Viewer, groups: &[], setor: None, email: None, celular: None },
            None,
        )
        .await
        .unwrap();
        assert!(roles_of_user(&db, &u).await.unwrap().is_empty());

        // User delete cascades.
        users::set_groups(&db, &u, &[g2.clone()], None).await.unwrap();
        assign(&db, &u, &g2, "gestor", None).await.unwrap();
        users::delete(&db, &u, None).await.unwrap();
        assert!(roles_of_user(&db, &u).await.unwrap().is_empty());

        delete_group(&db, &g2, None).await.unwrap();
        assert!(!list_definitions(&db).await.unwrap().contains_key(&g2));
    }
}
