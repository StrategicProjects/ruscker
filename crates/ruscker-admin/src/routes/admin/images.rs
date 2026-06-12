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
    extract::{DefaultBodyLimit, Multipart, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Form, Json, Router,
};
use ruscker_config::Spec;
use serde::Deserialize;

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
        .route("/admin/media/{id}/rename", post(rename))
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
    /// Concatenated reference strings (every spec's logo + cover, plus the
    /// landing header/footer logo URLs). An image is "in use" when its
    /// `/assets/img/<filename>` appears here — drives the delete warning
    /// (#433).
    refs: String,
    /// Set on successful upload — drives a one-shot toast.
    flash_uploaded: Option<String>,
    /// Set when the upload REPLACED an existing image with the same
    /// filename (#815) — drives the "replaced" note.
    flash_replaced: bool,
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
                    // #433: referenced by a card (logo/cover) or a landing logo?
                    "in_use": self.refs.contains(&format!("/assets/img/{}", img.filename)),
                })
            })
            .collect();
        serde_json::to_string(&arr).unwrap_or_else(|_| "[]".into())
    }

}

#[derive(Deserialize)]
struct MediaQuery {
    /// Error code from a failed rename redirect (`taken` | `invalid`).
    #[serde(default)]
    error: Option<String>,
}

async fn index(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(q): Query<MediaQuery>,
) -> Response {
    // A rename can bounce back with an error code; localize it into the
    // shared error flash. Success is silent — the renamed tile is visible.
    let flash_error = q.error.as_deref().map(|code| {
        let key = match code {
            "taken" => "admin-images-rename-taken",
            _ => "admin-images-rename-invalid",
        };
        state.locales.t(loc, key, None)
    });
    render_index(&state, loc, theme, editor.role, None, false, flash_error).await
}

