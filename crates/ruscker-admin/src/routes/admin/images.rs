//! Admin > image library — gallery, upload, delete.
//!
//! Upload pipeline:
//!   1. axum::Multipart streams the form field
//!   2. `images::process_upload_async` sniffs MIME and runs bounded decode,
//!      re-encodes PNG/JPEG to WebP (SVG passes through)
//!   3. `db::images::insert` writes the BLOB + audit row
//!   4. Redirect back to /admin/images
//!
//! Batch import (`/admin/media/import-zip`): a `.zip` is unpacked by
//! `media_zip::extract_images` and every image in it goes through steps
//! 2–3 exactly like a single upload.
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

use super::KpiMetric;

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
        // The batch import has its own, larger body limit: one archive
        // carries many images. Merged after the layer above so that limit
        // does not apply to it.
        .merge(
            Router::new()
                .route("/admin/media/import-zip", post(import_zip))
                .layer(DefaultBodyLimit::max(ZIP_BODY_LIMIT)),
        )
}

/// The archive itself plus multipart framing.
const ZIP_BODY_LIMIT: usize = crate::media_zip::MAX_ZIP_BYTES + 1024 * 1024;

/// What the page says once, after a form post.
#[derive(Default)]
struct Flash {
    /// Set on successful upload — drives a one-shot toast.
    uploaded: Option<String>,
    /// The originally-requested name when the upload was auto-renamed to
    /// avoid overwriting an existing image — drives the "renamed" note.
    renamed: Option<String>,
    error: Option<String>,
    /// The outcome of a batch import.
    import: Option<ImportFlash>,
}

/// The outcome of a `.zip` import, already in the viewer's language.
struct ImportFlash {
    /// "12 imported · 3 already in the library · 1 not imported".
    summary: String,
    /// The first few files that were not imported, as `name — reason`.
    problems: Vec<String>,
    /// How many more problems there were than `problems` shows.
    more: usize,
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
    kpi_files: i64,
    kpi_in_use: i64,
    kpi_total_bytes: i64,
    /// Concatenated reference strings (every spec's logo + cover, plus the
    /// landing header/footer logo URLs). An image is "in use" when its
    /// `/assets/img/<filename>` appears here — drives the delete warning
    /// (#433).
    refs: String,
    flash_uploaded: Option<String>,
    flash_renamed: Option<String>,
    flash_error: Option<String>,
    flash_import: Option<ImportFlash>,
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

