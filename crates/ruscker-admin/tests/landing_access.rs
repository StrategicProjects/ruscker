//! Integration test for per-group app visibility on the public landing
//! (#155, Slice 3). Builds a config with one open spec and two
//! restricted ones (by group and by username) and asserts the landing
//! shows the right cards for anonymous / admin / per-group sessions.
//!
//! Uses an in-process `Router::oneshot` plus an in-memory SQLite for the
//! user → groups lookup. No socket bound.

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
    - id: open-app
      display-name: Open App
      container-image: demo/img
    - id: analysts-app
      display-name: Analysts App
      container-image: demo/img
      access-groups: [analysts]
    - id: vip-user-app
      display-name: VIP User App
      container-image: demo/img
      access-users: [carol]
"#;

/// A fresh, migrated SQLite at a unique temp path. (The crate's
/// `open_memory` is `#[cfg(test)]`-only, so integration tests open a
/// file instead.) Wipes the showcase seed rows so the landing
/// handler falls back to the YAML `proxy.specs` for the assertions
/// below (the DB-empty-→-YAML fallback in `routes::landing`).
async fn open_db() -> ConfigDb {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ruscker-landing-access-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let pool = ruscker_admin::db::open(&path).await.unwrap();
    sqlx::query("DELETE FROM specs").execute(&pool).await.unwrap();
    ConfigDb::Sqlite(pool)
}

/// AppState with admin auth configured (so sessions resolve) and an
/// in-memory DB for the user → groups lookup.
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
    }
}

