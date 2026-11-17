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
    - id: claims-both
      display-name: Both additional claims
      type: api
      container-image: test/api
      identity-claims: [email, setor]
    - id: claims-email
      display-name: Email claim only
      type: api
      container-image: test/api
      identity-claims: [email]
    - id: identity-all
      display-name: Default and additional identity
      type: api
      container-image: test/api
      add-default-http-headers: true
      identity-claims: [email, setor]
    - id: roles-open
      display-name: Group roles, open app
      type: api
      container-image: test/api
      identity-claims: [roles]
    - id: roles-restricted
      display-name: Group roles, restricted app
      type: api
      container-image: test/api
      access-groups: [ops]
      identity-claims: [roles]
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
        activity: ruscker_admin::activity::ActivitySink::default(),
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
            .header("X-Ruscker-User-Email", "mallory@example.test")
            .header("X-Ruscker-User-Setor", "Attackers")
            .header("X-Ruscker-User-Roles", "ops:admin")
            // Underscore spellings: a different header on the wire, the
            // same variable inside a Shiny/WSGI/Rack app.
            .header("X_SP_UserId", "mallory")
            .header("X_SP_UserGroups", "attackers")
            .header("X_Ruscker_User_Email", "mallory@example.test")
            .header("X-Ruscker_User_Roles", "ops:admin");
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
    ruscker_admin::db::users::update_profile(
        &db,
        "alice",
        Some("Gestão"),
        Some("alice@example.test"),
        Some("+55 81 99999-9999"),
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
    // A signed-in user cannot ride a forged underscore header past the
    // authoritative one either (it would win inside the app).
    assert_no_underscore_identity(&headers);
    assert_eq!(
        headers.get("x-sp-usergroups").map(String::as_str),
        Some("analysts,ops")
    );
    assert!(!headers.contains_key("x-ruscker-user-email"));
}

/// Group names are free-form UTF-8 (`gestão` is a perfectly normal
/// pt-BR group) — the header value must go out as raw UTF-8 bytes, not
/// be silently dropped by an ASCII-only conversion (codex review, #1001).
#[tokio::test]
async fn accented_group_names_reach_the_upstream() {
    let db = open_db().await;
    ruscker_admin::db::users::create(
        &db,
        "joana",
        "joanapass1",
        Role::Viewer,
        false,
        &["gestão".into()],
        Some("admin"),
    )
    .await
    .unwrap();
    let (upstream, capture) = echo_upstream().await;
    let state = state(db, "identity-on", upstream).await;
    let sid = state
        .admin_sessions
        .create(Role::Viewer, Some("joana".into()))
        .await;
    let cookie = format!("{COOKIE_NAME}={sid}");
    send(state, "/api/identity-on/data", Some(&cookie), false).await;
    let headers = capture.await.unwrap();

    assert_eq!(
        headers.get("x-sp-userid").map(String::as_str),
        Some("joana")
    );
    assert_eq!(
        headers.get("x-sp-usergroups").map(String::as_str),
        Some("gestão"),
        "captured headers: {headers:?}"
    );
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
    assert_no_underscore_identity(&headers);
}

