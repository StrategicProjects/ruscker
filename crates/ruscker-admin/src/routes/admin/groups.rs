//! Admin > Groups (#503 read-only + #540 CRUD + #990 scoped membership).
//!
//! Groups in Ruscker are **derived**, not a first-class entity: a group is
//! any name that appears in a user's memberships or a spec's `access-groups`.
//! This page surfaces them — for each group, its member users and the apps it
//! gates. Admins may create, rename, delete, and change membership. Editors
//! may only add/remove non-Admin members of groups they themselves belong to.
//! Because there's no `groups` table, every edit rewrites the name across the
//! users and specs that reference it; a group exists exactly as long as
//! something points at it (adding the first member creates it; removing the
//! last reference makes it disappear).

use askama::Template;
use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

use crate::auth::{RequireAdmin, Role};
use crate::db::users::UserRow;
use crate::i18n::{Locale, Locales};
use crate::scope::EditorScope;
use crate::theme::Theme;
use crate::AppState;

use super::KpiMetric;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/groups", get(index))
        .route("/admin/groups/create", post(create))
        .route("/admin/groups/rename", post(rename))
        .route("/admin/groups/delete", post(delete))
        .route("/admin/groups/members/add", post(add_member))
        .route("/admin/groups/members/remove", post(remove_member))
        .route("/admin/groups/roles/define", post(define_role))
        .route("/admin/groups/roles/remove", post(remove_role))
        .route("/admin/groups/roles/assign", post(assign_role))
        .route("/admin/groups/roles/unassign", post(unassign_role))
}

/// One app reference inside a group — id, display name, plus the card
/// logo and display-type key so the chips can carry the catalog's
/// per-type tint (#809).
struct AppRef {
    id: String,
    name: String,
    logo: Option<String>,
    kind: &'static str,
}

impl AppRef {
    fn from_spec(s: &ruscker_config::Spec) -> Self {
        Self {
            id: s.id.clone(),
            name: s.display_name.clone().unwrap_or_else(|| s.id.clone()),
            logo: s.template_properties.get_str("logo").map(str::to_string),
            kind: crate::view_model::DisplayType::from_spec(s).key(),
        }
    }
}

/// A derived group with its members, the apps it gates and its group
/// roles (#1067).
struct GroupView {
    name: String,
    members: Vec<String>,
    apps: Vec<AppRef>,
    /// Roles defined in this group (sorted).
    roles: Vec<String>,
    /// Visible members that hold at least one role, with those roles.
    /// Built only from members already on this card, so an Editor never
    /// sees the roles of an account outside their scope.
    holders: Vec<HolderView>,
}

/// One member of a group card and the group roles they hold there.
struct HolderView {
    username: String,
    roles: Vec<String>,
}

#[derive(Template)]
#[template(path = "admin/groups.html")]
struct GroupsPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// Mount prefix for base-path-correct URLs (#294).
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    /// Current session role — drives nav and structural-action gating.
    role: Role,
    groups: Vec<GroupView>,
    kpi_groups: i64,
    kpi_members: i64,
    kpi_apps: i64,
    /// Apps with no `access-groups` — visible to everyone (#623). Shown in
    /// a dedicated "Public apps" rail so the page accounts for every spec,
    /// not only the gated ones.
    public_apps: Vec<AppRef>,
    /// Eligible usernames for the "add member" picker. Editors never receive
    /// Admin targets; unscoped Admins receive every account.
    all_users: Vec<String>,
    /// A scoped Editor's own (normalized) username: their row gets no
    /// group-role assign/unassign controls (#1067). `None` for Admins.
    self_target: Option<String>,
    flash: Option<String>,
}

impl GroupsPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// Whether `username` is the scoped Editor viewing the page.
    fn is_self(&self, username: &str) -> bool {
        self.self_target.as_deref() == Some(username)
    }

    fn kpis(&self) -> [KpiMetric; 3] {
        [
            KpiMetric::new("ti-users-group", "admin-groups-kpi-total", self.kpi_groups),
            KpiMetric::new("ti-user", "admin-groups-kpi-members", self.kpi_members),
            KpiMetric::new("ti-app-window", "admin-groups-kpi-apps", self.kpi_apps),
        ]
    }
}

