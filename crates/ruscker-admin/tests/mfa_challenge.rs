//! MFA challenge, trusted-device grant, replay, and revocation coverage.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use chrono::{Duration, Utc};
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::mfa::{MfaDecision, DEVICE_COOKIE};
use ruscker_admin::{router, AppState};
use ruscker_config::{Config, Spec};
use std::sync::Arc;
use tower::ServiceExt;
use tower_cookies::{Cookie, Cookies};

const YAML: &str = "proxy:\n  title: Test\n  specs: []\n";
const PASSWORD: &str = "CorrectPass9!";

async fn state_with_db() -> (AppState, ruscker_admin::db::ConfigDb) {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let path = std::env::temp_dir().join(format!("ruscker-mfa-challenge-{}.db", uuid::Uuid::new_v4()));
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
        master_key: ruscker_admin::crypto::MasterKey::parse(&"ab".repeat(32)).unwrap(),
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

async fn create_user(db: &ruscker_admin::db::ConfigDb, username: &str) {
    ruscker_admin::db::users::create(
        db,
        username,
        PASSWORD,
        Role::Viewer,
        false,
        &[],
        Some("test"),
    )
    .await
    .unwrap();
}

async fn session(state: &AppState, username: &str) -> (String, String) {
    let id = state
        .admin_sessions
        .create(Role::Viewer, Some(username.to_string()))
        .await;
    (id.clone(), format!("{COOKIE_NAME}={id}"))
}

async fn request(
    state: AppState,
    method: &str,
    uri: &str,
    body: &str,
    cookie: &str,
) -> (StatusCode, Option<String>, Vec<String>, String) {
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
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);
    let set_cookies = response
        .headers()
        .get_all("set-cookie")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .map(str::to_string)
        .collect();
    let bytes = to_bytes(response.into_body(), 2 << 20).await.unwrap();
    (status, location, set_cookies, String::from_utf8(bytes.to_vec()).unwrap())
}

fn cookie_pair(set_cookies: &[String], name: &str) -> String {
    set_cookies
        .iter()
        .find(|c| c.starts_with(&format!("{name}=")))
        .and_then(|c| c.split(';').next())
        .unwrap_or_else(|| panic!("response must set {name}"))
        .to_string()
}

fn jar(device_pair: &str) -> Cookies {
    let (name, value) = device_pair.split_once('=').unwrap();
    let cookies = Cookies::default();
    cookies.add(Cookie::new(name.to_string(), value.to_string()));
    cookies
}

fn spec(days: u16) -> Spec {
    serde_yaml_ng::from_str(&format!(
        "id: guarded\ncontainer-image: example/app\nrequire-mfa: true\nmfa-validity-days: {days}\n"
    ))
    .unwrap()
}

fn unprotected_spec() -> Spec {
    serde_yaml_ng::from_str("id: open\ncontainer-image: example/app\n").unwrap()
}

