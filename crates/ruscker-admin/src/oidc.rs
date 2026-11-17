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
//! Nothing here issues a token yet; that is slices 3 and 4.
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
use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING};
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
/// Safe under concurrency and across HA nodes: if two callers race to
/// create the first key, both rows are kept (both are published in the
/// JWKS) and everyone then agrees on the same "newest" one, because the
/// choice is re-read from the database after the insert.
pub async fn ensure_signing_key(db: &ConfigDb, master_key: &MasterKey) -> Result<SigningKey> {
    if !master_key.is_configured() {
        bail!("RUSCKER_MASTER_KEY is not set; the OIDC provider needs it to protect its signing key");
    }
    if let Some(newest) = oidc_keys::list_newest_first(db).await?.first() {
        return load(newest, master_key);
    }
    let fresh = generate(master_key)?;
    oidc_keys::insert(db, &fresh).await?;
    tracing::info!(kid = %fresh.kid, "generated the OIDC provider signing key");
    let keys = oidc_keys::list_newest_first(db).await?;
    let newest = keys.first().ok_or_else(|| anyhow!("OIDC signing key vanished after insert"))?;
    load(newest, master_key)
}

/// The JSON Web Key Set: every stored public key. Clients fetch this to
/// verify token signatures. Older keys stay published so tokens signed
/// before a rotation remain verifiable.
pub async fn jwks(db: &ConfigDb) -> Result<Value> {
    let mut keys = Vec::new();
    for k in oidc_keys::list_newest_first(db).await? {
        let jwk: Value = serde_json::from_str(&k.public_jwk)
            .with_context(|| format!("stored JWK for key {} is not JSON", k.kid))?;
        keys.push(jwk);
    }
    Ok(json!({ "keys": keys }))
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
    use ring::signature::{UnparsedPublicKey, ECDSA_P256_SHA256_FIXED};

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

        let set = jwks(&db).await.unwrap();
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
        let set = jwks(&db).await.unwrap();
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
        oidc_keys::insert(&db, &a).await.unwrap();
        let err = ensure_signing_key(&db, &mk).await.unwrap_err();
        assert!(err.to_string().contains("does not match"), "{err}");
    }

    #[tokio::test]
    async fn two_first_keys_converge_on_one_signer() {
        // Two nodes each generated a "first" key at the same time. Both
        // are published; both nodes then pick the same one to sign with.
        let db = mem_db().await;
        let mk = MasterKey::parse(KEY_A).unwrap();
        oidc_keys::insert(&db, &generate(&mk).unwrap()).await.unwrap();
        oidc_keys::insert(&db, &generate(&mk).unwrap()).await.unwrap();
        let one = ensure_signing_key(&db, &mk).await.unwrap();
        let two = ensure_signing_key(&db, &mk).await.unwrap();
        assert_eq!(one.kid, two.kid);
        assert_eq!(jwks(&db).await.unwrap()["keys"].as_array().unwrap().len(), 2);
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
        let set = jwks(&db).await.unwrap();
        assert_eq!(set["keys"].as_array().unwrap().len(), 1);
        UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, point_from_jwk(&set["keys"][0]))
            .verify(b"x", &sig)
            .unwrap();
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