#[derive(Deserialize)]
struct GroupsQuery {
    #[serde(default)]
    flash: Option<String>,
}

fn redirect_flash(flash: &str) -> Response {
    Redirect::to(&format!("/admin/groups?flash={flash}")).into_response()
}

async fn index(
    scope: EditorScope,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(q): Query<GroupsQuery>,
) -> Response {
    type GroupData = (BTreeSet<String>, BTreeSet<String>);

    // group name → (member usernames, app ids). BTree everywhere so the
    // page is deterministically ordered (groups, members, apps) with no dups.
    let mut map: BTreeMap<String, GroupData> = BTreeMap::new();
    let mut all_users: Vec<String> = Vec::new();

    // An Editor's own authoritative row is enough to establish each owned
    // group even if no app references it. A failed scope lookup yields an
    // empty list, so this remains fail-closed.
    if !scope.unscoped {
        for group in &scope.groups {
            map.entry(group.clone()).or_default();
        }
    }

    // Users contribute members and the add-member picker. Editors may add any
    // non-Admin account to an owned group, but Admin accounts are neither
    // exposed nor offered as mutation targets.
    if let Some(db) = state.db.as_ref() {
        match crate::db::users::list_all(db).await {
            Ok(users) => {
                for u in users {
                    if scope.unscoped || u.role != Role::Admin {
                        all_users.push(u.username.clone());
                    }
                    if !scope.may_touch_user(&u) {
                        continue;
                    }
                    for group in &u.groups {
                        if scope.may_touch_group(group) {
                            map.entry(group.clone())
                                .or_default()
                                .0
                                .insert(u.username.clone());
                        }
                    }
                }
            }
            Err(error) => {
                tracing::warn!(error = ?error, "group member lookup failed; using empty list");
            }
        }
    }
    all_users.sort();

    // Specs contribute apps (by `access-groups`). Resolve and authorize from
    // the effective authoritative catalog before inserting ids into the view;
    // a missing row can never be interpreted as an unrestricted app.
    let specs = crate::catalog::effective_specs_cached(&state).await;
    for spec in specs.iter().filter(|spec| scope.may_touch_spec(spec)) {
        if let Some(groups) = spec.access_groups.as_ref() {
            for group in groups {
                if scope.may_touch_group(group) {
                    map.entry(group.clone())
                        .or_default()
                        .1
                        .insert(spec.id.clone());
                }
            }
        }
    }

    // Group roles (#1067). A group that only has role definitions still
    // exists for the Admin (who can delete it); an Editor only ever sees the
    // groups already in their scope. A failed read renders no roles rather
    // than failing the page — nothing here grants access.
    let (mut definitions, assignments) = match state.db.as_ref() {
        Some(db) => {
            let defs = crate::db::group_roles::list_definitions(db).await;
            let held = crate::db::group_roles::list_assignments(db).await;
            if let Err(error) = defs.as_ref().and(held.as_ref()) {
                tracing::warn!(error = ?error, "group role lookup failed; showing none");
            }
            (defs.unwrap_or_default(), held.unwrap_or_default())
        }
        None => Default::default(),
    };
    if scope.unscoped {
        for group in definitions.keys() {
            map.entry(group.clone()).or_default();
        }
    }

    let groups: Vec<GroupView> = map
        .into_iter()
        .map(|(name, (members, app_ids))| {
            let roles = definitions.remove(&name).unwrap_or_default();
            let mut holders: BTreeMap<String, Vec<String>> = BTreeMap::new();
            for a in assignments.iter().filter(|a| a.group == name) {
                if members.contains(&a.username) && roles.contains(&a.role) {
                    holders.entry(a.username.clone()).or_default().push(a.role.clone());
                }
            }
            let holders: Vec<HolderView> = holders
                .into_iter()
                .map(|(username, roles)| HolderView { username, roles })
                .collect();
            (name, members, app_ids, roles, holders)
        })
        .map(|(name, members, app_ids, roles, holders)| GroupView {
            name,
            roles,
            holders,
            members: members.into_iter().collect(),
            apps: app_ids
                .into_iter()
                .filter_map(|id| {
                    specs
                        .iter()
                        .find(|spec| spec.id == id)
                        .map(AppRef::from_spec)
                })
                .collect(),
        })
        .collect();
    let kpi_groups = groups.len() as i64;
    let kpi_members = groups
        .iter()
        .flat_map(|group| group.members.iter().map(String::as_str))
        .collect::<BTreeSet<_>>()
        .len() as i64;
    let kpi_apps = groups
        .iter()
        .flat_map(|group| group.apps.iter().map(|app| app.id.as_str()))
        .collect::<BTreeSet<_>>()
        .len() as i64;

    // Truly public specs — open to everyone — need BOTH access-groups and
    // access-users empty (#623 audit: a users-only-gated spec is not public).
    let mut public_apps: Vec<AppRef> = specs
        .iter()
        .filter(|spec| scope.may_touch_spec(spec) && spec.is_open())
        .map(AppRef::from_spec)
        .collect();
    public_apps.sort_by(|a, b| a.name.cmp(&b.name));

    let page = GroupsPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "groups",
        role: scope.role,
        groups,
        kpi_groups,
        kpi_members,
        kpi_apps,
        public_apps,
        all_users,
        self_target: (!scope.unscoped)
            .then(|| scope.actor.as_deref().map(crate::db::users::normalize_username))
            .flatten(),
        flash: q.flash,
    };
    super::render(&page)
}

