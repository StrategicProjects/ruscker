//! Integration tests for `POST /oauth/token` and `/oauth/userinfo`
//! (OIDC provider slice 4) — the full sign-in, end to end: authorize
//! with a real session, redeem the code as the client, verify the ID
//! token against the published JWKS, read userinfo.
//!
//! Each refusal is asserted together with what it must NOT do: issue a
//! token, or (for a failed client authentication) burn the code.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine;
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::db::oidc_clients::ClientSettings;
use ruscker_admin::db::ConfigDb;
use ruscker_admin::i18n::Locales;
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use serde_json::{json, Value};
use std::sync::Arc;
use tower::ServiceExt;

const ON: &str = "server:\n  useForwardHeaders: true\n  oidc-provider:\n    enabled: true\n    issuer: https://portal.example\nproxy:\n  title: T\n  port: 8088\n  specs: []\n";
const OFF: &str = "proxy:\n  title: T\n  port: 8088\n  specs: []\n";
const ISSUER: &str = "https://portal.example";
const KEY: &str = "00000000000000000000000000000000000000000000000000000000000000aa";
const PASSWORD: &str = "CorrectPass9!";
const CB: &str = "https://radar.example/cb";
/// RFC 7636 Appendix B.
const VERIFIER: &str = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
const CHALLENGE: &str = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";

async fn open_db() -> ConfigDb {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ruscker-oidc-token-{}-{}.db",
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

struct Fixture {
    state: AppState,
    db: ConfigDb,
    /// Secret of client `radar` (groups: fiscal).
    radar: String,
    /// Secret of client `norma` (open to any signed-in user).
    norma: String,
}

async fn fixture(yaml: &str) -> Fixture {
    let db = open_db().await;
    let client = |groups: &[&str], cb: &str| ClientSettings {
        name: "C".into(),
        redirect_uris: vec![cb.into()],
        access_groups: groups.iter().map(|g| g.to_string()).collect(),
        require_mfa: false,
        enabled: true,
    };
    let radar = ruscker_admin::db::oidc_clients::create(&db, "radar", &client(&["fiscal"], CB), None)
        .await
        .unwrap()
        .unwrap();
    let norma = ruscker_admin::db::oidc_clients::create(&db, "norma", &client(&[], CB), None)
        .await
        .unwrap()
        .unwrap();
    let groups = vec!["fiscal".to_string(), "outro".to_string()];
    ruscker_admin::db::users::create(&db, "ana", PASSWORD, Role::Viewer, false, &groups, None)
        .await
        .unwrap();
    ruscker_admin::db::users::update_profile(&db, "ana", None, Some("ana@example.org"), None, None)
        .await
        .unwrap();
    let state = app_state(db.clone(), yaml, Some(KEY)).await;
    Fixture { state, db, radar, norma }
}

/// Run the browser half as `ana`: returns a fresh authorization code
/// for `client_id` with the given scope.
async fn authorize(f: &Fixture, client_id: &str, scope: &str) -> String {
    let sid = f.state.admin_sessions.create(Role::Viewer, Some("ana".into())).await;
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in [
        ("response_type", "code"),
        ("client_id", client_id),
        ("redirect_uri", CB),
        ("scope", scope),
        ("state", "st"),
        ("nonce", "n-456"),
        ("code_challenge", CHALLENGE),
        ("code_challenge_method", "S256"),
    ] {
        q.append_pair(k, v);
    }
    let resp = router(f.state.clone())
        .oneshot(
            Request::builder()
                .uri(format!("/oauth/authorize?{}", q.finish()))
                .header(header::COOKIE, format!("{COOKIE_NAME}={sid}"))
                // The user's browser, as seen by the provider.
                .header(header::USER_AGENT, "TestBrowser/1.0")
                .header("x-forwarded-for", "203.0.113.9")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let loc = resp.headers()[header::LOCATION].to_str().unwrap().to_string();
    let query = loc.split_once('?').unwrap().1.to_string();
    url::form_urlencoded::parse(query.as_bytes())
        .find(|(k, _)| k == "code")
        .unwrap_or_else(|| panic!("no code in {loc}"))
        .1
        .into_owned()
}

struct Reply {
    status: StatusCode,
    www_authenticate: String,
    cache: String,
    json: Value,
}

async fn send(state: &AppState, req: Request<Body>) -> Reply {
    let resp = router(state.clone()).oneshot(req).await.unwrap();
    let h = |name: header::HeaderName| {
        resp.headers().get(name).and_then(|v| v.to_str().ok()).unwrap_or("").to_string()
    };
    let (status, www_authenticate, cache) =
        (resp.status(), h(header::WWW_AUTHENTICATE), h(header::CACHE_CONTROL));
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    Reply { status, www_authenticate, cache, json: serde_json::from_slice(&bytes).unwrap_or(Value::Null) }
}

fn form(pairs: &[(&str, &str)]) -> String {
    let mut q = url::form_urlencoded::Serializer::new(String::new());
    for (k, v) in pairs {
        q.append_pair(k, v);
    }
    q.finish()
}

/// The standard exchange body for `code`.
fn exchange(code: &str) -> Vec<(&'static str, String)> {
    vec![
        ("grant_type", "authorization_code".into()),
        ("code", code.into()),
        ("redirect_uri", CB.into()),
        ("code_verifier", VERIFIER.into()),
    ]
}

/// POST /oauth/token with HTTP Basic client authentication.
async fn token_basic(f: &Fixture, id: &str, secret: &str, body: &[(&str, String)]) -> Reply {
    let pairs: Vec<(&str, &str)> = body.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let req = Request::builder()
        .method("POST")
        .uri("/oauth/token")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::AUTHORIZATION, format!("Basic {}", STANDARD.encode(format!("{id}:{secret}"))))
        // The client system's SERVER — not the user.
        .header(header::USER_AGENT, "system-backend/2.0")
        .header("x-forwarded-for", "192.0.2.50")
        .body(Body::from(form(&pairs)))
        .unwrap();
    send(&f.state, req).await
}

async fn userinfo(f: &Fixture, bearer: &str) -> Reply {
    let mut req = Request::builder().uri("/oauth/userinfo");
    if !bearer.is_empty() {
        req = req.header(header::AUTHORIZATION, format!("Bearer {bearer}"));
    }
    send(&f.state, req.body(Body::empty()).unwrap()).await
}

fn jwt_parts(token: &str) -> (Value, Value) {
    let p: Vec<&str> = token.split('.').collect();
    assert_eq!(p.len(), 3);
    let dec = |s: &str| serde_json::from_slice(&URL_SAFE_NO_PAD.decode(s).unwrap()).unwrap();
    (dec(p[0]), dec(p[1]))
}

fn assert_grant_refused(r: &Reply) {
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json["error"], "invalid_grant");
    assert!(r.json.get("id_token").is_none() && r.json.get("access_token").is_none());
}

