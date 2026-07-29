//! Integration tests for the role-based route guards (#101).
//!
//! These exercise the server-side enforcement, not the nav hiding:
//! we mint an opaque session for a given [`Role`] directly in the
//! shared session store, attach it as the admin cookie, and assert
//! the status code each `/admin/*` route returns.
//!
//! No DB / backend is wired, so an *allowed* route falls through to a
//! `503` (its handler needs `--db` / `--docker`) — which still proves
//! the guard let the request past. A *denied* route returns `403`
//! before the handler runs, and an *unauthenticated* request is
//! redirected to the login form. We assert on those three shapes.

use axum::body::{to_bytes, Body};
use axum::http::{Request, StatusCode};
use axum::response::Response;
use chrono::Utc;
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::{router, AppState};
use ruscker_config::{Config, Spec};
use ruscker_core::{Replica, ReplicaId, ReplicaState};
use std::net::SocketAddr;
use std::sync::Arc;
use tower::ServiceExt;

const YAML: &str = r#"
proxy:
  title: Test
  specs:
    - id: myapp
      display-name: My App
      container-image: org/app:1
"#;

fn state() -> AppState {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let config = Config::from_yaml(YAML).expect("parse config");
    let locales = ruscker_admin::i18n::Locales::load().expect("load locales");
    AppState {
        config: Arc::new(config),
        base_path: Arc::from(""),
        locales: Arc::new(locales),
        // Break-glass admin token configured.
        admin_auth: AdminAuth {
            admin: Some("admin-tok".into()),
        },
        admin_sessions: Arc::new(ruscker_admin::auth::InMemoryAdminSessionStore::default()),
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
        logout_index: Arc::new(dashmap::DashMap::new()),
        leader: Arc::new(ruscker_admin::leader::AlwaysLeader),
        metrics: ruscker_admin::metrics_cache::MetricsCache::new(),
        draining: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        spec_cache: std::sync::Arc::new(dashmap::DashMap::new()),
        identity_cache: Default::default(),
        catalog_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        access_counter: std::sync::Arc::new(ruscker_admin::access_counter::AccessCounter::default()),
        alerts: ruscker_admin::alerts::AlertSink::default(),
        activity: ruscker_admin::activity::ActivitySink::default(),
    }
}

/// Mint a live session for `role` in the state's store and return the
/// cookie header value to send it back. The store is behind an `Arc`
/// shared with the router built from the same `state`.
async fn cookie_for(state: &AppState, role: Role) -> String {
    let id = state
        .admin_sessions
        .create(role, Some("test-user".into()))
        .await;
    format!("{COOKIE_NAME}={id}")
}

async fn send(state: AppState, method: &str, uri: &str, cookie: Option<&str>) -> StatusCode {
    send_request(state, method, uri, cookie, Body::empty(), None)
        .await
        .status()
}

async fn send_request(
    state: AppState,
    method: &str,
    uri: &str,
    cookie: Option<&str>,
    body: Body,
    content_type: Option<&str>,
) -> Response {
    let app = router(state);
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        builder = builder.header("cookie", c);
    }
    if let Some(value) = content_type {
        builder = builder.header("content-type", value);
    }
    let req = builder.body(body).unwrap();
    app.oneshot(req).await.unwrap()
}

async fn response_body(response: Response) -> String {
    let bytes = to_bytes(response.into_body(), 2 * 1024 * 1024)
        .await
        .expect("read response body");
    String::from_utf8(bytes.to_vec()).expect("response body is utf-8")
}

const TIME_A_REPLICA: &str = "aaaaaaaa-1111-4111-8111-111111111111";
const TIME_B_REPLICA: &str = "bbbbbbbb-2222-4222-8222-222222222222";
const OPEN_REPLICA: &str = "cccccccc-3333-4333-8333-333333333333";

fn replica(id: &str, spec_id: &str) -> Replica {
    Replica {
        id: ReplicaId(uuid::Uuid::parse_str(id).expect("valid replica id")),
        spec_id: spec_id.to_string(),
        container_id: format!("container-{spec_id}"),
        upstream: "127.0.0.1:3838"
            .parse::<SocketAddr>()
            .expect("valid upstream"),
        state: ReplicaState::Ready,
        started_at: Utc::now(),
        sessions_active: 0,
        sessions_max: 1,
        host: None,
    }
}

fn app(yaml: &str) -> Spec {
    serde_yaml_ng::from_str(yaml).expect("parse scoped test app")
}

