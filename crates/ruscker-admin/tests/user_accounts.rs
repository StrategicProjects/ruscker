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
    assert_eq!(ok_loc, "/admin/dashboard");

    let (bad_status, _) = post(state, "/admin/login", "username=alice&password=WRONG", None).await;
    assert_eq!(bad_status, StatusCode::UNAUTHORIZED);
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

    let (status, loc) = post(
        state.clone(),
        "/admin/account/password",
        "current=bobpass12&new_password=bobpass-new9&confirm=bobpass-new9",
        Some(&cookie),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/dashboard");

    // The guard no longer fires — the dashboard renders.
    let (dash_status, _) = get(state, "/admin/dashboard", Some(&cookie)).await;
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
