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
use ruscker_core::{CoreResult, ImageInfo, ManagedContainer, VolumeInfo};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/disk", get(index))
        .route("/admin/disk/containers/remove", post(remove_container))
        .route("/admin/disk/containers/prune", post(prune_stopped))
        .route("/admin/disk/images/remove", post(remove_image))
        .route("/admin/disk/images/prune", post(prune_images))
        .route("/admin/disk/volumes/create", post(create_volume))
        .route("/admin/disk/volumes/remove", post(remove_volume))
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

/// One named-volume row, enriched with whether it's safe to remove
/// (#987). Mirrors [`ImageRow`]: display data + the safety verdict the
/// template's per-row button keys on.
struct VolumeRow {
    /// Volume name — also the removal handle.
    name: String,
    /// Volume driver (usually `local`).
    driver: String,
    /// Creation date for display (`YYYY-MM-DD`, or `—` when the daemon
    /// didn't report one).
    created: String,
    /// How many containers on the host mount this volume — counted over
    /// **ALL** containers, running or stopped, Ruscker's or not, so a
    /// neighbour's mount keeps the volume "in use" here too.
    refs: i64,
    /// A spec in the effective catalog names this volume in its
    /// `container-volumes` — removing it would wipe data the next spawn
    /// expects, so it's held even at zero container references.
    catalog_ref: bool,
    /// Carries the `ruscker.created` label — it was made from this panel,
    /// as opposed to a neighbour's (e.g. ShinyProxy's) volume.
    ruscker_created: bool,
}