/// Real SQLite catalog + real opaque account session for the #990 scope
/// integration tests. `time-a` deliberately also carries `legacy-ops`:
/// the Editor shares one group and may edit it, but must not erase the
/// foreign membership that is hidden from their form.
async fn scoped_state() -> (AppState, ruscker_admin::db::ConfigDb) {
    let mut state = state();
    state.config = Arc::new(
        Config::from_yaml("proxy:\n  title: Scoped test\n  specs: []\n")
            .expect("parse empty scoped config"),
    );
    let path =
        std::env::temp_dir().join(format!("ruscker-rbac-scope-{}.db", uuid::Uuid::new_v4()));
    let pool = ruscker_admin::db::open(&path).await.expect("open scoped db");
    let db = ruscker_admin::db::ConfigDb::Sqlite(pool);
    // `db::open` seeds the product showcase. This fixture needs a closed,
    // exact three-app catalog so row/KPI assertions document scope precisely.
    for seeded in ruscker_admin::db::specs::list_all(&db)
        .await
        .expect("list seeded showcase")
    {
        ruscker_admin::db::specs::delete_one(&db, &seeded.id, Some("test-reset"))
            .await
            .expect("remove seeded showcase app");
    }
    for spec in [
        app(
            "id: time-a\n\
             display-name: Time A\n\
             container-image: org/time-a:1\n\
             access-groups: [time-a, legacy-ops]\n",
        ),
        app(
            "id: time-b\n\
             display-name: Time B\n\
             container-image: org/time-b:1\n\
             access-groups: [time-b]\n",
        ),
        app(
            "id: open-app\n\
             display-name: Open App\n\
             container-image: org/open:1\n",
        ),
    ] {
        ruscker_admin::db::specs::upsert_one(&db, &spec, Some("seed"))
            .await
            .expect("seed scoped app");
    }
    ruscker_admin::db::users::create(
        &db,
        "editor-a",
        "EditorPass9!",
        Role::Editor,
        false,
        &["time-a".to_string()],
        Some("seed"),
    )
    .await
    .expect("create scoped Editor");
    {
        let mut registry = state.replicas.write().await;
        registry.add(replica(TIME_A_REPLICA, "time-a"));
        registry.add(replica(TIME_B_REPLICA, "time-b"));
        registry.add(replica(OPEN_REPLICA, "open-app"));
    }
    state.db = Some(db.clone());
    (state, db)
}

/// Insert an account without spending argon2 time in authorization tests.
///
/// These tests mint opaque sessions directly (the established RBAC harness
/// pattern), so the password hash is never read. The row itself is real and
/// authoritative: `EditorScope` refetches it from this table on every request.
async fn seed_user(
    db: &ruscker_admin::db::ConfigDb,
    username: &str,
    role: Role,
    groups: &[&str],
) {
    let ruscker_admin::db::ConfigDb::Sqlite(pool) = db else {
        panic!("RBAC scope fixture must use SQLite");
    };
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO users
           (id, username, password_hash, role, must_change_password,
            groups, created_at, updated_at, created_by)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind(username)
    .bind("unused-test-hash")
    .bind(role.as_str())
    .bind(0_i64)
    .bind(groups.join(","))
    .bind(now)
    .bind(now)
    .bind("seed")
    .execute(pool)
    .await
    .expect("seed scoped user");
}

async fn scoped_user_state() -> (AppState, ruscker_admin::db::ConfigDb) {
    let (state, db) = scoped_state().await;
    for (username, role, groups) in [
        ("viewer-a", Role::Viewer, &["time-a"][..]),
        ("shared-user", Role::Viewer, &["time-a", "time-b"][..]),
        ("viewer-b", Role::Viewer, &["time-b"][..]),
        ("admin-a", Role::Admin, &["time-a"][..]),
    ] {
        seed_user(&db, username, role, groups).await;
    }
    (state, db)
}

async fn scoped_cookie(state: &AppState, role: Role, actor: Option<&str>) -> String {
    let id = state
        .admin_sessions
        .create(role, actor.map(str::to_string))
        .await;
    format!("{COOKIE_NAME}={id}")
}

fn metric_values(body: &str) -> Vec<&str> {
    const OPEN: &str = "<div class=\"metric__value tnum\">";
    body.match_indices(OPEN)
        .filter_map(|(start, _)| {
            let value = &body[start + OPEN.len()..];
            value.find("</div>").map(|end| value[..end].trim())
        })
        .collect()
}

// ── Viewer: no panel — portal authenticated-user role (#857) ─────────

#[tokio::test]
async fn viewer_redirected_from_dashboard_to_portal() {
    let st = state();
    let c = cookie_for(&st, Role::Viewer).await;
    // A Viewer is NOT a panel operator (#857): it signs in to unlock its
    // group's cards on the portal, so the dashboard sends it back to the
    // landing (303) instead of rendering.
    assert_eq!(
        send(st, "GET", "/admin/dashboard", Some(&c)).await,
        StatusCode::SEE_OTHER
    );
}

#[tokio::test]
async fn viewer_cannot_reach_apps_or_admin_sections() {
    for uri in [
        "/admin/specs",
        "/admin/media",
        "/admin/credentials",
        "/admin/landing",
        "/admin/blocks",
        "/admin/audit",
        "/admin/logs",
        "/admin/logs/poll?cursor=0",
        "/admin/logs/stream",
    ] {
        let st = state();
        let c = cookie_for(&st, Role::Viewer).await;
        assert_eq!(
            send(st, "GET", uri, Some(&c)).await,
            StatusCode::FORBIDDEN,
            "viewer must be 403 on {uri}"
        );
    }
}

