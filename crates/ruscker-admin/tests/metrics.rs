//! Integration tests for the opt-in Prometheus `/metrics` route (#109).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::sync::Arc;
use tower::ServiceExt;

fn state(yaml: &str) -> AppState {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let config = Config::from_yaml(yaml).expect("parse config");
    let locales = ruscker_admin::i18n::Locales::load().expect("load locales");
    AppState {
        config: Arc::new(config),
        locales: Arc::new(locales),
        admin_auth: Default::default(),
        admin_sessions: Default::default(),
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
        metrics: ruscker_admin::metrics_cache::MetricsCache::new(),
        draining: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }
}

async fn get_metrics(state: AppState) -> axum::http::Response<Body> {
    let req = Request::builder()
        .uri("/metrics")
        .body(Body::empty())
        .unwrap();
    router(state).oneshot(req).await.unwrap()
}

#[tokio::test]
async fn metrics_disabled_by_default_returns_404() {
    let resp = get_metrics(state("proxy:\n  title: T\n  specs: []\n")).await;
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn metrics_enabled_serves_prometheus_text() {
    let resp = get_metrics(state(
        "proxy:\n  title: T\n  metrics-enabled: true\n  specs: []\n",
    ))
    .await;
    assert_eq!(resp.status(), StatusCode::OK);
    let ct = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(ct.starts_with("text/plain"), "content-type was {ct}");

    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    // No backend wired in this test ⇒ up=0, no replicas.
    assert!(body.contains("ruscker_up 0"), "body:\n{body}");
    assert!(body.contains("ruscker_replicas_total 0"));
    assert!(body.contains("# TYPE ruscker_sessions_tracked gauge"));
}
