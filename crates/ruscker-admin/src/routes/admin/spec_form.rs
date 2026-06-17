//! Admin > Add/Edit spec form.
//!
//! Single template (`templates/admin/spec_form.html`) used for both
//! New and Edit, parameterized by `mode`. On submit, the form posts
//! to `/admin/specs` (create) or `/admin/specs/:id` (update); on
//! delete, to `/admin/specs/:id/delete`.
//!
//! Field scope covers what the SEPE YAML actually uses today
//! (~80% of specs). Phase 2.5 adds advanced fields (replicas,
//! scaling, env vars, volumes) behind a collapsible section.

use anyhow::Result;
use askama::Template;
use axum::{
    extract::{Form, Path, Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use ruscker_config::{
    ApiSpec, OrderedFloat, Placement, RoutingStrategy, Spec, SpecKindOverride, TemplateProperties,
};
use std::collections::BTreeMap;
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Value as YamlValue;
use std::collections::HashMap;

use crate::auth::{RequireEditor, Role};
use crate::db;
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::view_model::DisplayType;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/specs/new", get(new_form))
        .route("/admin/specs", post(create))
        .route(
            "/admin/specs/{id}/edit",
            get(edit_form),
        )
        .route("/admin/specs/{id}/duplicate", get(duplicate_form))
        .route("/admin/specs/image-check", get(image_check))
        // POST starts the pull (a side effect → CSRF-guarded); the GET
        // only follows the resulting progress stream (#720 audit P2).
        .route("/admin/specs/image-pull", post(image_pull_start))
        .route("/admin/specs/image-pull/events", get(image_pull_events))
        .route("/admin/specs/{id}/repull", post(image_repull))
        .route("/admin/specs/{id}", post(update))
        .route("/admin/specs/{id}/delete", post(delete))
}

// ── Image presence check (#498, slice A) ────────────────────────

#[derive(Deserialize)]
struct ImageCheckQuery {
    image: String,
}

/// Result of [`image_check`]. `status` is one of:
/// - `present`   — the image is on the server (ready to launch)
/// - `absent`    — not on the server (it'll be pulled on first launch)
/// - `empty`     — no image name typed
/// - `unresolved`— the name still carries a `${VAR}` (resolved at pull)
/// - `no-backend`— Docker isn't connected, so we can't check
/// - `error`     — the daemon check failed
#[derive(Serialize)]
struct ImageCheckResult {
    status: &'static str,
}

/// `GET /admin/specs/image-check?image=<name>` — does the backend already
/// have this image locally? Powers the spec editor's "on server" indicator
/// (#498). Pull-free and Editor-gated; a quick yes/no, no registry round
/// trip (that's slice B's explicit Pull button).
async fn image_check(
    _: RequireEditor,
    State(state): State<AppState>,
    Query(q): Query<ImageCheckQuery>,
) -> Json<ImageCheckResult> {
    let image = q.image.trim();
    let status = if image.is_empty() {
        "empty"
    } else if image.contains("${") {
        // A `${VAR}` image is only resolved at pull time; checking the
        // literal would always miss. Tell the editor it's deferred.
        "unresolved"
    } else if let Some(backend) = state.backend.as_ref() {
        match backend.image_present(image).await {
            Ok(true) => "present",
            Ok(false) => "absent",
            Err(err) => {
                tracing::warn!(image, error = ?err, "image presence check failed");
                "error"
            }
        }
    } else {
        "no-backend"
    };
    Json(ImageCheckResult { status })
}

#[derive(Deserialize)]
struct ImagePullQuery {
    image: String,
    /// Name of a stored registry credential (the form's picker) for a
    /// private image. Empty ⇒ anonymous pull (public images).
    #[serde(default)]
    credential: String,
}

/// One progress message from a running pull job.
enum PullEvent {
    Line(String),
    Done,
}

/// A started-but-not-yet-followed image pull. The progress stream is
/// parked here under a one-shot token; the follower GET takes the
/// receiver exactly once.
struct PullJob {
    rx: tokio::sync::Mutex<Option<tokio::sync::mpsc::Receiver<PullEvent>>>,
}

/// Per-job progress buffer. A bounded channel caps the memory a single
/// pull can hold even if its follower is slow or never connects: the
/// producer task applies backpressure at this many queued lines instead
/// of growing without bound (#874).
const PULL_CHANNEL_CAP: usize = 256;

/// Ceiling on concurrent daemon pulls. Repeated "Update image" clicks
/// (or many tabs / scripted hammering) used to each spawn an independent
/// pull task + daemon stream, which can saturate disk and network — a
/// light operational DoS (#874). A permit is held for the lifetime of a
/// pull task; starts beyond the cap are refused with 503 until a slot
/// frees. Editor-gated, so this is an accidental-hammering guardrail.
const MAX_CONCURRENT_PULLS: usize = 4;

/// Available pull slots (see [`MAX_CONCURRENT_PULLS`]). Module static,
/// same pattern as [`PULL_JOBS`]; an owned permit rides into each pull
/// task and is released when the task ends (success, error, or the
/// follower disconnecting and dropping the receiver).
static PULL_SLOTS: std::sync::LazyLock<std::sync::Arc<tokio::sync::Semaphore>> =
    std::sync::LazyLock::new(|| std::sync::Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_PULLS)));

/// In-flight pull jobs keyed by one-shot token (#720 audit P2). A pull is
/// a side effect, so it's *started* by a CSRF-guarded POST and *followed*
/// by a side-effect-free GET/SSE — instead of a GET that pulled. A module
/// static (same pattern as the thumbnail caches in `routes::assets`), so
/// it needs no `AppState` field; an unfollowed job is swept after a grace
/// period so a started-but-never-watched pull can't leak its entry.
static PULL_JOBS: std::sync::LazyLock<
    dashmap::DashMap<String, std::sync::Arc<PullJob>>,
> = std::sync::LazyLock::new(dashmap::DashMap::new);

const PULL_JOB_TTL_SECS: u64 = 300;

/// `POST /admin/specs/image-pull` (form: `image`, `credential`) — START
/// pulling an absent image (#498 slice B; #720 P2 moved the side effect
/// off GET). Validates, kicks off the daemon pull, parks the progress
/// stream under a random token, and returns `{ "job": "<token>" }`. The
/// editor then opens an EventSource on `…/image-pull/events?job=<token>`.
/// Editor-gated; the POST inherits the chrome CSRF (Fetch-Metadata) guard.
async fn image_pull_start(
    _: RequireEditor,
    State(state): State<AppState>,
    Form(q): Form<ImagePullQuery>,
) -> Response {
    let image = q.image.trim().to_string();
    if image.is_empty() || image.contains("${") {
        return (
            StatusCode::BAD_REQUEST,
            "image name required (and must be free of ${…})",
        )
            .into_response();
    }
    let creds = resolve_pull_creds(&state, &q.credential).await;
    match start_pull(&state, &image, creds).await {
        Ok(token) => Json(serde_json::json!({ "job": token })).into_response(),
        Err(resp) => resp,
    }
}

/// Forward a daemon pull's progress lines into the job channel, then a
/// terminal `Done`. Returns early (cancelling the forward) the moment the
/// follower disconnects and drops the receiver — `Sender::send` errors on
/// a bounded channel just as it did on the unbounded one, so a never- or
/// no-longer-followed pull can't keep the task alive forever (#874).
async fn forward_pull(mut line_stream: ruscker_core::LogStream, tx: tokio::sync::mpsc::Sender<PullEvent>) {
    use futures_util::StreamExt;
    while let Some(line) = line_stream.next().await {
        if tx.send(PullEvent::Line(line)).await.is_err() {
            return; // follower disconnected (or job swept)
        }
    }
    let _ = tx.send(PullEvent::Done).await;
}

/// Kick off a daemon image pull, park its progress stream under a
/// one-shot token, and return the token (the follower then opens the
/// `…/image-pull/events?job=<token>` SSE). Shared by the spec form's
/// "Update image" and the Apps-list per-row re-pull (#855). On a
/// start-time failure (backend down, immediate pull error) returns the
/// error `Response` to bubble up unchanged.
async fn start_pull(
    state: &AppState,
    image: &str,
    creds: Option<ruscker_core::RegistryCredentials>,
) -> Result<String, Response> {
    let Some(backend) = state.backend.clone() else {
        return Err(
            (StatusCode::SERVICE_UNAVAILABLE, "Docker backend not connected").into_response(),
        );
    };
    // Refuse before touching the daemon if every pull slot is busy, so a
    // burst of starts can't pile up concurrent pulls (#874).
    let permit = match PULL_SLOTS.clone().try_acquire_owned() {
        Ok(p) => p,
        Err(_) => {
            tracing::warn!(image, "image pull refused: too many concurrent pulls");
            return Err((
                StatusCode::SERVICE_UNAVAILABLE,
                "too many image pulls in progress; retry shortly",
            )
                .into_response());
        }
    };
    let line_stream = match backend.pull_image(image, creds.as_ref(), None).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(image, error = ?e, "image pull start failed");
            return Err((StatusCode::BAD_GATEWAY, format!("pull failed: {e}")).into_response());
        }
    };
    // Drive the pull on a task, forwarding lines into the channel; the
    // follower GET drains them. A terminal `Done` lets the follower close.
    // The bounded channel applies backpressure (caps queued memory); the
    // owned permit is held here and released when the task ends.
    let (tx, rx) = tokio::sync::mpsc::channel::<PullEvent>(PULL_CHANNEL_CAP);
    tokio::spawn(async move {
        let _permit = permit;
        forward_pull(line_stream, tx).await;
    });
    let token = uuid::Uuid::new_v4().to_string();
    PULL_JOBS.insert(
        token.clone(),
        std::sync::Arc::new(PullJob {
            rx: tokio::sync::Mutex::new(Some(rx)),
        }),
    );
    // Sweep an unfollowed job after a grace period (no follower ever
    // connected) so the registry can't grow without bound.
    let sweep = token.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(PULL_JOB_TTL_SECS)).await;
        PULL_JOBS.remove(&sweep);
    });
    tracing::info!(image, job = %token, "image pull started; awaiting follower");
    Ok(token)
}

