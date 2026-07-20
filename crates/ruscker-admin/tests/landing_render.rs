//! Integration test: render the landing for the real
//! `examples/application.yml` in each of the four locales and assert
//! key structural invariants. Catches template regressions before
//! they hit a browser.
//!
//! Uses an in-process `Router::oneshot` — no socket bound.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use ruscker_admin::i18n::Locales;
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::sync::Arc;
use tower::ServiceExt;

fn app_state() -> AppState {
    let yaml = std::fs::read_to_string("../../examples/application.yml")
        .expect("read examples/application.yml");
    // The fixture references ${DOCKER_REGISTRY_PASSWORD}; provide a
    // dummy so parsing succeeds.
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    state_from_yaml(&yaml)
}

/// Build an `AppState` (no DB, so the YAML `landing-customization` is the
/// source of truth) from an inline config. Used by the sections-layout
/// test, which needs a config that sets `catalog-layout: sections`.
fn state_from_yaml(yaml: &str) -> AppState {
    let config = Config::from_yaml(yaml).expect("parse config");
    let locales = Locales::load().expect("load locales");
    AppState {
        config: Arc::new(config),
        base_path: Arc::from(""),
        locales: Arc::new(locales),
        admin_auth: Default::default(),
        admin_sessions: Arc::new(ruscker_admin::auth::InMemoryAdminSessionStore::default()),
        log_buffer: None,
        login_limiter: std::sync::Arc::new(ruscker_admin::auth::LoginRateLimiter::default_policy()),
        api_limiter: std::sync::Arc::new(ruscker_admin::ratelimit::ApiRateLimiter::new()),
        db: None,
        images_dir: None,
        master_key: Default::default(),
        backend: None,
        replicas: std::sync::Arc::new(tokio::sync::RwLock::new(Default::default())),
        cookie_key: ruscker_proxy::sticky::CookieKey::random(),
        spawn_locks: std::sync::Arc::new(dashmap::DashMap::new()),
        sessions: std::sync::Arc::new(ruscker_admin::sessions::InMemorySessionStore::new()),
        logout_index: std::sync::Arc::new(dashmap::DashMap::new()),
        leader: std::sync::Arc::new(ruscker_admin::leader::AlwaysLeader),
        metrics: ruscker_admin::metrics_cache::MetricsCache::new(),
        draining: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        spec_cache: std::sync::Arc::new(dashmap::DashMap::new()),
        identity_cache: Default::default(),
        catalog_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        access_counter: std::sync::Arc::new(ruscker_admin::access_counter::AccessCounter::default()),
        alerts: ruscker_admin::alerts::AlertSink::default(),
        activity: ruscker_admin::activity::ActivitySink::default(),
    }
}

async fn get_with_cookie(cookie: Option<&str>) -> (StatusCode, String) {
    let app = router(app_state());
    let mut req = Request::builder().method("GET").uri("/");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, c);
    }
    let response = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

