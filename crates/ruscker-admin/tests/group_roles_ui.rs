//! Group roles on the Groups page (#1067): who may define, remove, assign
//! and unassign, with exact status codes, against data that EXISTS, and
//! proof that a refusal changes nothing. Harness copied from
//! `sso_clients_ui.rs` (full router over a temp SQLite file, sessions
//! minted straight into the store).
//!
//! The rule under test: defining/removing a role is Admin-only (like group
//! rename/delete); assigning/unassigning is open to an Editor inside their
//! own groups, and anything outside that scope answers 404 — the same
//! non-disclosure rule as add/remove member.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::db::group_roles::{self, Assignment};
use ruscker_admin::db::ConfigDb;
use ruscker_admin::i18n::Locales;
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::sync::Arc;
use tower::ServiceExt;

const CONFIG: &str = r#"
proxy:
  title: Ruscker Test
  port: 8088
  specs:
    - id: fazenda-app
      display-name: Fazenda App
      container-image: demo/fazenda
      access-groups: [fazenda]
"#;

async fn open_db() -> ConfigDb {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ruscker-group-roles-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    ConfigDb::Sqlite(ruscker_admin::db::open(&path).await.unwrap())
}

fn app_state(db: ConfigDb) -> AppState {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let config = Config::from_yaml(CONFIG).expect("parse config");
    let locales = Locales::load().expect("load locales");
    AppState {
        config: Arc::new(config),
        base_path: Arc::from(""),
        locales: Arc::new(locales),
        admin_auth: AdminAuth::with_token("test-token"),
        admin_sessions: Arc::new(ruscker_admin::auth::InMemoryAdminSessionStore::default()),
        log_buffer: None,
        login_limiter: Arc::new(ruscker_admin::auth::LoginRateLimiter::default_policy()),
        api_limiter: Arc::new(ruscker_admin::ratelimit::ApiRateLimiter::new()),
        db: Some(db),
        images_dir: None,
        master_key: Default::default(),
        backend: None,
        replicas: Arc::new(tokio::sync::RwLock::new(Default::default())),
        cookie_key: ruscker_proxy::sticky::CookieKey::random(),
        spawn_locks: Arc::new(dashmap::DashMap::new()),
        sessions: Arc::new(ruscker_admin::sessions::InMemorySessionStore::new()),
        logout_index: Arc::new(dashmap::DashMap::new()),
        leader: Arc::new(ruscker_admin::leader::AlwaysLeader),
        metrics: ruscker_admin::metrics_cache::MetricsCache::new(),
        draining: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        spec_cache: std::sync::Arc::new(dashmap::DashMap::new()),
        identity_cache: Default::default(),
        catalog_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        access_counter: Arc::new(ruscker_admin::access_counter::AccessCounter::default()),
        alerts: ruscker_admin::alerts::AlertSink::default(),
        activity: ruscker_admin::activity::ActivitySink::default(),
    }
}

/// The fixture:
/// * `fazenda` — Editor `ed`, Viewers `ana` and `cris`, Admin `boss`;
///   roles `gestor` and `analista`; `ana` holds `gestor`.
/// * `saude` — Viewer `bia`; role `medico`; `bia` holds `medico`.
async fn fixture() -> (AppState, ConfigDb) {
    let db = open_db().await;
    for (name, role, groups) in [
        ("root", Role::Admin, &[][..]),
        ("ed", Role::Editor, &["fazenda"][..]),
        ("ana", Role::Viewer, &["fazenda"][..]),
        ("cris", Role::Viewer, &["fazenda"][..]),
        ("boss", Role::Admin, &["fazenda"][..]),
        ("bia", Role::Viewer, &["saude"][..]),
        ("vic", Role::Viewer, &["fazenda"][..]),
    ] {
        let groups: Vec<String> = groups.iter().map(|g| g.to_string()).collect();
        ruscker_admin::db::users::create(&db, name, "Correct#Pass9", role, false, &groups, Some("seed"))
            .await
            .unwrap();
    }
    for (g, r) in [("fazenda", "gestor"), ("fazenda", "analista"), ("saude", "medico")] {
        group_roles::define(&db, g, r, Some("seed")).await.unwrap();
    }
    group_roles::assign(&db, "ana", "fazenda", "gestor", None).await.unwrap();
    group_roles::assign(&db, "bia", "saude", "medico", None).await.unwrap();
    (app_state(db.clone()), db)
}