#[tokio::test]
async fn full_sign_in_yields_a_verifiable_id_token_and_userinfo() {
    let f = fixture(ON).await;
    let user = ruscker_admin::db::users::fetch(&f.db, "ana").await.unwrap().unwrap();
    let code = authorize(&f, "radar", "openid profile email groups").await;
    let before = chrono::Utc::now().timestamp();

    let r = token_basic(&f, "radar", &f.radar, &exchange(&code)).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.json);
    assert_eq!(r.cache, "no-store");
    assert_eq!(r.json["token_type"], "Bearer");
    assert_eq!(r.json["expires_in"], 300);
    assert_eq!(r.json["scope"], "openid profile email groups");
    let id_token = r.json["id_token"].as_str().unwrap();
    let access_token = r.json["access_token"].as_str().unwrap();

    // The ID token verifies against the key the JWKS endpoint publishes —
    // done the way a client library does it, from the public document.
    let jwks = send(&f.state, Request::builder().uri("/oauth/jwks").body(Body::empty()).unwrap()).await;
    let (header, claims) = jwt_parts(id_token);
    assert_eq!(header["alg"], "ES256");
    assert_eq!(header["typ"], "JWT");
    let jwk = jwks.json["keys"]
        .as_array()
        .unwrap()
        .iter()
        .find(|k| k["kid"] == header["kid"])
        .expect("kid is published");
    let mut point = vec![0x04];
    point.extend(URL_SAFE_NO_PAD.decode(jwk["x"].as_str().unwrap()).unwrap());
    point.extend(URL_SAFE_NO_PAD.decode(jwk["y"].as_str().unwrap()).unwrap());
    let (signing_input, sig) = id_token.rsplit_once('.').unwrap();
    ring::signature::UnparsedPublicKey::new(&ring::signature::ECDSA_P256_SHA256_FIXED, point)
        .verify(signing_input.as_bytes(), &URL_SAFE_NO_PAD.decode(sig).unwrap())
        .expect("ID token signature verifies with the published key");

    assert_eq!(claims["iss"], ISSUER);
    assert_eq!(claims["aud"], "radar");
    assert_eq!(claims["sub"], user.id.as_str(), "sub is the immutable account id");
    assert_eq!(claims["nonce"], "n-456");
    assert_eq!(claims["preferred_username"], "ana");
    assert_eq!(claims["email"], "ana@example.org");
    // Only the groups that grant access to THIS client.
    assert_eq!(claims["groups"], json!(["fiscal"]));
    let (iat, exp) = (claims["iat"].as_i64().unwrap(), claims["exp"].as_i64().unwrap());
    assert!(iat >= before && exp - iat == 300);
    assert!(claims.get("auth_time").is_none(), "not emitted: we do not know it");

    // Userinfo agrees with the ID token.
    let u = userinfo(&f, access_token).await;
    assert_eq!(u.status, StatusCode::OK);
    assert_eq!(u.cache, "no-store");
    for k in ["sub", "preferred_username", "email", "groups"] {
        assert_eq!(u.json[k], claims[k], "{k}");
    }
    assert!(u.json.get("aud").is_none(), "userinfo carries identity claims only");

    // One activity event for the sign-in.
    let mut rx = f.state.activity.take_receiver().expect("receiver");
    let ev = rx.try_recv().expect("an activity event");
    assert_eq!(ev.event_type.as_str(), "sso.login");
    assert_eq!(ev.username.as_deref(), Some("ana"));
    assert_eq!(ev.spec_id.as_deref(), Some("radar"));
    // The row is written when the system's server redeems the code, but
    // the address and browser are the USER's, carried by the code.
    assert_eq!(ev.client_ip.as_deref(), Some("203.0.113.9"));
    assert_eq!(ev.user_agent.as_deref(), Some("TestBrowser/1.0"));
    assert!(rx.try_recv().is_err(), "exactly one");
}

