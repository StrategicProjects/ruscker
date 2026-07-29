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

/// A derived group with its members and the apps it gates.
struct GroupView {
    name: String,
    members: Vec<String>,
    apps: Vec<AppRef>,
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
    flash: Option<String>,
}

impl GroupsPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
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

    let groups: Vec<GroupView> = map
        .into_iter()
        .map(|(name, (members, app_ids))| GroupView {
            name,
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
/// it. Touches both user memberships and spec `access-groups`.
async fn rewrite_group(state: &AppState, old: &str, new: Option<&str>, actor: &str) {
    let Some(db) = state.db.as_ref() else { return };

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
    rewrite_group(&state, &old, Some(&new), admin.actor()).await;
    redirect_flash("renamed")
}

#[derive(Deserialize)]
struct DeleteForm {
    name: String,
}

async fn delete(admin: RequireAdmin, State(state): State<AppState>, Form(f): Form<DeleteForm>) -> Response {
    let Some(name) = clean_group(&f.name) else {
        return redirect_flash("bad-input");
    };
    rewrite_group(&state, &name, None, admin.actor()).await;
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
