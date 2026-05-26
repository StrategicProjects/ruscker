//! WebSocket forwarding — the bidirectional pump between a client
//! socket (axum) and the upstream container socket (tokio-tungstenite).
//!
//! The two directions run as **independent tasks** decoupled from each
//! other, not a single `select!` loop: in the old loop, awaiting a send
//! to a slow peer stalled *both* reads (head-of-line blocking), and a
//! client that opened a socket and never read could pin the connection
//! forever. Now each direction backpressures only its own producer (the
//! correct behaviour for stateful Shiny/Streamlit frames — we must not
//! drop them), an **idle watchdog** reaps a connection with no traffic
//! either way, and a closing side gets a short grace to drain the other.
//!
//! Errors are logged via `tracing` but never propagated — once axum has
//! returned its 101, there's no HTTP-shaped error left to surface.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message as AxMsg, WebSocket};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header, HeaderValue};
use tokio_tungstenite::tungstenite::Message as TgMsg;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// Tear a connection down if neither side sends a frame for this long.
/// Shiny/Streamlit heartbeat well under this, so it only catches dead or
/// abandoned sockets.
const IDLE_TIMEOUT: Duration = Duration::from_secs(600);
/// How often the watchdog checks for idleness.
const IDLE_CHECK: Duration = Duration::from_secs(30);
/// After one side closes, how long the other may keep draining its final
/// frames (the close handshake, a last burst) before forced teardown.
const DRAIN_GRACE: Duration = Duration::from_secs(5);

type UpstreamStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Connect to `upstream_ws_url` and pump frames between `client` and the
/// upstream in both directions until either closes (or the connection
/// goes idle). `cookie` / `subprotocols` are forwarded onto the upstream
/// handshake so the app sees the client's session and negotiated
/// protocol.
pub async fn pump(
    client: WebSocket,
    upstream_ws_url: String,
    cookie: Option<String>,
    subprotocols: Option<String>,
) {
    let mut request = match upstream_ws_url.as_str().into_client_request() {
        Ok(r) => r,
        Err(err) => {
            tracing::error!(error = ?err, url = %upstream_ws_url, "build upstream ws request failed");
            return;
        }
    };
    // Forward the client's session cookie and requested subprotocol.
    if let Some(v) = cookie
        .as_deref()
        .and_then(|s| HeaderValue::from_str(s).ok())
    {
        request.headers_mut().insert(header::COOKIE, v);
    }
    if let Some(v) = subprotocols
        .as_deref()
        .and_then(|s| HeaderValue::from_str(s).ok())
    {
        request
            .headers_mut()
            .insert(header::SEC_WEBSOCKET_PROTOCOL, v);
    }

    let upstream = match tokio_tungstenite::connect_async(request).await {
        Ok((s, _resp)) => s,
        Err(err) => {
            tracing::error!(error = ?err, url = %upstream_ws_url, "upstream ws connect failed");
            return;
        }
    };

    let (cli_tx, cli_rx) = client.split();
    let (up_tx, up_rx) = upstream.split();

    // Shared last-activity clock (monotonic millis); both directions
    // touch it, the watchdog reads it.
    let last = Arc::new(AtomicU64::new(now_ms()));

    let mut c2u = tokio::spawn(client_to_upstream(cli_rx, up_tx, last.clone()));
    let mut u2c = tokio::spawn(upstream_to_client(up_rx, cli_tx, last.clone()));
    let mut watchdog = tokio::spawn(idle_watchdog(last));

    // Whichever finishes first wins; give the *other* direction a short
    // grace to drain before we abort everything.
    tokio::select! {
        _ = &mut c2u => { let _ = tokio::time::timeout(DRAIN_GRACE, &mut u2c).await; }
        _ = &mut u2c => { let _ = tokio::time::timeout(DRAIN_GRACE, &mut c2u).await; }
        _ = &mut watchdog => {}
    }
    c2u.abort();
    u2c.abort();
    watchdog.abort();
}

/// Forward client → upstream until the client closes/errors.
async fn client_to_upstream(
    mut rx: SplitStream<WebSocket>,
    mut tx: SplitSink<UpstreamStream, TgMsg>,
    last: Arc<AtomicU64>,
) {
    while let Some(msg) = rx.next().await {
        last.store(now_ms(), Ordering::Relaxed);
        let out = match msg {
            Ok(AxMsg::Text(t)) => TgMsg::text(t.to_string()),
            Ok(AxMsg::Binary(b)) => TgMsg::binary(b.to_vec()),
            Ok(AxMsg::Ping(p)) => TgMsg::Ping(p.to_vec().into()),
            Ok(AxMsg::Pong(p)) => TgMsg::Pong(p.to_vec().into()),
            Ok(AxMsg::Close(_)) => break,
            Err(err) => {
                tracing::debug!(error = ?err, "client ws error");
                break;
            }
        };
        if tx.send(out).await.is_err() {
            break;
        }
    }
    // Client side ended — ask the upstream to close too.
    let _ = tx.send(TgMsg::Close(None)).await;
}

/// Forward upstream → client until the upstream closes/errors.
async fn upstream_to_client(
    mut rx: SplitStream<UpstreamStream>,
    mut tx: SplitSink<WebSocket, AxMsg>,
    last: Arc<AtomicU64>,
) {
    while let Some(msg) = rx.next().await {
        last.store(now_ms(), Ordering::Relaxed);
        let out = match msg {
            Ok(TgMsg::Text(t)) => AxMsg::Text(t.to_string().into()),
            Ok(TgMsg::Binary(b)) => AxMsg::Binary(b.to_vec().into()),
            Ok(TgMsg::Ping(p)) => AxMsg::Ping(p.to_vec().into()),
            Ok(TgMsg::Pong(p)) => AxMsg::Pong(p.to_vec().into()),
            Ok(TgMsg::Close(_)) => break,
            // Raw frames are an opt-in tungstenite mode we don't enable.
            Ok(TgMsg::Frame(_)) => break,
            Err(err) => {
                tracing::debug!(error = ?err, "upstream ws error");
                break;
            }
        };
        if tx.send(out).await.is_err() {
            break;
        }
    }
    let _ = tx.send(AxMsg::Close(None)).await;
}

/// Returns when the connection has seen no frames either way for
/// `IDLE_TIMEOUT`; the caller treats that as a dead socket and tears it
/// down.
async fn idle_watchdog(last: Arc<AtomicU64>) {
    let idle_ms = IDLE_TIMEOUT.as_millis() as u64;
    loop {
        tokio::time::sleep(IDLE_CHECK).await;
        if now_ms().saturating_sub(last.load(Ordering::Relaxed)) >= idle_ms {
            return;
        }
    }
}

/// Monotonic milliseconds since first call — a cheap clock for the idle
/// check that needs no wall-clock and fits an `AtomicU64`.
fn now_ms() -> u64 {
    use std::sync::OnceLock;
    static START: OnceLock<std::time::Instant> = OnceLock::new();
    START
        .get_or_init(std::time::Instant::now)
        .elapsed()
        .as_millis() as u64
}
