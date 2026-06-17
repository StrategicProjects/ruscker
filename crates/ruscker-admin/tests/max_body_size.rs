//! Integration tests for `max-body-size` enforcement on proxied
//! routes.
//!
//! Uses `Router::oneshot` with no backend wired. A request under the
//! cap passes the gate and falls through to `503 no backend`; one
//! over the declared `Content-Length` is rejected with `413` before
//! any backend work. The fast path keys off the `Content-Length`
//! header, so these tests set it explicitly and don't need a real
//! body of that size.

use axum::body::Body;
use axum::http::header::CONTENT_LENGTH;
use axum::http::{Request, StatusCode};
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::sync::Arc;
use tower::ServiceExt;

/// `tight` overrides the global with a 1 KiB cap and enables CORS;
/// `global` inherits the 1 MiB `proxy.max-body-size`.
const YAML: &str = r#"
proxy:
  max-body-size: 1m
  specs:
    - id: tight
      display-name: Tight API
      container-image: org/api:1
      type: api
      max-body-size: "1k"
      api:
        cors: true
    - id: global
      display-name: Global App
      container-image: org/app:1
"#;

fn state() -> AppState {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let config = Config::from_yaml(YAML).expect("parse config");
    let locales = ruscker_admin::i18n::Locales::load().expect("load locales");
    AppState {
        config: Arc::new(config),
        base_path: Arc::from(""),
        locales: Arc::new(locales),
        admin_auth: Default::default(),
        admin_sessions: Arc::new(ruscker_admin::auth::InMemoryAdminSessionStore::default()),
        log_buffer: None,
        login_limiter: Arc::new(ruscker_admin::auth::LoginRateLimiter::default_policy()),
        api_limiter: Arc::new(ruscker_admin::ratelimit::ApiRateLimiter::new()),
        db: None,
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

/// POST to `uri` declaring `content_length` bytes of body.
async fn post_with_len(uri: &str, content_length: u64) -> axum::http::Response<Body> {
    let app = router(state());
    let req = Request::builder()
        .method("POST")
        .uri(uri)
        .header(CONTENT_LENGTH, content_length)
        .body(Body::empty())
        .unwrap();
    app.oneshot(req).await.unwrap()
}

#[tokio::test]
async fn spec_override_rejects_over_its_cap() {
    // `tight` caps at 1 KiB; 2000 bytes is over.
    let resp = post_with_len("/api/tight", 2000).await;
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    // CORS headers ride on the 413 for an api+cors spec.
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .map(|v| v.to_str().unwrap()),
        Some("*")
    );
}

#[tokio::test]
async fn under_cap_passes_gate() {
    // 500 bytes is under `tight`'s 1 KiB cap, so it passes the body
    // gate and falls through to the no-backend 503.
    let resp = post_with_len("/api/tight", 500).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn global_default_applies_to_spec_without_override() {
    // `global` has no override, so the 1 MiB `proxy.max-body-size`
    // applies. 2 MiB is over.
    let resp = post_with_len("/app/global", 2 * 1024 * 1024).await;
    assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // Just under 1 MiB passes the gate (then 503: no backend).
    let resp = post_with_len("/app/global", 1000).await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
}
