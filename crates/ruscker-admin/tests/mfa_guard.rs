//! Per-app MFA enforcement on the real proxy router (#1005 slice 4).
//!
//! Successful cases use a ready replica backed by a loopback HTTP echo;
//! blocked cases prove the request returns before any upstream/backend work.

use async_trait::async_trait;
use axum::body::{to_bytes, Body};
use axum::http::{header, HeaderMap, Request, StatusCode};
use chrono::{Duration, Utc};
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::db::ConfigDb;
use ruscker_admin::mfa::DEVICE_COOKIE;
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use ruscker_core::{
    ContainerBackend, CoreResult, Replica, ReplicaId, ReplicaMetrics, ReplicaRegistry,
    ReplicaState,
};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tower::ServiceExt;

const CONFIG: &str = r#"
proxy:
  specs:
    - id: protected-app
      display-name: Protected app
      container-image: test/app
      require-mfa: true
      inject-base-href: false
    - id: session-app
      display-name: Session-bound app
      container-image: test/app
      require-mfa: true
      mfa-validity-days: 0
      inject-base-href: false
    - id: protected-api
      display-name: Protected API
      type: api
      container-image: test/api
      require-mfa: true
      api:
        cors: true
    - id: open-app
      display-name: Open app
      container-image: test/app
      inject-base-href: false
"#;

const PASSWORD: &str = "CorrectPass9!";

#[derive(Default)]
struct RecordingBackend {
    spawned: AtomicBool,
}

#[async_trait]
impl ContainerBackend for RecordingBackend {
    async fn spawn(&self, _spec_id: &str, _image: &str) -> CoreResult<Replica> {
        self.spawned.store(true, Ordering::SeqCst);
        panic!("MFA-blocked request attempted to spawn a replica")
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
    let path = std::env::temp_dir().join(format!(
        "ruscker-mfa-guard-{}.db",
        uuid::Uuid::new_v4().simple()
    ));
    ConfigDb::Sqlite(ruscker_admin::db::open(&path).await.expect("open test DB"))
}

async fn echo_upstream(expected_requests: usize) -> (SocketAddr, tokio::task::JoinHandle<usize>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        for _ in 0..expected_requests {
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
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok",
                )
                .await
                .unwrap();
        }
        expected_requests
    });
    (addr, task)
}

fn state(
    db: Option<ConfigDb>,
    spec_id: Option<&str>,
    upstream: SocketAddr,
    backend: Arc<RecordingBackend>,
    base_path: &str,
) -> AppState {
    let config = Config::from_yaml(CONFIG).expect("parse MFA guard config");
    let mut replicas = ReplicaRegistry::new();
    if let Some(spec_id) = spec_id {
        replicas.add(Replica {
            id: ReplicaId(uuid::Uuid::new_v4()),
            spec_id: spec_id.to_string(),
            container_id: "mfa-guard-test".into(),
            upstream,
            state: ReplicaState::Ready,
            started_at: Utc::now(),
            sessions_active: 0,
            sessions_max: if spec_id.ends_with("api") { 100 } else { 1 },
            host: None,
        });
    }
    AppState {
        config: Arc::new(config),
        base_path: Arc::from(base_path),
        locales: Arc::new(ruscker_admin::i18n::Locales::load().expect("load locales")),
        admin_auth: AdminAuth::with_token("break-glass-token"),
        admin_sessions: Arc::new(ruscker_admin::auth::InMemoryAdminSessionStore::default()),
        log_buffer: None,
        login_limiter: Arc::new(ruscker_admin::auth::LoginRateLimiter::default_policy()),
        api_limiter: Arc::new(ruscker_admin::ratelimit::ApiRateLimiter::new()),
        db,
        images_dir: None,
        master_key: ruscker_admin::crypto::MasterKey::parse(&"ab".repeat(32)).unwrap(),
        backend: Some(backend),
        replicas: Arc::new(tokio::sync::RwLock::new(replicas)),
        cookie_key: ruscker_proxy::sticky::CookieKey::random(),
        spawn_locks: Arc::new(dashmap::DashMap::new()),
        sessions: Arc::new(ruscker_admin::sessions::InMemorySessionStore::new()),
        logout_index: Arc::new(dashmap::DashMap::new()),
        leader: Arc::new(ruscker_admin::leader::AlwaysLeader),
        metrics: ruscker_admin::metrics_cache::MetricsCache::new(),
        draining: Arc::new(AtomicBool::new(false)),
        spec_cache: Arc::new(dashmap::DashMap::new()),
        identity_cache: Default::default(),
        catalog_cache: Arc::new(tokio::sync::RwLock::new(None)),
        access_counter: Arc::new(ruscker_admin::access_counter::AccessCounter::default()),
        alerts: ruscker_admin::alerts::AlertSink::default(),
    }
}