async fn landing_body(state: AppState, cookie: Option<String>) -> String {
    let app = router(state);
    let mut req = Request::builder().method("GET").uri("/");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let resp = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Cards render `data-name="{display_name|lower}"`; this matches the
/// presence of a specific card without tripping on other markup.
fn has_card(body: &str, lower_name: &str) -> bool {
    body.contains(&format!(r#"data-name="{lower_name}""#))
}

#[tokio::test]
async fn anonymous_sees_only_open_specs() {
    let db = open_db().await;
    let state = app_state(db).await;
    let body = landing_body(state, None).await;

    assert!(has_card(&body, "open app"), "open spec visible to anyone");
    assert!(!has_card(&body, "analysts app"), "group-restricted spec hidden");
    assert!(!has_card(&body, "vip user app"), "user-restricted spec hidden");
    // Anonymous visitor gets the sign-in affordance.
    assert!(body.contains(r#"href="/admin/login""#), "sign-in link present");
}

#[tokio::test]
async fn show_admin_link_false_hides_signin_for_anonymous() {
    // #156: a fully-public portal can hide the admin entrance.
    const CFG: &str = r#"
proxy:
  title: Public Portal
  port: 8088
  landing-customization:
    show-admin-link: false
  specs:
    - id: open-app
      display-name: Open App
      container-image: demo/img
"#;
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let config = Config::from_yaml(CFG).expect("parse config");
    let mut state = app_state(open_db().await).await;
    state.config = Arc::new(config);
    let body = landing_body(state, None).await;

    assert!(has_card(&body, "open app"), "cards still render");
    assert!(
        !body.contains(r#"href="/admin/login""#),
        "sign-in entrance hidden when show-admin-link=false"
    );
}

#[tokio::test]
async fn base_path_nests_the_portal_and_keeps_health_at_root() {
    // #173 slice 1: with a base path the whole portal moves under it,
    // while /healthz stays at the root for load-balancer probes.
    let mut state = app_state(open_db().await).await;
    state.base_path = Arc::from("/box");
    let app = router(state);

    async fn code(app: axum::Router, uri: &str) -> StatusCode {
        app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap()
            .status()
    }

    // Landing lives under /box now; the bare root no longer matches.
    assert_eq!(code(app.clone(), "/box").await, StatusCode::OK);
    // `/box/` (what a browser/nginx sends) redirects to the canonical /box.
    assert_eq!(
        code(app.clone(), "/box/").await,
        StatusCode::PERMANENT_REDIRECT
    );
    // Deeper routes match under the prefix directly.
    assert_eq!(code(app.clone(), "/box/admin/login").await, StatusCode::OK);
    assert_eq!(code(app.clone(), "/").await, StatusCode::NOT_FOUND);
    // Health is mounted at the root regardless of the base path.
    assert_eq!(code(app.clone(), "/healthz").await, StatusCode::OK);
    assert_eq!(code(app, "/box/healthz").await, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn chrome_self_prefixes_under_base_path() {
    // #294: templates emit `{{ base }}`-prefixed URLs directly, so a page
    // rendered under `--base-path /box` is already correct WITHOUT the
    // response-body rewriter (which now only touches the `Location`
    // header). The landing uses the shared `_layout.html`, so this also
    // covers the `window.RUSCKER_BASE` injection the page JS relies on for
    // URLs it builds at runtime.
    let mut state = app_state(open_db().await).await;
    state.base_path = Arc::from("/box");
    let app = router(state);

    let resp = app
        .oneshot(
            Request::builder()
                .uri("/box")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        axum::body::to_bytes(resp.into_body(), 1 << 20)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();

    // Asset URLs carry the prefix straight from the template.
    assert!(
        body.contains(r#"href="/box/assets/styles.css"#),
        "stylesheet href self-prefixed"
    );
    // The runtime base is exposed for page JS that builds URLs at runtime.
    assert!(
        body.contains(r#"window.RUSCKER_BASE = "/box""#),
        "RUSCKER_BASE set in the layout"
    );
    // No bare (un-prefixed) chrome asset URL slipped through — proving the
    // template self-prefixes rather than relying on a body rewriter.
    assert!(
        !body.contains(r#"href="/assets/"#),
        "no bare /assets href in the rendered body"
    );
}

#[tokio::test]
async fn spec_form_action_self_prefixes_under_base_path() {
    // #357: the spec form's `<form action>` is the create/update POST
    // target. Under `--base-path` it must carry the prefix or the POST
    // 404s at the reverse proxy — it lacked `{{ base }}` (a #294
    // survivor), so editing a spec landed on `/admin/specs/{id}` (no
    // `/box`). Covers both the Edit (update) and New (create) actions.
    let db = open_db().await;
    let cfg = Config::from_yaml(
        "proxy:\n  specs:\n    - id: voila\n      display-name: Voila\n      container-image: x:1\n",
    )
    .expect("parse spec yaml");
    ruscker_admin::db::specs::upsert_one(&db, &cfg.proxy.specs[0], None)
        .await
        .expect("insert spec");

    let mut state = app_state(db).await;
    state.base_path = Arc::from("/box");
    let sid = state.admin_sessions.create(Role::Admin, None).await;
    let app = router(state);
    let cookie = format!("{COOKIE_NAME}={sid}");

    async fn body_text(resp: axum::response::Response) -> String {
        String::from_utf8(
            axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec(),
        )
        .unwrap()
    }

    // Edit form (the reported case): action → /box/admin/specs/voila.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .uri("/box/admin/specs/voila/edit")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_text(resp).await;
    assert!(
        body.contains(r#"action="/box/admin/specs/voila""#),
        "edit form action must be base-prefixed"
    );
    assert!(
        !body.contains(r#"action="/admin/specs/voila""#),
        "no bare (un-prefixed) edit action should slip through"
    );

    // New form: action → /box/admin/specs.
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/box/admin/specs/new")
                .header(header::COOKIE, &cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = body_text(resp).await;
    assert!(
        body.contains(r#"action="/box/admin/specs""#),
        "new form action must be base-prefixed"
    );
}

#[tokio::test]
async fn login_under_base_path_honors_admin_referer() {
    // #186 (#173 follow-up): a non-admin signing in from
    // `/box/admin/specs` should land back on `/box/admin/specs`, not
    // be bounced into `/box/admin/dashboard`. The handler must strip
    // the base prefix before its `starts_with("/admin/")` check; the
    // response middleware re-prefixes the `Location` on the way out.
    let db = open_db().await;
    ruscker_admin::db::users::create(
        &db,
        "alice",
        "alicepass1",
        Role::Admin,
        false,
        &[],
        Some("admin"),
    )
    .await
    .unwrap();
    let mut state = app_state(db).await;
    state.base_path = Arc::from("/box");
    let app = router(state);

    // Same-host referer mimicking what a real browser sends when the
    // user submitted the form from `/box/admin/specs`.
    let req = Request::builder()
        .method("POST")
        .uri("/box/admin/login")
        .header(header::HOST, "example.test")
        .header(header::REFERER, "https://example.test/box/admin/specs")
        .header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(Body::from("username=alice&password=alicepass1"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let loc = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(loc, "/box/admin/specs", "expected base-prefixed referer");
}

#[tokio::test]
async fn login_under_base_path_honors_landing_referer() {
    // The `/` short-circuit (#155) has to survive the base-path strip too:
    // a non-admin signing in from `/box/` should land on `/box/`, not
    // `/box/admin/dashboard`.
    let db = open_db().await;
    ruscker_admin::db::users::create(
        &db,
        "alice",
        "alicepass1",
        Role::Viewer,
        false,
        &[],
        Some("admin"),
    )
    .await
    .unwrap();
    let mut state = app_state(db).await;
    state.base_path = Arc::from("/box");
    let app = router(state);

    let req = Request::builder()
        .method("POST")
        .uri("/box/admin/login")
        .header(header::HOST, "example.test")
        .header(header::REFERER, "https://example.test/box/")
        .header(
            header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .body(Body::from("username=alice&password=alicepass1"))
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::SEE_OTHER);
    let loc = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    // The handler emits `Location: /`; the middleware prefixes it to
    // `/box/`. (The canonical landing is `/box`; the `/box/` form gets
    // a 308 to it on the next request — that's covered elsewhere.)
    assert_eq!(loc, "/box/", "landing referer should round-trip under prefix");
}

#[tokio::test]
async fn admin_session_sees_every_spec() {
    let db = open_db().await;
    let state = app_state(db).await;
    // A break-glass token session: Admin role, no username.
    let sid = state.admin_sessions.create(Role::Admin, None).await;
    let body = landing_body(state, Some(format!("{COOKIE_NAME}={sid}"))).await;

    assert!(has_card(&body, "open app"));
    assert!(has_card(&body, "analysts app"), "admin sees group-restricted");
    assert!(has_card(&body, "vip user app"), "admin sees user-restricted");
    // Signed-in affordance instead of sign-in.
    assert!(body.contains(r#"action="/admin/logout""#), "sign-out present");
}

#[tokio::test]
async fn group_member_sees_their_group_spec() {
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
    let body = landing_body(state, Some(format!("{COOKIE_NAME}={sid}"))).await;

    assert!(has_card(&body, "open app"));
    assert!(has_card(&body, "analysts app"), "alice is in analysts");
    assert!(!has_card(&body, "vip user app"), "alice is not the VIP user");
}

#[tokio::test]
async fn named_user_sees_their_user_spec() {
    let db = open_db().await;
    ruscker_admin::db::users::create(
        &db,
        "carol",
        "carolpass1",
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
        .create(Role::Viewer, Some("carol".to_string()))
        .await;
    let body = landing_body(state, Some(format!("{COOKIE_NAME}={sid}"))).await;

    assert!(has_card(&body, "open app"));
    assert!(has_card(&body, "vip user app"), "carol is the VIP user");
    assert!(!has_card(&body, "analysts app"), "carol is not in analysts");
}

#[tokio::test]
async fn spec_form_preview_matches_landing_svg_fit() {
    // #359: the preview's logo <img> must mirror the landing card's fit
    // rule — an SVG logo gets `rcover-img--contain` (object-fit: contain),
    // not the default `cover` that crops it. The binding is evaluated by
    // Alpine client-side; this is a regression guard that the rule stays
    // wired into the rendered preview (the landing applies the same rule
    // server-side, so the two cards agree).
    let state = app_state(open_db().await).await;
    let sid = state.admin_sessions.create(Role::Admin, None).await;
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/admin/specs/new")
                .header(header::COOKIE, format!("{COOKIE_NAME}={sid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec(),
    )
    .unwrap();
    assert!(
        body.contains("rcover-img--contain") && body.contains(".svg')"),
        "preview must apply the SVG contain-fit rule like the landing card"
    );
}

#[tokio::test]
async fn duplicate_opens_new_form_prefilled_with_fresh_id() {
    // #368: duplicating a spec opens the *New* form pre-filled from the
    // source, with a unique `-copy` id so the submit creates a brand-new
    // spec (the source is untouched).
    let db = open_db().await;
    let cfg = Config::from_yaml(
        "proxy:\n  specs:\n    - id: voila\n      display-name: Voila\n      container-image: img/voila:1\n",
    )
    .expect("parse spec yaml");
    ruscker_admin::db::specs::upsert_one(&db, &cfg.proxy.specs[0], None)
        .await
        .expect("insert spec");
    let state = app_state(db).await;
    let sid = state.admin_sessions.create(Role::Admin, None).await;
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/admin/specs/voila/duplicate")
                .header(header::COOKIE, format!("{COOKIE_NAME}={sid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body = String::from_utf8(
        axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec(),
    )
    .unwrap();
    // Fresh id + source fields carried over.
    assert!(body.contains(r#"value="voila-copy""#), "id suffixed to -copy");
    assert!(body.contains("img/voila:1"), "source image pre-filled");
    // New mode → the form posts to the create endpoint (no id in action).
    assert!(body.contains(r#"action="/admin/specs""#), "New-mode create action");
}

#[tokio::test]
async fn duplicate_works_for_a_config_only_spec() {
    // #907: `open-app` lives only in the YAML config (never inserted into
    // the DB). Duplicating it used to 404 (the handler only looked in the
    // DB) even though the button is shown for config-only rows; it must
    // fall back to the effective catalog and open the prefilled New form.
    let db = open_db().await; // empty DB → `open-app` is config-only
    let state = app_state(db).await;
    let sid = state.admin_sessions.create(Role::Admin, None).await;
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/admin/specs/open-app/duplicate")
                .header(header::COOKIE, format!("{COOKIE_NAME}={sid}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK, "config-only duplicate must not 404");
    let body = String::from_utf8(
        axum::body::to_bytes(resp.into_body(), 1 << 20).await.unwrap().to_vec(),
    )
    .unwrap();
    assert!(body.contains(r#"value="open-app-copy""#), "id suffixed to -copy");
    assert!(body.contains(r#"action="/admin/specs""#), "New-mode create action");
}

#[tokio::test]
async fn favicon_routes_serve_raster_with_correct_content_type() {
    // #374: Safari & co. ignore the SVG favicon and fall back to
    // `/favicon.ico` — previously unserved (black placeholder). Now the
    // raster hooks serve with the right content-types.
    let app = router(app_state(open_db().await).await);
    async fn ct(app: axum::Router, uri: &str) -> (StatusCode, String) {
        let resp = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .unwrap();
        let code = resp.status();
        let ct = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();
        (code, ct)
    }
    assert_eq!(ct(app.clone(), "/favicon.ico").await, (StatusCode::OK, "image/x-icon".into()));
    assert_eq!(ct(app.clone(), "/favicon-32.png").await, (StatusCode::OK, "image/png".into()));
    assert_eq!(
        ct(app.clone(), "/apple-touch-icon.png").await,
        (StatusCode::OK, "image/png".into())
    );
    assert_eq!(ct(app, "/favicon.svg").await, (StatusCode::OK, "image/svg+xml".into()));
}