impl VolumeRow {
    /// Safe to offer for removal: no container mounts it, no current spec
    /// references it, **and** Ruscker created it. A volume WITHOUT the
    /// `ruscker.created` label is never removable from the panel — even
    /// unreferenced, it's a third party's DATA, and deleted data is
    /// irrecoverable (the #894 never-touch-what-isn't-ours rule, applied
    /// to volumes). The daemon refusing an in-use volume is only the
    /// final backstop; this check is the policy.
    fn removable(&self) -> bool {
        self.refs == 0 && !self.catalog_ref && self.ruscker_created
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
    /// The volume inventory failed — or this backend doesn't implement
    /// volumes at all (multihost + mocks: the trait default is `Err`,
    /// fail closed, never a false "no volumes").
    volumes_unavailable: bool,
    /// True when the host container listing FAILED, so image in-use can't
    /// be trusted: the page then treats every image as in-use (no remove /
    /// prune) and shows a warning banner (#871 follow-up, fail closed).
    usage_unknown: bool,
    containers: Vec<ManagedContainer>,
    images: Vec<ImageRow>,
    /// Named host volumes, sorted by name (#987).
    volumes: Vec<VolumeRow>,
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
        Some("volume-created") => (Some("admin-disk-flash-volume-created"), false),
        Some("volume-removed") => (Some("admin-disk-flash-volume-removed"), false),
        Some("volume-bad-name") => (Some("admin-disk-flash-volume-bad-name"), true),
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
            volumes_unavailable: false,
            usage_unknown: false,
            containers: Vec::new(),
            images: Vec::new(),
            volumes: Vec::new(),
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

    // These six inputs are independent — three Docker round-trips, a
    // container-ref enumeration, and two DB reads. Awaiting them serially
    // stacked Docker-daemon latency on every Disk-tab load (#: perf
    // audit); run them concurrently so the page costs ~one round-trip
    // instead of the sum. (The catalog feeds both the image AND the
    // named-volume cross-references, so it's fetched once here;
    // `ruscker_images` is the provenance table; the rest hit the daemon.)
    let ruscker_images_fut = async {
        match state.db.as_ref() {
            Some(db) => crate::db::ruscker_images::all(db).await.unwrap_or_default(),
            None => HashSet::new(),
        }
    };
    let (containers, images, catalog, ruscker_images, container_refs, volumes) = tokio::join!(
        backend.list_managed_containers(),
        backend.list_images(),
        crate::catalog::effective_specs_cached(&state),
        ruscker_images_fut,
        backend.all_container_image_refs(),
        backend.list_volumes(),
    );
    let spec_images = catalog_image_refs(&catalog);
    let (containers, containers_unavailable) = inventory_or_empty(containers, "containers");
    let (images, images_unavailable) = inventory_or_empty(images, "images");

    // Named volumes (#987). An `Err` here is expected for backends that
    // don't do volumes (multihost, mocks — the trait default is Err by
    // design): render the card as "unavailable", never as an empty list
    // (the fail-closed rule from #889).
    let volume_refs = named_volume_refs(&catalog);
    let (volumes, volumes_unavailable) = match volumes {
        Ok(v) => {
            let mut rows: Vec<VolumeRow> =
                v.into_iter().map(|v| volume_row(v, &volume_refs)).collect();
            rows.sort_by(|a, b| a.name.cmp(&b.name));
            (rows, false)
        }
        Err(e) => {
            tracing::warn!(error = %e, "disk: volume listing failed or unsupported");
            (Vec::new(), true)
        }
    };

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
        volumes_unavailable,
        usage_unknown,
        containers,
        images,
        volumes,
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
    catalog_image_refs(&crate::catalog::effective_specs_cached(state).await)
}

/// Pure half of [`spec_image_refs`], for callers that already hold the
/// catalog (the index fetches it once and derives image + volume refs).
fn catalog_image_refs(specs: &[ruscker_config::Spec]) -> HashSet<String> {
    specs.iter().filter_map(|s| s.container_image.clone()).collect()
}

/// The set of NAMED volumes the live catalog references (#987). A spec's
/// `container-volumes` entries are Docker `source:dest[:ro]` strings; a
/// `source` that doesn't start with `/`, `.` or `~` is a named volume (a
/// path is a bind mount, which isn't a removable object). A referenced
/// name is never offered for removal even at zero container references —
/// the next spawn of that spec would silently recreate it EMPTY, losing
/// whatever the app had stored.
fn named_volume_refs(specs: &[ruscker_config::Spec]) -> HashSet<String> {
    let mut named = HashSet::new();
    for spec in specs {
        for entry in spec.volumes.iter().flatten() {
            let source = entry.split(':').next().unwrap_or("");
            if !source.is_empty() && !source.starts_with(['/', '.', '~']) {
                named.insert(source.to_string());
            }
        }
    }
    named
}

/// Build a display row, resolving the catalog cross-reference. The one
/// place `VolumeRow`s are made, so the index table and the remove
/// handler's server-side re-check share the same `removable()` verdict.
fn volume_row(v: VolumeInfo, volume_refs: &HashSet<String>) -> VolumeRow {
    VolumeRow {
        // RFC 3339 → its date part, enough for a disk panel.
        created: match v.created_at.as_deref() {
            Some(ts) => ts.chars().take(10).collect(),
            None => "—".to_string(),
        },
        catalog_ref: volume_refs.contains(&v.name),
        name: v.name,
        driver: v.driver,
        refs: v.refs,
        ruscker_created: v.ruscker_created,
    }
}

/// Docker's own volume-name shape (`[a-zA-Z0-9][a-zA-Z0-9_.-]*`), capped
/// at 100 chars — checked here so a typo gets a friendly flash instead of
/// a raw daemon error. Written with `chars()` on purpose (no regex crate
/// in the workspace); ASCII-only, so `len()` (bytes) == char count for
/// any string that passes the per-char checks.
fn is_valid_volume_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 100 {
        return false;
    }
    let mut chars = name.chars();
    // Safe unwrap-less first(): emptiness was checked above.
    let Some(first) = chars.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
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

#[derive(Debug, Deserialize)]
struct NameForm {
    name: String,
}

/// Create a named volume from the panel (#987). The backend labels it
/// `ruscker.created=true`, which is exactly what later makes it eligible
/// for removal here — a volume made outside Ruscker never is.
async fn create_volume(
    _: RequireAdmin,
    State(state): State<AppState>,
    Form(form): Form<NameForm>,
) -> Response {
    let Some(backend) = state.backend.as_ref() else {
        return redirect("error");
    };
    let name = form.name.trim();
    if !is_valid_volume_name(name) {
        return redirect("volume-bad-name");
    }
    match backend.create_volume(name).await {
        Ok(()) => redirect("volume-created"),
        Err(e) => {
            tracing::warn!(error = %e, name = %name, "disk: create volume failed");
            redirect("error")
        }
    }
}

/// Remove a named volume — deleting its DATA for good, so this is the
/// most careful handler on the panel. The button only renders for a
/// removable volume, but a crafted POST could send any name: re-derive
/// `removable()` server-side from a fresh listing + the catalog, and
/// fail CLOSED when the inventory can't be read (#894/#987). Only a
/// zero-reference, catalog-unreferenced, **Ruscker-created** volume
/// passes; the daemon refusing an in-use volume (no force) is just the
/// final backstop behind this check.
async fn remove_volume(
    _: RequireAdmin,
    State(state): State<AppState>,
    Form(form): Form<NameForm>,
) -> Response {
    let Some(backend) = state.backend.as_ref() else {
        return redirect("error");
    };
    let name = form.name.trim();
    let volumes = match backend.list_volumes().await {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, name = %name, "disk: list volumes for remove failed");
            return redirect("error");
        }
    };
    let volume_refs = named_volume_refs(&crate::catalog::effective_specs_cached(&state).await);
    let ok = volumes
        .into_iter()
        .filter(|v| v.name == name)
        .any(|v| volume_row(v, &volume_refs).removable());
    if !ok {
        tracing::warn!(
            name = %name,
            "disk: refusing to remove a volume that is referenced, foreign, or unknown"
        );
        return redirect("error");
    }

    match backend.remove_volume(name).await {
        Ok(()) => redirect("volume-removed"),
        Err(e) => {
            // The daemon refuses a volume that turned in-use between the
            // check and the removal — generic flash, detail in the log.
            tracing::warn!(error = %e, name = %name, "disk: remove volume failed");
            redirect("error")
        }
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
            volumes_unavailable: true,
            usage_unknown: false,
            containers: Vec::new(),
            images: Vec::new(),
            volumes: Vec::new(),
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
        assert!(html.contains("volume inventory"));
        assert_eq!(html.matches("This is a partial view").count(), 3);
        assert!(!html.contains("No local images."));
        assert!(!html.contains("No Ruscker-managed containers."));
        assert!(!html.contains("No named volumes."));
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

    /// #987: only NAMED volume sources count as catalog references — a
    /// path source (`/`, `.`, `~`) is a bind mount, not a volume object.
    #[test]
    fn named_volume_refs_distinguishes_names_from_bind_paths() {
        let spec: ruscker_config::Spec = serde_yaml_ng::from_str(
            "id: x\ncontainer-image: a:1\ncontainer-volumes:\n\
             \x20 - /srv/x:/data\n\
             \x20 - meuvol:/data\n\
             \x20 - outro:/data:ro\n\
             \x20 - ./rel:/data\n\
             \x20 - ~/home:/data\n\
             \x20 - ':/broken'\n",
        )
        .expect("parse spec");
        let refs = named_volume_refs(&[spec]);
        assert!(refs.contains("meuvol"), "plain named volume");
        assert!(refs.contains("outro"), "named volume with :ro");
        assert!(!refs.contains("/srv/x"), "absolute path is a bind mount");
        assert!(!refs.contains("./rel"), "relative path is a bind mount");
        assert!(!refs.contains("~/home"), "home path is a bind mount");
        assert_eq!(refs.len(), 2, "empty source is ignored");

        // A spec without `container-volumes` contributes nothing.
        let bare: ruscker_config::Spec =
            serde_yaml_ng::from_str("id: y\ncontainer-image: a:1").expect("parse bare spec");
        assert!(named_volume_refs(&[bare]).is_empty());
    }

    /// #987: the create form's name gate (Docker's own volume-name shape).
    #[test]
    fn volume_name_validation_matches_dockers_rule() {
        assert!(is_valid_volume_name("data"));
        assert!(is_valid_volume_name("my-vol_1.bak"));
        assert!(is_valid_volume_name("0"));
        assert!(is_valid_volume_name(&"a".repeat(100)));

        assert!(!is_valid_volume_name(""));
        assert!(!is_valid_volume_name(&"a".repeat(101)));
        assert!(!is_valid_volume_name("-leading-dash"));
        assert!(!is_valid_volume_name(".leading-dot"));
        assert!(!is_valid_volume_name("has space"));
        assert!(!is_valid_volume_name("has/slash"));
        assert!(!is_valid_volume_name("acentuação"));
    }

    fn vol(name: &str, refs: i64, ruscker_created: bool) -> VolumeInfo {
        VolumeInfo {
            name: name.into(),
            driver: "local".into(),
            created_at: Some("2026-07-13T10:00:00Z".into()),
            refs,
            ruscker_created,
        }
    }

    /// #987: a volume is removable only at zero references, unreferenced
    /// by the catalog, AND Ruscker-created — a neighbour's volume is
    /// never offered even when idle (its data is irrecoverable, #894).
    #[test]
    fn volume_removable_requires_idle_uncatalogued_and_ruscker_created() {
        let refs: HashSet<String> = ["appdata".to_string()].into_iter().collect();

        // Idle, not in the catalog, ours → removable.
        assert!(volume_row(vol("scratch", 0, true), &refs).removable());
        // A container mounts it → held.
        assert!(!volume_row(vol("scratch", 2, true), &refs).removable());
        // A catalog spec names it → held even at zero references.
        assert!(!volume_row(vol("appdata", 0, true), &refs).removable());
        // No `ruscker.created` label → NEVER removable from the panel.
        assert!(!volume_row(vol("neighbour", 0, false), &refs).removable());

        // Display bits: RFC 3339 → date part; missing timestamp → em dash.
        assert_eq!(volume_row(vol("scratch", 0, true), &refs).created, "2026-07-13");
        let undated = VolumeInfo {
            created_at: None,
            ..vol("scratch", 0, true)
        };
        assert_eq!(volume_row(undated, &refs).created, "—");
    }
}