/// No spelling that an app server would fold onto an identity variable
/// (`_` for `-`) may reach the upstream.
fn assert_no_underscore_identity<V>(headers: &std::collections::HashMap<String, V>) {
    for name in headers.keys() {
        let folded = name.replace('_', "-");
        assert!(
            name == &folded || !(folded.starts_with("x-sp-") || folded.starts_with("x-ruscker-user-")),
            "forged identity header reached the app: {name}"
        );
    }
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
async fn declared_email_and_setor_are_forwarded_without_x_sp_headers() {
    let (state, cookie, capture) = logged_in_state("claims-both").await;
    send(state, "/app/claims-both/data", Some(&cookie), false).await;
    let headers = capture.await.unwrap();

    assert_eq!(
        headers.get("x-ruscker-user-email").map(String::as_str),
        Some("alice@example.test")
    );
    assert_eq!(
        headers.get("x-ruscker-user-setor").map(String::as_str),
        Some("Gestão")
    );
    assert!(!headers.contains_key("x-sp-userid"));
    assert!(!headers.contains_key("x-sp-usergroups"));
    assert!(!headers.contains_key("x-ruscker-user-celular"));
}

#[tokio::test]
async fn email_only_spec_does_not_receive_setor() {
    let (state, cookie, capture) = logged_in_state("claims-email").await;
    send(state, "/api/claims-email/data", Some(&cookie), false).await;
    let headers = capture.await.unwrap();

    assert_eq!(
        headers.get("x-ruscker-user-email").map(String::as_str),
        Some("alice@example.test")
    );
    assert!(!headers.contains_key("x-ruscker-user-setor"));
}

#[tokio::test]
async fn missing_declared_email_is_omitted_instead_of_sent_empty() {
    let db = open_db().await;
    ruscker_admin::db::users::create(
        &db,
        "bruno",
        "brunopass1",
        Role::Viewer,
        false,
        &[],
        Some("admin"),
    )
    .await
    .unwrap();
    ruscker_admin::db::users::update_profile(
        &db,
        "bruno",
        Some("Financeiro"),
        None,
        None,
        Some("admin"),
    )
    .await
    .unwrap();
    let (upstream, capture) = echo_upstream().await;
    let state = state(db, "claims-email", upstream).await;
    let sid = state
        .admin_sessions
        .create(Role::Viewer, Some("bruno".into()))
        .await;
    let cookie = format!("{COOKIE_NAME}={sid}");

    send(state, "/api/claims-email/data", Some(&cookie), false).await;
    let headers = capture.await.unwrap();
    assert!(!headers.contains_key("x-ruscker-user-email"));
    assert!(!headers.contains_key("x-ruscker-user-setor"));
}

#[tokio::test]
async fn anonymous_request_cannot_forge_declared_claims() {
    let db = open_db().await;
    let (upstream, capture) = echo_upstream().await;
    let state = state(db, "claims-both", upstream).await;
    send(state, "/api/claims-both/data", None, true).await;
    let headers = capture.await.unwrap();

    assert!(!headers.contains_key("x-ruscker-user-email"));
    assert!(!headers.contains_key("x-ruscker-user-setor"));
    assert!(!headers.contains_key("x-sp-userid"));
}

#[tokio::test]
async fn break_glass_session_has_no_identity_headers() {
    let db = open_db().await;
    let (upstream, capture) = echo_upstream().await;
    let state = state(db, "identity-all", upstream).await;
    let sid = state.admin_sessions.create(Role::Admin, None).await;
    let cookie = format!("{COOKIE_NAME}={sid}");
    send(state, "/api/identity-all/data", Some(&cookie), false).await;
    let headers = capture.await.unwrap();

    assert!(!headers.contains_key("x-sp-userid"));
    assert!(!headers.contains_key("x-sp-usergroups"));
    assert!(!headers.contains_key("x-ruscker-user-email"));
    assert!(!headers.contains_key("x-ruscker-user-setor"));
}


/// The trusted-device MFA grant cookie is root-scoped (#1005 slice 4 needs
/// it on /app/*), so browsers send it on proxied requests — it is an MFA
/// BEARER and must be stripped before the container, like every Ruscker
/// cookie (#258; codex review of slice 3).
#[tokio::test]
async fn mfa_device_cookie_never_reaches_the_upstream() {
    let db = open_db().await;
    let (upstream, capture) = echo_upstream().await;
    let state = state(db, "identity-off", upstream).await;
    let request = Request::builder()
        .method("GET")
        .uri("/api/identity-off/data")
        .header(
            header::COOKIE,
            "app_own=keep; __ruscker_mfa_device=g1.deadbeef; __ruscker_mfa_ceremony=c1",
        )
        .body(Body::empty())
        .unwrap();
    let response = router(state).oneshot(request).await.unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
    let headers = capture.await.unwrap();
    let cookie = headers.get("cookie").cloned().unwrap_or_default();
    assert!(cookie.contains("app_own=keep"), "app cookies must pass: {cookie}");
    assert!(
        !cookie.contains("__ruscker_mfa"),
        "MFA cookies must never reach a container: {cookie}"
    );
}

/// An upstream that answers with several `Set-Cookie` headers, so we can
/// prove the proxy strips the ones whose names Ruscker owns (#1010) — a
/// same-origin app must not be able to overwrite the user's session,
/// sticky, or MFA device cookie — while the app's own cookie passes.
async fn set_cookie_upstream() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        // Accumulate across reads: the CRLFCRLF terminator can split across
        // TCP segments (codex review), which a per-chunk check would miss.
        let mut raw = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let n = stream.read(&mut chunk).await.unwrap();
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&chunk[..n]);
            if raw.windows(4).any(|w| w == b"\r\n\r\n") {
                break;
            }
        }
        stream
            .write_all(
                b"HTTP/1.1 200 OK\r\n\
                  Set-Cookie: app_pref=dark; Path=/\r\n\
                  Set-Cookie: __ruscker_mfa_device=EVIL; Path=/\r\n\
                  Set-Cookie: ruscker_admin_session=EVIL; Path=/\r\n\
                  Set-Cookie: __ruscker_session_identity-off=EVIL; Path=/\r\n\
                  Content-Length: 2\r\nConnection: close\r\n\r\nok",
            )
            .await
            .unwrap();
    });
    (addr, task)
}