async fn create_user(db: &ConfigDb, username: &str) {
    ruscker_admin::db::users::create(
        db,
        username,
        PASSWORD,
        Role::Viewer,
        false,
        &[],
        Some("test"),
    )
    .await
    .unwrap();
}

async fn create_session(state: &AppState, role: Role, actor: Option<&str>) -> (String, String) {
    let id = state
        .admin_sessions
        .create(role, actor.map(str::to_string))
        .await;
    (id.clone(), format!("{COOKIE_NAME}={id}"))
}

async fn enroll(state: &AppState, db: &ConfigDb, username: &str) {
    let enrollment = ruscker_admin::mfa::begin(username).unwrap();
    let (secret_enc, nonce) = state
        .master_key
        .encrypt(enrollment.secret_base32.as_bytes())
        .unwrap();
    let ceremony = uuid::Uuid::new_v4().to_string();
    ruscker_admin::db::mfa::begin_enrollment(db, username, &secret_enc, &nonce, &ceremony)
        .await
        .unwrap();
    ruscker_admin::db::mfa::confirm_enrollment(db, username, username, &ceremony)
        .await
        .unwrap();
}

async fn issue_grant(
    db: &ConfigDb,
    username: &str,
    session_id: &str,
) -> (String, String) {
    let factor = ruscker_admin::db::mfa::fetch(db, username)
        .await
        .unwrap()
        .unwrap();
    let token = ruscker_admin::mfa::generate_device_token().unwrap();
    let token_hash = ruscker_admin::mfa::hash_device_token(&token).unwrap();
    let verified_at = Utc::now();
    let id = ruscker_admin::db::mfa_grants::issue(
        db,
        username,
        &token_hash,
        &ruscker_admin::mfa::session_binding(session_id),
        factor.confirmed_at.unwrap(),
        verified_at,
        verified_at + Duration::days(i64::from(ruscker_config::MAX_MFA_VALIDITY_DAYS)),
        factor.security_epoch,
        None,
        None,
        "mfa.verify",
        username,
    )
    .await
    .unwrap()
    .expect("issue grant");
    (id, token)
}

fn cookie_jar(session_cookie: &str, grant: Option<&(String, String)>) -> String {
    match grant {
        Some((id, token)) => format!("{session_cookie}; {DEVICE_COOKIE}={id}.{token}"),
        None => session_cookie.to_string(),
    }
}

