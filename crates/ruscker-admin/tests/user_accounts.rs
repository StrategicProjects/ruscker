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
        base_path: Arc::from(""),
        locales: Arc::new(locales),
        admin_auth: AdminAuth::with_token("break-glass-tok"),
        admin_sessions: Arc::new(ruscker_admin::auth::InMemoryAdminSessionStore::default()),
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
        logout_index: Arc::new(dashmap::DashMap::new()),
        leader: Arc::new(ruscker_admin::leader::AlwaysLeader),
        metrics: ruscker_admin::metrics_cache::MetricsCache::new(),
        draining: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        spec_cache: std::sync::Arc::new(dashmap::DashMap::new()),
        identity_cache: Default::default(),
        catalog_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        access_counter: std::sync::Arc::new(ruscker_admin::access_counter::AccessCounter::default()),
        alerts: ruscker_admin::alerts::AlertSink::default(),
        activity: ruscker_admin::activity::ActivitySink::default(),
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
    ruscker_admin::db::users::create(&ruscker_admin::db::ConfigDb::Sqlite(pool.clone()), "alice", "alicepass1", Role::Editor, false, &[], None)
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
    // Editor lands on the Apps list, not the dashboard (#852).
    assert_eq!(ok_loc, "/admin/specs");

    let (bad_status, _) = post(state, "/admin/login", "username=alice&password=WRONG", None).await;
    assert_eq!(bad_status, StatusCode::UNAUTHORIZED);
}

