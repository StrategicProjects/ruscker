//! OIDC provider — signing keys and the public metadata documents
//! (slice 2 of `docs/adr/0005-oidc-provider.md`).
//!
//! What lives here:
//!
//! * [`SigningKey`] — an ES256 (ECDSA P-256 + SHA-256) key pair that will
//!   sign ID tokens. Generated and used through `ring`.
//! * [`ensure_signing_key`] — load the current key, creating the first
//!   one on demand. The private half is stored encrypted under
//!   `RUSCKER_MASTER_KEY`; the provider refuses to work without it.
//! * [`jwks`] and [`discovery`] — the two JSON documents a client reads
//!   to learn where the endpoints are and how to verify a token.
//!
//! * [`sign_jwt`] / [`verify_jwt`] — compact JWS (RFC 7515) with that
//!   key: what an ID token and the provider's own access token are.
//! * [`pkce_matches`] — the S256 proof-key check of RFC 7636.
//!
//! ## Why ES256
//!
//! `ring` (already a dependency) can both *generate* and sign with P-256
//! keys. It can sign with RSA but cannot generate RSA keys, which would
//! mean a new dependency or an operator-supplied key file. ES256 is
//! supported by every mainstream OIDC client library.

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use ring::rand::SystemRandom;
use ring::signature::{
    EcdsaKeyPair, KeyPair, UnparsedPublicKey, ECDSA_P256_SHA256_FIXED,
    ECDSA_P256_SHA256_FIXED_SIGNING,
};
use serde_json::{json, Value};

use crate::crypto::MasterKey;
use crate::db::oidc_keys::{self, StoredKey};
use crate::db::ConfigDb;

/// The only signature algorithm this provider uses.
pub const ALG: &str = "ES256";

/// A ready-to-use signing key.
pub struct SigningKey {
    /// Key id, placed in every token header (`kid`).
    pub kid: String,
    pair: EcdsaKeyPair,
}

impl SigningKey {
    /// Sign `message` and return the raw 64-byte `r || s` signature —
    /// exactly the encoding a JWS with `alg: ES256` carries (RFC 7518
    /// §3.4), so the caller only has to base64url it.
    pub fn sign(&self, message: &[u8]) -> Result<Vec<u8>> {
        let sig = self
            .pair
            .sign(&SystemRandom::new(), message)
            .map_err(|_| anyhow!("ES256 signing failed"))?;
        Ok(sig.as_ref().to_vec())
    }
}

impl std::fmt::Debug for SigningKey {
    /// Never print key material, not even by accident in a log line.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SigningKey").field("kid", &self.kid).finish_non_exhaustive()
    }
}

/// Build the public JWK and its key id from an uncompressed P-256 point
/// (`0x04 || X || Y`, 65 bytes — what `ring` returns).
///
/// The `kid` is the RFC 7638 JWK thumbprint: SHA-256 over the canonical
/// JSON of the required members in lexicographic order. Deriving it from
/// the key (instead of a random id) means two nodes that somehow hold
/// the same key agree on its id.
fn public_jwk(point: &[u8]) -> Result<(String, Value)> {
    if point.len() != 65 || point[0] != 0x04 {
        bail!("unexpected P-256 public key encoding");
    }
    let x = URL_SAFE_NO_PAD.encode(&point[1..33]);
    let y = URL_SAFE_NO_PAD.encode(&point[33..65]);
    // Canonical form for the thumbprint: exactly these members, this
    // order, no whitespace.
    let canonical = format!(r#"{{"crv":"P-256","kty":"EC","x":"{x}","y":"{y}"}}"#);
    let digest = ring::digest::digest(&ring::digest::SHA256, canonical.as_bytes());
    let kid = URL_SAFE_NO_PAD.encode(digest.as_ref());
    let jwk = json!({
        "kty": "EC",
        "crv": "P-256",
        "x": x,
        "y": y,
        "use": "sig",
        "alg": ALG,
        "kid": kid,
    });
    Ok((kid, jwk))
}

/// Generate a brand-new key and its encrypted, storable form.
fn generate(master_key: &MasterKey) -> Result<StoredKey> {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng)
        .map_err(|_| anyhow!("generate ES256 key"))?;
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
        .map_err(|_| anyhow!("load freshly generated ES256 key"))?;
    let (kid, jwk) = public_jwk(pair.public_key().as_ref())?;
    let (pkcs8_enc, nonce) = master_key.encrypt(pkcs8.as_ref())?;
    Ok(StoredKey {
        kid,
        alg: ALG.to_string(),
        pkcs8_enc,
        nonce,
        public_jwk: jwk.to_string(),
        created_at: Utc::now(),
    })
}

