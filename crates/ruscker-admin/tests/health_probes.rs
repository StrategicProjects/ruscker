//! Integration tests for the `/healthz` and `/readyz` probes.
//!
//! Uses an in-process `Router::oneshot` — no socket bound. Covers:
//! liveness always 200; readiness 200 when no deps are configured;
//! readiness 503 when a configured backend is unreachable.

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use ruscker_core::{ContainerBackend, CoreError, CoreResult, Replica, ReplicaId, ReplicaMetrics};
use std::sync::Arc;
use tower::ServiceExt;

const MINIMAL_YAML: &str = r#"
proxy:
  title: Test
  specs: []
"#;

fn base_state() -> AppState {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let config = Config::from_yaml(MINIMAL_YAML).expect("parse config");
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
        leader: Arc::new(ruscker_admin::leader::AlwaysLeader),
        metrics: ruscker_admin::metrics_cache::MetricsCache::new(),
        draining: Arc::new(std::sync::atomic::AtomicBool::new(false)),
    }
}

async fn get(state: AppState, uri: &str) -> (StatusCode, String) {
    let app = router(state);
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .unwrap();
    let resp = app.oneshot(req).await.unwrap();
    let status = resp.status();
    let body = axum::body::to_bytes(resp.into_body(), 64 * 1024)
        .await
        .unwrap();
    (status, String::from_utf8(body.to_vec()).unwrap())
}

/// A backend whose every `list()` call fails — stands in for an
/// unreachable Docker daemon so `/readyz` can be driven to 503.
struct UnreachableBackend;

#[async_trait]
impl ContainerBackend for UnreachableBackend {
    async fn spawn(&self, _spec_id: &str, _image: &str) -> CoreResult<Replica> {
        Err(CoreError::Backend("down".into()))
    }
    async fn stop(&self, _replica_id: &ReplicaId) -> CoreResult<()> {
        Err(CoreError::Backend("down".into()))
    }
    async fn list(&self) -> CoreResult<Vec<Replica>> {
        Err(CoreError::Backend("daemon unreachable".into()))
    }
    async fn metrics(&self, _replica_id: &ReplicaId) -> CoreResult<ReplicaMetrics> {
        Err(CoreError::Backend("down".into()))
    }
}

#[tokio::test]
async fn healthz_is_always_ok() {
    let (status, body) = get(base_state(), "/healthz").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"status\":\"ok\""), "body: {body}");
    assert!(body.contains("\"service\":\"ruscker\""), "body: {body}");
}

#[tokio::test]
async fn readyz_is_ready_with_no_dependencies() {
    // Landing-only mode (no --db, no --docker): nothing to probe,
    // so readiness is trivially satisfied.
    let (status, body) = get(base_state(), "/readyz").await;
    assert_eq!(status, StatusCode::OK);
    assert!(body.contains("\"status\":\"ready\""), "body: {body}");
}

#[tokio::test]
async fn readyz_503_when_draining() {
    // Once the draining flag is set, readiness must fail fast and
    // skip the dependency probes — even with healthy deps.
    let state = base_state();
    state
        .draining
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (status, body) = get(state, "/readyz").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains("\"status\":\"draining\""), "body: {body}");
}

#[tokio::test]
async fn readyz_503_when_backend_unreachable() {
    let mut state = base_state();
    state.backend = Some(Arc::new(UnreachableBackend));
    let (status, body) = get(state, "/readyz").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(body.contains("\"status\":\"not_ready\""), "body: {body}");
    assert!(body.contains("\"docker\":\"unreachable\""), "body: {body}");
    // The internal error string must not leak through the probe.
    assert!(
        !body.contains("daemon unreachable"),
        "body leaked error: {body}"
    );
}
