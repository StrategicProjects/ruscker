//! Admin > Logs — the Ruscker process log (the `tracing` stream),
//! tailed from the in-memory ring buffer (#100). Read-only; admin-only.
//!
//! `GET /admin/logs` renders the current snapshot; short requests to
//! `GET /admin/logs/poll` fetch new lines without occupying a persistent
//! HTTP/1.1 connection (#1039).

use askama::Template;
use axum::extract::{Query, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::auth::{RequireAdmin, Role};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

/// How many of the most recent lines the initial page renders. Polling
/// appends anything newer; the full buffer is a download away.
/// Matches the per-replica logs viewer's tail (#200).
const INITIAL_TAIL: usize = 500;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/logs", get(index))
        .route("/admin/logs/poll", get(poll))
        .route("/admin/logs/stream", get(retired_stream))
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
    /// First polling cursor, captured atomically with `lines`.
    cursor: u64,
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
    let (lines, total, available, cursor) = match &state.log_buffer {
        // Only the most recent slice — polling appends what's
        // newer, and the full buffer is one download away (#200).
        Some(b) => {
            let (lines, total, cursor) = b.tail_with_cursor(INITIAL_TAIL);
            (lines, total, true, cursor)
        }
        None => (Vec::new(), 0, false, 0),
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
        cursor,
    })
}

#[derive(Deserialize)]
struct PollQuery {
    #[serde(default)]
    cursor: u64,
}

#[derive(Serialize)]
struct PollResponse {
    lines: Vec<String>,
    /// A decimal string avoids losing u64 precision in JavaScript.
    cursor: String,
    available: bool,
}

/// Return immediately with all lines newer than `cursor`. This deliberately
/// stays a finite request: an intermediary that only speaks HTTP/1.1 cannot
/// strand an infinite response and head-of-line block later admin navigation.
async fn poll(
    _: RequireAdmin,
    State(state): State<AppState>,
    Query(query): Query<PollQuery>,
) -> Response {
    let (lines, cursor, available) = match &state.log_buffer {
        Some(buffer) => {
            let (lines, cursor) = buffer.since(query.cursor);
            (lines, cursor, true)
        }
        None => (Vec::new(), query.cursor, false),
    };
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(PollResponse {
            lines,
            cursor: cursor.to_string(),
            available,
        }),
    )
        .into_response()
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

/// Stop pre-upgrade EventSource clients from reconnecting forever during a
/// rolling deployment. EventSource treats 204 as a terminal response.
async fn retired_stream(_: RequireAdmin) -> StatusCode {
    StatusCode::NO_CONTENT
}