#[tokio::test]
async fn scopes_decide_the_claims_and_an_open_client_sees_all_groups() {
    let f = fixture(ON).await;
    // `openid` alone: the subject and nothing about the person.
    let code = authorize(&f, "radar", "openid").await;
    let r = token_basic(&f, "radar", &f.radar, &exchange(&code)).await;
    let (_, claims) = jwt_parts(r.json["id_token"].as_str().unwrap());
    for absent in ["preferred_username", "name", "email", "groups"] {
        assert!(claims.get(absent).is_none(), "{absent} must need its scope");
    }
    let u = userinfo(&f, r.json["access_token"].as_str().unwrap()).await;
    assert_eq!(u.json.as_object().unwrap().len(), 1, "{}", u.json);

    // Client without a group gate: every group, via client_secret_post.
    let code = authorize(&f, "norma", "openid groups").await;
    let mut body = exchange(&code);
    body.push(("client_id", "norma".into()));
    body.push(("client_secret", f.norma.clone()));
    let pairs: Vec<(&str, &str)> = body.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let req = Request::builder()
        .method("POST")
        .uri("/oauth/token")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded; charset=UTF-8")
        .body(Body::from(form(&pairs)))
        .unwrap();
    let r = send(&f.state, req).await;
    assert_eq!(r.status, StatusCode::OK, "{}", r.json);
    let (_, claims) = jwt_parts(r.json["id_token"].as_str().unwrap());
    assert_eq!(claims["aud"], "norma");
    assert_eq!(claims["groups"], json!(["fiscal", "outro"]));
}

#[tokio::test]
async fn a_code_is_redeemed_once() {
    let f = fixture(ON).await;
    let code = authorize(&f, "radar", "openid").await;
    assert_eq!(token_basic(&f, "radar", &f.radar, &exchange(&code)).await.status, StatusCode::OK);
    assert_grant_refused(&token_basic(&f, "radar", &f.radar, &exchange(&code)).await);
    assert_grant_refused(&token_basic(&f, "radar", &f.radar, &exchange("not-a-code")).await);
}

