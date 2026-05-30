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
    Json, Router,
};

use crate::auth::{RequireEditor, Role};
use crate::db;
use crate::i18n::{Locale, Locales};
use crate::images;
use crate::theme::Theme;
use crate::AppState;

const UPLOAD_BODY_LIMIT: usize = 12 * 1024 * 1024;

pub fn routes() -> Router<AppState> {
    Router::new()
        // The media library — card logos/covers. Mounted at
        // `/admin/media` (the nav label is "Media") to avoid the
        // "Docker images" confusion (#9). The DB table + Rust
        // module stay `images` — that's internal and accurate.
        .route("/admin/media", get(index).post(upload))
        // Inline upload from the spec form (#213): same pipeline, JSON
        // response so the form can add + select the image without a
        // full page navigation to the media library.
        .route("/admin/media/upload-inline", post(upload_inline))
        .route("/admin/media/{id}/delete", post(delete))
        // 301 from the old path so any bookmarks / in-flight
        // links keep working.
        .route(
            "/admin/images",
            get(|| async { Redirect::permanent("/admin/media") }),
        )
        .layer(DefaultBodyLimit::max(UPLOAD_BODY_LIMIT))
}

#[derive(Template)]
#[template(path = "admin/images.html")]
struct ImagesPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// Mount prefix for base-path-correct URLs (#294).
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    /// Current session role (Editor or Admin) — drives nav gating.
    role: Role,
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

    /// The gallery as a JSON array, seeding the Alpine component that
    /// renders only a page of tiles at a time + filters by filename
    /// (#317). Same "data in `x-data`, paginate client-side" shape as
    /// the spec-form logo picker (#293) — the whole list is small
    /// metadata (no blobs), so paging/filtering is instant in the
    /// browser and the DOM + thumbnail fetches stay bounded regardless
    /// of library size. `ImageMeta` isn't `Serialize`, so we build the
    /// objects here; `size` is pre-formatted so the template needs no
    /// per-row Rust helper. A `dims` string is `""` when unknown.
    fn images_json(&self) -> String {
        let arr: Vec<serde_json::Value> = self
            .images
            .iter()
            .map(|img| {
                let dims = match (img.width, img.height) {
                    (Some(w), Some(h)) => format!("{w}×{h}"),
                    _ => String::new(),
                };
                serde_json::json!({
                    "id": img.id,
                    "filename": img.filename,
                    "mime": img.mime_type,
                    "size": self.fmt_size(&img.size_bytes),
                    "dims": dims,
                })
            })
            .collect();
        serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
    }
}

async fn index(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    render_index(&state, loc, theme, editor.role, None, None).await
}

async fn render_index(
    state: &AppState,
    loc: Locale,
    theme: Theme,
    role: Role,
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
        base: state.base_path.clone(),
        nav_section: "images",
        role,
        images,
        flash_uploaded,
        flash_error,
    };
    super::render(&page)
}

async fn upload(
    editor: RequireEditor,
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
        match db::images::insert(pool, processed, Some(editor.actor())).await {
            Ok(_id) => {
                // A re-upload replaced the bytes behind this filename —
                // drop any cached thumbnail so the new image shows (#301).
                crate::routes::assets::invalidate_thumb(&stored_name);
                last_uploaded_name = Some(stored_name);
            }
            Err(err) => {
                tracing::error!(error = ?err, "image insert failed");
                last_error = Some(format!("save: {err}"));
            }
        }
    }

    render_index(
        &state,
        loc,
        theme,
        editor.role,
        last_uploaded_name,
        last_error,
    )
    .await
}

/// JSON shape returned by [`upload_inline`]. On success `filename` +
/// `url` are set; on failure `error` carries an operator-facing string.
#[derive(serde::Serialize)]
struct InlineUpload {
    #[serde(skip_serializing_if = "Option::is_none")]
    filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

fn inline_err(status: StatusCode, msg: impl Into<String>) -> Response {
    (
        status,
        Json(InlineUpload {
            filename: None,
            url: None,
            error: Some(msg.into()),
        }),
    )
        .into_response()
}

/// Upload a single image and return JSON (`{filename, url}`), used by
/// the spec form's inline picker. Same `process_upload` + `insert`
/// pipeline as [`upload`]; only the response shape differs.
async fn upload_inline(
    editor: RequireEditor,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return inline_err(StatusCode::SERVICE_UNAVAILABLE, "no database");
    };
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(err) => return inline_err(StatusCode::BAD_REQUEST, format!("multipart: {err}")),
        };
        if field.name() != Some("file") {
            continue;
        }
        let filename = field.file_name().unwrap_or("upload").to_string();
        let mime = field.content_type().map(|s| s.to_string());
        let bytes = match field.bytes().await {
            Ok(b) => b.to_vec(),
            Err(err) => return inline_err(StatusCode::BAD_REQUEST, format!("read upload: {err}")),
        };
        if bytes.is_empty() {
            continue;
        }
        let processed = match images::process_upload(&filename, mime.as_deref(), bytes) {
            Ok(p) => p,
            Err(err) => return inline_err(StatusCode::UNPROCESSABLE_ENTITY, err.to_string()),
        };
        let name = processed.filename.clone();
        return match db::images::insert(pool, processed, Some(editor.actor())).await {
            Ok(_) => {
                crate::routes::assets::invalidate_thumb(&name); // #301
                Json(InlineUpload {
                    url: Some(format!("/assets/img/{name}")),
                    filename: Some(name),
                    error: None,
                })
                .into_response()
            }
            Err(err) => {
                tracing::error!(error = ?err, "inline image insert failed");
                inline_err(StatusCode::INTERNAL_SERVER_ERROR, "save failed")
            }
        };
    }
    inline_err(StatusCode::BAD_REQUEST, "no file field in upload")
}

async fn delete(
    editor: RequireEditor,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    match db::images::delete_one(pool, &id, Some(editor.actor())).await {
        Ok(filename) => {
            // Drop the cached thumbnail of the deleted image (#301).
            if let Some(name) = filename {
                crate::routes::assets::invalidate_thumb(&name);
            }
            Redirect::to("/admin/media").into_response()
        }
        Err(err) => {
            tracing::error!(error = ?err, id, "image delete failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response()
        }
    }
}
