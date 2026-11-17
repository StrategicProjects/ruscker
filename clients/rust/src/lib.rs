//! Sign people in to a Rust backend through a Ruscker portal.
//!
//! The portal is an OpenID Connect provider with a narrow profile:
//! authorization code flow, PKCE `S256`, a confidential client, ID tokens
//! signed with ES256. This crate is pre-set for exactly that profile so an
//! integration is three calls:
//!
//! 1. [`Client::discover`] once at startup;
//! 2. [`Client::start_sign_in`] in the "sign in" route — redirect the
//!    browser to the URL and keep the [`Pending`] value until it returns;
//! 3. [`Client::finish_sign_in`] in the callback route — it returns the
//!    verified [`Identity`], from which the application issues its own
//!    session.
//!
//! No cryptography is implemented here: signatures are checked by
//! `jsonwebtoken` against the provider's published keys, randomness comes
//! from the operating system (`getrandom`), and the PKCE hash from `sha2`.
//!
//! What the crate deliberately does not do: sessions (the portal's tokens
//! live five minutes and there is no refresh), sign-out at the portal, and
//! deciding what groups and roles mean inside the application.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use base64::Engine as _;
use jsonwebtoken::jwk::JwkSet;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use url::Url;

/// The scopes asked for when [`Config::scopes`] is not called: every claim
/// the provider offers.
pub const DEFAULT_SCOPES: &str = "openid profile email groups roles setor";

/// How long any single request to the provider may take. The provider
/// answers in milliseconds; a hung connection should fail the sign-in
/// rather than hold the callback request forever.
const HTTP_TIMEOUT: Duration = Duration::from_secs(10);

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// What the portal's administrator gives the system's developers.
#[derive(Clone)]
pub struct Config {
    /// The portal's address, exactly as its discovery document states it
    /// (e.g. `https://portal.example.org`, including a subpath if any).
    pub issuer: String,
    /// The system's id, as registered on the portal.
    pub client_id: String,
    /// The secret shown once at registration. Server side only.
    pub client_secret: String,
    /// The system's callback URL, exactly as registered.
    pub redirect_uri: String,
    /// Space-separated scopes; must contain `openid`.
    pub scopes: String,
}

impl Config {
    /// A configuration with the default scopes ([`DEFAULT_SCOPES`]).
    pub fn new(
        issuer: impl Into<String>,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
        redirect_uri: impl Into<String>,
    ) -> Self {
        Config {
            issuer: issuer.into(),
            client_id: client_id.into(),
            client_secret: client_secret.into(),
            redirect_uri: redirect_uri.into(),
            scopes: DEFAULT_SCOPES.to_string(),
        }
    }

    /// Ask for other scopes than the default (space-separated).
    pub fn scopes(mut self, scopes: impl Into<String>) -> Self {
        self.scopes = scopes.into();
        self
    }
}

