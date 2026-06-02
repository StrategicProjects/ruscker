//! Admin > Groups (read-only, #503).
//!
//! Groups in Ruscker are **derived**, not a first-class entity: a group is
//! any name that appears in a user's memberships or a spec's `access-groups`.
//! This page surfaces them — for each group, its member users and the apps it
//! gates — so an operator can spot typos / orphan groups and see who can use
//! what. No CRUD: memberships are edited on the user, app access on the spec.

use askama::Template;
use axum::{extract::State, response::Response, routing::get, Router};
use std::collections::{BTreeMap, BTreeSet};

use crate::auth::{RequireAdmin, Role};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new().route("/admin/groups", get(index))
}

/// One app reference inside a group (id + display name).
struct AppRef {
    id: String,
    name: String,
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
}

impl GroupsPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

async fn index(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    // group name → (member usernames, apps id→name). BTree everywhere so the
    // page is deterministically ordered (groups, members, apps) with no dups.
    let mut map: BTreeMap<String, (BTreeSet<String>, BTreeMap<String, String>)> = BTreeMap::new();

    // Users contribute members.
    if let Some(db) = state.db.as_ref() {
        if let Ok(users) = crate::db::users::list_all(db).await {
            for u in users {
                for g in u.groups {
                    map.entry(g).or_default().0.insert(u.username.clone());
                }
            }
        }
    }

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
                .map(|(id, name)| AppRef { id, name })
                .collect(),
        })
        .collect();

    let page = GroupsPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "groups",
        role: Role::Admin,
        groups,
    };
    super::render(&page)
}
