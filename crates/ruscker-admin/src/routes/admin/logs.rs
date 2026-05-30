//! Admin > Logs — the Ruscker process log (the `tracing` stream),
//! tailed from the in-memory ring buffer (#100). Read-only; admin-only.
//!
//! `GET /admin/logs` renders the current snapshot; `GET
//! /admin/logs/stream` is an SSE feed of new lines (the page follows).

use std::convert::Infallible;
use std::time::Duration;

use askama::Template;
use axum::extract::State;
use axum::http::header;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use futures_util::Stream;

use crate::auth::{RequireAdmin, Role};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

const TICK: Duration = Duration::from_secs(1);
const KEEPALIVE: Duration = Duration::from_secs(15);
/// How many of the most recent lines the initial page renders. The SSE
/// stream appends anything newer; the full buffer is a download away.
/// Matches the per-replica logs viewer's tail (#200).
const INITIAL_TAIL: usize = 500;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/logs", get(index))
        .route("/admin/logs/stream", get(stream))
        .route("/admin/logs/download", get(download))
}

#[derive(Template)]
#[template(path = "admin/process_logs.html")]
struct LogsPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// Mount prefix for base-path-correct URLs (#294).
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    /// Current session role (always Admin here) - drives nav gating.
    role: Role,
    /// The most recent buffered lines (capped at [`INITIAL_TAIL`]),
    /// oldest-first. Empty when no buffer is wired (e.g. started without
    /// the CLI's tracing layer).
    lines: Vec<String>,
    /// Total lines currently buffered — drives the "showing last N of M"
    /// notice + download affordance when the render is truncated.
    total: usize,
    /// Whether a log buffer is actually wired.
    available: bool,
}

impl LogsPage<'_> {
    /// Whether the render is a truncated tail of a larger buffer.
    fn truncated(&self) -> bool {
        self.total > self.lines.len()
    }
}

impl LogsPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

async fn index(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    let (lines, total, available) = match &state.log_buffer {
        // Only the most recent slice — the SSE stream appends what's
        // newer, and the full buffer is one download away (#200).
        Some(b) => {
            let (lines, total) = b.tail(INITIAL_TAIL);
            (lines, total, true)
        }
        None => (Vec::new(), 0, false),
    };
    super::render(&LogsPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "logs",
        role: Role::Admin,
        lines,
        total,
        available,
    })
}

/// Full buffer as a `text/plain` attachment — for forensics, since the
/// page renders only the recent tail. Admin-gated like the rest.
async fn download(_: RequireAdmin, State(state): State<AppState>) -> Response {
    let body = match &state.log_buffer {
        Some(b) => b.snapshot().join("\n"),
        None => String::new(),
    };
    (
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"ruscker.log\"",
            ),
        ],
        body,
    )
        .into_response()
}

/// SSE feed of lines appended since the client connected. The page
/// already rendered the snapshot, so we start from the live cursor and
/// only stream what's new.
async fn stream(
    _: RequireAdmin,
    State(state): State<AppState>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    use async_stream::stream;
    let buf = state.log_buffer.clone();
    let s = stream! {
        let Some(buf) = buf else {
            // No buffer wired — nothing to stream; keep-alive holds the
            // connection so the client doesn't error-loop.
            std::future::pending::<()>().await;
            return;
        };
        let mut cursor = buf.cursor();
        let mut ticker = tokio::time::interval(TICK);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            let (new, next) = buf.since(cursor);
            cursor = next;
            if !new.is_empty() {
                yield Ok::<_, Infallible>(Event::default().data(new.join("\n")));
            }
        }
    };
    Sse::new(s).keep_alive(KeepAlive::new().interval(KEEPALIVE))
}