/// A failed CLIENT authentication never touches the code: whoever
/// intercepted a code but not the secret cannot even invalidate it.
#[tokio::test]
async fn failed_client_authentication_does_not_burn_the_code() {
    let f = fixture(ON).await;
    let code = authorize(&f, "radar", "openid").await;

    let r = token_basic(&f, "radar", "wrong-secret", &exchange(&code)).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.json["error"], "invalid_client");
    assert!(r.www_authenticate.starts_with("Basic"), "Basic attempt gets a Basic challenge");
    // Another client's valid secret is not this client's secret.
    assert_eq!(token_basic(&f, "radar", &f.norma, &exchange(&code)).await.status, StatusCode::UNAUTHORIZED);
    assert_eq!(token_basic(&f, "nope", &f.radar, &exchange(&code)).await.status, StatusCode::UNAUTHORIZED);

    // No credentials at all.
    let pairs: Vec<(&str, String)> = exchange(&code);
    let pairs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let anonymous = |body: String| {
        Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .body(Body::from(body))
            .unwrap()
    };
    let r = send(&f.state, anonymous(form(&pairs))).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);
    assert_eq!(r.json["error"], "invalid_client");
    // client_id without a secret (a "public client" attempt) is refused.
    let r = send(&f.state, anonymous(format!("{}&client_id=radar", form(&pairs)))).await;
    assert_eq!(r.status, StatusCode::UNAUTHORIZED);

    // A disabled client cannot redeem…
    let off = ClientSettings {
        name: "C".into(),
        redirect_uris: vec![CB.into()],
        access_groups: vec!["fiscal".into()],
        require_mfa: false,
        enabled: false,
    };
    ruscker_admin::db::oidc_clients::update(&f.db, "radar", &off, None).await.unwrap();
    assert_eq!(token_basic(&f, "radar", &f.radar, &exchange(&code)).await.status, StatusCode::UNAUTHORIZED);
    let on = ClientSettings { enabled: true, ..off };
    ruscker_admin::db::oidc_clients::update(&f.db, "radar", &on, None).await.unwrap();

    // …and after all those failures the code is still good, once.
    assert_eq!(token_basic(&f, "radar", &f.radar, &exchange(&code)).await.status, StatusCode::OK);
}

/// Everything a code is bound to. Each wrong binding is refused AND
/// spends the code: an authenticated client gets one attempt.
#[tokio::test]
async fn wrong_bindings_are_refused_and_spend_the_code() {
    let f = fixture(ON).await;
    let good = |code: &str| exchange(code);

    // PKCE: wrong verifier, the challenge itself (`plain`), missing.
    for verifier in ["wrong-verifier-wrong-verifier-wrong-verifier-1", CHALLENGE] {
        let code = authorize(&f, "radar", "openid").await;
        let mut body = good(&code);
        body.retain(|(k, _)| *k != "code_verifier");
        body.push(("code_verifier", verifier.to_string()));
        assert_grant_refused(&token_basic(&f, "radar", &f.radar, &body).await);
        assert_grant_refused(&token_basic(&f, "radar", &f.radar, &good(&code)).await);
    }

    // Redirect URI differs from the one in the authorization request.
    let code = authorize(&f, "radar", "openid").await;
    let mut body = good(&code);
    body.retain(|(k, _)| *k != "redirect_uri");
    body.push(("redirect_uri", "https://radar.example/other".into()));
    assert_grant_refused(&token_basic(&f, "radar", &f.radar, &body).await);
    assert_grant_refused(&token_basic(&f, "radar", &f.radar, &good(&code)).await);

    // A code issued to `radar`, presented by `norma` with ITS valid secret.
    let code = authorize(&f, "radar", "openid").await;
    assert_grant_refused(&token_basic(&f, "norma", &f.norma, &good(&code)).await);
    assert_grant_refused(&token_basic(&f, "radar", &f.radar, &good(&code)).await);
}

/// The user is re-read at redemption: losing access, or being replaced
/// by another account of the same name, between the two steps wins.
#[tokio::test]
async fn the_user_is_checked_again_at_redemption() {
    let f = fixture(ON).await;

    let code = authorize(&f, "radar", "openid").await;
    ruscker_admin::db::users::set_groups(&f.db, "ana", &["outro".to_string()], None).await.unwrap();
    assert_grant_refused(&token_basic(&f, "radar", &f.radar, &exchange(&code)).await);
    ruscker_admin::db::users::set_groups(&f.db, "ana", &["fiscal".to_string()], None).await.unwrap();

    // Same username, different account.
    let code = authorize(&f, "radar", "openid").await;
    ruscker_admin::db::users::delete(&f.db, "ana", None).await.unwrap();
    ruscker_admin::db::users::create(&f.db, "ana", PASSWORD, Role::Viewer, false, &["fiscal".to_string()], None)
        .await
        .unwrap();
    assert_grant_refused(&token_basic(&f, "radar", &f.radar, &exchange(&code)).await);
}

