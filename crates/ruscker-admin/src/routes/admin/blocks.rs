//! Admin > Custom landing blocks (#54).
//!
//! CRUD for the operator-authored HTML blocks rendered in the
//! landing `top` / `bottom` slots. DB-only; gated by `AdminSession`
//! and a wired `--db`. Reordering within a slot is a follow-up
//! slice — new blocks append to the end of their slot.

use askama::Template;
use axum::{
    extract::{Form, Json, Path, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
    routing::{get, post},
    Router,
};
use serde::Deserialize;

use crate::auth::{RequireAdmin, Role};
use crate::db::landing_blocks::{self, BlockInput, LandingBlock, SLOTS};
use crate::i18n::{Locale, Locales};
use crate::theme::Theme;
use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/admin/blocks", get(index).post(create))
        .route("/admin/blocks/new", get(new_form))
        .route("/admin/blocks/{id}", get(edit_form).post(update))
        .route("/admin/blocks/{id}/delete", post(delete))
        .route("/admin/blocks/{id}/move/{dir}", post(move_block))
        .route("/admin/blocks/reorder", post(reorder))
}

// ── List ─────────────────────────────────────────────────────────

/// One slot's blocks, in render (position) order. The blocks editor
/// is folded into the Portal page (#242), so the `LandingPage` reuses
/// this type + [`grouped_blocks`] to render the same per-slot lists.
pub(crate) struct SlotGroup {
    pub(crate) slot: &'static str,
    pub(crate) blocks: Vec<LandingBlock>,
}

/// Split a flat, `(slot, position)`-ordered block list into one group
/// per known slot (in `SLOTS` order) for the per-slot reorder lists.
pub(crate) fn grouped_blocks(blocks: &[LandingBlock]) -> Vec<SlotGroup> {
    SLOTS
        .iter()
        .map(|&slot| SlotGroup {
            slot,
            blocks: blocks.iter().filter(|b| b.slot == slot).cloned().collect(),
        })
        .collect()
}

/// The standalone blocks list page was folded into the Portal page
/// (#242); the route stays so old links/bookmarks still resolve.
async fn index(_: RequireAdmin) -> Response {
    Redirect::to("/admin/landing").into_response()
}

// ── Form (new / edit) ────────────────────────────────────────────

#[derive(Template)]
#[template(path = "admin/block_form.html")]
struct BlockFormPage<'a> {
    locale: Locale,
    theme: Theme,
    locales: &'a Locales,
    locales_all: &'static [Locale],
    /// Mount prefix for base-path-correct URLs (#294).
    base: std::sync::Arc<str>,
    nav_section: &'static str,
    /// Current session role - drives nav gating.
    role: Role,
    /// "new" or "edit" — drives the form action + heading.
    mode: &'static str,
    /// Empty for a new block.
    id: String,
    slot: String,
    title: String,
    html: String,
    csp_origins: String,
    enabled: bool,
    slots: &'static [&'static str],
}

impl BlockFormPage<'_> {
    fn t(&self, key: &str) -> String {
        self.locales.t(self.locale, key, None)
    }
}

async fn new_form(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
) -> Response {
    if state.db.as_ref().is_none() {
        return no_db();
    }
    super::render(&BlockFormPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "blocks",
        role: Role::Admin,
        mode: "new",
        id: String::new(),
        slot: "top".into(),
        title: String::new(),
        html: String::new(),
        csp_origins: String::new(),
        enabled: true,
        slots: SLOTS,
    })
}

async fn edit_form(
    _: RequireAdmin,
    State(state): State<AppState>,
    loc: Locale,
    theme: Theme,
    Path(id): Path<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return no_db();
    };
    let block = match landing_blocks::fetch_one(pool, &id).await {
        Ok(Some(b)) => b,
        Ok(None) => return (StatusCode::NOT_FOUND, "block not found").into_response(),
        Err(e) => {
            tracing::error!(error = ?e, id, "fetch block failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response();
        }
    };
    super::render(&BlockFormPage {
        locale: loc,
        theme,
        locales: &state.locales,
        locales_all: &Locale::ALL,
        base: state.base_path.clone(),
        nav_section: "blocks",
        role: Role::Admin,
        mode: "edit",
        id: block.id,
        slot: block.slot,
        title: block.title,
        html: block.html,
        csp_origins: block.csp_origins,
        enabled: block.enabled,
        slots: SLOTS,
    })
}

