//! Admin > image library — gallery, upload, delete.
//!
//! Upload pipeline:
//!   1. axum::Multipart streams the form field
//!   2. `images::process_upload` sniffs MIME, decodes,
//!      re-encodes PNG/JPEG to WebP (SVG passes through)
//!   3. `db::images::insert` writes the BLOB + audit row
//!   4. Redirect back to /admin/images
//!
//! Body cap of 12 MB applied per route — slightly above the
//! 10 MB cap inside `process_upload` to account for multipart
//! framing overhead.

use askama::Template;
use axum::{
    extract::{DefaultBodyLimit, Multipart, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};

use crate::auth::AdminSession;
use crate::db;
use crate::i18n::{Locale, Locales};
use crate::images;
use crate::theme::Theme;
use crate::AppState;

const UPLOAD_BODY_LIMIT: usize = 12 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/images", get(index).post(upload))
        .route("/admin/images/{id}/delete", post(delete))
        .layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT))
}

#[derive(Template)]
#[template(path = "admin/images.html")]
struct ImagesPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    nav_section: &'static str,
    images: Vec<db::images::ImageMeta>,
    /// Set on successful upload — drives a one-shot toast.
    flash_uploaded: Option<String>,
    flash_error: Option<String>,
}

impl<'a> ImagesPage<'a> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// "12.3 KB" / "1.4 MB" — for the size column. Takes `&i64`
    /// so the template can pass the borrowed field straight in
    /// without writing `*img.size_bytes`.
    fn fmt_size(&self, bytes: &i64) -> String {
        let b = *bytes;
        if b < 1024 {
            format!("{} B", b)
        } else if b < 1024 * 1024 {
            format!("{:.1} KB", b as f64 / 1024.0)
        } else {
            format!("{:.1} MB", b as f64 / 1024.0 / 1024.0)
        }
    }
}

async fn index(
    _: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    render_index(&state, loc, theme, None, None).await
}

async fn render_index(
    state: &AppState,
    loc: Locale,
    theme: Theme,
    flash_uploaded: Option<String>,
    flash_error: Option<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            "database not attached — start with --db <path>",
        )
            .into_response();
    };
    let images = match db::images::list_all(pool).await {
        Ok(v) => v,
        Err(err) => {
            tracing::error!(error = ?err, "list images failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let page = ImagesPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        nav_section: "images",
        images,
        flash_uploaded,
        flash_error,
    };
    super::render(&page)
}

async fn upload(
    _: AdminSession,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    mut multipart: Multipart,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };

    // The form has a single file field named `file`. We iterate
    // through everything and pick the first one whose field_name
    // matches; other fields (CSRF token, etc.) are ignored.
    let mut last_uploaded_name: Option<String> = None;
    let mut last_error: Option<String> = None;

    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(err) => {
                last_error = Some(format!("multipart parse error: {err}"));
                break;
            }
        };
        if field.name() != Some("file") {
            continue;
        }
        let filename = field.file_name().unwrap_or("upload").to_string();
        let mime = field.content_type().map(|s| s.to_string());
        let bytes = match field.bytes().await {
            Ok(b) => b.to_vec(),
            Err(err) => {
                last_error = Some(format!("read upload: {err}"));
                continue;
            }
        };
        if bytes.is_empty() {
            continue; // empty file input — nothing chosen
        }

        let processed = match images::process_upload(&filename, mime.as_deref(), bytes) {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!(error = ?err, filename, "rejecting upload");
                last_error = Some(err.to_string());
                continue;
            }
        };
        let stored_name = processed.filename.clone();
        match db::images::insert(pool, processed, Some("admin")).await {
            Ok(_id) => {
                last_uploaded_name = Some(stored_name);
            }
            Err(err) => {
                tracing::error!(error = ?err, "image insert failed");
                last_error = Some(format!("save: {err}"));
            }
        }
    }

    render_index(&state, loc, theme, last_uploaded_name, last_error).await
}

async fn delete(
    _: AdminSession,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    match db::images::delete_one(pool, &id, Some("admin")).await {
        Ok(_) => Redirect::to("/admin/images").into_response(),
        Err(err) => {
            tracing::error!(error = ?err, id, "image delete failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response()
        }
    }
}
