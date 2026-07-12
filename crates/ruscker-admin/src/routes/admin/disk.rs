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
use ruscker_core::{CoreResult, ImageInfo, ManagedContainer};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/disk", get(index))
        .route("/admin/disk/containers/remove", post(remove_container))
        .route("/admin/disk/containers/prune", post(prune_stopped))
        .route("/admin/disk/images/remove", post(remove_image))
        .route("/admin/disk/images/prune", post(prune_images))
        .route("/admin/disk/reclaim", post(reclaim))
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
    /// A live managed container is built from it (matched by tag or id).
    used_by_container: bool,
    /// A current spec references it by tag.
    used_by_spec: bool,
    /// Ruscker has managed this image (a spec referenced it or it was
    /// pulled here) — provenance from `db::ruscker_images` (#894). A
    /// `false` here is a neighbour's image (e.g. ShinyProxy's): shown, but
    /// never offered for removal.
    ruscker_managed: bool,
}

impl ImageRow {
    fn in_use(&self) -> bool {
        self.used_by_container || self.used_by_spec
    }

    /// Safe to offer for removal: not in use **and** a Ruscker image. A
    /// neighbour's idle image (no Ruscker provenance) is never removable —
    /// deleting it is irrecoverable on a host that can't re-pull (#894).
    fn removable(&self) -> bool {
        !self.in_use() && self.ruscker_managed
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
    /// The Docker backend exists, but its managed-container inventory
    /// failed. Kept separate from an empty successful listing so the UI
    /// never reports "no containers" during a daemon outage.
    containers_unavailable: bool,
    /// The Docker backend exists, but its image inventory failed.
    images_unavailable: bool,
    /// True when the host container listing FAILED, so image in-use can't
    /// be trusted: the page then treats every image as in-use (no remove /
    /// prune) and shows a warning banner (#871 follow-up, fail closed).
    usage_unknown: bool,
    containers: Vec<ManagedContainer>,
    images: Vec<ImageRow>,
    /// How many of `containers` are stopped (drives the prune button).
    stopped_count: usize,
    /// How many images are unused (drives the "remove all unused" button).
    unused_images_count: usize,
    /// Sum of image sizes (may over-count shared layers — labelled as
    /// such in the UI).
    images_total_bytes: i64,
    /// Host disk usage for the hero (#623). `disk_available` is false when
    /// the statvfs call failed; the template then hides the hero.
    disk_available: bool,
    disk_total: i64,
    disk_used: i64,
    disk_free: i64,
    disk_pct: u32,
    /// Stacked-bar segments (bytes + width%). Ruscker images, everything
    /// else used, then free — they sum to the total.
    seg_images_bytes: i64,
    seg_other_bytes: i64,
    seg_images_pct: f64,
    seg_other_pct: f64,
    seg_free_pct: f64,
    flash: Option<&'static str>,
    flash_error: bool,
}

/// Host filesystem totals from `statvfs`. `used + free == total`.
struct DiskUsage {
    total: i64,
    used: i64,
    free: i64,
}

/// Best-effort host disk usage for the filesystem holding `path`. `None`
/// if the POSIX `statvfs(3)` call fails or reports a zero-size filesystem.
fn disk_usage(path: &str) -> Option<DiskUsage> {
    let c = std::ffi::CString::new(path).ok()?;
    // SAFETY: `s` is a zeroed, properly-aligned `statvfs`. `statvfs` only
    // writes its scalar fields and returns 0 on success / -1 on error; we
    // read the integer fields afterwards. `c` outlives the call.
    let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
        return None;
    }
    let frsize = s.f_frsize as i64;
    let total = frsize.saturating_mul(s.f_blocks as i64);
    let free = frsize.saturating_mul(s.f_bfree as i64);
    if total <= 0 {
        return None;
    }
    Some(DiskUsage {
        total,
        used: total.saturating_sub(free),
        free,
    })
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
        Some("reclaimed") => (Some("admin-disk-flash-reclaimed"), false),
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
            containers_unavailable: false,
            images_unavailable: false,
            usage_unknown: false,
            containers: Vec::new(),
            images: Vec::new(),
            stopped_count: 0,
            unused_images_count: 0,
            images_total_bytes: 0,
            disk_available: false,
            disk_total: 0,
            disk_used: 0,
            disk_free: 0,
            disk_pct: 0,
            seg_images_bytes: 0,
            seg_other_bytes: 0,
            seg_images_pct: 0.0,
            seg_other_pct: 0.0,
            seg_free_pct: 0.0,
            flash,
            flash_error,
        });
    };

    // These five inputs are independent — two Docker round-trips, a
    // container-ref enumeration, and two DB reads. Awaiting them serially
    // stacked Docker-daemon latency on every Disk-tab load (#: perf
    // audit); run them concurrently so the page costs ~one round-trip
    // instead of the sum. (`spec_image_refs` reads the catalog;
    // `ruscker_images` the provenance table; the rest hit the daemon.)
    let ruscker_images_fut = async {
        match state.db.as_ref() {
            Some(db) => crate::db::ruscker_images::all(db).await.unwrap_or_default(),
            None => HashSet::new(),
        }
    };
    let (containers, images, spec_images, ruscker_images, container_refs) = tokio::join!(
        backend.list_managed_containers(),
        backend.list_images(),
        spec_image_refs(&state),
        ruscker_images_fut,
        backend.all_container_image_refs(),
    );
    let (containers, containers_unavailable) = inventory_or_empty(containers, "containers");
    let (images, images_unavailable) = inventory_or_empty(images, "images");

    // The image refs **every** container on the host is built from —
    // not just Ruscker-managed ones (#871) — so an image backing a
    // non-Ruscker container (e.g. ShinyProxy) is never flagged "unused".
    // (The daemon's per-image count is unreliable, #585, so we derive it
    // from the live container set.) `None` ⇒ the listing FAILED: fail
    // closed (every image reads as in-use, nothing prunable) rather than
    // assuming the host runs nothing and exposing every image to removal.
    let container_images: Option<HashSet<String>> = match container_refs {
        Ok(v) => Some(v.into_iter().collect()),
        Err(e) => {
            tracing::warn!(error = %e, "disk: container image refs failed; failing closed (all in-use)");
            None
        }
    };
    let usage_unknown = container_images.is_none();

    let stopped_count = containers.iter().filter(|c| !c.running).count();
    let images_total_bytes: i64 = images.iter().map(|i| i.size_bytes).sum();
    let images: Vec<ImageRow> = images
        .into_iter()
        .map(|i| image_row(i, &spec_images, container_images.as_ref(), &ruscker_images))
        .collect();
    // Count what the "remove all unused" button would actually reclaim:
    // Ruscker images, not in use. (When usage is unknown nothing is
    // removable — `removable()` is already false — so this is 0.)
    let unused_images_count = images.iter().filter(|r| r.removable()).count();

    // Host disk usage for the hero. statvfs the filesystem holding the
    // uploaded-images dir when known, else the root — the host disk on a
    // typical single-volume deploy. Best-effort: hidden if it fails.
    let disk_path = state
        .images_dir
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_string());
    let usage = disk_usage(&disk_path);
    let (
        disk_available,
        disk_total,
        disk_used,
        disk_free,
        disk_pct,
        seg_images_bytes,
        seg_other_bytes,
        seg_images_pct,
        seg_other_pct,
        seg_free_pct,
    ) = match usage {
        Some(u) => {
            let imgs = images_total_bytes.clamp(0, u.used);
            let other = (u.used - imgs).max(0);
            // One-decimal width % so the inline style stays tidy.
            let pct = |b: i64| ((b as f64 / u.total as f64 * 1000.0).round() / 10.0).max(0.0);
            (
                true,
                u.total,
                u.used,
                u.free,
                ((u.used as f64 / u.total as f64) * 100.0).round() as u32,
                imgs,
                other,
                pct(imgs),
                pct(other),
                pct(u.free),
            )
        }
        None => (false, 0, 0, 0, 0, 0, 0, 0.0, 0.0, 0.0),
    };

    super::render(&DiskPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "disk",
        role: Role::Admin,
        available: true,
        containers_unavailable,
        images_unavailable,
        usage_unknown,
        containers,
        images,
        stopped_count,
        unused_images_count,
        images_total_bytes,
        disk_available,
        disk_total,
        disk_used,
        disk_free,
        disk_pct,
        seg_images_bytes,
        seg_other_bytes,
        seg_images_pct,
        seg_other_pct,
        seg_free_pct,
        flash,
        flash_error,
    })
}

