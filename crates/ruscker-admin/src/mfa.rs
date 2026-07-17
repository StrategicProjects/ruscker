//! TOTP enrollment primitives and recovery-code handling (#1005 slice 2).
//!
//! TOTP uses HMAC-SHA-1 deliberately: authenticator apps universally support
//! it, RFC 6238 defines it, and SHA-1 collision attacks do not weaken HMAC in
//! this use. Six digits, a 30-second step and one step of skew match the broad
//! authenticator-app interoperability profile.

use anyhow::{anyhow, Context, Result};
use qrcode::render::svg;
use qrcode::QrCode;
use ring::{digest, rand as ring_rand};
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use totp_rs::{Algorithm, Secret, TOTP};

pub const RECOVERY_CODE_COUNT: usize = 10;
pub const RECOVERY_CODE_LEN: usize = 10;
const RECOVERY_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Material shown on the setup screen and persisted only after encryption.
pub struct Enrollment {
    pub secret_base32: String,
    pub qr_svg: String,
}

/// Build the standard Ruscker TOTP profile from a base32 secret.
pub fn totp(secret_base32: &str, username: &str) -> Result<TOTP> {
    let secret = Secret::Encoded(secret_base32.to_string())
        .to_bytes()
        .map_err(|e| anyhow!("decode TOTP secret: {e}"))?;
    TOTP::new(
        Algorithm::SHA1,
        6,
        1,
        30,
        secret,
        Some("Ruscker".to_string()),
        username.to_string(),
    )
    .map_err(|e| anyhow!("build TOTP profile: {e}"))
}

/// Generate a fresh 160-bit CSPRNG secret and render its otpauth URL locally
/// as SVG. No secret-bearing request leaves the Ruscker process.
pub fn begin(username: &str) -> Result<Enrollment> {
    let secret_base32 = Secret::generate_secret().to_encoded().to_string();
    render_enrollment(&secret_base32, username)
}

/// Re-render an existing pending enrollment after a mistyped confirmation
/// code. This never rotates the secret: the encrypted DB row is authoritative.
pub fn render_enrollment(secret_base32: &str, username: &str) -> Result<Enrollment> {
    let profile = totp(secret_base32, username)?;
    let qr = QrCode::new(profile.get_url().as_bytes()).context("encode TOTP QR")?;
    let qr_svg = qr
        .render::<svg::Color>()
        .min_dimensions(240, 240)
        .dark_color(svg::Color("#111827"))
        .light_color(svg::Color("#ffffff"))
        .build();
    Ok(Enrollment {
        secret_base32: secret_base32.to_string(),
        qr_svg,
    })
}

pub fn verify_totp(secret_base32: &str, username: &str, code: &str) -> Result<bool> {
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(false);
    }
    totp(secret_base32, username)?
        .check_current(code)
        .context("read system time for TOTP")
}

/// Generate ten high-entropy, human-readable one-time recovery codes.
/// The 32-symbol alphabet maps exactly to five random bits, avoiding modulo
/// bias while omitting ambiguous `0/O/1/I` glyphs.
pub fn generate_recovery_codes() -> Result<Vec<String>> {
    let rng = ring_rand::SystemRandom::new();
    let mut random = [0u8; RECOVERY_CODE_COUNT * RECOVERY_CODE_LEN];
    ring_rand::SecureRandom::fill(&rng, &mut random)
        .map_err(|_| anyhow!("generate recovery codes"))?;
    Ok(random
        .chunks_exact(RECOVERY_CODE_LEN)
        .map(|chunk| {
            chunk
                .iter()
                .map(|b| RECOVERY_ALPHABET[(b & 31) as usize] as char)
                .collect()
        })
        .collect())
}

/// Hash a high-entropy recovery code with a fresh random salt.
///
/// These server-generated codes carry roughly 50 bits of entropy and are
/// rate-limited when consumed. Salted SHA-256 is therefore sufficient and
/// keeps each check O(1); password-oriented Argon2 is deliberately reserved
/// for low-entropy, user-chosen passwords.
pub fn hash_recovery_code(code: &str) -> Result<String> {
    let rng = ring_rand::SystemRandom::new();
    let mut salt = [0u8; 16];
    ring_rand::SecureRandom::fill(&rng, &mut salt)
        .map_err(|_| anyhow!("generate recovery-code salt"))?;
    let normalized = code.trim().to_ascii_uppercase();
    let mut input = Vec::with_capacity(salt.len() + normalized.len());
    input.extend_from_slice(&salt);
    input.extend_from_slice(normalized.as_bytes());
    let hash = digest::digest(&digest::SHA256, &input);
    Ok(format!(
        "{}:{}",
        hex::encode(salt),
        hex::encode(hash.as_ref())
    ))
}