/// A group name is a free-form label; reject only what would break storage
/// (groups are stored comma-joined) or render the entry invisible.
fn clean_group(raw: &str) -> Option<String> {
    let g = raw.trim();
    if g.is_empty() || g.contains(',') {
        return None;
    }
    Some(g.to_string())
}

fn not_found() -> Response {
    StatusCode::NOT_FOUND.into_response()
}

/// Persist one membership list and invalidate the proxy identity cache only
/// after the authoritative write succeeds.
async fn save_memberships(
    state: &AppState,
    user: &UserRow,
    groups: &[String],
    actor: &str,
    success: &str,
) -> Response {
    let Some(db) = state.db.as_ref() else {
        return redirect_flash("bad-input");
    };
    match crate::db::users::set_groups(db, &user.username, groups, Some(actor)).await {
        Ok(()) => {
            state.invalidate_identity_cache();
            redirect_flash(success)
        }
        Err(error) => {
            tracing::warn!(
                user = %user.username,
                error = ?error,
                "group membership write failed"
            );
            redirect_flash("bad-input")
        }
    }
}

#[derive(Deserialize)]
struct MemberForm {
    group: String,
    username: String,
}

/// Creating a derived group means adding its first member. This remains a
/// distinct Admin-only handler so the membership endpoint opened to Editors
/// cannot grow a new authority boundary through its UI or route contract.
async fn create(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Form(form): Form<MemberForm>,
) -> Response {
    let Some(group) = clean_group(&form.group) else {
        return redirect_flash("bad-input");
    };
    let Some(db) = state.db.as_ref() else {
        return redirect_flash("bad-input");
    };
    match crate::db::users::fetch(db, &form.username).await {
        Ok(Some(user)) => {
            let mut groups = user.groups.clone();
            if !groups.iter().any(|existing| existing == &group) {
                groups.push(group);
            }
            save_memberships(&state, &user, &groups, admin.actor(), "created").await
        }
        _ => redirect_flash("bad-input"),
    }
}