/// Preserve the difference between an empty Docker inventory and a failed
/// inventory request. The panel can still render its other independent data,
/// but must label the failed section as degraded instead of showing a false
/// empty state.
fn inventory_or_empty<T>(
    result: CoreResult<Vec<T>>,
    inventory: &'static str,
) -> (Vec<T>, bool) {
    match result {
        Ok(items) => (items, false),
        Err(e) => {
            tracing::warn!(error = %e, inventory, "disk: Docker inventory listing failed");
            (Vec::new(), true)
        }
    }
}

/// The set of image refs the live (DB-first) catalog references — an
/// image carrying one of these tags is "in use by a spec" and is never
/// offered for removal.
async fn spec_image_refs(state: &AppState) -> HashSet<String> {
    crate::catalog::effective_specs_cached(state)
        .await
        .iter()
        .filter_map(|s| s.container_image.clone())
        .collect()
}

/// True when **any** host container is built from this image (by tag or
/// id). `container_images` is the all-container ref set (#871), so an
/// image backing a non-Ruscker container counts as in-use. The daemon's
/// `ImageSummary.containers` count is unreliable on a plain `list_images`
/// (#585), so we derive in-use from the live container set instead.
///
/// `None` means the container set **could not be determined** (the daemon
/// listing failed): we then fail CLOSED and report the image as in-use,
/// so a transient Docker error can never flip a ShinyProxy (or Ruscker)
/// image to "unused" and expose it to removal/prune — irrecoverable on a
/// host that can't re-pull (#871 follow-up).
fn image_used_by_container(i: &ImageInfo, container_images: Option<&HashSet<String>>) -> bool {
    match container_images {
        None => true,
        Some(set) => set.contains(&i.id) || i.tags.iter().any(|t| set.contains(t)),
    }
}

