//! Integration tests for `GET /oauth/authorize` (OIDC provider slice 3).
//!
//! The property under test is ORDER: nothing is redirected to a client
//! before the client and its redirect URI check out; no code is issued
//! before the user is known, passes the client's group gate and (when
//! required) has proved MFA in this session. Every refusal is asserted
//! together with "and no code was stored".

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use chrono::{Duration, Utc};
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::db::oidc_clients::ClientSettings;
use ruscker_admin::db::ConfigDb;
use ruscker_admin::i18n::Locales;
use ruscker_admin::mfa::DEVICE_COOKIE;
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::sync::Arc;
use tower::ServiceExt;

const ON: &str = "server:\n  oidc-provider:\n    enabled: true\n    issuer: https://portal.example\nproxy:\n  title: T\n  port: 8088\n  specs: []\n";
const ON_BOX: &str = "server:\n  oidc-provider:\n    enabled: true\n    issuer: https://portal.example/box\nproxy:\n  title: T\n  port: 8088\n  specs: []\n";
const OFF: &str = "proxy:\n  title: T\n  port: 8088\n  specs: []\n";
const KEY: &str = "00000000000000000000000000000000000000000000000000000000000000aa";
const PASSWORD: &str = "CorrectPass9!";
const CB: &str = "https://radar.example/cb";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

