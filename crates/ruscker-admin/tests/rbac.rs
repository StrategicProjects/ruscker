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

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
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
    let app = router(state);
    let mut builder = Request::builder().method(method).uri(uri);
    if let Some(c) = cookie {
        builder = builder.header("cookie", c);
    }
    let req = builder.body(Body::empty()).unwrap();
    app.oneshot(req).await.unwrap().status()
}

// ── Viewer: dashboard only ──────────────────────────────────────────

#[tokio::test]
async fn viewer_can_view_dashboard() {
    let st = state();
    let c = cookie_for(&st, Role::Viewer).await;
    // No DB needed for the dashboard render, so this is a clean 200.
    assert_eq!(
        send(st, "GET", "/admin/dashboard", Some(&c)).await,
        StatusCode::OK
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
    let status = send(state(), "GET", "/admin/specs", None).await;
    assert!(
        status.is_redirection(),
        "no session ⇒ redirect to login, got {status}"
    );
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