// Hand-written so the secret never ends up in a log line via `{:?}`.
impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("issuer", &self.issuer)
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("redirect_uri", &self.redirect_uri)
            .field("scopes", &self.scopes)
            .finish()
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Why a sign-in (or the discovery) failed.
///
/// The variants separate the cases an application may want to treat
/// differently: [`Error::Provider`] is usually the person's situation
/// (e.g. `access_denied` — not in the system's groups) and deserves a
/// friendly page; the others mean a stale or forged callback, a
/// misconfiguration, or an attack, and are best logged and answered with
/// "please sign in again".
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// The provider sent the person back with `error=…`.
    #[error("the provider refused the sign-in: {error}")]
    Provider {
        /// The OAuth error code, e.g. `access_denied`.
        error: String,
        /// The provider's human-readable explanation, if any.
        description: Option<String>,
    },
    /// The callback's `state` is missing or is not the one this browser
    /// was given: a stale tab, a second sign-in in another tab, or a
    /// forged callback. Nothing was sent to the provider.
    #[error("the callback's state does not match the stored one")]
    StateMismatch,
    /// An issuer that is not the configured one: in the callback's `iss`
    /// parameter, or in the discovery document.
    #[error("issuer mismatch: expected {expected:?}, got {found:?}")]
    IssuerMismatch {
        /// The configured issuer.
        expected: String,
        /// The issuer that was presented.
        found: String,
    },
    /// The callback query is malformed (no `code`, a repeated parameter).
    #[error("malformed callback: {0}")]
    InvalidCallback(&'static str),
    /// The token endpoint refused the code exchange.
    #[error("the token exchange was refused ({status}): {error}")]
    TokenRefused {
        /// The HTTP status of the answer.
        status: u16,
        /// The OAuth error code: `invalid_client` (wrong secret),
        /// `invalid_grant` (code used, expired or not ours), …
        error: String,
        /// The provider's explanation, if any.
        description: Option<String>,
    },
    /// The ID token failed verification; the string says which check.
    #[error("invalid ID token: {0}")]
    InvalidIdToken(String),
    /// The discovery document or the key set could not be used.
    #[error("discovery failed: {0}")]
    Discovery(String),
    /// The provider could not be reached, or answered something that is
    /// not the expected JSON.
    #[error("HTTP error talking to the provider: {0}")]
    Http(#[from] reqwest::Error),
}

// ---------------------------------------------------------------------------
// Values kept between the two halves of a sign-in
// ---------------------------------------------------------------------------

/// The three values to keep between [`Client::start_sign_in`] and
/// [`Client::finish_sign_in`], bound to the person's browser.
///
/// Serializable so it fits a server-side session or a short-lived signed
/// (or private) `HttpOnly` cookie. It is not secret in the way a password
/// is, but it must not be readable or writable by other sites — never put
/// it in the URL or in a plain, unsigned cookie.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Pending {
    state: String,
    nonce: String,
    code_verifier: String,
}

impl Pending {
    /// A fresh set of values from the OS random source.
    fn generate() -> Self {
        Pending { state: random_token(), nonce: random_token(), code_verifier: random_token() }
    }

    /// The `state` sent with the request (handy for correlating logs).
    pub fn state(&self) -> &str {
        &self.state
    }
}

// The verifier is the secret half of PKCE; keep it out of `{:?}` output.
impl std::fmt::Debug for Pending {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Pending").field("state", &self.state).finish_non_exhaustive()
    }
}

/// 32 bytes (256 bits) from the operating system's random source, as
/// base64url without padding: 43 characters of `A-Z a-z 0-9 - _`, which
/// is valid as a state, a nonce and a PKCE verifier alike.
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    // The OS random source failing is not something a sign-in can recover
    // from (the `rand` crate panics in the same situation); refusing to
    // continue beats continuing with predictable values.
    getrandom::fill(&mut bytes).expect("the operating system's random source is unavailable");
    URL_SAFE_NO_PAD.encode(bytes)
}

/// The PKCE `S256` challenge: base64url(SHA-256(verifier)), RFC 7636 §4.2.
fn pkce_challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// Compare two strings without leaking, through timing, how many leading
/// characters matched.
fn same(a: &str, b: &str) -> bool {
    a.as_bytes().ct_eq(b.as_bytes()).into()
}

// ---------------------------------------------------------------------------
// The identity
// ---------------------------------------------------------------------------

/// Who signed in, taken from the verified ID token.
///
/// Store [`Identity::sub`] as the key to the person: a username can be
/// reused after an account is deleted, and `email` is neither verified nor
/// unique.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Identity {
    /// The person's permanent id on the portal.
    pub sub: String,
    /// `preferred_username` (with the `profile` scope).
    pub username: Option<String>,
    /// Full name, or the username when the portal has none (with the
    /// `profile` scope).
    pub name: Option<String>,
    /// E-mail, when the account has one and `email` was asked for.
    pub email: Option<String>,
    /// Department, when the account has one and `setor` was asked for.
    pub setor: Option<String>,
    /// Groups. For a restricted system, only those that give access to it.
    pub groups: Vec<String>,
    /// Group → roles the person holds in it; empty when none.
    pub group_roles: BTreeMap<String, Vec<String>>,
    /// Every claim of the ID token, as received.
    pub claims: Map<String, Value>,
}

