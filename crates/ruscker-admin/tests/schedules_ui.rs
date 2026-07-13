//! Integration tests for the Schedules admin page (#986 slice C):
//! create (with server-side cron/spec validation), toggle, delete, and
//! the admin-only gate. Harness copied from `proxy_access.rs` — a full
//! `router()` over a temp-file SQLite, sessions minted directly in the
//! shared store (the `rbac.rs` trick).

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
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
    - id: etl-app
      display-name: ETL App
      container-image: demo/etl
    # No container-image ⇒ auto-classified External (nothing to run).
    - id: external-app
      display-name: External App
"#;

async fn open_db() -> ConfigDb {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ruscker-schedules-ui-{}-{}.db",
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
        catalog_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        access_counter: Arc::new(ruscker_admin::access_counter::AccessCounter::default()),
        alerts: ruscker_admin::alerts::AlertSink::default(),
    }
}

/// Mint an admin session straight into the shared store and return the
/// cookie header value.
async fn admin_cookie(state: &AppState) -> String {
    let sid = state
        .admin_sessions
        .create(Role::Admin, Some("root".into()))
        .await;
    format!("{COOKIE_NAME}={sid}")
}

async fn get(state: AppState, path: &str, cookie: &str) -> (StatusCode, String) {
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
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

/// POST a form; returns (status, Location header).
async fn post(state: AppState, uri: &str, body: &str, cookie: &str) -> (StatusCode, String) {
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let loc = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    (status, loc)
}

#[tokio::test]
async fn create_valid_schedule_persists_and_renders() {
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;

    let (status, loc) = post(
        state.clone(),
        "/admin/schedules",
        "spec_id=etl-app&cron=0+3+*+*+*&cmd=Rscript%0Aetl.R&timeout_mins=90",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/schedules?flash=created");

    // Persisted with the parsed argv + timeout in seconds.
    let db = state.db.clone().unwrap();
    let all = ruscker_admin::db::schedules::list_all(&db).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].spec_id, "etl-app");
    assert_eq!(all[0].cron, "0 3 * * *");
    assert_eq!(
        all[0].cmd_override(),
        Some(vec!["Rscript".to_string(), "etl.R".to_string()])
    );
    assert_eq!(all[0].timeout_secs, Some(90 * 60));
    assert!(all[0].enabled);

    // And the page shows it (spec id + the enabled badge's toggle form).
    let (status, html) = get(state, "/admin/schedules?flash=created", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("etl-app"), "schedule row renders");
    assert!(html.contains("0 3 * * *"), "cron renders");
    assert!(html.contains("/toggle"), "toggle action renders");
    // External specs never appear in the create form's app select.
    assert!(!html.contains(r#"value="external-app""#), "External is not schedulable");
}

#[tokio::test]
async fn invalid_cron_is_rejected_and_not_persisted() {
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;

    let (status, loc) = post(
        state.clone(),
        "/admin/schedules",
        "spec_id=etl-app&cron=not+a+cron&cmd=&timeout_mins=",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/schedules?flash=bad-cron");

    let db = state.db.clone().unwrap();
    assert!(ruscker_admin::db::schedules::list_all(&db).await.unwrap().is_empty());
}

#[tokio::test]
async fn unknown_or_external_spec_is_rejected() {
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;

    // Not in the catalog at all.
    let (_, loc) = post(
        state.clone(),
        "/admin/schedules",
        "spec_id=no-such-app&cron=0+3+*+*+*&cmd=&timeout_mins=",
        &cookie,
    )
    .await;
    assert_eq!(loc, "/admin/schedules?flash=bad-spec");

    // In the catalog but External (no container-image → nothing to run).
    let (_, loc) = post(
        state.clone(),
        "/admin/schedules",
        "spec_id=external-app&cron=0+3+*+*+*&cmd=&timeout_mins=",
        &cookie,
    )
    .await;
    assert_eq!(loc, "/admin/schedules?flash=bad-spec");

    let db = state.db.clone().unwrap();
    assert!(ruscker_admin::db::schedules::list_all(&db).await.unwrap().is_empty());
}

#[tokio::test]
async fn toggle_and_delete_round_trip() {
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;
    let db = state.db.clone().unwrap();

    ruscker_admin::db::schedules::insert(&db, "etl-app", "0 3 * * *", None, None, Some("root"))
        .await
        .unwrap();
    let id = ruscker_admin::db::schedules::list_all(&db).await.unwrap()[0].id;

    // Toggle: enabled → disabled.
    let (status, loc) = post(
        state.clone(),
        &format!("/admin/schedules/{id}/toggle"),
        "",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/schedules?flash=toggled");
    assert!(!ruscker_admin::db::schedules::list_all(&db).await.unwrap()[0].enabled);

    // Toggle again: back to enabled.
    let (_, loc) = post(
        state.clone(),
        &format!("/admin/schedules/{id}/toggle"),
        "",
        &cookie,
    )
    .await;
    assert_eq!(loc, "/admin/schedules?flash=toggled");
    assert!(ruscker_admin::db::schedules::list_all(&db).await.unwrap()[0].enabled);

    // Delete removes the row.
    let (_, loc) = post(
        state.clone(),
        &format!("/admin/schedules/{id}/delete"),
        "",
        &cookie,
    )
    .await;
    assert_eq!(loc, "/admin/schedules?flash=deleted");
    assert!(ruscker_admin::db::schedules::list_all(&db).await.unwrap().is_empty());
}

#[tokio::test]
async fn schedules_are_admin_only() {
    let state = app_state(open_db().await).await;
    let editor = state
        .admin_sessions
        .create(Role::Editor, Some("ed".into()))
        .await;
    let cookie = format!("{COOKIE_NAME}={editor}");

    let (status, _) = get(state.clone(), "/admin/schedules", &cookie).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "Editor cannot view schedules");

    let (status, _) = post(
        state,
        "/admin/schedules",
        "spec_id=etl-app&cron=0+3+*+*+*&cmd=&timeout_mins=",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "Editor cannot create schedules");
}
