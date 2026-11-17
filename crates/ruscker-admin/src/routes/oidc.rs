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
//! Paths are relative to the portal's mount point, and OIDC requires
//! the discovery document to live at
//! `{issuer}/.well-known/openid-configuration` — so the configured
//! issuer must be the portal's public base URL *including* any base
//! path. That is enforced: an issuer whose path is not the mount point
//! leaves the provider off (`oidc::active_issuer`).

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

/// The issuer when the provider can serve, or the refusal to answer
/// with (status + body) when it cannot.
///
/// * off, or issuer/base-path mismatch ⇒ 404 (the endpoints do not
///   exist);
/// * enabled but no master key ⇒ 503 on BOTH documents. Discovery
///   needs no key itself, but advertising a `jwks_uri` that can only
///   ever answer 503 is a provider that half-works.
fn provider(state: &AppState) -> Result<String, (StatusCode, &'static str)> {
    let Some(issuer) = crate::oidc::active_issuer(&state.config, &state.base_path) else {
        return Err((StatusCode::NOT_FOUND, ""));
    };
    if !state.master_key.is_configured() {
        return Err((StatusCode::SERVICE_UNAVAILABLE, "provider not configured"));
    }
    Ok(issuer)
}

async fn discovery(State(state): State<AppState>) -> Response {
    match provider(&state) {
        Ok(issuer) => json(crate::oidc::discovery(&issuer)),
        Err(refusal) => refusal.into_response(),
    }
}

async fn jwks(State(state): State<AppState>) -> Response {
    if let Err(refusal) = provider(&state) {
        return refusal.into_response();
    }
    let Some(db) = state.db.as_ref() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "no db").into_response();
    };
    // The common path reads the public keys and decrypts nothing. Only
    // when there is no key at all do we create the first one — so a
    // client that reads the JWKS before anything else never finds it
    // empty. Once a key exists this endpoint cannot be made to write.
    match crate::oidc::published_jwks(db).await {
        Ok(Some(set)) => return json(set),
        Ok(None) => {}
        Err(err) => {
            tracing::error!(error = ?err, "build JWKS failed");
            return (StatusCode::INTERNAL_SERVER_ERROR, "jwks error").into_response();
        }
    }
    if let Err(err) = crate::oidc::ensure_signing_key(db, &state.master_key).await {
        tracing::error!(error = ?err, "could not create the OIDC signing key");
        return (StatusCode::SERVICE_UNAVAILABLE, "signing key unavailable").into_response();
    }
    match crate::oidc::published_jwks(db).await {
        Ok(Some(set)) => json(set),
        Ok(None) => (StatusCode::SERVICE_UNAVAILABLE, "signing key unavailable").into_response(),
        Err(err) => {
            tracing::error!(error = ?err, "build JWKS failed");
            (StatusCode::INTERNAL_SERVER_ERROR, "jwks error").into_response()
        }
    }
}