// ── Mutations ────────────────────────────────────────────────────

#[derive(Debug, Default, Deserialize)]
#[serde(default)]
struct BlockForm {
    slot: String,
    title: String,
    html: String,
    csp_origins: String,
    /// HTML checkboxes submit a value when checked, nothing when not.
    enabled: Option<String>,
}

impl BlockForm {
    fn into_input(self) -> BlockInput {
        // Guard the slot to a known value so a tampered form can't
        // create an unrenderable block.
        let slot = if SLOTS.contains(&self.slot.as_str()) {
            self.slot
        } else {
            "top".to_string()
        };
        BlockInput {
            slot,
            enabled: self.enabled.is_some(),
            title: self.title.trim().to_string(),
            html: self.html,
            csp_origins: self.csp_origins.trim().to_string(),
        }
    }
}

async fn create(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Form(form): Form<BlockForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return no_db();
    };
    match landing_blocks::insert(pool, &form.into_input(), Some(admin.actor())).await {
        Ok(_) => Redirect::to("/admin/landing").into_response(),
        Err(e) => {
            tracing::error!(error = ?e, "create block failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

async fn update(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Path(id): Path<String>,
    Form(form): Form<BlockForm>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return no_db();
    };
    match landing_blocks::update(pool, &id, &form.into_input(), Some(admin.actor())).await {
        Ok(true) => Redirect::to("/admin/landing").into_response(),
        Ok(false) => (StatusCode::NOT_FOUND, "block not found").into_response(),
        Err(e) => {
            tracing::error!(error = ?e, id, "update block failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

async fn delete(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return no_db();
    };
    match landing_blocks::delete(pool, &id, Some(admin.actor())).await {
        Ok(_) => Redirect::to("/admin/landing").into_response(),
        Err(e) => {
            tracing::error!(error = ?e, id, "delete block failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

async fn move_block(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Path((id, dir)): Path<(String, String)>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return no_db();
    };
    let up = match dir.as_str() {
        "up" => true,
        "down" => false,
        _ => return (StatusCode::BAD_REQUEST, "dir must be up|down").into_response(),
    };
    match landing_blocks::move_block(pool, &id, up, Some(admin.actor())).await {
        Ok(_) => Redirect::to("/admin/landing").into_response(),
        Err(e) => {
            tracing::error!(error = ?e, id, "move block failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

/// New within-slot order, posted by the drag-and-drop handler as JSON.
#[derive(Debug, Deserialize)]
struct ReorderReq {
    slot: String,
    ids: Vec<String>,
}

/// `POST /admin/blocks/reorder` — persist a drag-reordered slot. Same
/// origin + the `SameSite=Strict` admin cookie cover CSRF. Returns
/// `204` so the client keeps the DOM order it already rendered.
async fn reorder(
    admin: RequireAdmin,
    State(state): State<AppState>,
    Json(req): Json<ReorderReq>,
) -> Response {
    let Some(pool) = state.db.as_ref() else {
        return no_db();
    };
    match landing_blocks::reorder(pool, &req.slot, &req.ids, Some(admin.actor())).await {
        Ok(true) => StatusCode::NO_CONTENT.into_response(),
        Ok(false) => (StatusCode::BAD_REQUEST, "unknown slot").into_response(),
        Err(e) => {
            tracing::error!(error = ?e, slot = req.slot, "reorder blocks failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "db error").into_response()
        }
    }
}

fn no_db() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        "custom blocks need a database — start with --db",
    )
        .into_response()
}