#[tokio::test]
async fn reserved_set_cookies_from_the_app_never_reach_the_client() {
    let db = open_db().await;
    let (upstream, task) = set_cookie_upstream().await;
    let state = state(db, "identity-off", upstream).await;
    let response = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri("/api/identity-off/data")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let set_cookies: Vec<String> = response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .map(|v| v.to_str().unwrap().to_string())
        .collect();
    let _ = axum::body::to_bytes(response.into_body(), 1024).await.unwrap();
    task.await.unwrap();

    // The app's own cookie survives…
    assert!(
        set_cookies.iter().any(|c| c.starts_with("app_pref=dark")),
        "app cookie must pass through: {set_cookies:?}"
    );
    // …but every reserved Ruscker cookie the app tried to set is gone.
    assert!(
        !set_cookies.iter().any(|c| {
            let name = c.split(';').next().unwrap_or("").split('=').next().unwrap_or("");
            name.starts_with("ruscker_") || name.starts_with("__ruscker_")
        }),
        "reserved cookies must be stripped: {set_cookies:?}"
    );
}

// ── Group roles (#1067): `identity-claims: [roles]` → X-Ruscker-User-Roles ──

/// Give `alice` (groups `analysts`, `ops`) one role in each group.
async fn alice_with_roles(state: &AppState) {
    let db = state.db.as_ref().unwrap();
    use ruscker_admin::db::group_roles;
    group_roles::define(db, "analysts", "leitor", Some("admin")).await.unwrap();
    group_roles::define(db, "ops", "plantao", Some("admin")).await.unwrap();
    group_roles::assign(db, "alice", "analysts", "leitor", None).await.unwrap();
    group_roles::assign(db, "alice", "ops", "plantao", None).await.unwrap();
}

#[tokio::test]
async fn group_roles_need_the_opt_in() {
    let (state, cookie, capture) = logged_in_state("identity-all").await;
    alice_with_roles(&state).await;
    send(state, "/api/identity-all/data", Some(&cookie), false).await;
    let headers = capture.await.unwrap();
    assert!(headers.contains_key("x-ruscker-user-email"), "{headers:?}");
    assert!(!headers.contains_key("x-ruscker-user-roles"), "{headers:?}");
}

#[tokio::test]
async fn open_app_receives_every_group_role() {
    let (state, cookie, capture) = logged_in_state("roles-open").await;
    alice_with_roles(&state).await;
    send(state, "/api/roles-open/data", Some(&cookie), true).await;
    let headers = capture.await.unwrap();
    assert_eq!(
        headers.get("x-ruscker-user-roles").map(String::as_str),
        Some("analysts:leitor,ops:plantao"),
        "the forged inbound value must be replaced, not appended: {headers:?}"
    );
    // The claim is independent of the X-SP headers and of the account role.
    assert!(!headers.contains_key("x-sp-userid"));
}

#[tokio::test]
async fn restricted_app_receives_only_roles_of_its_access_groups() {
    let (state, cookie, capture) = logged_in_state("roles-restricted").await;
    alice_with_roles(&state).await;
    send(state, "/api/roles-restricted/data", Some(&cookie), false).await;
    let headers = capture.await.unwrap();
    assert_eq!(
        headers.get("x-ruscker-user-roles").map(String::as_str),
        Some("ops:plantao"),
        "`analysts` is not in access-groups: {headers:?}"
    );
}

#[tokio::test]
async fn header_is_omitted_when_the_user_holds_no_role() {
    let (state, cookie, capture) = logged_in_state("roles-open").await;
    // A spoofed inbound header must not survive the absence of real roles.
    send(state, "/api/roles-open/data", Some(&cookie), true).await;
    let headers = capture.await.unwrap();
    assert!(!headers.contains_key("x-ruscker-user-roles"), "{headers:?}");
}