/// `POST /admin/specs/{id}/repull` — force a re-pull of a DB spec's
/// image from the Apps list (#855). Resolves the spec's image +
/// registry creds (same path as spawn), starts the pull, and returns
/// `{ "job": "<token>" }` for the row to follow over SSE. Editor-gated;
/// inherits the chrome CSRF guard.
async fn image_repull(
    _: RequireEditor,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(db) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "database not attached").into_response();
    };
    let spec = match crate::db::specs::fetch_one(db, &id).await {
        Ok(Some(s)) => s,
        Ok(None) => return (StatusCode::NOT_FOUND, "no such app").into_response(),
        Err(e) => {
            tracing::error!(id, error = ?e, "repull: load spec failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let Some(image) = spec
        .container_image
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    else {
        return (
            StatusCode::BAD_REQUEST,
            "this app has no container image to update",
        )
            .into_response();
    };
    if image.contains("${") {
        return (StatusCode::BAD_REQUEST, "image name still contains ${…}").into_response();
    }
    let creds = match crate::routes::proxy::resolve_creds(&state, &spec).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(id, error = ?e, "repull: credential resolve failed");
            return (StatusCode::BAD_GATEWAY, format!("credential error: {e}")).into_response();
        }
    };
    match start_pull(&state, image, creds).await {
        Ok(token) => Json(serde_json::json!({ "job": token })).into_response(),
        Err(resp) => resp,
    }
}

#[derive(Deserialize)]
struct PullEventsQuery {
    job: String,
}

/// `GET /admin/specs/image-pull/events?job=<token>` — FOLLOW a pull job
/// started by [`image_pull_start`]. Side-effect-free: it only streams the
/// already-running pull's progress over SSE (default events = lines, then
/// one terminal `done` event), then drops the job. An unknown or
/// already-followed token ⇒ 404. Editor-gated (RBAC preserved).
async fn image_pull_events(_: RequireEditor, Query(q): Query<PullEventsQuery>) -> Response {
    use axum::http::header::{HeaderName, CACHE_CONTROL};
    use axum::response::sse::{Event, KeepAlive, Sse};

    // Take the job out of the registry (so a token can't be replayed and
    // nothing leaks), then take its receiver exactly once.
    let Some((_, job)) = PULL_JOBS.remove(&q.job) else {
        return (StatusCode::NOT_FOUND, "unknown or already-followed pull job").into_response();
    };
    let Some(mut rx) = job.rx.lock().await.take() else {
        return (StatusCode::NOT_FOUND, "pull job already followed").into_response();
    };
    let stream = async_stream::stream! {
        while let Some(ev) = rx.recv().await {
            match ev {
                PullEvent::Line(l) => {
                    yield Ok::<_, std::convert::Infallible>(Event::default().data(l));
                }
                PullEvent::Done => {
                    yield Ok(Event::default().event("done").data(""));
                    break;
                }
            }
        }
    };
    let sse = Sse::new(stream)
        .keep_alive(KeepAlive::new().interval(std::time::Duration::from_secs(15)));
    // `X-Accel-Buffering: no` so nginx streams the pull instead of buffering.
    (
        [
            (CACHE_CONTROL, "no-cache"),
            (HeaderName::from_static("x-accel-buffering"), "no"),
        ],
        sse,
    )
        .into_response()
}

/// Resolve a stored registry credential by name for [`image_pull`].
/// `None` (empty name / no DB / no master key / not found) ⇒ anonymous.
async fn resolve_pull_creds(
    state: &AppState,
    credential: &str,
) -> Option<ruscker_core::RegistryCredentials> {
    let name = credential.trim();
    if name.is_empty() {
        return None;
    }
    let pool = state.db.as_ref()?;
    if !state.master_key.is_configured() {
        return None;
    }
    crate::db::credentials::resolve(pool, &state.master_key, name)
        .await
        .ok()
        .flatten()
}

// ── Form payload ────────────────────────────────────────────────

/// Mirror of the form fields. Strings are unconditional so empty
/// inputs round-trip as `""` rather than disappearing; conversion
/// to [`Spec`] handles "empty means None".
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SpecForm {
    pub id: String,
    /// The spec `version` the edit form was rendered against (#745) —
    /// optimistic concurrency: a submit whose base is stale (someone
    /// else saved meanwhile) is rejected instead of last-write-wins.
    /// Empty on the create form and on pre-#745 tabs (check skipped).
    #[serde(default)]
    pub base_version: String,
    pub display_name: String,
    pub description: String,
    /// "app" | "talk" | "report" | "package" | "api" | "link"
    pub display_type: String,
    pub container_image: String,
    /// "active" | "inactive"
    pub state: String,
    pub subject: String,
    /// "Featured" carousel flag (#506) — an HTML checkbox: present ("on")
    /// when checked, absent when not.
    pub featured: String,
    pub logo: String,
    /// Card-cover CSS background (`template-properties.cover`):
    /// a solid color or a gradient string. Empty ⇒ fall back to
    /// the per-kind tint. Not validated server-side (browser
    /// fail-softs), same policy as `landing_customization.header_bg`.
    #[serde(default)]
    pub cover: String,
    /// Per-app accent colour (`template-properties.accent`). When set and
    /// no explicit `cover`, the landing card cover is tinted from it
    /// (#701). Empty ⇒ no accent.
    #[serde(default)]
    pub accent: String,
    /// Short monogram (`template-properties.monogram`, 1–2 chars) shown on
    /// the card cover when there's no logo; empty ⇒ the id is used.
    #[serde(default)]
    pub monogram: String,
    /// Updated date in DD/MM/YYYY. Empty ⇒ stamp with today.
    pub updated: String,
    /// External link target (for type=link/package).
    pub link: String,
    pub seats_per_container: String,
    pub max_lifetime: String,

    // ── Advanced (collapsible). Empty string ⇒ keep the schema
    //    default; nothing here is required. ──────────────────────
    /// `heartbeat-timeout` override in ms; `-1` = never expire.
    pub heartbeat_timeout: String,
    /// Fractional CPUs, e.g. `0.5` (`container-cpu-limit`).
    pub container_cpu_limit: String,
    /// Memory cap, e.g. `512m` / `1.5g` (`container-memory-limit`).
    pub container_memory_limit: String,
    /// Replica pool floor / ceiling (`min`/`max-replicas`).
    pub min_replicas: String,
    pub max_replicas: String,
    /// API: concurrent requests a replica handles before scale-up.
    pub concurrent_requests_per_replica: String,
    /// Bind-mount volumes, one `"/host:/container[:ro]"` per line.
    pub volumes: String,
    /// API sub-fields (only meaningful for `type: api`).
    pub api_port: String,
    pub api_docs_path: String,
    pub api_health_path: String,
    pub api_rate_limit: String,
    /// Checkbox: non-empty ("on") ⇒ permissive CORS enabled.
    pub api_cors: String,
    /// Checkbox: non-empty ("on") ⇒ inject `<base href>` + rewrite
    /// root-relative URLs in `/app/{spec}` HTML (the default). Empty
    /// (unchecked) ⇒ the app self-routes from the forwarded-prefix
    /// headers, so the HTML transform is turned off. Defaults to
    /// checked on a new form.
    pub inject_base_href: String,

    // ── Advanced · Runtime ──────────────────────────────────────
    /// Inner port the app listens on (`container-port`). Blank ⇒
    /// per-kind default (3838 Shiny). Needed for Streamlit (8501),
    /// Dash (8050), Jupyter (8888), …
    pub container_port: String,
    /// Docker `--platform` (`linux/amd64`) for emulated images.
    pub platform: String,
    /// Docker network to attach the container to (`container-network`).
    /// Blank ⇒ the daemon default bridge; created if missing.
    pub container_network: String,
    /// Soft lifetime cap in minutes (`container-lifetime`).
    pub container_lifetime: String,
    /// Checkbox ("on") ⇒ stop the container when the user logs out.
    pub stop_on_logout: String,

    // ── Advanced · Environment + command ────────────────────────
    /// `container-env`, one `NAME=value` per line.
    pub container_env: String,
    /// `container-cmd`, one argv token per line.
    pub container_cmd: String,

    // ── Advanced · Registry (private images) ────────────────────
    pub docker_registry_domain: String,
    pub docker_registry_username: String,
    pub docker_registry_password: String,
    /// Name of a stored credential (credentials store) to use instead
    /// of the inline username/password above.
    pub docker_registry_credential: String,

    // ── Advanced · Access (per-app, Phase 6 / #155) ─────────────
    /// Groups allowed to see + reach this app — comma- or newline-
    /// separated. Blank (and `access-users` blank) ⇒ open to everyone.
    pub access_groups: String,
    /// Usernames allowed to see + reach this app — comma/newline list.
    pub access_users: String,

    // ── Advanced · Resources (requests + body cap) ──────────────
    /// Soft CPU reservation in fractional cores (`container-cpu-request`).
    pub container_cpu_request: String,
    /// Soft memory reservation (`container-memory-request`), e.g. `256m`.
    pub container_memory_request: String,
    /// Per-spec proxied-body cap (`max-body-size`), e.g. `10m`.
    pub max_body_size: String,

    // ── Advanced · Scaling thresholds ───────────────────────────
    /// Utilization fraction (0–1) that triggers scale-up.
    pub scale_up_threshold: String,
    /// Utilization fraction (0–1) below which a replica is reaped.
    pub scale_down_threshold: String,
    /// Seconds below `scale-down-threshold` before reaping.
    pub scale_down_grace: String,
    /// Seconds to drain a replica's sessions before stopping it.
    pub drain_timeout: String,

    // ── Advanced · Routing + multi-host placement ───────────────
    /// `routing-strategy`: ""(default) | least-connections |
    /// round-robin | weighted-random | resource-aware.
    pub routing_strategy: String,
    /// `placement`: ""(default=spread) | spread | bin-pack.
    pub placement: String,
    /// Checkbox ("on") ⇒ prefer distinct hosts for this spec's replicas.
    pub anti_affinity: String,
}

