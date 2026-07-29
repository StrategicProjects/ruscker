//! Group-derived authorization scope for admin-panel Editors (#990).
//!
//! This lives outside [`crate::auth`] because authentication answers
//! **who** the caller is and which role their session carries, while this
//! module answers **which rows** that identity may act on. Keeping those
//! concerns separate also makes the slice-0 boundary explicit: defining
//! [`EditorScope`] alone changes no handler or page behaviour. Later slices
//! can opt individual handlers into this primitive without growing a second,
//! inconsistent access rule.

use std::collections::BTreeSet;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::response::Response;
use ruscker_config::Spec;

use crate::auth::{RequireEditor, Role};
use crate::db::{self, users::UserRow, ConfigDb};

/// The group scope carried by an authenticated Editor-or-Admin request.
///
/// Admins, including the break-glass token session whose `actor` is `None`,
/// are deliberately unscoped: emergency access must remain useful even when
/// the account database is absent or damaged. Editors are the opposite. Their
/// groups are fetched from `users` on every extraction so removing a group
/// takes effect on the next request; session-caching them would keep revoked
/// access alive because `set_groups` does not revoke sessions.
///
/// A missing account, missing database, or failed lookup therefore produces an
/// empty (but still scoped) group list. That fail-closed result grants access
/// only to open specs and never silently promotes an Editor to global access.
#[derive(Clone, Debug)]
pub struct EditorScope {
    pub role: Role,
    pub actor: Option<String>,
    pub groups: Vec<String>,
    pub unscoped: bool,
}

impl EditorScope {
    async fn from_editor(editor: RequireEditor, db: Option<&ConfigDb>) -> Self {
        if editor.role == Role::Admin {
            return Self {
                role: editor.role,
                actor: editor.actor,
                groups: Vec::new(),
                unscoped: true,
            };
        }

        let groups = match (db, editor.actor.as_deref()) {
            (Some(db), Some(username)) => match db::users::fetch(db, username).await {
                Ok(Some(row)) => row.groups,
                Ok(None) => Vec::new(),
                Err(error) => {
                    tracing::warn!(
                        actor = username,
                        error = ?error,
                        "editor-scope lookup failed; using empty scope"
                    );
                    Vec::new()
                }
            },
            _ => Vec::new(),
        };

        Self {
            role: editor.role,
            actor: editor.actor,
            groups,
            unscoped: false,
        }
    }

    fn has_group(&self, group: &str) -> bool {
        self.groups.iter().any(|owned| owned == group)
    }

    /// Whether this caller may mutate or operate one spec.
    ///
    /// Open specs are shared admin-panel resources, so every Editor may work
    /// on them. Restricted specs require at least one shared `access-groups`
    /// entry; this extends the same group model used by landing/proxy access
    /// instead of introducing app ownership.
    ///
    /// [`Spec::is_open`] means **both** `access-groups` and `access-users` are
    /// empty. Consequently, a spec restricted only through `access-users` is
    /// not reachable by any scoped Editor and remains Admin-only. That is a
    /// deliberate safe default: a named-user ACL provides no group boundary
    /// from which Editor authority could be derived.
    pub fn may_touch_spec(&self, spec: &Spec) -> bool {
        self.unscoped
            || spec.is_open()
            || spec
                .access_groups
                .as_deref()
                .is_some_and(|groups| groups.iter().any(|group| self.has_group(group)))
    }

    /// Whether this caller may mutate one user account.
    ///
    /// Scoped Editors may only manage a user across a shared team boundary.
    /// Admin targets are excluded even when groups overlap so an Editor can
    /// never change or weaken the account that controls global access.
    pub fn may_touch_user(&self, user: &UserRow) -> bool {
        self.unscoped
            || (user.role != Role::Admin && user.groups.iter().any(|group| self.has_group(group)))
    }

    /// Whether every requested group is already inside this caller's scope.
    ///
    /// Requiring a subset prevents an Editor from granting a group they do not
    /// possess and then using that newly-created overlap to cross a team
    /// boundary. Admins remain free to assign the complete group vocabulary.
    pub fn may_assign_groups(&self, requested: &[String]) -> bool {
        self.unscoped || requested.iter().all(|group| self.has_group(group))
    }