/// Rewrite group `old` everywhere it's referenced. `new = Some(n)` renames it
/// (folding into `n` if a user/spec already has both); `new = None` deletes
/// it. Touches user memberships, spec `access-groups` and SSO client gates.
///
/// Returns `false` — having changed NOTHING — when a rename could not
/// move the group's roles first. Carrying on in that case would be
/// destructive: each member's `set_groups` prunes the roles they held
/// under the old name, so every holder would silently lose them while
/// the page reported success.
async fn rewrite_group(state: &AppState, old: &str, new: Option<&str>, actor: &str) -> bool {
    let Some(db) = state.db.as_ref() else { return false };

    // Group roles (#1067) move BEFORE the memberships: `set_groups` below
    // prunes the assignments of every group a user leaves, so on a rename
    // they must already live under the new name. On a delete the order is
    // reversed (prune per member first, then drop the definitions, below).
    if let Some(n) = new {
        if let Err(e) = crate::db::group_roles::rename_group(db, old, n, Some(actor)).await {
            tracing::error!(error = ?e, "group rename aborted: could not move its group roles");
            return false;
        }
    }

    if let Ok(users) = crate::db::users::list_all(db).await {
        for u in users {
            if !u.groups.iter().any(|g| g == old) {
                continue;
            }
            let mut groups: Vec<String> = u.groups.into_iter().filter(|g| g != old).collect();
            if let Some(n) = new {
                if !groups.iter().any(|g| g == n) {
                    groups.push(n.to_string());
                }
            }
            if let Err(e) = crate::db::users::set_groups(db, &u.username, &groups, Some(actor)).await
            {
                tracing::warn!(user = %u.username, error = ?e, "group rewrite (user) failed");
            }
        }
        // A rename/delete touches many memberships at once (#1001).
        state.invalidate_identity_cache();
    }

    if let Ok(specs) = crate::db::specs::list_all(db).await {
        for spec in specs {
            let Some(ag) = spec.access_groups.as_ref() else {
                continue;
            };
            if !ag.iter().any(|g| g == old) {
                continue;
            }
            let mut groups: Vec<String> = ag.iter().filter(|g| *g != old).cloned().collect();
            if let Some(n) = new {
                if !groups.iter().any(|g| g == n) {
                    groups.push(n.to_string());
                }
            }
            let mut spec = spec;
            // An empty `access-groups` becomes `None` (no group gate) rather
            // than an empty list — keeps the effective-access logic simple.
            spec.access_groups = if groups.is_empty() { None } else { Some(groups) };
            if let Err(e) = crate::db::specs::upsert_one(db, &spec, Some(actor)).await {
                tracing::warn!(spec = %spec.id, error = ?e, "group rewrite (spec) failed");
            }
        }
    }

    // SSO clients gated by this group follow the rename; a delete that
    // would empty a client's gate disables it instead of opening it
    // (see `db::oidc_clients::rewrite_group`).
    if let Err(e) = crate::db::oidc_clients::rewrite_group(db, old, new, Some(actor)).await {
        tracing::warn!(error = ?e, "group rewrite (sso clients) failed");
    }

    if new.is_none() {
        if let Err(e) = crate::db::group_roles::delete_group(db, old, Some(actor)).await {
            tracing::warn!(error = ?e, "group rewrite (group roles) failed");
        }
    }
    // Roles moved or vanished: the header must follow on the next request.
    state.invalidate_identity_cache();
    true
}

#[derive(Deserialize)]
struct RenameForm {
    old: String,
    new: String,
}

async fn rename(admin: RequireAdmin, State(state): State<AppState>, Form(f): Form<RenameForm>) -> Response {
    let (Some(old), Some(new)) = (clean_group(&f.old), clean_group(&f.new)) else {
        return redirect_flash("bad-input");
    };
    if old == new {
        return redirect_flash("renamed");
    }
    if rewrite_group(&state, &old, Some(&new), admin.actor()).await {
        redirect_flash("renamed")
    } else {
        redirect_flash("failed")
    }
}

#[derive(Deserialize)]
struct DeleteForm {
    name: String,
}

async fn delete(admin: RequireAdmin, State(state): State<AppState>, Form(f): Form<DeleteForm>) -> Response {
    let Some(name) = clean_group(&f.name) else {
        return redirect_flash("bad-input");
    };
    // A delete has no "move roles first" step, so it cannot abort.
    let _ = rewrite_group(&state, &name, None, admin.actor()).await;
    redirect_flash("deleted")
}

async fn add_member(
    scope: EditorScope,
    State(state): State<AppState>,
    Form(form): Form<MemberForm>,
) -> Response {
    let Some(group) = clean_group(&form.group) else {
        return redirect_flash("bad-input");
    };
    if !scope.may_touch_group(&group) {
        return not_found();
    }
    let Some(db) = state.db.as_ref() else {
        return redirect_flash("bad-input");
    };
    match crate::db::users::fetch(db, &form.username).await {
        Ok(Some(user)) => {
            let mut requested: Vec<String> = user
                .groups
                .iter()
                .filter(|existing| scope.may_touch_group(existing))
                .cloned()
                .collect();
            if !requested.iter().any(|existing| existing == &group) {
                requested.push(group);
            }
            let groups = scope.merge_preserving_out_of_scope(&user.groups, &requested);

            // `may_touch_user` normally evaluates the current membership. For
            // an add, evaluate the authoritative row plus the proposed owned
            // group: this permits inducting a non-Admin into the team while
            // retaining the same central predicate that excludes Admin.
            let mut proposed = user.clone();
            proposed.groups = groups.clone();
            if !scope.may_touch_user(&proposed) {
                return not_found();
            }
            save_memberships(&state, &user, &groups, scope.actor(), "member-added").await
        }
        _ => not_found(),
    }
}

