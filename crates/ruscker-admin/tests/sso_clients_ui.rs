//! Integration tests for the SSO client registry (OIDC provider, slice 1):
//! the secret is shown exactly once and never cached, invalid input and
//! duplicate ids change nothing, edit/rotate/delete, and the admin-only
//! gate. Harness copied from `schedules_ui.rs`.

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::db::ConfigDb;
use ruscker_admin::i18n::Locales;
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::sync::Arc;
use tower::ServiceExt;

const CONFIG: &str = r#"
proxy:
  title: Ruscker Test
  port: 8088
  specs:
    - id: etl-app
      display-name: ETL App
      container-image: demo/etl
    # No container-image ⇒ auto-classified External (nothing to run).
    - id: external-app
      display-name: External App
"#;

async fn open_db() -> ConfigDb {
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ruscker-sso-clients-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    ConfigDb::Sqlite(ruscker_admin::db::open(&path).await.unwrap())
}

async fn app_state(db: ConfigDb) -> AppState {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let config = Config::from_yaml(CONFIG).expect("parse config");
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
        identity_cache: Default::default(),
        catalog_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        access_counter: Arc::new(ruscker_admin::access_counter::AccessCounter::default()),
        alerts: ruscker_admin::alerts::AlertSink::default(),
        activity: ruscker_admin::activity::ActivitySink::default(),
    }
}

/// Mint an admin session straight into the shared store and return the
/// cookie header value.
async fn admin_cookie(state: &AppState) -> String {
    let sid = state
        .admin_sessions
        .create(Role::Admin, Some("root".into()))
        .await;
    format!("{COOKIE_NAME}={sid}")
}