#[tokio::test]
async fn anonymous_request_cannot_forge_group_roles() {
    let db = open_db().await;
    let (upstream, capture) = echo_upstream().await;
    let state = state(db, "roles-open", upstream).await;
    send(state, "/api/roles-open/data", None, true).await;
    let headers = capture.await.unwrap();
    assert!(!headers.contains_key("x-ruscker-user-roles"), "{headers:?}");
}

/// The worked example of the encoding, end to end: the group name carries
/// `:`, spaces and an accent, so it is percent-encoded (UTF-8) and every
/// entry still splits on its single `:`.
#[tokio::test]
async fn group_names_are_percent_encoded_in_the_roles_header() {
    let db = open_db().await;
    let group = "Fazenda: núcleo A".to_string();
    ruscker_admin::db::users::create(&db, "joana", "joanapass1", Role::Viewer, false, std::slice::from_ref(&group), None)
        .await
        .unwrap();
    use ruscker_admin::db::group_roles;
    for role in ["gestor", "analista"] {
        group_roles::define(&db, &group, role, None).await.unwrap();
        group_roles::assign(&db, "joana", &group, role, None).await.unwrap();
    }
    let (upstream, capture) = echo_upstream().await;
    let state = state(db, "roles-open", upstream).await;
    let sid = state
        .admin_sessions
        .create(Role::Viewer, Some("joana".into()))
        .await;
    send(state, "/api/roles-open/data", Some(&format!("{COOKIE_NAME}={sid}")), false).await;
    let headers = capture.await.unwrap();
    assert_eq!(
        headers.get("x-ruscker-user-roles").map(String::as_str),
        Some("Fazenda%3A%20n%C3%BAcleo%20A:analista,Fazenda%3A%20n%C3%BAcleo%20A:gestor")
    );
}

/// Like [`echo_upstream`], but serves `n` sequential connections and
/// returns every captured header map in order.
async fn echo_upstream_n(
    n: usize,
) -> (
    SocketAddr,
    tokio::task::JoinHandle<Vec<HashMap<String, String>>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let task = tokio::spawn(async move {
        let mut all = Vec::new();
        for _ in 0..n {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut raw = Vec::new();
            let mut chunk = [0_u8; 4096];
            loop {
                let read = stream.read(&mut chunk).await.unwrap();
                if read == 0 {
                    break;
                }
                raw.extend_from_slice(&chunk[..read]);
                if raw.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let mut headers = HashMap::new();
            for line in String::from_utf8_lossy(&raw).split("\r\n").skip(1) {
                if let Some((name, value)) = line.split_once(':') {
                    headers.insert(name.to_ascii_lowercase(), value.trim().to_string());
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .await
                .unwrap();
            all.push(headers);
        }
        all
    });
    (addr, task)
}

/// The proxy caches identity per user for 30 s. Assigning a role on the
/// Groups page must reach the very next proxied request, not the next TTL.
#[tokio::test]
async fn assigning_a_role_reaches_the_next_request() {
    let db = open_db().await;
    ruscker_admin::db::users::create(
        &db,
        "alice",
        "alicepass1",
        Role::Viewer,
        false,
        &["analysts".into(), "ops".into()],
        None,
    )
    .await
    .unwrap();
    use ruscker_admin::db::group_roles;
    group_roles::define(&db, "analysts", "leitor", None).await.unwrap();
    group_roles::define(&db, "ops", "plantao", None).await.unwrap();
    group_roles::assign(&db, "alice", "analysts", "leitor", None).await.unwrap();

    let (upstream, capture) = echo_upstream_n(2).await;
    let state = state(db, "roles-open", upstream).await;
    let alice = state.admin_sessions.create(Role::Viewer, Some("alice".into())).await;
    let alice = format!("{COOKIE_NAME}={alice}");
    let admin = state.admin_sessions.create(Role::Admin, Some("root".into())).await;
    let admin = format!("{COOKIE_NAME}={admin}");

    // First request fills the identity cache.
    send(state.clone(), "/api/roles-open/data", Some(&alice), false).await;

    let response = router(state.clone())
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/admin/groups/roles/assign")
                .header(header::COOKIE, &admin)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from("group=ops&username=alice&role=plantao"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::SEE_OTHER);

    send(state, "/api/roles-open/data", Some(&alice), false).await;
    let seen = capture.await.unwrap();
    assert_eq!(
        seen[0].get("x-ruscker-user-roles").map(String::as_str),
        Some("analysts:leitor")
    );
    assert_eq!(
        seen[1].get("x-ruscker-user-roles").map(String::as_str),
        Some("analysts:leitor,ops:plantao"),
        "the assignment must invalidate the cached identity"
    );
}
