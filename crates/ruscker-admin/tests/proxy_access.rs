//! Integration test for the `/app` + `/api` access-enforcement guard
//! (#155, Slice 4). Hiding a card on the landing doesn't stop a direct
//! request; this guard rejects one for a spec the viewer can't reach.
//!
//! No container backend is wired, so an *allowed* request falls through
//! to the backend check (503) — we assert it was NOT rejected (no 403 /
//! login redirect). A *denied* request returns before the backend is
//! ever consulted.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use axum::response::Response;
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
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
    - id: open-app
      display-name: Open App
      container-image: demo/img
    - id: analysts-app
      display-name: Analysts App
      container-image: demo/img
      access-groups: [analysts]
    - id: open-api
      display-name: Open API
      type: api
      container-image: demo/img
    - id: locked-api
      display-name: Locked API
      type: api
      container-image: demo/img
      access-groups: [analysts]
"#;

async fn open_db() -> ConfigDb {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ruscker-proxy-access-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    ConfigDb::Sqlite(ruscker_admin::db::open(&path).await.unwrap())
}

async fn app_state(db: ConfigDb) -> AppState {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let config = Config::from_yaml(CONFIG).expect("parse config");
    let locales = Locales::load().expect("load locales");
    AppState {
        config: Arc::new(config),
        base_path: Arc::from(""),
        locales: Arc::new(locales),
        admin_auth: AdminAuth {
            admin: Some(Arc::from("test-token")),
        },
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
        catalog_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        access_counter: Arc::new(ruscker_admin::access_counter::AccessCounter::default()),
        alerts: ruscker_admin::alerts::AlertSink::default(),
    }
}

async fn get_response(state: AppState, path: &str, cookie: Option<String>) -> Response {
    let app = router(state);
    let mut req = Request::builder().method("GET").uri(path);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap()
}

async fn get(state: AppState, path: &str, cookie: Option<String>) -> StatusCode {
    get_response(state, path, cookie).await.status()
}

/// A request that the guard lets through lands on the "no backend"
/// 503 — never a 403 or a login redirect.
const ALLOWED: StatusCode = StatusCode::SERVICE_UNAVAILABLE;

#[tokio::test]
async fn open_spec_reachable_anonymously() {
    let state = app_state(open_db().await).await;
    assert_eq!(get(state, "/app/open-app/", None).await, ALLOWED);
}

#[tokio::test]
async fn open_api_reachable_anonymously() {
    let state = app_state(open_db().await).await;
    assert_eq!(get(state, "/api/open-api/", None).await, ALLOWED);
}

#[tokio::test]
async fn restricted_app_redirects_anonymous_to_login() {
    let state = app_state(open_db().await).await;
    // Redirect::to ⇒ 303 See Other. The original path and query are
    // carried in `next`, so a successful login can return here (#924).
    let response = get_response(
        state,
        "/app/analysts-app/report%20name?tab=a%2Fb",
        None,
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::SEE_OTHER,
        "anon → login redirect"
    );
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some(
            "/admin/login?next=%2Fapp%2Fanalysts-app%2Freport%2520name%3Ftab%3Da%252Fb"
        )
    );
}

#[tokio::test]
async fn restricted_app_login_redirect_keeps_the_base_path() {
    let mut state = app_state(open_db().await).await;
    state.base_path = Arc::from("/box");
    let response = get_response(state, "/box/app/analysts-app/report?tab=2", None).await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok()),
        Some("/box/admin/login?next=%2Fapp%2Fanalysts-app%2Freport%3Ftab%3D2")
    );
}

#[tokio::test]
async fn restricted_api_forbids_anonymous() {
    let state = app_state(open_db().await).await;
    assert_eq!(
        get(state, "/api/locked-api/", None).await,
        StatusCode::FORBIDDEN,
        "anon API client gets 403, never a redirect"
    );
}

#[tokio::test]
async fn admin_session_reaches_restricted_app() {
    let state = app_state(open_db().await).await;
    let sid = state.admin_sessions.create(Role::Admin, None).await;
    assert_eq!(
        get(state, "/app/analysts-app/", Some(format!("{COOKIE_NAME}={sid}"))).await,
        ALLOWED,
        "admin passes the guard"
    );
}

#[tokio::test]
async fn group_member_reaches_restricted_app() {
    let db = open_db().await;
    ruscker_admin::db::users::create(
        &db,
        "alice",
        "alicepass1",
        Role::Viewer,
        false,
        &["analysts".to_string()],
        Some("admin"),
    )
    .await
    .unwrap();
    let state = app_state(db).await;
    let sid = state
        .admin_sessions
        .create(Role::Viewer, Some("alice".to_string()))
        .await;
    assert_eq!(
        get(state, "/app/analysts-app/", Some(format!("{COOKIE_NAME}={sid}"))).await,
        ALLOWED,
        "alice is in analysts → guard passes"
    );
}

#[tokio::test]
async fn non_member_forbidden_from_restricted_app() {
    let db = open_db().await;
    ruscker_admin::db::users::create(
        &db,
        "dave",
        "davepass12",
        Role::Viewer,
        false,
        &[],
        Some("admin"),
    )
    .await
    .unwrap();
    let state = app_state(db).await;
    let sid = state
        .admin_sessions
        .create(Role::Viewer, Some("dave".to_string()))
        .await;
    // Logged in but not in the group ⇒ 403, not a login redirect.
    assert_eq!(
        get(state, "/app/analysts-app/", Some(format!("{COOKIE_NAME}={sid}"))).await,
        StatusCode::FORBIDDEN,
        "dave lacks the group → 403"
    );
}