impl SpecForm {
    /// Build a form pre-filled from an existing [`Spec`] for the
    /// edit view.
    pub fn from_spec(spec: &Spec) -> Self {
        let tp = &spec.template_properties;
        let dt = DisplayType::from_spec(spec);
        Self {
            id: spec.id.clone(),
            base_version: String::new(),
            display_name: spec.display_name.clone().unwrap_or_default(),
            description: spec.description.clone().unwrap_or_default(),
            display_type: dt.key().to_string(),
            container_image: spec.container_image.clone().unwrap_or_default(),
            state: tp
                .get_str("state")
                .map(str::to_string)
                .unwrap_or_else(|| "active".into()),
            subject: tp.get_str("subject").map(str::to_string).unwrap_or_default(),
            // Pre-check the "Featured" box from the spec (#506).
            featured: if spec.is_featured() { "on".into() } else { String::new() },
            logo: tp.get_str("logo").map(str::to_string).unwrap_or_default(),
            cover: tp.get_str("cover").map(str::to_string).unwrap_or_default(),
            accent: tp.get_str("accent").map(str::to_string).unwrap_or_default(),
            monogram: tp.get_str("monogram").map(str::to_string).unwrap_or_default(),
            updated: tp.get_str("updated").map(str::to_string).unwrap_or_default(),
            link: tp.get_str("link").map(str::to_string).unwrap_or_default(),
            seats_per_container: spec
                .seats_per_container
                .map(|n| n.to_string())
                .unwrap_or_default(),
            max_lifetime: spec.max_lifetime.map(|n| n.to_string()).unwrap_or_default(),
            heartbeat_timeout: spec
                .heartbeat_timeout
                .map(|n| n.to_string())
                .unwrap_or_default(),
            container_cpu_limit: spec
                .container_cpu_limit
                .map(|n| n.to_string())
                .unwrap_or_default(),
            container_memory_limit: spec.container_memory_limit.clone().unwrap_or_default(),
            min_replicas: spec.min_replicas.map(|n| n.to_string()).unwrap_or_default(),
            max_replicas: spec.max_replicas.map(|n| n.to_string()).unwrap_or_default(),
            concurrent_requests_per_replica: spec
                .concurrent_requests_per_replica
                .map(|n| n.to_string())
                .unwrap_or_default(),
            volumes: spec
                .volumes
                .as_ref()
                .map(|v| v.join("\n"))
                .unwrap_or_default(),
            api_port: spec
                .api
                .as_ref()
                .and_then(|a| a.port)
                .map(|n| n.to_string())
                .unwrap_or_default(),
            api_docs_path: spec
                .api
                .as_ref()
                .and_then(|a| a.docs_path.clone())
                .unwrap_or_default(),
            api_health_path: spec
                .api
                .as_ref()
                .and_then(|a| a.health_path.clone())
                .unwrap_or_default(),
            api_rate_limit: spec
                .api
                .as_ref()
                .and_then(|a| a.rate_limit.clone())
                .unwrap_or_default(),
            api_cors: if spec.api.as_ref().map(|a| a.cors).unwrap_or(false) {
                "on".into()
            } else {
                String::new()
            },
            inject_base_href: if spec.effective_inject_base_href() {
                "on".into()
            } else {
                String::new()
            },

            // ── Advanced (new) ──────────────────────────────────
            container_port: spec.container_port.map(|n| n.to_string()).unwrap_or_default(),
            platform: spec.platform.clone().unwrap_or_default(),
            container_network: spec.container_network.clone().unwrap_or_default(),
            container_lifetime: spec.container_lifetime.map(|n| n.to_string()).unwrap_or_default(),
            stop_on_logout: checkbox(spec.stop_on_logout.unwrap_or(false)),
            // `container-env` shown as sorted `NAME=value` lines.
            container_env: spec.env_pairs().join("\n"),
            container_cmd: spec
                .container_cmd
                .as_ref()
                .map(|v| v.join("\n"))
                .unwrap_or_default(),
            docker_registry_domain: spec.docker_registry_domain.clone().unwrap_or_default(),
            docker_registry_username: spec.docker_registry_username.clone().unwrap_or_default(),
            // Never prefill the registry password (#260): the stored
            // value is a `${VAR}` literal at best and a legacy cleartext
            // secret at worst — neither should be rendered. Blank here +
            // "blank ⇒ keep" in `into_spec` makes the field write-only.
            docker_registry_password: String::new(),
            docker_registry_credential: spec.docker_registry_credential.clone().unwrap_or_default(),
            access_groups: spec.access_groups.as_ref().map(|v| v.join(", ")).unwrap_or_default(),
            access_users: spec.access_users.as_ref().map(|v| v.join(", ")).unwrap_or_default(),
            container_cpu_request: spec
                .container_cpu_request
                .map(|n| n.to_string())
                .unwrap_or_default(),
            container_memory_request: spec.container_memory_request.clone().unwrap_or_default(),
            max_body_size: spec.max_body_size.clone().unwrap_or_default(),
            scale_up_threshold: spec.scale_up_threshold.map(|f| f.0.to_string()).unwrap_or_default(),
            scale_down_threshold: spec
                .scale_down_threshold
                .map(|f| f.0.to_string())
                .unwrap_or_default(),
            scale_down_grace: spec.scale_down_grace.map(|n| n.to_string()).unwrap_or_default(),
            drain_timeout: spec.drain_timeout.map(|n| n.to_string()).unwrap_or_default(),
            routing_strategy: spec.routing_strategy.map(routing_to_key).unwrap_or_default(),
            placement: spec.placement.map(placement_to_key).unwrap_or_default(),
            anti_affinity: checkbox(spec.anti_affinity.unwrap_or(false)),
        }
    }

