//! TOTP enrollment and admin-reset integration coverage (#1005 slice 2).

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::sync::Arc;
use tower::ServiceExt;

const YAML: &str = "proxy:\n  title: Test\n  specs: []\n";
const PASSWORD: &str = "CorrectPass9!";

async fn state_with_db(master_key: bool) -> (AppState, ruscker_admin::db::ConfigDb) {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let path = std::env::temp_dir().join(format!("ruscker-mfa-{}.db", uuid::Uuid::new_v4()));
    let pool = ruscker_admin::db::open(&path).await.expect("open db");
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool);
    let state = AppState {
        config: Arc::new(Config::from_yaml(YAML).expect("parse config")),
        base_path: Arc::from(""),
        locales: Arc::new(ruscker_admin::i18n::Locales::load().expect("load locales")),
        admin_auth: AdminAuth::with_token("break-glass-tok"),
        admin_sessions: Arc::new(ruscker_admin::auth::InMemoryAdminSessionStore::default()),
        log_buffer: None,
        login_limiter: Arc::new(ruscker_admin::auth::LoginRateLimiter::default_policy()),
        api_limiter: Arc::new(ruscker_admin::ratelimit::ApiRateLimiter::new()),
        db: Some(db.clone()),
        images_dir: None,
        master_key: if master_key {
            ruscker_admin::crypto::MasterKey::parse(&"ab".repeat(32)).unwrap()
        } else {
            Default::default()
        },
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
    };
    (state, db)
}

async fn create_user(db: &ruscker_admin::db::ConfigDb, username: &str, must_change: bool) {
    ruscker_admin::db::users::create(
        db,
        username,
        PASSWORD,
        Role::Viewer,
        must_change,
        &[],
        Some("test"),
    )
    .await
    .unwrap();
}

async fn cookie(state: &AppState, role: Role, actor: Option<String>) -> String {
    let id = state.admin_sessions.create(role, actor).await;
    format!("{COOKIE_NAME}={id}")
}

async fn request(
    state: AppState,
    method: &str,
    uri: &str,
    body: &str,
    cookie: &str,
) -> (StatusCode, String, Option<String>) {
    let response = router(state)
        .oneshot(
            Request::builder()
                .method(method)
                .uri(uri)
                .header("cookie", cookie)
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let location = response
        .headers()
        .get("location")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = to_bytes(response.into_body(), 2 << 20).await.unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap(), location)
}

async fn start(state: AppState, cookie: &str) -> (StatusCode, String) {
    let (status, body, _) = request(
        state,
        "POST",
        "/admin/account/mfa/start",
        "current_password=CorrectPass9%21&next=%2Fapp%2Fdemo%2F",
        cookie,
    )
    .await;
    (status, body)
}

fn decrypted_secret(state: &AppState, row: &ruscker_admin::db::mfa::MfaRow) -> String {
    let plaintext = state
        .master_key
        .decrypt(&row.secret_enc, &row.secret_nonce)
        .unwrap();
    String::from_utf8(plaintext.to_vec()).unwrap()
}