async fn enroll(
    state: &AppState,
    db: &ruscker_admin::db::ConfigDb,
    username: &str,
    session_cookie: &str,
) -> String {
    let (status, _, set_cookies, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/start",
        "current_password=CorrectPass9%21",
        session_cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let browser_cookie = format!(
        "{session_cookie}; {}",
        cookie_pair(&set_cookies, "__ruscker_mfa_ceremony")
    );
    let row = ruscker_admin::db::mfa::fetch(db, username)
        .await
        .unwrap()
        .unwrap();
    let secret = String::from_utf8(
        state
            .master_key
            .decrypt(&row.secret_enc, &row.secret_nonce)
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    let code = ruscker_admin::mfa::totp(&secret, username)
        .unwrap()
        .generate_current()
        .unwrap();
    let (status, _, _, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/confirm",
        &format!("code={code}"),
        &browser_cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    secret
}

fn next_totp(secret: &str, username: &str) -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    ruscker_admin::mfa::totp(secret, username)
        .unwrap()
        .generate(now + 30)
}

async fn challenge_totp(
    state: &AppState,
    session_cookie: &str,
    code: &str,
) -> (StatusCode, Vec<String>) {
    let (status, location, cookies, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/challenge",
        &format!("kind=totp&code={code}&next=%2Fapp%2Fguarded%2F"),
        session_cookie,
    )
    .await;
    if status == StatusCode::SEE_OTHER {
        assert_eq!(location.as_deref(), Some("/app/guarded/"));
    }
    (status, cookies)
}

async fn grant_count(db: &ruscker_admin::db::ConfigDb, username: &str) -> i64 {
    let ruscker_admin::db::ConfigDb::Sqlite(pool) = db else {
        unreachable!()
    };
    sqlx::query_as::<_, (i64,)>("SELECT COUNT(*) FROM user_mfa_grants WHERE username = ?")
        .bind(username)
        .fetch_one(pool)
        .await
        .unwrap()
        .0
}

async fn create_eval_grant(
    db: &ruscker_admin::db::ConfigDb,
    username: &str,
    session_id: &str,
    factor: chrono::DateTime<Utc>,
    token: &str,
    verified_at: chrono::DateTime<Utc>,
    expires_at: chrono::DateTime<Utc>,
) -> String {
    let hash = ruscker_admin::mfa::hash_device_token(token).unwrap();
    // One grant per (username, session_binding) now (#1005), so give each
    // eval scenario a distinct binding — they model separate browsers. The
    // 7-day specs these exercise don't check the binding, so this is inert
    // for the behavior under test.
    ruscker_admin::db::mfa_grants::create(
        db,
        username,
        &hash,
        &ruscker_admin::mfa::session_binding(&format!("{session_id}:{token}")),
        factor,
        verified_at,
        expires_at,
        0,
    )
    .await
    .unwrap()
        .expect("grant issued under current epoch")
}

#[tokio::test]
async fn totp_grant_satisfies_age_window_and_session_binding_and_rejects_replay() {
    let (state, db) = state_with_db().await;
    create_user(&db, "alice").await;
    let (session_id, session_cookie) = session(&state, "alice").await;
    let secret = enroll(&state, &db, "alice", &session_cookie).await;
    let code = next_totp(&secret, "alice");
    let (status, set_cookies) = challenge_totp(&state, &session_cookie, &code).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let device = cookie_pair(&set_cookies, DEVICE_COOKIE);
    assert!(set_cookies.iter().any(|c| c.contains("HttpOnly")));
    assert!(set_cookies.iter().any(|c| c.contains("SameSite=Strict")));
    assert!(set_cookies.iter().any(|c| c.contains("Path=/")));

    let cookies = jar(&device);
    assert_eq!(
        ruscker_admin::mfa::evaluate(&state, "alice", &session_id, &cookies, &spec(7)).await,
        MfaDecision::Satisfied
    );
    assert_eq!(
        ruscker_admin::mfa::evaluate(&state, "alice", &session_id, &cookies, &spec(0)).await,
        MfaDecision::Satisfied
    );
    let (new_session_id, _) = session(&state, "alice").await;
    assert_eq!(
        ruscker_admin::mfa::evaluate(&state, "alice", &new_session_id, &cookies, &spec(0)).await,
        MfaDecision::ChallengeRequired
    );
    assert_eq!(
        ruscker_admin::mfa::evaluate(&state, "alice", &new_session_id, &cookies, &spec(7)).await,
        MfaDecision::Satisfied
    );
    assert_eq!(
        challenge_totp(&state, &session_cookie, &code).await.0,
        StatusCode::UNAUTHORIZED
    );
    let ruscker_admin::db::ConfigDb::Sqlite(pool) = &db else {
        unreachable!()
    };
    let verified_audits: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM audit_log WHERE actor = 'alice' AND action = 'mfa.verify'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    // Two mfa.verify: enrollment's initial device grant + the explicit
    // challenge this test drives (#1005 slice 4 — enrollment now establishes
    // the first proof so the user isn't bounced to a redundant challenge).
    assert_eq!(verified_audits.0, 2);
}

#[tokio::test]
async fn recovery_code_consumes_once_audits_and_creates_normal_grant() {
    let (state, db) = state_with_db().await;
    create_user(&db, "recover").await;
    let (session_id, session_cookie) = session(&state, "recover").await;
    enroll(&state, &db, "recover", &session_cookie).await;
    let recovery = "ABCD234567";
    ruscker_admin::db::mfa::replace_recovery_codes(
        &db,
        "recover",
        &[ruscker_admin::mfa::hash_recovery_code(recovery).unwrap()],
    )
    .await
    .unwrap();
    let (status, _, set_cookies, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/challenge",
        &format!("kind=recovery&code={recovery}"),
        &session_cookie,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let device = cookie_pair(&set_cookies, DEVICE_COOKIE);
    assert_eq!(
        ruscker_admin::mfa::evaluate(&state, "recover", &session_id, &jar(&device), &spec(7)).await,
        MfaDecision::Satisfied
    );
    assert_eq!(
        request(
            state.clone(),
            "POST",
            "/admin/account/mfa/challenge",
            &format!("kind=recovery&code={recovery}"),
            &session_cookie,
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let ruscker_admin::db::ConfigDb::Sqlite(pool) = &db else {
        unreachable!()
    };
    let audits: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM audit_log WHERE actor = 'recover' AND action = 'mfa.recovery_used'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(audits.0, 1);
}

/// The user's current security epoch — direct grant creation in tests must
/// read it the same way the challenge route does (revocations bump it).
async fn current_epoch(db: &ruscker_admin::db::ConfigDb, username: &str) -> i64 {
    ruscker_admin::db::mfa::fetch(db, username)
        .await
        .unwrap()
        .map(|row| row.security_epoch)
        .unwrap_or(0)
}

#[tokio::test]
async fn password_reset_factor_reset_and_forget_revoke_grants() {
    let (state, db) = state_with_db().await;
    create_user(&db, "revoke").await;
    let (session_id, session_cookie) = session(&state, "revoke").await;
    let secret = enroll(&state, &db, "revoke", &session_cookie).await;
    let (_, set_cookies) = challenge_totp(&state, &session_cookie, &next_totp(&secret, "revoke")).await;
    let device = cookie_pair(&set_cookies, DEVICE_COOKIE);
    assert_eq!(grant_count(&db, "revoke").await, 1);
    let combined = format!("{session_cookie}; {device}");
    let (status, _, cleared, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/device/forget",
        "",
        &combined,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(grant_count(&db, "revoke").await, 0);
    assert!(cleared
        .iter()
        .any(|c| c.starts_with(DEVICE_COOKIE) && c.contains("Path=/")));

    // Direct grants exercise the silent cleanup hooks without waiting for a
    // fresh TOTP step between each security mutation.
    let factor = ruscker_admin::db::mfa::fetch(&db, "revoke")
        .await
        .unwrap()
        .unwrap()
        .confirmed_at
        .unwrap();
    for token in ["all-one", "all-two"] {
        // Distinct bindings = two browsers; revoke-all must clear both.
        ruscker_admin::db::mfa_grants::create(
            &db,
            "revoke",
            &ruscker_admin::mfa::hash_device_token(token).unwrap(),
            &format!("binding-{token}"),
            factor,
            Utc::now(),
            Utc::now() + Duration::days(30),
        0,
        )
        .await
        .unwrap()
        .expect("grant issued under current epoch");
    }
    assert_eq!(grant_count(&db, "revoke").await, 2);
    let (status, _, _, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/devices/revoke",
        "",
        &session_cookie,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(grant_count(&db, "revoke").await, 0);
    let ruscker_admin::db::ConfigDb::Sqlite(pool) = &db else {
        unreachable!()
    };
    let revoke_audits: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM audit_log
          WHERE actor = 'revoke' AND action = 'mfa.trusted_device.revoke'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(revoke_audits.0, 2);

    ruscker_admin::db::mfa_grants::create(
        &db,
        "revoke",
        &ruscker_admin::mfa::hash_device_token("a").unwrap(),
        "binding",
        factor,
        Utc::now(),
        Utc::now() + Duration::days(30),
        current_epoch(&db, "revoke").await,
    )
    .await
    .unwrap()
        .expect("grant issued under current epoch");
    ruscker_admin::db::users::set_password(
        &db,
        "revoke",
        "ChangedPass8!",
        false,
        Some("revoke"),
    )
    .await
    .unwrap();
    assert_eq!(grant_count(&db, "revoke").await, 0);
    let old_token = "d".repeat(64);
    let old_id = ruscker_admin::db::mfa_grants::create(
        &db,
        "revoke",
        &ruscker_admin::mfa::hash_device_token(&old_token).unwrap(),
        &ruscker_admin::mfa::session_binding(&session_id),
        factor,
        Utc::now(),
        Utc::now() + Duration::days(30),
        current_epoch(&db, "revoke").await,
    )
    .await
    .unwrap()
        .expect("grant issued under current epoch");
    ruscker_admin::db::mfa::reset(&db, "revoke", "root")
        .await
        .unwrap();
    assert_eq!(grant_count(&db, "revoke").await, 0);

    // Reset + re-enrollment rotates `confirmed_at`; the old browser proof
    // remains unusable even if its cookie is retained.
    let replacement = ruscker_admin::mfa::begin("revoke").unwrap();
    let (ciphertext, nonce) = state
        .master_key
        .encrypt(replacement.secret_base32.as_bytes())
        .unwrap();
    ruscker_admin::db::mfa::begin_enrollment(
        &db,
        "revoke",
        &ciphertext,
        &nonce,
        "replacement-ceremony",
    )
    .await
    .unwrap();
    ruscker_admin::db::mfa::confirm_enrollment(
        &db,
        "revoke",
        "revoke",
        "replacement-ceremony",
    )
    .await
    .unwrap();
    let old_cookie = jar(&format!("{DEVICE_COOKIE}={old_id}.{old_token}"));
    assert_eq!(
        ruscker_admin::mfa::evaluate(&state, "revoke", &session_id, &old_cookie, &spec(7)).await,
        MfaDecision::ChallengeRequired
    );

    let replacement_factor = ruscker_admin::db::mfa::fetch(&db, "revoke")
        .await
        .unwrap()
        .unwrap()
        .confirmed_at
        .unwrap();
    ruscker_admin::db::mfa_grants::create(
        &db,
        "revoke",
        &ruscker_admin::mfa::hash_device_token("delete-cascade").unwrap(),
        "binding",
        replacement_factor,
        Utc::now(),
        Utc::now() + Duration::days(30),
        0,
    )
    .await
    .unwrap()
        .expect("grant issued under current epoch");
    ruscker_admin::db::users::delete(&db, "revoke", Some("root"))
        .await
        .unwrap();
    assert_eq!(grant_count(&db, "revoke").await, 0);
}

#[tokio::test]
async fn decision_rejects_expired_old_window_and_changed_factor_and_identifies_unenrolled() {
    let (state, db) = state_with_db().await;
    create_user(&db, "decision").await;
    create_user(&db, "unenrolled").await;
    let (session_id, session_cookie) = session(&state, "decision").await;
    enroll(&state, &db, "decision", &session_cookie).await;
    let factor = ruscker_admin::db::mfa::fetch(&db, "decision")
        .await
        .unwrap()
        .unwrap()
        .confirmed_at
        .unwrap();
    let expired_token = "a".repeat(64);
    let expired_id = create_eval_grant(
        &db,
        "decision",
        &session_id,
        factor,
        &expired_token,
        Utc::now() - Duration::days(1),
        Utc::now() - Duration::seconds(1),
    )
    .await;
    let expired = jar(&format!("{DEVICE_COOKIE}={expired_id}.{expired_token}"));
    assert_eq!(
        ruscker_admin::mfa::evaluate(&state, "decision", &session_id, &expired, &spec(7)).await,
        MfaDecision::ChallengeRequired
    );

    let old_token = "b".repeat(64);
    let old_id = create_eval_grant(
        &db,
        "decision",
        &session_id,
        factor,
        &old_token,
        Utc::now() - Duration::days(2),
        Utc::now() + Duration::days(28),
    )
    .await;
    let old = jar(&format!("{DEVICE_COOKIE}={old_id}.{old_token}"));
    assert_eq!(
        ruscker_admin::mfa::evaluate(&state, "decision", &session_id, &old, &spec(1)).await,
        MfaDecision::ChallengeRequired
    );

    let fresh_token = "c".repeat(64);
    let fresh_id = create_eval_grant(
        &db,
        "decision",
        &session_id,
        factor,
        &fresh_token,
        Utc::now(),
        Utc::now() + Duration::days(30),
    )
    .await;
    let fresh = jar(&format!("{DEVICE_COOKIE}={fresh_id}.{fresh_token}"));
    let ruscker_admin::db::ConfigDb::Sqlite(pool) = &db else {
        unreachable!()
    };
    sqlx::query("UPDATE user_mfa SET confirmed_at = ? WHERE username = 'decision'")
        .bind(factor + Duration::seconds(1))
        .execute(pool)
        .await
        .unwrap();
    assert_eq!(
        ruscker_admin::mfa::evaluate(&state, "decision", &session_id, &fresh, &spec(7)).await,
        MfaDecision::ChallengeRequired
    );
    assert_eq!(
        ruscker_admin::mfa::evaluate(&state, "unenrolled", "session", &Cookies::default(), &spec(7)).await,
        MfaDecision::EnrollmentRequired
    );
    assert_eq!(
        ruscker_admin::mfa::evaluate(&state, "unenrolled", "session", &Cookies::default(), &unprotected_spec()).await,
        MfaDecision::Satisfied
    );
}

#[tokio::test]
async fn challenge_rate_limit_uses_atomic_sixth_attempt_rejection() {
    let (state, db) = state_with_db().await;
    let username = format!("limited-{}", uuid::Uuid::new_v4().simple());
    create_user(&db, &username).await;
    let (_, session_cookie) = session(&state, &username).await;
    enroll(&state, &db, &username, &session_cookie).await;
    for _ in 0..5 {
        assert_eq!(
            request(
                state.clone(),
                "POST",
                "/admin/account/mfa/challenge",
                "kind=totp&code=000000",
                &session_cookie,
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let (status, _, _, body) = request(
        state,
        "POST",
        "/admin/account/mfa/challenge",
        "kind=totp&code=000000",
        &session_cookie,
    )
    .await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(body.contains("data-mfa-error=\"rate-limited\""));
}


/// Codex review round 2 (#1005): a revocation that lands while a challenge
/// is in flight must win — the in-flight grant INSERT reads a stale epoch
/// and issues nothing.
#[tokio::test]
async fn revocation_racing_a_challenge_prevents_grant_issuance() {
    let (state, db) = state_with_db().await;
    create_user(&db, "race").await;
    let (session_id, session_cookie) = session(&state, "race").await;
    let _secret = enroll(&state, &db, "race", &session_cookie).await;
    let row = ruscker_admin::db::mfa::fetch(&db, "race")
        .await
        .unwrap()
        .unwrap();
    let stale_epoch = row.security_epoch;

    // The "in-flight challenge" read the epoch, THEN the revocation commits
    // (password change bumps the epoch inside its transaction).
    ruscker_admin::db::users::set_password(&db, "race", "ChangedPass8!", false, Some("race"))
        .await
        .unwrap();

    let refused = ruscker_admin::db::mfa_grants::create(
        &db,
        "race",
        &ruscker_admin::mfa::hash_device_token(&"e".repeat(64)).unwrap(),
        &ruscker_admin::mfa::session_binding(&session_id),
        row.confirmed_at.unwrap(),
        Utc::now(),
        Utc::now() + Duration::days(30),
        stale_epoch,
    )
    .await
    .unwrap();
    assert!(refused.is_none(), "stale-epoch grant must be refused");
    assert_eq!(grant_count(&db, "race").await, 0);
}


/// Belt for the pg MVCC case (codex review r3, #1005): even if a grant's
/// conditional INSERT slips past a racing revocation (READ COMMITTED can
/// read the pre-revocation epoch), the grant carries that OLD epoch and
/// evaluate rejects it against the live factor row.
#[tokio::test]
async fn grant_issued_under_old_epoch_is_rejected_at_read_time() {
    let (state, db) = state_with_db().await;
    create_user(&db, "mvcc").await;
    let (session_id, session_cookie) = session(&state, "mvcc").await;
    let _secret = enroll(&state, &db, "mvcc", &session_cookie).await;
    let row = ruscker_admin::db::mfa::fetch(&db, "mvcc").await.unwrap().unwrap();
    let token = "f".repeat(64);
    // Distinct binding from the enrollment grant (a second browser), so this
    // manual grant coexists — the test is about epoch, not binding.
    let id = ruscker_admin::db::mfa_grants::create(
        &db,
        "mvcc",
        &ruscker_admin::mfa::hash_device_token(&token).unwrap(),
        &ruscker_admin::mfa::session_binding(&format!("{session_id}:mvcc")),
        row.confirmed_at.unwrap(),
        Utc::now(),
        Utc::now() + Duration::days(30),
        row.security_epoch,
    )
    .await
    .unwrap()
    .expect("issued under current epoch");

    // Simulate the revocation whose DELETE snapshot missed this grant:
    // bump the epoch directly, leaving the grant row in place.
    let ruscker_admin::db::ConfigDb::Sqlite(pool) = &db else { unreachable!() };
    sqlx::query("UPDATE user_mfa SET security_epoch = security_epoch + 1 WHERE username = 'mvcc'")
        .execute(pool)
        .await
        .unwrap();

    let cookies = jar(&format!("{DEVICE_COOKIE}={id}.{token}"));
    assert_eq!(
        ruscker_admin::mfa::evaluate(&state, "mvcc", &session_id, &cookies, &spec(7)).await,
        MfaDecision::ChallengeRequired,
        "an old-epoch grant must never be accepted"
    );
}


/// Codex review r4 (#1005): a recovery code must never be burned when the
/// grant issuance is refused — the transactional issue() rolls the spend
/// back on a stale epoch, leaving the code usable for the next attempt.
#[tokio::test]
async fn stale_epoch_rolls_back_recovery_consumption() {
    let (state, db) = state_with_db().await;
    create_user(&db, "rollback").await;
    let (session_id, session_cookie) = session(&state, "rollback").await;
    let _secret = enroll(&state, &db, "rollback", &session_cookie).await;
    let row = ruscker_admin::db::mfa::fetch(&db, "rollback").await.unwrap().unwrap();
    // One known recovery code: replace the set with a hash we control.
    let code = "abcd2efgh3";
    let hash = ruscker_admin::mfa::hash_recovery_code(code).unwrap();
    ruscker_admin::db::mfa::replace_recovery_codes(&db, "rollback", &[hash]).await.unwrap();
    let rid = ruscker_admin::db::mfa::find_recovery_candidate(&db, "rollback", code)
        .await
        .unwrap()
        .expect("candidate");

    let refused = ruscker_admin::db::mfa_grants::issue(
        &db,
        "rollback",
        &ruscker_admin::mfa::hash_device_token(&"a".repeat(64)).unwrap(),
        &ruscker_admin::mfa::session_binding(&session_id),
        row.confirmed_at.unwrap(),
        Utc::now(),
        Utc::now() + Duration::days(30),
        row.security_epoch + 1, // stale on purpose: epoch moved
        Some(&rid),
        None,
        "mfa.recovery_used",
        "rollback",
    )
    .await
    .unwrap();
    assert_eq!(
        refused,
        Err(ruscker_admin::db::mfa_grants::IssueRefusal::StaleEpoch)
    );
    // The code survived the refusal and still matches.
    assert!(ruscker_admin::db::mfa::find_recovery_candidate(&db, "rollback", code)
        .await
        .unwrap()
        .is_some());
    // The stale issuance added no grant; enrollment's initial grant remains.
    assert_eq!(grant_count(&db, "rollback").await, 1);
}

/// Codex review r4 (#1005): a successful re-challenge must retire the
/// browser's previous grant — the overwritten cookie's old value cannot
/// remain a live bearer.
#[tokio::test]
async fn rechallenge_rotates_the_previous_grant() {
    let (state, db) = state_with_db().await;
    create_user(&db, "rotate").await;
    let (_session_id, session_cookie) = session(&state, "rotate").await;
    let secret = enroll(&state, &db, "rotate", &session_cookie).await;

    let first = next_totp(&secret, "rotate");
    let (status, set_cookies) = challenge_totp(&state, &session_cookie, &first).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let old_device = cookie_pair(&set_cookies, DEVICE_COOKIE);
    assert_eq!(grant_count(&db, "rotate").await, 1);

    // Second challenge FROM THE SAME BROWSER (carries the old device
    // cookie): the old grant must be replaced, not accumulated. A second
    // step inside the same 30s wall-clock window can't beat the replay
    // guard, so simulate time passing by lowering the recorded step.
    let ruscker_admin::db::ConfigDb::Sqlite(pool) = &db else { unreachable!() };
    sqlx::query("UPDATE user_mfa SET last_used_step = 0 WHERE username = 'rotate'")
        .execute(pool)
        .await
        .unwrap();
    let with_device = format!("{session_cookie}; {old_device}");
    let code = next_totp(&secret, "rotate");
    let (status, _, set_cookies2, _) = request(
        state.clone(),
        "POST",
        "/admin/account/mfa/challenge",
        &format!("kind=totp&code={code}"),
        &with_device,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let new_device = cookie_pair(&set_cookies2, DEVICE_COOKIE);
    assert_ne!(old_device, new_device, "cookie must rotate");
    assert_eq!(grant_count(&db, "rotate").await, 1, "old grant retired, not accumulated");
}


/// Codex review r6 (#1005): recovery codes are the break-glass-in-your-
/// wallet path — they compare salted hashes and must keep working on a
/// node that restarted WITHOUT the master key. Only TOTP (which decrypts
/// the stored secret) requires the key.
#[tokio::test]
async fn recovery_challenge_works_without_master_key() {
    let (state, db) = state_with_db().await;
    create_user(&db, "nokey").await;
    let (_sid, session_cookie) = session(&state, "nokey").await;
    let _secret = enroll(&state, &db, "nokey", &session_cookie).await;
    let code = "abcd2efgh3";
    let hash = ruscker_admin::mfa::hash_recovery_code(code).unwrap();
    ruscker_admin::db::mfa::replace_recovery_codes(&db, "nokey", &[hash]).await.unwrap();

    // The same deployment, restarted without RUSCKER_MASTER_KEY.
    let keyless = AppState {
        master_key: ruscker_admin::crypto::MasterKey::default(),
        ..state.clone()
    };

    // TOTP fails closed with the configuration hint…
    let (status, _, _, _) = request(
        keyless.clone(),
        "POST",
        "/admin/account/mfa/challenge",
        "kind=totp&code=123456",
        &session_cookie,
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);

    // …but a valid recovery code still proves possession and issues a grant.
    let (status, _, set_cookies, _) = request(
        keyless,
        "POST",
        "/admin/account/mfa/challenge",
        &format!("kind=recovery&code={code}"),
        &session_cookie,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(set_cookies.iter().any(|c| c.starts_with(DEVICE_COOKIE)));
    assert_eq!(grant_count(&db, "nokey").await, 1);
}

/// Codex review r8 (#1005): one grant per browser-session. Issuing twice for
/// the same session_binding UPSERTs the single row (no double grant), and —
/// crucially — a stale device cookie after a revocation is NOT a lockout:
/// the next challenge simply issues a fresh grant.
#[tokio::test]
async fn issuance_keeps_one_grant_per_session_and_recovers_after_revocation() {
    let (state, db) = state_with_db().await;
    create_user(&db, "rot2").await;
    let (session_id, session_cookie) = session(&state, "rot2").await;
    let _secret = enroll(&state, &db, "rot2", &session_cookie).await;
    let row = ruscker_admin::db::mfa::fetch(&db, "rot2").await.unwrap().unwrap();
    let binding = ruscker_admin::mfa::session_binding(&session_id);

    let mint = |token: &str| {
        let db = db.clone();
        let binding = binding.clone();
        let confirmed = row.confirmed_at.unwrap();
        let epoch = row.security_epoch;
        let token = token.to_string();
        async move {
            ruscker_admin::db::mfa_grants::issue(
                &db, "rot2",
                &ruscker_admin::mfa::hash_device_token(&token).unwrap(),
                &binding, confirmed, Utc::now(), Utc::now() + Duration::days(30),
                epoch, None, None, "mfa.verify", "rot2",
            )
            .await
            .unwrap()
        }
    };

    // Two issues for the SAME session_binding → UPSERT → exactly one grant.
    assert!(mint(&"a".repeat(64)).await.is_ok());
    assert!(mint(&"b".repeat(64)).await.is_ok());
    assert_eq!(grant_count(&db, "rot2").await, 1);

    // A revocation deletes the grant; the browser still holds its (now stale)
    // cookie. The next challenge must ISSUE, not refuse/lock out.
    ruscker_admin::db::mfa_grants::revoke_all(&db, "rot2", "rot2", "test").await.unwrap();
    assert_eq!(grant_count(&db, "rot2").await, 0);
    // revoke_all bumped the epoch — re-read it, as a real challenge would.
    let row = ruscker_admin::db::mfa::fetch(&db, "rot2").await.unwrap().unwrap();
    let fresh = ruscker_admin::db::mfa_grants::issue(
        &db, "rot2",
        &ruscker_admin::mfa::hash_device_token(&"c".repeat(64)).unwrap(),
        &binding, row.confirmed_at.unwrap(), Utc::now(), Utc::now() + Duration::days(30),
        row.security_epoch, None, None, "mfa.verify", "rot2",
    ).await.unwrap();
    assert!(fresh.is_ok(), "a stale cookie after revocation must not lock the user out");
    assert_eq!(grant_count(&db, "rot2").await, 1);
}

/// Codex review r9 (#1005): issuance opportunistically purges the user's
/// expired grants, so session-only MFA can't accumulate rows forever.
#[tokio::test]
async fn issuance_sweeps_the_users_expired_grants() {
    let (state, db) = state_with_db().await;
    create_user(&db, "sweep").await;
    let (_session_id, session_cookie) = session(&state, "sweep").await;
    let _secret = enroll(&state, &db, "sweep", &session_cookie).await;
    let row = ruscker_admin::db::mfa::fetch(&db, "sweep").await.unwrap().unwrap();

    // An already-expired grant from an old browser-session.
    ruscker_admin::db::mfa_grants::create(
        &db, "sweep",
        &ruscker_admin::mfa::hash_device_token(&"a".repeat(64)).unwrap(),
        &ruscker_admin::mfa::session_binding("old-login"),
        row.confirmed_at.unwrap(),
        Utc::now() - Duration::days(40),
        Utc::now() - Duration::days(10), // expired
        row.security_epoch,
    ).await.unwrap().expect("seed expired grant");
    // enroll's initial grant + this expired one.
    assert_eq!(grant_count(&db, "sweep").await, 2);

    // A fresh issuance for yet another session sweeps the expired row.
    ruscker_admin::db::mfa_grants::issue(
        &db, "sweep",
        &ruscker_admin::mfa::hash_device_token(&"b".repeat(64)).unwrap(),
        &ruscker_admin::mfa::session_binding("new-login"),
        row.confirmed_at.unwrap(), Utc::now(), Utc::now() + Duration::days(30),
        row.security_epoch, None, None, "mfa.verify", "sweep",
    ).await.unwrap().expect("issue");
    // enroll grant + new grant; the expired one is gone.
    assert_eq!(grant_count(&db, "sweep").await, 2);
    let ruscker_admin::db::ConfigDb::Sqlite(pool) = &db else { unreachable!() };
    let (expired,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM user_mfa_grants WHERE username = 'sweep' AND expires_at < ?",
    )
    .bind(Utc::now())
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(expired, 0, "expired grants must be swept");
}