/// Decrypt a stored key and check it is the key it claims to be.
fn load(stored: &StoredKey, master_key: &MasterKey) -> Result<SigningKey> {
    if stored.alg != ALG {
        bail!("unsupported signing algorithm `{}`", stored.alg);
    }
    let pkcs8 = master_key
        .decrypt(&stored.pkcs8_enc, &stored.nonce)
        .context("decrypt OIDC signing key (was RUSCKER_MASTER_KEY changed?)")?;
    let pair =
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pkcs8, &SystemRandom::new())
            .map_err(|_| anyhow!("stored OIDC signing key is not a valid ES256 key"))?;
    // The published JWK and the private key must be the same key pair.
    // If the row was tampered with (or half-restored from a backup),
    // signing with one while publishing the other would produce tokens
    // no client can verify — fail loudly here instead.
    let (kid, _) = public_jwk(pair.public_key().as_ref())?;
    if kid != stored.kid {
        bail!("stored OIDC signing key does not match its key id");
    }
    Ok(SigningKey { kid, pair })
}

/// The current signing key, creating the first one if none exists.
///
/// Safe under concurrency and across HA nodes: creation is a single
/// atomic "insert if the table is empty" ([`oidc_keys::insert_first`]),
/// and the key to use is always re-read from the database afterwards,
/// so a caller that lost the race ends up with the winner's key.
///
/// If the stored key cannot be decrypted (the master key changed) this
/// fails and does NOT mint a replacement: silently switching keys would
/// invalidate every token in flight and hide the misconfiguration.
/// Recovery is an operator decision — restore the old master key, or
/// delete the rows of `oidc_keys` to start over.
pub async fn ensure_signing_key(db: &ConfigDb, master_key: &MasterKey) -> Result<SigningKey> {
    if !master_key.is_configured() {
        bail!("RUSCKER_MASTER_KEY is not set; the OIDC provider needs it to protect its signing key");
    }
    if let Some(newest) = oidc_keys::list_newest_first(db).await?.first() {
        return load(newest, master_key);
    }
    let fresh = generate(master_key)?;
    if oidc_keys::insert_first(db, &fresh).await? {
        tracing::info!(kid = %fresh.kid, "generated the OIDC provider signing key");
    }
    let keys = oidc_keys::list_newest_first(db).await?;
    let newest = keys.first().ok_or_else(|| anyhow!("OIDC signing key vanished after insert"))?;
    load(newest, master_key)
}

/// The JSON Web Key Set: every stored public key, read straight from
/// the clear `public_jwk` column — nothing is decrypted, so this keeps
/// working (and existing tokens stay verifiable) even while the private
/// half is unreadable. Returns `None` when no key exists yet.
///
/// Note for a future rotation feature: clients cache this document
/// (see the endpoint's `max-age`), so a new key must be PUBLISHED for at
/// least that long before it starts signing.
pub async fn published_jwks(db: &ConfigDb) -> Result<Option<Value>> {
    let stored = oidc_keys::list_newest_first(db).await?;
    if stored.is_empty() {
        return Ok(None);
    }
    let mut keys = Vec::with_capacity(stored.len());
    for k in stored {
        let jwk: Value = serde_json::from_str(&k.public_jwk)
            .with_context(|| format!("stored JWK for key {} is not JSON", k.kid))?;
        keys.push(jwk);
    }
    Ok(Some(json!({ "keys": keys })))
}