async fn open_db() -> ConfigDb {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ruscker-oidc-authz-{}-{}.db",
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

struct Reply {
    status: StatusCode,
    location: String,
    cache: String,
    body: String,
}

async fn get(state: &AppState, uri: &str, cookie: &str) -> Reply {
    let mut req = Request::builder().method("GET").uri(uri);
    if !cookie.is_empty() {
        req = req.header(header::COOKIE, cookie);
    }
    let resp = router(state.clone()).oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
    let h = |name: header::HeaderName| {
        resp.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
    };
    let (status, location, cache) = (resp.status(), h(header::LOCATION), h(header::CACHE_CONTROL));
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    Reply { status, location, cache, body: String::from_utf8_lossy(&bytes).into_owned() }
}

/// A well-formed authorization request for client `radar`, with `extra`
/// appended and any `key=` in `without` removed.
fn request(extra: &str, without: &[&str]) -> String {
    let base = [
        ("response_type", "code"),
        ("client_id", "radar"),
        ("redirect_uri", CB),
        ("scope", "openid email groups"),
        ("state", "st-123"),
        ("nonce", "n-456"),
        ("code_challenge", CHALLENGE),
        ("code_challenge_method", "S256"),
    ];
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in base {
        if !without.contains(&k) {
            q.append_pair(k, v);
        }
    }
    format!("/oauth/authorize?{}{extra}", q.finish())
}

/// Query parameters of a redirect back to the client.
fn params(location: &str) -> std::collections::HashMap<String, String> {
    let q = location.split_once('?').map(|(_, q)| q).unwrap_or("");
    url::form_urlencoded::parse(q.as_bytes()).into_owned().collect()
}

async fn stored_codes(db: &ConfigDb) -> i64 {
    let ConfigDb::Sqlite(pool) = db else { unreachable!() };
    let (n,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM oidc_auth_codes")
        .fetch_one(pool)
        .await
        .unwrap();
    n
}

/// Client `radar` (group `fiscal`), users `ana` (in it) and `bia` (not).
async fn seed(db: &ConfigDb, require_mfa: bool) {
    let settings = ClientSettings {
        name: "Radar".into(),
        redirect_uris: vec![CB.into(), "https://radar.example/cb2?tenant=1".into()],
        access_groups: vec!["fiscal".into()],
        require_mfa,
        enabled: true,
    };
    ruscker_admin::db::oidc_clients::create(db, "radar", &settings, None).await.unwrap().unwrap();
    for (name, groups) in [("ana", vec!["fiscal".to_string()]), ("bia", vec![])] {
        ruscker_admin::db::users::create(db, name, PASSWORD, Role::Viewer, false, &groups, None)
            .await
            .unwrap();
    }
}

async fn session(state: &AppState, role: Role, actor: Option<&str>) -> (String, String) {
    let id = state.admin_sessions.create(role, actor.map(str::to_string)).await;
    (id.clone(), format!("{COOKIE_NAME}={id}"))
}

async fn setup(yaml: &str, require_mfa: bool) -> (AppState, ConfigDb) {
    let db = open_db().await;
    seed(&db, require_mfa).await;
    (app_state(db.clone(), yaml, Some(KEY)).await, db)
}

#[tokio::test]
async fn the_endpoint_does_not_exist_when_the_provider_is_off() {
    let (state, db) = setup(OFF, false).await;
    let (_, ana) = session(&state, Role::Viewer, Some("ana")).await;
    let r = get(&state, &request("", &[]), &ana).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(stored_codes(&db).await, 0);
}

/// Until the client AND its redirect URI are verified, the browser is
/// sent nowhere: a 400 page, no `Location`, whoever is signed in.
#[tokio::test]
async fn nothing_is_redirected_before_client_and_redirect_uri_check_out() {
    let (state, db) = setup(ON, false).await;
    let (_, ana) = session(&state, Role::Viewer, Some("ana")).await;

    let unknown = request("", &["client_id"]) + "&client_id=nope";
    let evil = request("", &["redirect_uri"]) + "&redirect_uri=https%3A%2F%2Fevil.example%2Fcb";
    // Registered URI plus a suffix, and a prefix of it: exact match only.
    let suffix = request("", &["redirect_uri"]) + "&redirect_uri=https%3A%2F%2Fradar.example%2Fcb%2Fx";
    let prefix = request("", &["redirect_uri"]) + "&redirect_uri=https%3A%2F%2Fradar.example%2F";
    // Two redirect_uri values: the good one first, then the evil one.
    let twice = request("&redirect_uri=https%3A%2F%2Fevil.example%2Fcb", &[]);
    for uri in [
        unknown,
        evil,
        suffix,
        prefix,
        twice,
        request("", &["client_id"]),
        request("", &["redirect_uri"]),
    ] {
        for cookie in [ana.as_str(), ""] {
            let r = get(&state, &uri, cookie).await;
            assert_eq!(r.status, StatusCode::BAD_REQUEST, "{uri}");
            assert_eq!(r.location, "", "no redirect for {uri}");
            assert!(!r.body.contains("evil.example"), "the bad URI is not echoed");
        }
    }

    // A disabled client reads exactly like an unknown one.
    let mut off = ClientSettings {
        name: "Radar".into(),
        redirect_uris: vec![CB.into()],
        access_groups: vec!["fiscal".into()],
        require_mfa: false,
        enabled: false,
    };
    ruscker_admin::db::oidc_clients::update(&db, "radar", &off, None).await.unwrap();
    let r = get(&state, &request("", &[]), &ana).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.location, "");
    assert_eq!(stored_codes(&db).await, 0);
    off.enabled = true;
    ruscker_admin::db::oidc_clients::update(&db, "radar", &off, None).await.unwrap();
}

/// Malformed-but-attributable requests go back to the client as an
/// OAuth error, with its state, and never with a code.
#[tokio::test]
async fn protocol_errors_return_to_the_client_without_a_code() {
    let (state, db) = setup(ON, false).await;
    let (_, ana) = session(&state, Role::Viewer, Some("ana")).await;
    let cases = [
        (request("", &["response_type"]) + "&response_type=token", "unsupported_response_type"),
        (request("", &["response_type"]) + "&response_type=id_token", "unsupported_response_type"),
        (request("", &["response_type"]), "unsupported_response_type"),
        (request("", &["scope"]) + "&scope=email", "invalid_scope"),
        (request("", &["scope"]), "invalid_scope"),
        // PKCE: missing, `plain`, method missing, malformed challenge.
        (request("", &["code_challenge"]), "invalid_request"),
        (request("", &["code_challenge_method"]) + "&code_challenge_method=plain", "invalid_request"),
        (request("", &["code_challenge_method"]), "invalid_request"),
        (request("", &["code_challenge"]) + "&code_challenge=short", "invalid_request"),
        (request("&request=eyJhbGciOiJub25lIn0..", &[]), "request_not_supported"),
        (request("&request_uri=https%3A%2F%2Fevil.example%2Fr", &[]), "request_not_supported"),
    ];
    for (uri, error) in cases {
        let r = get(&state, &uri, &ana).await;
        assert_eq!(r.status, StatusCode::FOUND, "{uri}");
        assert!(r.location.starts_with(&format!("{CB}?")), "{uri} → {}", r.location);
        let p = params(&r.location);
        assert_eq!(p.get("error").map(String::as_str), Some(error), "{uri}");
        assert_eq!(p.get("state").map(String::as_str), Some("st-123"));
        assert!(!p.contains_key("code"));
    }
    assert_eq!(stored_codes(&db).await, 0);
}

#[tokio::test]
async fn an_anonymous_browser_is_sent_to_login_and_comes_back() {
    let (state, db) = setup(ON, false).await;
    let uri = request("", &[]);
    let r = get(&state, &uri, "").await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let (path, query) = r.location.split_once('?').unwrap();
    assert_eq!(path, "/admin/login");
    let next = params(&format!("x?{query}")).remove("next").unwrap();
    assert_eq!(next, uri, "the login returns to this exact request");

    // The login page keeps that destination (it is on its allowlist)…
    let login = get(&state, &r.location, "").await;
    assert_eq!(login.status, StatusCode::OK);
    assert!(login.body.contains("/oauth/authorize?"), "next survives into the login form");

    // …and a client that asked for no UI gets an error instead.
    let silent = get(&state, &request("&prompt=none", &[]), "").await;
    assert_eq!(silent.status, StatusCode::FOUND);
    assert_eq!(params(&silent.location).get("error").map(String::as_str), Some("login_required"));
    assert_eq!(stored_codes(&db).await, 0);
}

#[tokio::test]
async fn a_member_gets_a_single_use_code_bound_to_the_request() {
    let (state, db) = setup(ON, false).await;
    let (_, ana) = session(&state, Role::Viewer, Some("ana")).await;
    let r = get(&state, &request("", &[]), &ana).await;
    assert_eq!(r.status, StatusCode::FOUND);
    assert_eq!(r.cache, "no-store");
    assert!(r.location.starts_with(&format!("{CB}?")));
    let p = params(&r.location);
    assert_eq!(p.get("state").map(String::as_str), Some("st-123"));
    assert_eq!(p.get("iss").map(String::as_str), Some("https://portal.example"));
    assert!(!p.contains_key("error"));
    let code = p.get("code").expect("a code").clone();

    // What the code is bound to.
    let user = ruscker_admin::db::users::fetch(&db, "ana").await.unwrap().unwrap();
    let redeemed = ruscker_admin::db::oidc_codes::consume(&db, &code).await.unwrap().expect("valid");
    let g = redeemed.grant;
    assert_eq!(g.client_id, "radar");
    assert_eq!(g.user_id, user.id);
    assert_eq!(g.username, "ana");
    assert_eq!(g.redirect_uri, CB);
    assert_eq!(g.code_challenge, CHALLENGE);
    assert_eq!(g.nonce.as_deref(), Some("n-456"));
    assert_eq!(g.scope, "openid email groups");
    // Single use.
    assert!(ruscker_admin::db::oidc_codes::consume(&db, &code).await.unwrap().is_none());

    // Unknown scopes are dropped, not granted; a registered URI that
    // already has a query gets `&`, not a second `?`.
    let uri = request("", &["scope", "redirect_uri"])
        + "&scope=openid+admin+offline_access&redirect_uri=https%3A%2F%2Fradar.example%2Fcb2%3Ftenant%3D1";
    let r = get(&state, &uri, &ana).await;
    assert_eq!(r.status, StatusCode::FOUND);
    assert!(r.location.starts_with("https://radar.example/cb2?tenant=1&code="), "{}", r.location);
    let code = params(&r.location).remove("code").unwrap();
    let g = ruscker_admin::db::oidc_codes::consume(&db, &code).await.unwrap().unwrap().grant;
    assert_eq!(g.scope, "openid");
    assert_eq!(g.redirect_uri, "https://radar.example/cb2?tenant=1");
}

#[tokio::test]
async fn the_group_gate_is_read_live_and_has_no_admin_bypass() {
    let (state, db) = setup(ON, false).await;
    let denied = |r: &Reply| {
        assert_eq!(r.status, StatusCode::FOUND);
        assert!(r.location.starts_with(&format!("{CB}?")));
        let p = params(&r.location);
        assert_eq!(p.get("error").map(String::as_str), Some("access_denied"));
        assert!(!p.contains_key("code"));
    };

    // Not in the client's group.
    let (_, bia) = session(&state, Role::Viewer, Some("bia")).await;
    denied(&get(&state, &request("", &[]), &bia).await);

    // An Admin ACCOUNT outside the group is refused like anyone else.
    ruscker_admin::db::users::create(&db, "root", PASSWORD, Role::Admin, false, &[], None)
        .await
        .unwrap();
    let (_, root) = session(&state, Role::Admin, Some("root")).await;
    denied(&get(&state, &request("", &[]), &root).await);

    // A break-glass token session has no identity to give.
    let (_, token) = session(&state, Role::Admin, None).await;
    denied(&get(&state, &request("", &[]), &token).await);

    // A session whose account was deleted.
    let (_, ghost) = session(&state, Role::Viewer, Some("ghost")).await;
    denied(&get(&state, &request("", &[]), &ghost).await);
    assert_eq!(stored_codes(&db).await, 0);

    // Membership is read per request: the SAME session works once the
    // user joins the group, and stops working when they leave.
    ruscker_admin::db::users::set_groups(&db, "bia", &["fiscal".to_string()], None).await.unwrap();
    let r = get(&state, &request("", &[]), &bia).await;
    assert!(params(&r.location).contains_key("code"), "{}", r.location);
    ruscker_admin::db::users::set_groups(&db, "bia", &[], None).await.unwrap();
    denied(&get(&state, &request("", &[]), &bia).await);
}

#[tokio::test]
async fn an_admin_assigned_password_must_be_changed_first() {
    let (state, db) = setup(ON, false).await;
    ruscker_admin::db::users::create(&db, "novo", PASSWORD, Role::Viewer, true, &["fiscal".to_string()], None)
        .await
        .unwrap();
    let (_, novo) = session(&state, Role::Viewer, Some("novo")).await;
    let uri = request("", &[]);
    let r = get(&state, &uri, &novo).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let (path, query) = r.location.split_once('?').unwrap();
    assert_eq!(path, "/admin/account/password");
    assert_eq!(params(&format!("x?{query}")).remove("next").unwrap(), uri);

    let silent = get(&state, &request("&prompt=none", &[]), &novo).await;
    assert_eq!(
        params(&silent.location).get("error").map(String::as_str),
        Some("interaction_required")
    );
    assert_eq!(stored_codes(&db).await, 0);
}

/// A client that requires MFA: no factor ⇒ enrolment; factor but no
/// proof in this session ⇒ challenge; proof given in ANOTHER session ⇒
/// still a challenge; proof in this session ⇒ code.
#[tokio::test]
async fn mfa_must_be_proved_in_this_session() {
    let (state, db) = setup(ON, true).await;
    let (sid, ana) = session(&state, Role::Viewer, Some("ana")).await;
    let uri = request("", &[]);
    let detour = |r: &Reply, page: &str| {
        assert_eq!(r.status, StatusCode::SEE_OTHER, "{}", r.location);
        let (path, query) = r.location.split_once('?').unwrap();
        assert_eq!(path, page);
        assert_eq!(params(&format!("x?{query}")).remove("next").unwrap(), uri);
    };

    detour(&get(&state, &uri, &ana).await, "/admin/account/mfa");

    enroll(&state, &db, "ana").await;
    detour(&get(&state, &uri, &ana).await, "/admin/account/mfa/challenge");
    let silent = get(&state, &request("&prompt=none", &[]), &ana).await;
    assert_eq!(
        params(&silent.location).get("error").map(String::as_str),
        Some("interaction_required")
    );

    // Proof bound to a different login session of the same user.
    let (other_sid, other) = session(&state, Role::Viewer, Some("ana")).await;
    let (gid, token) = issue_grant(&db, "ana", &other_sid).await;
    let stolen = format!("{ana}; {DEVICE_COOKIE}={gid}.{token}");
    detour(&get(&state, &uri, &stolen).await, "/admin/account/mfa/challenge");
    assert_eq!(stored_codes(&db).await, 0);

    // In the session that proved it, it works…
    let own = format!("{other}; {DEVICE_COOKIE}={gid}.{token}");
    let r = get(&state, &uri, &own).await;
    assert_eq!(r.status, StatusCode::FOUND);
    assert!(params(&r.location).contains_key("code"), "{}", r.location);

    // …and so does a fresh proof in the first one.
    let (gid, token) = issue_grant(&db, "ana", &sid).await;
    let r = get(&state, &uri, &format!("{ana}; {DEVICE_COOKIE}={gid}.{token}")).await;
    assert!(params(&r.location).contains_key("code"), "{}", r.location);
}

/// Mounted under a base path: detours to our own pages carry the prefix,
/// the redirect to the client's absolute URL is left untouched.
#[tokio::test]
async fn under_a_base_path_only_local_redirects_get_the_prefix() {
    let db = open_db().await;
    seed(&db, false).await;
    let mut state = app_state(db.clone(), ON_BOX, Some(KEY)).await;
    state.base_path = Arc::from("/box");
    let uri = format!("/box{}", request("", &[]));

    let r = get(&state, &uri, "").await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert!(r.location.starts_with("/box/admin/login?next="), "{}", r.location);
    assert!(!r.location.starts_with("/box/box"), "prefixed once");

    let (_, ana) = session(&state, Role::Viewer, Some("ana")).await;
    let r = get(&state, &uri, &ana).await;
    assert_eq!(r.status, StatusCode::FOUND);
    assert!(r.location.starts_with(&format!("{CB}?code=")), "{}", r.location);
    assert_eq!(
        params(&r.location).get("iss").map(String::as_str),
        Some("https://portal.example/box")
    );
}

/// Oversized values are refused rather than stored or echoed.
#[tokio::test]
async fn oversized_state_and_nonce_are_refused() {
    let (state, db) = setup(ON, false).await;
    let (_, ana) = session(&state, Role::Viewer, Some("ana")).await;
    let big = "x".repeat(2000);
    let r = get(&state, &(request("", &["state"]) + &format!("&state={big}")), &ana).await;
    assert_eq!(r.status, StatusCode::FOUND);
    assert!(!r.location.contains(&big), "the oversized state is not echoed");
    assert_eq!(params(&r.location).get("error").map(String::as_str), Some("invalid_request"));
    let r = get(&state, &(request("", &["nonce"]) + &format!("&nonce={big}")), &ana).await;
    assert_eq!(params(&r.location).get("error").map(String::as_str), Some("invalid_request"));
    assert_eq!(stored_codes(&db).await, 0);
}