#[tokio::test]
async fn viewer_cannot_perform_dashboard_actions() {
    let st = state();
    let c = cookie_for(&st, Role::Viewer).await;
    let uri = "/admin/dashboard/replicas/11111111-2222-3333-4444-555555555555/stop";
    assert_eq!(
        send(st, "POST", uri, Some(&c)).await,
        StatusCode::FORBIDDEN,
        "viewer is read-only on the dashboard"
    );
}

// ── Editor: apps + media + dashboard actions, but not admin-only ────

#[tokio::test]
async fn editor_passes_guard_on_apps_and_media() {
    // Guard lets the request through; the handler then 503s because
    // no DB is attached. The point is it's NOT a 403.
    for uri in ["/admin/specs", "/admin/media"] {
        let st = state();
        let c = cookie_for(&st, Role::Editor).await;
        let status = send(st, "GET", uri, Some(&c)).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "editor allowed on {uri}");
        assert_eq!(
            status,
            StatusCode::SERVICE_UNAVAILABLE,
            "guard passed, handler reached (needs --db) on {uri}"
        );
    }
}

#[tokio::test]
async fn editor_can_perform_dashboard_actions() {
    // RequireEditor passes; no backend ⇒ 503, but crucially not 403.
    let st = state();
    let c = cookie_for(&st, Role::Editor).await;
    let uri = "/admin/dashboard/replicas/11111111-2222-3333-4444-555555555555/stop";
    let status = send(st, "POST", uri, Some(&c)).await;
    assert_ne!(status, StatusCode::FORBIDDEN, "editor may stop/restart");
}

#[tokio::test]
async fn editor_cannot_reach_admin_only_sections() {
    for uri in [
        "/admin/credentials",
        "/admin/landing",
        "/admin/blocks",
        "/admin/audit",
        "/admin/logs",
        "/admin/logs/poll?cursor=0",
        "/admin/logs/stream",
    ] {
        let st = state();
        let c = cookie_for(&st, Role::Editor).await;
        assert_eq!(
            send(st, "GET", uri, Some(&c)).await,
            StatusCode::FORBIDDEN,
            "editor must be 403 on admin-only {uri}"
        );
    }
}

// ── Admin: everything ───────────────────────────────────────────────

#[tokio::test]
async fn admin_passes_guard_everywhere() {
    // Each handler 503s for lack of db/backend, but never 403/redirect.
    for uri in [
        "/admin/dashboard",
        "/admin/specs",
        "/admin/media",
        "/admin/credentials",
        "/admin/audit",
    ] {
        let st = state();
        let c = cookie_for(&st, Role::Admin).await;
        assert_ne!(
            send(st, "GET", uri, Some(&c)).await,
            StatusCode::FORBIDDEN,
            "admin is never forbidden ({uri})"
        );
    }
}

// ── Unauthenticated: redirect to login, never 403 ───────────────────

#[tokio::test]
async fn unauthenticated_is_redirected_to_login() {
    for uri in [
        "/admin/specs",
        "/admin/logs/poll?cursor=0",
        "/admin/logs/stream",
    ] {
        let status = send(state(), "GET", uri, None).await;
        assert!(
            status.is_redirection(),
            "no session ⇒ redirect to login on {uri}, got {status}"
        );
    }
}

// ── Inline image upload (#213) — same RequireEditor guard ───────────

#[tokio::test]
async fn inline_upload_is_editor_gated() {
    // Viewer: forbidden before the handler runs.
    let st = state();
    let c = cookie_for(&st, Role::Viewer).await;
    assert_eq!(
        send(st, "POST", "/admin/media/upload-inline", Some(&c)).await,
        StatusCode::FORBIDDEN,
        "viewer cannot upload images"
    );
    // Editor: guard passes (empty body ⇒ a 4xx/503 from the handler,
    // never 403).
    let st = state();
    let c = cookie_for(&st, Role::Editor).await;
    assert_ne!(
        send(st, "POST", "/admin/media/upload-inline", Some(&c)).await,
        StatusCode::FORBIDDEN,
        "editor may upload inline"
    );
    // Anonymous: redirected to login, never a public write endpoint.
    let status = send(state(), "POST", "/admin/media/upload-inline", None).await;
    assert!(
        status.is_redirection(),
        "anon upload ⇒ redirect to login, got {status}"
    );
}

// ── Image pull: side effect on POST, follow-only GET (#720 P2) ──────

#[tokio::test]
async fn image_pull_start_is_post_only() {
    // The old side-effecting `GET /admin/specs/image-pull` is gone: the
    // path now only accepts POST, so a GET is 405 (no pull on GET).
    assert_eq!(
        send(state(), "GET", "/admin/specs/image-pull", None).await,
        StatusCode::METHOD_NOT_ALLOWED,
        "pull must not be startable via GET"
    );
}

#[tokio::test]
async fn image_pull_is_editor_gated() {
    // Viewer is forbidden on both the start (POST) and the follow (GET).
    let st = state();
    let c = cookie_for(&st, Role::Viewer).await;
    assert_eq!(
        send(st, "POST", "/admin/specs/image-pull", Some(&c)).await,
        StatusCode::FORBIDDEN,
    );
    let st = state();
    let c = cookie_for(&st, Role::Viewer).await;
    assert_eq!(
        send(st, "GET", "/admin/specs/image-pull/events?job=bogus", Some(&c)).await,
        StatusCode::FORBIDDEN,
    );
}