    fn kpis(&self) -> [KpiMetric; 3] {
        [
            KpiMetric::new("ti-photo", "admin-images-kpi-files", self.kpi_files),
            KpiMetric::new("ti-circle-check", "admin-images-kpi-in-use", self.kpi_in_use),
            KpiMetric::new(
                "ti-database",
                "admin-images-kpi-total-size",
                self.fmt_size(&self.kpi_total_bytes),
            ),
        ]
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
    let flash = Flash { error: flash_error, ..Flash::default() };
    render_index(&state, loc, theme, editor.role, flash).await
}

async fn render_index(
    state: &AppState,
    loc: Locale,
    theme: Theme,
    role: Role,
    flash: Flash,
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
    {
        // #902: reuse the cached effective catalog instead of a fresh
        // full-deserialize just to scrape logo/cover URLs.
        let specs = crate::catalog::effective_specs_cached(state).await;
        for s in specs.iter() {
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
        // The intro banner background (#1065) stores a bare filename.
        if let Some(f) = lc.effective_intro_image() {
            refs.push_str(&format!("/assets/img/{f}\n"));
        }
        // So does the sign-in page's side image (#1083).
        if let Some(f) = lc.effective_login_image() {
            refs.push_str(&format!("/assets/img/{f}\n"));
        }
    }
    // SSO client logos (#1080) store a bare filename too.
    if let Ok(clients) = db::oidc_clients::list_all(pool).await {
        for f in clients.iter().filter_map(|c| c.logo.as_deref()) {
            refs.push_str(&format!("/assets/img/{f}\n"));
        }
    }
    let kpi_files = images.len() as i64;
    let kpi_in_use = images
        .iter()
        .filter(|image| refs.contains(&format!("/assets/img/{}", image.filename)))
        .count() as i64;
    let kpi_total_bytes = images.iter().map(|image| image.size_bytes).sum();
    let page = ImagesPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "images",
        role,
        images,
        kpi_files,
        kpi_in_use,
        kpi_total_bytes,
        refs,
        flash_uploaded: flash.uploaded,
        flash_renamed: flash.renamed,
        flash_error: flash.error,
        flash_import: flash.import,
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
    let mut last_renamed_from: Option<String> = None;
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

        let mut processed =
            match images::process_upload_async(filename.clone(), mime, bytes).await {
                Ok(p) => p,
                Err(err) => {
                    tracing::warn!(error = ?err, filename, "rejecting upload");
                    last_error = Some(err.to_string());
                    continue;
                }
            };
        // Don't silently overwrite a same-named image: pick a free name
        // (`logo.webp` → `logo-2.webp`) so both are kept, and tell the
        // operator it was renamed. The import path keeps exact names.
        let desired = processed.filename.clone();
        let unique = match db::images::unique_filename(pool, &desired).await {
            Ok(u) => u,
            Err(err) => {
                tracing::error!(error = ?err, "unique filename failed");
                last_error = Some(format!("save: {err}"));
                continue;
            }
        };
        if unique != desired {
            processed.filename = unique.clone();
            last_renamed_from = Some(desired);
        }
        let stored_name = processed.filename.clone();
        match db::images::insert(pool, processed, Some(editor.actor())).await {
            Ok(_id) => {
                // Drop any cached thumbnail keyed by this filename (#301).
                crate::routes::assets::invalidate_thumb(&stored_name);
                last_uploaded_name = Some(stored_name);
            }
            Err(err) => {
                tracing::error!(error = ?err, "image insert failed");
                last_error = Some(format!("save: {err}"));
            }
        }
    }

    let flash = Flash {
        uploaded: last_uploaded_name,
        renamed: last_renamed_from,
        error: last_error,
        import: None,
    };
    render_index(&state, loc, theme, editor.role, flash).await
}

/// How many "not imported" files the page lists by name; the rest is a
/// count.
const IMPORT_PROBLEMS_SHOWN: usize = 8;

/// Batch import: one `.zip` (form field `zip`) whose images each go
/// through the single-upload pipeline. Three rules worth knowing:
///
/// - **Re-importing the same archive adds nothing.** A file whose name and
///   stored bytes are already in the library is counted as "already
///   there", so an archive can be sent again after adding logos to it.
/// - **A same-named file with DIFFERENT content keeps both** (`logo.svg` →
///   `logo-2.svg`), the same rule as a single upload: an image in use is
///   never replaced behind the cards that show it.
/// - **One bad file does not stop the rest.** It is reported by name.
async fn import_zip(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    mut multipart: Multipart,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let t = |key: &str| state.locales.t(loc, key, None);
    let fail = |key: &str| Flash { error: Some(t(key)), ..Flash::default() };

    // The archive: the first non-empty `zip` field.
    let mut zip_bytes: Option<axum::body::Bytes> = None;
    loop {
        match multipart.next_field().await {
            Ok(Some(field)) if field.name() == Some("zip") => match field.bytes().await {
                Ok(b) if !b.is_empty() => {
                    zip_bytes = Some(b);
                    break;
                }
                Ok(_) => {}
                // Over the body limit, or the connection dropped.
                Err(_) => {
                    return render_index(&state, loc, theme, editor.role, fail("admin-images-import-too-large")).await;
                }
            },
            Ok(Some(_)) => {}
            Ok(None) => break,
            Err(_) => {
                return render_index(&state, loc, theme, editor.role, fail("admin-images-import-too-large")).await;
            }
        }
    }
    let Some(zip_bytes) = zip_bytes else {
        return render_index(&state, loc, theme, editor.role, fail("admin-images-import-not-zip")).await;
    };

    // One import at a time (its files sit in memory until stored), and the
    // unpacking itself under the shared budget of blocking image jobs.
    let _turn = match images::zip_import_turn().await {
        Ok(turn) => turn,
        Err(err) => {
            tracing::error!(error = ?err, "zip import turn unavailable");
            return (StatusCode::SERVICE_UNAVAILABLE, "import unavailable").into_response();
        }
    };
    let extracted = images::extract_zip_async(zip_bytes).await;
    let extracted = match extracted {
        Ok(Ok(e)) => e,
        Ok(Err(refusal)) => {
            use crate::media_zip::ZipRefusal;
            let key = match refusal {
                ZipRefusal::NotAZip => "admin-images-import-not-zip",
                ZipRefusal::TooManyEntries => "admin-images-import-too-many",
                ZipRefusal::TooLargeUnpacked => "admin-images-import-too-large",
            };
            return render_index(&state, loc, theme, editor.role, fail(key)).await;
        }
        Err(err) => {
            tracing::error!(error = ?err, "zip extraction task failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "import failed").into_response();
        }
    };

    // An archive with no image in it at all: say that, not "0 imported".
    if extracted.files.is_empty() && extracted.skipped.is_empty() {
        return render_index(&state, loc, theme, editor.role, fail("admin-images-import-empty")).await;
    }

    let (mut imported, mut already) = (0usize, 0usize);
    // (file name, reason) for everything that looked like an image and
    // did not make it in.
    let mut problems: Vec<(String, String)> = extracted
        .skipped
        .into_iter()
        .map(|(name, why)| {
            let key = match why {
                crate::media_zip::SkipReason::TooLarge => "admin-images-import-skip-large",
                crate::media_zip::SkipReason::Unreadable => "admin-images-import-skip-unreadable",
            };
            (name, t(key))
        })
        .collect();
    for file in extracted.files {
        let name = file.name.clone();
        let mut processed = match images::process_upload_async(file.name, None, file.bytes).await {
            Ok(p) => p,
            Err(err) => {
                tracing::warn!(error = ?err, filename = %name, "zip import: file rejected");
                problems.push((name, err.to_string()));
                continue;
            }
        };
        // Same name already in the library: the same bytes mean it is
        // already there; different bytes get a free name.
        match db::images::fetch_by_filename(pool, &processed.filename).await {
            Ok(Some((_, stored))) if stored == processed.bytes => {
                already += 1;
                continue;
            }
            Ok(Some(_)) => match db::images::unique_filename(pool, &processed.filename).await {
                Ok(unique) => processed.filename = unique,
                Err(err) => {
                    tracing::error!(error = ?err, "unique filename failed");
                    problems.push((name, t("admin-images-import-skip-save")));
                    continue;
                }
            },
            Ok(None) => {}
            Err(err) => {
                tracing::error!(error = ?err, "zip import: lookup failed");
                problems.push((name, t("admin-images-import-skip-save")));
                continue;
            }
        }
        let stored_name = processed.filename.clone();
        match db::images::insert(pool, processed, Some(editor.actor())).await {
            Ok(_) => {
                crate::routes::assets::invalidate_thumb(&stored_name);
                imported += 1;
            }
            Err(err) => {
                tracing::error!(error = ?err, "zip import: insert failed");
                problems.push((name, t("admin-images-import-skip-save")));
            }
        }
    }
    tracing::info!(
        actor = editor.actor(),
        imported,
        already,
        not_imported = problems.len(),
        other_files = extracted.not_images,
        "media zip import"
    );

    let mut args = fluent_bundle::FluentArgs::new();
    args.set("imported", imported as i64);
    args.set("already", already as i64);
    args.set("failed", problems.len() as i64);
    let summary = state.locales.t(loc, "admin-images-import-summary", Some(&args));
    let more = problems.len().saturating_sub(IMPORT_PROBLEMS_SHOWN);
    let problems = problems
        .into_iter()
        .take(IMPORT_PROBLEMS_SHOWN)
        .map(|(name, why)| format!("{name} — {why}"))
        .collect();
    let flash = Flash {
        import: Some(ImportFlash { summary, problems, more }),
        ..Flash::default()
    };
    render_index(&state, loc, theme, editor.role, flash).await
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
        let mut processed = match images::process_upload_async(filename, mime, bytes).await {
            Ok(p) => p,
            Err(err) => return inline_err(StatusCode::UNPROCESSABLE_ENTITY, err.to_string()),
        };
        // Auto-rename on collision so we never overwrite an existing image;
        // the JSON returns the actual stored name, which the picker uses.
        match db::images::unique_filename(pool, &processed.filename).await {
            Ok(u) => processed.filename = u,
            Err(err) => {
                tracing::error!(error = ?err, "inline unique filename failed");
                return inline_err(StatusCode::INTERNAL_SERVER_ERROR, "save failed");
            }
        }
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
            // Intro banner background (#1065): a bare filename, exact match.
            if lc.effective_intro_image() == Some(old.as_str()) {
                lc.intro_image = Some(new.clone());
                touched = true;
            }
            if lc.effective_login_image() == Some(old.as_str()) {
                lc.login_image = Some(new.clone());
                touched = true;
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
            // Intro banner background (#1065): drop it with the image.
            if lc.effective_intro_image() == Some(filename.as_str()) {
                lc.intro_image = None;
                touched = true;
            }
            if lc.effective_login_image() == Some(filename.as_str()) {
                lc.login_image = None;
                touched = true;
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