    /// Build a [`Spec`] from the submitted form, **merged onto `base`**
    /// (the existing spec, on edit). The form overwrites only the fields
    /// it owns; everything it doesn't model — `container-lifetime`,
    /// `docker-registry-*`, `*-request`, `max-body-size`, scaling
    /// thresholds, `routing-strategy`, `stop-on-logout`, and any custom
    /// `template-properties` keys — passes through from `base` instead of
    /// being silently dropped. `base` is `None` for a brand-new spec.
    /// Empty optional strings become `None`; numeric strings parse
    /// optimistically.
    pub fn into_spec(self, base: Option<&Spec>, role: Role) -> Result<Spec> {
        let dt = DisplayType::parse(&self.display_type).unwrap_or(DisplayType::App);
        // App/API/External set an explicit kind; Talk/Report are purely
        // visual, so keep whatever run-kind override `base` carried.
        let kind_override = match dt {
            // "App" badge covers Shiny + generic interactive apps. Preserve
            // an existing app-family run-kind (App/Streamlit/Dash/Voilà) so
            // editing e.g. a Jupyter card doesn't silently revert it to
            // Shiny (#231); a brand-new "app" still defaults to Shiny.
            DisplayType::App => match base.and_then(|b| b.kind_override) {
                Some(
                    k @ (SpecKindOverride::App
                    | SpecKindOverride::Streamlit
                    | SpecKindOverride::Dash
                    | SpecKindOverride::Voila),
                ) => Some(k),
                _ => Some(SpecKindOverride::Shiny),
            },
            DisplayType::Talk | DisplayType::Report => base.and_then(|b| b.kind_override),
            DisplayType::Package | DisplayType::Link => Some(SpecKindOverride::External),
            DisplayType::Api => Some(SpecKindOverride::Api),
        };

        let updated = if self.updated.trim().is_empty() {
            Utc::now().format("%d/%m/%Y").to_string()
        } else {
            self.updated.trim().to_string()
        };

        // Start from base so custom template-properties keys (anything
        // the form doesn't render) survive an edit, then overwrite the
        // managed keys. Empty managed fields are *removed* so the form
        // stays authoritative for the keys it owns.
        let mut tp_map: HashMap<String, YamlValue> = base
            .map(|b| b.template_properties.0.clone())
            .unwrap_or_default();
        // type, state, updated are always set so chips/filters render.
        tp_map.insert("type".into(), YamlValue::String(dt.key().to_string()));
        tp_map.insert("state".into(), YamlValue::String(self.state.clone()));
        tp_map.insert("updated".into(), YamlValue::String(updated));
        // The access lock is now derived from `Spec::is_open()`
        // (`access-groups`/`access-users`), not a decorative `icon`
        // flag — prune any stale value inherited from an older DB (#346).
        tp_map.remove("icon");
        set_or_remove(&mut tp_map, "subject", &self.subject);
        set_or_remove(&mut tp_map, "logo", &self.logo);
        set_or_remove(&mut tp_map, "cover", &self.cover);
        set_or_remove(&mut tp_map, "accent", &self.accent);
        set_or_remove(&mut tp_map, "monogram", &self.monogram);
        set_or_remove(&mut tp_map, "link", &self.link);
        // Prune the dead decorative-lock flag (#839, removed in #858) from
        // any spec that still carries it, on save.
        tp_map.remove("locked");

        let container_image = match dt {
            DisplayType::Package | DisplayType::Link => None,
            _ => empty_to_none(&self.container_image),
        };

        // Advanced API block: built for API specs, or whenever any
        // API field was filled in (empty otherwise ⇒ schema defaults).
        let cors = !self.api_cors.trim().is_empty();
        let api_filled = cors
            || [
                &self.api_port,
                &self.api_docs_path,
                &self.api_health_path,
                &self.api_rate_limit,
            ]
            .iter()
            .any(|s| !s.trim().is_empty());
        let api = if matches!(dt, DisplayType::Api) || api_filled {
            Some(ApiSpec {
                port: parse_opt(&self.api_port),
                docs_path: empty_to_none(&self.api_docs_path),
                health_path: empty_to_none(&self.api_health_path),
                rate_limit: empty_to_none(&self.api_rate_limit),
                cors,
            })
        } else {
            None
        };

        Ok(Spec {
            id: self.id.trim().to_string(),
            display_name: empty_to_none(&self.display_name),
            description: empty_to_none(&self.description),
            // "Featured" carousel flag (#506) — checkbox present ⇒ true;
            // absent ⇒ None (so a normal spec carries no `featured` noise).
            featured: (!self.featured.trim().is_empty()).then_some(true),
            container_image,
            seats_per_container: parse_opt(&self.seats_per_container),
            max_lifetime: parse_opt(&self.max_lifetime),
            heartbeat_timeout: parse_opt(&self.heartbeat_timeout),
            container_cpu_limit: parse_opt(&self.container_cpu_limit),
            container_memory_limit: empty_to_none(&self.container_memory_limit),
            template_properties: TemplateProperties(tp_map),
            kind_override,
            api,
            min_replicas: parse_opt(&self.min_replicas),
            max_replicas: parse_opt(&self.max_replicas),
            concurrent_requests_per_replica: parse_opt(&self.concurrent_requests_per_replica),
            // Bind-mount volumes are an Admin-only field (#302): they map
            // to `HostConfig.binds`, i.e. host access. An Editor's submit
            // keeps the base spec's volumes; only an Admin can set/change
            // them. (The form also hides the field for non-Admins.)
            volumes: if role == Role::Admin {
                lines_to_vec(&self.volumes)
            } else {
                base.and_then(|b| b.volumes.clone())
            },
            // Checked ⇒ leave unset (the `true` default keeps the
            // exported YAML clean); unchecked ⇒ explicit `false`.
            inject_base_href: if self.inject_base_href.trim().is_empty() {
                Some(false)
            } else {
                None
            },
            // ── Advanced fields: the form is authoritative now (it
            //    pre-fills from the spec in `from_spec`), so blank ⇒
            //    None ⇒ the schema/runtime default. Clearing a field
            //    clears it; an untouched field round-trips. ──────────
            container_port: parse_opt(&self.container_port),
            platform: empty_to_none(&self.platform),
            // Advanced field, form is authoritative (pre-filled from the
            // spec in `from_spec`): blank ⇒ None ⇒ daemon default bridge.
            container_network: empty_to_none(&self.container_network),
            // `labels` (#851) has no form input yet — preserve the base
            // spec's value so a YAML/import-set label map survives an
            // admin edit instead of being wiped. UI input is a follow-up.
            labels: base.and_then(|b| b.labels.clone()),
            container_lifetime: parse_opt(&self.container_lifetime),
            stop_on_logout: checkbox_opt(&self.stop_on_logout),
            container_env: parse_env(&self.container_env),
            container_cmd: lines_to_vec(&self.container_cmd),
            docker_registry_domain: empty_to_none(&self.docker_registry_domain),
            docker_registry_username: empty_to_none(&self.docker_registry_username),
            // Write-only (#260): a blank field keeps the existing stored
            // value (the form never shows it), so editing a spec doesn't
            // wipe the password; a non-blank value replaces it.
            docker_registry_password: match empty_to_none(&self.docker_registry_password) {
                Some(v) => Some(v),
                None => base.and_then(|b| b.docker_registry_password.clone()),
            },
            docker_registry_credential: empty_to_none(&self.docker_registry_credential),
            access_groups: list_to_vec(&self.access_groups),
            access_users: list_to_vec(&self.access_users),
            container_cpu_request: parse_opt(&self.container_cpu_request),
            container_memory_request: empty_to_none(&self.container_memory_request),
            max_body_size: empty_to_none(&self.max_body_size),
            scale_up_threshold: parse_opt::<f64>(&self.scale_up_threshold).map(OrderedFloat),
            scale_down_threshold: parse_opt::<f64>(&self.scale_down_threshold).map(OrderedFloat),
            scale_down_grace: parse_opt(&self.scale_down_grace),
            drain_timeout: parse_opt(&self.drain_timeout),
            routing_strategy: routing_from_key(&self.routing_strategy),
            placement: placement_from_key(&self.placement),
            anti_affinity: checkbox_opt(&self.anti_affinity),
        })
    }

    /// Server-side validation. Returns a list of fluent message
    /// keys describing each problem; empty list = OK.
    pub fn validate(&self, mode: FormMode) -> Vec<&'static str> {
        let mut errs = Vec::new();
        if self.id.trim().is_empty() {
            errs.push("spec-form-error-id-required");
        } else if !is_kebab_id(self.id.trim()) {
            errs.push("spec-form-error-id-shape");
        }
        if self.display_name.trim().is_empty() {
            errs.push("spec-form-error-name-required");
        }
        if matches!(mode, FormMode::New) && self.id.trim().is_empty() {
            // duplicate-id check happens later (needs DB access)
        }

        // Numeric fields: a non-empty but unparseable value used to be
        // silently dropped to None (= schema default). Flag it instead,
        // so a pt-BR `0,5` CPU or a typo'd count doesn't quietly mean
        // "no limit / default".
        let int_fields = [
            &self.seats_per_container,
            &self.max_lifetime,
            &self.heartbeat_timeout,
            &self.min_replicas,
            &self.max_replicas,
            &self.concurrent_requests_per_replica,
            &self.api_port,
            &self.container_port,
            &self.container_lifetime,
            &self.scale_down_grace,
            &self.drain_timeout,
        ];
        if int_fields
            .iter()
            .any(|v| !v.trim().is_empty() && v.trim().parse::<i64>().is_err())
        {
            errs.push("spec-form-error-number");
        }
        // A max-containers ceiling of 0 refuses every spawn (`live >= max`),
        // so the app can never start. Reject it server-side, not just via
        // the HTML `min=1` a crafted POST can bypass (#877).
        if matches!(self.max_replicas.trim().parse::<i64>(), Ok(n) if n <= 0) {
            errs.push("spec-form-error-max-replicas-zero");
        }
        // Ports are 1–65535.
        for p in [&self.container_port, &self.api_port] {
            if !p.trim().is_empty()
                && !matches!(p.trim().parse::<u32>(), Ok(n) if (1..=65535).contains(&n))
            {
                errs.push("spec-form-error-port");
                break;
            }
        }

        // CPU (limit + request) must be a positive, finite number of
        // cores (catches `0,5`).
        for cpu in [&self.container_cpu_limit, &self.container_cpu_request] {
            if !cpu.trim().is_empty()
                && !matches!(cpu.trim().parse::<f64>(), Ok(v) if v.is_finite() && v > 0.0)
            {
                errs.push("spec-form-error-cpu");
                break;
            }
        }

        // Memory-style sizes: limit, request, and the per-spec body cap.
        for mem in [
            &self.container_memory_limit,
            &self.container_memory_request,
            &self.max_body_size,
        ] {
            if !mem.trim().is_empty() && !ruscker_config::is_valid_memory_size(mem.trim()) {
                errs.push("spec-form-error-memory");
                break;
            }
        }

        // Scaling thresholds are utilization fractions in (0, 1].
        for th in [&self.scale_up_threshold, &self.scale_down_threshold] {
            if !th.trim().is_empty()
                && !matches!(th.trim().parse::<f64>(), Ok(v) if v.is_finite() && v > 0.0 && v <= 1.0)
            {
                errs.push("spec-form-error-threshold");
                break;
            }
        }

        // Replica pool: max must be >= min when both are given.
        if let (Ok(min), Ok(max)) = (
            self.min_replicas.trim().parse::<u32>(),
            self.max_replicas.trim().parse::<u32>(),
        ) {
            if max < min {
                errs.push("spec-form-error-replica-range");
            }
        }

        // Each volume line must be valid Docker bind syntax.
        if self
            .volumes
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .any(|l| !ruscker_config::is_valid_volume_bind(l))
        {
            errs.push("spec-form-error-volume");
        }

        // container-network (when set) must be a valid Docker network name,
        // else the container create fails at spawn (#892).
        if !self.container_network.trim().is_empty()
            && !ruscker_config::is_valid_network_name(self.container_network.trim())
        {
            errs.push("spec-form-error-network");
        }

