//! Sticky-session cookie: signed payload that binds a visitor to
//! one replica for the lifetime of their Shiny session.
//!
//! ## Why sticky
//!
//! Shiny/Streamlit/Dash apps keep per-session state inside the
//! container. Once user X has connected to replica R1, every HTTP
//! request and every WebSocket frame must reach R1 — round-robin
//! to R2 would land in a process that has never heard of X.
//!
//! ## Wire format
//!
//! ```text
//!   <payload_bytes> | <16-byte HMAC-SHA256 truncated>
//!   ↓ all of that
//!   base64url (no padding)
//! ```
//!
//! `payload_bytes` is JSON: `{"s":"<spec>","r":"<uuid>","i":"<uuid>"}`.
//! Truncated HMAC is fine for cookie integrity — we're not
//! protecting confidentiality (the cookie has no secrets), only
//! that the server signed it. 16 bytes = 128 bits of attacker
//! work to forge.
//!
//! ## Key lifecycle
//!
//! [`CookieKey`] reads from the `RUSCKER_COOKIE_KEY` env var (hex
//! or base64). If absent, it auto-generates a 32-byte random key
//! at startup — **per-process, in-memory only**. Restarting the
//! server with no key set invalidates every prior session, which
//! is the correct behavior for the default ("if you didn't pin
//! it, you don't care if it rotates").

use anyhow::{anyhow, Context, Result};
use base64::Engine;
use ring::hmac;
use ruscker_core::ReplicaId;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use uuid::Uuid;
use zeroize::Zeroizing;

/// Cookie name set on responses and read on requests. The leading
/// `__` follows the convention for proxy-controlled cookies that
/// the app must not stomp on.
pub const COOKIE_NAME: &str = "__ruscker_session";

/// Bytes of HMAC kept in the cookie. Trade-off: 16 B = 128 bits
/// of forgery resistance, ~22 base64 chars vs. 44 for the full
/// SHA-256. 128 bits is way past anything an attacker can brute
/// in a session window.
const TAG_LEN: usize = 16;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StickySession {
    /// Per-visitor identifier (so the server can purge a single
    /// session without affecting siblings on the same replica).
    #[serde(rename = "i")]
    pub session_id: Uuid,
    /// Which spec the visitor opened. Used to defend against a
    /// stale cookie pointing into a different app after
    /// rename/delete.
    #[serde(rename = "s")]
    pub spec_id: String,
    /// The replica the visitor is bound to. Looked up against
    /// [`ruscker_core::ReplicaRegistry`] on every request.
    #[serde(rename = "r")]
    pub replica_id: ReplicaId,
}

impl StickySession {
    pub fn new(spec_id: impl Into<String>, replica_id: ReplicaId) -> Self {
        Self {
            session_id: Uuid::new_v4(),
            spec_id: spec_id.into(),
            replica_id,
        }
    }
}

/// HMAC key for signing/verifying [`StickySession`] cookies. Held
/// inside an `Arc<Zeroizing<...>>` so cloning `AppState` per
/// request doesn't copy 32 bytes of key material, and the last
/// drop wipes the buffer.
#[derive(Clone, Debug)]
pub struct CookieKey {
    inner: Arc<Zeroizing<[u8; 32]>>,
}

impl CookieKey {
    /// 32 bytes from `RUSCKER_COOKIE_KEY` (hex or base64), else
    /// freshly randomized. Auto-generated keys log a
    /// `tracing::info!` so operators see "your sticky sessions
    /// will reset on restart" in the boot logs.
    pub fn from_env_or_random() -> Result<Self> {
        match std::env::var("RUSCKER_COOKIE_KEY").ok().filter(|s| !s.is_empty()) {
            Some(raw) => Self::parse(&raw),
            None => {
                tracing::info!(
                    "RUSCKER_COOKIE_KEY not set; generated a random key. \
                     Set it explicitly to keep sessions alive across restarts."
                );
                Ok(Self::random())
            }
        }
    }

    /// Parse a cookie-key string (hex or base64) into the 32-byte key.
    /// Named `parse` instead of `from_str` so it doesn't shadow the
    /// [`std::str::FromStr`] convention — `T::from_str(s)` on an
    /// inherent impl gets flagged by clippy because callers may expect
    /// the trait semantics.
    pub fn parse(raw: &str) -> Result<Self> {
        let raw = raw.trim();
        let bytes = if raw.len() == 64 && raw.chars().all(|c| c.is_ascii_hexdigit()) {
            let v = decode_hex(raw)?;
            v.try_into()
                .map_err(|_| anyhow!("hex must decode to 32 bytes"))?
        } else if let Ok(b) = base64::engine::general_purpose::STANDARD.decode(raw) {
            b.try_into()
                .map_err(|_| anyhow!("base64 must decode to 32 bytes"))?
        } else if let Ok(b) = base64::engine::general_purpose::STANDARD_NO_PAD.decode(raw) {
            b.try_into()
                .map_err(|_| anyhow!("base64 must decode to 32 bytes"))?
        } else {
            return Err(anyhow!(
                "RUSCKER_COOKIE_KEY must be 32 bytes encoded as hex (64ch) or base64 (44ch)"
            ));
        };
        Ok(Self {
            inner: Arc::new(Zeroizing::new(bytes)),
        })
    }