async fn cookie(state: &AppState, role: Role, actor: &str) -> String {
    let sid = state.admin_sessions.create(role, Some(actor.into())).await;
    format!("{COOKIE_NAME}={sid}")
}

/// POST a form; returns (status, Location header).
async fn post(state: &AppState, uri: &str, body: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let mut req = Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let resp = router(state.clone())
        .oneshot(req.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let loc = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    (resp.status(), loc)
}

async fn get(state: &AppState, uri: &str, cookie: &str) -> (StatusCode, String) {
    let resp = router(state.clone())
        .oneshot(
            Request::builder()
                .uri(uri)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// Everything the tests can change, in one comparable value.
async fn snapshot(db: &ConfigDb) -> (Vec<(String, Vec<String>)>, Vec<Assignment>) {
    (
        group_roles::list_definitions(db).await.unwrap().into_iter().collect(),
        group_roles::list_assignments(db).await.unwrap(),
    )
}

fn held(db_rows: &[Assignment], user: &str, group: &str, role: &str) -> bool {
    db_rows
        .iter()
        .any(|a| a.username == user && a.group == group && a.role == role)
}

// ── Define / remove (Admin only) ────────────────────────────────────────

#[tokio::test]
async fn admin_defines_and_removes_a_role() {
    let (state, db) = fixture().await;
    let admin = cookie(&state, Role::Admin, "root").await;

    let (st, loc) = post(&state, "/admin/groups/roles/define", "group=fazenda&role=auditor", Some(&admin)).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/groups?flash=role-added");
    assert!(group_roles::list_definitions(&db).await.unwrap()["fazenda"].contains(&"auditor".to_string()));

    // Removing `gestor` takes it from its holder too.
    let (st, loc) = post(&state, "/admin/groups/roles/remove", "group=fazenda&role=gestor", Some(&admin)).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/groups?flash=role-removed");
    let (defs, rows) = snapshot(&db).await;
    assert!(!defs.iter().any(|(g, rs)| g == "fazenda" && rs.contains(&"gestor".to_string())));
    assert!(!held(&rows, "ana", "fazenda", "gestor"));

    // Removing a role that does not exist: 404, nothing changes.
    let before = snapshot(&db).await;
    let (st, _) = post(&state, "/admin/groups/roles/remove", "group=fazenda&role=gestor", Some(&admin)).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(snapshot(&db).await, before);
}

#[tokio::test]
async fn invalid_names_and_unknown_groups_change_nothing() {
    let (state, db) = fixture().await;
    let admin = cookie(&state, Role::Admin, "root").await;
    let before = snapshot(&db).await;

    for bad in ["two+words", "a%3Ab", "a%2Cb", "gest%C3%A3o", "", &"a".repeat(41)] {
        let (st, loc) = post(
            &state,
            "/admin/groups/roles/define",
            &format!("group=fazenda&role={bad}"),
            Some(&admin),
        )
        .await;
        assert_eq!(st, StatusCode::SEE_OTHER, "{bad}");
        assert_eq!(loc, "/admin/groups?flash=bad-role", "{bad}");
    }
    // A group nobody references: refusing it keeps a typo from creating one.
    let (st, _) = post(&state, "/admin/groups/roles/define", "group=fazend&role=gestor", Some(&admin)).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(snapshot(&db).await, before);
}

#[tokio::test]
async fn only_an_admin_may_define_or_remove() {
    let (state, db) = fixture().await;
    let editor = cookie(&state, Role::Editor, "ed").await;
    let viewer = cookie(&state, Role::Viewer, "ana").await;
    let before = snapshot(&db).await;

    for (uri, body) in [
        ("/admin/groups/roles/define", "group=fazenda&role=auditor"),
        ("/admin/groups/roles/remove", "group=fazenda&role=gestor"),
    ] {
        // Even the Editor who owns `fazenda` cannot change its vocabulary.
        assert_eq!(post(&state, uri, body, Some(&editor)).await.0, StatusCode::FORBIDDEN, "{uri}");
        assert_eq!(post(&state, uri, body, Some(&viewer)).await.0, StatusCode::FORBIDDEN, "{uri}");
        let (st, loc) = post(&state, uri, body, None).await;
        assert_eq!(st, StatusCode::SEE_OTHER, "{uri}");
        assert_eq!(loc, "/admin/login", "{uri}");
    }
    assert_eq!(snapshot(&db).await, before);
}

// ── Assign / unassign (Editor in scope) ─────────────────────────────────

#[tokio::test]
async fn editor_assigns_and_unassigns_inside_their_group() {
    let (state, db) = fixture().await;
    let editor = cookie(&state, Role::Editor, "ed").await;

    let (st, loc) = post(
        &state,
        "/admin/groups/roles/assign",
        "group=fazenda&username=cris&role=analista",
        Some(&editor),
    )
    .await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/groups?flash=role-assigned");
    assert!(held(&snapshot(&db).await.1, "cris", "fazenda", "analista"));

    let (st, loc) = post(
        &state,
        "/admin/groups/roles/unassign",
        "group=fazenda&username=ana&role=gestor",
        Some(&editor),
    )
    .await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/groups?flash=role-unassigned");
    assert!(!held(&snapshot(&db).await.1, "ana", "fazenda", "gestor"));

    // The audit trail names the Editor.
    let audits = ruscker_admin::db::audit::list(&db, &ruscker_admin::db::audit::AuditFilter::new())
        .await
        .unwrap();
    assert!(audits.iter().any(|e| e.action == "group-role.assign"
        && e.actor.as_deref() == Some("ed")
        && e.target.as_deref() == Some("user:cris")));
    assert!(audits.iter().any(|e| e.action == "group-role.unassign"
        && e.actor.as_deref() == Some("ed")
        && e.target.as_deref() == Some("user:ana")));
}

#[tokio::test]
async fn editor_out_of_scope_gets_404_and_nothing_changes() {
    let (state, db) = fixture().await;
    let editor = cookie(&state, Role::Editor, "ed").await;
    let before = snapshot(&db).await;

    for (uri, body, why) in [
        // A foreign group, with a member and a role that both exist.
        ("/admin/groups/roles/assign", "group=saude&username=bia&role=medico", "foreign group"),
        ("/admin/groups/roles/unassign", "group=saude&username=bia&role=medico", "foreign group"),
        // An Admin account inside the Editor's own group.
        ("/admin/groups/roles/assign", "group=fazenda&username=boss&role=gestor", "admin target"),
        // A user who exists but is not in the group.
        ("/admin/groups/roles/assign", "group=fazenda&username=bia&role=gestor", "non-member"),
        // A role defined in ANOTHER group.
        ("/admin/groups/roles/assign", "group=fazenda&username=cris&role=medico", "foreign role"),
        // An assignment that does not exist.
        ("/admin/groups/roles/unassign", "group=fazenda&username=cris&role=gestor", "not held"),
    ] {
        let (st, _) = post(&state, uri, body, Some(&editor)).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{why}: {uri} {body}");
    }
    assert_eq!(snapshot(&db).await, before);
}

/// Group roles are authorization data inside the hosted apps, so a scoped
/// Editor granting or shedding their OWN would be self-escalation there.
/// Same rule as the Users page's scoped self-edit, answered 404. An Admin
/// (unscoped) is not restricted.
#[tokio::test]
async fn editor_cannot_change_their_own_group_roles_but_an_admin_can() {
    let (state, db) = fixture().await;
    group_roles::assign(&db, "ed", "fazenda", "analista", None).await.unwrap();
    let editor = cookie(&state, Role::Editor, "ed").await;
    let before = snapshot(&db).await;

    for (uri, body) in [
        ("/admin/groups/roles/assign", "group=fazenda&username=ed&role=gestor"),
        ("/admin/groups/roles/unassign", "group=fazenda&username=ed&role=analista"),
        // The comparison is on the normalized username.
        ("/admin/groups/roles/assign", "group=fazenda&username=ED&role=gestor"),
        ("/admin/groups/roles/unassign", "group=fazenda&username=+Ed+&role=analista"),
    ] {
        let (st, _) = post(&state, uri, body, Some(&editor)).await;
        assert_eq!(st, StatusCode::NOT_FOUND, "{uri} {body}");
    }
    assert_eq!(snapshot(&db).await, before);

    // The page offers no control on the Editor's own row: of the two
    // holders in fazenda (ana, ed) only ana's role can be taken away, and
    // `ed` is not in the assign picker.
    let (_, html) = get(&state, "/admin/groups", &editor).await;
    assert_eq!(html.matches(r#"action="/admin/groups/roles/unassign""#).count(), 1, "{html}");
    let assign_form = &html[html.find(r#"action="/admin/groups/roles/assign""#).unwrap()..];
    let assign_form = &assign_form[..assign_form.find("</form>").unwrap()];
    assert!(!assign_form.contains(r#"<option value="ed">"#), "{assign_form}");
    assert!(assign_form.contains(r#"<option value="cris">"#));

    let admin = cookie(&state, Role::Admin, "root").await;
    let (st, _) = post(&state, "/admin/groups/roles/assign", "group=fazenda&username=ed&role=gestor", Some(&admin)).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    let (st, _) = post(&state, "/admin/groups/roles/unassign", "group=fazenda&username=ed&role=analista", Some(&admin)).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    let rows = snapshot(&db).await.1;
    assert!(held(&rows, "ed", "fazenda", "gestor"));
    assert!(!held(&rows, "ed", "fazenda", "analista"));
}

#[tokio::test]
async fn viewer_and_anonymous_cannot_assign() {
    let (state, db) = fixture().await;
    let viewer = cookie(&state, Role::Viewer, "ana").await;
    let before = snapshot(&db).await;
    for (uri, body) in [
        ("/admin/groups/roles/assign", "group=fazenda&username=cris&role=analista"),
        ("/admin/groups/roles/unassign", "group=fazenda&username=ana&role=gestor"),
    ] {
        assert_eq!(post(&state, uri, body, Some(&viewer)).await.0, StatusCode::FORBIDDEN, "{uri}");
        let (st, loc) = post(&state, uri, body, None).await;
        assert_eq!(st, StatusCode::SEE_OTHER, "{uri}");
        assert_eq!(loc, "/admin/login", "{uri}");
    }
    assert_eq!(snapshot(&db).await, before);
}

#[tokio::test]
async fn admin_assigns_in_any_group_but_never_to_a_non_member() {
    let (state, db) = fixture().await;
    let admin = cookie(&state, Role::Admin, "root").await;
    let (st, _) = post(
        &state,
        "/admin/groups/roles/assign",
        "group=fazenda&username=boss&role=gestor",
        Some(&admin),
    )
    .await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert!(held(&snapshot(&db).await.1, "boss", "fazenda", "gestor"));

    let before = snapshot(&db).await;
    let (st, _) = post(
        &state,
        "/admin/groups/roles/assign",
        "group=saude&username=ana&role=medico",
        Some(&admin),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND, "ana is not in saude");
    assert_eq!(snapshot(&db).await, before);
}

// ── Consistency through the real routes ─────────────────────────────────

#[tokio::test]
async fn removing_a_member_on_the_groups_page_drops_their_roles() {
    let (state, db) = fixture().await;
    let editor = cookie(&state, Role::Editor, "ed").await;
    let (st, _) = post(
        &state,
        "/admin/groups/members/remove",
        "group=fazenda&username=ana",
        Some(&editor),
    )
    .await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert!(!held(&snapshot(&db).await.1, "ana", "fazenda", "gestor"));
}

/// If the group's roles cannot be moved, the rename must change NOTHING.
/// Going on to rewrite the memberships would prune every holder's roles
/// under the old name — silently, behind a "renamed" message.
#[tokio::test]
async fn a_rename_that_cannot_move_the_roles_changes_nothing() {
    let (state, db) = fixture().await;
    let admin = cookie(&state, Role::Admin, "root").await;
    let ConfigDb::Sqlite(pool) = &db else { unreachable!() };
    // Make the roles move fail the way a database error would.
    sqlx::query(
        "CREATE TRIGGER fail_role_move BEFORE INSERT ON group_roles
         BEGIN SELECT RAISE(ABORT, 'simulated failure'); END",
    )
    .execute(pool)
    .await
    .unwrap();
    let before = snapshot(&db).await;
    let groups_before = ruscker_admin::db::users::fetch(&db, "ana").await.unwrap().unwrap().groups;

    let (st, loc) = post(&state, "/admin/groups/rename", "old=fazenda&new=tesouro", Some(&admin)).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/groups?flash=failed", "the page must not claim success");
    assert_eq!(snapshot(&db).await, before, "roles and holders untouched");
    let groups_after = ruscker_admin::db::users::fetch(&db, "ana").await.unwrap().unwrap().groups;
    assert_eq!(groups_after, groups_before, "memberships untouched");
    assert!(held(&snapshot(&db).await.1, "ana", "fazenda", "gestor"));

    // With the fault gone the same rename goes through and keeps the role.
    sqlx::query("DROP TRIGGER fail_role_move").execute(pool).await.unwrap();
    let (_, loc) = post(&state, "/admin/groups/rename", "old=fazenda&new=tesouro", Some(&admin)).await;
    assert_eq!(loc, "/admin/groups?flash=renamed");
    assert!(held(&snapshot(&db).await.1, "ana", "tesouro", "gestor"));
}

#[tokio::test]
async fn renaming_a_group_carries_its_roles_and_deleting_drops_them() {
    let (state, db) = fixture().await;
    let admin = cookie(&state, Role::Admin, "root").await;

    let (st, _) = post(
        &state,
        "/admin/groups/rename",
        "old=fazenda&new=Fazenda%3A+n%C3%BAcleo+A",
        Some(&admin),
    )
    .await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    let (defs, rows) = snapshot(&db).await;
    let renamed = "Fazenda: núcleo A";
    assert!(!defs.iter().any(|(g, _)| g == "fazenda"));
    assert!(defs
        .iter()
        .any(|(g, rs)| g == renamed && rs == &vec!["analista".to_string(), "gestor".to_string()]));
    assert!(held(&rows, "ana", renamed, "gestor"), "{rows:?}");

    let (st, _) = post(
        &state,
        "/admin/groups/delete",
        "name=Fazenda%3A+n%C3%BAcleo+A",
        Some(&admin),
    )
    .await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    let (defs, rows) = snapshot(&db).await;
    assert!(!defs.iter().any(|(g, _)| g == renamed));
    assert!(rows.iter().all(|a| a.group != renamed));
    // The other group is untouched.
    assert!(held(&rows, "bia", "saude", "medico"));
}

#[tokio::test]
async fn user_edit_form_and_user_delete_drop_roles() {
    let (state, db) = fixture().await;
    let admin = cookie(&state, Role::Admin, "root").await;
    group_roles::assign(&db, "cris", "fazenda", "analista", None).await.unwrap();

    // The consolidated edit form (update_account): ana leaves fazenda.
    let (st, _) = post(
        &state,
        "/admin/users/ana/edit",
        "role=viewer&groups=outro&setor=&email=&celular=",
        Some(&admin),
    )
    .await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert!(!held(&snapshot(&db).await.1, "ana", "fazenda", "gestor"));

    // The bare groups endpoint (set_groups): cris leaves fazenda.
    let (st, _) = post(&state, "/admin/users/cris/groups", "groups=", Some(&admin)).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert!(!held(&snapshot(&db).await.1, "cris", "fazenda", "analista"));

    // Delete through the Users page.
    let (st, _) = post(&state, "/admin/users/bia/delete", "", Some(&admin)).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert!(!held(&snapshot(&db).await.1, "bia", "saude", "medico"));
    assert!(snapshot(&db).await.1.is_empty(), "{:?}", snapshot(&db).await.1);
}

// ── Rendering ───────────────────────────────────────────────────────────

#[tokio::test]
async fn page_shows_roles_and_scopes_them_for_an_editor() {
    let (state, _db) = fixture().await;
    let admin = cookie(&state, Role::Admin, "root").await;
    let (st, html) = get(&state, "/admin/groups", &admin).await;
    assert_eq!(st, StatusCode::OK);
    assert!(html.contains(r#"action="/admin/groups/roles/define""#));
    assert!(html.contains(r#"action="/admin/groups/roles/remove""#));
    assert!(html.contains(r#"action="/admin/groups/roles/assign""#));
    assert!(html.contains("medico"), "Admin sees every group's roles");

    let editor = cookie(&state, Role::Editor, "ed").await;
    let (st, html) = get(&state, "/admin/groups", &editor).await;
    assert_eq!(st, StatusCode::OK);
    assert!(!html.contains(r#"action="/admin/groups/roles/define""#), "no vocabulary edits");
    assert!(!html.contains(r#"action="/admin/groups/roles/remove""#));
    assert!(html.contains(r#"action="/admin/groups/roles/assign""#));
    assert!(html.contains(r#"action="/admin/groups/roles/unassign""#));
    assert!(html.contains("gestor"));
    assert!(!html.contains("medico"), "a foreign group's roles never render");
    assert!(!html.contains(">boss<"), "Admin accounts are not offered to an Editor");
}

/// The user edit form carries the person's full name (#1081): saved
/// with the rest, shown again on the form, cleared by a blank value —
/// and an Editor cannot set it for someone outside their groups.
#[tokio::test]
async fn edit_form_saves_and_clears_the_full_name() {
    let (state, db) = fixture().await;
    let admin = cookie(&state, Role::Admin, "root").await;
    let full = |db: &ConfigDb, u: &'static str| {
        let db = db.clone();
        async move { ruscker_admin::db::users::full_name(&db, u).await.unwrap() }
    };

    let (st, _) = post(
        &state,
        "/admin/users/ana/edit",
        "full_name=Ana+Maria+de+Souza&role=viewer&groups=fazenda&setor=&email=&celular=",
        Some(&admin),
    )
    .await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert_eq!(full(&db, "ana").await.as_deref(), Some("Ana Maria de Souza"));
    let (_, html) = get(&state, "/admin/users/ana/edit", &admin).await;
    assert!(html.contains(r#"name="full_name" value="Ana Maria de Souza""#));

    // An Editor scoped to `fazenda` cannot touch `bia` (group `saude`).
    let ed = cookie(&state, Role::Editor, "ed").await;
    let (st, _) = post(
        &state,
        "/admin/users/bia/edit",
        "full_name=Hacked&role=viewer&groups=saude&setor=&email=&celular=",
        Some(&ed),
    )
    .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_eq!(full(&db, "bia").await, None);

    // Blank clears it.
    let (st, _) = post(
        &state,
        "/admin/users/ana/edit",
        "full_name=&role=viewer&groups=fazenda&setor=&email=&celular=",
        Some(&admin),
    )
    .await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert_eq!(full(&db, "ana").await, None);
}
