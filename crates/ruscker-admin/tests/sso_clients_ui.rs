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
/// A session for `name` with `role`, creating the account first: a session
/// whose account does not exist is sent to the login (security review
/// 2026-11-19), which would hide the role check a test is about.
async fn session_as(state: &AppState, role: Role, name: &str) -> String {
    if let Some(db) = state.db.as_ref() {
        if ruscker_admin::db::users::fetch(db, name).await.unwrap().is_none() {
            ruscker_admin::db::users::create(db, name, "SessionPass9!", role, false, &[], None)
                .await
                .expect("fixture account");
        }
    }
    state.admin_sessions.create(role, Some(name.to_string())).await
}

async fn admin_cookie(state: &AppState) -> String {
    // The account behind the session must exist: a session whose account
    // is gone is sent back to the login (security review 2026-11-19).
    if let Some(db) = state.db.as_ref() {
        if ruscker_admin::db::users::fetch(db, "root").await.unwrap().is_none() {
            ruscker_admin::db::users::create(db, "root", "RootPass9!", Role::Admin, false, &[], None)
                .await
                .expect("fixture account");
        }
    }
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
    // It can be closed: a plain link to the list, fetched again.
    assert!(html.contains(r#"href="/admin/sso-clients" class="btn" data-sso-dismiss"#));
    // Each client opens its edit page from its id and its name.
    let (_, list) = get(state.clone(), "/admin/sso-clients", &cookie).await;
    assert!(list.matches(r#"href="/admin/sso-clients/radar-fiscal/edit""#).count() >= 3);
    // Shown once, so it must not be a toast that fades away by itself.
    assert!(html.contains("data-sso-secret data-keep"));
    assert!(html.contains(r#".admin-flash-ok:not([data-keep])"#));

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
    // Saved: back to the list (the edit page used to stay open, which
    // read as "nothing happened"), and the list says so.
    assert_eq!(status, StatusCode::SEE_OTHER);
    assert_ne!(cache, "no-store", "no secret on a plain save");
    assert!(shown_secret(&html).is_none());
    let (st, loc) = post(
        state.clone(),
        "/admin/sso-clients/radar-fiscal/edit",
        "name=Radar&redirect_uris=https%3A%2F%2Fradar.example%2Fcb&access_groups=",
        &cookie,
    )
    .await;
    assert_eq!((st, loc.as_str()), (StatusCode::SEE_OTHER, "/admin/sso-clients?flash=saved"));
    let (_, list) = get(state.clone(), "/admin/sso-clients?flash=saved", &cookie).await;
    assert!(list.contains("admin-flash-ok"), "the list confirms the save");
    let c = ruscker_admin::db::oidc_clients::fetch(&db, "radar-fiscal").await.unwrap().unwrap();
    assert_eq!(c.name, "Radar");
    assert!(!c.require_mfa && !c.enabled && c.access_groups.is_empty());
    assert!(c.verify_secret(&old), "saving settings never touches the secret");

    // Rotate: new secret shown once, old one dead. The form carries the
    // version rendered on the edit page.
    let (_, html) = get(state.clone(), "/admin/sso-clients/radar-fiscal/edit", &cookie).await;
    assert!(html.contains(r#"name="secret_version" value="1""#), "rotate form carries the version");
    let rotate_body = "secret_version=1";
    let (status, cache, html) =
        post_page(state.clone(), "/admin/sso-clients/radar-fiscal/rotate", rotate_body, &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(cache, "no-store");
    let new = shown_secret(&html).expect("rotated secret banner");
    assert!(html.contains(r#"name="secret_version" value="2""#), "page now carries the new version");
    assert!(html.contains(r#"href="/admin/sso-clients/radar-fiscal/edit" class="btn" data-sso-dismiss"#),
        "closing a rotated secret returns to the edit page");
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
        let sid = session_as(&state, role, &format!("someone-{}", role.as_str())).await;
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

    // When the SSO refusal budget is exceeded, the "recording limit
    // reached" marker is an `sso.` event: visible on THIS tab, where an
    // Admin looks at refusals, and selectable in its filter.
    for i in 0..300 {
        state.activity.record_limited(
            "sso.denied",
            &format!("sso.denied|u{i}|norma|not-in-group"),
            1,
            ActivityEvent::sso_denied(Some(format!("u{i}")), "norma", "not-in-group"),
        );
    }
    let mut rx = state.activity.take_receiver().unwrap();
    let mut batch = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        batch.push(ev);
    }
    assert!(batch.iter().any(|e| e.event_type.as_str() == "sso.suppressed"));
    ruscker_admin::db::user_activity::insert_batch(&db, &batch).await.unwrap();
    let (_, html) = get(state.clone(), "/admin/sso-access?event=sso.suppressed", &cookie).await;
    assert_eq!(html.matches("ti-alert-triangle audit-row-icon").count(), 1, "one marker row");

    // The general Activity page still shows everything, refusals included.
    let (_, html) = get(state.clone(), "/admin/activity", &cookie).await;
    assert!(html.contains("painel-interno") && html.contains("norma"));

    // The SSO section has its own nav entry; Credentials no longer hosts it.
    let (_, creds) = get(state.clone(), "/admin/credentials", &cookie).await;
    assert_eq!(creds.matches(r#"href="/admin/sso-access""#).count(), 1, "only the top nav links there");
    assert!(!creds.contains(r#"href="/admin/sso-clients""#));

    // Admin only.
    let sid = session_as(&state, Role::Editor, "ed").await;
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

/// The scope checkboxes on the client form: ticked ones are saved; a
/// post without them (a script, an old tab) leaves the scopes alone —
/// it must never reopen a client an Admin had restricted.
#[tokio::test]
async fn allowed_scopes_are_saved_and_survive_a_form_without_them() {
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;
    let db = state.db.clone().unwrap();
    post_page(state.clone(), "/admin/sso-clients", CREATE, &cookie).await;
    let scopes = |db: &ConfigDb| {
        let db = db.clone();
        async move { ruscker_admin::db::oidc_clients::fetch(&db, "radar-fiscal").await.unwrap().unwrap().allowed_scopes }
    };
    let all = || Some(["profile", "email", "groups", "roles", "setor"].map(String::from).to_vec());
    // Registered without the checkboxes (a script): an explicit list of
    // every scope known today — never "unrestricted".
    assert_eq!(scopes(&db).await, all());
    let (_, html) = get(state.clone(), "/admin/sso-clients/radar-fiscal/edit", &cookie).await;
    assert_eq!(html.matches(r#"name="scope_"#).count(), 5);
    let ticked = |html: &str| -> Vec<String> {
        html.split(r#"name="scope_"#)
            .skip(1)
            .filter(|chunk| chunk.split('>').next().unwrap().contains("checked"))
            .map(|chunk| chunk.split('"').next().unwrap().to_string())
            .collect()
    };
    assert_eq!(ticked(&html).len(), 5);

    // Registered through the form with two boxes ticked.
    let body = "client_id=norma&name=Norma&redirect_uris=https%3A%2F%2Fnorma.example%2Fcb&access_groups=&scopes_present=1&scope_email=on&scope_groups=on";
    let (st, _, _) = post_page(state.clone(), "/admin/sso-clients", body, &cookie).await;
    assert_eq!(st, StatusCode::OK);
    let norma = ruscker_admin::db::oidc_clients::fetch(&db, "norma").await.unwrap().unwrap();
    assert_eq!(norma.allowed_scopes, Some(vec!["email".to_string(), "groups".to_string()]));

    let base = "name=Radar&redirect_uris=https%3A%2F%2Fradar.example%2Fcb&access_groups=&enabled=on";
    let (st, _) = post(
        state.clone(),
        "/admin/sso-clients/radar-fiscal/edit",
        &format!("{base}&scopes_present=1&scope_profile=on&scope_roles=on"),
        &cookie,
    )
    .await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert_eq!(scopes(&db).await, Some(vec!["profile".to_string(), "roles".to_string()]));

    // No checkboxes in the post: untouched — and the page that comes
    // back shows the STORED boxes, not all of them, or the next Save
    // would grant everything. Same on a validation error.
    let (st, _, _) = post_page(state.clone(), "/admin/sso-clients/radar-fiscal/edit", base, &cookie).await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert_eq!(scopes(&db).await, Some(vec!["profile".to_string(), "roles".to_string()]));
    let (_, html) = get(state.clone(), "/admin/sso-clients/radar-fiscal/edit", &cookie).await;
    assert_eq!(ticked(&html), ["profile", "roles"]);
    let (_, _, html) = post_page(
        state.clone(),
        "/admin/sso-clients/radar-fiscal/edit",
        "name=Radar&redirect_uris=http%3A%2F%2Finsecure.example%2Fcb",
        &cookie,
    )
    .await;
    assert_eq!(ticked(&html), ["profile", "roles"]);
    assert_eq!(scopes(&db).await, Some(vec!["profile".to_string(), "roles".to_string()]));

    // Checkboxes present, none ticked: the id alone.
    let (st, _) = post(
        state.clone(),
        "/admin/sso-clients/radar-fiscal/edit",
        &format!("{base}&scopes_present=1"),
        &cookie,
    )
    .await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    assert_eq!(scopes(&db).await, Some(vec![]));
}

/// The section opens on Access; registering a system is a button there
/// and on the list, and the form lives on its own page.
#[tokio::test]
async fn section_opens_on_access_and_registration_has_its_own_page() {
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;
    const NEW: &str = r#"href="/admin/sso-clients/new""#;
    const FORM: &str = r#"<form method="post" action="/admin/sso-clients""#;

    let (_, access) = get(state.clone(), "/admin/sso-access", &cookie).await;
    assert!(access.contains(NEW), "register button on the Access tab");
    // The general activity page is the same template: no SSO button there.
    let (_, activity) = get(state.clone(), "/admin/activity", &cookie).await;
    assert!(!activity.contains(NEW));

    let (_, list) = get(state.clone(), "/admin/sso-clients", &cookie).await;
    assert!(list.contains(NEW) && !list.contains(FORM), "the list links to the form, it does not carry it");

    let (status, new) = get(state.clone(), "/admin/sso-clients/new", &cookie).await;
    assert_eq!(status, StatusCode::OK);
    assert!(new.contains(FORM) && new.contains(r#"name="client_id""#));

    // A refused registration comes back to the form with what was typed.
    let bad = "client_id=okid&name=Meu+Sistema&redirect_uris=http%3A%2F%2Fa.example%2Fcb";
    let (_, _, html) = post_page(state.clone(), "/admin/sso-clients", bad, &cookie).await;
    assert!(html.contains(FORM) && html.contains(r#"value="okid""#) && html.contains(r#"value="Meu Sistema""#));

    // Not open to an Editor.
    let sid = session_as(&state, Role::Editor, "someone").await;
    let (status, _) = get(state, "/admin/sso-clients/new", &format!("{COOKIE_NAME}={sid}")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

/// The logo travels in a hidden input rendered with the STORED value, so
/// the gallery (which needs scripts) is never what keeps it.
#[tokio::test]
async fn logo_field_carries_the_stored_value_without_scripts() {
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;
    let db = state.db.clone().unwrap();
    let body = format!("{CREATE}&logo=radar.svg");
    post_page(state.clone(), "/admin/sso-clients", &body, &cookie).await;
    let (_, html) = get(state.clone(), "/admin/sso-clients/radar-fiscal/edit", &cookie).await;
    assert!(html.contains(r#"<input type="hidden" name="logo" value="radar.svg""#));
    assert!(!html.contains("<select id=\"sso-logo\""));
    let c = ruscker_admin::db::oidc_clients::fetch(&db, "radar-fiscal").await.unwrap().unwrap();
    assert_eq!(c.logo.as_deref(), Some("radar.svg"));
}

/// The two address fields are saved, and a post that lacks them (a
/// script, a page loaded before they existed) keeps what is stored.
#[tokio::test]
async fn client_addresses_are_saved_and_survive_a_form_without_them() {
    let state = app_state(open_db().await).await;
    let cookie = admin_cookie(&state).await;
    let db = state.db.clone().unwrap();
    let body = format!(
        "{CREATE}&client_uri=https%3A%2F%2Fradar.example%2F&post_logout_redirect_uris=https%3A%2F%2Fradar.example%2Fbye"
    );
    post_page(state.clone(), "/admin/sso-clients", &body, &cookie).await;
    let fetch = || async { ruscker_admin::db::oidc_clients::fetch(&db, "radar-fiscal").await.unwrap().unwrap() };
    let c = fetch().await;
    assert_eq!(c.client_uri.as_deref(), Some("https://radar.example/"));
    assert_eq!(c.post_logout_redirect_uris, vec!["https://radar.example/bye".to_string()]);

    // An edit without the fields: kept.
    let (st, _, _) = post_page(
        state.clone(),
        "/admin/sso-clients/radar-fiscal/edit",
        "name=Radar&redirect_uris=https%3A%2F%2Fradar.example%2Fcb&enabled=on",
        &cookie,
    )
    .await;
    assert_eq!(st, StatusCode::SEE_OTHER);
    let c = fetch().await;
    assert_eq!(c.name, "Radar");
    assert_eq!(c.client_uri.as_deref(), Some("https://radar.example/"));
    assert_eq!(c.post_logout_redirect_uris.len(), 1);

    // An edit that empties them: cleared.
    post_page(
        state.clone(),
        "/admin/sso-clients/radar-fiscal/edit",
        "name=Radar&redirect_uris=https%3A%2F%2Fradar.example%2Fcb&enabled=on&client_uri=&post_logout_redirect_uris=",
        &cookie,
    )
    .await;
    let c = fetch().await;
    assert_eq!(c.client_uri, None);
    assert!(c.post_logout_redirect_uris.is_empty());

    // The edit page shows both fields.
    let (_, html) = get(state, "/admin/sso-clients/radar-fiscal/edit", &cookie).await;
    assert!(html.contains(r#"name="client_uri""#) && html.contains(r#"name="post_logout_redirect_uris""#));
}
