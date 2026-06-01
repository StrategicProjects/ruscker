//! Admin > Disk — reclaim space left behind by stopped replicas and
//! unused images (#453 part B). Admin-only.
//!
//! Everything here is **label-scoped** on the container side
//! (`ruscker.replica_id`), so a prune never touches a non-Ruscker
//! container on a shared host. Image removal is offered only for images
//! no container uses and no current spec references, and runs without
//! `--force` so the daemon is the final backstop.

use askama::Template;
use axum::{
    extract::{Form, Query, State},
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use std::collections::HashSet;

use crate::auth::{RequireAdmin, Role};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;
use ruscker_core::{ImageInfo, ManagedContainer};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/disk", get(index))
        .route("/admin/disk/containers/remove", post(remove_container))
        .route("/admin/disk/containers/prune", post(prune_stopped))
        .route("/admin/disk/images/remove", post(remove_image))
        .route("/admin/disk/images/prune", post(prune_images))
}

/// One image row, enriched with whether it's safe to remove.
struct ImageRow {
    /// Full id (`sha256:…`), used as the removal handle.
    id: String,
    /// Short id for display (12 hex chars).
    short_id: String,
    /// Primary `repo:tag`, or an untagged marker.
    name: String,
    size_bytes: i64,
    /// A container currently uses it (`containers > 0`).
    used_by_container: bool,
    /// A current spec references it by tag.
    used_by_spec: bool,
}

impl ImageRow {
    fn in_use(&self) -> bool {
        self.used_by_container || self.used_by_spec
    }
}

#[derive(Template)]
#[template(path = "admin/disk.html")]
struct DiskPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    role: Role,
    /// False when the server started without `--docker` — the page then
    /// shows a banner instead of empty tables.
    available: bool,
    containers: Vec<ManagedContainer>,
    images: Vec<ImageRow>,
    /// How many of `containers` are stopped (drives the prune button).
    stopped_count: usize,
    /// How many images are unused (drives the "remove all unused" button).
    unused_images_count: usize,
    /// Sum of image sizes (may over-count shared layers — labelled as
    /// such in the UI).
    images_total_bytes: i64,
    flash: Option<&'static str>,
    flash_error: bool,
}

impl DiskPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// Human-readable byte size (binary units). Kept tiny — the disk
    /// panel is the only caller. Takes `&i64` because Askama passes
    /// method arguments by reference.
    fn human(&self, bytes: &i64) -> String {
        const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
        let bytes = *bytes;
        if bytes <= 0 {
            return "0 B".into();
        }
        let mut v = bytes as f64;
        let mut u = 0;
        while v >= 1024.0 && u < UNITS.len() - 1 {
            v /= 1024.0;
            u += 1;
        }
        if u == 0 {
            format!("{bytes} B")
        } else {
            format!("{v:.1} {}", UNITS[u])
        }
    }
}

#[derive(Debug, Deserialize)]
struct FlashQuery {
    flash: Option<String>,
}

async fn index(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Query(q): Query<FlashQuery>,
) -> Response {
    let (flash, flash_error) = match q.flash.as_deref() {
        Some("removed") => (Some("admin-disk-flash-removed"), false),
        Some("pruned") => (Some("admin-disk-flash-pruned"), false),
        Some("images-pruned") => (Some("admin-disk-flash-images-pruned"), false),
        Some("nothing") => (Some("admin-disk-flash-nothing"), false),
        Some("error") => (Some("admin-disk-flash-error"), true),
        _ => (None, false),
    };

    // Landing-only / no Docker: render the "backend missing" banner.
    let Some(backend) = state.backend.as_ref() else {
        return super::render(&DiskPage {
            locale: loc,
            theme,
            locales: &state.locales,
            locales_all: &Locale::ALL,
            base: state.base_path.clone(),
            nav_section: "disk",
            role: Role::Admin,
            available: false,
            containers: Vec::new(),
            images: Vec::new(),
            stopped_count: 0,
            unused_images_count: 0,
            images_total_bytes: 0,
            flash,
            flash_error,
        });
    };

    let containers = backend.list_managed_containers().await.unwrap_or_default();
    let images = backend.list_images().await.unwrap_or_default();

    // Which image refs does the live catalog reference? Used to flag an
    // image as "in use by a spec" so the panel won't offer to remove it.
    let spec_images = spec_image_refs(&state).await;

    let stopped_count = containers.iter().filter(|c| !c.running).count();
    let images_total_bytes = images.iter().map(|i| i.size_bytes).sum();
    let images: Vec<ImageRow> = images.into_iter().map(|i| image_row(i, &spec_images)).collect();
    let unused_images_count = images.iter().filter(|r| !r.in_use()).count();

    super::render(&DiskPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "disk",
        role: Role::Admin,
        available: true,
        containers,
        images,
        stopped_count,
        unused_images_count,
        images_total_bytes,
        flash,
        flash_error,
    })
}

/// The set of image refs the live (DB-first) catalog references — an
/// image carrying one of these tags is "in use by a spec" and is never
/// offered for removal.
async fn spec_image_refs(state: &AppState) -> HashSet<String> {
    crate::catalog::effective_specs(state.db.as_ref(), &state.config)
        .await
        .iter()
        .filter_map(|s| s.container_image.clone())
        .collect()
}