/// Every password sign-in attempt is audited with where it came from
/// (#1079): the success, the wrong password, and the unknown account —
/// whose typed name is NOT kept, because people type passwords there.
#[tokio::test]
async fn login_outcomes_are_recorded_with_where_they_came_from() {
    use ruscker_admin::activity::{ActivityEventType, AuthMethod};
    let (mut state, pool) = state_with_db().await;
    // Behind a reverse proxy: the address is the right-most forwarded hop.
    state.config = Arc::new(
        Config::from_yaml("server:\n  useForwardHeaders: true\nproxy:\n  title: T\n  specs: []\n").unwrap(),
    );
    ruscker_admin::db::users::create(
        &ruscker_admin::db::ConfigDb::Sqlite(pool.clone()),
        "alice",
        "alicepass1",
        Role::Editor,
        false,
        &[],
        None,
    )
    .await
    .unwrap();
    const UA: &str = "Mozilla/5.0 (X11; Linux x86_64; rv:152.0) Gecko/20100101 Firefox/152.0";
    let login = |state: AppState, body: &'static str, ua: String| async move {
        router(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/login")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("user-agent", ua)
                    // Left-most is whatever the client claims; ours is the right-most.
                    .header("x-forwarded-for", "198.51.100.7, 203.0.113.9")
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    };
    // The drain task isn't started in this harness, so the receiver holds
    // exactly what the capture sites enqueued (deterministic, no timing).
    let mut rx = state.activity.take_receiver().expect("activity receiver available");

    assert_eq!(login(state.clone(), "username=alice&password=alicepass1", UA.into()).await, StatusCode::SEE_OTHER);
    let ev = rx.try_recv().expect("a login.success was recorded");
    assert_eq!(ev.event_type, ActivityEventType::LoginSuccess);
    assert_eq!(ev.username.as_deref(), Some("alice"));
    assert_eq!(ev.auth_method, Some(AuthMethod::Password));
    assert!(ev.spec_id.is_none() && ev.detail.is_none());
    assert_eq!(ev.client_ip.as_deref(), Some("203.0.113.9"), "the trusted hop, not the client's claim");
    assert_eq!(ev.user_agent.as_deref(), Some(UA));

    // Wrong password for a real account: named, with the reason.
    assert_eq!(login(state.clone(), "username=alice&password=WRONG", UA.into()).await, StatusCode::UNAUTHORIZED);
    let ev = rx.try_recv().expect("a login.failed was recorded");
    assert_eq!(ev.event_type, ActivityEventType::LoginFailed);
    assert_eq!(ev.username.as_deref(), Some("alice"));
    assert_eq!(ev.detail.as_deref(), Some("wrong-password"));
    assert_eq!(ev.client_ip.as_deref(), Some("203.0.113.9"));

    // Unknown account: the typed text is kept NOWHERE in the event.
    assert_eq!(
        login(state.clone(), "username=MyS3cretPassw0rd&password=x", UA.into()).await,
        StatusCode::UNAUTHORIZED
    );
    let ev = rx.try_recv().expect("a login.failed was recorded");
    assert_eq!(ev.username, None);
    assert_eq!(ev.detail.as_deref(), Some("unknown-account"));
    assert!(!format!("{ev:?}").contains("MyS3cretPassw0rd"));

    // A hostile User-Agent is cut down to size.
    let long = "x".repeat(5000);
    assert_eq!(login(state.clone(), "username=alice&password=WRONG", long).await, StatusCode::UNAUTHORIZED);
    let ev = rx.try_recv().unwrap();
    assert_eq!(ev.user_agent.as_deref().map(str::len), Some(256));
    assert!(rx.try_recv().is_err(), "one event per attempt");

    // A proxy that adds its hop as a SEPARATE header line: the last line
    // is ours; the first is whatever the client sent. A tab in the
    // User-Agent (legal in a header value) is dropped.
    let resp = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("user-agent", "Odd\tAgent/1.0")
                .header("x-forwarded-for", "10.99.99.99")
                .header("x-forwarded-for", "203.0.113.50")
                .body(Body::from("username=nobody&password=x"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let ev = rx.try_recv().unwrap();
    assert_eq!(ev.client_ip.as_deref(), Some("203.0.113.50"));
    assert_eq!(ev.user_agent.as_deref(), Some("OddAgent/1.0"));

    // Without the forwarded-headers opt-in the header is not believed
    // (and there is no TCP peer under `oneshot`): no address is recorded.
    let (direct, _pool) = state_with_db().await;
    let mut rx = direct.activity.take_receiver().unwrap();
    assert_eq!(login(direct, "username=nobody&password=x", UA.into()).await, StatusCode::UNAUTHORIZED);
    assert_eq!(rx.try_recv().unwrap().client_ip, None);
}

#[tokio::test]
async fn password_login_honors_only_a_local_app_next() {
    let (state, pool) = state_with_db().await;
    ruscker_admin::db::users::create(
        &ruscker_admin::db::ConfigDb::Sqlite(pool),
        "alice",
        "alicepass1",
        Role::Viewer,
        false,
        &[],
        None,
    )
    .await
    .unwrap();

    let (safe_status, safe_location) = post(
        state.clone(),
        "/admin/login",
        "username=alice&password=alicepass1&next=%2Fapp%2Fanalysts-app%2F%3Ftab%3D1",
        None,
    )
    .await;
    assert_eq!(safe_status, StatusCode::SEE_OTHER);
    assert_eq!(safe_location, "/app/analysts-app/?tab=1");

    let (external_status, external_location) = post(
        state,
        "/admin/login",
        "username=alice&password=alicepass1&next=https%3A%2F%2Fevil.example%2Fpwn",
        None,
    )
    .await;
    assert_eq!(external_status, StatusCode::SEE_OTHER);
    assert_eq!(
        external_location, "/",
        "external next falls back to role home"
    );
}

#[tokio::test]
async fn app_next_is_reprefixed_under_a_base_path() {
    let (mut state, pool) = state_with_db().await;
    state.base_path = Arc::from("/box");
    ruscker_admin::db::users::create(
        &ruscker_admin::db::ConfigDb::Sqlite(pool),
        "alice",
        "alicepass1",
        Role::Viewer,
        false,
        &[],
        None,
    )
    .await
    .unwrap();

    let (status, location) = post(
        state,
        "/box/admin/login",
        "username=alice&password=alicepass1&next=%2Fapp%2Fanalysts-app%2F",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, "/box/app/analysts-app/");
}

#[tokio::test]
async fn token_login_honors_a_local_app_next_after_bootstrap() {
    let (state, pool) = state_with_db().await;
    ruscker_admin::db::users::create(
        &ruscker_admin::db::ConfigDb::Sqlite(pool),
        "admin",
        "adminpass1",
        Role::Admin,
        false,
        &[],
        None,
    )
    .await
    .unwrap();

    let (status, location) = post(
        state,
        "/admin/login/token",
        "token=break-glass-tok&next=%2Fapp%2Fanalysts-app%2F",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(location, "/app/analysts-app/");
}

#[tokio::test]
async fn first_login_with_must_change_redirects_to_password() {
    let (state, pool) = state_with_db().await;
    // must_change = true ⇒ first login lands on the change-password page.
    ruscker_admin::db::users::create(&ruscker_admin::db::ConfigDb::Sqlite(pool.clone()), "bob", "bobpass12", Role::Viewer, true, &[], None)
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
async fn app_next_survives_mandatory_password_rotation() {
    let (state, pool) = state_with_db().await;
    ruscker_admin::db::users::create(
        &ruscker_admin::db::ConfigDb::Sqlite(pool),
        "bob",
        "bobpass12",
        Role::Viewer,
        true,
        &[],
        None,
    )
    .await
    .unwrap();
    let app = router(state);

    let login = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from(
                    "username=bob&password=bobpass12&next=%2Fapp%2Fanalysts-app%2F%3Ftab%3D1",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        login
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some("/admin/account/password?next=%2Fapp%2Fanalysts-app%2F%3Ftab%3D1")
    );
    let cookie = login
        .headers()
        .get("set-cookie")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(';').next())
        .expect("login issues a session cookie")
        .to_string();

    let password_page = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/admin/account/password?next=%2Fapp%2Fanalysts-app%2F%3Ftab%3D1")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(password_page.status(), StatusCode::OK);
    let body = axum::body::to_bytes(password_page.into_body(), 1 << 20)
        .await
        .unwrap();
    assert!(
        std::str::from_utf8(&body)
            .unwrap()
            .contains(r#"name="next" value="/app/analysts-app/?tab=1""#),
        "password form must preserve the validated destination"
    );

    let changed = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/account/password")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("cookie", cookie)
                .body(Body::from(
                    "current=bobpass12&new_password=Bob!pass-new9&confirm=Bob!pass-new9&next=%2Fapp%2Fanalysts-app%2F%3Ftab%3D1",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(changed.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        changed
            .headers()
            .get("location")
            .and_then(|value| value.to_str().ok()),
        Some("/app/analysts-app/?tab=1")
    );
}

/// Count `<form ` opens, then verify none of them is nested inside
/// another `<form>`. Cheap structural check that doesn't need an HTML
/// parser. Returns `(open_count, max_depth)`.
/// An operator who signed in with the break-glass token (to recover a
/// forgotten password, say) and clicks "Change password" used to be
/// bounced to the dashboard with no explanation. The token has no
/// account: the page now says so and points at Users, and shows no form.
#[tokio::test]
async fn password_page_explains_itself_to_a_token_session() {
    let (state, _pool) = state_with_db().await;
    let sid = state.admin_sessions.create(Role::Admin, None).await;
    let cookie = format!("{COOKIE_NAME}={sid}");
    let resp = router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/admin/account/password")
                .header(axum::http::header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "no silent redirect");
    let body = String::from_utf8(axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
    assert!(body.contains(r#"href="/admin/users""#), "points at Users");
    assert!(!body.contains(r#"name="new_password""#), "no form that could not work");

    // A real account still gets the form.
    ruscker_admin::db::users::create(
        state.db.as_ref().unwrap(),
        "ana",
        "CorrectPass9!",
        Role::Viewer,
        false,
        &[],
        None,
    )
    .await
    .unwrap();
    let sid = state.admin_sessions.create(Role::Viewer, Some("ana".into())).await;
    let resp = router(state)
        .oneshot(
            Request::builder()
                .uri("/admin/account/password")
                .header(axum::http::header::COOKIE, format!("{COOKIE_NAME}={sid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body = String::from_utf8(axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
    assert!(body.contains(r#"name="new_password""#));
}

/// One request to the password or token form: status, `Retry-After`, body.
async fn sign_in(state: AppState, uri: &'static str, body: &'static str, from: Option<&'static str>) -> (StatusCode, Option<String>, String) {
    let mut req = Request::builder()
        .method("POST")
        .uri(uri)
        .header("content-type", "application/x-www-form-urlencoded");
    if let Some(from) = from {
        req = req.header("x-forwarded-for", from);
    }
    let resp = router(state).oneshot(req.body(Body::from(body)).unwrap()).await.unwrap();
    let status = resp.status();
    let retry = resp.headers().get("retry-after").and_then(|v| v.to_str().ok()).map(str::to_string);
    let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
    (status, retry, String::from_utf8_lossy(&body).into_owned())
}

/// A state behind a trusted proxy, with two accounts and a throttle whose
/// first wait is a full minute — so no assertion depends on how fast a
/// password check runs on the test machine.
async fn throttled_state() -> AppState {
    use ruscker_admin::auth::{LoginPolicy, LoginRateLimiter};
    let (mut state, pool) = state_with_db().await;
    state.config = Arc::new(
        Config::from_yaml("server:\n  useForwardHeaders: true\nproxy:\n  title: T\n  specs: []\n").unwrap(),
    );
    state.login_limiter = Arc::new(LoginRateLimiter::with_policy(LoginPolicy {
        backoff_base: std::time::Duration::from_secs(60),
        ..LoginPolicy::default()
    }));
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool);
    for who in ["alice", "bob"] {
        ruscker_admin::db::users::create(&db, who, "Correct#Pass9", Role::Editor, false, &[], None)
            .await
            .unwrap();
    }
    state
}

const WRONG: &str = "username=alice&password=WRONG";
const RIGHT: &str = "username=alice&password=Correct%23Pass9";
const BOB: &str = "username=bob&password=Correct%23Pass9";
const MALLORY: Option<&str> = Some("203.0.113.66");

/// The sign-in throttle works per account and per address: repeated wrong
/// passwords for one account get `429` + `Retry-After` and the sign-in
/// page saying so — while another person signs in normally, which the
/// old single global window did not allow.
#[tokio::test]
async fn repeated_wrong_passwords_slow_one_account_not_everyone() {
    let state = throttled_state().await;
    for _ in 0..3 {
        let (status, _, _) = sign_in(state.clone(), "/admin/login", WRONG, MALLORY).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
    }
    // The next attempt is told to wait, even with the RIGHT password:
    // the throttle answers before the password is looked at.
    let (status, retry, body) = sign_in(state.clone(), "/admin/login", RIGHT, MALLORY).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    let secs: u64 = retry.expect("Retry-After").parse().unwrap();
    assert!((50..=60).contains(&secs), "{secs}");
    assert!(body.contains("admin-login-card") && body.contains("Muitas tentativas"), "the sign-in page, saying why");

    // Nobody else is affected: bob from the same address, and alice
    // herself from her own.
    assert_eq!(sign_in(state.clone(), "/admin/login", BOB, MALLORY).await.0, StatusCode::SEE_OTHER);
    assert_eq!(sign_in(state.clone(), "/admin/login", RIGHT, Some("198.51.100.20")).await.0, StatusCode::SEE_OTHER);
    // Her success does not lift the guesser's own wait.
    assert_eq!(sign_in(state.clone(), "/admin/login", WRONG, MALLORY).await.0, StatusCode::TOO_MANY_REQUESTS);
}

/// Attempts are counted when admitted, not when they fail: a burst that
/// arrives before any password has been checked is bounded all the same.
#[tokio::test]
async fn a_parallel_burst_cannot_outrun_the_throttle() {
    let state = throttled_state().await;
    let mut tasks = Vec::new();
    for _ in 0..40 {
        let state = state.clone();
        tasks.push(tokio::spawn(async move { sign_in(state, "/admin/login", WRONG, MALLORY).await.0 }));
    }
    let mut checked = 0;
    for t in tasks {
        match t.await.unwrap() {
            StatusCode::UNAUTHORIZED => checked += 1,
            StatusCode::TOO_MANY_REQUESTS => {}
            other => panic!("unexpected {other}"),
        }
    }
    assert_eq!(checked, 3, "only three of forty simultaneous guesses reached the password check");
    // And the burst did not close sign-in for anyone else.
    assert_eq!(sign_in(state.clone(), "/admin/login", BOB, Some("198.51.100.20")).await.0, StatusCode::SEE_OTHER);
}

/// Guesses against one account spread over many addresses stop at the
/// account's own ceiling — except for the person signing in from where
/// they have signed in before.
#[tokio::test]
async fn the_account_ceiling_holds_but_not_against_a_known_address() {
    let state = throttled_state().await;
    const HOME: Option<&str> = Some("198.51.100.20");
    assert_eq!(sign_in(state.clone(), "/admin/login", RIGHT, HOME).await.0, StatusCode::SEE_OTHER);
    // 30 wrong passwords for alice, one per address.
    const ADDRS: [&str; 30] = [
        "192.0.2.1", "192.0.2.2", "192.0.2.3", "192.0.2.4", "192.0.2.5", "192.0.2.6", "192.0.2.7", "192.0.2.8",
        "192.0.2.9", "192.0.2.10", "192.0.2.11", "192.0.2.12", "192.0.2.13", "192.0.2.14", "192.0.2.15",
        "192.0.2.16", "192.0.2.17", "192.0.2.18", "192.0.2.19", "192.0.2.20", "192.0.2.21", "192.0.2.22",
        "192.0.2.23", "192.0.2.24", "192.0.2.25", "192.0.2.26", "192.0.2.27", "192.0.2.28", "192.0.2.29",
        "192.0.2.30",
    ];
    for from in ADDRS {
        assert_eq!(sign_in(state.clone(), "/admin/login", WRONG, Some(from)).await.0, StatusCode::UNAUTHORIZED);
    }
    // A 31st address is refused without a password check…
    let (status, retry, _) = sign_in(state.clone(), "/admin/login", RIGHT, Some("192.0.2.99")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(retry.unwrap().parse::<u64>().unwrap() > 500, "the ten-minute account window");
    // …but alice, from the address she signed in from, gets in.
    assert_eq!(sign_in(state.clone(), "/admin/login", RIGHT, HOME).await.0, StatusCode::SEE_OTHER);
}

/// The break-glass token is one secret that opens everything: its form
/// keeps one low ceiling for all addresses together, and is separate
/// from the password form in both directions.
#[tokio::test]
async fn the_token_form_has_its_own_ceiling() {
    let state = throttled_state().await;
    const ADDRS: [&str; 10] = [
        "192.0.2.1", "192.0.2.2", "192.0.2.3", "192.0.2.4", "192.0.2.5", "192.0.2.6", "192.0.2.7", "192.0.2.8",
        "192.0.2.9", "192.0.2.10",
    ];
    for from in ADDRS {
        assert_eq!(sign_in(state.clone(), "/admin/login/token", "token=nope", Some(from)).await.0, StatusCode::UNAUTHORIZED);
    }
    // Ten wrong tokens in a minute close the form to an eleventh address:
    // spreading guesses over addresses buys nothing.
    let (status, retry, body) = sign_in(state.clone(), "/admin/login/token", "token=nope", Some("192.0.2.200")).await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert!(retry.is_some() && body.contains("Muitas tentativas"));
    // Accounts still sign in.
    assert_eq!(sign_in(state.clone(), "/admin/login", BOB, Some("192.0.2.200")).await.0, StatusCode::SEE_OTHER);
}

/// Without trusted forwarded headers every request has the same address
/// (the proxy's, or none in this harness): the address layer is then one
/// ceiling shared by everyone, as documented — not a way around it.
#[tokio::test]
async fn without_trusted_forwarded_headers_the_address_is_shared() {
    use ruscker_admin::auth::{LoginPolicy, LoginRateLimiter};
    let (mut state, pool) = state_with_db().await;
    state.login_limiter = Arc::new(LoginRateLimiter::with_policy(LoginPolicy { address_max: 4, ..LoginPolicy::default() }));
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool);
    ruscker_admin::db::users::create(&db, "bob", "Correct#Pass9", Role::Editor, false, &[], None).await.unwrap();
    // Four wrong guesses on four accounts, each claiming another address.
    for (body, claimed) in [
        ("username=u1&password=x", "192.0.2.1"),
        ("username=u2&password=x", "192.0.2.2"),
        ("username=u3&password=x", "192.0.2.3"),
        ("username=u4&password=x", "192.0.2.4"),
    ] {
        assert_eq!(sign_in(state.clone(), "/admin/login", body, Some(claimed)).await.0, StatusCode::UNAUTHORIZED);
    }
    // The claimed addresses were ignored: the shared ceiling is reached.
    assert_eq!(sign_in(state.clone(), "/admin/login", BOB, Some("192.0.2.9")).await.0, StatusCode::TOO_MANY_REQUESTS);
}

/// The people the throttle knows survive a restart: they are rebuilt from
/// the audit log, which has every correct sign-in with its address.
#[tokio::test]
async fn known_sign_ins_survive_a_restart_through_the_audit_log() {
    use ruscker_admin::activity::load_known_sign_ins;
    use ruscker_admin::auth::{LoginPolicy, LoginRateLimiter};
    let state = throttled_state().await;
    let db = state.db.clone().unwrap();
    const HOME: Option<&str> = Some("198.51.100.20");

    // alice signs in from home; the event reaches the audit table (the
    // drain task is not running in this harness, so write the batch).
    let mut rx = state.activity.take_receiver().unwrap();
    assert_eq!(sign_in(state.clone(), "/admin/login", RIGHT, HOME).await.0, StatusCode::SEE_OTHER);
    let mut batch = Vec::new();
    while let Ok(event) = rx.try_recv() {
        batch.push(event);
    }
    ruscker_admin::db::user_activity::insert_batch(&db, &batch).await.unwrap();

    // "Restart": a brand-new throttle that has seen nothing.
    let mut restarted = state.clone();
    restarted.login_limiter = Arc::new(LoginRateLimiter::with_policy(LoginPolicy {
        backoff_base: std::time::Duration::from_secs(60),
        account_max: 5,
        ..LoginPolicy::default()
    }));
    // Someone fills alice's account ceiling from five addresses.
    for from in ["192.0.2.1", "192.0.2.2", "192.0.2.3", "192.0.2.4", "192.0.2.5"] {
        assert_eq!(sign_in(restarted.clone(), "/admin/login", WRONG, Some(from)).await.0, StatusCode::UNAUTHORIZED);
    }
    // Without the log, alice is a stranger to the new throttle…
    assert_eq!(sign_in(restarted.clone(), "/admin/login", RIGHT, HOME).await.0, StatusCode::TOO_MANY_REQUESTS);
    // …with it, she is known at home again — and only there.
    let since = chrono::Utc::now() - chrono::Duration::days(30);
    let read = load_known_sign_ins(&db, &restarted.login_limiter, since).await.unwrap();
    assert_eq!(read, 1);
    assert_eq!(sign_in(restarted.clone(), "/admin/login", RIGHT, HOME).await.0, StatusCode::SEE_OTHER);
    assert_eq!(sign_in(restarted.clone(), "/admin/login", RIGHT, Some("192.0.2.99")).await.0, StatusCode::TOO_MANY_REQUESTS);

    // Her password is reset (someone may have had it). After the next
    // pass over the database she is no longer known at home — nor is
    // whoever else signed in with the old password, wherever from — and
    // the old sign-in is not read back.
    tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    ruscker_admin::db::users::set_password(&db, "alice", "Changed#Pass8", false, Some("root")).await.unwrap();
    load_known_sign_ins(&db, &restarted.login_limiter, since).await.unwrap();
    assert_eq!(
        sign_in(restarted.clone(), "/admin/login", "username=alice&password=Changed%23Pass8", HOME).await.0,
        StatusCode::TOO_MANY_REQUESTS,
        "a stranger again until she signs in with the new password"
    );
}

/// With a stable key the throttle keeps its own memory — keyed hashes,
/// no addresses — so it does not depend on the audit log: it survives a
/// restart even when the audit rows are gone (a short retention), and it
/// covers the break-glass token, which is never in the audit log.
#[tokio::test]
async fn the_throttles_own_memory_survives_without_the_audit_log() {
    use ruscker_admin::activity::load_known_sign_ins;
    use ruscker_admin::auth::{LoginPolicy, LoginRateLimiter};
    let state = throttled_state().await;
    let db = state.db.clone().unwrap();
    const KEY: [u8; 32] = [42; 32];
    const HOME: Option<&str> = Some("198.51.100.20");
    const OPS: Option<&str> = Some("198.51.100.30");
    assert!(state.login_limiter.set_memory_key(KEY));

    // alice signs in with her password, the operator with the token.
    assert_eq!(sign_in(state.clone(), "/admin/login", RIGHT, HOME).await.0, StatusCode::SEE_OTHER);
    assert_eq!(sign_in(state.clone(), "/admin/login/token", "token=break-glass-tok", OPS).await.0, StatusCode::SEE_OTHER);

    // What was stored: two rows, hashes only — no address anywhere.
    let ruscker_admin::db::ConfigDb::Sqlite(pool) = &db else { unreachable!() };
    let rows: Vec<(String, String, String)> =
        sqlx::query_as("SELECT kind, username, pair_hash FROM signin_memory ORDER BY kind")
            .fetch_all(pool)
            .await
            .unwrap();
    assert_eq!(rows.iter().map(|r| (r.0.as_str(), r.1.as_str())).collect::<Vec<_>>(), [("password", "alice"), ("token", "")]);
    assert!(rows.iter().all(|r| r.2.len() == 64 && !r.2.contains("198")));

    // "Restart" with the audit log empty (as with a short retention).
    sqlx::query("DELETE FROM user_activity").execute(pool).await.unwrap();
    let restart = |key: Option<[u8; 32]>| {
        let mut s = state.clone();
        let limiter = LoginRateLimiter::with_policy(LoginPolicy {
            backoff_base: std::time::Duration::from_secs(60),
            account_max: 5,
            token_max: 3,
            ..LoginPolicy::default()
        });
        if let Some(key) = key {
            limiter.set_memory_key(key);
        }
        s.login_limiter = Arc::new(limiter);
        s
    };
    let since = chrono::Utc::now() - chrono::Duration::days(30);
    let attack = |s: AppState| async move {
        for from in ["192.0.2.1", "192.0.2.2", "192.0.2.3", "192.0.2.4", "192.0.2.5"] {
            assert_eq!(sign_in(s.clone(), "/admin/login", WRONG, Some(from)).await.0, StatusCode::UNAUTHORIZED);
        }
        for from in ["192.0.2.11", "192.0.2.12", "192.0.2.13"] {
            assert_eq!(sign_in(s.clone(), "/admin/login/token", "token=nope", Some(from)).await.0, StatusCode::UNAUTHORIZED);
        }
    };

    // Same key: both are known again, from the hashes alone.
    let same = restart(Some(KEY));
    assert_eq!(load_known_sign_ins(&db, &same.login_limiter, since).await.unwrap(), 2);
    attack(same.clone()).await;
    assert_eq!(sign_in(same.clone(), "/admin/login", RIGHT, HOME).await.0, StatusCode::SEE_OTHER);
    assert_eq!(sign_in(same.clone(), "/admin/login/token", "token=break-glass-tok", OPS).await.0, StatusCode::SEE_OTHER);
    // …and only there.
    assert_eq!(sign_in(same.clone(), "/admin/login", RIGHT, Some("192.0.2.99")).await.0, StatusCode::TOO_MANY_REQUESTS);

    // Without the key the rows mean nothing (and are not even read).
    let keyless = restart(None);
    assert_eq!(load_known_sign_ins(&db, &keyless.login_limiter, since).await.unwrap(), 0);
    attack(keyless.clone()).await;
    assert_eq!(sign_in(keyless.clone(), "/admin/login", RIGHT, HOME).await.0, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(sign_in(keyless.clone(), "/admin/login/token", "token=break-glass-tok", OPS).await.0, StatusCode::TOO_MANY_REQUESTS);

    // The race: a password reset lands while a sign-in with the OLD
    // password is still in flight. Simulated by resetting just before —
    // the sign-in sees a change within its margin and forgets the pair
    // instead of remembering it; and the row it wrote is dated at the
    // check, so the read leaves it out as well.
    let raced = restart(Some(KEY));
    ruscker_admin::db::users::set_password(&db, "bob", "Correct#Pass9", false, Some("root")).await.unwrap();
    assert_eq!(sign_in(raced.clone(), "/admin/login", BOB, HOME).await.0, StatusCode::SEE_OTHER);
    for from in ["192.0.2.21", "192.0.2.22", "192.0.2.23", "192.0.2.24", "192.0.2.25"] {
        assert_eq!(sign_in(raced.clone(), "/admin/login", "username=bob&password=WRONG", Some(from)).await.0, StatusCode::UNAUTHORIZED);
    }
    assert_eq!(sign_in(raced.clone(), "/admin/login", BOB, HOME).await.0, StatusCode::TOO_MANY_REQUESTS, "not remembered");
    let bob_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM signin_memory WHERE username = 'bob'")
        .fetch_one(pool)
        .await
        .unwrap();
    assert_eq!(bob_rows, 0);

    // alice changes her own password through the page: her rows go,
    // in the table and in memory, and only hers.
    let sid = state.admin_sessions.create(Role::Editor, Some("alice".into())).await;
    let cookie = format!("{COOKIE_NAME}={sid}");
    let (status, _) = post(
        state.clone(),
        "/admin/account/password",
        "current=Correct%23Pass9&new_password=Changed%23Pass8&confirm=Changed%23Pass8",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let left: Vec<(String, String)> = sqlx::query_as("SELECT kind, username FROM signin_memory ORDER BY kind")
        .fetch_all(pool)
        .await
        .unwrap();
    assert_eq!(left, [("token".to_string(), String::new())], "only the token row is left");
    let after = restart(Some(KEY));
    assert_eq!(load_known_sign_ins(&db, &after.login_limiter, since).await.unwrap(), 1);
}

/// Refused sign-ins are reachable without an account, so their audit
/// rows are budgeted: a burst from one address leaves a handful of rows,
/// not one per attempt — and cannot push real events out of the queue.
#[tokio::test]
async fn a_burst_of_refused_logins_writes_a_bounded_number_of_rows() {
    let (mut state, _pool) = state_with_db().await;
    state.config = Arc::new(
        Config::from_yaml("server:\n  useForwardHeaders: true\nproxy:\n  title: T\n  specs: []\n").unwrap(),
    );
    let mut rx = state.activity.take_receiver().unwrap();
    let mut tasks = Vec::new();
    for _ in 0..60 {
        let state = state.clone();
        tasks.push(tokio::spawn(async move {
            router(state)
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/admin/login")
                        .header("content-type", "application/x-www-form-urlencoded")
                        .header("x-forwarded-for", "203.0.113.9")
                        .body(Body::from("username=nobody&password=x"))
                        .unwrap(),
                )
                .await
                .unwrap()
                .status()
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let mut rows = 0;
    while rx.try_recv().is_ok() {
        rows += 1;
    }
    assert!((1..=10).contains(&rows), "{rows} rows for 60 attempts from one address");
    assert!(state.activity.suppressed() > 0 || rows < 10);
}

fn form_nesting(body: &str) -> (usize, usize) {
    let mut depth: usize = 0;
    let mut max: usize = 0;
    let mut opens: usize = 0;
    let bytes = body.as_bytes();
    let mut i = 0;
    while i + 6 <= bytes.len() {
        if &bytes[i..i + 6] == b"<form " {
            opens += 1;
            depth += 1;
            if depth > max {
                max = depth;
            }
            i += 6;
        } else if i + 7 <= bytes.len() && &bytes[i..i + 7] == b"</form>" {
            depth = depth.saturating_sub(1);
            i += 7;
        } else {
            i += 1;
        }
    }
    (opens, max)
}

#[tokio::test]
async fn login_page_chrome_cluster_is_outside_the_login_form() {
    // #182 + #183: theme + language pickers moved into a top-right
    // chrome cluster that lives in `<body>` BEFORE the login form.
    // Each picker is its own POST form sibling to the login form.
    //
    // Originally (#181) the cluster was inside the card and used
    // `formaction` to escape the outer login form. With the cluster
    // hoisted into the body, regular `<form>` wrappers are fine and
    // the formaction trick is no longer needed.
    let (state, _pool) = state_with_db().await;
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/admin/login")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = std::str::from_utf8(&bytes).unwrap();

    // 3 sibling forms expected: theme picker + locale picker + the
    // outer login form. None nested.
    let (opens, max_depth) = form_nesting(body);
    assert_eq!(opens, 3, "expected 3 sibling <form> tags, got {opens}");
    assert_eq!(max_depth, 1, "no <form> should be nested inside another");

    // The chrome cluster's POSTs must target the same endpoints the
    // old footer used.
    assert!(
        body.contains(r#"action="/__set/theme""#),
        "chrome theme form should POST to /__set/theme"
    );
    assert!(
        body.contains(r#"action="/__set/locale""#),
        "chrome locale form should POST to /__set/locale"
    );
    // The cluster itself.
    assert!(
        body.contains(r#"class="chrome-cluster""#),
        "chrome cluster scaffolding missing"
    );
    // Active locale + active theme carry aria-current="true" — the
    // current-selection ARIA pattern (we dropped the menuitemradio
    // role + aria-checked combo per the #195 review).
    assert!(
        body.contains(r#"aria-current="true""#),
        "active locale/theme should carry aria-current"
    );
}

#[tokio::test]
async fn setup_page_chrome_cluster_is_outside_the_setup_form() {
    // Twin of `login_page_chrome_cluster_is_outside_the_login_form` for
    // /admin/setup.
    let (state, _pool) = state_with_db().await;
    let sid = state.admin_sessions.create(Role::Admin, None).await;
    let cookie = format!("{COOKIE_NAME}={sid}");
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/admin/setup")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = std::str::from_utf8(&bytes).unwrap();

    let (opens, max_depth) = form_nesting(body);
    assert_eq!(opens, 3, "expected 3 sibling <form> tags, got {opens}");
    assert_eq!(max_depth, 1, "no <form> should be nested inside another");
    assert!(
        body.contains(r#"action="/__set/theme""#),
        "chrome theme form should POST to /__set/theme"
    );
    assert!(
        body.contains(r#"action="/__set/locale""#),
        "chrome locale form should POST to /__set/locale"
    );
    assert!(
        body.contains(r#"class="chrome-cluster""#),
        "chrome cluster scaffolding missing"
    );
}

async fn get(state: AppState, uri: &str, cookie: Option<&str>) -> (StatusCode, String) {
    let app = router(state);
    let mut b = Request::builder().method("GET").uri(uri);
    if let Some(c) = cookie {
        b = b.header("cookie", c);
    }
    let resp = app.oneshot(b.body(Body::empty()).unwrap()).await.unwrap();
    let status = resp.status();
    let loc = resp
        .headers()
        .get("location")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    (status, loc)
}

/// #454: a logged-in account that still carries `must_change_password`
/// is bounced off every other admin route back to the password page —
/// so the post-login prompt can't be skipped by navigating away.
#[tokio::test]
async fn must_change_user_is_pinned_to_password_page() {
    let (state, pool) = state_with_db().await;
    ruscker_admin::db::users::create(
        &ruscker_admin::db::ConfigDb::Sqlite(pool.clone()),
        "bob",
        "bobpass12",
        Role::Viewer,
        true,
        &[],
        None,
    )
    .await
    .unwrap();
    let sid = state
        .admin_sessions
        .create(Role::Viewer, Some("bob".into()))
        .await;
    let cookie = format!("{COOKIE_NAME}={sid}");

    // Any other admin route redirects to the change-password page.
    let (status, loc) = get(state.clone(), "/admin/dashboard", Some(&cookie)).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/account/password");

    // …but the password page itself stays reachable (no redirect loop).
    let (pw_status, _) = get(state, "/admin/account/password", Some(&cookie)).await;
    assert_eq!(pw_status, StatusCode::OK);
}

/// #454: completing the change clears the flag, and the guard then lets
/// the user through. There is no "keep current" escape hatch any more —
/// a real password change is the only way past.
#[tokio::test]
async fn changing_password_lifts_the_guard() {
    let (state, pool) = state_with_db().await;
    // Editor (not Viewer): this test is about the must-change guard
    // lifting, which is role-agnostic. An Editor's home is the dashboard,
    // so the post-change landing assertions below stay valid — a Viewer's
    // home is now the portal (#857), covered separately in rbac.rs.
    ruscker_admin::db::users::create(
        &ruscker_admin::db::ConfigDb::Sqlite(pool.clone()),
        "bob",
        "bobpass12",
        Role::Editor,
        true,
        &[],
        None,
    )
    .await
    .unwrap();
    let sid = state
        .admin_sessions
        .create(Role::Editor, Some("bob".into()))
        .await;
    let cookie = format!("{COOKIE_NAME}={sid}");

    // Do the POST by hand (not via the `post` helper) because we need
    // the response's Set-Cookie: since #739 a password change revokes
    // every session of the account — including the one making the
    // request — and re-issues a fresh cookie on the response.
    let app = router(state.clone());
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/account/password")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("cookie", &cookie)
                .body(Body::from(
                    "current=bobpass12&new_password=Bob!pass-new9&confirm=Bob!pass-new9",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    // Editor's post-change landing is the Apps list now (#852).
    assert_eq!(
        resp.headers().get("location").and_then(|v| v.to_str().ok()),
        Some("/admin/specs")
    );
    let fresh_cookie = resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|c| c.split(';').next())
        .expect("the change re-issues a session cookie (#739)")
        .to_string();

    // The old session died with the change (#739)…
    let (old_status, old_loc) = get(state.clone(), "/admin/dashboard", Some(&cookie)).await;
    assert_eq!(old_status, StatusCode::SEE_OTHER);
    assert_eq!(old_loc, "/admin/login");

    // …and on the fresh session the guard no longer fires — the
    // dashboard renders.
    let (dash_status, _) = get(state, "/admin/dashboard", Some(&fresh_cookie)).await;
    assert_eq!(dash_status, StatusCode::OK);
}

/// A normal account (no pending change) reaches the panel untouched —
/// the guard must not redirect everyone.
#[tokio::test]
async fn settled_user_reaches_the_dashboard() {
    let (state, pool) = state_with_db().await;
    ruscker_admin::db::users::create(
        &ruscker_admin::db::ConfigDb::Sqlite(pool.clone()),
        "alice",
        "alicepass1",
        Role::Editor,
        false,
        &[],
        None,
    )
    .await
    .unwrap();
    let sid = state
        .admin_sessions
        .create(Role::Editor, Some("alice".into()))
        .await;
    let cookie = format!("{COOKIE_NAME}={sid}");
    let (status, _) = get(state, "/admin/dashboard", Some(&cookie)).await;
    assert_eq!(status, StatusCode::OK);
}

/// #452: with a log buffer wired but still empty, the Logs tab shows an
/// explicit "nothing captured yet" message — distinct from the "buffer
/// not wired" message — so an operator can tell an idle log from a broken
/// tab. Renders in the default locale (pt).
#[tokio::test]
async fn logs_tab_distinguishes_empty_buffer_from_unwired() {
    let (mut state, _pool) = state_with_db().await;
    // Wired but empty.
    state.log_buffer = Some(ruscker_admin::logbuf::LogBuffer::new(16));
    let sid = state.admin_sessions.create(Role::Admin, None).await;
    let cookie = format!("{COOKIE_NAME}={sid}");
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/admin/logs")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = std::str::from_utf8(&bytes).unwrap();
    assert!(
        body.contains("Nenhum log capturado"),
        "empty-buffer hint missing"
    );
    assert!(
        !body.contains("Buffer de log não disponível"),
        "should not show the unwired-buffer message when a buffer exists"
    );
    assert!(body.contains("/admin/logs/poll"), "poll endpoint missing");
    assert!(
        !body.contains("new EventSource"),
        "the process-log page must not open a persistent connection"
    );
}

/// #1039: process-log following uses finite cursor polling. This avoids an
/// infinite HTTP/1.1 response being retained by a reverse proxy and blocking
/// subsequent admin navigation on the same backend connection.
#[tokio::test]
async fn process_log_poll_is_incremental_and_legacy_sse_is_retired() {
    let (mut state, _pool) = state_with_db().await;
    let buffer = ruscker_admin::logbuf::LogBuffer::new(16);
    buffer.push_line("first");
    let cursor = buffer.cursor();
    buffer.push_line("second");
    state.log_buffer = Some(buffer);

    let sid = state.admin_sessions.create(Role::Admin, None).await;
    let cookie = format!("{COOKIE_NAME}={sid}");
    let app = router(state);
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/admin/logs/poll?cursor={cursor}"))
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response.headers().get("cache-control").unwrap(),
        "no-store"
    );
    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["lines"], serde_json::json!(["second"]));
    assert_eq!(body["cursor"], (cursor + 1).to_string());
    assert_eq!(body["available"], true);

    let retired = app
        .oneshot(
            Request::builder()
                .uri("/admin/logs/stream")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(retired.status(), StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn last_admin_cannot_be_deleted() {
    let (state, pool) = state_with_db().await;
    ruscker_admin::db::users::create(&ruscker_admin::db::ConfigDb::Sqlite(pool.clone()), "root", "rootpass1", Role::Admin, false, &[], None)
        .await
        .unwrap();
    // Mint an admin session directly (the shared store the router reads).
    let sid = state
        .admin_sessions
        .create(Role::Admin, Some("root".into()))
        .await;
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

#[tokio::test]
async fn consolidated_user_edit_is_prefilled_and_saves_all_fields() {
    let (state, pool) = state_with_db().await;
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool.clone());
    ruscker_admin::db::users::create(
        &db,
        "root",
        "rootpass1",
        Role::Admin,
        false,
        &[],
        None,
    )
    .await
    .unwrap();
    ruscker_admin::db::users::create(
        &db,
        "alice",
        "alicepass1",
        Role::Viewer,
        false,
        &["old-group".to_string()],
        Some("root"),
    )
    .await
    .unwrap();
    ruscker_admin::db::users::update_profile(
        &db,
        "alice",
        Some("Old department"),
        Some("old@example.com"),
        None,
        Some("root"),
    )
    .await
    .unwrap();

    let sid = state
        .admin_sessions
        .create(Role::Admin, Some("root".into()))
        .await;
    let cookie = format!("{COOKIE_NAME}={sid}");

    let list = router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/admin/users")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(list.status(), StatusCode::OK);
    let list_body = axum::body::to_bytes(list.into_body(), 1 << 20)
        .await
        .unwrap();
    let list_body = std::str::from_utf8(&list_body).unwrap();
    assert!(list_body.contains(r#"href="/admin/users/alice/edit""#));

    let edit = router(state.clone())
        .oneshot(
            Request::builder()
                .uri("/admin/users/alice/edit")
                .header("cookie", &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(edit.status(), StatusCode::OK);
    let edit_body = axum::body::to_bytes(edit.into_body(), 1 << 20)
        .await
        .unwrap();
    let edit_body = std::str::from_utf8(&edit_body).unwrap();
    assert!(edit_body.contains(r#"value="old-group""#));
    assert!(edit_body.contains(r#"value="Old department""#));
    assert!(edit_body.contains(r#"value="old@example.com""#));

    let (status, loc) = post(
        state,
        "/admin/users/alice/edit",
        "role=editor&groups=analysts%2C+managers&setor=Data&email=alice%40example.com&celular=555-0100",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/users?flash=saved");

    let user = ruscker_admin::db::users::fetch(&db, "alice")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.role, Role::Editor);
    assert_eq!(user.groups, vec!["analysts", "managers"]);
    assert_eq!(user.setor.as_deref(), Some("Data"));
    assert_eq!(user.email.as_deref(), Some("alice@example.com"));
    assert_eq!(user.celular.as_deref(), Some("555-0100"));
}

#[tokio::test]
async fn password_policy_enforced_on_create_and_reset() {
    let (state, pool) = state_with_db().await;
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool.clone());
    ruscker_admin::db::users::create(&db, "root", "Root!pass1", Role::Admin, false, &[], None)
        .await
        .unwrap();
    let sid = state
        .admin_sessions
        .create(Role::Admin, Some("root".into()))
        .await;
    let cookie = format!("{COOKIE_NAME}={sid}");

    // The report's exact motivating example: teste123 (8 chars, but no
    // uppercase and no special) must be rejected with the policy flash…
    let (status, loc) = post(
        state.clone(),
        "/admin/users",
        "username=dave&password=teste123&role=viewer",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/users?flash=weak-password");
    assert!(
        ruscker_admin::db::users::fetch(&db, "dave").await.unwrap().is_none(),
        "weak-password create must not persist the user"
    );

    // …and a compliant password passes.
    let (_, loc) = post(
        state.clone(),
        "/admin/users",
        "username=dave&password=Dave!pass1&role=viewer",
        Some(&cookie),
    )
    .await;
    assert_eq!(loc, "/admin/users?flash=created");

    // Admin reset: weak rejected (flash lands on the edit page), the
    // stored hash untouched…
    let (_, loc) = post(
        state.clone(),
        "/admin/users/dave/password",
        "password=12345678",
        Some(&cookie),
    )
    .await;
    assert_eq!(loc, "/admin/users/dave/edit?flash=weak-password");
    assert!(
        ruscker_admin::db::users::verify_login(&db, "dave", "Dave!pass1")
            .await
            .unwrap()
            .is_some(),
        "old password still valid after a rejected reset"
    );

    // …and a compliant reset goes through.
    let (_, loc) = post(
        state,
        "/admin/users/dave/password",
        "password=N0va!senha",
        Some(&cookie),
    )
    .await;
    assert_eq!(loc, "/admin/users?flash=saved");
    assert!(ruscker_admin::db::users::verify_login(&db, "dave", "N0va!senha")
        .await
        .unwrap()
        .is_some());
}

/// The users list paginates + searches server-side (#999): `?q=` filters
/// against the DB (not the rendered DOM) and an out-of-range `?page=`
/// clamps instead of 404ing or rendering an empty table.
#[tokio::test]
async fn users_list_server_side_search_and_page_clamp() {
    let (state, _pool) = state_with_db().await;
    let db = state.db.clone().unwrap();
    for (name, pw) in [
        ("root", "Root!pass1"),
        ("alice", "Alice!pass1"),
        ("bob", "Bob!pass12"),
    ] {
        ruscker_admin::db::users::create(
            &db,
            name,
            pw,
            if name == "root" { Role::Admin } else { Role::Viewer },
            false,
            &[],
            None,
        )
        .await
        .unwrap();
    }
    let sid = state
        .admin_sessions
        .create(Role::Admin, Some("root".into()))
        .await;
    let cookie = format!("{COOKIE_NAME}={sid}");

    let get_body = |uri: String| {
        let state = state.clone();
        let cookie = cookie.clone();
        async move {
            let resp = router(state)
                .oneshot(
                    Request::builder()
                        .uri(uri)
                        .header("cookie", cookie)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            let body = axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap();
            String::from_utf8(body.to_vec()).unwrap()
        }
    };

    // The search hits the DB, so it filters rows before they render.
    let body = get_body("/admin/users?q=alice".to_string()).await;
    assert!(body.contains(r#"href="/admin/users/alice/edit""#));
    assert!(!body.contains(r#"href="/admin/users/bob/edit""#));
    // The term is echoed back into the search box (HTML-escaped by Askama).
    assert!(body.contains(r#"value="alice""#));

    // No match ⇒ empty table, no user rows.
    let body = get_body("/admin/users?q=zzz-no-such-user".to_string()).await;
    assert!(!body.contains("/edit\""));

    // An out-of-range page clamps back into range (all 3 fit on page 1).
    let body = get_body("/admin/users?page=999".to_string()).await;
    assert!(body.contains(r#"href="/admin/users/alice/edit""#));
    assert!(body.contains(r#"href="/admin/users/bob/edit""#));
}

/// A client system's "change your password" link (#1068 follow-up): a
/// signed-out person comes back to the password page after signing in,
/// the page links back to the system at its REGISTERED address, and after
/// the change it says so and offers the way back.
#[tokio::test]
async fn password_page_returns_to_the_client_system() {
    use ruscker_admin::db::oidc_clients::{self, ClientSettings};
    let (state, pool) = state_with_db().await;
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool);
    let client = |id: &str, home: Option<&str>| ClientSettings {
        name: format!("Sistema {id}"),
        redirect_uris: vec![format!("https://{id}.example/cb")],
        access_groups: vec![],
        require_mfa: false,
        enabled: true,
        logo: None,
        allowed_scopes: None,
        client_uri: home.map(String::from),
        post_logout_redirect_uris: vec![],
    };
    oidc_clients::create(&db, "ykatu", &client("ykatu", Some("https://ykatu.example/")), None).await.unwrap();
    oidc_clients::create(&db, "semhome", &client("semhome", None), None).await.unwrap();
    ruscker_admin::db::users::create(&db, "ana", "CorrectPass9!", Role::Viewer, false, &[], None)
        .await
        .unwrap();
    let app = router(state);
    let send = |req: Request<Body>| {
        let app = app.clone();
        async move { app.oneshot(req).await.unwrap() }
    };
    let location = |r: &axum::response::Response| {
        r.headers().get("location").and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
    };
    let body = |r: axum::response::Response| async move {
        String::from_utf8(axum::body::to_bytes(r.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap()
    };

    // Signed out: the login page, carrying the way back here.
    let r = send(Request::builder().uri("/admin/account/password?client_id=ykatu").body(Body::empty()).unwrap()).await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(location(&r), "/admin/login?next=%2Fadmin%2Faccount%2Fpassword%3Fclient_id%3Dykatu");
    // …and only for the password page: another admin address keeps the
    // plain login redirect.
    let r = send(Request::builder().uri("/admin/users").body(Body::empty()).unwrap()).await;
    assert_eq!(location(&r), "/admin/login");

    // Signing in lands on the password page, not the panel.
    let r = send(
        Request::builder()
            .method("POST")
            .uri("/admin/login")
            .header("content-type", "application/x-www-form-urlencoded")
            .body(Body::from(
                "username=ana&password=CorrectPass9!&next=%2Fadmin%2Faccount%2Fpassword%3Fclient_id%3Dykatu",
            ))
            .unwrap(),
    )
    .await;
    assert_eq!(location(&r), "/admin/account/password?client_id=ykatu");
    let cookie = r.headers().get("set-cookie").and_then(|v| v.to_str().ok()).unwrap().split(';').next().unwrap().to_string();

    // The page links back to the registered address.
    let get = |uri: &str, cookie: &str| {
        Request::builder().uri(uri).header("cookie", cookie).body(Body::empty()).unwrap()
    };
    let html = body(send(get("/admin/account/password?client_id=ykatu", &cookie)).await).await;
    assert!(html.contains(r#"href="https://ykatu.example/""#), "link back to the system");
    assert!(html.contains("Sistema ykatu"));
    assert!(html.contains(r#"name="client_id" value="ykatu""#));
    // No home address, unknown id, or an address in the query: no link.
    for q in ["client_id=semhome", "client_id=nope", "client_id=https%3A%2F%2Fevil.example"] {
        let html = body(send(get(&format!("/admin/account/password?{q}"), &cookie)).await).await;
        assert!(!html.contains("pw-back"), "{q}");
        assert!(!html.contains("evil.example"), "{q}");
    }

    // The change: back to the page, which says it is done and offers the way back.
    let r = send(
        Request::builder()
            .method("POST")
            .uri("/admin/account/password")
            .header("content-type", "application/x-www-form-urlencoded")
            .header("cookie", &cookie)
            .body(Body::from("current=CorrectPass9!&new_password=Nova-Senha9!&confirm=Nova-Senha9!&client_id=ykatu"))
            .unwrap(),
    )
    .await;
    assert_eq!(location(&r), "/admin/account/password?client_id=ykatu&changed=1");
    let cookie = r.headers().get("set-cookie").and_then(|v| v.to_str().ok()).unwrap().split(';').next().unwrap().to_string();
    let html = body(send(get("/admin/account/password?client_id=ykatu&changed=1", &cookie)).await).await;
    assert!(html.contains("data-keep"), "a lasting confirmation, not a toast");
    assert!(!html.contains(r#"name="new_password""#), "not the form again");
    assert!(html.matches(r#"href="https://ykatu.example/""#).count() >= 2);
}

/// A must-change account already headed for the password page is not
/// sent to `/admin/account/password?next=/admin/account/password…`.
#[tokio::test]
async fn must_change_login_headed_for_the_password_page_goes_there_once() {
    let (state, pool) = state_with_db().await;
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool);
    ruscker_admin::db::users::create(&db, "bob", "bobpass12", Role::Viewer, true, &[], None).await.unwrap();
    let app = router(state);
    let r = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/login")
                .header("content-type", "application/x-www-form-urlencoded")
                .body(Body::from("username=bob&password=bobpass12&next=%2Fadmin%2Faccount%2Fpassword%3Fclient_id%3Dykatu"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        r.headers().get("location").and_then(|v| v.to_str().ok()),
        Some("/admin/account/password?client_id=ykatu")
    );
    // A `changed=1` typed into the link does not hide the form from an
    // account that must still change its password.
    use ruscker_admin::db::oidc_clients::{self, ClientSettings};
    oidc_clients::create(
        &db,
        "ykatu",
        &ClientSettings {
            name: "Ykatu".into(),
            redirect_uris: vec!["https://ykatu.example/cb".into()],
            access_groups: vec![],
            require_mfa: false,
            enabled: true,
            logo: None,
            allowed_scopes: None,
            client_uri: Some("https://ykatu.example/".into()),
            post_logout_redirect_uris: vec![],
        },
        None,
    )
    .await
    .unwrap();
    let cookie = r.headers().get("set-cookie").and_then(|v| v.to_str().ok()).unwrap().split(';').next().unwrap().to_string();
    let page = app
        .oneshot(
            Request::builder()
                .uri("/admin/account/password?client_id=ykatu&changed=1")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let html = String::from_utf8(axum::body::to_bytes(page.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
    assert!(html.contains(r#"name="new_password""#), "the form is still there");
}

/// Security review 2026-11-19: the password page's "current password"
/// check is a password oracle for whoever holds a session, so it shares
/// the re-authentication budget (5 per minute per account, counted at
/// admission), and the right password is refused too once over budget.
#[tokio::test]
async fn password_change_guesses_are_throttled() {
    let (state, pool) = state_with_db().await;
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool);
    ruscker_admin::db::users::create(&db, "gina", "CorrectPass9!", Role::Viewer, false, &[], None)
        .await
        .unwrap();
    let sid = state.admin_sessions.create(Role::Viewer, Some("gina".into())).await;
    let cookie = format!("{COOKIE_NAME}={sid}");
    let app = router(state);
    let try_current = |current: &str| {
        let app = app.clone();
        let cookie = cookie.clone();
        let body = format!("current={current}&new_password=Nova-Senha9%21&confirm=Nova-Senha9%21");
        async move {
            app.oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/admin/account/password")
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("cookie", cookie)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
        }
    };
    for _ in 0..5 {
        assert_eq!(try_current("Wrong-Guess9%21").await, StatusCode::OK, "a wrong guess re-renders the form");
    }
    assert_eq!(try_current("Wrong-Guess9%21").await, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(try_current("CorrectPass9%21").await, StatusCode::TOO_MANY_REQUESTS, "over budget: even the right one waits");
    assert!(
        ruscker_admin::db::users::verify_login(&db, "gina", "CorrectPass9!").await.unwrap().is_some(),
        "the password did not change"
    );
}

/// Security review 2026-11-19: "changing" the administrator-assigned
/// password to itself must not clear the mandatory rotation.
#[tokio::test]
async fn new_password_must_differ_from_the_current_one() {
    let (state, pool) = state_with_db().await;
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool);
    ruscker_admin::db::users::create(&db, "hugo", "Initial-Pass9!", Role::Viewer, true, &[], None)
        .await
        .unwrap();
    let sid = state.admin_sessions.create(Role::Viewer, Some("hugo".into())).await;
    let resp = router(state)
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/account/password")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("cookie", format!("{COOKIE_NAME}={sid}"))
                .body(Body::from(
                    "current=Initial-Pass9%21&new_password=Initial-Pass9%21&confirm=Initial-Pass9%21",
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "re-rendered with an error, not a redirect");
    let u = ruscker_admin::db::users::fetch(&db, "hugo").await.unwrap().unwrap();
    assert!(u.must_change_password, "rotation still required");
}

/// Security review 2026-11-19: the mandatory-password barrier used to
/// let the request through when the account could not be read. It now
/// refuses (503) instead, and a session whose account is gone is sent to
/// sign in again.
#[tokio::test]
async fn must_change_barrier_fails_closed() {
    let (state, pool) = state_with_db().await;
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool.clone());
    ruscker_admin::db::users::create(&db, "ivo", "Initial-Pass9!", Role::Admin, true, &[], None)
        .await
        .unwrap();
    let app = router(state.clone());
    let get = |cookie: String| {
        let app = app.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .uri("/admin/users")
                    .header("cookie", cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };
    // A session for an account that does not exist.
    let ghost = state.admin_sessions.create(Role::Admin, Some("ghost".into())).await;
    let r = get(format!("{COOKIE_NAME}={ghost}")).await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(r.headers().get("location").and_then(|v| v.to_str().ok()), Some("/admin/login"));
    // The database goes away: the session cannot be confirmed against the
    // account, so nothing is let through — back to the sign-in.
    let sid = state.admin_sessions.create(Role::Admin, Some("ivo".into())).await;
    pool.close().await;
    let r = get(format!("{COOKIE_NAME}={sid}")).await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(r.headers().get("location").and_then(|v| v.to_str().ok()), Some("/admin/login"));
}

/// Codex review of #1122: the Postgres session store answers "no
/// session" when its database fails. The must-change decision used to
/// cache "no change required" for that answer, so once the database came
/// back the same session skipped the barrier for the whole cache TTL.
#[tokio::test]
async fn a_failing_session_store_leaves_no_cached_clearance() {
    use std::sync::atomic::{AtomicBool, Ordering};
    use ruscker_admin::auth::AdminSessionStore as _;
    /// The in-memory store, made to answer "no session" while `down`.
    #[derive(Debug)]
    struct Flaky {
        inner: ruscker_admin::auth::InMemoryAdminSessionStore,
        down: AtomicBool,
    }
    #[async_trait::async_trait]
    impl ruscker_admin::auth::AdminSessionStore for Flaky {
        async fn create_at_epoch(&self, role: Role, actor: Option<String>, epoch: i64) -> String {
            self.inner.create_at_epoch(role, actor, epoch).await
        }
        async fn validate(&self, id: &str) -> Option<ruscker_admin::auth::SessionInfo> {
            if self.down.load(Ordering::SeqCst) { None } else { self.inner.validate(id).await }
        }
        async fn remove(&self, id: &str) -> bool {
            self.inner.remove(id).await
        }
        async fn revoke_by_actor(&self, actor: &str) {
            self.inner.revoke_by_actor(actor).await
        }
    }
    let (mut state, pool) = state_with_db().await;
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool);
    ruscker_admin::db::users::create(&db, "lia", "Initial-Pass9!", Role::Admin, true, &[], None)
        .await
        .unwrap();
    let store = std::sync::Arc::new(Flaky {
        inner: ruscker_admin::auth::InMemoryAdminSessionStore::default(),
        down: AtomicBool::new(false),
    });
    state.admin_sessions = store.clone();
    let sid = store.create(Role::Admin, Some("lia".into())).await;
    let get = || {
        let app = router(state.clone());
        let sid = sid.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .uri("/admin/users")
                    .header("cookie", format!("{COOKIE_NAME}={sid}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
        }
    };
    store.down.store(true, Ordering::SeqCst);
    let _ = get().await; // the store is failing: refused, and nothing cached
    store.down.store(false, Ordering::SeqCst);
    let r = get().await;
    assert_eq!(r.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        r.headers().get("location").and_then(|v| v.to_str().ok()),
        Some("/admin/account/password"),
        "the barrier holds once the store is back"
    );
}

/// Security review 2026-11-19 (finding 3), redone after the Codex review
/// of #1123: a sign-in verifies the password against one read of the
/// account and creates the session afterwards. A demotion, reset or delete
/// that lands in between has already revoked the account's sessions — not
/// this one. The session carries the account's authentication generation
/// read with the password, and every change moves it in its own
/// transaction, so the session is refused from its first use. The race is
/// staged by minting at the generation the sign-in read, after the change.
#[tokio::test]
async fn a_session_issued_under_an_old_generation_is_refused() {
    let (state, pool) = state_with_db().await;
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool);
    ruscker_admin::db::users::create(&db, "jade", "Jade-Pass9!", Role::Admin, false, &[], None)
        .await
        .unwrap();
    // Another Admin, so jade is not the last one (that demotion is refused).
    ruscker_admin::db::users::create(&db, "boss", "Boss-Pass9!", Role::Admin, false, &[], None)
        .await
        .unwrap();
    let app = router(state.clone());
    let get = |sid: String| {
        let app = app.clone();
        async move {
            app.oneshot(
                Request::builder()
                    .uri("/admin/users")
                    .header("cookie", format!("{COOKIE_NAME}={sid}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
        }
    };
    let (_, epoch) = ruscker_admin::db::users::verify_login_with_epoch(&db, "jade", "Jade-Pass9!")
        .await
        .unwrap()
        .unwrap();
    // Unchanged: the session stands.
    let sid = state.admin_sessions.create_at_epoch(Role::Admin, Some("jade".into()), epoch).await;
    assert_eq!(get(sid).await, StatusCode::OK);
    // Demoted while the password was being checked: refused.
    ruscker_admin::db::users::set_role(&db, "jade", Role::Viewer, None).await.unwrap();
    state.invalidate_identity_cache();
    let stale = state.admin_sessions.create_at_epoch(Role::Admin, Some("jade".into()), epoch).await;
    assert_eq!(get(stale).await, StatusCode::SEE_OTHER, "the stale Admin session is refused");
    // A password reset moves it too.
    let (_, epoch) = ruscker_admin::db::users::role_and_epoch(&db, "jade").await.unwrap().unwrap();
    ruscker_admin::db::users::set_password(&db, "jade", "Reset-Pass9!", false, None).await.unwrap();
    state.invalidate_identity_cache();
    let stale = state.admin_sessions.create_at_epoch(Role::Viewer, Some("jade".into()), epoch).await;
    assert_eq!(get(stale).await, StatusCode::SEE_OTHER);
}

/// Codex review of #1123: an account edit decided "role changed?" on a
/// role it had read before its transaction, so a concurrent promotion
/// could be overwritten without ending the promoted sessions. The decision
/// is now made inside the transaction, on the stored role.
#[tokio::test]
async fn an_account_edit_moves_the_generation_only_on_a_real_role_change() {
    let (_state, pool) = state_with_db().await;
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool);
    ruscker_admin::db::users::create(&db, "pia", "Pia-Pass9!", Role::Viewer, false, &[], None)
        .await
        .unwrap();
    let epoch = |db: ruscker_admin::db::ConfigDb| async move {
        ruscker_admin::db::users::role_and_epoch(&db, "pia").await.unwrap().unwrap().1
    };
    let edit = |role: Role| ruscker_admin::db::users::AccountUpdate {
        role,
        groups: &[],
        setor: None,
        email: None,
        celular: None,
    };
    let e0 = epoch(db.clone()).await;
    // Same role: no change, no move.
    ruscker_admin::db::users::update_account(&db, "pia", edit(Role::Viewer), None).await.unwrap();
    assert_eq!(epoch(db.clone()).await, e0);
    // Someone promotes her meanwhile…
    ruscker_admin::db::users::set_role(&db, "pia", Role::Editor, None).await.unwrap();
    let e1 = epoch(db.clone()).await;
    assert!(e1 > e0);
    // …and an edit written from a page that still showed "Viewer" puts the
    // Viewer role back: a real change, so the generation moves again.
    ruscker_admin::db::users::update_account(&db, "pia", edit(Role::Viewer), None).await.unwrap();
    assert!(epoch(db.clone()).await > e1);
}

/// Finding 3, second half: the session minted after a password change
/// takes the account's CURRENT role, not the one the old session carried.
#[tokio::test]
async fn password_change_mints_the_current_role() {
    let (state, pool) = state_with_db().await;
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool);
    ruscker_admin::db::users::create(&db, "kim", "Kim-Pass9!", Role::Viewer, false, &[], None)
        .await
        .unwrap();
    // A session that (wrongly) still says Admin — e.g. issued before a demotion.
    let sid = state.admin_sessions.create(Role::Admin, Some("kim".into())).await;
    let resp = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/account/password")
                .header("content-type", "application/x-www-form-urlencoded")
                .header("cookie", format!("{COOKIE_NAME}={sid}"))
                .body(Body::from("current=Kim-Pass9%21&new_password=Kim-Nova9%21&confirm=Kim-Nova9%21"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let new_sid = resp
        .headers()
        .get("set-cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|c| c.split(';').next())
        .and_then(|c| c.split('=').nth(1))
        .unwrap()
        .to_string();
    let info = state.admin_sessions.validate(&new_sid).await.unwrap();
    assert_eq!(info.role, Role::Viewer, "the new session is not an Admin one");
}
