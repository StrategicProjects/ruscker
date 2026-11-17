//! Integration tests for the OIDC provider's public metadata endpoints
//! (slice 2): they do not exist unless the provider is enabled with a
//! usable issuer, the discovery document is built from the CONFIGURED
//! issuer (never the request's Host), and the JWKS publishes a key that
//! verifies what the provider signs. Harness from `schedules_ui.rs`.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use ruscker_admin::auth::AdminAuth;
use ruscker_admin::db::ConfigDb;
use ruscker_admin::i18n::Locales;
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::sync::Arc;
use tower::ServiceExt;

const OFF: &str = "proxy:\n  title: T\n  port: 8088\n  specs: []\n";
const ON: &str = "server:\n  oidc-provider:\n    enabled: true\n    issuer: https://portal.example/\nproxy:\n  title: T\n  port: 8088\n  specs: []\n";
const BROKEN: &str = "server:\n  oidc-provider:\n    enabled: true\n    issuer: http://portal.example\nproxy:\n  title: T\n  port: 8088\n  specs: []\n";
/// Same provider, mounted under `/box`.
const ON_BOX: &str = "server:\n  oidc-provider:\n    enabled: true\n    issuer: https://portal.example/box/\nproxy:\n  title: T\n  port: 8088\n  specs: []\n";
const KEY: &str = "00000000000000000000000000000000000000000000000000000000000000aa";

async fn open_db() -> ConfigDb {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ruscker-oidc-meta-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    ConfigDb::Sqlite(ruscker_admin::db::open(&path).await.unwrap())
}

async fn app_state(db: ConfigDb, yaml: &str, master_key: Option<&str>) -> AppState {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let config = Config::from_yaml(yaml).expect("parse config");
    let locales = Locales::load().expect("load locales");
    AppState {
        config: Arc::new(config),
        base_path: Arc::from(""),
        locales: Arc::new(locales),
        admin_auth: AdminAuth::with_token("test-token"),
        admin_sessions: Arc::new(ruscker_admin::auth::InMemoryAdminSessionStore::default()),
        log_buffer: None,
        login_limiter: Arc::new(ruscker_admin::auth::LoginRateLimiter::default_policy()),
        api_limiter: Arc::new(ruscker_admin::ratelimit::ApiRateLimiter::new()),
        db: Some(db),
        images_dir: None,
        master_key: match master_key {
            Some(k) => ruscker_admin::crypto::MasterKey::parse(k).expect("master key"),
            None => Default::default(),
        },
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
        identity_cache: Default::default(),
        catalog_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        access_counter: Arc::new(ruscker_admin::access_counter::AccessCounter::default()),
        alerts: ruscker_admin::alerts::AlertSink::default(),
        activity: ruscker_admin::activity::ActivitySink::default(),
    }
}

/// GET with a hostile Host header; returns (status, Cache-Control, body).
async fn get(state: AppState, path: &str) -> (StatusCode, String, String) {
    let resp = router(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .header(header::HOST, "evil.example")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let cache = resp
        .headers()
        .get(header::CACHE_CONTROL)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, cache, String::from_utf8_lossy(&bytes).into_owned())
}

const DISCOVERY: &str = "/.well-known/openid-configuration";
const JWKS: &str = "/oauth/jwks";

