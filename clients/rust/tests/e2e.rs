//! End-to-end tests against a running test provider.
//!
//! Start one with `clients/testing/provider.sh`, `source` the env file it
//! writes, then `cargo test --test e2e`. Without `RUSCKER_ISSUER` every
//! test here returns early (and says so), so `cargo test` passes anywhere.

use std::path::PathBuf;
use std::process::Command;

use ruscker_sso::{Client, Config, Error, Pending};
use url::Url;

/// The provider's settings from the environment, or `None` (skip).
fn config() -> Option<Config> {
    let Ok(issuer) = std::env::var("RUSCKER_ISSUER") else {
        eprintln!("RUSCKER_ISSUER is not set: skipping the end-to-end test");
        return None;
    };
    let var = |name: &str| std::env::var(name).unwrap_or_else(|_| panic!("{name} must be set with RUSCKER_ISSUER"));
    Some(Config::new(issuer, var("RUSCKER_CLIENT_ID"), var("RUSCKER_CLIENT_SECRET"), var("RUSCKER_REDIRECT_URI")))
}

/// Play the signed-in person's browser: follow the authorization URL and
/// return the callback URL the provider redirects to.
fn authorize(url: &Url) -> String {
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../testing/authorize.sh");
    let out = Command::new(script).arg(url.as_str()).output().expect("run authorize.sh");
    assert!(out.status.success(), "authorize.sh failed: {}", String::from_utf8_lossy(&out.stderr));
    let callback = String::from_utf8(out.stdout).unwrap().trim().to_string();
    assert!(callback.contains("code="), "no code in the redirect: {callback:?}");
    callback
}

/// A sign-in up to the callback: the client, the stored values and the
/// callback URL.
async fn begin() -> Option<(Client, Pending, String)> {
    let client = Client::discover(config()?).await.expect("discovery");
    let (url, pending) = client.start_sign_in();
    let callback = authorize(&url);
    Some((client, pending, callback))
}

fn param(callback: &str, name: &str) -> String {
    let url = Url::parse(callback).unwrap();
    url.query_pairs().find(|(k, _)| k == name).map(|(_, v)| v.into_owned()).unwrap()
}

/// Read a private field of `Pending` through its serialized form, the
/// same way an application stores it.
fn pending_field(p: &Pending, name: &str) -> String {
    serde_json::to_value(p).unwrap()[name].as_str().unwrap().to_string()
}

fn with_field(p: &Pending, name: &str, value: &str) -> Pending {
    let mut v = serde_json::to_value(p).unwrap();
    v[name] = value.into();
    serde_json::from_value(v).unwrap()
}

#[tokio::test]
async fn full_sign_in_returns_the_identity() {
    let Some((client, pending, callback)) = begin().await else { return };
    let id = client.finish_sign_in(&callback, &pending).await.expect("sign-in");
    assert!(!id.sub.is_empty());
    assert_eq!(id.username.as_deref(), Some("ana"));
    assert_eq!(id.name.as_deref(), Some("Ana Maria de Souza"));
    assert_eq!(id.email.as_deref(), Some("ana@example.org"));
    assert_eq!(id.setor.as_deref(), Some("GAPE"));
    assert_eq!(id.groups, ["fiscal"]);
    assert_eq!(id.roles("fiscal"), ["analista", "gestor"]);
    assert!(id.has_role("fiscal", "gestor"));
    assert_eq!(id.group_roles.len(), 1);
    assert_eq!(id.claims["aud"], client.config().client_id.as_str());
}

#[tokio::test]
async fn tampered_state_is_refused_before_any_network_call() {
    let Some((client, pending, callback)) = begin().await else { return };
    let state = param(&callback, "state");
    let forged = callback.replace(&format!("state={state}"), "state=forged");
    let err = client.finish_sign_in(&forged, &pending).await.unwrap_err();
    assert!(matches!(err, Error::StateMismatch), "{err:?}");
    // The code was not presented to the provider: it still works.
    client.finish_sign_in(&callback, &pending).await.expect("the untouched code still redeems");
}

#[tokio::test]
async fn replayed_code_is_refused() {
    let Some((client, pending, callback)) = begin().await else { return };
    client.finish_sign_in(&callback, &pending).await.expect("first use");
    match client.finish_sign_in(&callback, &pending).await {
        Err(Error::TokenRefused { error, .. }) => assert_eq!(error, "invalid_grant"),
        other => panic!("expected invalid_grant, got {other:?}"),
    }
}

#[tokio::test]
async fn wrong_nonce_is_refused() {
    let Some((client, pending, callback)) = begin().await else { return };
    let tampered = with_field(&pending, "nonce", "not-the-nonce-that-was-sent");
    match client.finish_sign_in(&callback, &tampered).await {
        Err(Error::InvalidIdToken(reason)) => assert!(reason.contains("nonce"), "{reason}"),
        other => panic!("expected a nonce failure, got {other:?}"),
    }
}

#[tokio::test]
async fn wrong_client_secret_is_refused() {
    let Some(mut cfg) = config() else { return };
    cfg.client_secret = "wrong-secret".into();
    let wrong = Client::discover(cfg).await.expect("discovery");
    let (url, pending) = wrong.start_sign_in();
    let callback = authorize(&url);
    match wrong.finish_sign_in(&callback, &pending).await {
        Err(Error::TokenRefused { status, error, .. }) => {
            assert_eq!(status, 401);
            assert_eq!(error, "invalid_client");
        }
        other => panic!("expected invalid_client, got {other:?}"),
    }
    // A failed client authentication does not spend the code.
    let right = Client::discover(config().unwrap()).await.unwrap();
    right.finish_sign_in(&callback, &pending).await.expect("the code is still good");
}

#[tokio::test]
async fn token_for_another_audience_is_refused() {
    let Some((client, pending, callback)) = begin().await else { return };
    let tokens = client
        .exchange_code(&param(&callback, "code"), &pending_field(&pending, "code_verifier"))
        .await
        .expect("exchange");
    let nonce = pending_field(&pending, "nonce");

    // The same, genuine token presented to a system with another client id.
    let mut other_cfg = config().unwrap();
    other_cfg.client_id = "another-system".into();
    let other = Client::discover(other_cfg).await.unwrap();
    match other.verify_id_token(&tokens.id_token, &nonce).await {
        Err(Error::InvalidIdToken(reason)) => assert!(reason.to_lowercase().contains("aud"), "{reason}"),
        other => panic!("expected an audience failure, got {other:?}"),
    }
    // …while the system it was issued to accepts it.
    client.verify_id_token(&tokens.id_token, &nonce).await.expect("right audience");
}