/// The issuer this process actually serves, or `None` when the provider
/// is off. Two conditions beyond the config switch:
///
/// * the issuer's path must equal the portal's mount point. OIDC puts
///   the discovery document at `{issuer}/.well-known/…`; an issuer of
///   `https://example.org` on a portal mounted at `/box` would hand out
///   a document full of URLs that do not exist.
///
/// The base path can come from `--base-path`, which config validation
/// cannot see, so the check lives here (and [`warn_if_misconfigured`]
/// says so once at boot).
pub fn active_issuer(config: &ruscker_config::Config, base_path: &str) -> Option<String> {
    let issuer = config.server.oidc_issuer()?;
    (ruscker_config::issuer_path(&issuer) == base_path).then_some(issuer)
}

/// Boot-time warnings for an enabled provider that cannot work. Each of
/// these states is fail-closed at request time (404 or 503); the
/// warning is what makes it discoverable without a client trying.
pub fn warn_if_misconfigured(
    config: &ruscker_config::Config,
    base_path: &str,
    master_key: &MasterKey,
) {
    let Some(issuer) = config.server.oidc_issuer() else {
        return;
    };
    if ruscker_config::issuer_path(&issuer) != base_path {
        tracing::warn!(
            issuer = %issuer,
            base_path = %base_path,
            "server.oidc-provider.issuer does not end with the portal's base path; \
             the OIDC provider stays off"
        );
    } else if !master_key.is_configured() {
        tracing::warn!(
            "server.oidc-provider is enabled but RUSCKER_MASTER_KEY is not set; \
             the OIDC provider answers 503 until it is"
        );
    }
}

/// Build a signed compact JWT: `base64url(header).base64url(claims).base64url(signature)`.
///
/// `typ` goes in the header and is how the two kinds of token this
/// provider mints are told apart (`JWT` for an ID token, `at+jwt` for
/// an access token, RFC 9068) — so one can never be accepted as the
/// other.
pub fn sign_jwt(key: &SigningKey, typ: &str, claims: &Value) -> Result<String> {
    let header = json!({ "alg": ALG, "typ": typ, "kid": key.kid });
    let signing_input = format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(header.to_string()),
        URL_SAFE_NO_PAD.encode(claims.to_string())
    );
    let signature = key.sign(signing_input.as_bytes())?;
    Ok(format!("{signing_input}.{}", URL_SAFE_NO_PAD.encode(signature)))
}

/// A JWT whose signature checked out against one of OUR published keys.
/// Only the signature is established here — the caller must still check
/// `typ`, `iss`, `aud`, `exp` and whatever else gives the token meaning.
#[derive(Debug)]
pub struct VerifiedJwt {
    pub typ: String,
    pub claims: Value,
}

