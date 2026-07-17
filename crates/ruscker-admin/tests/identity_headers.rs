//! Trusted identity-header forwarding for proxied HTTP requests (#1001).
//!
//! Each test puts a ready replica in the registry and points it at a tiny
//! local TCP upstream that captures the request head. This exercises the
//! real `/app` and `/api` forwarding path without Docker.

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use ruscker_admin::auth::{Role, COOKIE_NAME};
use ruscker_admin::db::ConfigDb;
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use ruscker_core::{
    ContainerBackend, CoreError, CoreResult, Replica, ReplicaId, ReplicaMetrics,
    ReplicaRegistry, ReplicaState,
};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tower::ServiceExt;

const CONFIG: &str = r#"
proxy:
  specs:
    - id: identity-on
      display-name: Identity on
      type: api
      container-image: test/api
      add-default-http-headers: true
    - id: identity-off
      display-name: Identity off
      type: api
      container-image: test/api
"#;

struct ReadyBackend;

#[async_trait]
impl ContainerBackend for ReadyBackend {
    async fn spawn(&self, _spec_id: &str, _image: &str) -> CoreResult<Replica> {
        Err(CoreError::Backend("unexpected spawn".into()))
    }

    async fn stop(&self, _replica_id: &ReplicaId) -> CoreResult<()> {
        Ok(())
    }

    async fn list(&self) -> CoreResult<Vec<Replica>> {
        Ok(Vec::new())
    }

    async fn metrics(&self, _replica_id: &ReplicaId) -> CoreResult<ReplicaMetrics> {
        Ok(ReplicaMetrics {
            cpu_percent: 0.0,
            memory_bytes: 0,
            network_rx_bytes: 0,
            network_tx_bytes: 0,
        })
    }
}

async fn open_db() -> ConfigDb {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ruscker-identity-headers-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    ConfigDb::Sqlite(ruscker_admin::db::open(&path).await.expect("open test DB"))
}

/// Accept one HTTP request, capture its headers, and answer `200 OK`.
async fn echo_upstream() -> (
    SocketAddr,
    tokio::task::JoinHandle<HashMap<String, String>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut raw = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let n = stream.read(&mut chunk).await.unwrap();
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&chunk[..n]);
            if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                break;
            }
        }

        let mut headers = HashMap::new();
        for line in String::from_utf8_lossy(&raw).split("\r\n").skip(1) {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
        headers
    });
    (addr, task)
}

async fn state(db: ConfigDb, spec_id: &str, upstream: SocketAddr) -> AppState {
    let config = Config::from_yaml(CONFIG).expect("parse test config");
    let mut replicas = ReplicaRegistry::new();
    replicas.add(Replica {
        id: ReplicaId(uuid::Uuid::new_v4()),
        spec_id: spec_id.to_string(),
        container_id: "identity-test".into(),
        upstream,
        state: ReplicaState::Ready,
        started_at: chrono::Utc::now(),
        sessions_active: 0,
        sessions_max: 100,
        host: None,
    });

    AppState {
        config: Arc::new(config),
        base_path: Arc::from(""),
        locales: Arc::new(ruscker_admin::i18n::Locales::load().expect("load locales")),
        admin_auth: ruscker_admin::auth::AdminAuth::with_token("test-token"),
        admin_sessions: Arc::new(
            ruscker_admin::auth::InMemoryAdminSessionStore::default(),
        ),
        log_buffer: None,
        login_limiter: Arc::new(ruscker_admin::auth::LoginRateLimiter::default_policy()),
        api_limiter: Arc::new(ruscker_admin::ratelimit::ApiRateLimiter::new()),
        db: Some(db),
        images_dir: None,
        master_key: Default::default(),
        backend: Some(Arc::new(ReadyBackend)),
        replicas: Arc::new(tokio::sync::RwLock::new(replicas)),
        cookie_key: ruscker_proxy::sticky::CookieKey::random(),
        spawn_locks: Arc::new(dashmap::DashMap::new()),
        sessions: Arc::new(ruscker_admin::sessions::InMemorySessionStore::new()),
        logout_index: Arc::new(dashmap::DashMap::new()),
        leader: Arc::new(ruscker_admin::leader::AlwaysLeader),
        metrics: ruscker_admin::metrics_cache::MetricsCache::new(),
        draining: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        spec_cache: Arc::new(dashmap::DashMap::new()),
        identity_cache: Default::default(),
        catalog_cache: Arc::new(tokio::sync::RwLock::new(None)),
        access_counter: Arc::new(ruscker_admin::access_counter::AccessCounter::default()),
        alerts: ruscker_admin::alerts::AlertSink::default(),
    }
}