impl Identity {
    /// Build the identity from verified claims. Claims that are absent, or
    /// of an unexpected type, are left empty rather than failing the
    /// sign-in: only `sub` is indispensable.
    fn from_claims(claims: Map<String, Value>) -> Result<Self, Error> {
        let text = |key: &str| claims.get(key).and_then(Value::as_str).map(str::to_owned);
        let sub = text("sub")
            .filter(|s| !s.is_empty())
            .ok_or_else(|| Error::InvalidIdToken("no sub claim".into()))?;
        let strings = |v: &Value| -> Vec<String> {
            v.as_array()
                .map(|a| a.iter().filter_map(Value::as_str).map(str::to_owned).collect())
                .unwrap_or_default()
        };
        let groups = claims.get("groups").map(strings).unwrap_or_default();
        let group_roles = claims
            .get("group_roles")
            .and_then(Value::as_object)
            .map(|m| m.iter().map(|(g, r)| (g.clone(), strings(r))).collect())
            .unwrap_or_default();
        Ok(Identity {
            sub,
            username: text("preferred_username"),
            name: text("name"),
            email: text("email"),
            setor: text("setor"),
            groups,
            group_roles,
            claims,
        })
    }

    /// Whether the person is in `group`.
    pub fn has_group(&self, group: &str) -> bool {
        self.groups.iter().any(|g| g == group)
    }

    /// Whether the person holds `role` in `group`.
    pub fn has_role(&self, group: &str, role: &str) -> bool {
        self.roles(group).iter().any(|r| r == role)
    }

    /// The roles the person holds in `group` (empty when none).
    pub fn roles(&self, group: &str) -> &[String] {
        self.group_roles.get(group).map(Vec::as_slice).unwrap_or(&[])
    }
}

// ---------------------------------------------------------------------------
// The client
// ---------------------------------------------------------------------------

/// The part of the discovery document this crate uses.
#[derive(Debug, Clone, Deserialize)]
struct Metadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    jwks_uri: String,
}

/// The token endpoint's answer. Only the ID token is used; the access
/// token is valid five minutes and only at `/oauth/userinfo`.
#[derive(Clone, Deserialize)]
pub struct TokenResponse {
    /// The signed ID token (a JWT). Verify it with
    /// [`Client::verify_id_token`] before trusting anything in it.
    pub id_token: String,
    /// The access token, for `/oauth/userinfo`.
    pub access_token: Option<String>,
    /// The scopes granted.
    pub scope: Option<String>,
}

impl std::fmt::Debug for TokenResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TokenResponse").field("scope", &self.scope).finish_non_exhaustive()
    }
}

/// An OAuth error body, `{"error": …, "error_description": …}`.
#[derive(Deserialize)]
struct OAuthError {
    error: String,
    error_description: Option<String>,
}

/// A client for one registered system. Cheap to clone (it is an `Arc`
/// inside), so put it in the application's shared state.
#[derive(Clone, Debug)]
pub struct Client {
    inner: Arc<Inner>,
}

#[derive(Debug)]
struct Inner {
    config: Config,
    metadata: Metadata,
    http: reqwest::Client,
    /// The provider's public keys, refreshed once when a token names a key
    /// id we have not seen (the provider can rotate keys). A std lock is
    /// enough: it is never held across an `.await`.
    jwks: RwLock<Arc<JwkSet>>,
}

impl Client {
    /// Read the provider's discovery document and key set, once.
    ///
    /// Fails if the document's `issuer` is not exactly the configured one
    /// — the same string is compared in every token, so a mismatch here
    /// would only fail later, on every sign-in.
    pub async fn discover(config: Config) -> Result<Self, Error> {
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            // The token endpoint never redirects; following a redirect
            // would send the code (and the secret) somewhere else.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        Self::discover_with(config, http).await
    }