#[tokio::test]
async fn malformed_token_requests_are_refused_without_spending_the_code() {
    let f = fixture(ON).await;
    let code = authorize(&f, "radar", "openid").await;
    let basic = format!("Basic {}", STANDARD.encode(format!("radar:{}", f.radar)));
    let post = |content_type: Option<&str>, body: String| {
        let mut req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header(header::AUTHORIZATION, basic.clone());
        if let Some(ct) = content_type {
            req = req.header(header::CONTENT_TYPE, ct);
        }
        req.body(Body::from(body)).unwrap()
    };
    let ok_body = {
        let pairs = exchange(&code);
        let pairs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
        form(&pairs)
    };
    let fe = "application/x-www-form-urlencoded";

    let cases = [
        (post(None, ok_body.clone()), "invalid_request"),
        (post(Some("application/json"), ok_body.clone()), "invalid_request"),
        // A repeated parameter.
        (post(Some(fe), format!("{ok_body}&code=other")), "invalid_request"),
        // Both authentication methods at once.
        (post(Some(fe), format!("{ok_body}&client_id=radar&client_secret={}", f.radar)), "invalid_request"),
        (post(Some(fe), ok_body.replace("authorization_code", "password")), "unsupported_grant_type"),
        (post(Some(fe), ok_body.replace("authorization_code", "refresh_token")), "unsupported_grant_type"),
        (post(Some(fe), ok_body.replace("code_verifier", "x")), "invalid_request"),
    ];
    for (req, error) in cases {
        let r = send(&f.state, req).await;
        assert_eq!(r.status, StatusCode::BAD_REQUEST, "{error}");
        assert_eq!(r.json["error"], error);
    }
    // GET is not a way to redeem.
    let r = send(&f.state, Request::builder().uri(format!("/oauth/token?{ok_body}")).body(Body::empty()).unwrap()).await;
    assert_eq!(r.status, StatusCode::METHOD_NOT_ALLOWED);
    // None of that consumed the code.
    assert_eq!(token_basic(&f, "radar", &f.radar, &exchange(&code)).await.status, StatusCode::OK);
}

#[tokio::test]
async fn userinfo_accepts_only_a_live_access_token_of_ours() {
    let f = fixture(ON).await;
    let code = authorize(&f, "radar", "openid profile").await;
    let r = token_basic(&f, "radar", &f.radar, &exchange(&code)).await;
    let access = r.json["access_token"].as_str().unwrap().to_string();
    let id_token = r.json["id_token"].as_str().unwrap().to_string();
    let user = ruscker_admin::db::users::fetch(&f.db, "ana").await.unwrap().unwrap();

    let refused = |r: &Reply| {
        assert_eq!(r.status, StatusCode::UNAUTHORIZED);
        assert!(r.www_authenticate.starts_with("Bearer"));
        assert!(r.json.is_null(), "no claims on a refusal");
    };
    refused(&userinfo(&f, "").await);
    refused(&userinfo(&f, "garbage").await);
    // The ID token is not an access token.
    refused(&userinfo(&f, &id_token).await);

    // Tokens we really signed, but that must not be accepted.
    let key = ruscker_admin::oidc::ensure_signing_key(&f.db, &f.state.master_key).await.unwrap();
    let now = chrono::Utc::now().timestamp();
    let claims = |patch: Value| {
        let mut c = json!({
            "iss": ISSUER, "aud": ISSUER, "sub": user.id, "username": "ana",
            "client_id": "radar", "scope": "openid profile", "iat": now, "exp": now + 60,
        });
        for (k, v) in patch.as_object().unwrap() {
            c[k] = v.clone();
        }
        c
    };
    let sign = |typ: &str, c: Value| ruscker_admin::oidc::sign_jwt(&key, typ, &c).unwrap();
    // Sanity: the baseline is accepted, so each refusal below is due to
    // the one thing changed.
    assert_eq!(userinfo(&f, &sign("at+jwt", claims(json!({})))).await.status, StatusCode::OK);
    refused(&userinfo(&f, &sign("at+jwt", claims(json!({"exp": now - 1})))).await);
    refused(&userinfo(&f, &sign("at+jwt", claims(json!({"aud": "radar"})))).await);
    refused(&userinfo(&f, &sign("at+jwt", claims(json!({"iss": "https://evil.example"})))).await);
    refused(&userinfo(&f, &sign("JWT", claims(json!({})))).await);
    refused(&userinfo(&f, &sign("at+jwt", claims(json!({"sub": "someone-else"})))).await);
    refused(&userinfo(&f, &sign("at+jwt", claims(json!({"client_id": "nope"})))).await);

    // The real token stops working when the user loses access.
    assert_eq!(userinfo(&f, &access).await.status, StatusCode::OK);
    ruscker_admin::db::users::set_groups(&f.db, "ana", &[], None).await.unwrap();
    refused(&userinfo(&f, &access).await);
}