async fn send(
    state: AppState,
    path: &str,
    cookie: Option<&str>,
    forged: bool,
) {
    let mut request = Request::builder().method("GET").uri(path);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    if forged {
        request = request
            .header("X-SP-UserId", "mallory")
            .header("X-SP-UserGroups", "attackers")
            .header("X-Ruscker-User-Email", "mallory@example.test");
    }
    let response = router(state)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    // The response body owns the upstream connection; consume it so every
    // test completes the full forwarding lifecycle before joining capture.
    let _ = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
}

async fn logged_in_state(
    spec_id: &str,
) -> (
    AppState,
    String,
    tokio::task::JoinHandle<HashMap<String, String>>,
) {
    let db = open_db().await;
    ruscker_admin::db::users::create(
        &db,
        "alice",
        "alicepass1",
        Role::Viewer,
        false,
        &["analysts".into(), "ops".into()],
        Some("admin"),
    )
    .await
    .unwrap();
    let (upstream, capture) = echo_upstream().await;
    let state = state(db, spec_id, upstream).await;
    let sid = state
        .admin_sessions
        .create(Role::Viewer, Some("alice".into()))
        .await;
    (state, format!("{COOKIE_NAME}={sid}"), capture)
}

#[tokio::test]
async fn enabled_app_gets_authoritative_user_and_db_groups() {
    let (state, cookie, capture) = logged_in_state("identity-on").await;
    send(state, "/app/identity-on/data", Some(&cookie), true).await;
    let headers = capture.await.unwrap();

    assert_eq!(
        headers.get("x-sp-userid").map(String::as_str),
        Some("alice"),
        "captured headers: {headers:?}"
    );
    assert_eq!(
        headers.get("x-sp-usergroups").map(String::as_str),
        Some("analysts,ops")
    );
    assert!(!headers.contains_key("x-ruscker-user-email"));
}

#[tokio::test]
async fn anonymous_api_request_cannot_forge_reserved_identity() {
    let db = open_db().await;
    let (upstream, capture) = echo_upstream().await;
    let state = state(db, "identity-on", upstream).await;
    send(state, "/api/identity-on/data", None, true).await;
    let headers = capture.await.unwrap();

    assert!(!headers.contains_key("x-sp-userid"));
    assert!(!headers.contains_key("x-sp-usergroups"));
    assert!(!headers.contains_key("x-ruscker-user-email"));
}

#[tokio::test]
async fn feature_off_sends_no_identity_for_logged_in_user() {
    let (state, cookie, capture) = logged_in_state("identity-off").await;
    send(state, "/api/identity-off/data", Some(&cookie), false).await;
    let headers = capture.await.unwrap();

    assert!(!headers.contains_key("x-sp-userid"));
    assert!(!headers.contains_key("x-sp-usergroups"));
}

#[tokio::test]
async fn break_glass_session_has_no_identity_headers() {
    let db = open_db().await;
    let (upstream, capture) = echo_upstream().await;
    let state = state(db, "identity-on", upstream).await;
    let sid = state.admin_sessions.create(Role::Admin, None).await;
    let cookie = format!("{COOKIE_NAME}={sid}");
    send(state, "/api/identity-on/data", Some(&cookie), false).await;
    let headers = capture.await.unwrap();

    assert!(!headers.contains_key("x-sp-userid"));
    assert!(!headers.contains_key("x-sp-usergroups"));
}
