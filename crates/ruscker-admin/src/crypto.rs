//! Cryptographic primitives for the credentials store.
//!
//! Operators supply a 32-byte master key in `RUSCKER_MASTER_KEY`
//! as either:
//!
//! - 64 lowercase hex chars (e.g. `b5e3...c1`)
//! - 44-char base64 (with or without `=` padding)
//!
//! Generate one with `openssl rand -hex 32` or
//! `openssl rand -base64 32`.
//!
//! The key encrypts every credential password using AES-256-GCM
//! with a fresh 12-byte nonce per row. The nonce is stored
//! alongside the ciphertext (it's not secret — it just has to be
//! unique per (key, plaintext) pair, which a CSPRNG guarantees
//! with overwhelming probability).
//!
//! If the master key is rotated, every prior credential becomes
//! unreadable. Re-entry is the only recovery — by design; that's
//! also the property that limits the blast radius if the env var
//! leaks but the DB doesn't.

use aes_gcm::aead::{Aead, AeadCore, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, Context, Result};
use base64::Engine;
use std::sync::Arc;
use zeroize::Zeroizing;

/// The 32-byte AES-256-GCM key, derived from `RUSCKER_MASTER_KEY`.
///
/// Wrapped in `Arc` because `AppState` is cloned per request and we
/// don't want to copy the secret 31 times per landing render. The
/// underlying buffer is `Zeroizing<[u8; 32]>` — when the last `Arc`
/// drops the bytes are wiped from memory.
#[derive(Clone, Debug, Default)]
pub struct MasterKey {
    inner: Option<Arc<Zeroizing<[u8; 32]>>>,
}

impl MasterKey {
    pub fn from_env() -> Result<Self> {
        let Ok(raw) = std::env::var("RUSCKER_MASTER_KEY") else {
            return Ok(Self::default());
        };
        if raw.is_empty() {
            return Ok(Self::default());
        }
        Self::from_str(&raw)
    }

    pub fn from_str(raw: &str) -> Result<Self> {
        let bytes = decode_key_material(raw)?;
        Ok(Self {
            inner: Some(Arc::new(Zeroizing::new(bytes))),
        })
    }

    pub fn is_configured(&self) -> bool {
        self.inner.is_some()
    }

    /// Encrypt `plaintext` with a fresh random nonce. Returns
    /// `(ciphertext, nonce)`; the caller persists both.
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<(Vec<u8>, Vec<u8>)> {
        let key = self.cipher_key()?;
        let cipher = Aes256Gcm::new(key);
        let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
        let ciphertext = cipher
            .encrypt(&nonce, plaintext)
            .map_err(|e| anyhow!("AES-GCM encrypt failed: {e}"))?;
        Ok((ciphertext, nonce.to_vec()))
    }

    /// Decrypt `ciphertext` with the supplied `nonce`. Returns the
    /// plaintext bytes inside a `Zeroizing` wrapper so callers
    /// don't accidentally leave it lying in memory.
    pub fn decrypt(
        &self,
        ciphertext: &[u8],
        nonce_bytes: &[u8],
    ) -> Result<Zeroizing<Vec<u8>>> {
        if nonce_bytes.len() != 12 {
            return Err(anyhow!(
                "AES-GCM nonce must be 12 bytes, got {}",
                nonce_bytes.len()
            ));
        }
        let key = self.cipher_key()?;
        let cipher = Aes256Gcm::new(key);
        let nonce = Nonce::from_slice(nonce_bytes);
        let plaintext = cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow!("AES-GCM decrypt failed: {e}"))?;
        Ok(Zeroizing::new(plaintext))
    }

    fn cipher_key(&self) -> Result<&Key<Aes256Gcm>> {
        let inner = self
            .inner
            .as_ref()
            .ok_or_else(|| anyhow!("master key not configured (RUSCKER_MASTER_KEY)"))?;
        // Aes256Gcm::Key is a transparent newtype over [u8; 32].
        Ok(Key::<Aes256Gcm>::from_slice(inner.as_ref().as_slice()))
    }
}

