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
    let (status, body, location, _) = request_full(state, method, uri, body, cookie).await;
    (status, body, location)
}

/// Like [`request`], also returning the response's `Set-Cookie` values so a
/// test can carry the enrollment-ceremony cookie from start to confirm the
/// way a real browser would (#1005 ceremony binding).
async fn request_full(
    state: AppState,
    method: &str,
    uri: &str,
    body: &str,
    cookie: &str,
) -> (StatusCode, String, Option<String>, Vec<String>) {
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
    let set_cookies: Vec<String> = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::to_string)
        .collect();
    let body = to_bytes(response.into_body(), 2 << 20).await.unwrap();
    (
        status,
        String::from_utf8(body.to_vec()).unwrap(),
        location,
        set_cookies,
    )
}

/// The `name=value` pair of the ceremony cookie from a start response.
fn ceremony_pair(set_cookies: &[String]) -> String {
    set_cookies
        .iter()
        .find(|c| c.starts_with("__ruscker_mfa_ceremony="))
        .and_then(|c| c.split(';').next())
        .expect("start must set the ceremony cookie")
        .to_string()
}

async fn start(state: AppState, cookie: &str) -> (StatusCode, String) {
    let (status, body, _) = start_with_ceremony(state, cookie).await;
    (status, body)
}

/// Start enrollment and return the session+ceremony cookie header a real
/// browser would send on the following confirm.
async fn start_with_ceremony(state: AppState, cookie: &str) -> (StatusCode, String, String) {
    let (status, body, _, set_cookies) = request_full(
        state,
        "POST",
        "/admin/account/mfa/start",
        "current_password=CorrectPass9%21&next=%2Fapp%2Fdemo%2F",
        cookie,
    )
    .await;
    let combined = if status == StatusCode::OK {
        format!("{cookie}; {}", ceremony_pair(&set_cookies))
    } else {
        cookie.to_string()
    };
    (status, body, combined)
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

    let (start_status, setup, browser_cookie) =
        start_with_ceremony(state.clone(), &user_cookie).await;
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
        &browser_cookie,
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

    let (start_status, _, browser_cookie) =
        start_with_ceremony(state.clone(), &user_cookie).await;
    assert_eq!(start_status, StatusCode::OK);
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
        &browser_cookie,
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
        &browser_cookie,
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
    let (start_status, _, browser_cookie) =
        start_with_ceremony(state.clone(), &user_cookie).await;
    assert_eq!(start_status, StatusCode::OK);
    for _ in 0..5 {
        let (status, _, _) = request(
            state.clone(),
            "POST",
            "/admin/account/mfa/confirm",
            "code=garbage",
            &browser_cookie,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, body, _) = request(
        state,
        "POST",
        "/admin/account/mfa/confirm",
        "code=garbage",
        &browser_cookie,
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
    let (start_status, _, browser_cookie) =
        start_with_ceremony(state.clone(), &user_cookie).await;
    assert_eq!(start_status, StatusCode::OK);
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
            &browser_cookie,
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


/// P1 from the codex review (#1005): a second session for the SAME user —
/// e.g. a stolen session cookie that never passed the password re-auth —
/// must not be able to fish the pending secret out of the wrong-code retry
/// re-render, nor confirm the enrollment.
#[tokio::test]
async fn another_session_without_ceremony_cookie_never_sees_the_secret() {
    let (state, db) = state_with_db(true).await;
    create_user(&db, "dana", false).await;
    let enrolling = cookie(&state, Role::Viewer, Some("dana".into())).await;
    let (start_status, _, _browser) = start_with_ceremony(state.clone(), &enrolling).await;
    assert_eq!(start_status, StatusCode::OK);
    let secret = decrypted_secret(
        &state,
        &ruscker_admin::db::mfa::fetch(&db, "dana").await.unwrap().unwrap(),
    );

    // A different session for the same user, WITHOUT the ceremony cookie.
    let hijacker = cookie(&state, Role::Viewer, Some("dana".into())).await;
    let (status, body, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/confirm",
        "code=000000",
        &hijacker,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(
        !body.contains(&secret),
        "the retry path must never disclose the secret to a session without the ceremony"
    );
    // And a correct code without the ceremony cannot confirm either.
    let code = ruscker_admin::mfa::totp(&secret, "dana")
        .unwrap()
        .generate_current()
        .unwrap();
    let (status, _, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/confirm",
        &format!("code={code}"),
        &hijacker,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(ruscker_admin::db::mfa::fetch(&db, "dana")
        .await
        .unwrap()
        .unwrap()
        .confirmed_at
        .is_none());
}

/// P2 from the codex review (#1005): a re-started enrollment replaces the
/// pending secret AND the ceremony, so a confirm still carrying the OLD
/// ceremony must fail — never confirm secret B with a proof of secret A.
#[tokio::test]
async fn stale_ceremony_after_restart_cannot_confirm() {
    let (state, db) = state_with_db(true).await;
    create_user(&db, "erik", false).await;
    let user_cookie = cookie(&state, Role::Viewer, Some("erik".into())).await;

    let (s1, _, old_browser) = start_with_ceremony(state.clone(), &user_cookie).await;
    assert_eq!(s1, StatusCode::OK);
    let old_secret = decrypted_secret(
        &state,
        &ruscker_admin::db::mfa::fetch(&db, "erik").await.unwrap().unwrap(),
    );

    // Re-start: new secret + new ceremony replace the pending row.
    let (s2, _, _new_browser) = start_with_ceremony(state.clone(), &user_cookie).await;
    assert_eq!(s2, StatusCode::OK);

    // The old browser proves the OLD secret with its OLD ceremony: rejected.
    let code = ruscker_admin::mfa::totp(&old_secret, "erik")
        .unwrap()
        .generate_current()
        .unwrap();
    let (status, _, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/confirm",
        &format!("code={code}"),
        &old_browser,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert!(ruscker_admin::db::mfa::fetch(&db, "erik")
        .await
        .unwrap()
        .unwrap()
        .confirmed_at
        .is_none());
}


/// Codex review round 2 (#1005): a stolen session cookie must not turn
/// /start into an unlimited password oracle — five wrong passwords rate-
/// limit the account, and even the CORRECT password is then refused until
/// the window passes.
#[tokio::test]
async fn start_password_reauth_is_rate_limited() {
    let (state, db) = state_with_db(true).await;
    let username = format!("oracle-{}", uuid::Uuid::new_v4().simple());
    create_user(&db, &username, false).await;
    let user_cookie = cookie(&state, Role::Viewer, Some(username.clone())).await;
    for _ in 0..5 {
        let (status, _, _) = request(
            state.clone(),
            "POST",
            "/admin/account/mfa/start",
            "current_password=wrong-guess",
            &user_cookie,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    let (status, body, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/start",
        "current_password=CorrectPass9%21",
        &user_cookie,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(body.contains("data-mfa-error=\"rate-limited\""));
    assert!(ruscker_admin::db::mfa::fetch(&db, &username)
        .await
        .unwrap()
        .is_none());
}
