//! Integration tests for the user-account auth flow (#107): token
//! bootstrap, username/password login, and the last-admin guard.
//!
//! Uses a real temp-file SQLite (the lib's in-memory helper is
//! test-private) and `Router::oneshot`. Sessions for the guard test
//! are minted directly in the shared store, the same trick as
//! `rbac.rs`.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::sync::Arc;
use tower::ServiceExt;

const YAML: &str = "proxy:\n  title: Test\n  specs: []\n";

async fn state_with_db() -> (AppState, sqlx::SqlitePool) {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let path = std::env::temp_dir().join(format!("ruscker-users-{}.db", uuid::Uuid::new_v4()));
    let pool = ruscker_admin::db::open(&path).await.expect("open db");
    let config = Config::from_yaml(YAML).expect("parse config");
    let locales = ruscker_admin::i18n::Locales::load().expect("load locales");
    let state = AppState {
        config: Arc::new(config),
        locales: Arc::new(locales),
        admin_auth: AdminAuth::with_token("break-glass-tok"),
        admin_sessions: Default::default(),
        log_buffer: None,
        login_limiter: Arc::new(ruscker_admin::auth::LoginRateLimiter::default_policy()),
        api_limiter: Arc::new(ruscker_admin::ratelimit::ApiRateLimiter::new()),
        db: Some(ruscker_admin::db::ConfigDb::Sqlite(pool.clone())),
        images_dir: None,
        master_key: Default::default(),
        backend: None,
        replicas: Arc::new(tokio::sync::RwLock::new(Default::default())),
        cookie_key: ruscker_proxy::sticky::CookieKey::random(),
        spawn_locks: Arc::new(dashmap::DashMap::new()),
        sessions: Arc::new(ruscker_admin::sessions::InMemorySessionStore::new()),
        metrics: ruscker_admin::metrics_cache::MetricsCache::new(),
        draining: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    };
    (state, pool)
}

async fn post(
    state: AppState,
    uri: &str,
    body: &str,
    cookie: Option<&str>,
) -> (StatusCode, String) {
    let app = router(state);
    let mut b = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(c) = cookie {
        b = b.header("cookie", c);
    }
    let resp = app
        .oneshot(b.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = resp.status();
    let loc = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    (status, loc)
}

#[tokio::test]
async fn token_login_bootstraps_to_setup() {
    let (state, _pool) = state_with_db().await;
    // Fresh DB, no admin account ⇒ token login routes to setup.
    let (status, loc) = post(state, "/admin/login/token", "token=break-glass-tok", None).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/setup");
}

#[tokio::test]
async fn wrong_token_is_rejected() {
    let (state, _pool) = state_with_db().await;
    let (status, _) = post(state, "/admin/login/token", "token=nope", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn password_login_succeeds_and_wrong_fails() {
    let (state, pool) = state_with_db().await;
    ruscker_admin::db::users::create(&ruscker_admin::db::ConfigDb::Sqlite(pool.clone()), "alice", "alicepass1", Role::Editor, false, None)
        .await
        .unwrap();

    let (ok_status, ok_loc) = post(
        state.clone(),
        "/admin/login",
        "username=alice&password=alicepass1",
        None,
    )
    .await;
    assert_eq!(ok_status, StatusCode::SEE_OTHER);
    assert_eq!(ok_loc, "/admin/dashboard");

    let (bad_status, _) = post(state, "/admin/login", "username=alice&password=WRONG", None).await;
    assert_eq!(bad_status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn first_login_with_must_change_redirects_to_password() {
    let (state, pool) = state_with_db().await;
    // must_change = true ⇒ first login lands on the change-password page.
    ruscker_admin::db::users::create(&ruscker_admin::db::ConfigDb::Sqlite(pool.clone()), "bob", "bobpass12", Role::Viewer, true, None)
        .await
        .unwrap();
    let (status, loc) = post(
        state,
        "/admin/login",
        "username=bob&password=bobpass12",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/account/password");
}

#[tokio::test]
async fn last_admin_cannot_be_deleted() {
    let (state, pool) = state_with_db().await;
    ruscker_admin::db::users::create(&ruscker_admin::db::ConfigDb::Sqlite(pool.clone()), "root", "rootpass1", Role::Admin, false, None)
        .await
        .unwrap();
    // Mint an admin session directly (the shared store the router reads).
    let sid = state
        .admin_sessions
        .create(Role::Admin, Some("root".into()));
    let cookie = format!("{COOKIE_NAME}={sid}");

    let (status, loc) = post(state, "/admin/users/root/delete", "", Some(&cookie)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(loc.contains("flash=last-admin"), "got {loc}");
    // The sole admin survives.
    assert_eq!(
        ruscker_admin::db::users::count_admins(&ruscker_admin::db::ConfigDb::Sqlite(pool.clone())).await.unwrap(),
        1
    );
}