async fn remove_member(
    scope: EditorScope,
    State(state): State<AppState>,
    Form(form): Form<MemberForm>,
) -> Response {
    let Some(group) = clean_group(&form.group) else {
        return redirect_flash("bad-input");
    };
    if !scope.may_touch_group(&group) {
        return not_found();
    }
    let Some(db) = state.db.as_ref() else {
        return redirect_flash("bad-input");
    };
    match crate::db::users::fetch(db, &form.username).await {
        Ok(Some(user)) if scope.may_touch_user(&user) => {
            // `set_groups` replaces the entire CSV. Feed only the remaining
            // in-scope memberships through the shared merge primitive so an
            // Editor removes this group surgically and preserves every
            // foreign-team membership.
            let requested: Vec<String> = user
                .groups
                .iter()
                .filter(|existing| {
                    existing.as_str() != group.as_str() && scope.may_touch_group(existing)
                })
                .cloned()
                .collect();
            let groups = scope.merge_preserving_out_of_scope(&user.groups, &requested);
            save_memberships(
                &state,
                &user,
                &groups,
                scope.actor(),
                "member-removed",
            )
            .await
        }
        _ => not_found(),
    }
}

// ── Group roles (#1067) ─────────────────────────────────────────────────
//
// Defining/removing a role changes the group's vocabulary, like rename or
// delete: Admin only. Assigning/unassigning is membership-level work, so an
// Editor may do it inside their own groups, with the same 404-not-403 rule
// as add/remove member. Every precondition is read from the authoritative
// rows (`users::fetch`, the group-roles tables), never from the page's view
// structs; `db::group_roles::assign` re-checks membership and definition in
// its own transaction.

#[derive(Deserialize)]
struct RoleForm {
    group: String,
    role: String,
}

#[derive(Deserialize)]
struct AssignForm {
    group: String,
    username: String,
    role: String,
}

/// Whether a group currently exists in the derived model: some user
/// belongs to it, some spec gates on it, or it already has roles. Fails
/// closed (`false`) when a lookup errors.
async fn group_exists(state: &AppState, group: &str) -> bool {
    let Some(db) = state.db.as_ref() else {
        return false;
    };
    match crate::db::group_roles::list_definitions(db).await {
        Ok(defs) if defs.contains_key(group) => return true,
        Ok(_) => {}
        Err(_) => return false,
    }
    match crate::db::users::list_all(db).await {
        Ok(users) if users.iter().any(|u| u.groups.iter().any(|g| g == group)) => return true,
        Ok(_) => {}
        Err(_) => return false,
    }
    crate::catalog::effective_specs_cached(state)
        .await
        .iter()
        .any(|spec| {
            spec.access_groups
                .as_deref()
                .is_some_and(|groups| groups.iter().any(|g| g == group))
        })
}

async fn define_role(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Form(form): Form<RoleForm>,
) -> Response {
    let Some(group) = clean_group(&form.group) else {
        return redirect_flash("bad-input");
    };
    let Some(role) = crate::db::group_roles::clean_role(&form.role) else {
        return redirect_flash("bad-role");
    };
    // Defining a role must not conjure a group out of a typo.
    if !group_exists(&state, &group).await {
        return not_found();
    }
    let Some(db) = state.db.as_ref() else {
        return redirect_flash("bad-input");
    };
    match crate::db::group_roles::define(db, &group, &role, Some(admin.actor())).await {
        Ok(_) => redirect_flash("role-added"),
        Err(error) => {
            tracing::warn!(group = %group, error = ?error, "group role define failed");
            redirect_flash("bad-input")
        }
    }
}