#[tokio::test]
async fn following_an_unknown_pull_job_is_404_for_editor() {
    // Editor passes the guard; the follow GET is side-effect-free and
    // returns 404 for a token that was never issued (or already drained).
    let st = state();
    let c = cookie_for(&st, Role::Editor).await;
    assert_eq!(
        send(st, "GET", "/admin/specs/image-pull/events?job=bogus", Some(&c)).await,
        StatusCode::NOT_FOUND,
        "unknown pull token ⇒ 404, never a side effect"
    );
}

// ── Editor group scope over applications (#990 slice 1) ─────────────

#[tokio::test]
async fn scoped_editor_lists_only_shared_and_open_apps_with_matching_kpis() {
    let (state, _db) = scoped_state().await;
    let cookie = scoped_cookie(&state, Role::Editor, Some("editor-a")).await;

    let response = send_request(
        state.clone(),
        "GET",
        "/admin/specs",
        Some(&cookie),
        Body::empty(),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_body(response).await;
    assert!(body.contains("data-id=\"time-a\""), "shared app is listed");
    assert!(body.contains("data-id=\"open-app\""), "open app is global");
    assert!(
        !body.contains("data-id=\"time-b\""),
        "foreign-team app stays hidden"
    );
    let listed = body.matches("<tr data-kind=").count();
    assert_eq!(listed, 2, "exactly the two authorized app rows");
    assert_eq!(
        metric_values(&body).first().copied(),
        Some("2"),
        "Total KPI must equal the filtered list"
    );

    let response = send_request(
        state,
        "GET",
        "/admin/dashboard",
        Some(&cookie),
        Body::empty(),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let dashboard = response_body(response).await;
    assert!(dashboard.contains(TIME_A_REPLICA));
    assert!(dashboard.contains(OPEN_REPLICA));
    assert!(
        !dashboard.contains(TIME_B_REPLICA),
        "foreign replica must not reach the dashboard grid"
    );
    assert!(
        dashboard.contains(
            "<div class=\"metric__value\" data-metric=\"containers\">2</div>"
        ),
        "container KPI must equal the filtered replica grid"
    );
}

#[tokio::test]
async fn scoped_editor_gets_404_on_every_foreign_app_or_replica_id_route() {
    let (state, _db) = scoped_state().await;
    let cookie = scoped_cookie(&state, Role::Editor, Some("editor-a")).await;

    // Every current app route carrying `{id}` is enumerated here. Filtering
    // the list alone is not authorization: a typed/guessed URL must fail.
    for (method, uri) in [
        ("GET", "/admin/specs/time-b/edit"),
        ("GET", "/admin/specs/time-b/duplicate"),
        ("POST", "/admin/specs/time-b"),
        ("POST", "/admin/specs/time-b/delete"),
        ("POST", "/admin/specs/time-b/featured/toggle"),
        ("POST", "/admin/specs/time-b/state/toggle"),
        ("POST", "/admin/specs/time-b/repull"),
    ] {
        let response = send_request(
            state.clone(),
            method,
            uri,
            Some(&cookie),
            Body::empty(),
            (method == "POST").then_some("application/x-www-form-urlencoded"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "foreign app route must be 404: {method} {uri}"
        );
    }

    // Replica ids are just another path to the owning spec. Logs, live logs,
    // stop and restart all resolve the effective Spec before backend access.
    for (method, uri) in [
        (
            "GET",
            "/admin/dashboard/logs/bbbbbbbb-2222-4222-8222-222222222222",
        ),
        (
            "GET",
            "/admin/dashboard/logs/bbbbbbbb-2222-4222-8222-222222222222/stream",
        ),
        (
            "POST",
            "/admin/dashboard/replicas/bbbbbbbb-2222-4222-8222-222222222222/stop",
        ),
        (
            "POST",
            "/admin/dashboard/replicas/bbbbbbbb-2222-4222-8222-222222222222/restart",
        ),
    ] {
        let status = send(state.clone(), method, uri, Some(&cookie)).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "foreign replica route must be 404: {method} {uri}"
        );
    }
}

#[tokio::test]
async fn admin_remains_unscoped_for_foreign_apps_and_replicas() {
    let (state, db) = scoped_state().await;
    let cookie = scoped_cookie(&state, Role::Admin, Some("admin")).await;

    let response = send_request(
        state.clone(),
        "GET",
        "/admin/specs",
        Some(&cookie),
        Body::empty(),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_body(response).await;
    for id in ["time-a", "time-b", "open-app"] {
        assert!(body.contains(&format!("data-id=\"{id}\"")), "Admin sees {id}");
    }
    assert_eq!(metric_values(&body).first().copied(), Some("3"));

    assert_eq!(
        send(
            state.clone(),
            "GET",
            "/admin/specs/time-b/edit",
            Some(&cookie),
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        send(
            state.clone(),
            "GET",
            "/admin/specs/time-b/duplicate",
            Some(&cookie),
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        send(
            state.clone(),
            "POST",
            "/admin/specs/time-b/featured/toggle",
            Some(&cookie),
        )
        .await,
        StatusCode::OK
    );
    assert_eq!(
        send(
            state.clone(),
            "POST",
            "/admin/specs/time-b/state/toggle",
            Some(&cookie),
        )
        .await,
        StatusCode::OK
    );
    for uri in [
        "/admin/specs/time-b/repull",
        "/admin/dashboard/replicas/bbbbbbbb-2222-4222-8222-222222222222/stop",
        "/admin/dashboard/replicas/bbbbbbbb-2222-4222-8222-222222222222/restart",
    ] {
        assert_eq!(
            send(state.clone(), "POST", uri, Some(&cookie)).await,
            StatusCode::SERVICE_UNAVAILABLE,
            "Admin passes scope and reaches the intentionally absent backend: {uri}"
        );
    }

    let response = send_request(
        state.clone(),
        "POST",
        "/admin/specs/time-b",
        Some(&cookie),
        Body::from(
            "display_name=Time+B+admin&display_type=app&state=active&\
             container_image=org%2Ftime-b%3A2&inject_base_href=on&access_groups=time-b",
        ),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        ruscker_admin::db::specs::fetch_one(&db, "time-b")
            .await
            .unwrap()
            .unwrap()
            .display_name
            .as_deref(),
        Some("Time B admin")
    );

    assert_eq!(
        send(
            state,
            "POST",
            "/admin/specs/time-b/delete",
            Some(&cookie),
        )
        .await,
        StatusCode::SEE_OTHER,
        "Admin can delete the foreign-team app"
    );
    assert!(
        ruscker_admin::db::specs::fetch_one(&db, "time-b")
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn scoped_editor_cannot_create_restricted_app_with_foreign_group() {
    let (state, db) = scoped_state().await;
    let cookie = scoped_cookie(&state, Role::Editor, Some("editor-a")).await;
    let response = send_request(
        state,
        "POST",
        "/admin/specs",
        Some(&cookie),
        Body::from(
            "id=foreign-new&display_name=Foreign+new&display_type=app&state=active&\
             container_image=org%2Fforeign%3A1&inject_base_href=on&access_groups=time-b",
        ),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    let body = response_body(response).await;
    assert!(
        body.contains("fora do seu escopo de Editor"),
        "localized scope validation must explain the rejection"
    );
    assert!(
        ruscker_admin::db::specs::fetch_one(&db, "foreign-new")
            .await
            .unwrap()
            .is_none(),
        "rejected app must not be persisted"
    );
}

#[tokio::test]
async fn scoped_editor_cannot_assign_foreign_group_and_edit_preserves_it() {
    let (state, db) = scoped_state().await;
    let cookie = scoped_cookie(&state, Role::Editor, Some("editor-a")).await;

    let edit = send_request(
        state.clone(),
        "GET",
        "/admin/specs/time-a/edit",
        Some(&cookie),
        Body::empty(),
        None,
    )
    .await;
    assert_eq!(edit.status(), StatusCode::OK);
    let edit_body = response_body(edit).await;
    assert!(
        !edit_body.contains("legacy-ops"),
        "foreign memberships are preserved server-side, not exposed as editable pills"
    );

    let rejected = send_request(
        state.clone(),
        "POST",
        "/admin/specs/time-a",
        Some(&cookie),
        Body::from(
            "display_name=Time+A&display_type=app&state=active&\
             container_image=org%2Ftime-a%3A1&inject_base_href=on&access_groups=time-b",
        ),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(rejected.status(), StatusCode::UNPROCESSABLE_ENTITY);

    let saved = send_request(
        state,
        "POST",
        "/admin/specs/time-a",
        Some(&cookie),
        Body::from(
            "display_name=Time+A+edited&display_type=app&state=active&\
             container_image=org%2Ftime-a%3A2&inject_base_href=on&access_groups=time-a",
        ),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::SEE_OTHER);
    let spec = ruscker_admin::db::specs::fetch_one(&db, "time-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(spec.display_name.as_deref(), Some("Time A edited"));
    assert_eq!(
        spec.access_groups.as_deref(),
        Some(&["legacy-ops".to_string(), "time-a".to_string()][..]),
        "the out-of-scope group survives the Editor's replacement save"
    );
}

// ── Editor group scope over users (#990 slice 2) ───────────────────

#[tokio::test]
async fn scoped_editor_lists_only_shared_non_admin_users_and_matching_kpis() {
    let (state, _db) = scoped_user_state().await;
    let cookie = scoped_cookie(&state, Role::Editor, Some("editor-a")).await;
    let response = send_request(
        state,
        "GET",
        "/admin/users",
        Some(&cookie),
        Body::empty(),
        None,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_body(response).await;

    for username in ["editor-a", "viewer-a", "shared-user"] {
        assert!(body.contains(username), "shared user {username} is listed");
    }
    assert!(!body.contains("viewer-b"), "foreign user stays hidden");
    assert!(
        !body.contains("admin-a"),
        "Admin stays hidden even with a shared group"
    );
    assert!(
        body.contains(r#"href="/admin/users""#),
        "the Users nav tab is visible to Editors"
    );
    assert_eq!(
        metric_values(&body).first().copied(),
        Some("3"),
        "Total KPI equals the scoped row set"
    );
    assert!(body.contains(r#"data-users-total="3""#));
    assert!(
        !body.contains(r#"href="/admin/users/editor-a/edit""#),
        "the Editor's own account has no edit affordance"
    );
    assert!(
        !body.contains(r#"value="admin""#),
        "the create-role selector does not offer Admin"
    );
    assert!(
        !body.contains(r#"action="/admin/users/import""#),
        "bulk import is not rendered for Editors"
    );
    assert!(
        !body.contains("/delete\""),
        "account deletion is not rendered for Editors"
    );
}

#[tokio::test]
async fn scoped_editor_gets_404_on_every_foreign_user_route() {
    let (state, _db) = scoped_user_state().await;
    let cookie = scoped_cookie(&state, Role::Editor, Some("editor-a")).await;

    // Every username route opened to Editors is enumerated. The authoritative
    // target row is checked before parsing form data, so malformed crafted
    // POSTs cannot turn validation differences into an existence oracle.
    for (method, uri) in [
        ("GET", "/admin/users/viewer-b/edit"),
        ("POST", "/admin/users/viewer-b/edit"),
        ("POST", "/admin/users/viewer-b/role"),
        ("POST", "/admin/users/viewer-b/groups"),
        ("POST", "/admin/users/viewer-b/profile"),
        ("POST", "/admin/users/viewer-b/password"),
    ] {
        let response = send_request(
            state.clone(),
            method,
            uri,
            Some(&cookie),
            Body::empty(),
            (method == "POST").then_some("application/x-www-form-urlencoded"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "foreign user route must be 404: {method} {uri}"
        );
    }
}

#[tokio::test]
async fn scoped_editor_creation_enforces_role_groups_and_audit_actor() {
    let (state, db) = scoped_user_state().await;
    let cookie = scoped_cookie(&state, Role::Editor, Some("editor-a")).await;

    for (username, role, groups, flash) in [
        ("bad-admin", "admin", "time-a", "scope-role"),
        ("bad-group", "viewer", "time-b", "scope-groups"),
        ("no-group", "viewer", "", "group-required"),
    ] {
        let body = format!(
            "username={username}&password=Valid%21Pass9&role={role}&groups={groups}"
        );
        let response = send_request(
            state.clone(),
            "POST",
            "/admin/users",
            Some(&cookie),
            Body::from(body),
            Some("application/x-www-form-urlencoded"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers().get("location").unwrap(),
            &format!("/admin/users?flash={flash}")
        );
        assert!(
            ruscker_admin::db::users::fetch(&db, username)
                .await
                .unwrap()
                .is_none(),
            "rejected account {username} must not be persisted"
        );
    }

    let localized = send_request(
        state.clone(),
        "GET",
        "/admin/users?flash=group-required",
        Some(&cookie),
        Body::empty(),
        None,
    )
    .await;
    let localized = response_body(localized).await;
    assert!(
        localized.contains("pelo menos um dos seus grupos"),
        "the validation error is localized and actionable"
    );

    let created = send_request(
        state,
        "POST",
        "/admin/users",
        Some(&cookie),
        Body::from(
            "username=new-teammate&password=Valid%21Pass9&role=editor&groups=time-a",
        ),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(created.status(), StatusCode::SEE_OTHER);
    let user = ruscker_admin::db::users::fetch(&db, "new-teammate")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.role, Role::Editor);
    assert_eq!(user.groups, vec!["time-a"]);

    let ruscker_admin::db::ConfigDb::Sqlite(pool) = &db else {
        panic!("RBAC scope fixture must use SQLite");
    };
    let actors: Vec<(Option<String>,)> = sqlx::query_as(
        "SELECT actor FROM audit_log
          WHERE action = 'user.create' AND target = 'user:new-teammate'",
    )
    .fetch_all(pool)
    .await
    .unwrap();
    assert_eq!(actors, vec![(Some("editor-a".to_string()),)]);
}

#[tokio::test]
async fn scoped_editor_cannot_delete_import_or_reset_mfa() {
    let (state, db) = scoped_user_state().await;
    let cookie = scoped_cookie(&state, Role::Editor, Some("editor-a")).await;

    for uri in [
        "/admin/users/viewer-a/delete",
        "/admin/users/viewer-a/mfa/reset",
        "/admin/users/import",
        "/admin/users/import/confirm",
    ] {
        assert_eq!(
            send(state.clone(), "POST", uri, Some(&cookie)).await,
            StatusCode::FORBIDDEN,
            "Admin-only operation must reject Editor: {uri}"
        );
    }
    assert!(
        ruscker_admin::db::users::fetch(&db, "viewer-a")
            .await
            .unwrap()
            .is_some(),
        "the forbidden delete leaves the account intact"
    );
}

#[tokio::test]
async fn scoped_editor_edit_preserves_foreign_group_and_can_reset_password() {
    let (state, db) = scoped_user_state().await;
    let cookie = scoped_cookie(&state, Role::Editor, Some("editor-a")).await;

    let edit = send_request(
        state.clone(),
        "GET",
        "/admin/users/shared-user/edit",
        Some(&cookie),
        Body::empty(),
        None,
    )
    .await;
    assert_eq!(edit.status(), StatusCode::OK);
    let edit = response_body(edit).await;
    assert!(edit.contains(r#"value="time-a""#));
    assert!(
        edit.contains(r#"data-readonly-group="time-b""#),
        "foreign membership is explained as read-only"
    );

    let saved = send_request(
        state.clone(),
        "POST",
        "/admin/users/shared-user/edit",
        Some(&cookie),
        Body::from(
            "role=editor&groups=time-a&setor=Helpdesk&email=shared%40example.com&celular=",
        ),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(saved.status(), StatusCode::SEE_OTHER);
    let user = ruscker_admin::db::users::fetch(&db, "shared-user")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.role, Role::Editor);
    assert_eq!(
        user.groups,
        vec!["time-a", "time-b"],
        "out-of-scope group survives the replacement update"
    );

    let reset = send_request(
        state,
        "POST",
        "/admin/users/viewer-a/password",
        Some(&cookie),
        Body::from("password=Helpdesk%21Pass9"),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(reset.status(), StatusCode::SEE_OTHER);
    assert!(
        ruscker_admin::db::users::verify_login(&db, "viewer-a", "Helpdesk!Pass9")
            .await
            .unwrap()
            .is_some(),
        "Editor helpdesk reset writes the new password"
    );

    let ruscker_admin::db::ConfigDb::Sqlite(pool) = &db else {
        panic!("RBAC scope fixture must use SQLite");
    };
    let wrong_actor: (i64,) = sqlx::query_as(
        "SELECT COUNT(*) FROM audit_log
          WHERE action IN ('user.update', 'user.password')
            AND target IN ('user:shared-user', 'user:viewer-a')
            AND actor <> 'editor-a'",
    )
    .fetch_one(pool)
    .await
    .unwrap();
    assert_eq!(wrong_actor.0, 0, "Editor is the audit actor on mutations");
}

#[tokio::test]
async fn scoped_editor_cannot_change_own_role_groups_or_password() {
    let (state, db) = scoped_user_state().await;
    let cookie = scoped_cookie(&state, Role::Editor, Some("editor-a")).await;

    for (uri, body) in [
        ("/admin/users/editor-a/role", "role=admin"),
        (
            "/admin/users/editor-a/groups",
            "groups=time-a%2Ctime-b",
        ),
        (
            "/admin/users/editor-a/edit",
            "role=admin&groups=time-a%2Ctime-b&setor=&email=&celular=",
        ),
        (
            "/admin/users/editor-a/password",
            "password=Escalate%21Pass9",
        ),
    ] {
        let response = send_request(
            state.clone(),
            "POST",
            uri,
            Some(&cookie),
            Body::from(body),
            Some("application/x-www-form-urlencoded"),
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::SEE_OTHER,
            "self mutation is rejected with a localized flash: {uri}"
        );
        assert!(
            response
                .headers()
                .get("location")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("flash=self-edit")
        );
    }

    let editor = ruscker_admin::db::users::fetch(&db, "editor-a")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(editor.role, Role::Editor);
    assert_eq!(editor.groups, vec!["time-a"]);
}

/// The admin-exclusion boundary must not depend on the stored casing of
/// `role`. That column is plain TEXT with no CHECK — it is lowercase only
/// because every write goes through `Role::as_str`, and #934 (OIDC/LDAP)
/// will add another writer. A row stored as `Admin` must still be invisible
/// to a scoped Editor.
#[tokio::test]
async fn scoped_editor_never_sees_an_admin_stored_with_odd_casing() {
    let (state, db) = scoped_user_state().await;
    let ruscker_admin::db::ConfigDb::Sqlite(pool) = &db else {
        panic!("fixture must use SQLite");
    };
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO users
           (id, username, password_hash, role, must_change_password,
            groups, created_at, updated_at, created_by)
         VALUES (?, ?, ?, 'Admin', 0, 'time-a', ?, ?, NULL)",
    )
    .bind(uuid::Uuid::new_v4().to_string())
    .bind("shouty-admin")
    .bind("unused-test-hash")
    .bind(now)
    .bind(now)
    .execute(pool)
    .await
    .unwrap();

    let cookie = scoped_cookie(&state, Role::Editor, Some("editor-a")).await;
    let response = send_request(
        state.clone(),
        "GET",
        "/admin/users",
        Some(&cookie),
        Body::empty(),
        None,
    )
    .await;
    let body = String::from_utf8(
        axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap()
            .to_vec(),
    )
    .unwrap();
    assert!(
        !body.contains("shouty-admin"),
        "an admin row sharing the Editor's group must stay hidden regardless of casing"
    );
}

#[tokio::test]
async fn scoped_user_count_search_and_pagination_share_one_sql_filter() {
    let (state, db) = scoped_user_state().await;
    // The fixture starts with three visible non-Admin users. Add 49 visible
    // and 60 newer foreign rows: post-page filtering would make page 1 empty,
    // while the SQL-scoped implementation still yields 50 + 2 visible rows.
    for i in 0..49 {
        seed_user(&db, &format!("team-a-{i:02}"), Role::Viewer, &["time-a"]).await;
    }
    for i in 0..60 {
        seed_user(&db, &format!("team-b-{i:02}"), Role::Viewer, &["time-b"]).await;
    }
    let cookie = scoped_cookie(&state, Role::Editor, Some("editor-a")).await;

    let page_one = send_request(
        state.clone(),
        "GET",
        "/admin/users",
        Some(&cookie),
        Body::empty(),
        None,
    )
    .await;
    let page_one = response_body(page_one).await;
    assert_eq!(page_one.matches("class=\"user-cell\"").count(), 50);
    assert!(page_one.contains(r#"data-users-total="52""#));
    assert_eq!(metric_values(&page_one).first().copied(), Some("52"));

    let page_two = send_request(
        state.clone(),
        "GET",
        "/admin/users?page=2",
        Some(&cookie),
        Body::empty(),
        None,
    )
    .await;
    let page_two = response_body(page_two).await;
    assert_eq!(page_two.matches("class=\"user-cell\"").count(), 2);
    assert!(page_two.contains(r#"data-users-page="2""#));
    assert!(page_two.contains(r#"data-users-total="52""#));

    let foreign_search = send_request(
        state,
        "GET",
        "/admin/users?q=viewer-b",
        Some(&cookie),
        Body::empty(),
        None,
    )
    .await;
    let foreign_search = response_body(foreign_search).await;
    assert!(foreign_search.contains(r#"data-users-total="0""#));
    assert_eq!(foreign_search.matches("class=\"user-cell\"").count(), 0);
}

#[tokio::test]
async fn admin_remains_unscoped_for_all_user_operations() {
    let (state, db) = scoped_user_state().await;
    let cookie = scoped_cookie(&state, Role::Admin, Some("admin-op")).await;

    let list = send_request(
        state.clone(),
        "GET",
        "/admin/users",
        Some(&cookie),
        Body::empty(),
        None,
    )
    .await;
    let list = response_body(list).await;
    assert!(list.contains("viewer-b"));
    assert!(list.contains("admin-a"));
    assert!(list.contains(r#"value="admin""#));
    assert!(list.contains(r#"action="/admin/users/import""#));
    assert!(list.contains("/delete\""));

    assert_eq!(
        send(
            state.clone(),
            "GET",
            "/admin/users/viewer-b/edit",
            Some(&cookie),
        )
        .await,
        StatusCode::OK
    );
    let created = send_request(
        state.clone(),
        "POST",
        "/admin/users",
        Some(&cookie),
        Body::from(
            "username=delegated-admin&password=Admin%21Pass9&role=admin&groups=time-b",
        ),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(created.status(), StatusCode::SEE_OTHER);
    assert_eq!(
        ruscker_admin::db::users::fetch(&db, "delegated-admin")
            .await
            .unwrap()
            .unwrap()
            .role,
        Role::Admin
    );

    assert_eq!(
        send(
            state.clone(),
            "POST",
            "/admin/users/viewer-b/mfa/reset",
            Some(&cookie),
        )
        .await,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        send(
            state.clone(),
            "POST",
            "/admin/users/viewer-b/delete",
            Some(&cookie),
        )
        .await,
        StatusCode::SEE_OTHER
    );
    assert!(
        ruscker_admin::db::users::fetch(&db, "viewer-b")
            .await
            .unwrap()
            .is_none()
    );

    let boundary = "RUSCKER-CSV-BOUNDARY";
    let multipart = format!(
        "--{boundary}\r\n\
         Content-Disposition: form-data; name=\"file\"; filename=\"users.csv\"\r\n\
         Content-Type: text/csv\r\n\r\n\
         username,role,password,groups\r\n\
         csv-user,viewer,Csv!Pass12,time-b\r\n\
         --{boundary}--\r\n"
    );
    let preview = send_request(
        state.clone(),
        "POST",
        "/admin/users/import",
        Some(&cookie),
        Body::from(multipart),
        Some(&format!("multipart/form-data; boundary={boundary}")),
    )
    .await;
    assert_eq!(preview.status(), StatusCode::OK);

    let confirm = send_request(
        state,
        "POST",
        "/admin/users/import/confirm",
        Some(&cookie),
        Body::from(
            "raw_csv=username%2Crole%2Cpassword%2Cgroups%0Acsv-user%2Cviewer%2CCsv%21Pass12%2Ctime-b%0A",
        ),
        Some("application/x-www-form-urlencoded"),
    )
    .await;
    assert_eq!(confirm.status(), StatusCode::SEE_OTHER);
    assert!(
        ruscker_admin::db::users::fetch(&db, "csv-user")
            .await
            .unwrap()
            .is_some(),
        "Admin bulk import remains functional"
    );
}