    /// [`Client::discover`] with an HTTP client of your own (proxy, custom
    /// roots, timeouts). Disable redirects on it.
    pub async fn discover_with(config: Config, http: reqwest::Client) -> Result<Self, Error> {
        let url = format!("{}/.well-known/openid-configuration", config.issuer.trim_end_matches('/'));
        let metadata: Metadata = http.get(&url).send().await?.error_for_status()?.json().await?;
        check_metadata(&config, &metadata)?;
        let jwks = fetch_jwks(&http, &metadata.jwks_uri).await?;
        Ok(Self::from_parts(config, metadata, http, jwks))
    }

    fn from_parts(config: Config, metadata: Metadata, http: reqwest::Client, jwks: JwkSet) -> Self {
        Client { inner: Arc::new(Inner { config, metadata, http, jwks: RwLock::new(Arc::new(jwks)) }) }
    }

    /// The configuration this client was built with.
    pub fn config(&self) -> &Config {
        &self.inner.config
    }

    /// Begin a sign-in: the URL to redirect the browser to, and the values
    /// to keep (bound to that browser) until it comes back.
    ///
    /// Panics only if the operating system's random source is unavailable.
    pub fn start_sign_in(&self) -> (Url, Pending) {
        let pending = Pending::generate();
        let config = &self.inner.config;
        let mut url = Url::parse(&self.inner.metadata.authorization_endpoint)
            .expect("authorization_endpoint was validated at discovery");
        url.query_pairs_mut()
            .append_pair("response_type", "code")
            .append_pair("client_id", &config.client_id)
            .append_pair("redirect_uri", &config.redirect_uri)
            .append_pair("scope", &config.scopes)
            .append_pair("state", &pending.state)
            .append_pair("nonce", &pending.nonce)
            .append_pair("code_challenge", &pkce_challenge(&pending.code_verifier))
            .append_pair("code_challenge_method", "S256");
        (url, pending)
    }

    /// Finish a sign-in from the callback's query string (with or without
    /// the leading `?`; a full URL is accepted too) and the stored
    /// [`Pending`] values.
    ///
    /// The checks run in this order, and the first failure stops it:
    /// an `error` parameter; `state`; `iss`; the code exchange; the ID
    /// token (signature, ES256, `iss`, `aud`, `exp`, `nonce`). Nothing
    /// reaches the network before `state` and `iss` have passed. Whatever
    /// the outcome, the [`Pending`] value must not be used again: drop it
    /// from the session — a code is single-use and is never retried.
    pub async fn finish_sign_in(&self, callback_query: &str, pending: &Pending) -> Result<Identity, Error> {
        let code = self.check_callback(callback_query, pending)?;
        let tokens = self.exchange_code(&code, &pending.code_verifier).await?;
        self.verify_id_token(&tokens.id_token, &pending.nonce).await
    }

    /// Steps 1–3 of [`Client::finish_sign_in`], with no network: returns
    /// the code when the callback is genuine.
    fn check_callback(&self, callback_query: &str, pending: &Pending) -> Result<String, Error> {
        let params = parse_query(callback_query)?;
        if let Some(error) = params.get("error") {
            return Err(Error::Provider {
                error: error.clone(),
                description: params.get("error_description").cloned(),
            });
        }
        match params.get("state") {
            Some(state) if same(state, &pending.state) => {}
            _ => return Err(Error::StateMismatch),
        }
        let issuer = &self.inner.config.issuer;
        if let Some(iss) = params.get("iss") {
            if iss != issuer {
                return Err(Error::IssuerMismatch { expected: issuer.clone(), found: iss.clone() });
            }
        }
        params.get("code").filter(|c| !c.is_empty()).cloned().ok_or(Error::InvalidCallback("no code"))
    }