/// An image is safe to reclaim when no container uses it (`containers`
/// is 0 or unknown-as-non-positive) **and** no current spec references
/// it by tag. Same rule the per-row remove button uses.
fn image_unused(i: &ImageInfo, spec_images: &HashSet<String>) -> bool {
    i.containers <= 0 && !i.tags.iter().any(|t| spec_images.contains(t))
}

/// Build a display row, resolving the primary tag and the in-use flags.
fn image_row(i: ImageInfo, spec_images: &HashSet<String>) -> ImageRow {
    let used_by_spec = i.tags.iter().any(|t| spec_images.contains(t));
    let name = i
        .tags
        .first()
        .cloned()
        .unwrap_or_else(|| "<untagged>".to_string());
    // `sha256:abcdef…` → `abcdef…` (first 12 hex) for a compact display id.
    let short_id = i
        .id
        .strip_prefix("sha256:")
        .unwrap_or(&i.id)
        .chars()
        .take(12)
        .collect();
    ImageRow {
        short_id,
        name,
        size_bytes: i.size_bytes,
        used_by_container: i.containers > 0,
        used_by_spec,
        id: i.id,
    }
}

#[derive(Debug, Deserialize)]
struct IdForm {
    id: String,
}

async fn remove_container(
    _: RequireAdmin,
    State(state): State<AppState>,
    Form(form): Form<IdForm>,
) -> Response {
    let Some(backend) = state.backend.as_ref() else {
        return redirect("error");
    };
    match backend.remove_container(&form.id).await {
        Ok(()) => redirect("removed"),
        Err(e) => {
            tracing::warn!(error = %e, id = %form.id, "disk: remove container failed");
            redirect("error")
        }
    }
}

async fn prune_stopped(_: RequireAdmin, State(state): State<AppState>) -> Response {
    let Some(backend) = state.backend.as_ref() else {
        return redirect("error");
    };
    match backend.prune_stopped().await {
        Ok(0) => redirect("nothing"),
        Ok(_) => redirect("pruned"),
        Err(e) => {
            tracing::warn!(error = %e, "disk: prune stopped failed");
            redirect("error")
        }
    }
}

async fn remove_image(
    _: RequireAdmin,
    State(state): State<AppState>,
    Form(form): Form<IdForm>,
) -> Response {
    let Some(backend) = state.backend.as_ref() else {
        return redirect("error");
    };
    match backend.remove_image(&form.id).await {
        Ok(()) => redirect("removed"),
        Err(e) => {
            // The daemon refuses an image still in use — surfaced as a
            // generic error flash; the detail goes to the log.
            tracing::warn!(error = %e, id = %form.id, "disk: remove image failed");
            redirect("error")
        }
    }
}

/// Remove every **unused** image in one click (#463) — `containers == 0`
/// and not referenced by any current spec. Never `--force`s, and never
/// runs a host-wide `docker image prune`: it only removes the exact
/// subset the panel flags as unused, so a non-Ruscker image the host
/// happens to have stays put.
async fn prune_images(_: RequireAdmin, State(state): State<AppState>) -> Response {
    let Some(backend) = state.backend.as_ref() else {
        return redirect("error");
    };
    let images = match backend.list_images().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "disk: list images for prune failed");
            return redirect("error");
        }
    };
    let spec_images = spec_image_refs(&state).await;
    let unused: Vec<String> = images
        .into_iter()
        .filter(|i| image_unused(i, &spec_images))
        .map(|i| i.id)
        .collect();
    if unused.is_empty() {
        return redirect("nothing");
    }
    let mut removed = 0;
    for id in &unused {
        match backend.remove_image(id).await {
            Ok(()) => removed += 1,
            // Best-effort: an image that turned in-use between listing and
            // removal (or a multi-tag image) just stays — logged, not fatal.
            Err(e) => tracing::warn!(error = %e, id = %id, "disk: prune image failed"),
        }
    }
    if removed > 0 {
        redirect("images-pruned")
    } else {
        redirect("error")
    }
}

/// Post/redirect/get back to the panel with a one-word flash code. The
/// base-path response rewriter re-prefixes the `Location` header.
fn redirect(flash: &str) -> Response {
    Redirect::to(&format!("/admin/disk?flash={flash}")).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(tags: &[&str], containers: i64) -> ImageInfo {
        ImageInfo {
            id: "sha256:abc".into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            size_bytes: 1,
            containers,
        }
    }

    /// #463: bulk prune must reclaim only images no container uses and no
    /// spec references — never an image still in use.
    #[test]
    fn image_unused_only_when_no_container_and_no_spec() {
        let catalog: HashSet<String> = ["nginx:alpine".to_string()].into_iter().collect();

        // Referenced by a spec → in use, never pruned.
        assert!(!image_unused(&img(&["nginx:alpine"], 0), &catalog));
        // A container uses it → in use.
        assert!(!image_unused(&img(&["other:1"], 3), &catalog));
        // No container, not in the catalog → reclaimable.
        assert!(image_unused(&img(&["leftover:1"], 0), &catalog));
        // Dangling (untagged), no container → reclaimable.
        assert!(image_unused(&img(&[], 0), &catalog));
        // Daemon couldn't count containers (-1, non-positive) and it's not
        // in the catalog → treated as reclaimable; remove_image runs
        // without --force, so the daemon still refuses a truly in-use one.
        assert!(image_unused(&img(&["x:1"], -1), &catalog));
    }
}
