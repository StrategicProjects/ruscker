//! Admin > Groups (#503 read-only + #540 CRUD).
//!
//! Groups in Ruscker are **derived**, not a first-class entity: a group is
//! any name that appears in a user's memberships or a spec's `access-groups`.
//! This page surfaces them — for each group, its member users and the apps it
//! gates — and lets an Admin **rename**, **delete**, and add/remove members.
//! Because there's no `groups` table, every edit rewrites the name across the
//! users and specs that reference it; a group exists exactly as long as
//! something points at it (adding the first member creates it; removing the
//! last reference makes it disappear).

use askama::Template;
use axum::{
    extract::{Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

use crate::auth::{RequireAdmin, Role};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/groups", get(index))
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
    /// Current session role (always Admin here) — drives nav gating.
    role: Role,
    groups: Vec<GroupView>,
    /// Apps with no `access-groups` — visible to everyone (#623). Shown in
    /// a dedicated "Public apps" rail so the page accounts for every spec,
    /// not only the gated ones.
    public_apps: Vec<AppRef>,
    /// All usernames, for the "add member" picker.
    all_users: Vec<String>,
    flash: Option<String>,
}

impl GroupsPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
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
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(q): Query<GroupsQuery>,
) -> Response {
    // group name → (member usernames, apps id→name). BTree everywhere so the
    // page is deterministically ordered (groups, members, apps) with no dups.
    let mut map: BTreeMap<String, (BTreeSet<String>, BTreeMap<String, String>)> = BTreeMap::new();
    let mut all_users: Vec<String> = Vec::new();

    // Users contribute members.
    if let Some(db) = state.db.as_ref() {
        if let Ok(users) = crate::db::users::list_all(db).await {
            for u in users {
                all_users.push(u.username.clone());
                for g in u.groups {
                    map.entry(g).or_default().0.insert(u.username.clone());
                }
            }
        }
    }
    all_users.sort();

    // Specs contribute apps (by `access-groups`).
    let specs = crate::catalog::effective_specs(state.db.as_ref(), &state.config).await;
    for s in &specs {
        if let Some(groups) = s.access_groups.as_ref() {
            let name = s.display_name.clone().unwrap_or_else(|| s.id.clone());
            for g in groups {
                map.entry(g.clone())
                    .or_default()
                    .1
                    .insert(s.id.clone(), name.clone());
            }
        }
    }

    let groups = map
        .into_iter()
        .map(|(name, (members, apps))| GroupView {
            name,
            members: members.into_iter().collect(),
            apps: apps
                .into_iter()
                .map(|(id, name)| {
                    // Resolve back to the spec for logo + type (#809);
                    // admin page, the linear scan is fine.
                    match specs.iter().find(|s| s.id == id) {
                        Some(s) => AppRef::from_spec(s),
                        None => AppRef { id, name, logo: None, kind: "app" },
                    }
                })
                .collect(),
        })
        .collect();

    // Truly public specs — open to everyone — need BOTH access-groups and
    // access-users empty (#623 audit: a users-only-gated spec is not public).
    let mut public_apps: Vec<AppRef> = specs
        .iter()
        .filter(|s| s.is_open())
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
        role: Role::Admin,
        groups,
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

#[derive(Deserialize)]
struct MemberForm {
    group: String,
    username: String,
}

async fn add_member(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Form(f): Form<MemberForm>,
) -> Response {
    let Some(group) = clean_group(&f.group) else {
        return redirect_flash("bad-input");
    };
    let Some(db) = state.db.as_ref() else {
        return redirect_flash("bad-input");
    };
    match crate::db::users::fetch(db, &f.username).await {
        Ok(Some(u)) => {
            let mut groups = u.groups;
            if !groups.iter().any(|g| g == &group) {
                groups.push(group);
            }
            let _ = crate::db::users::set_groups(db, &u.username, &groups, Some(admin.actor())).await;
            redirect_flash("member-added")
        }
        _ => redirect_flash("bad-input"),
    }
}

async fn remove_member(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Form(f): Form<MemberForm>,
) -> Response {
    let Some(db) = state.db.as_ref() else {
        return redirect_flash("bad-input");
    };
    if let Ok(Some(u)) = crate::db::users::fetch(db, &f.username).await {
        let groups: Vec<String> = u.groups.into_iter().filter(|g| g != &f.group).collect();
        let _ = crate::db::users::set_groups(db, &u.username, &groups, Some(admin.actor())).await;
    }
    redirect_flash("member-removed")
}