        // container-env: every non-blank line must be `NAME=value` with a
        // valid NAME. A typo'd line (missing `=`, or a key with spaces /
        // bad chars) used to be silently dropped by `parse_env` — turning
        // a mistake into lost config. Block the save instead (#720 P4).
        if self
            .container_env
            .lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .any(|l| match l.split_once('=') {
                None => true,
                Some((k, _)) => !is_valid_env_key(k.trim()),
            })
        {
            errs.push("spec-form-error-env");
        }

        errs
    }
}

/// A valid environment-variable name: a letter or `_`, then letters,
/// digits or `_` (the POSIX-ish shape Docker accepts). Empty is invalid.
fn is_valid_env_key(k: &str) -> bool {
    let mut chars = k.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn empty_to_none(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() { None } else { Some(t.to_string()) }
}
/// Set a managed template-property to the trimmed value, or remove the
/// key entirely when the form left it blank (so clearing a field clears
/// it, while unmanaged keys merged from the base stay put).
fn set_or_remove(map: &mut HashMap<String, YamlValue>, key: &str, val: &str) {
    let t = val.trim();
    if t.is_empty() {
        map.remove(key);
    } else {
        map.insert(key.to_string(), YamlValue::String(t.to_string()));
    }
}
fn parse_opt<T: std::str::FromStr>(s: &str) -> Option<T> {
    s.trim().parse().ok()
}
/// Split a textarea into trimmed, non-empty lines — `None` if all blank.
/// Used for the volumes field (one `host:container[:ro]` per line).
fn lines_to_vec(s: &str) -> Option<Vec<String>> {
    let v: Vec<String> = s
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}
/// Checkbox value for `from_spec`: `"on"` when set, else empty.
fn checkbox(on: bool) -> String {
    if on { "on".into() } else { String::new() }
}
/// Checkbox parse: non-empty (`"on"`) ⇒ `Some(true)`; blank ⇒ `None`
/// (the default-`false` semantics, kept out of the exported YAML).
fn checkbox_opt(s: &str) -> Option<bool> {
    if s.trim().is_empty() { None } else { Some(true) }
}
/// Parse `container-env` from a textarea: one `NAME=value` per line.
/// The first `=` splits; blank lines and lines without `=` are skipped.
/// `BTreeMap` so the resulting env list is deterministically ordered.
/// `None` when nothing valid is present.
fn parse_env(s: &str) -> Option<BTreeMap<String, String>> {
    let map: BTreeMap<String, String> = s
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let (k, v) = line.split_once('=')?;
            let k = k.trim();
            if k.is_empty() {
                return None;
            }
            Some((k.to_string(), v.trim().to_string()))
        })
        .collect();
    if map.is_empty() { None } else { Some(map) }
}
/// Parse an access list (`access-groups` / `access-users`) from a field
/// that accepts commas and/or newlines. `None` when all blank.
fn list_to_vec(s: &str) -> Option<Vec<String>> {
    let v: Vec<String> = s
        .split([',', '\n'])
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect();
    if v.is_empty() { None } else { Some(v) }
}
/// `routing-strategy` ↔ form key (the enum's kebab serde value).
fn routing_from_key(s: &str) -> Option<RoutingStrategy> {
    match s.trim() {
        "least-connections" => Some(RoutingStrategy::LeastConnections),
        "round-robin" => Some(RoutingStrategy::RoundRobin),
        "weighted-random" => Some(RoutingStrategy::WeightedRandom),
        "resource-aware" => Some(RoutingStrategy::ResourceAware),
        _ => None,
    }
}
fn routing_to_key(r: RoutingStrategy) -> String {
    match r {
        RoutingStrategy::LeastConnections => "least-connections",
        RoutingStrategy::RoundRobin => "round-robin",
        RoutingStrategy::WeightedRandom => "weighted-random",
        RoutingStrategy::ResourceAware => "resource-aware",
    }
    .into()
}
/// `placement` ↔ form key (the enum's kebab serde value).
fn placement_from_key(s: &str) -> Option<Placement> {
    match s.trim() {
        "spread" => Some(Placement::Spread),
        "bin-pack" => Some(Placement::BinPack),
        _ => None,
    }
}
fn placement_to_key(p: Placement) -> String {
    match p {
        Placement::Spread => "spread",
        Placement::BinPack => "bin-pack",
    }
    .into()
}
pub(crate) fn is_kebab_id(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        && s.chars()
            .next()
            .map(|c| c.is_ascii_alphabetic())
            .unwrap_or(false)
}

// ── Template ────────────────────────────────────────────────────

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum FormMode {
    New,
    Edit,
}
impl FormMode {
    pub fn is_new(self) -> bool {
        matches!(self, FormMode::New)
    }
    pub fn is_edit(self) -> bool {
        matches!(self, FormMode::Edit)
    }
}

#[derive(Template)]
#[template(path = "admin/spec_form.html")]
struct SpecFormPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// Mount prefix for base-path-correct URLs (#294).
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    /// Current session role (Editor or Admin) — drives nav gating.
    role: Role,
    mode: FormMode,
    form: SpecForm,
    /// True right after a successful create — `create` redirects to this
    /// edit form with `?created=1`, and the template then shows a
    /// confirmation dialog offering "back to the form" vs "go to the apps
    /// list" (#835). False on a plain edit/new/duplicate render.
    just_created: bool,
    /// Pre-validation errors (Fluent keys) shown above the form.
    errors: Vec<&'static str>,
    /// Filenames in the media library, for the logo picker. Empty
    /// when no DB is wired or the listing fails.
    logo_images: Vec<String>,
    /// Distinct subjects already used across the catalog, for the
    /// subject `<datalist>` (#746) — replaces a hardcoded pt-BR list
    /// that shipped domain-specific suggestions to every deployment
    /// and locale.
    subject_suggestions: Vec<String>,
    /// Names of stored registry credentials, for the
    /// `docker-registry-credential` datalist (#351). Empty when no DB
    /// is wired or the listing fails — the field stays free-text.
    credential_names: Vec<String>,
    /// Known group names (from every spec's `access-groups` + every user's
    /// memberships), for the access-group pill picker (#623). The field
    /// still accepts arbitrary names via the "add group" input.
    available_groups: Vec<String>,
}

impl<'a> SpecFormPage<'a> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }

    /// Whether `name` is the currently-selected registry credential — marks
    /// the `<option selected>` in the credential picker (#504).
    fn cred_selected(&self, name: &str) -> bool {
        self.form.docker_registry_credential == name
    }

    /// Whether the set credential is empty or a known stored name. False
    /// when it's a value absent from the store (a deleted credential, or one
    /// carried over from imported YAML) — the picker then keeps it as a
    /// trailing option so a save doesn't silently drop it (#504).
    fn cred_known(&self) -> bool {
        let cur = self.form.docker_registry_credential.trim();
        cur.is_empty() || self.credential_names.iter().any(|n| n.as_str() == cur)
    }

    /// JSON-encoded initial form values, ready to drop into the
    /// `x-data` attribute of the live-preview Alpine component.
    fn form_initial_json(&self) -> String {
        serde_json::to_string(&self.form).unwrap_or_else(|_| "{}".into())
    }

    /// Media-library filenames as a JSON array, seeding the Alpine logo
    /// picker so an inline upload can push new thumbnails reactively.
    fn logo_images_json(&self) -> String {
        serde_json::to_string(&self.logo_images).unwrap_or_else(|_| "[]".into())
    }

    /// Known group names as a JSON array, seeding the access-group pill
    /// picker (#623).
    fn available_groups_json(&self) -> String {
        serde_json::to_string(&self.available_groups).unwrap_or_else(|_| "[]".into())
    }


    /// Options for the kind picker: (key, label-fluent-key, tabler-icon).
    /// Order intentional — mirrors the public landing chip order.
    fn display_type_options(&self) -> &'static [(&'static str, &'static str, &'static str)] {
        &[
            ("app", "spec-form-kind-app", "app-window"),
            ("talk", "spec-form-kind-talk", "presentation"),
            ("report", "spec-form-kind-report", "file-text"),
            ("package", "spec-form-kind-package", "package"),
            ("api", "spec-form-kind-api", "api"),
            ("link", "spec-form-kind-link", "external-link"),
        ]
    }
}

// ── Handlers ─────────────────────────────────────────────────────

/// Media-library filenames for the logo picker. Empty when no DB is
/// wired or the query fails — the picker degrades to the text field.
/// Distinct, sorted `template-properties.subject` values across the
/// effective catalog (#746) — real data instead of a fixed list.
async fn subject_suggestions(state: &AppState) -> Vec<String> {
    let specs = crate::catalog::effective_specs(state.db.as_ref(), &state.config).await;
    let mut subjects: Vec<String> = specs
        .iter()
        .filter_map(|sp| sp.template_properties.get_str("subject"))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    subjects.sort();
    subjects.dedup();
    subjects
}

