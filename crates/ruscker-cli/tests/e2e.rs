//! Live end-to-end test (#110): drive the real `ruscker serve --docker`
//! binary against a real container and exercise the two arcs that were
//! previously only unit-tested — HTTP forwarding **and the WebSocket
//! pump**, both under the `/app/{spec}/` sub-path.
//!
//! Upstream is `jmalloc/echo-server` (a ~5 MB public image): it echoes
//! HTTP requests and upgrades to a WebSocket at `/.ws`, sending an
//! initial frame on connect. That's enough to prove the proxy forwards
//! and the WS pump completes the upgrade + relays a frame through the
//! sub-path.
//!
//! Gated behind the `e2e` feature (needs Docker + network):
//!   cargo test -p ruscker-cli --features e2e -- --nocapture
//!
//! The spec is `type: streamlit` (an InteractiveApp ⇒ sticky sessions +
//! WS forwarding) with `container-port: 8080` to point Ruscker at the
//! echo-server's listen port (#120).
#![cfg(feature = "e2e")]

use futures_util::StreamExt;
use std::process::Command;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const HOST: &str = "127.0.0.1";
const PORT: u16 = 8071;
const SPEC: &str = "echo";

/// Minimal HTTP/1.1 GET over a raw socket — avoids pulling a full HTTP
/// client into dev-deps. Returns (status_code, body_text). Best-effort
/// body parsing (chunk framing may leak in, which the substring asserts
/// tolerate).
async fn http_get(path: &str, timeout: Duration) -> anyhow::Result<(u16, String)> {
    let fut = async {
        let mut s = TcpStream::connect((HOST, PORT)).await?;
        let req = format!("GET {path} HTTP/1.1\r\nHost: {HOST}\r\nConnection: close\r\n\r\n");
        s.write_all(req.as_bytes()).await?;
        let mut buf = Vec::new();
        s.read_to_end(&mut buf).await?;
        let text = String::from_utf8_lossy(&buf).into_owned();
        let status = text
            .lines()
            .next()
            .and_then(|l| l.split_whitespace().nth(1))
            .and_then(|c| c.parse().ok())
            .unwrap_or(0);
        let body = text.splitn(2, "\r\n\r\n").nth(1).unwrap_or("").to_string();
        Ok::<_, anyhow::Error>((status, body))
    };
    tokio::time::timeout(timeout, fut).await?
}

/// Remove any echo containers Ruscker spawned (best-effort).
fn cleanup_containers() {
    if let Ok(out) = Command::new("docker")
        .args(["ps", "-aq", "--filter", "name=ruscker-echo"])
        .output()
    {
        for id in String::from_utf8_lossy(&out.stdout).split_whitespace() {
            let _ = Command::new("docker").args(["rm", "-f", id]).output();
        }
    }
}

#[tokio::test]
async fn e2e_proxy_and_websocket_through_subpath() {
    // Skip cleanly if Docker isn't usable.
    if Command::new("docker")
        .arg("info")
        .output()
        .map(|o| !o.status.success())
        .unwrap_or(true)
    {
        eprintln!("docker not available — skipping e2e");
        return;
    }

    let dir = std::env::temp_dir().join(format!("ruscker-e2e-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let cfg = dir.join("e2e.yml");
    std::fs::write(
        &cfg,
        format!(
            "proxy:\n  title: E2E\n  specs:\n    - id: {SPEC}\n      display-name: Echo\n      \
             container-image: jmalloc/echo-server:latest\n      type: streamlit\n      \
             container-port: 8080\n"
        ),
    )
    .unwrap();

    cleanup_containers();
    let bin = env!("CARGO_BIN_EXE_ruscker");
    let mut child = Command::new(bin)
        .args([
            "serve",
            "--config",
            cfg.to_str().unwrap(),
            "--bind",
            &format!("{HOST}:{PORT}"),
            "--docker",
        ])
        .env("RUST_LOG", "warn")
        .spawn()
        .expect("spawn ruscker");

    // Always tear down, even on assert panic.
    let result = run_checks().await;

    let _ = child.kill();
    let _ = child.wait();
    cleanup_containers();
    let _ = std::fs::remove_dir_all(&dir);

    result.expect("e2e checks");
}

async fn run_checks() -> anyhow::Result<()> {
    // 1. Wait for the server to come up (liveness).
    let start = Instant::now();
    loop {
        if let Ok((200, _)) = http_get("/healthz", Duration::from_secs(2)).await {
            break;
        }
        if start.elapsed() > Duration::from_secs(20) {
            anyhow::bail!("server never became healthy");
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    // 2. First request spawns the container (pull + run + wait-ready),
    //    then forwards to echo-server through the sub-path. Generous
    //    timeout for the cold spawn.
    let (status, body) = http_get(&format!("/app/{SPEC}/"), Duration::from_secs(45)).await?;
    anyhow::ensure!(status == 200, "GET /app/{SPEC}/ → {status}, body: {body}");
    anyhow::ensure!(
        body.contains("Request served by"),
        "echo body missing marker: {body}"
    );

    // 3. WebSocket through the sub-path: the upgrade must complete via
    //    Ruscker's pump, and echo-server sends an initial frame.
    let url = format!("ws://{HOST}:{PORT}/app/{SPEC}/.ws");
    let (mut ws, resp) = tokio::time::timeout(
        Duration::from_secs(15),
        tokio_tungstenite::connect_async(&url),
    )
    .await
    .map_err(|_| anyhow::anyhow!("ws connect timed out"))??;
    anyhow::ensure!(
        resp.status() == 101,
        "ws upgrade status {} (expected 101)",
        resp.status()
    );

    let frame = tokio::time::timeout(Duration::from_secs(10), ws.next())
        .await
        .map_err(|_| anyhow::anyhow!("no ws frame within 10s"))?;
    anyhow::ensure!(
        matches!(frame, Some(Ok(_))),
        "expected a ws frame, got {frame:?}"
    );

    Ok(())
}
