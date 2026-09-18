//! Server-side sort of the Users table (#1057): the order lives in the
//! query so it covers every page, the header announces it, and the
//! markup carries the live-region hooks the layout script binds to.

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
    let path = std::env::temp_dir().join(format!("ruscker-users-sort-{}.db", uuid::Uuid::new_v4()));
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

async fn get(state: AppState, uri: &str, cookie: &str) -> (StatusCode, String) {
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
    let body = to_bytes(resp.into_body(), 2 << 20).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

/// Usernames in row order, from the avatar markers (one per table row).
fn row_names(body: &str) -> Vec<String> {
    body.split("data-avatar=\"")
        .skip(1)
        .map(|rest| rest.split('"').next().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn users_table_sorts_server_side_and_carries_live_region_hooks() {
    let (state, pool) = state_with_db().await;
    let db = ConfigDb::Sqlite(pool);
    for (name, role) in [("root", Role::Admin), ("zoe", Role::Viewer), ("ana", Role::Editor), ("mia", Role::Viewer)] {
        ruscker_admin::db::users::create(&db, name, "Correct#Pass9", role, false, &[], Some("t"))
            .await
            .unwrap();
    }
    let id = state.admin_sessions.create(Role::Admin, Some("root".into())).await;
    let cookie = format!("{COOKIE_NAME}={id}");

    // Default listing: newest first, no sort noise in the markup's links.
    let (status, page) = get(state.clone(), "/admin/users", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("data-live=\"#live-users\""));
    assert!(page.contains("id=\"live-users\" data-live-region"));
    assert!(page.contains("data-server-sort=\"#users-filter\""));
    assert!(page.contains("aria-sort=\"descending\""), "created desc is the active default");
    assert!(page.contains("name=\"sort\" value=\"created\" data-default=\"created\""));

    let (_, asc) = get(state.clone(), "/admin/users?sort=username&dir=asc", &cookie).await;
    assert_eq!(row_names(&asc), ["ana", "mia", "root", "zoe"]);
    assert!(asc.contains("data-sort-key=\"username\" data-sort-dir=\"asc\" aria-sort=\"ascending\""));

    let (_, desc) = get(state.clone(), "/admin/users?sort=username&dir=desc", &cookie).await;
    assert_eq!(row_names(&desc), ["zoe", "root", "mia", "ana"]);

    let (_, by_role) = get(state.clone(), "/admin/users?sort=role&dir=asc&q=", &cookie).await;
    assert_eq!(row_names(&by_role), ["root", "ana", "mia", "zoe"], "admin < editor < viewer, then username");

    // Garbage sort falls back to the default instead of erroring.
    let (status, junk) = get(state, "/admin/users?sort=%27%3B+DROP&dir=up", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(junk.contains("aria-sort=\"descending\""));
}