/// Reachable without credentials, so what an anonymous caller can make
/// the endpoint DO is bounded: an oversized body is refused before it is
/// parsed, and an impossible client id never reaches the database.
#[tokio::test]
async fn anonymous_abuse_is_bounded() {
    let f = fixture(ON).await;
    let post = |body: String, auth: Option<String>| {
        let mut req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        if let Some(a) = auth {
            req = req.header(header::AUTHORIZATION, a);
        }
        req.body(Body::from(body)).unwrap()
    };
    // ~1 MB of distinct keys: refused by the body cap (413), not parsed.
    let huge: String = (0..150_000).map(|i| format!("k{i}=v&")).collect();
    let r = send(&f.state, post(huge, None)).await;
    assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE);
    // Within the size cap but with too many parameters.
    let many: String = (0..200).map(|i| format!("k{i}=v&")).collect();
    let r = send(&f.state, post(many, None)).await;
    assert_eq!(r.status, StatusCode::BAD_REQUEST);
    assert_eq!(r.json["error"], "invalid_request");

    // Client ids that could never be registered: plain invalid_client,
    // via the body and via Basic (where they are percent-decoded).
    let body = "grant_type=authorization_code&code=x&redirect_uri=x&code_verifier=x";
    for id in ["%00abc", "Radar%2F..", &"a".repeat(300)] {
        let r = send(&f.state, post(format!("{body}&client_id={id}&client_secret=x"), None)).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{id}");
        assert_eq!(r.json["error"], "invalid_client");
        let basic = format!("Basic {}", STANDARD.encode(format!("{id}:x")));
        let r = send(&f.state, post(body.to_string(), Some(basic))).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{id} (basic)");
    }
}

/// "One method, never both" — including an Authorization header that is
/// not readable Basic: it fails, it does not fall back to the body.
#[tokio::test]
async fn an_unreadable_authorization_header_is_not_ignored() {
    let f = fixture(ON).await;
    let code = authorize(&f, "radar", "openid").await;
    let pairs = exchange(&code);
    let pairs: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let body = format!("{}&client_id=radar&client_secret={}", form(&pairs), f.radar);
    for auth in ["Bearer something", "Basic !!!not-base64", "Negotiate abc"] {
        let req = Request::builder()
            .method("POST")
            .uri("/oauth/token")
            .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
            .header(header::AUTHORIZATION, auth)
            .body(Body::from(body.clone()))
            .unwrap();
        let r = send(&f.state, req).await;
        assert_eq!(r.status, StatusCode::UNAUTHORIZED, "{auth}");
        assert_eq!(r.json["error"], "invalid_client");
    }
    // A body client_id that disagrees with the Basic header.
    let mut with_other = exchange(&code);
    with_other.push(("client_id", "norma".into()));
    assert_eq!(token_basic(&f, "radar", &f.radar, &with_other).await.status, StatusCode::UNAUTHORIZED);
    // The scheme name is case-insensitive, and none of the above spent
    // the code.
    let req = Request::builder()
        .method("POST")
        .uri("/oauth/token")
        .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
        .header(header::AUTHORIZATION, format!("BASIC {}", STANDARD.encode(format!("radar:{}", f.radar))))
        .body(Body::from(form(&pairs)))
        .unwrap();
    assert_eq!(send(&f.state, req).await.status, StatusCode::OK);
}

#[tokio::test]
async fn an_expired_code_is_refused() {
    let f = fixture(ON).await;
    let code = authorize(&f, "radar", "openid").await;
    let ConfigDb::Sqlite(pool) = &f.db else { unreachable!() };
    sqlx::query("UPDATE oidc_auth_codes SET expires_at = ?")
        .bind(chrono::Utc::now() - chrono::Duration::seconds(1))
        .execute(pool)
        .await
        .unwrap();
    assert_grant_refused(&token_basic(&f, "radar", &f.radar, &exchange(&code)).await);
}