/// True when Ruscker has managed this image — a spec referenced it or it
/// was pulled here (`ruscker_images` provenance, #894). Matched by tag;
/// an untagged/dangling image has no ref to match (the host-safe "Reclaim
/// space" handles those separately).
fn image_is_ruscker(i: &ImageInfo, ruscker_images: &HashSet<String>) -> bool {
    i.tags.iter().any(|t| ruscker_images.contains(t))
}

/// An image is safe to **remove** when no container is built from it, no
/// current spec references it by tag, **and** it's a Ruscker image (#894 —
/// never a neighbour's idle cache). Same rule the per-row button and the
/// bulk prune use. (Uses the real container set, not the unreliable
/// `ImageInfo.containers` count — #585.) A `None` container set (listing
/// failed) makes this `false` for every image — nothing is removable.
fn image_removable(
    i: &ImageInfo,
    spec_images: &HashSet<String>,
    container_images: Option<&HashSet<String>>,
    ruscker_images: &HashSet<String>,
) -> bool {
    !image_used_by_container(i, container_images)
        && !i.tags.iter().any(|t| spec_images.contains(t))
        && image_is_ruscker(i, ruscker_images)
}

/// Build a display row, resolving the primary tag and the in-use flags.
fn image_row(
    i: ImageInfo,
    spec_images: &HashSet<String>,
    container_images: Option<&HashSet<String>>,
    ruscker_images: &HashSet<String>,
) -> ImageRow {
    let used_by_spec = i.tags.iter().any(|t| spec_images.contains(t));
    let used_by_container = image_used_by_container(&i, container_images);
    // A current spec referencing it is itself Ruscker provenance, even if
    // the durable record somehow missed it.
    let ruscker_managed = used_by_spec || image_is_ruscker(&i, ruscker_images);
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
        used_by_container,
        used_by_spec,
        ruscker_managed,
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

/// "Reclaim space" — host-SAFE cleanup: prune dangling images + the build
/// cache. Never removes a tagged image or any container (Ruscker or not),
/// unlike a full `docker system prune` — important on a host that also
/// runs ShinyProxy or other containers.
async fn reclaim(_: RequireAdmin, State(state): State<AppState>) -> Response {
    let Some(backend) = state.backend.as_ref() else {
        return redirect("error");
    };
    match backend.reclaim_space().await {
        Ok(0) => redirect("nothing"),
        Ok(bytes) => {
            tracing::info!(bytes, "disk: reclaimed dangling images + build cache");
            redirect("reclaimed")
        }
        Err(e) => {
            tracing::warn!(error = %e, "disk: reclaim space failed");
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

    // The button only renders for a removable image, but a crafted POST
    // could send any id — so enforce the same rule server-side (#894):
    // refuse anything not removable (in use, or no Ruscker provenance).
    // Fail closed if the host inventory can't be read.
    let images = match backend.list_images().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "disk: list images for remove failed");
            return redirect("error");
        }
    };
    let container_images: HashSet<String> = match backend.all_container_image_refs().await {
        Ok(v) => v.into_iter().collect(),
        Err(e) => {
            tracing::warn!(
                error = %e,
                id = %form.id,
                "disk: image remove aborted — container image refs failed"
            );
            return redirect("error");
        }
    };
    let spec_images = spec_image_refs(&state).await;
    let ruscker_images = match state.db.as_ref() {
        Some(db) => crate::db::ruscker_images::all(db).await.unwrap_or_default(),
        None => HashSet::new(),
    };
    let ok = images.iter().any(|i| {
        i.id == form.id
            && image_removable(i, &spec_images, Some(&container_images), &ruscker_images)
    });
    if !ok {
        tracing::warn!(id = %form.id, "disk: refusing to remove an image that is in use, foreign, or unknown");
        return redirect("error");
    }

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

/// Remove every **removable** image in one click (#463) — a Ruscker image
/// (#894 provenance) that no container is built from (on ANY host) and no
/// current spec references. Never `--force`s, and never runs a host-wide
/// `docker image prune`: it removes only the exact subset the panel flags.
///
/// Host-safe by construction: an image with no Ruscker provenance (a
/// side-by-side ShinyProxy's, say) is never a prune target even when idle,
/// so it can't be deleted on a host that can't re-pull (offline /
/// CDN-blocked). The container cross-reference (#871) protects anything
/// running, and the fail-closed in-use signal (#889/#896) blocks deleting
/// an image whose usage can't be determined.
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
    let ruscker_images = match state.db.as_ref() {
        Some(db) => crate::db::ruscker_images::all(db).await.unwrap_or_default(),
        None => HashSet::new(),
    };
    // ALL host containers (#871) — never prune an image a non-Ruscker
    // container is built from. If the listing FAILS we can't tell what's
    // in use, so abort the whole prune (fail closed) rather than treat
    // every image as unused and delete in-use ones (#871 follow-up).
    let container_images: HashSet<String> = match backend.all_container_image_refs().await {
        Ok(v) => v.into_iter().collect(),
        Err(e) => {
            tracing::warn!(error = %e, "disk: prune aborted — container image refs failed");
            return redirect("error");
        }
    };
    let unused: Vec<String> = images
        .into_iter()
        .filter(|i| image_removable(i, &spec_images, Some(&container_images), &ruscker_images))
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

    #[test]
    fn disk_usage_reads_the_root_filesystem() {
        // statvfs on "/" should succeed on any POSIX host and report a
        // non-empty filesystem whose used + free accounts for the total.
        let u = disk_usage("/").expect("statvfs / should succeed");
        assert!(u.total > 0, "total > 0");
        assert!(u.used >= 0 && u.free >= 0, "non-negative");
        assert_eq!(u.used + u.free, u.total, "used + free == total");
    }

    #[test]
    fn disk_usage_is_none_for_a_bad_path() {
        assert!(disk_usage("/no/such/path/should/exist/here").is_none());
    }

    #[test]
    fn inventory_errors_are_not_reported_as_empty_success() {
        let (items, unavailable) = inventory_or_empty::<u8>(Ok(Vec::new()), "test");
        assert!(items.is_empty());
        assert!(!unavailable, "a successful empty inventory is available");

        let error = ruscker_core::CoreError::Backend("daemon unavailable".into());
        let (items, unavailable) = inventory_or_empty::<u8>(Err(error), "test");
        assert!(items.is_empty());
        assert!(unavailable, "a failed inventory must stay distinguishable");
    }

    #[test]
    fn degraded_inventory_renders_warnings_not_empty_claims() {
        let locales = Locales::load().expect("load locales");
        let html = DiskPage {
            locale: Locale::En,
            theme: Theme::Auto,
            locales: &locales,
            locales_all: &Locale::ALL,
            base: std::sync::Arc::from(""),
            nav_section: "disk",
            role: Role::Admin,
            available: true,
            containers_unavailable: true,
            images_unavailable: true,
            usage_unknown: false,
            containers: Vec::new(),
            images: Vec::new(),
            stopped_count: 0,
            unused_images_count: 0,
            images_total_bytes: 0,
            disk_available: false,
            disk_total: 0,
            disk_used: 0,
            disk_free: 0,
            disk_pct: 0,
            seg_images_bytes: 0,
            seg_other_bytes: 0,
            seg_images_pct: 0.0,
            seg_other_pct: 0.0,
            seg_free_pct: 0.0,
            flash: None,
            flash_error: false,
        }
        .render()
        .expect("render degraded disk page");

        assert!(html.contains("image inventory"));
        assert!(html.contains("container inventory"));
        assert_eq!(html.matches("This is a partial view").count(), 2);
        assert!(!html.contains("No local images."));
        assert!(!html.contains("No Ruscker-managed containers."));
    }

    fn img(tags: &[&str], containers: i64) -> ImageInfo {
        ImageInfo {
            id: "sha256:abc".into(),
            tags: tags.iter().map(|s| s.to_string()).collect(),
            size_bytes: 1,
            containers,
        }
    }

    /// #463/#585/#894: an image is removable only when no container is
    /// built from it, no current spec references it, AND it's a Ruscker
    /// image — never a neighbour's idle cache.
    #[test]
    fn image_removable_requires_no_use_no_spec_and_ruscker_provenance() {
        let catalog: HashSet<String> = ["nginx:alpine".to_string()].into_iter().collect();
        // A live container built from "other:1".
        let in_use: HashSet<String> = ["other:1".to_string()].into_iter().collect();
        let none: HashSet<String> = HashSet::new();
        // Everything Ruscker has managed (provenance record).
        let ruscker: HashSet<String> = ["nginx:alpine", "other:1", "leftover:1", "x:1"]
            .iter()
            .map(|s| s.to_string())
            .collect();

        // Referenced by a current spec → in use, never removable.
        assert!(!image_removable(&img(&["nginx:alpine"], 0), &catalog, Some(&none), &ruscker));
        // A live container is built from it → in use (regardless of the
        // bogus `containers` count, -1 in practice).
        assert!(!image_removable(&img(&["other:1"], -1), &catalog, Some(&in_use), &ruscker));
        // No container, not in the catalog, and Ruscker's → removable.
        assert!(image_removable(&img(&["leftover:1"], 0), &catalog, Some(&none), &ruscker));
        assert!(image_removable(&img(&["x:1"], -1), &catalog, Some(&none), &ruscker));

        // #894: a NON-Ruscker image (a neighbour's, e.g. ShinyProxy's) is
        // never removable even when idle.
        let foreign: HashSet<String> = HashSet::new();
        assert!(!image_removable(&img(&["sp/app:1"], 0), &catalog, Some(&none), &foreign));

        // Dangling/untagged → no ref to prove provenance → not removable
        // here (the host-safe "Reclaim space" handles dangling images).
        assert!(!image_removable(&img(&[], 0), &catalog, Some(&none), &ruscker));

        // Matched by image id (a container) → in use.
        let by_id: HashSet<String> = ["sha256:abc".to_string()].into_iter().collect();
        assert!(!image_removable(&img(&["untagged:1"], -1), &catalog, Some(&by_id), &ruscker));
    }

    // #871/#889 follow-up: a FAILED container listing (None) must fail
    // closed — nothing is removable, even a Ruscker image. Otherwise a
    // transient Docker error could delete a still-in-use image
    // (irrecoverable on a host that can't re-pull).
    #[test]
    fn unknown_container_set_blocks_all_removal() {
        let catalog: HashSet<String> = HashSet::new();
        let ruscker: HashSet<String> = ["anything:1".to_string()].into_iter().collect();
        assert!(image_used_by_container(&img(&["anything:1"], 0), None));
        assert!(!image_removable(&img(&["anything:1"], 0), &catalog, None, &ruscker));
    }

    // #894: image_row sets ruscker_managed + removable() from provenance.
    #[test]
    fn image_row_marks_provenance_and_removability() {
        let catalog: HashSet<String> = HashSet::new();
        let none: HashSet<String> = HashSet::new();
        let ruscker: HashSet<String> = ["mine:1".to_string()].into_iter().collect();

        let mine = image_row(img(&["mine:1"], 0), &catalog, Some(&none), &ruscker);
        assert!(mine.ruscker_managed && mine.removable());

        let foreign = image_row(img(&["sp/app:1"], 0), &catalog, Some(&none), &ruscker);
        assert!(!foreign.ruscker_managed && !foreign.removable());
    }
}
