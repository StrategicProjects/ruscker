//! User-account management — Admin only.
//!
//! Create/edit/delete the per-user accounts that back password login
//! (see [`crate::db::users`]). Guarded by [`RequireAdmin`]; the nav
//! link is hidden for non-admins. A **last-admin guard** prevents
//! deleting or demoting the only remaining admin, so an operator can't
//! lock the role out of the portal.

use askama::Template;
use axum::{
    extract::{Form, Multipart, Path, Query, State},
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
        .route("/admin/users/import", post(import))
        .route("/admin/users/import/confirm", post(import_confirm))
        .route("/admin/users/{username}/edit", get(edit).post(save_edit))
        .route("/admin/users/{username}/role", post(set_role))
        .route("/admin/users/{username}/groups", post(set_groups))
        .route("/admin/users/{username}/profile", post(set_profile))
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
    /// Server-side pagination state (#999): 1-based current page,
    /// total page count, and how many users match the search.
    page: i64,
    pages: i64,
    total: i64,
    /// The active search term, echoed back into the search box and
    /// carried on the pager links; `""` when unfiltered.
    q: String,
    /// Username of the logged-in admin — flags the "you" row.
    me: String,
    /// "" | "saved" | "created" | "deleted" | "last-admin" | "bad-input"
    /// | "weak-password" | "exists" | "imported"
    flash: &'static str,
    /// `(imported, skipped)` counts for a CSV import summary (#862),
    /// shown when `flash == "imported"`.
    import_summary: Option<(usize, usize)>,
}

impl UsersPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// Localized "Page X of Y · N users" line under the table (#999).
    fn pager_status(&self) -> String {
        use fluent_bundle::FluentArgs;
        let mut args = FluentArgs::new();
        args.set("page", self.page);
        args.set("pages", self.pages);
        args.set("total", self.total);
        self.locales
            .t(self.locale, "admin-users-pager-status", Some(&args))
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

#[derive(Template)]
#[template(path = "admin/user_edit.html")]
struct UserEditPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    user: db::users::UserRow,
    me: String,
    /// "" | "saved" | "last-admin" | "bad-input" | "weak-password"
    flash: &'static str,
}

impl UserEditPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    fn role_label(&self, role: &Role) -> String {
        self.t(role.label_key())
    }

    fn all_roles(&self) -> [Role; 3] {
        [Role::Viewer, Role::Editor, Role::Admin]
    }
}

/// Rows per page on the users table (#999). The page paginates
/// server-side so a large user base (e.g. a big CSV import) doesn't
/// fetch and render thousands of rows per view.
const USERS_PER_PAGE: i64 = 50;

#[derive(Debug, Deserialize)]
pub struct UsersQuery {
    pub flash: Option<String>,
    /// CSV-import summary counts (#862), set on the post-import redirect.
    #[serde(default)]
    pub n: Option<usize>,
    #[serde(default)]
    pub skipped: Option<usize>,
    /// 1-based page of the users table (#999); out-of-range clamps.
    #[serde(default)]
    pub page: Option<i64>,
    /// Server-side search term (#999) — substring over username,
    /// groups and the profile fields.
    #[serde(default)]
    pub q: Option<String>,
}

fn redirect_flash(flash: &str) -> Response {
    Redirect::to(&format!("/admin/users?flash={flash}")).into_response()
}