/// Verify a compact JWT against the stored public keys. `None` for
/// anything that is not a well-formed ES256 token signed by a key we
/// published — malformed input is not an error, just not a valid token.
///
/// The algorithm is pinned to ES256 and the key is looked up by `kid`
/// among OUR keys; nothing in the token can choose a different
/// algorithm or supply its own key (the classic `alg: none` / embedded
/// `jwk` attacks).
pub async fn verify_jwt(db: &ConfigDb, token: &str) -> Result<Option<VerifiedJwt>> {
    let mut parts = token.split('.');
    let (Some(h), Some(c), Some(sig), None) =
        (parts.next(), parts.next(), parts.next(), parts.next())
    else {
        return Ok(None);
    };
    let decode = |part: &str| URL_SAFE_NO_PAD.decode(part).ok();
    let (Some(header), Some(claims), Some(signature)) = (decode(h), decode(c), decode(sig)) else {
        return Ok(None);
    };
    let Ok(header) = serde_json::from_slice::<Value>(&header) else {
        return Ok(None);
    };
    if header["alg"] != ALG {
        return Ok(None);
    }
    let (Some(kid), Some(typ)) = (header["kid"].as_str(), header["typ"].as_str()) else {
        return Ok(None);
    };
    let Some(stored) = oidc_keys::list_newest_first(db)
        .await?
        .into_iter()
        .find(|k| k.kid == kid && k.alg == ALG)
    else {
        return Ok(None);
    };
    let jwk: Value = serde_json::from_str(&stored.public_jwk)
        .with_context(|| format!("stored JWK for key {kid} is not JSON"))?;
    let (Some(x), Some(y)) = (
        jwk["x"].as_str().and_then(decode),
        jwk["y"].as_str().and_then(decode),
    ) else {
        bail!("stored JWK for key {kid} has no usable coordinates");
    };
    let mut point = Vec::with_capacity(65);
    point.push(0x04);
    point.extend(x);
    point.extend(y);
    let signing_input = &token[..h.len() + 1 + c.len()];
    if UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point)
        .verify(signing_input.as_bytes(), &signature)
        .is_err()
    {
        return Ok(None);
    }
    let Ok(claims) = serde_json::from_slice::<Value>(&claims) else {
        return Ok(None);
    };
    Ok(Some(VerifiedJwt { typ: typ.to_string(), claims }))
}

/// PKCE (RFC 7636) with `S256`: does `verifier` hash to `challenge`?
///
/// The verifier must be 43–128 characters of the unreserved set — the
/// RFC's own bounds; shorter would be guessable, and refusing odd
/// characters keeps every client library's encoding in agreement.
pub fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    let well_formed = (43..=128).contains(&verifier.len())
        && verifier
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~'));
    if !well_formed {
        return false;
    }
    let digest = ring::digest::digest(&ring::digest::SHA256, verifier.as_bytes());
    let computed = URL_SAFE_NO_PAD.encode(digest.as_ref());
    // Constant time: the challenge is not a secret, but there is no
    // reason to make this the one comparison that leaks.
    #[allow(deprecated)]
    ring::constant_time::verify_slices_are_equal(computed.as_bytes(), challenge.as_bytes()).is_ok()
}

