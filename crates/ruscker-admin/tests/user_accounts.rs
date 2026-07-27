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

#[tokio::test]
async fn successful_login_records_activity_but_failed_does_not() {
    use ruscker_admin::activity::{ActivityEventType, AuthMethod};
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

    // A correct password login enqueues exactly one `login.success`.
    let (ok, _) = post(
        state.clone(),
        "/admin/login",
        "username=alice&password=alicepass1",
        None,
    )
    .await;
    assert_eq!(ok, StatusCode::SEE_OTHER);

    // The drain task isn't started in this harness, so the receiver still
    // holds what the capture site enqueued (deterministic, no timing).
    let mut rx = state
        .activity
        .take_receiver()
        .expect("activity receiver available");
    let ev = rx.try_recv().expect("a login.success was recorded");
    assert_eq!(ev.event_type, ActivityEventType::LoginSuccess);
    assert_eq!(ev.username.as_deref(), Some("alice"));
    assert_eq!(ev.auth_method, Some(AuthMethod::Password));
    assert!(ev.spec_id.is_none(), "a login is not tied to a spec");

    // A wrong password records nothing.
    let (bad, _) = post(
        state.clone(),
        "/admin/login",
        "username=alice&password=WRONG",
        None,
    )
    .await;
    assert_eq!(bad, StatusCode::UNAUTHORIZED);
    assert!(
        rx.try_recv().is_err(),
        "a failed login must not record activity"
    );
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