    pub fn random() -> Self {
        use ring::rand::{SecureRandom, SystemRandom};
        let mut bytes = [0u8; 32];
        SystemRandom::new()
            .fill(&mut bytes)
            .expect("OS RNG must work");
        Self {
            inner: Arc::new(Zeroizing::new(bytes)),
        }
    }

    fn hmac_key(&self) -> hmac::Key {
        hmac::Key::new(hmac::HMAC_SHA256, self.inner.as_ref().as_slice())
    }
}

impl Default for CookieKey {
    fn default() -> Self {
        Self::random()
    }
}

/// Serialize + sign + base64-encode a session for use as the
/// cookie value.
pub fn encode(key: &CookieKey, session: &StickySession) -> Result<String> {
    let payload = serde_json::to_vec(session).context("serialize sticky session")?;
    let tag = hmac::sign(&key.hmac_key(), &payload);
    let truncated = &tag.as_ref()[..TAG_LEN];
    let mut wire = payload;
    wire.extend_from_slice(truncated);
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(&wire))
}

/// Verify a cookie value, returning the embedded session on
/// success. Errors are intentionally vague — leaking "bad
/// signature" vs "bad json" to the caller (who echoes them to
/// the wire) helps no one.
pub fn decode(key: &CookieKey, cookie_value: &str) -> Result<StickySession> {
    let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(cookie_value)
        .map_err(|_| anyhow!("invalid cookie encoding"))?;
    if raw.len() < TAG_LEN + 2 {
        return Err(anyhow!("cookie too short"));
    }
    let (payload, tag) = raw.split_at(raw.len() - TAG_LEN);

    // Recompute the full HMAC and compare its prefix in constant
    // time. ring's `verify` requires the full 32-byte tag, so we
    // reconstruct by signing and constant-time-comparing only the
    // truncated bytes ourselves.
    let expected = hmac::sign(&key.hmac_key(), payload);
    if !ct_eq(&expected.as_ref()[..TAG_LEN], tag) {
        return Err(anyhow!("bad signature"));
    }
    serde_json::from_slice::<StickySession>(payload)
        .map_err(|_| anyhow!("malformed payload"))
}

fn ct_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn decode_hex(s: &str) -> Result<Vec<u8>> {
    if s.len() % 2 != 0 {
        return Err(anyhow!("hex length must be even"));
    }
    let mut out = Vec::with_capacity(s.len() / 2);
    for i in (0..s.len()).step_by(2) {
        let byte = u8::from_str_radix(&s[i..i + 2], 16).map_err(|e| anyhow!("hex parse: {e}"))?;
        out.push(byte);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixed_key() -> CookieKey {
        CookieKey::parse(&"ab".repeat(32)).unwrap()
    }

    fn sample() -> StickySession {
        StickySession {
            session_id: Uuid::parse_str("11111111-1111-1111-1111-111111111111").unwrap(),
            spec_id: "sales-dashboard".into(),
            replica_id: ReplicaId(
                Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap(),
            ),
        }
    }

    #[test]
    fn round_trip() {
        let k = fixed_key();
        let s = sample();
        let cookie = encode(&k, &s).unwrap();
        let decoded = decode(&k, &cookie).unwrap();
        assert_eq!(decoded, s);
    }

    #[test]
    fn cookie_is_url_safe_no_padding() {
        let cookie = encode(&fixed_key(), &sample()).unwrap();
        assert!(!cookie.contains('='));
        assert!(!cookie.contains('+'));
        assert!(!cookie.contains('/'));
    }

    #[test]
    fn tampered_payload_rejected() {
        let k = fixed_key();
        let cookie = encode(&k, &sample()).unwrap();
        // Flip one char in the middle of the payload (before the
        // tag). The base64 will still decode but the JSON / tag
        // won't match.
        let mut bytes = cookie.into_bytes();
        let mid = bytes.len() / 4;
        bytes[mid] ^= 0x01;
        let cookie = String::from_utf8(bytes).unwrap();
        assert!(decode(&k, &cookie).is_err());
    }

    #[test]
    fn different_key_rejects_cookie() {
        let cookie = encode(&fixed_key(), &sample()).unwrap();
        let other = CookieKey::parse(&"cd".repeat(32)).unwrap();
        assert!(decode(&other, &cookie).is_err());
    }

    #[test]
    fn nonsense_input_rejected() {
        let k = fixed_key();
        assert!(decode(&k, "").is_err());
        assert!(decode(&k, "not-base64-!@#$").is_err());
        assert!(decode(&k, "a").is_err());
    }

    #[test]
    fn random_keys_differ() {
        let a = CookieKey::random();
        let b = CookieKey::random();
        // Encode the same session under each, cookies must differ.
        let s = sample();
        let ca = encode(&a, &s).unwrap();
        let cb = encode(&b, &s).unwrap();
        assert_ne!(ca, cb);
    }

    #[test]
    fn parses_hex_and_base64() {
        let hex = "ab".repeat(32);
        let bytes = vec![0xabu8; 32];
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let kh = CookieKey::parse(&hex).unwrap();
        let kb = CookieKey::parse(&b64).unwrap();
        let s = sample();
        assert_eq!(encode(&kh, &s).unwrap(), encode(&kb, &s).unwrap());
    }
}
