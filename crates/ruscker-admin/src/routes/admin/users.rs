//! User-account management — Admin only.
//!
//! Create/edit/delete the per-user accounts that back password login
//! (see [`crate::db::users`]). Guarded by [`RequireAdmin`]; the nav
//! link is hidden for non-admins. A **last-admin guard** prevents
//! deleting or demoting the only remaining admin, so an operator can't
//! lock the role out of the portal.

use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;

use crate::auth::{RequireAdmin, Role};
use crate::db;
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/users", get(index).post(create))
        .route("/admin/users/{username}/role", post(set_role))
        .route("/admin/users/{username}/groups", post(set_groups))
        .route("/admin/users/{username}/password", post(reset_password))
        .route("/admin/users/{username}/delete", post(delete))
}

#[derive(Template)]
#[template(path = "admin/users.html")]
struct UsersPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// Mount prefix for base-path-correct URLs (#294).
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    users: Vec<db::users::UserRow>,
    /// Username of the logged-in admin — flags the "you" row.
    me: String,
    /// "" | "saved" | "created" | "deleted" | "last-admin" | "bad-input" | "exists"
    flash: &'static str,
}

impl UsersPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// Human label for a role (Fluent).
    fn role_label(&self, role: &Role) -> String {
        self.t(role.label_key())
    }

    /// The three roles, for the create/edit selectors.
    fn all_roles(&self) -> [Role; 3] {
        [Role::Viewer, Role::Editor, Role::Admin]
    }
}

#[derive(Debug, Deserialize)]
pub struct UsersQuery {
    pub flash: Option<String>,
}

fn redirect_flash(flash: &str) -> Response {
    Redirect::to(&format!("/admin/users?flash={flash}")).into_response()
}

async fn index(
    admin: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(q): Query<UsersQuery>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "database not attached — start with --db <path>",
        )
            .into_response();
    };
    let users = match db::users::list_all(pool).await {
        Ok(u) => u,
        Err(e) => {
            tracing::error!(error = ?e, "list users failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let flash = match q.flash.as_deref() {
        Some("saved") => "saved",
        Some("created") => "created",
        Some("deleted") => "deleted",
        Some("last-admin") => "last-admin",
        Some("bad-input") => "bad-input",
        Some("exists") => "exists",
        _ => "",
    };
    let page = UsersPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "users",
        role: admin.role,
        users,
        me: admin.actor().to_string(),
        flash,
    };
    super::render(&page)
}

#[derive(Debug, Deserialize)]
pub struct CreateForm {
    pub username: String,
    pub password: String,
    pub role: String,
    /// Comma-separated group names; canonicalized in `db::users`.
    #[serde(default)]
    pub groups: String,
}

const MIN_PASSWORD_LEN: usize = 8;

async fn create(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Form(form): Form<CreateForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let username = db::users::normalize_username(&form.username);
    let role = Role::parse(&form.role).unwrap_or(Role::Viewer);
    if !db::users::is_valid_username(&username) || form.password.len() < MIN_PASSWORD_LEN {
        return redirect_flash("bad-input");
    }
    let groups = db::users::parse_groups(&form.groups);
    // New accounts get the "change your password?" prompt on first login.
    match db::users::create(
        pool,
        &username,
        &form.password,
        role,
        true,
        &groups,
        Some(admin.actor()),
    )
    .await
    {
        Ok(()) => redirect_flash("created"),
        Err(e) => {
            tracing::warn!(error = ?e, %username, "create user failed");
            redirect_flash("exists")
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct GroupsForm {
    /// Comma-separated group names; canonicalized in `db::users`.
    #[serde(default)]
    pub groups: String,
}

async fn set_groups(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Path(username): Path<String>,
    Form(form): Form<GroupsForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let groups = db::users::parse_groups(&form.groups);
    match db::users::set_groups(pool, &username, &groups, Some(admin.actor())).await {
        Ok(()) => redirect_flash("saved"),
        Err(e) => {
            tracing::warn!(error = ?e, %username, "set groups failed");
            redirect_flash("bad-input")
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct RoleForm {
    pub role: String,
}

async fn set_role(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Path(username): Path<String>,
    Form(form): Form<RoleForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let new_role = Role::parse(&form.role).unwrap_or(Role::Viewer);

    // Last-admin guard: don't let the only admin be demoted.
    if would_strip_last_admin(pool, &username, Some(new_role)).await {
        return redirect_flash("last-admin");
    }
    match db::users::set_role(pool, &username, new_role, Some(admin.actor())).await {
        Ok(()) => {
            // Kick the user's live sessions so the new role takes effect
            // now, not after the cookie expires (#544) — a demotion must
            // drop elevated access immediately.
            state.admin_sessions.revoke_by_actor(&username).await;
            redirect_flash("saved")
        }
        Err(e) => {
            tracing::warn!(error = ?e, %username, "set role failed");
            redirect_flash("bad-input")
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ResetForm {
    pub password: String,
}

async fn reset_password(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Path(username): Path<String>,
    Form(form): Form<ResetForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    if form.password.len() < MIN_PASSWORD_LEN {
        return redirect_flash("bad-input");
    }
    // Admin-assigned password ⇒ prompt the user to change it next login.
    match db::users::set_password(pool, &username, &form.password, true, Some(admin.actor())).await
    {
        Ok(()) => {
            // A password reset must invalidate existing sessions (#544).
            state.admin_sessions.revoke_by_actor(&username).await;
            redirect_flash("saved")
        }
        Err(e) => {
            tracing::warn!(error = ?e, %username, "reset password failed");
            redirect_flash("bad-input")
        }
    }
}

async fn delete(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Path(username): Path<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    // Last-admin guard: deleting the only admin would lock everyone out.
    if would_strip_last_admin(pool, &username, None).await {
        return redirect_flash("last-admin");
    }
    match db::users::delete(pool, &username, Some(admin.actor())).await {
        Ok(()) => {
            // A deleted user must lose access now, not at session expiry (#544).
            state.admin_sessions.revoke_by_actor(&username).await;
            redirect_flash("deleted")
        }
        Err(e) => {
            tracing::warn!(error = ?e, %username, "delete user failed");
            redirect_flash("bad-input")
        }
    }
}

/// Would changing `username`'s role to `new_role` (or deleting it when
/// `new_role` is `None`) leave the portal with zero admins? True ⇒ the
/// caller must refuse.
async fn would_strip_last_admin(
    pool: &crate::db::ConfigDb,
    username: &str,
    new_role: Option<Role>,
) -> bool {
    // Only relevant if the target is currently an admin.
    let target = match db::users::fetch(pool, username).await {
        Ok(Some(u)) => u,
        _ => return false,
    };
    if target.role != Role::Admin {
        return false;
    }
    // Staying admin is always fine.
    if new_role == Some(Role::Admin) {
        return false;
    }
    // Removing/demoting an admin is only a problem when it's the last.
    db::users::count_admins(pool).await.unwrap_or(0) <= 1
}