/// The OpenID Provider Metadata document (OIDC Discovery 1.0 §3),
/// served at `{issuer}/.well-known/openid-configuration`.
///
/// It describes the narrow profile of ADR 0005 and nothing more: one
/// flow, one signing algorithm, PKCE required, confidential clients.
/// Advertising less is deliberate — a client library will not attempt
/// what the document does not offer.
pub fn discovery(issuer: &str) -> Value {
    json!({
        "issuer": issuer,
        "authorization_endpoint": format!("{issuer}/oauth/authorize"),
        "token_endpoint": format!("{issuer}/oauth/token"),
        "userinfo_endpoint": format!("{issuer}/oauth/userinfo"),
        "jwks_uri": format!("{issuer}/oauth/jwks"),
        "response_types_supported": ["code"],
        "response_modes_supported": ["query"],
        "grant_types_supported": ["authorization_code"],
        "subject_types_supported": ["public"],
        "id_token_signing_alg_values_supported": [ALG],
        "token_endpoint_auth_methods_supported": ["client_secret_basic", "client_secret_post"],
        "code_challenge_methods_supported": ["S256"],
        "authorization_response_iss_parameter_supported": true,
        "request_parameter_supported": false,
        "request_uri_parameter_supported": false,
        "scopes_supported": ["openid", "profile", "email", "groups"],
        "claims_supported": [
            "sub", "iss", "aud", "exp", "iat", "nonce",
            "preferred_username", "name", "email", "groups"
        ],
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY_A: &str = "0000000000000000000000000000000000000000000000000000000000000001";
    const KEY_B: &str = "0000000000000000000000000000000000000000000000000000000000000002";

    async fn mem_db() -> ConfigDb {
        ConfigDb::Sqlite(crate::db::open_memory().await.expect("open in-memory"))
    }

    /// Rebuild the raw public point from a published JWK — what a client
    /// library does before verifying.
    fn point_from_jwk(jwk: &Value) -> Vec<u8> {
        let mut p = vec![0x04];
        p.extend(URL_SAFE_NO_PAD.decode(jwk["x"].as_str().unwrap()).unwrap());
        p.extend(URL_SAFE_NO_PAD.decode(jwk["y"].as_str().unwrap()).unwrap());
        p
    }

    #[tokio::test]
    async fn a_signature_verifies_against_the_published_jwks() {
        let db = mem_db().await;
        let mk = MasterKey::parse(KEY_A).unwrap();
        let key = ensure_signing_key(&db, &mk).await.unwrap();

        let msg = b"header.payload";
        let sig = key.sign(msg).unwrap();
        assert_eq!(sig.len(), 64, "raw r||s, the JWS ES256 encoding");

        let set = published_jwks(&db).await.unwrap().expect("a key set");
        let keys = set["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 1);
        let jwk = &keys[0];
        assert_eq!(jwk["kid"], key.kid.as_str());
        assert_eq!(jwk["alg"], "ES256");
        assert_eq!(jwk["use"], "sig");
        // The public document carries no private member.
        assert!(jwk.get("d").is_none());

        let public = UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point_from_jwk(jwk));
        public.verify(msg, &sig).expect("signature verifies with the published key");
        assert!(public.verify(b"header.tampered", &sig).is_err());
    }

    #[tokio::test]
    async fn ensure_is_idempotent_and_survives_a_reload() {
        let db = mem_db().await;
        let mk = MasterKey::parse(KEY_A).unwrap();
        let first = ensure_signing_key(&db, &mk).await.unwrap();
        let again = ensure_signing_key(&db, &mk).await.unwrap();
        assert_eq!(first.kid, again.kid);
        assert_eq!(oidc_keys::list_newest_first(&db).await.unwrap().len(), 1);

        // The reloaded key is the same key: its signature verifies with
        // the JWK published for the first one.
        let sig = again.sign(b"x").unwrap();
        let set = published_jwks(&db).await.unwrap().expect("a key set");
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point_from_jwk(&set["keys"][0]))
            .verify(b"x", &sig)
            .unwrap();
    }

    #[tokio::test]
    async fn the_private_key_is_encrypted_at_rest() {
        let db = mem_db().await;
        let mk = MasterKey::parse(KEY_A).unwrap();
        ensure_signing_key(&db, &mk).await.unwrap();
        let stored = oidc_keys::list_newest_first(&db).await.unwrap().remove(0);

        // What is on disk is not a usable PKCS#8 document…
        assert!(EcdsaKeyPair::from_pkcs8(
            &ECDSA_P256_SHA256_FIXED_SIGNING,
            &stored.pkcs8_enc,
            &SystemRandom::new()
        )
        .is_err());
        // …and a different master key cannot open it.
        let other = MasterKey::parse(KEY_B).unwrap();
        assert!(ensure_signing_key(&db, &other).await.is_err());
    }

    #[tokio::test]
    async fn refuses_to_work_without_a_master_key() {
        let db = mem_db().await;
        let err = ensure_signing_key(&db, &MasterKey::default()).await.unwrap_err();
        assert!(err.to_string().contains("RUSCKER_MASTER_KEY"));
        assert!(oidc_keys::list_newest_first(&db).await.unwrap().is_empty(), "nothing stored");
    }

    #[tokio::test]
    async fn a_mismatched_row_is_rejected() {
        let db = mem_db().await;
        let mk = MasterKey::parse(KEY_A).unwrap();
        // A private key stored under ANOTHER key's id (tampering, or a
        // half-restored backup).
        let mut a = generate(&mk).unwrap();
        let b = generate(&mk).unwrap();
        a.kid = b.kid.clone();
        assert!(oidc_keys::insert_first(&db, &a).await.unwrap());
        let err = ensure_signing_key(&db, &mk).await.unwrap_err();
        assert!(err.to_string().contains("does not match"), "{err}");
    }

    /// Many first requests at once create exactly ONE key, and all of
    /// them sign with it.
    #[tokio::test]
    async fn concurrent_first_use_creates_a_single_key() {
        let db = mem_db().await;
        let mk = MasterKey::parse(KEY_A).unwrap();
        let mut tasks = Vec::new();
        for _ in 0..12 {
            let (db, mk) = (db.clone(), mk.clone());
            tasks.push(tokio::spawn(async move {
                ensure_signing_key(&db, &mk).await.unwrap().kid
            }));
        }
        let mut kids = Vec::new();
        for t in tasks {
            kids.push(t.await.unwrap());
        }
        kids.dedup();
        assert_eq!(kids.len(), 1, "everyone signs with the same key");
        assert_eq!(oidc_keys::list_newest_first(&db).await.unwrap().len(), 1);
        // A second "first key" is refused outright.
        assert!(!oidc_keys::insert_first(&db, &generate(&mk).unwrap()).await.unwrap());
    }

    /// The JWKS never needs the private key: with the wrong master key
    /// signing fails, but the public set is still served, and no
    /// replacement key is minted behind the operator's back.
    #[tokio::test]
    async fn jwks_survives_an_unreadable_private_key() {
        let db = mem_db().await;
        let kid = ensure_signing_key(&db, &MasterKey::parse(KEY_A).unwrap()).await.unwrap().kid;
        let other = MasterKey::parse(KEY_B).unwrap();
        assert!(ensure_signing_key(&db, &other).await.is_err());
        let set = published_jwks(&db).await.unwrap().expect("still published");
        let keys = set["keys"].as_array().unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0]["kid"], kid.as_str());
    }

    #[tokio::test]
    async fn key_creation_is_audited_by_id_only() {
        let db = mem_db().await;
        assert!(published_jwks(&db).await.unwrap().is_none(), "no keys yet");
        let kid = ensure_signing_key(&db, &MasterKey::parse(KEY_A).unwrap()).await.unwrap().kid;
        let audits = crate::db::audit::list(&db, &crate::db::audit::AuditFilter::new())
            .await
            .unwrap();
        let ours: Vec<_> = audits.iter().filter(|e| e.action == "oidc-key.create").collect();
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0].target.as_deref(), Some(format!("oidc-key:{kid}").as_str()));
        assert!(ours[0].actor.is_none() && ours[0].diff.is_none());
    }

    #[test]
    fn active_issuer_requires_the_issuer_path_to_be_the_mount_point() {
        let cfg = |issuer: &str| {
            ruscker_config::Config::from_yaml(&format!(
                "server:\n  oidc-provider:\n    enabled: true\n    issuer: {issuer}\nproxy:\n  specs: []\n"
            ))
            .unwrap()
        };
        assert_eq!(active_issuer(&cfg("https://p.example"), "").as_deref(), Some("https://p.example"));
        assert_eq!(
            active_issuer(&cfg("https://p.example/box/"), "/box").as_deref(),
            Some("https://p.example/box")
        );
        // Mismatch in either direction ⇒ off.
        assert_eq!(active_issuer(&cfg("https://p.example"), "/box"), None);
        assert_eq!(active_issuer(&cfg("https://p.example/box"), ""), None);
        assert_eq!(active_issuer(&cfg("https://p.example/other"), "/box"), None);
    }

    // Key storage against a real Postgres (BYTEA / TIMESTAMPTZ). Gated
    // on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn signing_key_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pg = crate::db::open_pg(&url).await.unwrap();
        sqlx::query("DELETE FROM oidc_keys").execute(&pg).await.unwrap();
        let db = ConfigDb::Postgres(pg);
        let mk = MasterKey::parse(KEY_A).unwrap();

        let first = ensure_signing_key(&db, &mk).await.unwrap();
        let again = ensure_signing_key(&db, &mk).await.unwrap();
        assert_eq!(first.kid, again.kid);
        let sig = again.sign(b"x").unwrap();
        let set = published_jwks(&db).await.unwrap().expect("a key set");
        assert_eq!(set["keys"].as_array().unwrap().len(), 1);
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point_from_jwk(&set["keys"][0]))
            .verify(b"x", &sig)
            .unwrap();
        // The advisory-locked "first key only" insert, concurrently.
        sqlx::query("DELETE FROM oidc_keys")
            .execute(match &db {
                ConfigDb::Postgres(p) => p,
                ConfigDb::Sqlite(_) => unreachable!(),
            })
            .await
            .unwrap();
        let mut tasks = Vec::new();
        for _ in 0..8 {
            let (db, mk) = (db.clone(), mk.clone());
            tasks.push(tokio::spawn(async move {
                ensure_signing_key(&db, &mk).await.unwrap().kid
            }));
        }
        let mut kids = Vec::new();
        for t in tasks {
            kids.push(t.await.unwrap());
        }
        kids.dedup();
        assert_eq!(kids.len(), 1);
        assert_eq!(oidc_keys::list_newest_first(&db).await.unwrap().len(), 1);
    }

    /// Known answer: the P-256 key of RFC 7515 Appendix A.3, whose RFC
    /// 7638 thumbprint was computed independently (SHA-256 over the
    /// canonical `{"crv","kty","x","y"}` JSON). Catches a wrong member
    /// order, an extra member or padded base64 — none of which the
    /// determinism check below would notice.
    #[test]
    fn kid_matches_a_known_rfc7638_thumbprint() {
        let mut point = vec![0x04];
        point.extend(
            URL_SAFE_NO_PAD.decode("f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU").unwrap(),
        );
        point.extend(
            URL_SAFE_NO_PAD.decode("x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0").unwrap(),
        );
        let (kid, jwk) = public_jwk(&point).unwrap();
        assert_eq!(kid, "oKIywvGUpTVTyxMQ3bwIIeQUudfr_CkLMjCE19ECD-U");
        assert_eq!(jwk["x"], "f83OJ3D2xF1Bg8vub9tLe1gHMzV76e8Tus9uPHvRVEU");
        assert_eq!(jwk["y"], "x_FEzRu9m36HLN_tue659LNpXW6pCyStikYjKIWI5a0");
    }

    #[test]
    fn kid_is_the_rfc7638_thumbprint() {
        // The thumbprint is a pure function of the point: 43 base64url
        // characters (an unpadded SHA-256).
        let mut point = vec![0x04];
        point.extend([1u8; 32]);
        point.extend([2u8; 32]);
        let (kid, jwk) = public_jwk(&point).unwrap();
        let (kid2, _) = public_jwk(&point).unwrap();
        assert_eq!(kid, kid2);
        assert_eq!(kid.len(), 43);
        assert_eq!(jwk["kid"], kid.as_str());
        assert!(public_jwk(&point[..64]).is_err(), "wrong length");
    }

    #[tokio::test]
    async fn a_signed_jwt_verifies_and_tampering_does_not() {
        let db = mem_db().await;
        let mk = MasterKey::parse(KEY_A).unwrap();
        let key = ensure_signing_key(&db, &mk).await.unwrap();
        let token = sign_jwt(&key, "JWT", &json!({"sub": "u-1", "aud": "radar"})).unwrap();
        assert_eq!(token.split('.').count(), 3);

        let ok = verify_jwt(&db, &token).await.unwrap().expect("valid");
        assert_eq!(ok.typ, "JWT");
        assert_eq!(ok.claims["sub"], "u-1");

        let parts: Vec<&str> = token.split('.').collect();
        let enc = |v: Value| URL_SAFE_NO_PAD.encode(v.to_string());
        // Different claims under the same signature.
        let forged = format!("{}.{}.{}", parts[0], enc(json!({"sub": "admin", "aud": "radar"})), parts[2]);
        assert!(verify_jwt(&db, &forged).await.unwrap().is_none());
        // `alg: none`, with and without a signature.
        let none = enc(json!({"alg": "none", "typ": "JWT", "kid": key.kid}));
        assert!(verify_jwt(&db, &format!("{none}.{}.", parts[1])).await.unwrap().is_none());
        assert!(verify_jwt(&db, &format!("{none}.{}.{}", parts[1], parts[2])).await.unwrap().is_none());
        // A symmetric algorithm name (the "public key as HMAC secret" trick).
        let hs = enc(json!({"alg": "HS256", "typ": "JWT", "kid": key.kid}));
        assert!(verify_jwt(&db, &format!("{hs}.{}.{}", parts[1], parts[2])).await.unwrap().is_none());
        // Unknown kid; a key the token brings along is ignored.
        let other = enc(json!({"alg": "ES256", "typ": "JWT", "kid": "nope", "jwk": {"kty": "EC"}}));
        assert!(verify_jwt(&db, &format!("{other}.{}.{}", parts[1], parts[2])).await.unwrap().is_none());
        // Garbage.
        for junk in ["", "a.b", "a.b.c.d", "!!.!!.!!", &token[..token.len() - 2]] {
            assert!(verify_jwt(&db, junk).await.unwrap().is_none(), "{junk:?}");
        }
    }

    #[tokio::test]
    async fn a_jwt_signed_by_another_installation_is_refused() {
        let ours = mem_db().await;
        let theirs = mem_db().await;
        let mk = MasterKey::parse(KEY_A).unwrap();
        ensure_signing_key(&ours, &mk).await.unwrap();
        let their_key = ensure_signing_key(&theirs, &mk).await.unwrap();
        let token = sign_jwt(&their_key, "JWT", &json!({"sub": "u-1"})).unwrap();
        assert!(verify_jwt(&theirs, &token).await.unwrap().is_some());
        assert!(verify_jwt(&ours, &token).await.unwrap().is_none());
    }

    /// RFC 7636 Appendix B: the worked S256 example.
    #[test]
    fn pkce_known_answer_and_bounds() {
        let verifier = "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk";
        let challenge = "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM";
        assert!(pkce_matches(verifier, challenge));
        assert!(!pkce_matches(verifier, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cN"));
        assert!(!pkce_matches("wrong-verifier-wrong-verifier-wrong-verifier-1", challenge));
        // `plain` must not work: the challenge itself is not a verifier.
        assert!(!pkce_matches(challenge, challenge));
        // Bounds and alphabet.
        assert!(!pkce_matches(&verifier[..42], challenge));
        assert!(!pkce_matches(&"a".repeat(129), challenge));
        assert!(!pkce_matches(&format!("{} ", &verifier[..42]), challenge));
    }

    #[test]
    fn discovery_describes_the_narrow_profile() {
        let d = discovery("https://portal.example/box");
        assert_eq!(d["issuer"], "https://portal.example/box");
        assert_eq!(d["jwks_uri"], "https://portal.example/box/oauth/jwks");
        assert_eq!(d["authorization_endpoint"], "https://portal.example/box/oauth/authorize");
        assert_eq!(d["response_types_supported"], json!(["code"]));
        assert_eq!(d["grant_types_supported"], json!(["authorization_code"]));
        assert_eq!(d["code_challenge_methods_supported"], json!(["S256"]));
        assert_eq!(d["id_token_signing_alg_values_supported"], json!(["ES256"]));
        // Nothing weaker is on offer.
        let text = d.to_string();
        for banned in ["implicit", "password", "\"none\"", "plain", "id_token token"] {
            assert!(!text.contains(banned), "must not advertise {banned}");
        }
    }
}
