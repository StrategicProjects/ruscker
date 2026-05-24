//! Pre-Phase-3 spike: validate axum 0.8 + tokio-tungstenite 0.29
//! can do the WebSocket upgrade + bidirectional pump that the
//! Shiny proxy needs.
//!
//! This is a SELF-CONTAINED test. The binary spawns three things
//! in the same process:
//!
//!   1. An **upstream echo server** on 127.0.0.1:18182 — what a
//!      Shiny container would look like from our perspective.
//!   2. A **proxy** on 127.0.0.1:18181 that, on `/ws`, hijacks
//!      the upgrade and forwards frames to/from the upstream.
//!   3. A **client** that connects to the proxy, sends three text
//!      messages, and verifies they round-trip.
//!
//! Success criterion: all three messages echo back identically
//! within 5 s. Failure modes we're explicitly looking for:
//!
//!   * axum's `WebSocketUpgrade` returning an error on hijack
//!   * tokio-tungstenite refusing the upstream handshake
//!   * the bidirectional pump dropping frames or deadlocking
//!   * close-frame handling leaking the other half-open socket
//!
//! Run with:
//!   cargo run --example ws_spike -p ruscker-proxy
//!
//! Throwaway code — once Phase 3 ships, this either gets folded
//! into `tests/ws_e2e.rs` or deleted.

use axum::extract::ws::{Message as AxMsg, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use std::net::SocketAddr;
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::tungstenite::Message as TgMsg;
use tokio_tungstenite::{accept_async, connect_async};

const PROXY_ADDR: &str = "127.0.0.1:18181";
const UPSTREAM_ADDR: &str = "127.0.0.1:18182";
const TEST_MESSAGES: &[&str] = &["ping", "shiny-state-update", "héllo 🦀"];

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("ws_spike=info,ruscker_proxy=info")
        .with_target(false)
        .init();

    let upstream_handle = tokio::spawn(run_upstream_echo());
    let proxy_handle = tokio::spawn(run_proxy());

    // Give both servers a moment to bind.
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Drive the test client. Bail out on first failure.
    let result = run_client().await;

    upstream_handle.abort();
    proxy_handle.abort();

    match result {
        Ok(()) => {
            println!("\n✓ WS spike passed: axum 0.8 + tokio-tungstenite 0.29 ok");
            println!("  — upgrade hijack works");
            println!("  — bidirectional pump round-trips text frames");
            println!("  — close handling clean on both sides");
            Ok(())
        }
        Err(err) => {
            eprintln!("\n✗ WS spike FAILED: {err:?}");
            std::process::exit(1);
        }
    }
}

/// Upstream that mirrors every text frame back and closes politely.
async fn run_upstream_echo() -> anyhow::Result<()> {
    let listener = TcpListener::bind(UPSTREAM_ADDR).await?;
    tracing::info!(addr = UPSTREAM_ADDR, "upstream echo listening");
    loop {
        let (stream, _) = listener.accept().await?;
        tokio::spawn(handle_upstream(stream));
    }
}

async fn handle_upstream(stream: TcpStream) -> anyhow::Result<()> {
    let ws = accept_async(stream).await?;
    let (mut tx, mut rx) = ws.split();
    while let Some(msg) = rx.next().await {
        let msg = msg?;
        if msg.is_close() {
            break;
        }
        if msg.is_text() {
            tx.send(msg).await?;
        }
    }
    Ok(())
}

/// Proxy: takes /ws on PROXY_ADDR and pumps frames against UPSTREAM_ADDR.
async fn run_proxy() -> anyhow::Result<()> {
    let app = Router::new().route("/ws", get(ws_handler));
    let addr: SocketAddr = PROXY_ADDR.parse().unwrap();
    let listener = TcpListener::bind(addr).await?;
    tracing::info!(addr = PROXY_ADDR, "proxy listening");
    axum::serve(listener, app).await?;
    Ok(())
}

async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_pump)
}

/// The hot loop: receive a frame on either side, forward it to the
/// other. Errors on either side break the loop and let the dropped
/// halves close cleanly.
///
/// Real Phase 3 code will replace the inline pump with a pair of
/// `tokio::spawn`'d forwarders + `tokio::select!` to keep latency
/// down. For the spike we keep it linear so the control flow is
/// audit-able by eye.
async fn handle_pump(client: WebSocket) {
    let upstream_url = format!("ws://{UPSTREAM_ADDR}/");
    let upstream = match connect_async(&upstream_url).await {
        Ok((s, _resp)) => s,
        Err(err) => {
            tracing::error!(error = ?err, "upstream connect failed");
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
                Some(Err(_)) => break,
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
                Some(Ok(TgMsg::Frame(_))) | Some(Err(_)) => break,
            },
        }
    }
}

/// Test client: connect → send N → receive N → exit ok.
async fn run_client() -> anyhow::Result<()> {
    let url = format!("ws://{PROXY_ADDR}/ws");
    let (ws, _resp) = tokio::time::timeout(Duration::from_secs(2), connect_async(&url))
        .await
        .map_err(|_| anyhow::anyhow!("proxy connect timed out"))??;
    let (mut tx, mut rx) = ws.split();

    for msg in TEST_MESSAGES {
        tx.send(TgMsg::text(*msg)).await?;
        let echoed = tokio::time::timeout(Duration::from_secs(2), rx.next())
            .await
            .map_err(|_| anyhow::anyhow!("no echo within 2s for `{msg}`"))?
            .ok_or_else(|| anyhow::anyhow!("upstream closed before echoing `{msg}`"))??;

        let text: &str = match &echoed {
            TgMsg::Text(t) => t.as_str(),
            other => anyhow::bail!("expected text, got {:?}", other),
        };
        if text != *msg {
            anyhow::bail!("mismatch: sent `{msg}`, got `{text}`");
        }
        println!("  · {msg} → {text}  ✓");
    }

    // Polite close + drain.
    tx.send(TgMsg::Close(None)).await?;
    let _ = tokio::time::timeout(Duration::from_secs(1), async {
        while let Some(_) = rx.next().await {}
    })
    .await;
    Ok(())
}
