//! Integration test for the WebSocket pump (#81): a real client WS
//! connects through an axum app that runs `ws::pump`, which forwards to
//! a mock echo upstream. Validates bidirectional forwarding and a clean
//! client-initiated close end-to-end.

use std::time::Duration;

use axum::extract::ws::WebSocketUpgrade;
use axum::routing::any;
use axum::Router;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

/// Spawn a mock upstream that echoes text/binary frames back. Returns
/// its `ws://` URL.
async fn spawn_echo_upstream() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            while let Some(Ok(msg)) = ws.next().await {
                #[allow(clippy::collapsible_match)] // the natural read here is two statements
                match msg {
                    Message::Text(_) | Message::Binary(_) => {
                        if ws.send(msg).await.is_err() {
                            break;
                        }
                    }
                    Message::Close(_) => break,
                    _ => {}
                }
            }
        }
    });
    format!("ws://{addr}/")
}

/// Spawn an upstream that immediately closes with a diagnostic frame.
async fn spawn_closing_upstream() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        if let Ok((stream, _)) = listener.accept().await {
            let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
            ws.close(Some(tokio_tungstenite::tungstenite::protocol::CloseFrame {
                code: 1001u16.into(),
                reason: "maintenance".into(),
            }))
            .await
            .unwrap();
        }
    });
    format!("ws://{addr}/")
}

/// Spawn the axum proxy app (one `/ws` route → `ws::connect` +
/// `ws::pump`, the #730 two-step shape). Returns its bound address.
async fn spawn_proxy(upstream_url: String) -> std::net::SocketAddr {
    let app = Router::new().route(
        "/ws",
        any(move |ws: WebSocketUpgrade| {
            let url = upstream_url.clone();
            async move {
                let handshake = ruscker_proxy::ws::connect(&url, None, None)
                    .await
                    .expect("upstream handshake");
                ws.on_upgrade(move |sock| {
                    ruscker_proxy::ws::pump_with_context(
                        sock,
                        handshake.stream,
                        "test-app".into(),
                        "test-replica".into(),
                    )
                })
            }
        }),
    );
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    addr
}

#[tokio::test]
async fn pump_forwards_both_ways_and_closes_cleanly() {
    let upstream = spawn_echo_upstream().await;
    let proxy = spawn_proxy(upstream).await;

    let (mut client, _resp) = tokio_tungstenite::connect_async(format!("ws://{proxy}/ws"))
        .await
        .expect("client connects through the proxy");

    // Client → upstream → (echo) → client.
    client.send(Message::text("hello")).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), client.next())
        .await
        .expect("reply within 5s")
        .expect("a frame")
        .expect("not an error");
    assert_eq!(reply.to_text().unwrap(), "hello");

    // A second round-trip — the pump keeps forwarding.
    client.send(Message::text("world")).await.unwrap();
    let reply = tokio::time::timeout(Duration::from_secs(5), client.next())
        .await
        .expect("reply within 5s")
        .expect("a frame")
        .expect("not an error");
    assert_eq!(reply.to_text().unwrap(), "world");

    // Client closes; the pump should propagate and tear down without
    // hanging. The stream should end (None) shortly after.
    client.close(None).await.unwrap();
    let drained = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(frame) = client.next().await {
            if frame.is_err() {
                break;
            }
        }
    })
    .await;
    assert!(drained.is_ok(), "pump tore down without hanging");
}

#[tokio::test]
async fn pump_preserves_upstream_close_code_and_reason() {
    let upstream = spawn_closing_upstream().await;
    let proxy = spawn_proxy(upstream).await;
    let (mut client, _resp) = tokio_tungstenite::connect_async(format!("ws://{proxy}/ws"))
        .await
        .expect("client connects through the proxy");

    let frame = tokio::time::timeout(Duration::from_secs(5), client.next())
        .await
        .expect("close within 5s")
        .expect("a close frame")
        .expect("not an error");
    let Message::Close(Some(close)) = frame else {
        panic!("expected close frame, got {frame:?}");
    };
    assert_eq!(u16::from(close.code), 1001);
    assert_eq!(close.reason.as_str(), "maintenance");
}