fn redirect_edit_flash(username: &str, flash: &str) -> Response {
    Redirect::to(&format!("/admin/users/{username}/edit?flash={flash}")).into_response()
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
    // Server-side pagination + search (#999): count first, clamp the
    // requested page into range, then fetch only that page's rows.
    let search = q.q.as_deref().unwrap_or("").trim().to_string();
    let total = match db::users::count_filtered(pool, &search).await {
        Ok(n) => n,
        Err(e) => {
            tracing::error!(error = ?e, "count users failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    // Ceiling division (i64::div_ceil is still unstable on our floor).
    let pages = ((total + USERS_PER_PAGE - 1) / USERS_PER_PAGE).max(1);
    let page = q.page.unwrap_or(1).clamp(1, pages);
    let users = match db::users::list_page(pool, &search, USERS_PER_PAGE, (page - 1) * USERS_PER_PAGE)
        .await
    {
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
        Some("weak-password") => "weak-password",
        Some("exists") => "exists",
        Some("imported") => "imported",
        _ => "",
    };
    let import_summary = (flash == "imported")
        .then(|| (q.n.unwrap_or(0), q.skipped.unwrap_or(0)));
    let page = UsersPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "users",
        role: admin.role,
        users,
        page,
        pages,
        total,
        q: search,
        me: admin.actor().to_string(),
        flash,
        import_summary,
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
    /// Optional profile fields (#856) — blank ⇒ unset.
    #[serde(default)]
    pub setor: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub celular: String,
}

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
    if !db::users::is_valid_username(&username) {
        return redirect_flash("bad-input");
    }
    if !crate::auth::password_meets_policy(&form.password) {
        return redirect_flash("weak-password");
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
        Ok(()) => {
            // Optional profile fields (#856): only touch them (and emit
            // an audit row) when the operator actually filled one in.
            if [&form.setor, &form.email, &form.celular]
                .iter()
                .any(|s| !s.trim().is_empty())
            {
                if let Err(e) = db::users::update_profile(
                    pool,
                    &username,
                    Some(&form.setor),
                    Some(&form.email),
                    Some(&form.celular),
                    Some(admin.actor()),
                )
                .await
                {
                    tracing::warn!(error = ?e, %username, "set profile on create failed");
                }
            }
            redirect_flash("created")
        }
        Err(e) => {
            tracing::warn!(error = ?e, %username, "create user failed");
            redirect_flash("exists")
        }
    }
}

#[derive(Debug, Deserialize)]
struct EditQuery {
    flash: Option<String>,
}

async fn edit(
    admin: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Path(username): Path<String>,
    Query(q): Query<EditQuery>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let username = db::users::normalize_username(&username);
    if !db::users::is_valid_username(&username) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let user = match db::users::fetch(pool, &username).await {
        Ok(Some(user)) => user,
        Ok(None) => return StatusCode::NOT_FOUND.into_response(),
        Err(e) => {
            tracing::error!(error = ?e, %username, "fetch user for edit failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let flash = match q.flash.as_deref() {
        Some("saved") => "saved",
        Some("last-admin") => "last-admin",
        Some("bad-input") => "bad-input",
        Some("weak-password") => "weak-password",
        _ => "",
    };
    super::render(&UserEditPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "users",
        role: admin.role,
        user,
        me: admin.actor().to_string(),
        flash,
    })
}

#[derive(Debug, Deserialize)]
struct EditForm {
    role: String,
    #[serde(default)]
    groups: String,
    #[serde(default)]
    setor: String,
    #[serde(default)]
    email: String,
    #[serde(default)]
    celular: String,
}

async fn save_edit(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Path(username): Path<String>,
    Form(form): Form<EditForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let username = db::users::normalize_username(&username);
    if !db::users::is_valid_username(&username) {
        return redirect_flash("bad-input");
    }
    let Some(role) = Role::parse(&form.role) else {
        return redirect_edit_flash(&username, "bad-input");
    };
    let current = match db::users::fetch(pool, &username).await {
        Ok(Some(user)) => user,
        _ => return redirect_edit_flash(&username, "bad-input"),
    };
    if would_strip_last_admin(pool, &username, Some(role)).await {
        return redirect_edit_flash(&username, "last-admin");
    }

    let groups = db::users::parse_groups(&form.groups);
    let update = db::users::AccountUpdate {
        role,
        groups: &groups,
        setor: Some(&form.setor),
        email: Some(&form.email),
        celular: Some(&form.celular),
    };
    match db::users::update_account(pool, &username, update, Some(admin.actor())).await {
        Ok(()) => {
            if current.role != role {
                // Role changes take effect immediately, including self-demotion.
                state.admin_sessions.revoke_by_actor(&username).await;
            }
            redirect_flash("saved")
        }
        Err(e) => {
            tracing::warn!(error = ?e, %username, "consolidated user update failed");
            if would_strip_last_admin(pool, &username, Some(role)).await {
                redirect_edit_flash(&username, "last-admin")
            } else {
                redirect_edit_flash(&username, "bad-input")
            }
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ProfileForm {
    #[serde(default)]
    pub setor: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub celular: String,
}

async fn set_profile(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Path(username): Path<String>,
    Form(form): Form<ProfileForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    match db::users::update_profile(
        pool,
        &username,
        Some(&form.setor),
        Some(&form.email),
        Some(&form.celular),
        Some(admin.actor()),
    )
    .await
    {
        Ok(()) => redirect_flash("saved"),
        Err(e) => {
            tracing::warn!(error = ?e, %username, "set profile failed");
            redirect_flash("bad-input")
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
    if !crate::auth::password_meets_policy(&form.password) {
        return redirect_edit_flash(&db::users::normalize_username(&username), "weak-password");
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

// ── CSV bulk import (#862) ───────────────────────────────────────────

/// One parsed CSV row + its validation outcome. `error` ⇒ the row can't
/// be imported (bad username/password/role); `exists` ⇒ the username is
/// already taken (skipped, never overwritten).
struct CsvRow {
    line_no: usize,
    username: String,
    role: Role,
    password: String,
    groups: Vec<String>,
    setor: String,
    email: String,
    celular: String,
    error: Option<&'static str>,
    exists: bool,
}

impl CsvRow {
    /// Will this row actually create a user on confirm?
    fn importable(&self) -> bool {
        self.error.is_none() && !self.exists
    }

    /// Fluent key for the row's status chip.
    fn status_key(&self) -> &'static str {
        match (self.error, self.exists) {
            (Some(e), _) => match e {
                "bad-username" => "admin-users-import-status-bad-username",
                "bad-password" => "admin-users-import-status-bad-password",
                _ => "admin-users-import-status-bad-role",
            },
            (None, true) => "admin-users-import-status-exists",
            (None, false) => "admin-users-import-status-ok",
        }
    }
}

/// Split one CSV line into fields (RFC4180-ish, single-line records — a
/// user-admin CSV never has newlines inside a field). Handles
/// double-quoted fields, `""` escapes, and commas inside quotes.
fn split_csv_line(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut in_q = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        if in_q {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    cur.push('"');
                    chars.next();
                } else {
                    in_q = false;
                }
            } else {
                cur.push(c);
            }
        } else {
            match c {
                '"' => in_q = true,
                ',' => out.push(std::mem::take(&mut cur)),
                _ => cur.push(c),
            }
        }
    }
    out.push(cur);
    out.into_iter().map(|s| s.trim().to_string()).collect()
}

/// Parse a CSV body into validated rows. The first non-empty,
/// non-`#`-comment line is the header; columns are matched by name
/// (case-insensitive): `username` (required), `role`, `password`,
/// `groups`, `setor`, `email`, `celular`. `Err` only when the header is
/// unusable (no `username` column); per-row problems ride on `error`.
fn parse_csv_users(body: &str) -> Result<Vec<CsvRow>, &'static str> {
    let mut lines = body
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
    let (_, header_line) = lines.next().ok_or("empty-csv")?;
    let header: Vec<String> = split_csv_line(header_line)
        .into_iter()
        .map(|h| h.to_lowercase())
        .collect();
    let col = |name: &str| header.iter().position(|h| h == name);
    let Some(i_user) = col("username") else {
        return Err("no-username-column");
    };
    let (i_role, i_pass, i_groups, i_setor, i_email, i_cel) = (
        col("role"),
        col("password"),
        col("groups"),
        col("setor"),
        col("email"),
        col("celular"),
    );
    let get = |f: &[String], idx: Option<usize>| -> String {
        idx.and_then(|i| f.get(i)).cloned().unwrap_or_default()
    };

    let mut rows = Vec::new();
    for (line_no, line) in lines {
        let f = split_csv_line(line);
        let username = db::users::normalize_username(&get(&f, Some(i_user)));
        let password = get(&f, i_pass);
        let role_raw = get(&f, i_role);
        let role = Role::parse(&role_raw.to_lowercase()).unwrap_or(Role::Viewer);
        // In a CSV the comma is the field delimiter, so groups inside a
        // single field are separated by `;` — normalize to `,` so the
        // shared `parse_groups` splits them.
        let groups = db::users::parse_groups(&get(&f, i_groups).replace(';', ","));

        let error = if !db::users::is_valid_username(&username) {
            Some("bad-username")
        } else if !crate::auth::password_meets_policy(&password) {
            Some("bad-password")
        } else if !role_raw.is_empty() && Role::parse(&role_raw.to_lowercase()).is_none() {
            Some("bad-role")
        } else {
            None
        };

        rows.push(CsvRow {
            line_no: line_no + 1,
            username,
            role,
            password,
            groups,
            setor: get(&f, i_setor),
            email: get(&f, i_email),
            celular: get(&f, i_cel),
            error,
            exists: false,
        });
    }
    Ok(rows)
}

#[derive(Template)]
#[template(path = "admin/users_import_preview.html")]
struct UsersImportPreviewPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    rows: Vec<CsvRow>,
    /// The uploaded CSV, carried in a hidden field so confirm re-parses
    /// the same content (no second upload).
    raw_csv: String,
    /// How many rows will actually be created on confirm.
    importable: usize,
}

impl UsersImportPreviewPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
    fn role_label(&self, role: &Role) -> String {
        self.t(role.label_key())
    }
}

/// `POST /admin/users/import` — multipart CSV upload → parse → preview
/// (no writes). The valid rows are committed only by `import_confirm`.
async fn import(
    admin: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    mut multipart: Multipart,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let mut raw: Option<String> = None;
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) => {
                if field.name() != Some("file") {
                    continue;
                }
                match field.bytes().await {
                    Ok(b) if !b.is_empty() => raw = Some(String::from_utf8_lossy(&b).into_owned()),
                    Ok(_) => {}
                    Err(_) => return redirect_flash("bad-input"),
                }
            }
            Ok(None) => break,
            Err(_) => return redirect_flash("bad-input"),
        }
    }
    let Some(raw) = raw else {
        return redirect_flash("bad-input");
    };
    let mut rows = match parse_csv_users(&raw) {
        Ok(r) => r,
        Err(_) => return redirect_flash("bad-input"),
    };
    // Flag already-existing usernames (skipped, never overwritten).
    for r in &mut rows {
        if r.error.is_none() && matches!(db::users::fetch(pool, &r.username).await, Ok(Some(_))) {
            r.exists = true;
        }
    }
    let importable = rows.iter().filter(|r| r.importable()).count();
    let page = UsersImportPreviewPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "users",
        role: admin.role,
        rows,
        raw_csv: raw,
        importable,
    };
    super::render(&page)
}

