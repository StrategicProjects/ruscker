//! WebSocket forwarding — the bidirectional pump from the
//! `ws_spike` example, promoted to a library function. The
//! upgrade hijack itself happens in the route handler
//! (`axum::extract::ws::WebSocketUpgrade::on_upgrade`); this
//! module only owns the pump.
//!
//! Frame translation between axum's `Message` and
//! tokio-tungstenite's `Message` is mechanical but verbose, so
//! it's contained here behind a single `pump` entry point.

use axum::extract::ws::{Message as AxMsg, WebSocket};
use futures_util::{SinkExt, StreamExt};
use tokio_tungstenite::tungstenite::Message as TgMsg;

/// Connect to `upstream_ws_url` and pump frames between `client`
/// and the upstream in both directions until either side closes.
///
/// `upstream_ws_url` is the full ws:// URL on the upstream
/// container — e.g. `ws://127.0.0.1:38421/websocket` for a
/// Shiny app reached at `/app/foo/`.
///
/// Errors are logged via `tracing` but never propagated up — once
/// axum has accepted the upgrade response and returned 101 to
/// the client, there's no useful HTTP-shaped error to surface
/// anyway. We just shut the pump down cleanly.
pub async fn pump(client: WebSocket, upstream_ws_url: String) {
    let upstream = match tokio_tungstenite::connect_async(&upstream_ws_url).await {
        Ok((s, _resp)) => s,
        Err(err) => {
            tracing::error!(error = ?err, url = %upstream_ws_url, "upstream ws connect failed");
            return;
        }
    };

    let (mut cli_tx, mut cli_rx) = client.split();
    let (mut up_tx, mut up_rx) = upstream.split();

    loop {
        tokio::select! {
            msg = cli_rx.next() => match msg {
                Some(Ok(AxMsg::Text(t))) => {
                    let s: String = t.to_string();
                    if up_tx.send(TgMsg::text(s)).await.is_err() { break; }
                }
                Some(Ok(AxMsg::Binary(b))) => {
                    if up_tx.send(TgMsg::binary(b.to_vec())).await.is_err() { break; }
                }
                Some(Ok(AxMsg::Ping(p))) => {
                    if up_tx.send(TgMsg::Ping(p.to_vec().into())).await.is_err() { break; }
                }
                Some(Ok(AxMsg::Pong(p))) => {
                    if up_tx.send(TgMsg::Pong(p.to_vec().into())).await.is_err() { break; }
                }
                Some(Ok(AxMsg::Close(_))) | None => {
                    let _ = up_tx.send(TgMsg::Close(None)).await;
                    break;
                }
                Some(Err(err)) => {
                    tracing::debug!(error = ?err, "client ws error");
                    break;
                }
            },
            msg = up_rx.next() => match msg {
                Some(Ok(TgMsg::Text(t))) => {
                    let s: String = t.to_string();
                    if cli_tx.send(AxMsg::Text(s.into())).await.is_err() { break; }
                }
                Some(Ok(TgMsg::Binary(b))) => {
                    if cli_tx.send(AxMsg::Binary(b.to_vec().into())).await.is_err() { break; }
                }
                Some(Ok(TgMsg::Ping(p))) => {
                    if cli_tx.send(AxMsg::Ping(p.to_vec().into())).await.is_err() { break; }
                }
                Some(Ok(TgMsg::Pong(p))) => {
                    if cli_tx.send(AxMsg::Pong(p.to_vec().into())).await.is_err() { break; }
                }
                Some(Ok(TgMsg::Close(_))) | None => {
                    let _ = cli_tx.send(AxMsg::Close(None)).await;
                    break;
                }
                Some(Ok(TgMsg::Frame(_))) => {
                    // Raw frames are an opt-in mode of tungstenite
                    // that we don't enable; treat as a protocol
                    // violation and tear down.
                    break;
                }
                Some(Err(err)) => {
                    tracing::debug!(error = ?err, "upstream ws error");
                    break;
                }
            },
        }
    }
}
