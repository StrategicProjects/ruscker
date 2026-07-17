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
//! A peer that remains blocked beyond the send timeout loses the whole
//! connection instead of individual frames: partial frame loss would
//! silently corrupt an interactive app session.
//!
//! Every termination is logged with its origin, close code/reason, frame
//! counts, and a zero dropped-frame count. [`pump_with_context`] adds the
//! app and replica fields to the tracing span. Errors are never propagated
//! — once axum has returned its 101, there's no HTTP-shaped error left to
//! surface.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{CloseFrame as AxClose, Message as AxMsg, WebSocket};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{Sink, SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::{header, HeaderValue};
use tokio_tungstenite::tungstenite::protocol::CloseFrame as TgClose;
use tokio_tungstenite::tungstenite::Message as TgMsg;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::Instrument;

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
/// Maximum time one frame may wait for a slow peer. Stateful frames are
/// never dropped; exceeding this closes the connection and emits a warning.
const SEND_TIMEOUT: Duration = Duration::from_secs(30);

const CLIENT: &str = "client";
const UPSTREAM: &str = "upstream";

#[derive(Debug, Clone, PartialEq, Eq)]
enum SendOutcome {
    Sent,
    Failed(String),
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum EndReason {
    Close {
        code: Option<u16>,
        reason: String,
        forward: SendOutcome,
    },
    Eof {
        forward: SendOutcome,
    },
    ReceiveError(String),
    SendError(String),
    BackpressureTimeout,
    UnsupportedFrame,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DirectionEnd {
    origin: &'static str,
    target: &'static str,
    forwarded_frames: u64,
    reason: EndReason,
}

async fn send_with_timeout<S, M>(
    sink: &mut S,
    message: M,
    timeout: Duration,
) -> SendOutcome
where
    S: Sink<M> + Unpin,
    S::Error: std::fmt::Display,
{
    match tokio::time::timeout(timeout, sink.send(message)).await {
        Ok(Ok(())) => SendOutcome::Sent,
        Ok(Err(err)) => SendOutcome::Failed(err.to_string()),
        Err(_) => SendOutcome::TimedOut,
    }
}

/// An established upstream WebSocket connection.
pub type UpstreamStream = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Result of a successful upstream handshake: the stream plus the
/// subprotocol the upstream selected on its 101 (if any). The caller
/// must echo that selection on its own 101 to the client — a browser
/// that offered subprotocols and receives a 101 without one selected
/// is required by RFC 6455 §4.1 to fail the connection (#730).
pub struct UpstreamHandshake {
    pub stream: UpstreamStream,
    pub selected_protocol: Option<String>,
}

/// Open the WebSocket handshake to `upstream_ws_url`, forwarding the
/// client's `cookie` and offered `subprotocols` so the app sees the
/// client's session and can negotiate a protocol.
///
/// This runs *before* the client's own upgrade is answered: on failure
/// the caller still holds a plain HTTP request and can return a real
/// 502 instead of an opaque post-101 drop (#730).
pub async fn connect(
    upstream_ws_url: &str,
    cookie: Option<&str>,
    subprotocols: Option<&str>,
) -> Result<UpstreamHandshake, tokio_tungstenite::tungstenite::Error> {
    connect_with_headers(upstream_ws_url, cookie, subprotocols, &[]).await
}

/// Like [`connect`], with additional authoritative headers supplied by
/// the proxy. Keeping the original three-argument helper as a wrapper
/// preserves existing callers while identity-aware proxies can add the
/// trusted handshake headers they resolved themselves (#1001).
pub async fn connect_with_headers(
    upstream_ws_url: &str,
    cookie: Option<&str>,
    subprotocols: Option<&str>,
    extra_headers: &[(String, String)],
) -> Result<UpstreamHandshake, tokio_tungstenite::tungstenite::Error> {
    let mut request = upstream_ws_url.into_client_request()?;
    // Forward the client's session cookie and requested subprotocol.
    if let Some(v) = cookie.and_then(|s| HeaderValue::from_str(s).ok()) {
        request.headers_mut().insert(header::COOKIE, v);
    }
    if let Some(v) = subprotocols.and_then(|s| HeaderValue::from_str(s).ok()) {
        request
            .headers_mut()
            .insert(header::SEC_WEBSOCKET_PROTOCOL, v);
    }
    for (name, value) in extra_headers {
        if let (Ok(name), Ok(value)) = (
            name.parse::<header::HeaderName>(),
            HeaderValue::from_str(value),
        ) {
            request.headers_mut().insert(name, value);
        }
    }

    let (stream, resp) = tokio_tungstenite::connect_async(request).await?;
    let selected_protocol = resp
        .headers()
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    Ok(UpstreamHandshake {
        stream,
        selected_protocol,
    })
}

/// Pump frames between `client` and the already-connected `upstream`
/// in both directions until either closes (or the connection goes
/// idle). Open the upstream with [`connect`] first.
pub async fn pump(client: WebSocket, upstream: UpstreamStream) {
    pump_inner(client, upstream).await;
}

/// Like [`pump`], with a tracing span that identifies the app and replica.
/// The admin proxy uses this entry point so every close/error/timeout event
/// can be correlated with the affected container (#933).
pub async fn pump_with_context(
    client: WebSocket,
    upstream: UpstreamStream,
    spec: String,
    replica: String,
) {
    let span = tracing::info_span!("ws_pump", spec = %spec, replica = %replica);
    pump_inner(client, upstream).instrument(span).await;
}

async fn pump_inner(client: WebSocket, upstream: UpstreamStream) {
    let (cli_tx, cli_rx) = client.split();
    let (up_tx, up_rx) = upstream.split();

    // Shared last-activity clock (monotonic millis); both directions
    // touch it, the watchdog reads it.
    let last = Arc::new(AtomicU64::new(now_ms()));

    let mut c2u = tokio::spawn(
        client_to_upstream(cli_rx, up_tx, last.clone()).in_current_span(),
    );
    let mut u2c = tokio::spawn(
        upstream_to_client(up_rx, cli_tx, last.clone()).in_current_span(),
    );
    let mut watchdog = tokio::spawn(idle_watchdog(last).in_current_span());

    // Whichever finishes first wins; give the *other* direction a short
    // grace to drain before we abort everything.
    tokio::select! {
        result = &mut c2u => {
            log_join_result("client_to_upstream", result);
            match tokio::time::timeout(DRAIN_GRACE, &mut u2c).await {
                Ok(result) => log_join_result("upstream_to_client", result),
                Err(_) => log_drain_timeout("upstream_to_client"),
            }
        }
        result = &mut u2c => {
            log_join_result("upstream_to_client", result);
            match tokio::time::timeout(DRAIN_GRACE, &mut c2u).await {
                Ok(result) => log_join_result("client_to_upstream", result),
                Err(_) => log_drain_timeout("client_to_upstream"),
            }
        }
        _ = &mut watchdog => {
            tracing::info!(
                origin = "watchdog",
                idle_timeout_secs = IDLE_TIMEOUT.as_secs(),
                dropped_frames = 0_u64,
                "ws idle timeout"
            );
        }
    }
    c2u.abort();
    u2c.abort();
    watchdog.abort();
    tracing::debug!(dropped_frames = 0_u64, "ws pump ended");
}

fn log_join_result(direction: &'static str, result: Result<DirectionEnd, tokio::task::JoinError>) {
    match result {
        Ok(end) => log_direction_end(end),
        Err(err) => tracing::warn!(
            direction,
            error = ?err,
            dropped_frames = 0_u64,
            "ws direction task failed"
        ),
    }
}

fn log_drain_timeout(direction: &'static str) {
    tracing::warn!(
        direction,
        drain_timeout_ms = DRAIN_GRACE.as_millis() as u64,
        dropped_frames = 0_u64,
        backpressure_action = "close_connection",
        "ws drain timeout"
    );
}

fn log_direction_end(end: DirectionEnd) {
    let DirectionEnd {
        origin,
        target,
        forwarded_frames,
        reason,
    } = end;
    match reason {
        EndReason::Close {
            code,
            reason,
            forward: SendOutcome::Sent,
        } => tracing::debug!(
            origin,
            target,
            close_code = ?code,
            close_reason = ?reason,
            forwarded_frames,
            dropped_frames = 0_u64,
            "ws close frame"
        ),
        EndReason::Close {
            code,
            reason,
            forward: SendOutcome::Failed(error),
        } => tracing::debug!(
            origin,
            target,
            close_code = ?code,
            close_reason = ?reason,
            error = %error,
            forwarded_frames,
            dropped_frames = 0_u64,
            "ws close forwarding failed"
        ),
        EndReason::Close {
            code,
            reason,
            forward: SendOutcome::TimedOut,
        } => tracing::warn!(
            origin,
            target,
            close_code = ?code,
            close_reason = ?reason,
            send_timeout_ms = SEND_TIMEOUT.as_millis() as u64,
            forwarded_frames,
            dropped_frames = 0_u64,
            backpressure_action = "close_connection",
            "ws close forwarding timed out"
        ),
        EndReason::Eof {
            forward: SendOutcome::Sent,
        } => tracing::debug!(
            origin,
            target,
            forwarded_frames,
            dropped_frames = 0_u64,
            "ws stream ended"
        ),
        EndReason::Eof {
            forward: SendOutcome::Failed(error),
        } => tracing::debug!(
            origin,
            target,
            error = %error,
            forwarded_frames,
            dropped_frames = 0_u64,
            "ws stream ended; fallback close failed"
        ),
        EndReason::Eof {
            forward: SendOutcome::TimedOut,
        } => tracing::warn!(
            origin,
            target,
            send_timeout_ms = SEND_TIMEOUT.as_millis() as u64,
            forwarded_frames,
            dropped_frames = 0_u64,
            backpressure_action = "close_connection",
            "ws fallback close timed out"
        ),
        EndReason::ReceiveError(error) => tracing::debug!(
            origin,
            target,
            error = %error,
            forwarded_frames,
            dropped_frames = 0_u64,
            "ws receive error"
        ),
        EndReason::SendError(error) => tracing::debug!(
            origin,
            target,
            error = %error,
            forwarded_frames,
            dropped_frames = 0_u64,
            "ws send error"
        ),
        EndReason::BackpressureTimeout => tracing::warn!(
            origin,
            target,
            send_timeout_ms = SEND_TIMEOUT.as_millis() as u64,
            forwarded_frames,
            dropped_frames = 0_u64,
            backpressure_action = "close_connection",
            "ws send timed out"
        ),
        EndReason::UnsupportedFrame => tracing::debug!(
            origin,
            target,
            forwarded_frames,
            dropped_frames = 0_u64,
            "ws unsupported raw frame"
        ),
    }
}

/// Forward client → upstream until the client closes/errors.
async fn client_to_upstream(
    mut rx: SplitStream<WebSocket>,
    mut tx: SplitSink<UpstreamStream, TgMsg>,
    last: Arc<AtomicU64>,
) -> DirectionEnd {
    let mut forwarded_frames = 0_u64;
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
                let code = frame.as_ref().map(|f| f.code);
                let reason = frame
                    .as_ref()
                    .map(|f| f.reason.to_string())
                    .unwrap_or_default();
                let forward = send_with_timeout(
                    &mut tx,
                    TgMsg::Close(ax_close_to_tg(frame)),
                    SEND_TIMEOUT,
                )
                .await;
                if forward == SendOutcome::Sent {
                    forwarded_frames += 1;
                }
                return DirectionEnd {
                    origin: CLIENT,
                    target: UPSTREAM,
                    forwarded_frames,
                    reason: EndReason::Close {
                        code,
                        reason,
                        forward,
                    },
                };
            }
            Err(err) => {
                let _ = send_with_timeout(&mut tx, TgMsg::Close(None), SEND_TIMEOUT).await;
                return DirectionEnd {
                    origin: CLIENT,
                    target: UPSTREAM,
                    forwarded_frames,
                    reason: EndReason::ReceiveError(err.to_string()),
                };
            }
        };
        match send_with_timeout(&mut tx, out, SEND_TIMEOUT).await {
            SendOutcome::Sent => forwarded_frames += 1,
            SendOutcome::Failed(error) => {
                return DirectionEnd {
                    origin: CLIENT,
                    target: UPSTREAM,
                    forwarded_frames,
                    reason: EndReason::SendError(error),
                };
            }
            SendOutcome::TimedOut => {
                return DirectionEnd {
                    origin: CLIENT,
                    target: UPSTREAM,
                    forwarded_frames,
                    reason: EndReason::BackpressureTimeout,
                };
            }
        }
    }
    // Client side ended without an explicit close — ask upstream to close.
    let forward = send_with_timeout(&mut tx, TgMsg::Close(None), SEND_TIMEOUT).await;
    if forward == SendOutcome::Sent {
        forwarded_frames += 1;
    }
    DirectionEnd {
        origin: CLIENT,
        target: UPSTREAM,
        forwarded_frames,
        reason: EndReason::Eof { forward },
    }
}

