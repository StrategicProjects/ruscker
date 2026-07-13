//! Integration tests for API-spec proxy policies: per-client rate
//! limiting and CORS.
//!
//! Uses an in-process `Router::oneshot` — no socket bound, so there's
//! no `ConnectInfo` and the rate-limit client key falls back to
//! `"unknown"` (one shared bucket, which is exactly what we want for
//! a single-client test). No backend is wired, so a request that
//! passes the policy gate falls through to the `503 no backend`
//! response — which is enough to assert the gate's behaviour and
//! that CORS headers ride along on every API response.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::sync::Arc;
use tower::ServiceExt;

/// One API spec with both policies on. `rate-limit: 2/min` keeps the
/// test fast: the third request in a row is denied.
const YAML: &str = r#"
proxy:
  title: Test
  specs:
    - id: myapi
      display-name: My API
      container-image: org/api:1
      type: api
      api:
        cors: true
        rate-limit: "2/min"
    - id: plainapi
      display-name: Plain API
      container-image: org/api:1
      type: api
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
        access_counter: std::sync::Arc::new(ruscker_admin::access_counter::AccessCounter::default()),
        alerts: ruscker_admin::alerts::AlertSink::default(),
    }
}

async fn send(state: AppState, method: &str, uri: &str) -> axum::http::Response<Body> {
    let app = router(state);
    let req = Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    app.oneshot(req).await.unwrap()
}

#[tokio::test]
async fn cors_preflight_is_answered_locally() {
    let resp = send(state(), "OPTIONS", "/api/myapi").await;
    // Answered by the proxy, no upstream needed.
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let h = resp.headers();
    assert_eq!(
        h.get("access-control-allow-origin")
            .map(|v| v.to_str().unwrap()),
        Some("*")
    );
    assert!(h.contains_key("access-control-allow-methods"));
    assert!(h.contains_key("access-control-max-age"));
}

#[tokio::test]
async fn cors_headers_ride_on_normal_api_responses() {
    // No backend wired ⇒ 503, but the CORS headers must still be
    // present so a browser can read the (error) response.
    let resp = send(state(), "GET", "/api/myapi").await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .map(|v| v.to_str().unwrap()),
        Some("*")
    );
}

#[tokio::test]
async fn no_cors_headers_when_spec_opts_out() {
    let resp = send(state(), "GET", "/api/plainapi").await;
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        !resp.headers().contains_key("access-control-allow-origin"),
        "a spec without cors:true must not emit CORS headers"
    );
}

#[tokio::test]
async fn rate_limit_denies_after_budget_exhausted() {
    let st = state();
    // Budget is 2/min. First two pass the gate (then 503: no backend).
    for i in 0..2 {
        let resp = send(st.clone(), "GET", "/api/myapi").await;
        assert_eq!(
            resp.status(),
            StatusCode::SERVICE_UNAVAILABLE,
            "request {i} should pass the rate gate and hit the no-backend 503"
        );
    }
    // Third request within the window is throttled.
    let resp = send(st.clone(), "GET", "/api/myapi").await;
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(
        resp.headers().contains_key("retry-after"),
        "429 must carry a Retry-After header"
    );
    // CORS headers ride on the 429 too.
    assert_eq!(
        resp.headers()
            .get("access-control-allow-origin")
            .map(|v| v.to_str().unwrap()),
        Some("*")
    );
}

#[tokio::test]
async fn rate_limit_not_applied_to_unconfigured_spec() {
    let st = state();
    // `plainapi` has no rate-limit — many requests, never a 429.
    for _ in 0..10 {
        let resp = send(st.clone(), "GET", "/api/plainapi").await;
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }
}