    /// Whether this caller may assign the requested role.
    ///
    /// Viewer and Editor stay inside delegated administration; Admin does not,
    /// because granting it would let an Editor escape every group boundary.
    pub fn may_assign_role(&self, requested: Role) -> bool {
        self.unscoped || matches!(requested, Role::Viewer | Role::Editor)
    }

    /// Merge an Editor's requested memberships without erasing another team's.
    ///
    /// `db::users::set_groups` replaces the whole list. For a user shared by
    /// two teams, writing only the groups visible in one Editor's form would
    /// otherwise silently remove the other team's memberships. Scoped callers
    /// therefore replace only memberships they own: existing out-of-scope
    /// groups survive, requested in-scope groups are applied, and requested
    /// foreign groups are ignored. Admins perform an ordinary full replacement.
    ///
    /// The sorted, deduplicated result makes persistence and tests
    /// deterministic without changing the case-sensitive group semantics.
    pub fn merge_preserving_out_of_scope(
        &self,
        existing: &[String],
        requested: &[String],
    ) -> Vec<String> {
        if self.unscoped {
            return requested
                .iter()
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
        }

        existing
            .iter()
            .filter(|group| !self.has_group(group))
            .chain(requested.iter().filter(|group| self.has_group(group)))
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }
}

impl FromRequestParts<crate::AppState> for EditorScope {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &crate::AppState,
    ) -> Result<Self, Self::Rejection> {
        // Reusing the established guard preserves its exact split:
        // anonymous requests redirect to login, authenticated Viewers get 403.
        let editor = RequireEditor::from_request_parts(parts, state).await?;
        Ok(Self::from_editor(editor, state.db.as_ref()).await)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn scope(groups: &[&str]) -> EditorScope {
        EditorScope {
            role: Role::Editor,
            actor: Some("editor".into()),
            groups: groups.iter().map(|group| (*group).to_string()).collect(),
            unscoped: false,
        }
    }

    fn spec(groups: &[&str], users: &[&str]) -> Spec {
        let mut spec: Spec = serde_yaml_ng::from_str(
            "id: test\n\
             display-name: Test\n\
             container-image: test:latest\n",
        )
        .expect("parse test spec");
        spec.access_groups =
            (!groups.is_empty()).then(|| groups.iter().map(|group| (*group).into()).collect());
        spec.access_users =
            (!users.is_empty()).then(|| users.iter().map(|user| (*user).into()).collect());
        spec
    }

    fn user(role: Role, groups: &[&str]) -> UserRow {
        UserRow {
            id: "user-id".into(),
            username: "target".into(),
            role,
            must_change_password: false,
            groups: groups.iter().map(|group| (*group).to_string()).collect(),
            created_at: Utc::now(),
            created_by: None,
            setor: None,
            email: None,
            celular: None,
        }
    }

    fn editor_guard(actor: Option<&str>) -> RequireEditor {
        RequireEditor {
            role: Role::Editor,
            actor: actor.map(str::to_string),
        }
    }

    #[tokio::test]
    async fn admin_account_and_break_glass_token_remain_unrestricted() {
        let account = EditorScope::from_editor(
            RequireEditor {
                role: Role::Admin,
                actor: Some("admin".into()),
            },
            None,
        )
        .await;
        let token = EditorScope::from_editor(
            RequireEditor {
                role: Role::Admin,
                actor: None,
            },
            None,
        )
        .await;
        let restricted = spec(&["other-team"], &[]);
        let admin_target = user(Role::Admin, &[]);

        for scope in [&account, &token] {
            assert!(scope.unscoped);
            assert!(scope.groups.is_empty());
            assert!(scope.may_touch_spec(&restricted));
            assert!(scope.may_touch_user(&admin_target));
            assert!(scope.may_assign_groups(&["foreign".into()]));
            assert!(scope.may_assign_role(Role::Admin));
        }
        assert_eq!(account.actor.as_deref(), Some("admin"));
        assert_eq!(token.actor, None);
    }

    #[test]
    fn editor_touches_only_open_or_group_shared_specs() {
        let editor = scope(&["blue"]);

        assert!(editor.may_touch_spec(&spec(&[], &[])), "open spec");
        assert!(
            editor.may_touch_spec(&spec(&["blue", "green"], &[])),
            "shared group"
        );
        assert!(
            !editor.may_touch_spec(&spec(&["green"], &[])),
            "foreign group"
        );
        assert!(
            !editor.may_touch_spec(&spec(&[], &["named-user"])),
            "access-users alone provides no Editor group boundary"
        );
    }

    #[test]
    fn editor_never_touches_admin_or_user_without_shared_group() {
        let editor = scope(&["blue"]);

        assert!(
            !editor.may_touch_user(&user(Role::Admin, &["blue"])),
            "Admin stays protected even with a shared group"
        );
        assert!(
            !editor.may_touch_user(&user(Role::Viewer, &["green"])),
            "a foreign-team user stays protected"
        );
        assert!(editor.may_touch_user(&user(Role::Editor, &["blue", "green"])));
    }

    #[test]
    fn editor_cannot_assign_foreign_group_or_admin_role() {
        let editor = scope(&["blue", "red"]);

        assert!(editor.may_assign_groups(&["red".into(), "blue".into()]));
        assert!(!editor.may_assign_groups(&["blue".into(), "green".into()]));
        assert!(editor.may_assign_role(Role::Viewer));
        assert!(editor.may_assign_role(Role::Editor));
        assert!(!editor.may_assign_role(Role::Admin));
    }

    #[test]
    fn merge_preserves_foreign_group_when_adding_inside_scope() {
        let editor = scope(&["blue", "red"]);
        let merged = editor.merge_preserving_out_of_scope(
            &["other-team".into(), "blue".into()],
            &["blue".into(), "red".into()],
        );

        assert_eq!(merged, vec!["blue", "other-team", "red"]);
    }

    #[test]
    fn merge_preserves_foreign_group_when_removing_inside_scope() {
        let editor = scope(&["blue", "red"]);
        let merged = editor.merge_preserving_out_of_scope(
            &["red".into(), "other-team".into(), "blue".into()],
            &["red".into()],
        );

        assert_eq!(merged, vec!["other-team", "red"]);
    }

    #[test]
    fn editor_with_empty_scope_touches_only_open_specs() {
        let editor = scope(&[]);

        assert!(editor.may_touch_spec(&spec(&[], &[])));
        assert!(!editor.may_touch_spec(&spec(&["blue"], &[])));
        assert!(!editor.may_touch_spec(&spec(&[], &["named-user"])));
    }

    #[tokio::test]
    async fn editor_without_database_fails_closed_to_empty_scope() {
        let editor = EditorScope::from_editor(editor_guard(Some("gone")), None).await;

        assert!(!editor.unscoped);
        assert!(editor.groups.is_empty());
        assert!(editor.may_touch_spec(&spec(&[], &[])));
        assert!(!editor.may_touch_spec(&spec(&["blue"], &[])));
    }

    #[tokio::test]
    async fn deleted_editor_account_fails_closed_to_empty_scope() {
        let db = ConfigDb::Sqlite(crate::db::open_memory().await.unwrap());
        let editor = EditorScope::from_editor(editor_guard(Some("gone")), Some(&db)).await;

        assert!(!editor.unscoped);
        assert!(editor.groups.is_empty());
    }

    #[tokio::test]
    async fn editor_groups_are_refetched_on_every_scope_resolution() {
        let pool = crate::db::open_memory().await.unwrap();
        let now = Utc::now();
        sqlx::query(
            "INSERT INTO users
               (id, username, password_hash, role, must_change_password,
                groups, created_at, updated_at, created_by)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind("editor-id")
        .bind("editor")
        .bind("unused-test-hash")
        .bind("editor")
        .bind(0_i64)
        .bind("blue")
        .bind(now)
        .bind(now)
        .bind(Option::<String>::None)
        .execute(&pool)
        .await
        .unwrap();
        let db = ConfigDb::Sqlite(pool.clone());

        let first = EditorScope::from_editor(editor_guard(Some("editor")), Some(&db)).await;
        sqlx::query("UPDATE users SET groups = ? WHERE username = ?")
            .bind("red")
            .bind("editor")
            .execute(&pool)
            .await
            .unwrap();
        let second = EditorScope::from_editor(editor_guard(Some("editor")), Some(&db)).await;

        assert_eq!(first.groups, vec!["blue"]);
        assert_eq!(second.groups, vec!["red"]);
    }
}