/// Forward upstream → client until the upstream closes/errors.
async fn upstream_to_client(
    mut rx: SplitStream<UpstreamStream>,
    mut tx: SplitSink<WebSocket, AxMsg>,
    last: Arc<AtomicU64>,
) -> DirectionEnd {
    let mut forwarded_frames = 0_u64;
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
                let code = frame.as_ref().map(|f| u16::from(f.code));
                let reason = frame
                    .as_ref()
                    .map(|f| f.reason.to_string())
                    .unwrap_or_default();
                let forward = send_with_timeout(
                    &mut tx,
                    AxMsg::Close(tg_close_to_ax(frame)),
                    SEND_TIMEOUT,
                )
                .await;
                if forward == SendOutcome::Sent {
                    forwarded_frames += 1;
                }
                return DirectionEnd {
                    origin: UPSTREAM,
                    target: CLIENT,
                    forwarded_frames,
                    reason: EndReason::Close {
                        code,
                        reason,
                        forward,
                    },
                };
            }
            // Raw frames are an opt-in tungstenite mode we don't enable.
            Ok(TgMsg::Frame(_)) => {
                let _ = send_with_timeout(&mut tx, AxMsg::Close(None), SEND_TIMEOUT).await;
                return DirectionEnd {
                    origin: UPSTREAM,
                    target: CLIENT,
                    forwarded_frames,
                    reason: EndReason::UnsupportedFrame,
                };
            }
            Err(err) => {
                let _ = send_with_timeout(&mut tx, AxMsg::Close(None), SEND_TIMEOUT).await;
                return DirectionEnd {
                    origin: UPSTREAM,
                    target: CLIENT,
                    forwarded_frames,
                    reason: EndReason::ReceiveError(err.to_string()),
                };
            }
        };
        match send_with_timeout(&mut tx, out, SEND_TIMEOUT).await {
            SendOutcome::Sent => forwarded_frames += 1,
            SendOutcome::Failed(error) => {
                return DirectionEnd {
                    origin: UPSTREAM,
                    target: CLIENT,
                    forwarded_frames,
                    reason: EndReason::SendError(error),
                };
            }
            SendOutcome::TimedOut => {
                return DirectionEnd {
                    origin: UPSTREAM,
                    target: CLIENT,
                    forwarded_frames,
                    reason: EndReason::BackpressureTimeout,
                };
            }
        }
    }
    let forward = send_with_timeout(&mut tx, AxMsg::Close(None), SEND_TIMEOUT).await;
    if forward == SendOutcome::Sent {
        forwarded_frames += 1;
    }
    DirectionEnd {
        origin: UPSTREAM,
        target: CLIENT,
        forwarded_frames,
        reason: EndReason::Eof { forward },
    }
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
    use std::pin::Pin;
    use std::task::{Context, Poll};

    struct PendingSink;

    impl<T> Sink<T> for PendingSink {
        type Error = std::io::Error;

        fn poll_ready(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }

        fn start_send(self: Pin<&mut Self>, _item: T) -> Result<(), Self::Error> {
            Ok(())
        }

        fn poll_flush(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Pending
        }

        fn poll_close(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }
    }

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

    #[tokio::test]
    async fn blocked_send_becomes_timeout_instead_of_a_frame_drop() {
        let mut sink = PendingSink;
        let outcome = send_with_timeout(&mut sink, (), Duration::from_millis(5)).await;
        assert_eq!(outcome, SendOutcome::TimedOut);
    }

    #[test]
    fn direction_end_keeps_origin_close_and_frame_count() {
        let end = DirectionEnd {
            origin: UPSTREAM,
            target: CLIENT,
            forwarded_frames: 17,
            reason: EndReason::Close {
                code: Some(1001),
                reason: "maintenance".into(),
                forward: SendOutcome::Sent,
            },
        };
        assert_eq!(end.origin, "upstream");
        assert_eq!(end.target, "client");
        assert_eq!(end.forwarded_frames, 17);
        assert_eq!(
            end.reason,
            EndReason::Close {
                code: Some(1001),
                reason: "maintenance".into(),
                forward: SendOutcome::Sent,
            }
        );
    }

    // #730: the upstream handshake must carry the full URL (Jupyter
    // kernel channels key on `?session_id=`) and report back which
    // subprotocol the upstream selected, so the proxy can echo it on
    // its own 101 to the client.
    #[tokio::test]
    // tungstenite's accept_hdr callback returns its large ErrorResponse
    // by value; fine for a test.
    #[allow(clippy::result_large_err)]
    async fn connect_preserves_query_and_reports_upstream_selected_subprotocol() {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.unwrap();
            let mut seen_uri = String::new();
            let ws = tokio_tungstenite::accept_hdr_async(
                tcp,
                |req: &Request, mut resp: Response| {
                    seen_uri = req.uri().to_string();
                    // The app picks the SECOND offered protocol — proves the
                    // reported selection is the upstream's choice, not an
                    // echo of the client's first offer.
                    resp.headers_mut().insert(
                        header::SEC_WEBSOCKET_PROTOCOL,
                        HeaderValue::from_static("superchat"),
                    );
                    Ok(resp)
                },
            )
            .await
            .unwrap();
            drop(ws);
            seen_uri
        });

        let url = format!("ws://{addr}/api/kernels/k1/channels?session_id=abc");
        let hs = connect(&url, None, Some("chat, superchat"))
            .await
            .expect("upstream handshake");
        assert_eq!(hs.selected_protocol.as_deref(), Some("superchat"));
        assert_eq!(
            server.await.unwrap(),
            "/api/kernels/k1/channels?session_id=abc",
            "query string must reach the upstream handshake"
        );
    }
}