    /// Exchange an authorization code at the token endpoint, authenticating
    /// with the client secret over HTTP Basic. One attempt only: the
    /// provider spends the code whatever the outcome.
    ///
    /// Most applications call [`Client::finish_sign_in`] instead; this is
    /// public for tests and for code that needs the raw tokens.
    pub async fn exchange_code(&self, code: &str, code_verifier: &str) -> Result<TokenResponse, Error> {
        let config = &self.inner.config;
        let response = self
            .inner
            .http
            .post(&self.inner.metadata.token_endpoint)
            .header(reqwest::header::AUTHORIZATION, basic_auth(&config.client_id, &config.client_secret))
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", code),
                ("redirect_uri", config.redirect_uri.as_str()),
                ("code_verifier", code_verifier),
            ])
            .send()
            .await?;
        let status = response.status();
        if status.is_success() {
            return Ok(response.json().await?);
        }
        // An error answer is JSON `{error, error_description}`, except the
        // 413 for an oversized body, which has none.
        let body = response.bytes().await.unwrap_or_default();
        let parsed: Option<OAuthError> = serde_json::from_slice(&body).ok();
        Err(Error::TokenRefused {
            status: status.as_u16(),
            error: parsed.as_ref().map_or_else(|| format!("http_{}", status.as_u16()), |e| e.error.clone()),
            description: parsed.and_then(|e| e.error_description),
        })
    }

    /// Verify an ID token and turn it into an [`Identity`]: signature
    /// against the provider's keys, algorithm ES256 only (the token's own
    /// `alg` header never chooses it), `iss` = the issuer, `aud` = this
    /// client's id, `exp`, and `nonce` = the one sent.
    pub async fn verify_id_token(&self, id_token: &str, nonce: &str) -> Result<Identity, Error> {
        let bad = |reason: String| Error::InvalidIdToken(reason);
        let header = jsonwebtoken::decode_header(id_token).map_err(|e| bad(format!("unreadable header: {e}")))?;
        let kid = header.kid.ok_or_else(|| bad("the token names no key id (kid)".into()))?;
        let key = self.decoding_key(&kid).await?;

        let config = &self.inner.config;
        let mut validation = Validation::new(Algorithm::ES256);
        validation.set_issuer(&[config.issuer.as_str()]);
        validation.set_audience(&[config.client_id.as_str()]);
        validation.set_required_spec_claims(&["exp", "iss", "aud", "sub"]);
        let data = jsonwebtoken::decode::<Map<String, Value>>(id_token, &key, &validation)
            .map_err(|e| bad(e.to_string()))?;
        let claims = data.claims;

        match claims.get("nonce").and_then(Value::as_str) {
            Some(n) if same(n, nonce) => {}
            _ => return Err(bad("nonce mismatch".into())),
        }
        Identity::from_claims(claims)
    }

    /// The key named `kid`, refreshing the cached key set once if it is
    /// not there (the provider may have rotated its key).
    async fn decoding_key(&self, kid: &str) -> Result<DecodingKey, Error> {
        let cached = self.inner.jwks.read().expect("jwks lock poisoned").clone();
        let jwk = match cached.find(kid) {
            Some(jwk) => jwk.clone(),
            None => {
                let fresh = Arc::new(fetch_jwks(&self.inner.http, &self.inner.metadata.jwks_uri).await?);
                *self.inner.jwks.write().expect("jwks lock poisoned") = fresh.clone();
                fresh
                    .find(kid)
                    .cloned()
                    .ok_or_else(|| Error::InvalidIdToken(format!("signed with an unknown key {kid:?}")))?
            }
        };
        DecodingKey::from_jwk(&jwk).map_err(|e| Error::InvalidIdToken(format!("unusable key {kid:?}: {e}")))
    }
}

