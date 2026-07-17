//! Live WebSocket e2e against REAL Shiny and Streamlit containers
//! (#929). The URL-rewriting + WS arc was historically validated
//! against Jupyter and mocks; the value proposition is Shiny/Streamlit,
//! so this exercises exactly that, end to end:
//!
//! 1. the proxy cold-spawns the demo container on first request,
//! 2. the served HTML carries the injected `<base href="/app/{id}/">`
//!    (the rewrite arc),
//! 3. a WebSocket upgrade through `/app/{id}/…` reaches the app and
//!    frames flow app → client through `ws::pump` (real reactivity
//!    endpoints: Shiny's `websocket/`, Streamlit's `_stcore/stream`),
//! 4. a second connect (a browser refresh) works while the first is
//!    open, and the client can close cleanly.
//!
//! **Gated** behind the `ws-e2e` feature — it needs a Docker daemon and
//! pulls ~1 GB of demo images on first run (`openanalytics/
//! shinyproxy-demo` + `…-streamlit-demo`, forced `linux/amd64`; fine
//! under emulation on arm64, just slower). Run with:
//!
//! ```sh
//! cargo test -p ruscker-admin --features ws-e2e --test ws_e2e -- --nocapture
//! ```
//!
//! Containers are labelled by the backend; a drop guard `docker rm -f`s
//! them by spec label even when an assertion panics.
#![cfg(feature = "ws-e2e")]

use futures_util::{SinkExt, StreamExt};
use ruscker_admin::auth::AdminAuth;
use ruscker_admin::db::ConfigDb;
use ruscker_admin::i18n::Locales;
use ruscker_admin::{router, AppState};
use ruscker_config::Config;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite::{client::IntoClientRequest, Message};

const SHINY_SPEC: &str = "e2e-shiny";
const STREAMLIT_SPEC: &str = "e2e-streamlit";
const SHINY_IMAGE: &str = "openanalytics/shinyproxy-demo:latest";
const STREAMLIT_IMAGE: &str = "openanalytics/shinyproxy-streamlit-demo:latest";

/// First proxied request may include the whole container boot (image
/// already pulled by then) — keep it generous, emulation is slow.
const FIRST_LOAD_TIMEOUT: Duration = Duration::from_secs(180);
const WS_FRAME_TIMEOUT: Duration = Duration::from_secs(30);

fn config() -> Config {
    let yaml = format!(
        r#"
proxy:
  title: ws-e2e
  port: 8080
  specs:
    - id: {SHINY_SPEC}
      display-name: E2E Shiny
      description: ws-e2e
      container-image: {SHINY_IMAGE}
      platform: linux/amd64
    - id: {STREAMLIT_SPEC}
      display-name: E2E Streamlit
      description: ws-e2e
      type: streamlit
      container-image: {STREAMLIT_IMAGE}
      platform: linux/amd64
"#
    );
    Config::from_yaml(&yaml).expect("parse e2e config")
}

/// Removes the spec's containers when the test ends, pass or panic.
/// `docker` CLI (blocking) because `Drop` can't await.
struct Reaper(&'static str);
impl Drop for Reaper {
    fn drop(&mut self) {
        let ids = std::process::Command::new("docker")
            .args(["ps", "-aq", "--filter", &format!("label=ruscker.spec_id={}", self.0)])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        for id in ids.split_whitespace() {
            let _ = std::process::Command::new("docker").args(["rm", "-f", id]).output();
        }
    }
}

/// Pre-pull so the proxied first request times the container BOOT, not
/// the network. Failure is non-fatal — the image may already be local.
fn pull(image: &str) {
    eprintln!("pulling {image} (linux/amd64)…");
    let _ = std::process::Command::new("docker")
        .args(["pull", "--platform", "linux/amd64", image])
        .status();
}

async fn serve() -> (AppState, SocketAddr) {
    std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
    let backend = ruscker_docker::LocalDockerBackend::local().expect("docker daemon reachable");
    // Unique per test — both tests run in this process concurrently.
    use std::sync::atomic::{AtomicU64, Ordering};
    static N: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ruscker-ws-e2e-{}-{}.db",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let db = ConfigDb::Sqlite(ruscker_admin::db::open(&path).await.expect("open db"));
    let state = AppState {
        config: Arc::new(config()),
        base_path: Arc::from(""),
        locales: Arc::new(Locales::load().expect("locales")),
        admin_auth: AdminAuth {
            admin: Some(Arc::from("ws-e2e-token")),
        },
        admin_sessions: Arc::new(ruscker_admin::auth::InMemoryAdminSessionStore::default()),
        log_buffer: None,
        login_limiter: Arc::new(ruscker_admin::auth::LoginRateLimiter::default_policy()),
        api_limiter: Arc::new(ruscker_admin::ratelimit::ApiRateLimiter::new()),
        db: Some(db),
        images_dir: None,
        master_key: Default::default(),
        backend: Some(Arc::new(backend)),
        replicas: Arc::new(tokio::sync::RwLock::new(Default::default())),
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
    };
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state.clone()).into_make_service_with_connect_info::<SocketAddr>();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (state, addr)
}

