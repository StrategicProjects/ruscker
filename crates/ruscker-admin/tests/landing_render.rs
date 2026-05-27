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
    let config = Config::from_yaml(&yaml).expect("parse config");
    let locales = Locales::load().expect("load locales");
    AppState {
        config: Arc::new(config),
        locales: Arc::new(locales),
        admin_auth: Default::default(),
        admin_sessions: Default::default(),
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
        metrics: ruscker_admin::metrics_cache::MetricsCache::new(),
        draining: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
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
async fn landing_stylesheet_link_is_present() {
    let (_, body) = get_with_cookie(None).await;
    assert!(body.contains(r#"href="/assets/styles.css""#));
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
    // Cache directives must NOT include `immutable` — otherwise
    // browsers skip revalidation even on user-initiated reload.
    let cache = headers
        .get(header::CACHE_CONTROL)
        .unwrap()
        .to_str()
        .unwrap();
    assert!(cache.contains("must-revalidate"));
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