#[tokio::test]
async fn landing_renders_default_locale() {
    let (status, body) = get_with_cookie(None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains(r#"<html lang="pt""#), "default locale is pt-BR");
    // seo-title from the example landing-customization drives <title>.
    assert!(body.contains("Ruscker Demo Portal"));
    // Per-locale intro (pt) from landing-customization.intro-locales.
    assert!(body.contains("demonstração"), "pt intro should render");
    // 8 specs in examples/application.yml → 8 cards rendered.
    // Cards are <a class="rcard"> in the v2 layout.
    assert_eq!(body.matches(r#"class="rcard"#).count(), 8);
}

#[tokio::test]
async fn page_title_falls_back_to_proxy_title() {
    // #926 pt. 1: with no seo-title and no editor title, the browser
    // tab (<title> + og:title) must show the configured `proxy.title`
    // — it used to skip it and always show the localized default.
    let yaml = r#"
proxy:
  title: Meu Portal
  specs: []
"#;
    let app = router(state_from_yaml(yaml));
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(
        body.contains("<title>Meu Portal</title>"),
        "tab title uses proxy.title"
    );
    assert!(
        body.contains(r#"<meta property="og:title" content="Meu Portal">"#),
        "og:title uses proxy.title"
    );
}

#[tokio::test]
async fn page_title_prefers_editor_title_then_seo_title() {
    // The editor (landing-customization) title outranks proxy.title…
    let yaml = r#"
proxy:
  title: YAML Title
  landing-customization:
    title: Editor Title
  specs: []
"#;
    let app = router(state_from_yaml(yaml));
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("<title>Editor Title</title>"));

    // …and an explicit seo-title outranks both.
    let yaml = r#"
proxy:
  title: YAML Title
  landing-customization:
    title: Editor Title
    seo-title: SEO Title
  specs: []
"#;
    let app = router(state_from_yaml(yaml));
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("<title>SEO Title</title>"));
}

#[tokio::test]
async fn sections_layout_groups_cards_by_type() {
    // `catalog-layout: sections` (#701) renders one labeled group per
    // app type, in canonical order, each heading carrying the Alpine
    // per-section count binding so it hides when filters empty it.
    let yaml = r#"
proxy:
  title: Sections Test
  landing-customization:
    catalog-layout: sections
  specs:
    - id: alpha
      display-name: Alpha
      container-image: img:1
      template-properties:
        type: app
    - id: beta
      display-name: Beta
      container-image: img:2
      template-properties:
        type: report
    - id: gamma
      display-name: Gamma
      container-image: img:3
      template-properties:
        type: app
"#;
    let app = router(state_from_yaml(yaml));
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();

    // Sections wrapper + exactly the two present types get a heading
    // (app, report) — empty types (talk/package/api/link) are omitted.
    assert!(body.contains("catalog--sections"), "sections wrapper class");
    assert_eq!(
        body.matches("catalog-group__head").count(),
        2,
        "one heading per present type (app, report) only"
    );
    // Per-section x-show count bindings drive heading visibility.
    assert!(body.contains(r#"x-show="(sections['app'] || 0) > 0""#));
    assert!(body.contains(r#"x-show="(sections['report'] || 0) > 0""#));
    // All three cards still render (2 app + 1 report).
    assert_eq!(body.matches(r#"class="rcard"#).count(), 3);
}

#[tokio::test]
async fn grid_layout_is_a_single_unlabeled_group() {
    // The default (no `catalog-layout`) renders one group with no
    // heading and no per-section binding — the sections wrappers stay
    // visually inert (#701).
    let yaml = r#"
proxy:
  title: Grid Test
  specs:
    - id: alpha
      display-name: Alpha
      container-image: img:1
      template-properties:
        type: app
"#;
    let app = router(state_from_yaml(yaml));
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();
    assert!(!body.contains("catalog--sections"), "no sections modifier in grid");
    assert_eq!(body.matches("catalog-group__head").count(), 0, "no headings in grid");
    assert!(!body.contains("sections['"), "no per-section binding in grid");
}

#[tokio::test]
async fn filters_toggle_hides_filters_without_hiding_search() {
    // The crate-wide dead-code suppression hid that `show-filters` was
    // carried into the page model but never consulted by the template
    // (#935). Search and filters are independent appearance switches.
    let yaml = r#"
proxy:
  title: Filter Toggle Test
  landing-customization:
    show-search: true
    show-filters: false
  specs:
    - id: alpha
      display-name: Alpha
      container-image: img:1
"#;
    let app = router(state_from_yaml(yaml));
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    let body = String::from_utf8(body.to_vec()).unwrap();

    assert!(body.contains(r#"type="search""#), "search remains visible");
    assert!(!body.contains(r#"x-model="subject""#), "subject filter is hidden");
    assert!(!body.contains(r#"x-model="status_""#), "status filter is hidden");
    assert!(!body.contains(r#"class="chip""#), "type chips are hidden");
}

#[tokio::test]
async fn landing_honors_locale_cookie() {
    // Distinctive substrings from the per-locale intro in the example's
    // landing-customization.intro-locales — proves the locale cookie
    // drives the rendered copy.
    for (cookie, lang, intro) in [
        ("ruscker_locale=en", "en", "filters below"),
        ("ruscker_locale=es", "es", "demostración"),
        ("ruscker_locale=fr", "fr", "démonstration"),
    ] {
        let (status, body) = get_with_cookie(Some(cookie)).await;
        assert_eq!(status, StatusCode::OK, "locale {lang}");
        assert!(
            body.contains(&format!(r#"<html lang="{lang}""#)),
            "{lang}: html lang attribute missing"
        );
        assert!(
            body.contains(intro),
            "{lang}: intro {intro:?} not in body"
        );
    }
}

#[tokio::test]
async fn landing_emits_data_theme_when_set() {
    let (_, body) = get_with_cookie(Some("ruscker_theme=dark")).await;
    assert!(body.contains(r#"data-theme="dark""#));

    let (_, body) = get_with_cookie(Some("ruscker_theme=light")).await;
    assert!(body.contains(r#"data-theme="light""#));

    let (_, body) = get_with_cookie(None).await;
    assert!(
        !body.contains("data-theme="),
        "auto theme leaves the attribute off so prefers-color-scheme kicks in"
    );
}

#[tokio::test]
async fn set_theme_auto_removes_the_root_scoped_cookie() {
    let app = router(app_state());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/__set/theme")
                .header(header::COOKIE, "ruscker_theme=dark")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("theme=auto"))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let removal = response
        .headers()
        .get(header::SET_COOKIE)
        .expect("Auto emits a removal cookie")
        .to_str()
        .unwrap();
    assert!(removal.starts_with("ruscker_theme="), "{removal}");
    assert!(removal.contains("Path=/"), "{removal}");
    assert!(removal.contains("Max-Age=0"), "{removal}");
}

#[tokio::test]
async fn landing_stylesheet_link_is_present() {
    let (_, body) = get_with_cookie(None).await;
    // The URL carries a `?v=<version>` cache-buster (#289).
    assert!(body.contains(r#"href="/assets/styles.css?v="#));
}

#[tokio::test]
async fn assets_styles_css_served_with_cache_headers() {
    let app = router(app_state());
    let response = app
        .oneshot(
            Request::builder()
                .uri("/assets/styles.css")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "text/css; charset=utf-8"
    );
    // Bundled assets are cached for a short window to avoid a
    // conditional-GET round-trip per navigation (#269), but NOT
    // `immutable` — the bytes change across upgrades under the same URL,
    // so a hard reload / post-window request must still pick up new
    // bytes.
    let cache = headers
        .get(header::CACHE_CONTROL)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cache.contains("max-age=300"));
    assert!(!cache.contains("immutable"));
}

#[tokio::test]
async fn landing_carries_security_headers() {
    let app = router(app_state());
    let response = app
        .oneshot(Request::builder().uri("/").body(Body::empty()).unwrap())
        .await
        .unwrap();
    let h = response.headers();
    assert_eq!(h.get("x-content-type-options").unwrap(), "nosniff");
    assert_eq!(h.get("x-frame-options").unwrap(), "DENY");
    assert_eq!(h.get("referrer-policy").unwrap(), "same-origin");
    let csp = h.get("content-security-policy").unwrap().to_str().unwrap();
    assert!(csp.contains("default-src 'self'"));
    assert!(csp.contains("frame-ancestors 'none'"));
}

#[tokio::test]
async fn assets_img_rejects_path_traversal() {
    // The `/assets/img/{filename}` guard must reject decoded
    // slash / parent-dir tricks before any FS read. Axum decodes
    // `%2F` into `/`, which the `.contains('/')` check catches;
    // route-mismatch yields 404. Never a 200 / file read.
    for evil in [
        "/assets/img/..%2f..%2fetc%2fpasswd",
        "/assets/img/%2e%2e%2f%2e%2e%2fsecret",
        "/assets/img/..%5c..%5cwindows",
    ] {
        let app = router(app_state());
        let response = app
            .oneshot(Request::builder().uri(evil).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert!(
            response.status() == StatusCode::BAD_REQUEST
                || response.status() == StatusCode::NOT_FOUND,
            "traversal `{evil}` should be rejected, got {}",
            response.status()
        );
    }
}

#[tokio::test]
async fn set_locale_redirect_is_same_origin_only() {
    // An attacker-controlled Referer pointing off-origin must not
    // become an open redirect — we keep only the path.
    let app = router(app_state());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/__set/locale")
                .header(header::REFERER, "https://evil.example.com/phish")
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("locale=en"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    let location = response.headers().get(header::LOCATION).unwrap().to_str().unwrap();
    // Path preserved, host dropped → stays on our origin.
    assert_eq!(location, "/phish");
    assert!(!location.contains("evil.example.com"));
}
