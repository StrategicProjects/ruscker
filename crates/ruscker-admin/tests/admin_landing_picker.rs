//! #451: the admin landing editor's logos field uses the shared image
//! gallery picker (the same one the spec form uses), seeded with the
//! media library filenames — including the built-in logos seeded into
//! the library (#433). This renders `/admin/landing` as an admin and
//! asserts the picker is wired and carries data.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ruscker_admin::auth::{AdminAuth, Role, COOKIE_NAME};
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::sync::Arc;
use tower::ServiceExt;

const YAML: &str = "proxy:\n  title: Test\n  specs: []\n";

async fn state_with_db() -> AppState {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let path = std::env::temp_dir().join(format!("ruscker-landing-{}.db", uuid::Uuid::new_v4()));
    // `open` runs migrations + seeds the built-in logos into the images
    // table (#433), so the logo picker has a non-empty gallery.
    let pool = ruscker_admin::db::open(&path).await.expect("open db");
    let config = Config::from_yaml(YAML).expect("parse config");
    let locales = ruscker_admin::i18n::Locales::load().expect("load locales");
    AppState {
        config: Arc::new(config),
        base_path: Arc::from(""),
        locales: Arc::new(locales),
        admin_auth: AdminAuth::with_token("break-glass-tok"),
        admin_sessions: Arc::new(ruscker_admin::auth::InMemoryAdminSessionStore::default()),
        log_buffer: None,
        login_limiter: Arc::new(ruscker_admin::auth::LoginRateLimiter::default_policy()),
        api_limiter: Arc::new(ruscker_admin::ratelimit::ApiRateLimiter::new()),
        db: Some(ruscker_admin::db::ConfigDb::Sqlite(pool)),
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
        catalog_cache: std::sync::Arc::new(tokio::sync::RwLock::new(None)),
        access_counter: std::sync::Arc::new(ruscker_admin::access_counter::AccessCounter::default()),
    }
}

#[tokio::test]
async fn landing_logos_use_the_shared_gallery_picker() {
    let state = state_with_db().await;
    let sid = state.admin_sessions.create(Role::Admin, None).await;
    let cookie = format!("{COOKIE_NAME}={sid}");
    let app = router(state);
    let resp = app
        .oneshot(
            Request::builder()
                .uri("/admin/landing")
                .header("cookie", cookie)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), 1 << 20)
        .await
        .unwrap();
    let body = std::str::from_utf8(&bytes).unwrap();

    // The shared modal + per-row "Choose image" wiring is present.
    assert!(
        body.contains("image-picker-overlay"),
        "shared picker modal missing"
    );
    assert!(
        body.contains("openImagePicker"),
        "per-row picker trigger missing"
    );
    // The Alpine factory gets a 2nd argument (the media-library list),
    // and the built-in logos seeded into the library show up there — so
    // the gallery is actually populated.
    assert!(
        body.contains("ruscker-mark.svg"),
        "seeded built-in logos missing from the picker gallery"
    );
    // The old free-text-only datalist is gone.
    assert!(
        !body.contains("builtin-logo-list"),
        "the datalist should be replaced by the gallery picker"
    );
}