/// Accept hex (64 chars), base64 (44 chars with padding), or
/// base64-no-pad (43 chars). Anything else is an error.
fn decode_key_material(raw: &str) -> Result<[u8; 32]> {
    let raw = raw.trim();
    // Hex check first — unambiguous (64 lowercase hex chars).
    if raw.len() == 64 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
        let v = hex::decode(raw).context("decode master key as hex")?;
        return v
            .try_into()
            .map_err(|_| anyhow!("hex key must decode to 32 bytes"));
    }
    // Else try base64. We attempt all four common variants
    // (standard ± padding, URL-safe ± padding) inline, because
    // base64::Engine isn't dyn-compatible (its methods are
    // generic) so we can't loop over `&dyn Engine`.
    let try_decode = |bytes: Result<Vec<u8>, _>| -> Option<[u8; 32]> {
        bytes.ok().and_then(|v| {
            (v.len() == 32).then(|| {
                let mut out = [0u8; 32];
                out.copy_from_slice(&v);
                out
            })
        })
    };
    use base64::engine::general_purpose::{
        STANDARD, STANDARD_NO_PAD, URL_SAFE, URL_SAFE_NO_PAD,
    };
    if let Some(b) = try_decode(STANDARD.decode(raw)) {
        return Ok(b);
    }
    if let Some(b) = try_decode(STANDARD_NO_PAD.decode(raw)) {
        return Ok(b);
    }
    if let Some(b) = try_decode(URL_SAFE.decode(raw)) {
        return Ok(b);
    }
    if let Some(b) = try_decode(URL_SAFE_NO_PAD.decode(raw)) {
        return Ok(b);
    }
    Err(anyhow!(
        "RUSCKER_MASTER_KEY must be 32 bytes encoded as 64-hex or 44-base64 (try `openssl rand -hex 32`)"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_key() -> MasterKey {
        // 32 bytes of zeroes — fine for tests. Real deployments
        // use `openssl rand -hex 32`.
        MasterKey::from_str("0000000000000000000000000000000000000000000000000000000000000000").unwrap()
    }

    #[test]
    fn from_env_returns_unconfigured_when_missing() {
        // Snapshot+restore env, in case CI exports it.
        let prior = std::env::var("RUSCKER_MASTER_KEY").ok();
        std::env::remove_var("RUSCKER_MASTER_KEY");
        let k = MasterKey::from_env().unwrap();
        assert!(!k.is_configured());
        if let Some(p) = prior {
            std::env::set_var("RUSCKER_MASTER_KEY", p);
        }
    }

    #[test]
    fn accepts_hex_64() {
        let k = MasterKey::from_str(&"ab".repeat(32)).unwrap();
        assert!(k.is_configured());
    }

    #[test]
    fn accepts_base64() {
        // 32 bytes of 0x42 → fixed base64
        let b64 = base64::engine::general_purpose::STANDARD.encode([0x42u8; 32]);
        assert_eq!(b64.len(), 44);
        let k = MasterKey::from_str(&b64).unwrap();
        assert!(k.is_configured());
    }

    #[test]
    fn rejects_short_input() {
        assert!(MasterKey::from_str("too-short").is_err());
    }

    #[test]
    fn encrypt_then_decrypt_roundtrip() {
        let k = fixed_key();
        let (ct, nonce) = k.encrypt(b"hunter2-the-password").unwrap();
        let pt = k.decrypt(&ct, &nonce).unwrap();
        assert_eq!(pt.as_slice(), b"hunter2-the-password");
    }

    #[test]
    fn ciphertexts_differ_per_call() {
        let k = fixed_key();
        let (ct1, _) = k.encrypt(b"same plaintext").unwrap();
        let (ct2, _) = k.encrypt(b"same plaintext").unwrap();
        assert_ne!(ct1, ct2, "fresh nonce ⇒ different ciphertext each call");
    }

    #[test]
    fn decrypt_with_wrong_nonce_fails() {
        let k = fixed_key();
        let (ct, _nonce) = k.encrypt(b"data").unwrap();
        let bad_nonce = vec![0u8; 12];
        assert!(k.decrypt(&ct, &bad_nonce).is_err());
    }

    #[test]
    fn decrypt_with_tampered_ciphertext_fails() {
        let k = fixed_key();
        let (mut ct, nonce) = k.encrypt(b"data").unwrap();
        let last = ct.len() - 1;
        ct[last] ^= 1;
        assert!(k.decrypt(&ct, &nonce).is_err());
    }

    #[test]
    fn unconfigured_key_refuses_to_encrypt() {
        let k = MasterKey::default();
        assert!(!k.is_configured());
        assert!(k.encrypt(b"hi").is_err());
    }
}
