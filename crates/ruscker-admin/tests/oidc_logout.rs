//! Integration tests for `/oauth/logout` (OpenID Connect RP-Initiated
//! Logout): a client system's "sign out everywhere".
//!
//! The properties: a bare link never ends a session (the person is asked,
//! and the answer is a same-origin POST); the browser is only ever sent
//! to a return address REGISTERED for the client the request names; a
//! bad request is refused, never redirected.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::db::oidc_clients::ClientSettings;
use ruscker_admin::db::ConfigDb;
use ruscker_admin::i18n::Locales;
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use serde_json::json;
use std::sync::Arc;
use tower::ServiceExt;

const ON: &str = "server:\n  oidc-provider:\n    enabled: true\n    issuer: https://portal.example\nproxy:\n  title: T\n  port: 8088\n  specs: []\n";
const KEY: &str = "00000000000000000000000000000000000000000000000000000000000000aa";
const BYE: &str = "https://radar.example/bye";

async fn open_db() -> ConfigDb {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ruscker-oidc-logout-{}-{}.db",
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

struct Reply {
    status: StatusCode,
    location: String,
    body: String,
}

async fn send(state: &AppState, req: Request<Body>) -> Reply {
    let resp = router(state.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let location = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let body = String::from_utf8(axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap().to_vec()).unwrap();
    Reply { status, location, body }
}

fn get(uri: &str, cookie: Option<&str>, site: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().uri(uri);
    if let Some(c) = cookie {
        b = b.header(header::COOKIE, c);
    }
    if let Some(s) = site {
        b = b.header("sec-fetch-site", s);
    }
    b.body(Body::empty()).unwrap()
}

fn post(form: &str, cookie: &str, site: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/oauth/logout")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::COOKIE, cookie)
        .header("sec-fetch-site", site)
        .body(Body::from(form.to_string()))
        .unwrap()
}

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect()
}

/// A provider with two clients (`radar` with a return address, `norma`
/// without) and one signed-in person. Returns the session cookie.
async fn setup() -> (AppState, ConfigDb, String) {
    let db = open_db().await;
    let state = app_state(db.clone(), ON, Some(KEY)).await;
    let client = |logout: &[&str]| ClientSettings {
        name: "Radar".into(),
        redirect_uris: vec!["https://radar.example/cb".into()],
        access_groups: vec![],
        require_mfa: false,
        enabled: true,
        logo: None,
        allowed_scopes: None,
        client_uri: Some("https://radar.example/".into()),
        post_logout_redirect_uris: logout.iter().map(|s| s.to_string()).collect(),
    };
    ruscker_admin::db::oidc_clients::create(&db, "radar", &client(&[BYE]), None).await.unwrap();
    ruscker_admin::db::oidc_clients::create(&db, "norma", &client(&[]), None).await.unwrap();
    // The account behind the session must exist (sessions are confirmed
    // against its authentication generation).
    ruscker_admin::db::users::create(&db, "ana", "Ana-Pass9!", Role::Viewer, false, &[], None)
        .await
        .unwrap();
    let sid = state.admin_sessions.create(Role::Viewer, Some("ana".into())).await;
    (state, db, format!("{COOKIE_NAME}={sid}"))
}

/// An ID token as the provider signs them (`typ: JWT`), already expired —
/// a hint may be.
async fn id_token(state: &AppState, db: &ConfigDb, iss: &str, aud: &str) -> String {
    let key = ruscker_admin::oidc::ensure_signing_key(db, &state.master_key).await.unwrap();
    ruscker_admin::oidc::sign_jwt(&key, "JWT", &json!({"iss": iss, "aud": aud, "sub": "x", "iat": 1, "exp": 2})).unwrap()
}

async fn signed_in(state: &AppState, cookie: &str) -> bool {
    state.admin_sessions.validate(cookie.split('=').nth(1).unwrap()).await.is_some()
}

#[tokio::test]
async fn discovery_announces_the_endpoint() {
    let (state, _, _) = setup().await;
    let r = send(&state, get("/.well-known/openid-configuration", None, None)).await;
    assert!(r.body.contains(r#""end_session_endpoint":"https://portal.example/oauth/logout""#), "{}", r.body);
}

/// The whole round trip: asked, confirmed, signed out, sent back with `state`.
#[tokio::test]
async fn signed_in_person_is_asked_then_signed_out_and_sent_back() {
    let (state, db, cookie) = setup().await;
    let hint = id_token(&state, &db, "https://portal.example", "radar").await;
    let q = format!("id_token_hint={hint}&post_logout_redirect_uri={}&state=xyz", enc(BYE));

    // A link alone ends nothing: the page asks.
    let resp = router(state.clone())
        .oneshot(get(&format!("/oauth/logout?{q}"), Some(&cookie), Some("same-origin")))
        .await
        .unwrap();
    // Browsers apply `form-action` to the redirect after the post: the
    // page must allow the return address's origin, and only that.
    let csp = resp.headers().get(header::CONTENT_SECURITY_POLICY).unwrap().to_str().unwrap().to_string();
    assert!(csp.ends_with("form-action 'self' https://radar.example"), "{csp}");
    assert_eq!(csp.matches("radar.example").count(), 1, "nothing else widened: {csp}");
    let r = send(&state, get(&format!("/oauth/logout?{q}"), Some(&cookie), Some("same-origin"))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains(r#"action="/oauth/logout""#));
    assert!(r.body.contains(r#"name="client_id" value="radar""#));
    assert!(r.body.contains("Radar"), "the registered name");
    assert!(!r.body.contains(&hint), "the hint is not echoed into the page");
    assert!(signed_in(&state, &cookie).await, "still signed in after the GET");

    // A cross-site POST is refused by the CSRF guard.
    let form = format!("client_id=radar&post_logout_redirect_uri={}&state=xyz", enc(BYE));
    let r = send(&state, post(&form, &cookie, "cross-site")).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(signed_in(&state, &cookie).await);

    // Confirmed from our own page: signed out, back to the system with state.
    let r = send(&state, post(&form, &cookie, "same-origin")).await;
    assert_eq!(r.status, StatusCode::FOUND);
    assert_eq!(r.location, "https://radar.example/bye?state=xyz");
    assert!(!signed_in(&state, &cookie).await, "the portal session is gone");
}

#[tokio::test]
async fn without_a_session_the_browser_goes_straight_back() {
    let (state, _, _) = setup().await;
    let q = format!("client_id=radar&post_logout_redirect_uri={}&state=s1", enc(BYE));
    let r = send(&state, get(&format!("/oauth/logout?{q}"), None, Some("same-origin"))).await;
    assert_eq!(r.status, StatusCode::FOUND);
    assert_eq!(r.location, "https://radar.example/bye?state=s1");

    // Arriving from the system's site, the Strict cookie was not sent:
    // first a bounce through our own origin, to see a session if any.
    let r = send(&state, get(&format!("/oauth/logout?{q}"), None, Some("cross-site"))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains(r#"http-equiv="refresh""#) && r.body.contains("/oauth/logout?client_id=radar"));
    assert!(r.body.contains("bounced=1"));
    // A browser without Fetch Metadata also withholds the Strict cookie:
    // bounced too, and only once.
    let r = send(&state, get(&format!("/oauth/logout?{q}"), None, None)).await;
    assert!(r.body.contains(r#"http-equiv="refresh""#), "no header: bounce");
    let r = send(&state, get(&format!("/oauth/logout?{q}&bounced=1"), None, None)).await;
    assert_eq!(r.status, StatusCode::FOUND, "after the bounce: back to the system");
    assert_eq!(r.location, "https://radar.example/bye?state=s1");

    // No return address asked for: a page that says so, linking home.
    let r = send(&state, get("/oauth/logout?client_id=radar", None, Some("same-origin"))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains(r#"href="https://radar.example/""#));
}

/// Never a redirect to an address the client did not register.
#[tokio::test]
async fn bad_requests_are_refused_not_redirected() {
    let (state, db, cookie) = setup().await;
    let foreign = id_token(&state, &db, "https://other.example", "radar").await;
    let for_norma = id_token(&state, &db, "https://portal.example", "norma").await;
    let access = {
        let key = ruscker_admin::oidc::ensure_signing_key(&db, &state.master_key).await.unwrap();
        ruscker_admin::oidc::sign_jwt(&key, "at+jwt", &json!({"iss": "https://portal.example", "aud": "radar", "sub": "x", "exp": 2})).unwrap()
    };
    let cases = [
        // Not registered for the client.
        format!("client_id=radar&post_logout_redirect_uri={}", enc("https://evil.example/")),
        // A client with no return addresses at all.
        format!("client_id=norma&post_logout_redirect_uri={}", enc(BYE)),
        // A return address without any client.
        format!("post_logout_redirect_uri={}", enc(BYE)),
        // A hint issued by someone else, a forged one, or one that disagrees with client_id.
        format!("id_token_hint={foreign}"),
        "id_token_hint=e30.e30.AAAA".to_string(),
        format!("id_token_hint={for_norma}&client_id=radar"),
        // Unknown client.
        "client_id=ghost".to_string(),
        // An access token is not an ID token.
        format!("id_token_hint={access}"),
        // A state the client could not get back intact.
        format!("client_id=radar&post_logout_redirect_uri={}&state={}", enc(BYE), "s".repeat(513)),
    ];
    for q in &cases {
        for cookie in [None, Some(cookie.as_str())] {
            let r = send(&state, get(&format!("/oauth/logout?{q}"), cookie, Some("same-origin"))).await;
            assert_eq!(r.status, StatusCode::BAD_REQUEST, "{q}");
            assert!(r.location.is_empty(), "{q}");
        }
    }
    // The POST validates again: a forged form cannot pick the address.
    let form = format!("client_id=radar&post_logout_redirect_uri={}", enc("https://evil.example/"));
    let r = send(&state, post(&form, &cookie, "same-origin")).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert!(signed_in(&state, &cookie).await, "nothing ended on a refused request");

    // A disabled client is unknown here too.
    let mut off = ruscker_admin::db::oidc_clients::fetch(&db, "radar").await.unwrap().unwrap();
    off.enabled = false;
    let s = ClientSettings {
        name: off.name,
        redirect_uris: off.redirect_uris,
        access_groups: off.access_groups,
        require_mfa: off.require_mfa,
        enabled: false,
        logo: off.logo,
        allowed_scopes: off.allowed_scopes,
        client_uri: off.client_uri,
        post_logout_redirect_uris: off.post_logout_redirect_uris,
    };
    ruscker_admin::db::oidc_clients::update(&db, "radar", &s, None).await.unwrap();
    let q = format!("client_id=radar&post_logout_redirect_uri={}", enc(BYE));
    let r = send(&state, get(&format!("/oauth/logout?{q}"), None, Some("same-origin"))).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
}

/// With nothing named, a signed-in person is still asked, and confirming
/// signs them out to a page that says so.
#[tokio::test]
async fn bare_request_asks_and_ends_on_a_page() {
    let (state, _, cookie) = setup().await;
    let r = send(&state, get("/oauth/logout", Some(&cookie), Some("same-origin"))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains(r#"action="/oauth/logout""#));
    let r = send(&state, post("", &cookie, "same-origin")).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(!r.body.contains(r#"action="/oauth/logout""#));
    assert!(!signed_in(&state, &cookie).await);
}

/// The widened `form-action` exists only where a post will be followed by
/// a redirect to a registered address.
#[tokio::test]
async fn csp_is_widened_only_on_a_confirmation_with_a_return_address() {
    let (state, _, cookie) = setup().await;
    let csp_of = |req: Request<Body>| {
        let state = state.clone();
        async move {
            let resp = router(state).oneshot(req).await.unwrap();
            resp.headers().get(header::CONTENT_SECURITY_POLICY).unwrap().to_str().unwrap().to_string()
        }
    };
    // Confirmation without a return address.
    let csp = csp_of(get("/oauth/logout?client_id=radar", Some(&cookie), Some("same-origin"))).await;
    assert!(csp.ends_with("form-action 'self'"), "{csp}");
    // The "signed out" page.
    let csp = csp_of(post("client_id=radar", &cookie, "same-origin")).await;
    assert!(csp.ends_with("form-action 'self'"), "{csp}");
}

#[tokio::test]
async fn no_master_key_means_unavailable() {
    let db = open_db().await;
    let state = app_state(db, ON, None).await;
    let r = send(&state, get("/oauth/logout", None, None)).await;
    assert_eq!(r.status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn provider_off_means_no_endpoint() {
    let db = open_db().await;
    let state = app_state(db, "proxy:\n  title: T\n  port: 8088\n  specs: []\n", Some(KEY)).await;
    let r = send(&state, get("/oauth/logout", None, None)).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
}

/// Codex review of #1123 (third pass): when the store cannot record the
/// sign-out, the confirmation does not send the person back as if it were
/// done — it says so on the portal's page.
#[tokio::test]
async fn an_unrecorded_sign_out_is_reported_not_redirected() {
    use ruscker_admin::auth::{AdminSessionStore, InMemoryAdminSessionStore, SessionInfo};
    /// Signs out in memory but reports that nothing was recorded.
    #[derive(Debug, Default)]
    struct Unrecorded(InMemoryAdminSessionStore);
    #[async_trait::async_trait]
    impl AdminSessionStore for Unrecorded {
        async fn create_at_epoch(&self, role: Role, actor: Option<String>, epoch: i64, id: Option<String>) -> String {
            self.0.create_at_epoch(role, actor, epoch, id).await
        }
        async fn validate(&self, id: &str) -> Option<SessionInfo> {
            self.0.validate(id).await
        }
        async fn remove(&self, id: &str) -> bool {
            self.0.remove(id).await;
            false
        }
        async fn revoke_by_actor(&self, actor: &str) {
            self.0.revoke_by_actor(actor).await
        }
    }
    let (mut state, _, _) = setup().await;
    let store = std::sync::Arc::new(Unrecorded::default());
    let sid = store.0.create(Role::Viewer, Some("ana".into())).await;
    state.admin_sessions = store;
    let form = format!("client_id=radar&post_logout_redirect_uri={}&state=xyz", enc(BYE));
    let r = send(&state, post(&form, &format!("{COOKIE_NAME}={sid}"), "same-origin")).await;
    assert_eq!(r.status, StatusCode::OK, "no redirect to the system");
    assert!(r.location.is_empty());
    assert!(r.body.contains(r#"role="alert""#), "the page says it was not confirmed");
}