async fn render_index(
    state: &AppState,
    loc: Locale,
    theme: Theme,
    role: Role,
    flash_uploaded: Option<String>,
    flash_replaced: bool,
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
    // Collect every place an image URL can be referenced, so the gallery
    // can warn before deleting one that's in use (#433): each spec's logo +
    // cover, and the landing header/footer logos.
    let mut refs = String::new();
    if let Ok(specs) = db::specs::list_all(pool).await {
        for s in &specs {
            for key in ["logo", "cover"] {
                if let Some(v) = s.template_properties.0.get(key).and_then(|v| v.as_str()) {
                    refs.push_str(v);
                    refs.push('\n');
                }
            }
        }
    }
    if let Ok(lc) = db::landing::fetch(pool).await {
        for logo in &lc.logos {
            refs.push_str(&logo.url);
            refs.push('\n');
        }
    }
    let page = ImagesPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "images",
        role,
        images,
        refs,
        flash_uploaded,
        flash_replaced,
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
    let mut last_replaced = false;
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
        // Same name ⇒ REPLACE the stored image (#815). Re-uploading a
        // file under an existing name means "update it" — the old
        // auto-rename to `logo-2.webp` kept every spec/landing reference
        // pointing at the stale image (the reported bug).
        // `db::images::insert` swaps the row in one transaction; the
        // thumb/digest invalidation below changes the ETag, and
        // `/assets/img` serves max-age=60 + must-revalidate, so the new
        // bytes show on the next revalidation. The flash tells the
        // operator a replacement happened.
        let replaced = db::images::filename_taken(pool, &processed.filename)
            .await
            .unwrap_or(false);
        let stored_name = processed.filename.clone();
        match db::images::insert(pool, processed, Some(editor.actor())).await {
            Ok(_id) => {
                // Drop any cached thumbnail keyed by this filename (#301).
                crate::routes::assets::invalidate_thumb(&stored_name);
                last_uploaded_name = Some(stored_name);
                last_replaced = replaced;
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
        last_replaced,
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
        // Same name ⇒ REPLACE (#815), like the Media page: the insert
        // swaps the row and the invalidation below refreshes the
        // thumb/ETag, so the picker (and every existing reference)
        // shows the new bytes.
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

/// Asset path of the built-in Ruscker mark — what a card falls back to
/// when its image is deleted (#560), so it never renders a broken image.
/// Seeded into Media as a built-in (#433), so it's always available.
const RUSCKER_DEFAULT_LOGO: &str = "/assets/img/ruscker-mark.svg";

/// True when a `logo`/`cover` value points at `filename`.
fn references_image(value: &str, filename: &str) -> bool {
    value == filename || value.contains(&format!("/assets/img/{filename}"))
}

/// Rewrite a `logo`/`cover`/landing URL that referenced `old` to point at
/// `new`, in both the bare-filename and `/assets/img/<file>` forms.
fn rewrite_reference(value: &str, old: &str, new: &str) -> String {
    if value == old {
        return new.to_string();
    }
    value.replace(
        &format!("/assets/img/{old}"),
        &format!("/assets/img/{new}"),
    )
}

#[derive(Deserialize)]
struct RenameForm {
    newname: String,
}

/// Rename a Media image. The new name keeps the original extension (the
/// BLOB is unchanged); a collision is rejected (not silently suffixed —
/// the operator chose this name). Every spec logo/cover and landing logo
/// that referenced the old name is rewritten to the new one, so cards
/// don't break.
async fn rename(
    editor: RequireEditor,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<RenameForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let old = match db::images::filename_for(pool, &id).await {
        Ok(Some(f)) => f,
        Ok(None) => return Redirect::to("/admin/media").into_response(),
        Err(err) => {
            tracing::error!(error = ?err, id, "rename lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "lookup failed").into_response();
        }
    };

    // Keep the original extension; sanitize the operator's stem.
    let old_ext = old.rsplit_once('.').map(|(_, e)| e).unwrap_or("");
    let san = crate::images::sanitize_basename(&form.newname);
    let new = if old_ext.is_empty() {
        san.clone()
    } else {
        crate::images::rewrite_extension(&san, old_ext)
    };
    if san.trim_matches('.').is_empty() {
        return Redirect::to("/admin/media?error=invalid").into_response();
    }
    if new == old {
        return Redirect::to("/admin/media").into_response(); // no-op
    }
    match db::images::filename_taken(pool, &new).await {
        Ok(true) => return Redirect::to("/admin/media?error=taken").into_response(),
        Ok(false) => {}
        Err(err) => {
            tracing::error!(error = ?err, "rename collision check failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "check failed").into_response();
        }
    }

    // Compute the rewritten spec + landing references in memory, then
    // commit the image rename together with EVERY reference in a single
    // transaction (#720 audit P2). Previously each rewrite was its own
    // transaction and the image rename came last, so a failure there left
    // cards pointing at a filename that no longer existed.
    let specs_to_update: Vec<Spec> = match db::specs::list_all(pool).await {
        Ok(specs) => specs
            .into_iter()
            .filter_map(|mut spec| {
                let logo_hit = spec
                    .template_properties
                    .get_str("logo")
                    .is_some_and(|v| references_image(v, &old));
                let cover_hit = spec
                    .template_properties
                    .get_str("cover")
                    .is_some_and(|v| references_image(v, &old));
                if !logo_hit && !cover_hit {
                    return None;
                }
                if let Some(v) = spec.template_properties.get_str("logo").filter(|_| logo_hit) {
                    let nv = rewrite_reference(v, &old, &new);
                    spec.template_properties.set_str("logo", &nv);
                }
                if let Some(v) = spec.template_properties.get_str("cover").filter(|_| cover_hit) {
                    let nv = rewrite_reference(v, &old, &new);
                    spec.template_properties.set_str("cover", &nv);
                }
                Some(spec)
            })
            .collect(),
        Err(err) => {
            tracing::error!(error = ?err, "list specs for rename failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "rename failed").into_response();
        }
    };
    let landing = match db::landing::fetch(pool).await {
        Ok(mut lc) => {
            let mut touched = false;
            for logo in &mut lc.logos {
                if references_image(&logo.url, &old) {
                    logo.url = rewrite_reference(&logo.url, &old, &new);
                    touched = true;
                }
            }
            touched.then_some(lc)
        }
        Err(err) => {
            tracing::error!(error = ?err, "fetch landing for rename failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "rename failed").into_response();
        }
    };

    match db::images::rename_with_refs(
        pool,
        &id,
        &new,
        &specs_to_update,
        landing.as_ref(),
        Some(editor.actor()),
    )
    .await
    {
        Ok(()) => {
            crate::routes::assets::invalidate_thumb(&old);
            crate::routes::assets::invalidate_thumb(&new);
            tracing::info!(id, %old, %new, specs = specs_to_update.len(),
                "image renamed atomically; references rewritten");
            Redirect::to("/admin/media").into_response()
        }
        // The in-tx collision re-check rolled the whole batch back: the
        // target name was taken between the pre-check and the commit.
        Err(err) if err.to_string().contains("taken") => {
            tracing::warn!(error = ?err, id, "image rename rolled back (name taken)");
            Redirect::to("/admin/media?error=taken").into_response()
        }
        Err(err) => {
            tracing::error!(error = ?err, id, "image rename failed (rolled back)");
            (StatusCode::INTERNAL_SERVER_ERROR, "rename failed").into_response()
        }
    }
}

async fn delete(
    editor: RequireEditor,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };

    // Find the filename first, so we can fix up any app that used it before
    // it's gone — otherwise that card would render a broken image (#560).
    let filename = match db::images::filename_for(pool, &id).await {
        Ok(Some(f)) => f,
        Ok(None) => return Redirect::to("/admin/media").into_response(), // already gone
        Err(err) => {
            tracing::error!(error = ?err, id, "image lookup failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "lookup failed").into_response();
        }
    };

    // Reset every reference in memory, then delete the image AND commit
    // the resets in a single transaction (#720 audit P2) — a referencing
    // spec `logo` falls back to the Ruscker mark, a `cover` is cleared
    // (kind tint), and a landing logo url is cleared. Doing it atomically
    // means a failure can't leave a card pointing at a gone image (or
    // remove the image while a card still references it).
    let mut reset_specs: Vec<Spec> = Vec::new();
    match db::specs::list_all(pool).await {
        Ok(specs) => {
            for mut spec in specs {
                let logo_hit = spec
                    .template_properties
                    .get_str("logo")
                    .is_some_and(|v| references_image(v, &filename));
                let cover_hit = spec
                    .template_properties
                    .get_str("cover")
                    .is_some_and(|v| references_image(v, &filename));
                if !logo_hit && !cover_hit {
                    continue;
                }
                if logo_hit {
                    spec.template_properties
                        .set_str("logo", RUSCKER_DEFAULT_LOGO);
                }
                if cover_hit {
                    spec.template_properties.remove("cover");
                }
                reset_specs.push(spec);
            }
        }
        Err(err) => {
            tracing::error!(error = ?err, "list specs for delete failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response();
        }
    }
    let landing = match db::landing::fetch(pool).await {
        Ok(mut lc) => {
            let mut touched = false;
            for logo in &mut lc.logos {
                if references_image(&logo.url, &filename) {
                    logo.url = String::new();
                    touched = true;
                }
            }
            touched.then_some(lc)
        }
        Err(err) => {
            tracing::error!(error = ?err, "fetch landing for delete failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response();
        }
    };

    match db::images::delete_with_refs(pool, &id, &reset_specs, landing.as_ref(), Some(editor.actor()))
        .await
    {
        Ok(name) => {
            if let Some(n) = name {
                crate::routes::assets::invalidate_thumb(&n);
            }
            tracing::info!(id, %filename, reset = reset_specs.len(),
                "image deleted atomically; references reset");
            Redirect::to("/admin/media").into_response()
        }
        Err(err) => {
            tracing::error!(error = ?err, id, "image delete failed (rolled back)");
            (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response()
        }
    }
}
