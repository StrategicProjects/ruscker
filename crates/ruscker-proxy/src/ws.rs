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

use axum::extract::ws::{CloseFrame as AxClose, Message as AxMsg, WebSocket};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::CloseFrame as TgClose;
use tokio_tungstenite::tungstenite::Message as TgMsg;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

/// Translate a client (axum) close frame to the upstream (tungstenite)
/// shape, preserving the close code and reason. Forwarding the real frame
/// — instead of an empty `Close(None)` — lets the peer see the actual
/// intent (e.g. 1001 going-away vs 1011 internal error).
fn ax_close_to_tg(c: Option<AxClose>) -> Option<TgClose> {
    c.map(|f| TgClose {
        code: f.code.into(),
        reason: f.reason.as_str().into(),
    })
}

/// Translate an upstream (tungstenite) close frame to the client (axum)
/// shape, preserving code and reason.
fn tg_close_to_ax(c: Option<TgClose>) -> Option<AxClose> {
    c.map(|f| AxClose {
        code: f.code.into(),
        reason: f.reason.as_str().into(),
    })
}

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
            // Binary/Ping/Pong are `bytes::Bytes` on both sides → pass them
            // through zero-copy (no per-frame heap copy) for high-throughput
            // binary Shiny/Streamlit traffic.
            Ok(AxMsg::Text(t)) => TgMsg::text(t.to_string()),
            Ok(AxMsg::Binary(b)) => TgMsg::Binary(b),
            Ok(AxMsg::Ping(p)) => TgMsg::Ping(p),
            Ok(AxMsg::Pong(p)) => TgMsg::Pong(p),
            Ok(AxMsg::Close(frame)) => {
                // Forward the client's actual close code/reason, then stop
                // (don't also send the fallback Close(None) below).
                let _ = tx.send(TgMsg::Close(ax_close_to_tg(frame))).await;
                return;
            }
            Err(err) => {
                tracing::debug!(error = ?err, "client ws error");
                break;
            }
        };
        if tx.send(out).await.is_err() {
            break;
        }
    }
    // Client side ended without an explicit close — ask upstream to close.
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
            // Zero-copy pass-through (shared `bytes::Bytes`).
            Ok(TgMsg::Binary(b)) => AxMsg::Binary(b),
            Ok(TgMsg::Ping(p)) => AxMsg::Ping(p),
            Ok(TgMsg::Pong(p)) => AxMsg::Pong(p),
            Ok(TgMsg::Close(frame)) => {
                // Forward the upstream's actual close code/reason, then stop.
                let _ = tx.send(AxMsg::Close(tg_close_to_ax(frame))).await;
                return;
            }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn close_frame_translation_preserves_code_and_reason() {
        // axum → tungstenite (client closing toward upstream)
        let ax = Some(AxClose {
            code: 1011,
            reason: "boom".into(),
        });
        let tg = ax_close_to_tg(ax).expect("some");
        assert_eq!(u16::from(tg.code), 1011);
        assert_eq!(tg.reason.as_str(), "boom");

        // tungstenite → axum (upstream closing toward client)
        let tg2 = Some(TgClose {
            code: 1001u16.into(),
            reason: "going away".into(),
        });
        let ax2 = tg_close_to_ax(tg2).expect("some");
        assert_eq!(ax2.code, 1001);
        assert_eq!(ax2.reason.as_str(), "going away");
    }

    #[test]
    fn close_frame_translation_passes_none_through() {
        assert!(ax_close_to_tg(None).is_none());
        assert!(tg_close_to_ax(None).is_none());
    }
}