async fn logo_filenames(state: &AppState) -> Vec<String> {
    match state.db.as_ref() {
        Some(pool) => db::images::list_all(pool)
            .await
            .map(|imgs| imgs.into_iter().map(|i| i.filename).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Names of stored registry credentials, for the spec form's
/// `docker-registry-credential` datalist (#351) — surfaces the names
/// the operator created on the Credentials page so the two screens
/// connect. Empty when no DB is wired or the query fails; the field
/// then degrades to a plain free-text input.
async fn credential_names(state: &AppState) -> Vec<String> {
    match state.db.as_ref() {
        Some(pool) => db::credentials::list_all(pool)
            .await
            .map(|creds| creds.into_iter().map(|c| c.name).collect())
            .unwrap_or_default(),
        None => Vec::new(),
    }
}

/// Known group names for the access-group pill picker (#623): the union of
/// every effective spec's `access-groups` and every user's memberships,
/// sorted and de-duplicated. Empty when no DB is wired — the picker then
/// just offers the "add group" input.
async fn group_names(state: &AppState) -> Vec<String> {
    let mut set = std::collections::BTreeSet::new();
    for s in crate::catalog::effective_specs(state.db.as_ref(), &state.config).await {
        if let Some(groups) = s.access_groups.as_ref() {
            set.extend(groups.iter().cloned());
        }
    }
    if let Some(db) = state.db.as_ref() {
        if let Ok(users) = db::users::list_all(db).await {
            for u in users {
                set.extend(u.groups);
            }
        }
    }
    set.into_iter().collect()
}

async fn new_form(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    let page = SpecFormPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "specs",
        role: editor.role,
        mode: FormMode::New,
        form: SpecForm {
            // Sensible defaults for a new app
            display_type: "app".into(),
            state: "active".into(),            // The HTML base-href transform is on by default — the
            // safe behaviour for apps that don't self-route.
            inject_base_href: "on".into(),
            ..Default::default()
        },
        just_created: false,
        errors: Vec::new(),
        logo_images: logo_filenames(&state).await,
        subject_suggestions: subject_suggestions(&state).await,
        available_groups: group_names(&state).await,
        credential_names: credential_names(&state).await,
    };
    super::render(&page)
}

/// Query for the edit form. `created=1` is set by the create redirect so
/// the freshly-created app shows the post-create confirmation (#835).
#[derive(Debug, Default, Deserialize)]
struct EditFormQuery {
    created: Option<String>,
}

async fn edit_form(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Path(id): Path<String>,
    Query(q): Query<EditFormQuery>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let spec = match db::specs::fetch_one(pool, &id).await {
        Ok(Some(s)) => s,
        Ok(None) => return (StatusCode::NOT_FOUND, "spec not found").into_response(),
        Err(e) => {
            tracing::error!(error = ?e, id, "fetch spec failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let page = SpecFormPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "specs",
        role: editor.role,
        mode: FormMode::Edit,
        form: {
            let mut f = SpecForm::from_spec(&spec);
            // Optimistic-concurrency token (#745): the template emits it
            // as a hidden field; `update` rejects a stale submit.
            f.base_version = db::specs::fetch_version(pool, &id)
                .await
                .ok()
                .flatten()
                .map(|v| v.to_string())
                .unwrap_or_default();
            f
        },
        just_created: q.created.is_some(),
        errors: Vec::new(),
        logo_images: logo_filenames(&state).await,
        subject_suggestions: subject_suggestions(&state).await,
        available_groups: group_names(&state).await,
        credential_names: credential_names(&state).await,
    };
    super::render(&page)
}

/// Pick a fresh `{base}-copy[-N]` id that no spec uses yet, so the
/// duplicate doesn't collide with the source (or a previous copy).
/// Falls back to the bare `-copy` after a sane cap — the create-time
/// duplicate-id validation is the final backstop.
async fn unique_copy_id(pool: &crate::db::ConfigDb, base: &str) -> String {
    let first = format!("{base}-copy");
    let free = |id: &str| {
        let id = id.to_string();
        async move { matches!(db::specs::fetch_one(pool, &id).await, Ok(None)) }
    };
    if free(&first).await {
        return first;
    }
    for n in 2..=99 {
        let candidate = format!("{base}-copy-{n}");
        if free(&candidate).await {
            return candidate;
        }
    }
    first
}

/// Open the **New** spec form pre-filled from an existing spec (#368).
/// The operator duplicates a near-identical card and edits only what
/// differs. The form starts in `FormMode::New` with a fresh unique id,
/// so the submit hits `POST /admin/specs` and creates a brand-new spec
/// (the source is untouched). Works on `config`-only specs too — that's
/// a handy way to fork a YAML-defined spec into an editable DB one. The
/// registry password is write-only (#260), so it isn't carried over.
async fn duplicate_form(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Path(id): Path<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    let spec = match db::specs::fetch_one(pool, &id).await {
        Ok(Some(s)) => s,
        Ok(None) => return (StatusCode::NOT_FOUND, "spec not found").into_response(),
        Err(e) => {
            tracing::error!(error = ?e, id, "fetch spec for duplicate failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    let mut form = SpecForm::from_spec(&spec);
    form.id = unique_copy_id(pool, &spec.id).await;
    let page = SpecFormPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "specs",
        role: editor.role,
        mode: FormMode::New,
        form,
        just_created: false,
        errors: Vec::new(),
        logo_images: logo_filenames(&state).await,
        subject_suggestions: subject_suggestions(&state).await,
        available_groups: group_names(&state).await,
        credential_names: credential_names(&state).await,
    };
    super::render(&page)
}

async fn create(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Form(form): Form<SpecForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };

    let mut errors = form.validate(FormMode::New);

    // Uniqueness check
    if errors.is_empty() {
        match db::specs::fetch_one(pool, form.id.trim()).await {
            Ok(Some(_)) => errors.push("spec-form-error-id-duplicate"),
            Ok(None) => {}
            Err(e) => {
                tracing::error!(error = ?e, "duplicate-check failed");
                return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
            }
        }
    }

    if !errors.is_empty() {
        return render_form_with_errors(
            &state,
            loc,
            theme,
            editor.role,
            FormMode::New,
            form,
            errors,
        )
        .await;
    }

    let id = form.id.trim().to_string();
    let spec = match form.into_spec(None, editor.role) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = ?e, "form → spec failed");
            return (StatusCode::BAD_REQUEST, "invalid form data").into_response();
        }
    };

    // insert_new fails CLOSED on an existing id (#745) — the friendly
    // pre-check above is just UX; this is the race-proof gate (the old
    // upsert silently overwrote the loser of a concurrent create).
    match db::specs::insert_new(pool, &spec, Some(editor.actor())).await {
        // Land on the new app's edit form with `?created=1` so the page
        // shows the post-create confirmation (#835): keep editing here, or
        // jump to the apps list.
        Ok(true) => {
            Redirect::to(&format!("/admin/specs/{}/edit?created=1", id)).into_response()
        }
        Ok(false) => {
            render_form_with_errors(
                &state,
                loc,
                theme,
                editor.role,
                FormMode::New,
                SpecForm::from_spec(&spec),
                vec!["spec-form-error-id-duplicate"],
            )
            .await
        }
        Err(e) => {
            tracing::error!(error = ?e, "save failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "save failed").into_response()
        }
    }
}

async fn update(
    editor: RequireEditor,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Path(id): Path<String>,
    Form(mut form): Form<SpecForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };

    // The URL id wins over the form id — operators don't get to
    // rename specs through the form (it would orphan the audit
    // log target). Renaming is a separate planned action.
    form.id = id.clone();

    let errors = form.validate(FormMode::Edit);
    if !errors.is_empty() {
        return render_form_with_errors(
            &state,
            loc,
            theme,
            editor.role,
            FormMode::Edit,
            form,
            errors,
        )
        .await;
    }

    // Load the existing spec as the merge base so fields the form
    // doesn't model (registry creds, lifetimes, limits, scaling, custom
    // template-properties) survive the edit instead of being wiped.
    let base = match db::specs::fetch_one(pool, &id).await {
        Ok(b) => b,
        Err(e) => {
            tracing::error!(error = ?e, id, "load base spec failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    // This is the *edit* path — the spec must already exist. Without
    // this guard `into_spec(None)` + `upsert_one` would silently
    // (re)create a spec at this id (e.g. a stale tab POSTing to a
    // since-deleted spec). #261
    if base.is_none() {
        return (StatusCode::NOT_FOUND, format!("spec `{id}` not found")).into_response();
    }

    // Optimistic concurrency (#745): the form carries the version it
    // was rendered against; if someone saved meanwhile, re-render with
    // a conflict error instead of silently last-write-winning over
    // their changes. An empty token (create form / pre-#745 tab) skips
    // the check.
    if let Ok(submitted) = form.base_version.trim().parse::<i64>() {
        let current = db::specs::fetch_version(pool, &id).await.ok().flatten();
        if current.is_some_and(|v| v != submitted) {
            let mut stale = form;
            stale.base_version = current.map(|v| v.to_string()).unwrap_or_default();
            return render_form_with_errors(
                &state,
                loc,
                theme,
                editor.role,
                FormMode::Edit,
                stale,
                vec!["spec-form-error-stale"],
            )
            .await;
        }
    }

    let spec = match form.into_spec(base.as_ref(), editor.role) {
        Ok(s) => s,
        Err(e) => {
            tracing::error!(error = ?e, "form → spec failed");
            return (StatusCode::BAD_REQUEST, "invalid form data").into_response();
        }
    };

    match db::specs::upsert_one(pool, &spec, Some(editor.actor())).await {
        Ok(_) => Redirect::to(&format!("/admin/specs/{}/edit", id)).into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "save failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "save failed").into_response()
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
    match db::specs::delete_one(pool, &id, Some(editor.actor())).await {
        Ok(_) => {
            // Reap the app's containers so a delete doesn't leave orphans
            // eating disk (#453). Best-effort and logged inside; the DB row
            // is already gone, so we never block the redirect on Docker.
            crate::scaler::stop_spec(&state, &id).await;
            Redirect::to("/admin/specs").into_response()
        }
        Err(e) => {
            tracing::error!(error = ?e, id, "delete failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "delete failed").into_response()
        }
    }
}

async fn render_form_with_errors(
    state: &AppState,
    loc: Locale,
    theme: Theme,
    role: Role,
    mode: FormMode,
    form: SpecForm,
    errors: Vec<&'static str>,
) -> Response {
    let page = SpecFormPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "specs",
        role,
        mode,
        form,
        just_created: false,
        errors,
        logo_images: logo_filenames(state).await,
        subject_suggestions: subject_suggestions(state).await,
        available_groups: group_names(state).await,
        credential_names: credential_names(state).await,
    };
    let body = match page.render() {
        Ok(s) => s,
        Err(_) => return (StatusCode::INTERNAL_SERVER_ERROR, "render").into_response(),
    };
    (StatusCode::UNPROCESSABLE_ENTITY, axum::response::Html(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ruscker_config::Config;

    /// A spec carrying fields the form doesn't model must survive an
    /// edit: `from_spec` (load into the form) → `into_spec(Some(base))`
    /// (save) must preserve registry creds, lifetimes, limits, scaling
    /// thresholds and custom template-properties. Regression for #74.
    #[test]
    fn edit_preserves_unmodelled_fields() {
        let yaml = r#"
proxy:
  specs:
    - id: ops
      display-name: Ops
      container-image: registry.example.com/acme/ops:latest
      container-lifetime: 360
      stop-on-logout: true
      docker-registry-username: acme
      docker-registry-domain: registry.example.com
      docker-registry-credential: dh-creds
      container-cpu-request: 0.25
      container-memory-request: 128m
      max-body-size: 25m
      routing-strategy: round-robin
      min-replicas: 1
      max-replicas: 4
      template-properties:
        type: app
        state: active
        custom-key: keep-me
"#;
        let cfg = Config::from_yaml(yaml).expect("parse fixture");
        let original = &cfg.proxy.specs[0];

        // Round-trip: load into the form, change a managed field, save.
        let mut form = SpecForm::from_spec(original);
        form.display_name = "Ops (edited)".into();
        let merged = form.into_spec(Some(original), Role::Admin).expect("into_spec");

        // Managed field changed.
        assert_eq!(merged.display_name.as_deref(), Some("Ops (edited)"));
        // Unmodelled fields preserved (the #74 bug would None these).
        assert_eq!(merged.container_lifetime, Some(360));
        assert_eq!(merged.stop_on_logout, Some(true));
        assert_eq!(merged.docker_registry_username.as_deref(), Some("acme"));
        assert_eq!(
            merged.docker_registry_domain.as_deref(),
            Some("registry.example.com")
        );
        assert_eq!(
            merged.docker_registry_credential.as_deref(),
            Some("dh-creds")
        );
        assert_eq!(merged.container_cpu_request, Some(0.25));
        assert_eq!(merged.container_memory_request.as_deref(), Some("128m"));
        assert_eq!(merged.max_body_size.as_deref(), Some("25m"));
        assert!(merged.routing_strategy.is_some());
        // Custom template-property survives.
        assert_eq!(
            merged.template_properties.get_str("custom-key"),
            Some("keep-me")
        );
        // Form-managed advanced fields still round-trip.
        assert_eq!(merged.min_replicas, Some(1));
        assert_eq!(merged.max_replicas, Some(4));
    }

    #[test]
    fn volumes_are_admin_only() {
        // #302: bind mounts map to HostConfig.binds (host access), so an
        // Editor's submit must NOT add/change them — only an Admin can.
        let base: Spec =
            serde_yaml_ng::from_str("id: x\ncontainer-image: nginx\nvolumes:\n  - /host:/data")
                .unwrap();
        assert_eq!(base.volumes.as_deref(), Some(&["/host:/data".to_string()][..]));

        // Editor tries to add a dangerous mount → ignored, base kept.
        let mut form = SpecForm::from_spec(&base);
        form.volumes = "/:/host\n/var/run/docker.sock:/sock".into();
        let editor_spec = form.into_spec(Some(&base), Role::Editor).unwrap();
        assert_eq!(editor_spec.volumes, base.volumes, "Editor can't change volumes");

        // Admin's submit takes effect.
        let mut form = SpecForm::from_spec(&base);
        form.volumes = "/srv/new:/data".into();
        let admin_spec = form.into_spec(Some(&base), Role::Admin).unwrap();
        assert_eq!(admin_spec.volumes.as_deref(), Some(&["/srv/new:/data".to_string()][..]));
    }

    /// #260: the registry password is write-only. The form never loads
    /// it (`from_spec` blanks it), and a blank submit keeps the existing
    /// stored value rather than wiping it; a typed value replaces it.
    #[test]
    fn registry_password_is_write_only() {
        let yaml = r#"
proxy:
  specs:
    - id: ops
      display-name: Ops
      container-image: registry.example.com/acme/ops:latest
      docker-registry-username: acme
      docker-registry-password: ${DOCKER_REGISTRY_PASSWORD}
"#;
        std::env::set_var("DOCKER_REGISTRY_PASSWORD", "test");
        let cfg = Config::from_yaml(yaml).expect("parse fixture");
        let original = &cfg.proxy.specs[0];
        // Stored literal preserved at parse (the model from this PR).
        assert_eq!(
            original.docker_registry_password.as_deref(),
            Some("${DOCKER_REGISTRY_PASSWORD}")
        );

        // The form never exposes it.
        let form = SpecForm::from_spec(original);
        assert_eq!(form.docker_registry_password, "");

        // Blank submit ⇒ keep the stored value.
        let kept = SpecForm::from_spec(original)
            .into_spec(Some(original), Role::Admin)
            .expect("into_spec");
        assert_eq!(
            kept.docker_registry_password.as_deref(),
            Some("${DOCKER_REGISTRY_PASSWORD}"),
            "blank field must not wipe the stored password"
        );

        // A typed value replaces it.
        let mut form = SpecForm::from_spec(original);
        form.docker_registry_password = "${OTHER_VAR}".into();
        let replaced = form.into_spec(Some(original), Role::Admin).expect("into_spec");
        assert_eq!(replaced.docker_registry_password.as_deref(), Some("${OTHER_VAR}"));
    }

    /// A brand-new spec (no base) has no unmodelled fields to carry.
    #[test]
    fn create_without_base_leaves_unmodelled_none() {
        let form = SpecForm {
            id: "fresh".into(),
            display_name: "Fresh".into(),
            display_type: "app".into(),
            state: "active".into(),            ..Default::default()
        };
        let spec = form.into_spec(None, Role::Admin).expect("into_spec");
        assert_eq!(spec.id, "fresh");
        assert_eq!(spec.container_lifetime, None);
        assert_eq!(spec.docker_registry_username, None);
        assert_eq!(spec.max_body_size, None);
    }

    fn valid_form() -> SpecForm {
        SpecForm {
            id: "ok".into(),
            display_name: "Ok".into(),
            display_type: "app".into(),
            state: "active".into(),            ..Default::default()
        }
    }

    // #79/#83: malformed numbers used to silently default; now they're
    // form errors instead of "no limit / default".
    #[test]
    fn validate_rejects_malformed_numbers() {
        let mut f = valid_form();
        f.container_cpu_limit = "0,5".into(); // pt-BR comma
        assert!(f.validate(FormMode::New).contains(&"spec-form-error-cpu"));

        let mut f = valid_form();
        f.container_memory_limit = "512mb".into(); // typo
        assert!(f
            .validate(FormMode::New)
            .contains(&"spec-form-error-memory"));

        let mut f = valid_form();
        f.seats_per_container = "ten".into();
        assert!(f
            .validate(FormMode::New)
            .contains(&"spec-form-error-number"));

        let mut f = valid_form();
        f.min_replicas = "5".into();
        f.max_replicas = "2".into();
        assert!(f
            .validate(FormMode::New)
            .contains(&"spec-form-error-replica-range"));
    }

    // #877: a max-containers ceiling of 0 (or negative) refuses every
    // spawn, so the app could never start — reject it at save instead of
    // relying on the HTML `min=1` a crafted POST bypasses.
    #[test]
    fn validate_rejects_zero_max_replicas() {
        let mut f = valid_form();
        f.max_replicas = "0".into();
        assert!(f
            .validate(FormMode::New)
            .contains(&"spec-form-error-max-replicas-zero"));

        // A normal ceiling is fine.
        let mut ok = valid_form();
        ok.max_replicas = "3".into();
        assert!(!ok
            .validate(FormMode::New)
            .contains(&"spec-form-error-max-replicas-zero"));
    }

    // #892: an invalid Docker network name is rejected at save (it would
    // otherwise fail only at spawn); a valid one and the empty default pass.
    #[test]
    fn validate_rejects_bad_container_network() {
        let mut bad = valid_form();
        bad.container_network = "not a net".into();
        assert!(bad
            .validate(FormMode::New)
            .contains(&"spec-form-error-network"));

        let mut ok = valid_form();
        ok.container_network = "ruscker_net".into();
        assert!(!ok
            .validate(FormMode::New)
            .contains(&"spec-form-error-network"));

        // Empty (the default) → daemon default bridge, no error.
        assert!(!valid_form()
            .validate(FormMode::New)
            .contains(&"spec-form-error-network"));
    }

    // #874: a pull whose follower never connects (or disconnects) must
    // not keep its forwarding task alive forever. With the bounded
    // channel, the producer's `send` errors once the receiver is gone, so
    // the forwarder returns promptly even with far more lines than the
    // channel can hold.
    #[tokio::test]
    async fn forward_pull_terminates_without_a_follower() {
        let (tx, rx) = tokio::sync::mpsc::channel::<PullEvent>(2);
        drop(rx); // no follower ever takes the receiver
        let lines: ruscker_core::LogStream =
            Box::pin(futures_util::stream::iter((0..1000).map(|i| format!("layer {i}"))));
        tokio::time::timeout(std::time::Duration::from_secs(5), forward_pull(lines, tx))
            .await
            .expect("forwarder must not hang when nobody is following");
    }

    // The happy path: every line is delivered, then exactly one terminal
    // `Done`, then the channel closes.
    #[tokio::test]
    async fn forward_pull_delivers_lines_then_done() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<PullEvent>(PULL_CHANNEL_CAP);
        let lines: ruscker_core::LogStream =
            Box::pin(futures_util::stream::iter(vec!["a".to_string(), "b".to_string()]));
        forward_pull(lines, tx).await;
        assert!(matches!(rx.recv().await, Some(PullEvent::Line(l)) if l == "a"));
        assert!(matches!(rx.recv().await, Some(PullEvent::Line(l)) if l == "b"));
        assert!(matches!(rx.recv().await, Some(PullEvent::Done)));
        assert!(rx.recv().await.is_none());
    }

    #[test]
    fn volumes_round_trip_and_validate() {
        let mut f = valid_form();
        f.volumes = "/srv/data:/data\n/srv/www:/www:ro\n".into();
        let spec = f.into_spec(None, Role::Admin).expect("into_spec");
        assert_eq!(
            spec.volumes,
            Some(vec![
                "/srv/data:/data".to_string(),
                "/srv/www:/www:ro".to_string()
            ])
        );
        // Round-trips back into the textarea (newline-joined).
        let back = SpecForm::from_spec(&spec);
        assert_eq!(back.volumes, "/srv/data:/data\n/srv/www:/www:ro");

        // A malformed bind is a form error.
        let mut bad = valid_form();
        bad.volumes = "not-a-bind".into();
        assert!(bad
            .validate(FormMode::New)
            .contains(&"spec-form-error-volume"));
    }

    #[test]
    fn validate_accepts_good_numbers() {
        let mut f = valid_form();
        f.container_cpu_limit = "0.5".into();
        f.container_memory_limit = "512m".into();
        f.seats_per_container = "10".into();
        f.min_replicas = "1".into();
        f.max_replicas = "3".into();
        f.heartbeat_timeout = "-1".into();
        let errs = f.validate(FormMode::New);
        assert!(errs.is_empty(), "expected no errors, got {errs:?}");
    }

    // ── #211: newly-modelled advanced fields ────────────────────

    #[test]
    fn editing_an_app_card_preserves_interactive_kind() {
        // #231: a Jupyter/RStudio card is kind App (InteractiveApp). The
        // form's "app" badge must keep that on save, not revert to Shiny.
        let base: Spec = serde_yaml_ng::from_str(
            "id: nb\ndisplay-name: Jupyter\ncontainer-image: x\ntype: app",
        )
        .unwrap();
        assert_eq!(base.kind(), ruscker_config::SpecKind::InteractiveApp);
        let mut form = SpecForm::from_spec(&base);
        form.display_name = "Jupyter (edited)".into();
        let merged = form.into_spec(Some(&base), Role::Admin).unwrap();
        assert_eq!(merged.kind(), ruscker_config::SpecKind::InteractiveApp, "App kept, not Shiny");

        // A brand-new "app" (no base) still defaults to Shiny.
        let fresh = SpecForm {
            id: "s".into(), display_name: "S".into(), display_type: "app".into(),
            state: "active".into(), container_image: "x".into(),
            ..Default::default()
        };
        assert_eq!(fresh.into_spec(None, Role::Admin).unwrap().kind(), ruscker_config::SpecKind::Shiny);
    }

    #[test]
    fn new_advanced_fields_round_trip() {
        // Build a spec carrying every newly-modelled field, load it into
        // the form, and save it back unchanged — values must survive.
        let yaml = r#"
proxy:
  specs:
    - id: nb
      display-name: Notebook
      container-image: quay.io/jupyter/minimal-notebook:latest
      container-port: 8888
      platform: linux/amd64
      container-env:
        JUPYTER_TOKEN: ""
        GRANT_SUDO: "yes"
      container-cmd:
        - start-notebook.py
        - --ServerApp.base_url=/
      access-groups: [staff, ops]
      access-users: [alice]
      placement: bin-pack
      anti-affinity: true
      routing-strategy: round-robin
      scale-up-threshold: 0.8
      scale-down-threshold: 0.2
      scale-down-grace: 45
      drain-timeout: 20
      template-properties:
        type: app
        state: active
"#;
        let cfg = Config::from_yaml(yaml).expect("parse fixture");
        let original = &cfg.proxy.specs[0];
        let merged = SpecForm::from_spec(original)
            .into_spec(Some(original), Role::Admin)
            .expect("into_spec");

        assert_eq!(merged.container_port, Some(8888));
        assert_eq!(merged.platform.as_deref(), Some("linux/amd64"));
        assert_eq!(merged.env_pairs(), vec!["GRANT_SUDO=yes", "JUPYTER_TOKEN="]);
        assert_eq!(
            merged.container_cmd.as_deref(),
            Some(&["start-notebook.py".into(), "--ServerApp.base_url=/".into()][..])
        );
        assert_eq!(merged.access_groups.as_deref(), Some(&["staff".into(), "ops".into()][..]));
        assert_eq!(merged.access_users.as_deref(), Some(&["alice".into()][..]));
        assert_eq!(merged.placement, Some(Placement::BinPack));
        assert_eq!(merged.anti_affinity, Some(true));
        assert_eq!(merged.routing_strategy, Some(RoutingStrategy::RoundRobin));
        assert_eq!(merged.scale_up_threshold.map(|f| f.0), Some(0.8));
        assert_eq!(merged.scale_down_grace, Some(45));
        assert_eq!(merged.drain_timeout, Some(20));
    }

    #[test]
    fn clearing_an_advanced_field_clears_it() {
        // The form is authoritative: blanking a field that the base had
        // set drops it to None (= inherit the default), not preserve.
        let yaml = "proxy:\n  specs:\n    - id: a\n      container-image: x\n      platform: linux/amd64\n";
        let cfg = Config::from_yaml(yaml).unwrap();
        let base = &cfg.proxy.specs[0];
        let mut form = SpecForm::from_spec(base);
        assert_eq!(form.platform, "linux/amd64");
        form.platform = "  ".into(); // operator clears it
        let merged = form.into_spec(Some(base), Role::Admin).unwrap();
        assert_eq!(merged.platform, None);
    }

    #[test]
    fn parse_env_splits_on_first_equals_and_sorts() {
        let m = parse_env("FOO=bar\n  BAZ = qux \n\nDSN=postgres://u:p@h/db?x=1\nnoequals\n=noval")
            .expect("some");
        assert_eq!(m.get("FOO").map(String::as_str), Some("bar"));
        assert_eq!(m.get("BAZ").map(String::as_str), Some("qux"));
        // value may itself contain '=' (only the first splits)
        assert_eq!(m.get("DSN").map(String::as_str), Some("postgres://u:p@h/db?x=1"));
        // lines without a key, or with an empty key, are skipped
        assert!(!m.contains_key("noequals"));
        assert_eq!(m.len(), 3);
        assert!(parse_env("   \n\n").is_none());
    }

    #[test]
    fn list_to_vec_accepts_commas_and_newlines() {
        assert_eq!(
            list_to_vec("staff, ops\nresearch").as_deref(),
            Some(&["staff".into(), "ops".into(), "research".into()][..])
        );
        assert!(list_to_vec("  ,  \n").is_none());
    }

    #[test]
    fn validate_flags_bad_port_and_threshold() {
        let mut f = valid_form();
        f.container_port = "70000".into();
        assert!(f.validate(FormMode::New).contains(&"spec-form-error-port"));

        let mut f = valid_form();
        f.scale_up_threshold = "1.5".into();
        assert!(f.validate(FormMode::New).contains(&"spec-form-error-threshold"));

        let mut f = valid_form();
        f.container_port = "8501".into();
        f.scale_up_threshold = "0.8".into();
        f.max_body_size = "10m".into();
        f.container_cpu_request = "0.25".into();
        assert!(f.validate(FormMode::New).is_empty());
    }

    // #720 P4: an invalid container-env line is a config error, not a
    // silent drop. A missing `=` or a bad key blocks the save.
    #[test]
    fn validate_rejects_bad_container_env() {
        let mut f = valid_form();
        f.container_env = "JUST_A_KEY".into(); // no `=`
        assert!(f.validate(FormMode::New).contains(&"spec-form-error-env"));

        let mut f = valid_form();
        f.container_env = "BAD KEY=value".into(); // space in key
        assert!(f.validate(FormMode::New).contains(&"spec-form-error-env"));

        let mut f = valid_form();
        f.container_env = "1ABC=value".into(); // key starts with a digit
        assert!(f.validate(FormMode::New).contains(&"spec-form-error-env"));
    }

    #[test]
    fn validate_accepts_good_container_env() {
        let mut f = valid_form();
        // blank lines ok; values may contain `=` and `${VAR}`.
        f.container_env = "\nDATABASE_URL=postgres://x\n_LOG=info\nTOKEN=${API_TOKEN}\n".into();
        assert!(!f.validate(FormMode::New).contains(&"spec-form-error-env"));
    }
}