/// Plain-HTTP GET without `Accept: text/html`, so the proxy blocks on
/// the coalesced spawn instead of answering with the cold-start splash.
/// Returns (status, set-cookie values, body).
async fn get(addr: SocketAddr, path: &str) -> (u16, Vec<String>, String) {
    use http_body_util::BodyExt;
    let client = hyper_util::client::legacy::Client::builder(hyper_util::rt::TokioExecutor::new())
        .build_http::<http_body_util::Empty<hyper::body::Bytes>>();
    let uri: hyper::Uri = format!("http://{addr}{path}").parse().unwrap();
    let resp = client.get(uri).await.expect("proxied GET");
    let status = resp.status().as_u16();
    let cookies = resp
        .headers()
        .get_all(hyper::header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .filter_map(|v| v.split(';').next())
        .map(str::to_string)
        .collect();
    let body = resp.into_body().collect().await.expect("body").to_bytes();
    (status, cookies, String::from_utf8_lossy(&body).into_owned())
}

/// Open a WebSocket through the proxy, carrying the sticky cookie and
/// an optional subprotocol.
async fn ws_connect(
    addr: SocketAddr,
    path: &str,
    cookies: &[String],
    subprotocol: Option<&str>,
) -> Result<
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    tokio_tungstenite::tungstenite::Error,
> {
    let mut req = format!("ws://{addr}{path}").into_client_request()?;
    if !cookies.is_empty() {
        req.headers_mut()
            .insert("cookie", cookies.join("; ").parse().unwrap());
    }
    if let Some(p) = subprotocol {
        req.headers_mut()
            .insert("sec-websocket-protocol", p.parse().unwrap());
    }
    let (ws, resp) = tokio_tungstenite::connect_async(req).await?;
    assert_eq!(resp.status().as_u16(), 101, "proxy answered the upgrade");
    Ok(ws)
}

type WsStream = tokio_tungstenite::WebSocketStream<
    tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
>;

/// What proves the pump is alive for a given app.
#[derive(Clone, Copy)]
enum WsProof {
    /// The app answers our hello with a Text/Binary frame (Shiny: the
    /// `{"config":…}` reply to the init message).
    DataFrame,
    /// The app's server pings arrive through the pump AND our own Ping
    /// comes back as a Pong — a full client → proxy → app → proxy →
    /// client round trip. Used for Streamlit, whose first data frame
    /// (`NewSession` protobuf) only follows a full protobuf client
    /// handshake not worth faking by hand; its Tornado server pings
    /// every second and answers pings, which exercises both directions
    /// of the pump deterministically.
    PingRoundTrip,
}

/// Wait until `proof` is satisfied, within [`WS_FRAME_TIMEOUT`].
async fn assert_ws_alive(ws: &mut WsStream, spec: &str, proof: WsProof) {
    let deadline = tokio::time::Instant::now() + WS_FRAME_TIMEOUT;
    if matches!(proof, WsProof::PingRoundTrip) {
        ws.send(Message::Ping("ruscker-e2e".into()))
            .await
            .expect("client ping through the pump");
    }
    let mut server_pings = 0u32;
    let mut pong_back = false;
    loop {
        let frame = tokio::time::timeout_at(deadline, ws.next())
            .await
            .expect("app kept the socket alive within the window")
            .expect("stream open")
            .expect("frame ok");
        match (&frame, proof) {
            (Message::Text(_) | Message::Binary(_), WsProof::DataFrame) => {
                let shown = format!("{frame:?}");
                eprintln!("[{spec}] data frame: {:.140}", shown);
                return;
            }
            (Message::Pong(p), WsProof::PingRoundTrip) if p.as_ref() == b"ruscker-e2e" => {
                eprintln!("[{spec}] our ping round-tripped through the app");
                pong_back = true;
            }
            (Message::Ping(_), WsProof::PingRoundTrip) => {
                server_pings += 1;
                eprintln!("[{spec}] server keepalive ping #{server_pings}");
            }
            // A data frame also settles the round-trip mode (an app
            // that talks first is even stronger evidence).
            (Message::Text(_) | Message::Binary(_), WsProof::PingRoundTrip) => {
                eprintln!("[{spec}] unexpected bonus data frame — accepted");
                return;
            }
            _ => eprintln!("[{spec}] other frame: {frame:?}"),
        }
        if pong_back && server_pings >= 2 {
            return; // both directions proven
        }
    }
}

/// Drive one app end to end. `ws_path` is relative to the app root;
/// `page_marker` proves we got the real app page, not an interstitial.
async fn exercise_app(
    addr: SocketAddr,
    spec: &str,
    page_marker: &str,
    ws_path: &str,
    subprotocol: Option<&str>,
    client_hello: Option<Message>,
    proof: WsProof,
) {
    // 1. First proxied request cold-spawns the container and returns
    //    the app page with the injected <base href>.
    let started = std::time::Instant::now();
    let (status, cookies, body) = tokio::time::timeout(
        FIRST_LOAD_TIMEOUT,
        get(addr, &format!("/app/{spec}/")),
    )
    .await
    .expect("first proxied load within the budget");
    eprintln!("[{spec}] first load: {status} in {:?}", started.elapsed());
    assert_eq!(status, 200, "app page proxied: {body:.300}");
    assert!(
        body.contains(&format!("<base href=\"/app/{spec}/\"")),
        "rewriter injected the base href"
    );
    assert!(
        body.to_ascii_lowercase().contains(page_marker),
        "real app page (marker `{page_marker}`), got: {body:.300}"
    );
    assert!(!cookies.is_empty(), "sticky cookie issued on first bind");

    // 2. WebSocket upgrade through the proxy, riding the sticky cookie.
    let full_ws = format!("/app/{spec}/{ws_path}");
    let mut ws = ws_connect(addr, &full_ws, &cookies, subprotocol)
        .await
        .expect("WS upgrade through the proxy");

    // 3. Reactivity: (optionally) say hello like the app's own JS
    //    client would, then require the app-specific liveness proof
    //    through the pump.
    if let Some(hello) = client_hello.clone() {
        ws.send(hello).await.expect("client → app frame");
    }
    assert_ws_alive(&mut ws, spec, proof).await;

    // 4. Refresh survival: a second socket connects while the first
    //    is still open (same session cookie → same replica).
    let mut ws2 = ws_connect(addr, &full_ws, &cookies, subprotocol)
        .await
        .expect("second WS (refresh) upgrades too");
    if let Some(hello) = client_hello {
        ws2.send(hello).await.expect("client → app frame (2nd)");
    }
    assert_ws_alive(&mut ws2, spec, proof).await;

    // 5. Clean client-initiated close on both.
    ws.close(None).await.expect("close 1");
    ws2.close(None).await.expect("close 2");
}

/// R Shiny (httpuv) speaks plain WebSocket at `websocket/` under the
/// app root; the server answers the client's SockJS-style init.
#[tokio::test]
async fn shiny_websocket_arc_end_to_end() {
    let _reaper = Reaper(SHINY_SPEC);
    pull(SHINY_IMAGE);
    let (_state, addr) = serve().await;
    exercise_app(
        addr,
        SHINY_SPEC,
        "shiny",
        "websocket/",
        None,
        // The Shiny JS client's opening frame: a SockJS-framed init.
        // The server answers with its `{"config":…}` session frame.
        Some(Message::text(
            r#"["{\"method\":\"init\",\"data\":{}}"]"#,
        )),
        WsProof::DataFrame,
    )
    .await;
}

/// Streamlit serves its protobuf stream at `_stcore/stream`. Its first
/// data frame needs a full protobuf client handshake, so the liveness
/// proof here is the ping round trip (see [`WsProof::PingRoundTrip`]).
#[tokio::test]
async fn streamlit_websocket_arc_end_to_end() {
    let _reaper = Reaper(STREAMLIT_SPEC);
    pull(STREAMLIT_IMAGE);
    let (_state, addr) = serve().await;
    exercise_app(
        addr,
        STREAMLIT_SPEC,
        "streamlit",
        "_stcore/stream",
        Some("streamlit"),
        None,
        WsProof::PingRoundTrip,
    )
    .await;
}