/// The discovery document must name the configured issuer exactly, and
/// its endpoints must be URLs.
fn check_metadata(config: &Config, metadata: &Metadata) -> Result<(), Error> {
    if metadata.issuer != config.issuer {
        return Err(Error::IssuerMismatch { expected: config.issuer.clone(), found: metadata.issuer.clone() });
    }
    for (name, value) in [
        ("authorization_endpoint", &metadata.authorization_endpoint),
        ("token_endpoint", &metadata.token_endpoint),
        ("jwks_uri", &metadata.jwks_uri),
    ] {
        Url::parse(value).map_err(|e| Error::Discovery(format!("{name} is not a URL: {e}")))?;
    }
    Ok(())
}

async fn fetch_jwks(http: &reqwest::Client, uri: &str) -> Result<JwkSet, Error> {
    let body = http.get(uri).send().await?.error_for_status()?.bytes().await?;
    serde_json::from_slice(&body).map_err(|e| Error::Discovery(format!("unreadable key set: {e}")))
}

/// `Authorization: Basic …` per RFC 6749 §2.3.1: each half is
/// form-urlencoded before joining with `:` (a secret may contain `:`).
fn basic_auth(id: &str, secret: &str) -> String {
    let enc = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    format!("Basic {}", STANDARD.encode(format!("{}:{}", enc(id), enc(secret))))
}