#[tokio::test]
async fn full_enrollment_persists_encrypted_pending_then_displays_codes_once() {
    let (state, db) = state_with_db(true).await;
    create_user(&db, "alice", false).await;
    let user_cookie = cookie(&state, Role::Viewer, Some("alice".into())).await;

    let (start_status, setup) = start(state.clone(), &user_cookie).await;
    assert_eq!(start_status, StatusCode::OK);
    assert!(setup.contains("data-mfa-qr"));
    assert!(setup.contains("data-mfa-secret"));

    let pending = ruscker_admin::db::mfa::fetch(&db, "alice")
        .await
        .unwrap()
        .unwrap();
    assert!(pending.confirmed_at.is_none());
    let secret = decrypted_secret(&state, &pending);
    assert_ne!(pending.secret_enc.as_slice(), secret.as_bytes());
    assert_eq!(pending.secret_nonce.len(), 12);
    let code = ruscker_admin::mfa::totp(&secret, "alice")
        .unwrap()
        .generate_current()
        .unwrap();

    let (confirm_status, recovery, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/confirm",
        &format!("code={code}&next=%2Fapp%2Fdemo%2F"),
        &user_cookie,
    )
    .await;
    assert_eq!(confirm_status, StatusCode::OK);
    assert!(recovery.contains("data-recovery-codes"));
    assert!(recovery.contains("/app/demo/"));
    let row = ruscker_admin::db::mfa::fetch(&db, "alice")
        .await
        .unwrap()
        .unwrap();
    assert!(row.confirmed_at.is_some());
    let ruscker_admin::db::ConfigDb::Sqlite(pool) = &db else {
        unreachable!()
    };
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM user_mfa_recovery WHERE username = ? AND used_at IS NULL",
    )
    .bind("alice")
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(count, 10);
    let (enroll_audit,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM audit_log
          WHERE action = 'mfa.enroll' AND actor = 'alice' AND target = 'user:alice'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(enroll_audit, 1);

    let (status, status_page, _) = request(
        state,
        "GET",
        "/admin/account/mfa?next=%2Fapp%2Fdemo%2F",
        "",
        &user_cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(status_page.contains("data-mfa-state=\"enrolled\""));
    assert!(status_page.contains("href=\"/app/demo/\""));
    assert!(!status_page.contains("data-recovery-codes"));
    assert!(!status_page.contains(&secret));
}

#[tokio::test]
async fn wrong_password_is_rejected_and_wrong_code_preserves_pending_for_retry() {
    let (state, db) = state_with_db(true).await;
    let username = format!("retry-{}", uuid::Uuid::new_v4().simple());
    create_user(&db, &username, false).await;
    let user_cookie = cookie(&state, Role::Viewer, Some(username.clone())).await;
    let (bad_password, _, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/start",
        "current_password=wrong",
        &user_cookie,
    )
    .await;
    assert_eq!(bad_password, StatusCode::UNAUTHORIZED);
    assert!(ruscker_admin::db::mfa::fetch(&db, &username)
        .await
        .unwrap()
        .is_none());

    assert_eq!(start(state.clone(), &user_cookie).await.0, StatusCode::OK);
    let pending = ruscker_admin::db::mfa::fetch(&db, &username)
        .await
        .unwrap()
        .unwrap();
    let secret = decrypted_secret(&state, &pending);
    let (wrong, setup, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/confirm",
        "code=garbage",
        &user_cookie,
    )
    .await;
    assert_eq!(wrong, StatusCode::UNAUTHORIZED);
    assert!(setup.contains(&secret));
    assert!(ruscker_admin::db::mfa::fetch(&db, &username)
        .await
        .unwrap()
        .unwrap()
        .confirmed_at
        .is_none());

    let code = ruscker_admin::mfa::totp(&secret, &username)
        .unwrap()
        .generate_current()
        .unwrap();
    let (retry, _, _) = request(
        state,
        "POST",
        "/admin/account/mfa/confirm",
        &format!("code={code}"),
        &user_cookie,
    )
    .await;
    assert_eq!(retry, StatusCode::OK);
}

#[tokio::test]
async fn confirm_rate_limit_trips_after_five_wrong_codes() {
    let (state, db) = state_with_db(true).await;
    let username = format!("limited-{}", uuid::Uuid::new_v4().simple());
    create_user(&db, &username, false).await;
    let user_cookie = cookie(&state, Role::Viewer, Some(username)).await;
    assert_eq!(start(state.clone(), &user_cookie).await.0, StatusCode::OK);
    for _ in 0..5 {
        let (status, _, _) = request(
            state.clone(),
            "POST",
            "/admin/account/mfa/confirm",
            "code=garbage",
            &user_cookie,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, body, _) = request(
        state,
        "POST",
        "/admin/account/mfa/confirm",
        "code=garbage",
        &user_cookie,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(body.contains("data-mfa-error=\"rate-limited\""));
}

#[tokio::test]
async fn enrollment_fails_closed_without_master_key() {
    let (state, db) = state_with_db(false).await;
    create_user(&db, "nokey", false).await;
    let user_cookie = cookie(&state, Role::Viewer, Some("nokey".into())).await;
    let (status, body) = start(state, &user_cookie).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains("RUSCKER_MASTER_KEY"));
    assert!(ruscker_admin::db::mfa::fetch(&db, "nokey")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn admin_reset_removes_factor_and_codes_audits_and_allows_reenrollment() {
    let (state, db) = state_with_db(true).await;
    create_user(&db, "carol", false).await;
    create_user(&db, "root", false).await;
    let user_cookie = cookie(&state, Role::Viewer, Some("carol".into())).await;
    assert_eq!(start(state.clone(), &user_cookie).await.0, StatusCode::OK);
    let pending = ruscker_admin::db::mfa::fetch(&db, "carol")
        .await
        .unwrap()
        .unwrap();
    let secret = decrypted_secret(&state, &pending);
    let code = ruscker_admin::mfa::totp(&secret, "carol")
        .unwrap()
        .generate_current()
        .unwrap();
    assert_eq!(
        request(
            state.clone(),
            "POST",
            "/admin/account/mfa/confirm",
            &format!("code={code}"),
            &user_cookie,
        )
        .await
        .0,
        StatusCode::OK
    );

    let admin_cookie = cookie(&state, Role::Admin, Some("root".into())).await;
    let (edit_status, edit_body, _) = request(
        state.clone(),
        "GET",
        "/admin/users/carol/edit",
        "",
        &admin_cookie,
    )
    .await;
    assert_eq!(edit_status, StatusCode::OK);
    assert!(edit_body.contains("data-mfa-admin-state=\"configured\""));
    let (reset, _, location) = request(
        state.clone(),
        "POST",
        "/admin/users/carol/mfa/reset",
        "",
        &admin_cookie,
    )
    .await;
    assert_eq!(reset, StatusCode::SEE_OTHER);
    assert_eq!(
        location.as_deref(),
        Some("/admin/users/carol/edit?flash=mfa-reset")
    );
    assert!(ruscker_admin::db::mfa::fetch(&db, "carol")
        .await
        .unwrap()
        .is_none());
    let ruscker_admin::db::ConfigDb::Sqlite(pool) = &db else {
        unreachable!()
    };
    let (codes,): (i64,) =
        sqlx::query_as("SELECT COUNT(*) FROM user_mfa_recovery WHERE username = 'carol'")
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(codes, 0);
    let (audits,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM audit_log
          WHERE action = 'mfa.reset' AND actor = 'root' AND target = 'user:carol'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(audits, 1);
    assert_eq!(start(state, &user_cookie).await.0, StatusCode::OK);
}

#[tokio::test]
async fn break_glass_cannot_enroll_and_must_change_user_is_pinned_to_password() {
    let (state, db) = state_with_db(true).await;
    create_user(&db, "firstlogin", true).await;
    let token_cookie = cookie(&state, Role::Admin, None).await;
    let (break_glass, body) = start(state.clone(), &token_cookie).await;
    assert_eq!(break_glass, StatusCode::FORBIDDEN);
    assert!(body.contains("data-mfa-error=\"break-glass\""));

    let first_cookie = cookie(&state, Role::Viewer, Some("firstlogin".into())).await;
    let (status, _, location) =
        request(state, "GET", "/admin/account/mfa", "", &first_cookie).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location.as_deref(), Some("/admin/account/password"));
}