async fn remove_role(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Form(form): Form<RoleForm>,
) -> Response {
    let (Some(group), Some(role)) = (
        clean_group(&form.group),
        crate::db::group_roles::clean_role(&form.role),
    ) else {
        return redirect_flash("bad-input");
    };
    let Some(db) = state.db.as_ref() else {
        return redirect_flash("bad-input");
    };
    match crate::db::group_roles::remove(db, &group, &role, Some(admin.actor())).await {
        Ok(true) => {
            // Holders lost the role: the next proxied request must not
            // still carry it.
            state.invalidate_identity_cache();
            redirect_flash("role-removed")
        }
        Ok(false) => not_found(),
        Err(error) => {
            tracing::warn!(group = %group, error = ?error, "group role remove failed");
            redirect_flash("bad-input")
        }
    }
}

/// Whether a scoped Editor is targeting their own account. Same rule as
/// `users.rs::is_scoped_self_edit`, compared on the normalized username:
/// group roles are authorization data inside the hosted apps, so an Editor
/// granting their own would be self-escalation there (and, for symmetry,
/// they cannot unassign their own either — though leaving the group, which
/// only ever REDUCES what they hold, remains possible). An
/// unscoped Admin is not restricted.
fn is_scoped_self_target(scope: &EditorScope, username: &str) -> bool {
    !scope.unscoped
        && scope
            .actor
            .as_deref()
            .is_some_and(|actor| crate::db::users::normalize_username(actor) == username)
}

/// Shared authorization for assign/unassign. Returns the authoritative
/// user row only when the caller owns the group, may manage the user, is
/// not a scoped Editor acting on themselves, and the user really belongs
/// to the group; anything else is `None` → 404.
async fn authorize_role_target(
    scope: &EditorScope,
    state: &AppState,
    group: &str,
    username: &str,
) -> Option<UserRow> {
    if !scope.may_touch_group(group) {
        return None;
    }
    let db = state.db.as_ref()?;
    let user = crate::db::users::fetch(db, username).await.ok()??;
    if is_scoped_self_target(scope, &user.username) {
        return None;
    }
    (scope.may_touch_user(&user) && user.groups.iter().any(|g| g == group)).then_some(user)
}

async fn assign_role(
    scope: EditorScope,
    State(state): State<AppState>,
    Form(form): Form<AssignForm>,
) -> Response {
    let (Some(group), Some(role)) = (
        clean_group(&form.group),
        crate::db::group_roles::clean_role(&form.role),
    ) else {
        return redirect_flash("bad-input");
    };
    let Some(user) = authorize_role_target(&scope, &state, &group, &form.username).await else {
        return not_found();
    };
    let Some(db) = state.db.as_ref() else {
        return redirect_flash("bad-input");
    };
    use crate::db::group_roles::AssignOutcome;
    match crate::db::group_roles::assign(db, &user.username, &group, &role, Some(scope.actor()))
        .await
    {
        Ok(AssignOutcome::Assigned) => {
            state.invalidate_identity_cache();
            redirect_flash("role-assigned")
        }
        Ok(AssignOutcome::AlreadyHeld) => redirect_flash("role-assigned"),
        // The role is not defined in this group (or membership changed in
        // between): nothing to point at.
        Ok(AssignOutcome::UnknownRole | AssignOutcome::NotMember) => not_found(),
        Err(error) => {
            tracing::warn!(user = %user.username, error = ?error, "group role assign failed");
            redirect_flash("bad-input")
        }
    }
}

async fn unassign_role(
    scope: EditorScope,
    State(state): State<AppState>,
    Form(form): Form<AssignForm>,
) -> Response {
    let (Some(group), Some(role)) = (
        clean_group(&form.group),
        crate::db::group_roles::clean_role(&form.role),
    ) else {
        return redirect_flash("bad-input");
    };
    let Some(user) = authorize_role_target(&scope, &state, &group, &form.username).await else {
        return not_found();
    };
    let Some(db) = state.db.as_ref() else {
        return redirect_flash("bad-input");
    };
    match crate::db::group_roles::unassign(db, &user.username, &group, &role, Some(scope.actor()))
        .await
    {
        Ok(true) => {
            state.invalidate_identity_cache();
            redirect_flash("role-unassigned")
        }
        Ok(false) => not_found(),
        Err(error) => {
            tracing::warn!(user = %user.username, error = ?error, "group role unassign failed");
            redirect_flash("bad-input")
        }
    }
}