/// Parse the callback query into a map, refusing a repeated parameter (two
/// `state`s or two `code`s are not something a genuine redirect contains).
fn parse_query(input: &str) -> Result<HashMap<String, String>, Error> {
    let query = match input.split_once('?') {
        Some((_, q)) => q,
        None => input,
    };
    let query = query.split('#').next().unwrap_or("");
    let mut params = HashMap::new();
    for (k, v) in url::form_urlencoded::parse(query.as_bytes()) {
        if params.insert(k.into_owned(), v.into_owned()).is_some() {
            return Err(Error::InvalidCallback("a parameter is repeated"));
        }
    }
    Ok(params)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const ISSUER: &str = "https://portal.example.org";

    /// A client built without the network, pointing at endpoints nobody
    /// listens on — any test that reached the network would fail.
    fn offline_client() -> Client {
        let config = Config::new(ISSUER, "my-system", "s3cret", "https://app.example.org/auth/callback");
        let metadata = Metadata {
            issuer: ISSUER.into(),
            authorization_endpoint: format!("{ISSUER}/oauth/authorize"),
            token_endpoint: "http://127.0.0.1:9/oauth/token".into(),
            jwks_uri: "http://127.0.0.1:9/oauth/jwks".into(),
        };
        Client::from_parts(config, metadata, reqwest::Client::new(), JwkSet { keys: vec![] })
    }

    fn query(url: &Url) -> HashMap<String, String> {
        url.query_pairs().map(|(k, v)| (k.into_owned(), v.into_owned())).collect()
    }

    #[test]
    fn authorization_url_carries_the_profile() {
        let client = offline_client();
        let (url, pending) = client.start_sign_in();
        assert!(url.as_str().starts_with("https://portal.example.org/oauth/authorize?"));
        let q = query(&url);
        assert_eq!(q["response_type"], "code");
        assert_eq!(q["client_id"], "my-system");
        assert_eq!(q["redirect_uri"], "https://app.example.org/auth/callback");
        assert_eq!(q["scope"], DEFAULT_SCOPES);
        assert_eq!(q["code_challenge_method"], "S256");
        assert_eq!(q["state"], pending.state);
        assert_eq!(q["nonce"], pending.nonce);
        assert_eq!(q["code_challenge"], pkce_challenge(&pending.code_verifier));
        // The verifier itself never leaves the server.
        assert!(!url.as_str().contains(&pending.code_verifier));
    }

    #[test]
    fn custom_scopes_are_sent() {
        let mut client = offline_client();
        Arc::get_mut(&mut client.inner).unwrap().config.scopes = "openid profile".into();
        let (url, _) = client.start_sign_in();
        assert_eq!(query(&url)["scope"], "openid profile");
    }

    #[test]
    fn pending_values_are_random_and_well_formed() {
        let a = Pending::generate();
        let b = Pending::generate();
        for v in [&a.state, &a.nonce, &a.code_verifier] {
            // 32 random bytes → 43 base64url characters, a valid verifier.
            assert_eq!(v.len(), 43);
            assert!(v.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
        }
        assert_ne!(a.state, a.nonce);
        assert_ne!(a.nonce, a.code_verifier);
        assert_ne!(a, b);
    }

    #[test]
    fn pkce_challenge_matches_rfc_7636_example() {
        // RFC 7636, Appendix B.
        assert_eq!(
            pkce_challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
    }

    #[test]
    fn pending_round_trips_through_serde() {
        let p = Pending::generate();
        let json = serde_json::to_string(&p).unwrap();
        assert_eq!(serde_json::from_str::<Pending>(&json).unwrap(), p);
        // …and `{:?}` does not reveal the verifier.
        assert!(!format!("{p:?}").contains(&p.code_verifier));
    }

    #[test]
    fn error_callback_is_reported_with_its_code() {
        let client = offline_client();
        let (_, p) = client.start_sign_in();
        let q = format!("error=access_denied&error_description=not+in+group&state={}", p.state);
        match client.check_callback(&q, &p) {
            Err(Error::Provider { error, description }) => {
                assert_eq!(error, "access_denied");
                assert_eq!(description.as_deref(), Some("not in group"));
            }
            other => panic!("expected Provider error, got {other:?}"),
        }
    }

    #[test]
    fn wrong_or_missing_state_is_refused() {
        let client = offline_client();
        let (_, p) = client.start_sign_in();
        assert!(matches!(client.check_callback("code=abc&state=forged", &p), Err(Error::StateMismatch)));
        assert!(matches!(client.check_callback("code=abc", &p), Err(Error::StateMismatch)));
    }

    #[tokio::test]
    async fn state_mismatch_stops_before_the_network() {
        // The token endpoint is unreachable: an Http error would mean the
        // exchange was attempted.
        let client = offline_client();
        let (_, p) = client.start_sign_in();
        let err = client.finish_sign_in("?code=abc&state=forged", &p).await.unwrap_err();
        assert!(matches!(err, Error::StateMismatch), "{err:?}");
    }

    #[test]
    fn foreign_iss_is_refused_and_absent_iss_accepted() {
        let client = offline_client();
        let (_, p) = client.start_sign_in();
        let q = format!("code=abc&state={}&iss=https%3A%2F%2Fevil.example.org", p.state);
        assert!(matches!(client.check_callback(&q, &p), Err(Error::IssuerMismatch { .. })));
        let q = format!("code=abc&state={}&iss=https%3A%2F%2Fportal.example.org", p.state);
        assert_eq!(client.check_callback(&q, &p).unwrap(), "abc");
        let q = format!("code=abc&state={}", p.state);
        assert_eq!(client.check_callback(&q, &p).unwrap(), "abc");
    }

    #[test]
    fn full_callback_url_and_malformed_queries() {
        let client = offline_client();
        let (_, p) = client.start_sign_in();
        let url = format!("https://app.example.org/auth/callback?code=xyz&state={}", p.state);
        assert_eq!(client.check_callback(&url, &p).unwrap(), "xyz");
        let q = format!("state={}", p.state);
        assert!(matches!(client.check_callback(&q, &p), Err(Error::InvalidCallback(_))));
        let q = format!("code=a&code=b&state={}", p.state);
        assert!(matches!(client.check_callback(&q, &p), Err(Error::InvalidCallback(_))));
    }

    #[test]
    fn discovery_with_another_issuer_is_refused() {
        let config = Config::new(ISSUER, "c", "s", "https://app.example.org/cb");
        let mut metadata = Metadata {
            issuer: format!("{ISSUER}/"),
            authorization_endpoint: format!("{ISSUER}/oauth/authorize"),
            token_endpoint: format!("{ISSUER}/oauth/token"),
            jwks_uri: format!("{ISSUER}/oauth/jwks"),
        };
        assert!(matches!(check_metadata(&config, &metadata), Err(Error::IssuerMismatch { .. })));
        metadata.issuer = ISSUER.into();
        assert!(check_metadata(&config, &metadata).is_ok());
    }

    #[test]
    fn basic_auth_form_encodes_both_halves() {
        // "a:b c" → "a%3Ab+c"; base64("id:a%3Ab+c").
        assert_eq!(basic_auth("id", "a:b c"), format!("Basic {}", STANDARD.encode("id:a%3Ab+c")));
    }

    fn claims(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn identity_fields_and_helpers() {
        let id = Identity::from_claims(claims(json!({
            "sub": "5966f47e", "preferred_username": "ana", "name": "Ana Maria",
            "email": "ana@example.org", "setor": "FIN", "groups": ["fiscal"],
            "group_roles": {"fiscal": ["analista", "gestor"]}, "extra": 1
        })))
        .unwrap();
        assert_eq!(id.sub, "5966f47e");
        assert_eq!(id.username.as_deref(), Some("ana"));
        assert_eq!(id.name.as_deref(), Some("Ana Maria"));
        assert_eq!(id.email.as_deref(), Some("ana@example.org"));
        assert_eq!(id.setor.as_deref(), Some("FIN"));
        assert!(id.has_group("fiscal"));
        assert!(!id.has_group("other"));
        assert!(id.has_role("fiscal", "gestor"));
        assert!(!id.has_role("fiscal", "admin"));
        assert!(!id.has_role("other", "gestor"));
        assert_eq!(id.roles("fiscal"), ["analista", "gestor"]);
        assert!(id.roles("other").is_empty());
        assert_eq!(id.claims["extra"], 1);
    }

    #[test]
    fn missing_optional_claims_are_empty() {
        let id = Identity::from_claims(claims(json!({"sub": "x"}))).unwrap();
        assert_eq!(id.username, None);
        assert_eq!(id.name, None);
        assert_eq!(id.email, None);
        assert_eq!(id.setor, None);
        assert!(id.groups.is_empty());
        assert!(id.group_roles.is_empty());
        assert!(id.roles("fiscal").is_empty());
    }

    #[test]
    fn identity_without_sub_is_refused() {
        assert!(matches!(
            Identity::from_claims(claims(json!({"preferred_username": "ana"}))),
            Err(Error::InvalidIdToken(_))
        ));
    }

    #[test]
    fn debug_output_hides_the_secret() {
        let c = offline_client();
        assert!(!format!("{:?}", c.config()).contains("s3cret"));
    }

    /// The one-refresh-on-unknown-`kid` path, against the running test
    /// provider (skipped without `RUSCKER_ISSUER`). Lives here because it
    /// needs to start the client with an EMPTY key cache.
    #[tokio::test]
    async fn unknown_kid_refreshes_the_key_set_once() {
        let Ok(issuer) = std::env::var("RUSCKER_ISSUER") else {
            eprintln!("RUSCKER_ISSUER is not set: skipping");
            return;
        };
        let var = |n: &str| std::env::var(n).unwrap();
        let config = Config::new(issuer, var("RUSCKER_CLIENT_ID"), var("RUSCKER_CLIENT_SECRET"), var("RUSCKER_REDIRECT_URI"));
        let real = Client::discover(config.clone()).await.unwrap();
        let client =
            Client::from_parts(config, real.inner.metadata.clone(), reqwest::Client::new(), JwkSet { keys: vec![] });

        let (url, pending) = client.start_sign_in();
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../testing/authorize.sh");
        let out = std::process::Command::new(script).arg(url.as_str()).output().unwrap();
        let callback = String::from_utf8(out.stdout).unwrap();
        client.finish_sign_in(callback.trim(), &pending).await.expect("sign-in after a key refresh");
        assert!(!client.inner.jwks.read().unwrap().keys.is_empty(), "the cache was refreshed");
    }
}
