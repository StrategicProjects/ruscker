//! Integration tests for the "Atividades dos usuários" admin page
//! (#1021 fatia 3): admin-only gate, rendered rows, filters (event/user),
//! and server-side pagination. Harness mirrors `schedules_ui.rs` — a full
//! `router()` over a temp-file SQLite with a session minted directly.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use ruscker_admin::activity::{ActivityEvent, AuthMethod};
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::db::{user_activity, ConfigDb};
use ruscker_admin::i18n::Locales;
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::sync::Arc;
use tower::ServiceExt;

const CONFIG: &str = "proxy:\n  title: Test\n  specs: []\n";

async fn open_db() -> ConfigDb {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ruscker-activity-ui-{}-{}.db",
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

async fn admin_cookie(state: &AppState) -> String {
    let sid = state
        .admin_sessions
        .create(Role::Admin, Some("root".into()))
        .await;
    format!("{COOKIE_NAME}={sid}")
}

async fn get(state: AppState, path: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let app = router(state);
    let mut req = Request::builder().method("GET").uri(path);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let resp = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// One `data-avatar="…"` attribute is rendered per table row (the avatar
/// script references the bare `[data-avatar]` selector, without `="`, so it
/// isn't counted), giving the number of rows actually on the page.
fn row_count(body: &str) -> usize {
    body.matches("data-avatar=\"").count()
}

#[tokio::test]
async fn requires_admin() {
    let db = open_db().await;
    let state = app_state(db).await;
    // No session → the guard bounces (never 200).
    let (status, _) = get(state, "/admin/activity", None).await;
    assert_ne!(status, StatusCode::OK);
}

#[tokio::test]
async fn renders_rows_and_event_filter_narrows() {
    let db = open_db().await;
    user_activity::insert_batch(
        &db,
        &[
            ActivityEvent::login_success("alice", AuthMethod::Password, "l1", None),
            ActivityEvent::app_access(Some("alice".into()), "sales-dash", "a1", None, None),
            ActivityEvent::app_access(None, "public-dash", "a2", None, None),
        ],
    )
    .await
    .unwrap();
    let state = app_state(db).await;
    let cookie = admin_cookie(&state).await;

    // Unfiltered: all three rows, and the app names + user appear.
    let (status, body) = get(state.clone(), "/admin/activity", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(row_count(&body), 3);
    assert!(body.contains("sales-dash"));
    assert!(body.contains("alice"));

    // event=login.success → only the login row (no app spec in the table).
    let (status, body) = get(
        state.clone(),
        "/admin/activity?event=login.success",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(row_count(&body), 1);

    // event=app.access → the two accesses.
    let (_, body) = get(state, "/admin/activity?event=app.access", Some(&cookie)).await;
    assert_eq!(row_count(&body), 2);
}

#[tokio::test]
async fn user_filter_narrows_to_one_user() {
    let db = open_db().await;
    user_activity::insert_batch(
        &db,
        &[
            ActivityEvent::app_access(Some("alice".into()), "s", "a1", None, None),
            ActivityEvent::app_access(Some("bob".into()), "s", "a2", None, None),
            ActivityEvent::login_success("alice", AuthMethod::Password, "l1", None),
        ],
    )
    .await
    .unwrap();
    let state = app_state(db).await;
    let cookie = admin_cookie(&state).await;

    // alice: her app access + her login = 2 rows.
    let (status, body) = get(state, "/admin/activity?user=alice", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(row_count(&body), 2);
}

#[tokio::test]
async fn paginates_server_side() {
    let db = open_db().await;
    // 60 accesses → 50 on page 1, 10 on page 2 (newest first).
    let events: Vec<_> = (0..60)
        .map(|i| ActivityEvent::app_access(Some("u".into()), "s", format!("k{i}"), None, None))
        .collect();
    user_activity::insert_batch(&db, &events).await.unwrap();
    let state = app_state(db).await;
    let cookie = admin_cookie(&state).await;

    let (status, body1) = get(state.clone(), "/admin/activity", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(row_count(&body1), 50, "first page caps at the page size");

    let (status, body2) = get(state, "/admin/activity?page=2", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(row_count(&body2), 10, "second page has the remainder");
}