/// Constant-time verification against a stored salted SHA-256 value.
pub fn verify_recovery_code(code: &str, stored: &str) -> bool {
    let Some((salt_hex, hash_hex)) = stored.split_once(':') else {
        return false;
    };
    let (Ok(salt), Ok(expected)) = (hex::decode(salt_hex), hex::decode(hash_hex)) else {
        return false;
    };
    if salt.len() != 16 || expected.len() != digest::SHA256_OUTPUT_LEN {
        return false;
    }
    let normalized = code.trim().to_ascii_uppercase();
    let mut input = Vec::with_capacity(salt.len() + normalized.len());
    input.extend_from_slice(&salt);
    input.extend_from_slice(normalized.as_bytes());
    let actual = digest::digest(&digest::SHA256, &input);
    constant_time_equal(actual.as_ref(), &expected)
}

#[allow(deprecated)]
fn constant_time_equal(actual: &[u8], expected: &[u8]) -> bool {
    ring::constant_time::verify_slices_are_equal(actual, expected).is_ok()
}

/// Per-username confirmation limiter. A correct code clears that username's
/// failures; five wrong codes inside 60 seconds produce a friendly 429.
#[derive(Debug)]
pub struct ConfirmRateLimiter {
    failures: std::sync::Mutex<HashMap<String, VecDeque<Instant>>>,
    max: usize,
    window: Duration,
}

impl ConfirmRateLimiter {
    pub fn new(max: usize, window: Duration) -> Self {
        Self {
            failures: std::sync::Mutex::new(HashMap::new()),
            max,
            window,
        }
    }

    pub fn allow(&self, username: &str) -> bool {
        let now = Instant::now();
        let mut all = self.failures.lock().unwrap();
        let failures = all.entry(username.to_string()).or_default();
        while failures
            .front()
            .is_some_and(|at| now.duration_since(*at) > self.window)
        {
            failures.pop_front();
        }
        failures.len() < self.max
    }

    pub fn record_failure(&self, username: &str) {
        self.failures
            .lock()
            .unwrap()
            .entry(username.to_string())
            .or_default()
            .push_back(Instant::now());
    }

    pub fn record_success(&self, username: &str) {
        self.failures.lock().unwrap().remove(username);
    }
}

pub static CONFIRM_LIMITER: std::sync::LazyLock<ConfirmRateLimiter> =
    std::sync::LazyLock::new(|| ConfirmRateLimiter::new(5, Duration::from_secs(60)));

/// Per-username limiter for the password re-authentication at /start
/// (codex review, #1005): a stolen session cookie must not turn the
/// enrollment page into an unlimited password oracle (each guess runs a
/// deliberately-expensive argon2 verify). Same shape as the code limiter.
pub static REAUTH_LIMITER: std::sync::LazyLock<ConfirmRateLimiter> =
    std::sync::LazyLock::new(|| ConfirmRateLimiter::new(5, Duration::from_secs(60)));

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::MasterKey;

    #[test]
    fn secret_encrypt_decrypt_round_trip() {
        let key = MasterKey::parse(&"ab".repeat(32)).unwrap();
        let enrollment = begin("alice").unwrap();
        let (ciphertext, nonce) = key.encrypt(enrollment.secret_base32.as_bytes()).unwrap();
        assert_ne!(ciphertext, enrollment.secret_base32.as_bytes());
        let plaintext = key.decrypt(&ciphertext, &nonce).unwrap();
        assert_eq!(plaintext.as_slice(), enrollment.secret_base32.as_bytes());
    }

    #[test]
    fn totp_accepts_current_and_adjacent_steps_but_rejects_garbage() {
        let secret = Secret::generate_secret().to_encoded().to_string();
        let profile = totp(&secret, "alice").unwrap();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs();
        assert!(profile.check(&profile.generate(now), now));
        assert!(profile.check(&profile.generate(now - 30), now));
        assert!(profile.check(&profile.generate(now + 30), now));
        assert!(!profile.check("garbage", now));
        assert!(!profile.check("0000000", now));
    }

    #[test]
    fn recovery_hash_is_salted_and_verifies_constant_time() {
        let first = hash_recovery_code("ABCD234567").unwrap();
        let second = hash_recovery_code("ABCD234567").unwrap();
        assert_ne!(first, second);
        assert!(verify_recovery_code("abcd234567", &first));
        assert!(!verify_recovery_code("ABCD234568", &first));
    }
}
