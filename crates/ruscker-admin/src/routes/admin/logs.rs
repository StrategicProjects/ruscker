//! Admin > Logs — the Ruscker process log (the `tracing` stream),
//! tailed from the in-memory ring buffer (#100). Read-only; admin-only.
//!
//! `GET /admin/logs` renders the current snapshot; `GET
//! /admin/logs/stream` is an SSE feed of new lines (the page follows).

use std::convert::Infallible;
use std::time::Duration;

use askama::Template;
use axum::extract::State;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use futures_util::Stream;

use crate::auth::AdminSession;
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

const TICK: Duration = Duration::from_secs(1);
const KEEPALIVE: Duration = Duration::from_secs(15);

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/logs", get(index))
        .route("/admin/logs/stream", get(stream))
}

#[derive(Template)]
#[template(path = "admin/process_logs.html")]
struct LogsPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    nav_section: &'static str,
    /// Current buffered lines, oldest-first. Empty when no buffer is
    /// wired (e.g. started without the CLI's tracing layer).
    lines: Vec<String>,
    /// Whether a log buffer is actually wired.
    available: bool,
}

impl LogsPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

async fn index(
    _: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    let (lines, available) = match &state.log_buffer {
        Some(b) => (b.snapshot(), true),
        None => (Vec::new(), false),
    };
    super::render(&LogsPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        nav_section: "logs",
        lines,
        available,
    })
}

/// SSE feed of lines appended since the client connected. The page
/// already rendered the snapshot, so we start from the live cursor and
/// only stream what's new.
async fn stream(
    _: AdminSession,
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