async fn get(state: AppState, path: &str, cookie: &str) -> (StatusCode, String) {
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(path)
                .header(header::COOKIE, cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// POST a form; returns (status, Location header).
async fn post(state: AppState, uri: &str, body: &str, cookie: &str) -> (StatusCode, String) {
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    let status = resp.status();
    let loc = resp
        .headers()
        .get(header::LOCATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    (status, loc)
}

/// POST a form; returns (status, Cache-Control header, body).
async fn post_page(state: AppState, uri: &str, body: &str, cookie: &str) -> (StatusCode, String, String) {
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header(header::COOKIE, cookie)
                .header(header::CONTENT_TYPE, "application/x-www-form-urlencoded")
                .body(Body::from(body.to_string()))
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

/// Pull the 64-hex secret out of the one-time banner.
fn shown_secret(html: &str) -> Option<String> {
    let marker = "data-sso-secret-value";
    let at = html.find(marker)?;
    let rest = &html[at..];
    let start = rest.find('>')? + 1;
    let end = rest[start..].find('<')? + start;
    Some(rest[start..end].trim().to_string())
}

const CREATE: &str = "client_id=radar-fiscal&name=Radar+Fiscal\
&redirect_uris=https%3A%2F%2Fradar.example%2Fcb&access_groups=fiscal%2C+gestores&require_mfa=on";

#[tokio::test]
async fn create_shows_the_secret_once_and_persists_the_gate() {
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;

    let (status, cache, html) = post_page(state.clone(), "/admin/sso-clients", CREATE, &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cache, "no-store", "a page carrying a secret must not be cached");
    let secret = shown_secret(&html).expect("secret banner");
    assert_eq!(secret.len(), 64);

    let db = state.db.clone().unwrap();
    let c = ruscker_admin::db::oidc_clients::fetch(&db, "radar-fiscal")
        .await
        .unwrap()
        .expect("persisted");
    assert!(c.verify_secret(&secret), "the shown secret is the stored one");
    assert_eq!(c.access_groups, vec!["fiscal".to_string(), "gestores".to_string()]);
    assert!(c.require_mfa && c.enabled);
    assert_eq!(c.redirect_uris, vec!["https://radar.example/cb".to_string()]);

    // A plain reload of the list never shows the secret again.
    let (status, html) = get(state, "/admin/sso-clients", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("radar-fiscal"));
    assert!(!html.contains(&secret));
    assert!(shown_secret(&html).is_none());
}

#[tokio::test]
async fn invalid_input_is_rejected_and_not_persisted() {
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;
    let db = state.db.clone().unwrap();

    for body in [
        // Uppercase / slash in the id.
        "client_id=Bad%2FId&name=X&redirect_uris=https%3A%2F%2Fa.example%2Fcb",
        // Plain http to a non-loopback host.
        "client_id=okid&name=X&redirect_uris=http%3A%2F%2Fa.example%2Fcb",
        // Wildcard.
        "client_id=okid&name=X&redirect_uris=https%3A%2F%2F*.example%2Fcb",
        // No redirect URI at all.
        "client_id=okid&name=X&redirect_uris=",
    ] {
        let (status, _, html) = post_page(state.clone(), "/admin/sso-clients", body, &cookie).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(html.contains("role=\"alert\""), "error banner for {body}");
        assert!(shown_secret(&html).is_none(), "no secret for {body}");
    }
    assert!(ruscker_admin::db::oidc_clients::list_all(&db).await.unwrap().is_empty());
}

#[tokio::test]
async fn duplicate_id_keeps_the_existing_client_and_secret() {
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;
    let (_, _, html) = post_page(state.clone(), "/admin/sso-clients", CREATE, &cookie).await;
    let secret = shown_secret(&html).unwrap();

    let (status, _, html) = post_page(
        state.clone(),
        "/admin/sso-clients",
        "client_id=radar-fiscal&name=Other&redirect_uris=https%3A%2F%2Fevil.example%2Fcb",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(shown_secret(&html).is_none());

    let db = state.db.clone().unwrap();
    let c = ruscker_admin::db::oidc_clients::fetch(&db, "radar-fiscal").await.unwrap().unwrap();
    assert_eq!(c.name, "Radar Fiscal");
    assert!(c.verify_secret(&secret));
    assert!(!c.redirect_uri_registered("https://evil.example/cb"));
}

#[tokio::test]
async fn edit_rotate_and_delete() {
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;
    let db = state.db.clone().unwrap();
    let (_, _, html) = post_page(state.clone(), "/admin/sso-clients", CREATE, &cookie).await;
    let old = shown_secret(&html).unwrap();

    // Edit: drop MFA, disable (no `enabled` field = unchecked), new groups.
    let (status, cache, html) = post_page(
        state.clone(),
        "/admin/sso-clients/radar-fiscal/edit",
        "name=Radar&redirect_uris=https%3A%2F%2Fradar.example%2Fcb&access_groups=",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_ne!(cache, "no-store", "no secret on a plain save");
    assert!(shown_secret(&html).is_none());
    let c = ruscker_admin::db::oidc_clients::fetch(&db, "radar-fiscal").await.unwrap().unwrap();
    assert_eq!(c.name, "Radar");
    assert!(!c.require_mfa && !c.enabled && c.access_groups.is_empty());
    assert!(c.verify_secret(&old), "saving settings never touches the secret");

    // Rotate: new secret shown once, old one dead. The form carries the
    // version rendered on the edit page.
    assert!(html.contains(r#"name="secret_version" value="1""#), "rotate form carries the version");
    let rotate_body = "secret_version=1";
    let (status, cache, html) =
        post_page(state.clone(), "/admin/sso-clients/radar-fiscal/rotate", rotate_body, &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cache, "no-store");
    let new = shown_secret(&html).expect("rotated secret banner");
    assert!(html.contains(r#"name="secret_version" value="2""#), "page now carries the new version");
    let c = ruscker_admin::db::oidc_clients::fetch(&db, "radar-fiscal").await.unwrap().unwrap();
    assert!(c.verify_secret(&new) && !c.verify_secret(&old));

    // F5 on that result page re-sends the SAME request. It must not
    // rotate again: the secret just copied stays valid, and the page
    // says so instead of showing another secret.
    let (status, _, html) =
        post_page(state.clone(), "/admin/sso-clients/radar-fiscal/rotate", rotate_body, &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(shown_secret(&html).is_none(), "a replay shows no new secret");
    assert!(html.contains("role=\"alert\""), "and explains itself");
    let c = ruscker_admin::db::oidc_clients::fetch(&db, "radar-fiscal").await.unwrap().unwrap();
    assert!(c.verify_secret(&new), "the secret handed out survives the replay");

    // An invalid edit re-renders the form with an error and saves nothing.
    let (status, _, html) = post_page(
        state.clone(),
        "/admin/sso-clients/radar-fiscal/edit",
        "name=Radar&redirect_uris=http%3A%2F%2Fevil.example%2Fcb",
        &cookie,
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert!(html.contains("role=\"alert\""));
    let c = ruscker_admin::db::oidc_clients::fetch(&db, "radar-fiscal").await.unwrap().unwrap();
    assert!(!c.redirect_uri_registered("http://evil.example/cb"));

    // Delete.
    let (status, loc) = post(state.clone(), "/admin/sso-clients/radar-fiscal/delete", "", &cookie).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_eq!(loc, "/admin/sso-clients?flash=deleted");
    assert!(ruscker_admin::db::oidc_clients::fetch(&db, "radar-fiscal").await.unwrap().is_none());

    // Unknown ids are 404 on every per-client route.
    let (status, _) = get(state.clone(), "/admin/sso-clients/nope/edit", &cookie).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let (status, _) =
        post(state.clone(), "/admin/sso-clients/nope/rotate", "secret_version=1", &cookie).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    // …including an edit whose fields are INVALID: existence is checked
    // before validation, so no edit page is rendered for a ghost.
    for body in ["name=&redirect_uris=", "name=X&redirect_uris=https%3A%2F%2Fa.example%2Fcb"] {
        let (status, _) = post(state.clone(), "/admin/sso-clients/nope/edit", body, &cookie).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    }
    let (status, _) = post(state, "/admin/sso-clients/nope/delete", "", &cookie).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Every route, as an Editor, a Viewer and anonymously — against a
/// client that EXISTS, so "nothing changed" is a real observation.
#[tokio::test]
async fn registry_is_admin_only() {
    let state = app_state(open_db().await).await;
    let admin = admin_cookie(&state).await;
    let db = state.db.clone().unwrap();
    let (_, _, html) = post_page(state.clone(), "/admin/sso-clients", CREATE, &admin).await;
    let secret = shown_secret(&html).unwrap();

    let mut cookies: Vec<(String, StatusCode)> = Vec::new();
    for role in [Role::Editor, Role::Viewer] {
        let sid = state.admin_sessions.create(role, Some("someone".into())).await;
        cookies.push((format!("{COOKIE_NAME}={sid}"), StatusCode::FORBIDDEN));
    }
    // No session at all is sent to the login page.
    cookies.push((String::new(), StatusCode::SEE_OTHER));

    for (cookie, refusal) in &cookies {
        for uri in ["/admin/sso-clients", "/admin/sso-clients/radar-fiscal/edit"] {
            let (status, html) = get(state.clone(), uri, cookie).await;
            assert_eq!(status, *refusal, "GET {uri}");
            assert!(!html.contains("radar.example"), "GET {uri} leaks nothing");
        }
        for (uri, body) in [
            ("/admin/sso-clients", "client_id=other&name=X&redirect_uris=https%3A%2F%2Fa.example%2Fcb"),
            ("/admin/sso-clients/radar-fiscal/edit", "name=Hacked&redirect_uris=https%3A%2F%2Fevil.example%2Fcb"),
            ("/admin/sso-clients/radar-fiscal/rotate", "secret_version=1"),
            ("/admin/sso-clients/radar-fiscal/delete", ""),
        ] {
            let (status, _, html) = post_page(state.clone(), uri, body, cookie).await;
            assert_eq!(status, *refusal, "POST {uri}");
            assert!(shown_secret(&html).is_none(), "POST {uri} shows no secret");
        }
    }

    // After all of that the registry is exactly as the Admin left it.
    let all = ruscker_admin::db::oidc_clients::list_all(&db).await.unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].name, "Radar Fiscal");
    assert!(all[0].verify_secret(&secret), "secret not rotated");
    assert!(!all[0].redirect_uri_registered("https://evil.example/cb"));
}

/// The hook in the Groups page, end to end through its routes: a rename
/// follows into the client gate, and deleting the last group disables
/// the client. Removing the hook line in `groups.rs` fails this test.
#[tokio::test]
async fn groups_page_rename_and_delete_reach_the_client_gate() {
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;
    let db = state.db.clone().unwrap();
    post_page(
        state.clone(),
        "/admin/sso-clients",
        "client_id=radar&name=Radar&redirect_uris=https%3A%2F%2Fradar.example%2Fcb&access_groups=fiscal",
        &cookie,
    )
    .await;

    let (status, _) = post(state.clone(), "/admin/groups/rename", "old=fiscal&new=fazenda", &cookie).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let c = ruscker_admin::db::oidc_clients::fetch(&db, "radar").await.unwrap().unwrap();
    assert_eq!(c.access_groups, vec!["fazenda".to_string()]);
    assert!(c.enabled);

    let (status, _) = post(state.clone(), "/admin/groups/delete", "name=fazenda", &cookie).await;
    assert_eq!(status, StatusCode::SEE_OTHER);
    let c = ruscker_admin::db::oidc_clients::fetch(&db, "radar").await.unwrap().unwrap();
    assert!(!c.enabled, "disabled, not opened");
    assert_eq!(c.access_groups, vec!["fazenda".to_string()], "gate kept");
}

/// SSO section → Access (#1079): only SSO events, with who, which
/// system, from where and — for a refusal — why. Admin only. The
/// general Activity page keeps showing everything.
#[tokio::test]
async fn access_tab_lists_sso_sign_ins_and_refusals_only() {
    use ruscker_admin::activity::{ActivityEvent, AuthMethod, ClientInfo};
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;
    let db = state.db.clone().unwrap();
    let from = |ip: &str, ua: &str| ClientInfo { ip: Some(ip.into()), user_agent: Some(ua.into()) };
    let firefox = "Mozilla/5.0 (X11; Linux x86_64; rv:152.0) Gecko/20100101 Firefox/152.0";
    ruscker_admin::db::user_activity::insert_batch(
        &db,
        &[
            ActivityEvent::sso_login("ana", "radar-fiscal", "k1").with_client(from("203.0.113.9", firefox)),
            ActivityEvent::sso_denied(Some("bia".into()), "norma", "not-in-group").with_client(from("203.0.113.77", firefox)),
            ActivityEvent::login_success("carla", AuthMethod::Password, "k2", None),
            ActivityEvent::app_access(Some("carla".into()), "painel-interno", "k3", None, None),
        ],
    )
    .await
    .unwrap();

    let (status, html) = get(state.clone(), "/admin/sso-access", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    // Both SSO rows, with system, address, a readable browser and the
    // raw User-Agent as the tooltip.
    for needle in ["radar-fiscal", "norma", "203.0.113.9", "203.0.113.77", "Firefox · Linux", "rv:152.0"] {
        assert!(html.contains(needle), "missing {needle}");
    }
    // The refusal says why, in words (whichever locale the harness
    // negotiates), not as the raw machine code.
    assert!(
        html.contains("not in the system") || html.contains("fora dos grupos permitidos"),
        "refusal reason shown"
    );
    assert!(!html.contains(">not-in-group<"));
    // Nothing that is not SSO: the app access row is absent.
    assert!(!html.contains("painel-interno"));
    // It is the Access tab of the SSO section, and its own links stay on it.
    assert!(html.contains(r#"href="/admin/sso-clients""#));
    assert!(html.contains(r#"action="/admin/sso-access""#));
    assert!(!html.contains(r#"action="/admin/activity""#));

    // Filtering to refusals drops the sign-in row.
    let (_, html) = get(state.clone(), "/admin/sso-access?event=sso.denied", &cookie).await;
    assert!(html.contains("203.0.113.77") && !html.contains("203.0.113.9<"));
    // A non-SSO event name is not a way to widen the tab.
    let (_, html) = get(state.clone(), "/admin/sso-access?event=app.access", &cookie).await;
    assert!(!html.contains("painel-interno"));

    // The general Activity page still shows everything, refusals included.
    let (_, html) = get(state.clone(), "/admin/activity", &cookie).await;
    assert!(html.contains("painel-interno") && html.contains("norma"));

    // The SSO section has its own nav entry; Credentials no longer hosts it.
    let (_, creds) = get(state.clone(), "/admin/credentials", &cookie).await;
    assert_eq!(creds.matches(r#"href="/admin/sso-clients""#).count(), 1, "only the top nav links there");

    // Admin only.
    let sid = state.admin_sessions.create(Role::Editor, Some("ed".into())).await;
    let (status, html) = get(state.clone(), "/admin/sso-access", &format!("{COOKIE_NAME}={sid}")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!html.contains("203.0.113.9"));
    // The general Activity page shows the same rows, so it must be
    // just as closed to an Editor.
    let (status, html) = get(state.clone(), "/admin/activity", &format!("{COOKIE_NAME}={sid}")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(!html.contains("203.0.113.9"));
    let (status, _) = get(state, "/admin/sso-access", "").await;
    assert_eq!(status, StatusCode::SEE_OTHER);
}