#[derive(Debug, Deserialize)]
pub struct ImportConfirmForm {
    pub raw_csv: String,
}

/// `POST /admin/users/import/confirm` — re-parse the previewed CSV and
/// create the importable rows. New accounts get `must_change_password`
/// so the imported initial password is changed on first login. Existing
/// usernames + invalid rows are skipped (create is fail-closed on a
/// duplicate anyway). Redirects with an `imported`/`skipped` summary.
async fn import_confirm(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Form(form): Form<ImportConfirmForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let rows = match parse_csv_users(&form.raw_csv) {
        Ok(r) => r,
        Err(_) => return redirect_flash("bad-input"),
    };
    let mut imported = 0usize;
    let mut skipped = 0usize;
    for r in &rows {
        if r.error.is_some() {
            skipped += 1;
            continue;
        }
        match db::users::create(
            pool,
            &r.username,
            &r.password,
            r.role,
            true,
            &r.groups,
            Some(admin.actor()),
        )
        .await
        {
            Ok(()) => {
                if !r.setor.is_empty() || !r.email.is_empty() || !r.celular.is_empty() {
                    // #875: don't silently swallow a profile-write failure.
                    // The user was created but the profile didn't take →
                    // compensate by removing the half-created account and
                    // counting the row as skipped, so create+profile is
                    // effectively all-or-nothing. (A CSV row is never the
                    // last admin, so the delete guard can't block this.)
                    if let Err(e) = db::users::update_profile(
                        pool,
                        &r.username,
                        Some(&r.setor),
                        Some(&r.email),
                        Some(&r.celular),
                        Some(admin.actor()),
                    )
                    .await
                    {
                        tracing::warn!(user = %r.username, error = ?e, "csv import: profile write failed; rolling back the user");
                        let _ = db::users::delete(pool, &r.username, Some(admin.actor())).await;
                        skipped += 1;
                        continue;
                    }
                }
                imported += 1;
            }
            // Duplicate (raced in since preview) or a DB error — skip,
            // never overwrite an existing account.
            Err(_) => skipped += 1,
        }
    }
    Redirect::to(&format!(
        "/admin/users?flash=imported&n={imported}&skipped={skipped}"
    ))
    .into_response()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_quoted_csv_fields() {
        assert_eq!(split_csv_line("a,b,c"), vec!["a", "b", "c"]);
        // Comma + semicolons inside quotes stay in one field.
        assert_eq!(
            split_csv_line(r#"alice,editor,"x,y","g1;g2""#),
            vec!["alice", "editor", "x,y", "g1;g2"]
        );
        // `""` is an escaped quote.
        assert_eq!(
            split_csv_line(r#""he said ""hi""",z"#),
            vec![r#"he said "hi""#, "z"]
        );
    }

    #[test]
    fn parses_csv_users_with_validation() {
        let csv = "username,role,password,groups,setor\n\
                   alice,editor,Alice!pass1,\"a;b\",GAPE\n\
                   bad user,viewer,short,,\n";
        let rows = parse_csv_users(csv).unwrap();
        assert_eq!(rows.len(), 2);
        // Valid row: role parsed, groups split, profile carried.
        assert!(rows[0].error.is_none());
        assert!(rows[0].importable());
        assert_eq!(rows[0].role, Role::Editor);
        assert_eq!(rows[0].groups, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(rows[0].setor, "GAPE");
        // Invalid row: space in username (and short password) ⇒ error.
        assert!(rows[1].error.is_some());
        assert!(!rows[1].importable());

        // Blank role defaults to Viewer.
        let r = parse_csv_users("username,password\nbob,Bob!pass12\n").unwrap();
        assert_eq!(r[0].role, Role::Viewer);
        assert!(r[0].error.is_none());

        // No `username` column ⇒ header error.
        assert!(parse_csv_users("role,password\neditor,x").is_err());
    }
}