/// Userinfo trusts nothing but OUR signature: the same header and
/// claims with a signature that does not verify — tampered, or made by
/// another installation's key — are refused. (This is the test that
/// fails if signature verification is skipped.)
#[tokio::test]
async fn userinfo_refuses_a_token_we_did_not_sign() {
    let f = fixture(ON).await;
    let code = authorize(&f, "radar", "openid profile").await;
    let r = token_basic(&f, "radar", &f.radar, &exchange(&code)).await;
    let access = r.json["access_token"].as_str().unwrap().to_string();
    assert_eq!(userinfo(&f, &access).await.status, StatusCode::OK);

    // Same header and signature, claims swapped for another subject.
    let p: Vec<&str> = access.split('.').collect();
    let (_, mut claims) = jwt_parts(&access);
    claims["username"] = json!("root");
    let forged = format!("{}.{}.{}", p[0], URL_SAFE_NO_PAD.encode(claims.to_string()), p[2]);
    assert_eq!(userinfo(&f, &forged).await.status, StatusCode::UNAUTHORIZED);

    // A structurally perfect token signed by ANOTHER installation (same
    // claims, its own key) — and one with our kid but their signature.
    let other = fixture(ON).await;
    let their_key = ruscker_admin::oidc::ensure_signing_key(&other.db, &other.state.master_key).await.unwrap();
    let (header, claims) = jwt_parts(&access);
    let theirs = ruscker_admin::oidc::sign_jwt(&their_key, "at+jwt", &claims).unwrap();
    assert_eq!(userinfo(&f, &theirs).await.status, StatusCode::UNAUTHORIZED);
    let t: Vec<&str> = theirs.split('.').collect();
    let our_kid_their_sig = format!("{}.{}.{}", URL_SAFE_NO_PAD.encode(header.to_string()), t[1], t[2]);
    assert_eq!(userinfo(&f, &our_kid_their_sig).await.status, StatusCode::UNAUTHORIZED);
}

/// Userinfo reflects the present: a disabled client or a deleted user
/// ends it, and POST works like GET. A cross-site browser POST is
/// stopped by the CSRF guard.
#[tokio::test]
async fn userinfo_follows_revocation_and_accepts_post() {
    let f = fixture(ON).await;
    let code = authorize(&f, "radar", "openid").await;
    let r = token_basic(&f, "radar", &f.radar, &exchange(&code)).await;
    let access = r.json["access_token"].as_str().unwrap().to_string();
    let post = |site: Option<&str>| {
        let mut req = Request::builder()
            .method("POST")
            .uri("/oauth/userinfo")
            .header(header::AUTHORIZATION, format!("bearer {access}"));
        if let Some(site) = site {
            req = req.header("sec-fetch-site", site);
        }
        req.body(Body::empty()).unwrap()
    };
    assert_eq!(send(&f.state, post(None)).await.status, StatusCode::OK, "server-to-server POST");
    assert_eq!(send(&f.state, post(Some("cross-site"))).await.status, StatusCode::FORBIDDEN);

    let off = ClientSettings {
        name: "C".into(),
        redirect_uris: vec![CB.into()],
        access_groups: vec!["fiscal".into()],
        require_mfa: false,
        enabled: false,
    };
    ruscker_admin::db::oidc_clients::update(&f.db, "radar", &off, None).await.unwrap();
    assert_eq!(userinfo(&f, &access).await.status, StatusCode::UNAUTHORIZED, "client disabled");
    let on = ClientSettings { enabled: true, ..off };
    ruscker_admin::db::oidc_clients::update(&f.db, "radar", &on, None).await.unwrap();
    assert_eq!(userinfo(&f, &access).await.status, StatusCode::OK);

    ruscker_admin::db::users::delete(&f.db, "ana", None).await.unwrap();
    assert_eq!(userinfo(&f, &access).await.status, StatusCode::UNAUTHORIZED, "user deleted");
}

#[tokio::test]
async fn the_endpoints_do_not_exist_when_the_provider_is_off() {
    let f = fixture(OFF).await;
    let r = token_basic(&f, "radar", &f.radar, &exchange("x")).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert_eq!(userinfo(&f, "x").await.status, StatusCode::NOT_FOUND);
}