#[tokio::test]
async fn endpoints_do_not_exist_unless_enabled_with_a_usable_issuer() {
    for yaml in [OFF, BROKEN] {
        let state = app_state(open_db().await, yaml, Some(KEY)).await;
        let db = state.db.clone().unwrap();
        for path in [DISCOVERY, JWKS] {
            let (status, _, _) = get(state.clone(), path).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        }
        // And asking never created a key behind the operator's back.
        assert!(ruscker_admin::db::oidc_keys::list_newest_first(&db).await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn discovery_uses_the_configured_issuer_not_the_host_header() {
    let state = app_state(open_db().await, ON, Some(KEY)).await;
    let (status, cache, body) = get(state, DISCOVERY).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cache, "public, max-age=300");
    let d: serde_json::Value = serde_json::from_str(&body).expect("json");
    // Trailing slash normalized away.
    assert_eq!(d["issuer"], "https://portal.example");
    assert_eq!(d["jwks_uri"], "https://portal.example/oauth/jwks");
    assert!(!body.contains("evil.example"), "the Host header must not leak into the document");
}

#[tokio::test]
async fn jwks_publishes_the_key_the_provider_signs_with() {
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine;
    use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};

    let state = app_state(open_db().await, ON, Some(KEY)).await;
    let db = state.db.clone().unwrap();

    // First read creates the key; the second returns the same one.
    let (status, cache, body) = get(state.clone(), JWKS).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cache, "public, max-age=300");
    let (_, _, again) = get(state.clone(), JWKS).await;
    assert_eq!(body, again, "the key set is stable across requests");

    let set: serde_json::Value = serde_json::from_str(&body).expect("json");
    let keys = set["keys"].as_array().expect("keys");
    assert_eq!(keys.len(), 1);
    let jwk = &keys[0];
    assert_eq!(jwk["kty"], "EC");
    assert_eq!(jwk["crv"], "P-256");
    assert!(jwk.get("d").is_none(), "no private member in a public document");

    // What the provider signs verifies with what the endpoint published.
    let signer = ruscker_admin::oidc::ensure_signing_key(&db, &state.master_key).await.unwrap();
    assert_eq!(jwk["kid"], signer.kid.as_str());
    let sig = signer.sign(b"header.payload").unwrap();
    let mut point = vec![0x04];
    point.extend(URL_SAFE_NO_PAD.decode(jwk["x"].as_str().unwrap()).unwrap());
    point.extend(URL_SAFE_NO_PAD.decode(jwk["y"].as_str().unwrap()).unwrap());
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point)
        .verify(b"header.payload", &sig)
        .expect("verifies");
}

/// Under a base path the documents live at `{base}/…`, which is exactly
/// `{issuer}/…` — and an issuer whose path is NOT the mount point turns
/// the provider off instead of advertising URLs that do not exist.
#[tokio::test]
async fn issuer_path_must_be_the_mount_point() {
    // Mounted at /box with a matching issuer: served under /box only.
    let mut state = app_state(open_db().await, ON_BOX, Some(KEY)).await;
    state.base_path = Arc::from("/box");
    let (status, _, body) = get(state.clone(), "/box/.well-known/openid-configuration").await;
    assert_eq!(status, StatusCode::OK);
    let d: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(d["issuer"], "https://portal.example/box");
    assert_eq!(d["jwks_uri"], "https://portal.example/box/oauth/jwks");
    let (status, _, _) = get(state.clone(), "/box/oauth/jwks").await;
    assert_eq!(status, StatusCode::OK, "the advertised jwks_uri path exists");
    let (status, _, _) = get(state, DISCOVERY).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "not at the root");

    // Mismatches, both directions: off.
    let mut root_issuer_on_box = app_state(open_db().await, ON, Some(KEY)).await;
    root_issuer_on_box.base_path = Arc::from("/box");
    let box_issuer_on_root = app_state(open_db().await, ON_BOX, Some(KEY)).await;
    for (state, prefix) in [(root_issuer_on_box, "/box"), (box_issuer_on_root, "")] {
        let db = state.db.clone().unwrap();
        for path in [DISCOVERY, JWKS] {
            let (status, _, _) = get(state.clone(), &format!("{prefix}{path}")).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{prefix}{path}");
        }
        assert!(ruscker_admin::db::oidc_keys::list_newest_first(&db).await.unwrap().is_empty());
    }
}

/// Once a key exists, the JWKS is served from the public column alone:
/// it keeps answering even if the private half cannot be decrypted, and
/// repeated anonymous requests never add keys.
#[tokio::test]
async fn jwks_reads_never_write_or_decrypt_once_a_key_exists() {
    let db = open_db().await;
    let state = app_state(db.clone(), ON, Some(KEY)).await;
    let (_, _, first) = get(state, JWKS).await;

    // Same database, a DIFFERENT master key: the private key is now
    // unreadable, the public set is not.
    let other = "00000000000000000000000000000000000000000000000000000000000000bb";
    let state = app_state(db.clone(), ON, Some(other)).await;
    for _ in 0..3 {
        let (status, _, body) = get(state.clone(), JWKS).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, first);
    }
    assert_eq!(ruscker_admin::db::oidc_keys::list_newest_first(&db).await.unwrap().len(), 1);
}

#[tokio::test]
async fn jwks_is_unavailable_without_a_master_key() {
    let state = app_state(open_db().await, ON, None).await;
    let db = state.db.clone().unwrap();
    let (status, _, body) = get(state.clone(), JWKS).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!body.contains("keys"), "no empty key set is served");
    assert!(ruscker_admin::db::oidc_keys::list_newest_first(&db).await.unwrap().is_empty());
    // Discovery too: it would only advertise a jwks_uri that cannot work.
    let (status, _, body) = get(state, DISCOVERY).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert!(!body.contains("jwks_uri"));
}
