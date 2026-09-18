//! CSV export of the users table (#1056): header + rows, `all` vs
//! `filtered`, Editor scoping, the audit row and the download headers.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::db::ConfigDb;
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::sync::Arc;
use tower::ServiceExt;

const YAML: &str = "proxy:\n  title: Test\n  specs: []\n";

async fn state_with_db() -> (AppState, sqlx::SqlitePool) {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let path = std::env::temp_dir().join(format!("ruscker-users-export-{}.db", uuid::Uuid::new_v4()));
    let pool = ruscker_admin::db::open(&path).await.expect("open db");
    let state = AppState {
        config: Arc::new(Config::from_yaml(YAML).expect("parse config")),
        base_path: Arc::from(""),
        locales: Arc::new(ruscker_admin::i18n::Locales::load().expect("load locales")),
        admin_auth: AdminAuth::with_token("break-glass-tok"),
        admin_sessions: Arc::new(ruscker_admin::auth::InMemoryAdminSessionStore::default()),
        log_buffer: None,
        login_limiter: Arc::new(ruscker_admin::auth::LoginRateLimiter::default_policy()),
        api_limiter: Arc::new(ruscker_admin::ratelimit::ApiRateLimiter::new()),
        db: Some(ConfigDb::Sqlite(pool.clone())),
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
        spec_cache: Arc::new(dashmap::DashMap::new()),
        identity_cache: Default::default(),
        catalog_cache: Arc::new(tokio::sync::RwLock::new(None)),
        access_counter: Arc::new(ruscker_admin::access_counter::AccessCounter::default()),
        alerts: ruscker_admin::alerts::AlertSink::default(),
        activity: ruscker_admin::activity::ActivitySink::default(),
    };
    (state, pool)
}

async fn user(pool: &sqlx::SqlitePool, name: &str, role: Role, groups: &[&str], setor: Option<&str>) {
    let groups: Vec<String> = groups.iter().map(|g| g.to_string()).collect();
    ruscker_admin::db::users::create(
        &ConfigDb::Sqlite(pool.clone()),
        name,
        "Correct#Pass9",
        role,
        false,
        &groups,
        Some("test"),
    )
    .await
    .unwrap();
    if let Some(setor) = setor {
        ruscker_admin::db::users::update_profile(
            &ConfigDb::Sqlite(pool.clone()),
            name,
            Some(setor),
            None,
            None,
            Some("test"),
        )
        .await
        .unwrap();
    }
}

async fn cookie(state: &AppState, role: Role, actor: &str) -> String {
    let id = state.admin_sessions.create(role, Some(actor.into())).await;
    format!("{COOKIE_NAME}={id}")
}

/// GET returning (status, body, content-type, content-disposition).
async fn get(state: AppState, uri: &str, cookie: &str) -> (StatusCode, String, String, String) {
    let resp = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let hdr = |name: &str| {
        resp.headers()
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string()
    };
    let (ct, cd) = (hdr("content-type"), hdr("content-disposition"));
    let body = to_bytes(resp.into_body(), 2 << 20).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap(), ct, cd)
}

#[tokio::test]
async fn admin_exports_all_or_filtered_with_download_headers_and_audit() {
    let (state, pool) = state_with_db().await;
    user(&pool, "root", Role::Admin, &[], None).await;
    user(&pool, "ana", Role::Editor, &["saude", "dados"], Some("Estatística")).await;
    user(&pool, "bob", Role::Viewer, &["educacao"], None).await;
    let admin = cookie(&state, Role::Admin, "root").await;

    let (status, body, ct, cd) =
        get(state.clone(), "/admin/users/export.csv?scope=all&q=ana", &admin).await;
    assert_eq!(status, StatusCode::OK);
    assert!(ct.starts_with("text/csv"), "content-type: {ct}");
    assert!(cd.starts_with("attachment; filename=\"ruscker-users-"), "disposition: {cd}");
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines[0], "username,role,groups,setor,email,celular,created_at");
    // `scope=all` ignores `q`: every user, one row each.
    assert_eq!(lines.len(), 4, "{body}");
    assert!(lines.iter().any(|l| l.starts_with("ana,editor,saude;dados,Estatística,")));
    assert!(lines.iter().any(|l| l.starts_with("bob,viewer,educacao,,")));
    assert!(lines.iter().any(|l| l.starts_with("root,admin,,,")));

    // `scope=filtered` applies the page's search (username/groups/profile).
    let (status, body, _, cd) =
        get(state.clone(), "/admin/users/export.csv?scope=filtered&q=estat", &admin).await;
    assert_eq!(status, StatusCode::OK);
    assert!(cd.contains("-filtered.csv"));
    let lines: Vec<&str> = body.lines().collect();
    assert_eq!(lines.len(), 2, "{body}");
    assert!(lines[1].starts_with("ana,"));

    // Both exports were audited with scope + count, never the rows.
    let audits: Vec<(String,)> = sqlx::query_as(
        "SELECT diff_json FROM audit_log WHERE action = 'users.export' AND actor = 'root' ORDER BY id",
    )
    .fetch_all(&pool)
    .await
    .unwrap();
    assert_eq!(audits.len(), 2);
    assert!(audits[0].0.contains("\"scope\":\"all\"") && audits[0].0.contains("\"rows\":3"));
    assert!(audits[1].0.contains("\"scope\":\"filtered\"") && audits[1].0.contains("\"rows\":1"));
    assert!(!audits.iter().any(|(d,)| d.contains("Estatística")));
}

/// A group-scoped Editor (#990) exports exactly the users they can list:
/// members of their groups, never Admins, never other teams.
#[tokio::test]
async fn scoped_editor_exports_only_visible_users() {
    let (state, pool) = state_with_db().await;
    user(&pool, "root", Role::Admin, &[], None).await;
    user(&pool, "ana", Role::Editor, &["saude"], None).await;
    user(&pool, "carla", Role::Viewer, &["saude", "dados"], None).await;
    user(&pool, "bob", Role::Viewer, &["educacao"], None).await;
    let editor = cookie(&state, Role::Editor, "ana").await;

    let (status, body, _, _) = get(state.clone(), "/admin/users/export.csv?scope=all", &editor).await;
    assert_eq!(status, StatusCode::OK);
    let names: Vec<&str> = body
        .lines()
        .skip(1)
        .map(|l| l.split(',').next().unwrap())
        .collect();
    assert_eq!(names.len(), 2, "{body}");
    assert!(names.contains(&"ana") && names.contains(&"carla"));
    assert!(!names.contains(&"root") && !names.contains(&"bob"));

    // Viewers reach no admin section (#857): 403/redirect, never a file.
    let viewer = cookie(&state, Role::Viewer, "bob").await;
    let (status, body, ct, _) = get(state, "/admin/users/export.csv?scope=all", &viewer).await;
    assert_ne!(status, StatusCode::OK);
    assert!(!ct.starts_with("text/csv"));
    assert!(!body.contains("username,role"));
}

/// The page offers the export links: "all" always, "filtered" only with an
/// active search, carrying the term.
#[tokio::test]
async fn users_page_links_the_exports() {
    let (state, pool) = state_with_db().await;
    user(&pool, "root", Role::Admin, &[], None).await;
    let admin = cookie(&state, Role::Admin, "root").await;
    let (status, page, _, _) = get(state.clone(), "/admin/users", &admin).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("data-users-export=\"all\""));
    assert!(!page.contains("data-users-export=\"filtered\""));
    let (_, page, _, _) = get(state, "/admin/users?q=ro%20ot", &admin).await;
    assert!(page.contains("/admin/users/export.csv?scope=filtered&q=ro%20ot"));
}
