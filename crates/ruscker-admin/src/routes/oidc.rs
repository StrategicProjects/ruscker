//! OIDC provider — the public metadata endpoints (slice 2 of
//! `docs/adr/0005-oidc-provider.md`).
//!
//! * `GET /.well-known/openid-configuration` — where the endpoints are
//!   and what the provider supports.
//! * `GET /oauth/jwks` — the public keys that verify its tokens.
//!
//! Both are public by definition and carry no secret. Both answer **404
//! unless the provider is enabled with a usable issuer**
//! (`server.oidc-provider`), so a deployment that never opted in exposes
//! nothing new.
//!
//! Paths are relative to the portal's mount point, so the configured
//! issuer must be the portal's public base URL *including* any base
//! path — OIDC requires the discovery document to live at
//! `{issuer}/.well-known/openid-configuration`.

use axum::{
    extract::State,
    http::{header::CACHE_CONTROL, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};

use crate::AppState;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/.well-known/openid-configuration", get(discovery))
        .route("/oauth/jwks", get(jwks))
}

/// Clients cache these documents; five minutes keeps a key rotation
/// visible quickly without a request per login.
const CACHE: &str = "public, max-age=300";

fn json(body: serde_json::Value) -> Response {
    ([(CACHE_CONTROL, CACHE)], Json(body)).into_response()
}

async fn discovery(State(state): State<AppState>) -> Response {
    match state.config.server.oidc_issuer() {
        Some(issuer) => json(crate::oidc::discovery(&issuer)),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn jwks(State(state): State<AppState>) -> Response {
    if state.config.server.oidc_issuer().is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let Some(db) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    // Make sure a signing key exists before publishing the set: a client
    // that reads the JWKS first must never find it empty and conclude
    // the provider has no keys.
    if let Err(err) = crate::oidc::ensure_signing_key(db, &state.master_key).await {
        tracing::error!(error = ?err, "OIDC signing key unavailable");
        return (StatusCode::SERVICE_UNAVAILABLE, "signing key unavailable").into_response();
    }
    match crate::oidc::jwks(db).await {
        Ok(set) => json(set),
        Err(err) => {
            tracing::error!(error = ?err, "build JWKS failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "jwks error").into_response()
        }
    }
}