async fn send(
    state: AppState,
    uri: &str,
    cookie: Option<&str>,
) -> (StatusCode, HeaderMap, String) {
    let mut request = Request::builder().method("GET").uri(uri);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    let response = router(state)
        .oneshot(request.body(Body::empty()).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = to_bytes(response.into_body(), 1 << 20).await.unwrap();
    (status, headers, String::from_utf8(body.to_vec()).unwrap())
}

fn location(headers: &HeaderMap) -> Option<&str> {
    headers.get(header::LOCATION).and_then(|value| value.to_str().ok())
}

#[tokio::test]
async fn protected_app_guides_login_enrollment_and_challenge() {
    let db = open_db().await;
    let backend = Arc::new(RecordingBackend::default());
    let state = state(
        Some(db.clone()),
        Some("protected-app"),
        SocketAddr::from(([127, 0, 0, 1], 9)),
        backend.clone(),
        "/box",
    );

    let (status, headers, _) = send(
        state.clone(),
        "/box/app/protected-app/report%20one?tab=a%2Fb&sort=name",
        None,
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        location(&headers),
        Some("/box/admin/login?next=%2Fapp%2Fprotected-app%2Freport%2520one%3Ftab%3Da%252Fb%26sort%3Dname")
    );

    create_user(&db, "alice").await;
    let (_, session_cookie) = create_session(&state, Role::Viewer, Some("alice")).await;
    let (status, headers, _) = send(
        state.clone(),
        "/box/app/protected-app/data?view=1",
        Some(&session_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        location(&headers),
        Some("/box/admin/account/mfa?next=%2Fapp%2Fprotected-app%2Fdata%3Fview%3D1")
    );

    enroll(&state, &db, "alice").await;
    let (status, headers, _) = send(
        state.clone(),
        "/box/app/protected-app/data?view=1",
        Some(&session_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(
        location(&headers),
        Some("/box/admin/account/mfa/challenge?next=%2Fapp%2Fprotected-app%2Fdata%3Fview%3D1")
    );
    assert!(!backend.spawned.load(Ordering::SeqCst));
}

#[tokio::test]
async fn protected_app_with_valid_grant_reaches_upstream() {
    let db = open_db().await;
    let (upstream, reached) = echo_upstream(1).await;
    let backend = Arc::new(RecordingBackend::default());
    let state = state(
        Some(db.clone()),
        Some("protected-app"),
        upstream,
        backend.clone(),
        "",
    );
    create_user(&db, "app-user").await;
    let (session_id, session_cookie) =
        create_session(&state, Role::Viewer, Some("app-user")).await;
    enroll(&state, &db, "app-user").await;

    let grant = issue_grant(&db, "app-user", &session_id).await;
    let browser = cookie_jar(&session_cookie, Some(&grant));
    let (status, _, body) = send(
        state,
        "/app/protected-app/data?view=1",
        Some(&browser),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "ok");
    assert_eq!(reached.await.unwrap(), 1);
    assert!(!backend.spawned.load(Ordering::SeqCst));
}

#[tokio::test]
async fn protected_api_fails_closed_without_html_redirects() {
    let db = open_db().await;
    let state = state(
        Some(db.clone()),
        Some("protected-api"),
        SocketAddr::from(([127, 0, 0, 1], 9)),
        Arc::new(RecordingBackend::default()),
        "",
    );

    let (status, headers, body) = send(state.clone(), "/api/protected-api/data", None).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(location(&headers).is_none());
    assert!(body.contains("authenticated user session"));
    assert_eq!(headers.get("access-control-allow-origin").unwrap(), "*");

    create_user(&db, "bob").await;
    let (_, session_cookie) = create_session(&state, Role::Viewer, Some("bob")).await;
    enroll(&state, &db, "bob").await;
    let (status, headers, body) = send(
        state.clone(),
        "/api/protected-api/data",
        Some(&session_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(location(&headers).is_none());
    assert!(body.contains("complete it in the web portal"));
    assert_eq!(headers.get("access-control-allow-origin").unwrap(), "*");
}

#[tokio::test]
async fn protected_api_with_valid_grant_reaches_upstream() {
    let db = open_db().await;
    let (upstream, reached) = echo_upstream(1).await;
    let state = state(
        Some(db.clone()),
        Some("protected-api"),
        upstream,
        Arc::new(RecordingBackend::default()),
        "",
    );
    create_user(&db, "api-user").await;
    let (session_id, session_cookie) =
        create_session(&state, Role::Viewer, Some("api-user")).await;
    enroll(&state, &db, "api-user").await;

    let grant = issue_grant(&db, "api-user", &session_id).await;
    let browser = cookie_jar(&session_cookie, Some(&grant));
    let (status, _, body) = send(
        state,
        "/api/protected-api/data",
        Some(&browser),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "ok");
    assert_eq!(reached.await.unwrap(), 1);
}

#[tokio::test]
async fn zero_day_grant_is_bound_to_the_login_session_that_proved_mfa() {
    let db = open_db().await;
    let state = state(
        Some(db.clone()),
        None,
        SocketAddr::from(([127, 0, 0, 1], 9)),
        Arc::new(RecordingBackend::default()),
        "",
    );
    create_user(&db, "carol").await;
    enroll(&state, &db, "carol").await;
    let (bound_id, bound_cookie) = create_session(&state, Role::Viewer, Some("carol")).await;
    let (_, other_cookie) = create_session(&state, Role::Viewer, Some("carol")).await;
    let grant = issue_grant(&db, "carol", &bound_id).await;

    let wrong_browser = cookie_jar(&other_cookie, Some(&grant));
    let (status, headers, _) = send(
        state.clone(),
        "/app/session-app/",
        Some(&wrong_browser),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(location(&headers).unwrap().starts_with("/admin/account/mfa/challenge?next="));

    let (upstream, reached) = echo_upstream(1).await;
    state.replicas.write().await.add(Replica {
        id: ReplicaId(uuid::Uuid::new_v4()),
        spec_id: "session-app".into(),
        container_id: "mfa-session-test".into(),
        upstream,
        state: ReplicaState::Ready,
        started_at: Utc::now(),
        sessions_active: 0,
        sessions_max: 1,
        host: None,
    });
    let right_browser = cookie_jar(&bound_cookie, Some(&grant));
    let (status, _, body) = send(
        state,
        "/app/session-app/",
        Some(&right_browser),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "ok");
    assert_eq!(reached.await.unwrap(), 1);
}

#[tokio::test]
async fn blocked_request_never_enters_the_spawn_coalescer() {
    let db = open_db().await;
    let backend = Arc::new(RecordingBackend::default());
    let state = state(
        Some(db.clone()),
        None,
        SocketAddr::from(([127, 0, 0, 1], 9)),
        backend.clone(),
        "",
    );
    create_user(&db, "dora").await;
    let (_, session_cookie) = create_session(&state, Role::Viewer, Some("dora")).await;

    // No Accept:text/html: absent the MFA guard this request would block on
    // pick_or_spawn instead of taking the cold-start splash path.
    let (status, headers, _) = send(
        state,
        "/app/protected-app/data",
        Some(&session_cookie),
    )
    .await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert!(location(&headers).unwrap().starts_with("/admin/account/mfa?next="));
    assert!(!backend.spawned.load(Ordering::SeqCst));
}

#[tokio::test]
async fn break_glass_bypasses_and_audits_once_per_session_and_spec() {
    let db = open_db().await;
    let (upstream, reached) = echo_upstream(2).await;
    let state = state(
        Some(db.clone()),
        Some("protected-api"),
        upstream,
        Arc::new(RecordingBackend::default()),
        "",
    );
    let (_, token_cookie) = create_session(&state, Role::Admin, None).await;

    for _ in 0..2 {
        let (status, _, body) = send(
            state.clone(),
            "/api/protected-api/data",
            Some(&token_cookie),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, "ok");
    }
    assert_eq!(reached.await.unwrap(), 2);

    let ConfigDb::Sqlite(pool) = &db else {
        unreachable!()
    };
    let (count,): (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM audit_log
          WHERE action = 'mfa.break_glass_bypass'
            AND actor = 'token'
            AND target = 'spec:protected-api'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn unprotected_spec_still_forwards_without_a_database_or_mfa_state() {
    let (upstream, reached) = echo_upstream(1).await;
    let backend = Arc::new(RecordingBackend::default());
    let state = state(None, Some("open-app"), upstream, backend.clone(), "");
    let (status, headers, body) = send(state, "/app/open-app/data", None).await;
    assert_eq!(status, StatusCode::OK);
    assert!(location(&headers).is_none());
    assert_eq!(body, "ok");
    assert_eq!(reached.await.unwrap(), 1);
    assert!(!backend.spawned.load(Ordering::SeqCst));
}
