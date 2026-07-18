//! TOTP enrollment primitives and recovery-code handling (#1005 slice 2).
//!
//! TOTP uses HMAC-SHA-1 deliberately: authenticator apps universally support
//! it, RFC 6238 defines it, and SHA-1 collision attacks do not weaken HMAC in
//! this use. Six digits, a 30-second step and one step of skew match the broad
//! authenticator-app interoperability profile.

use anyhow::{anyhow, Context, Result};
use chrono::Utc;
use qrcode::render::svg;
use qrcode::QrCode;
use ring::{digest, rand as ring_rand};
use ruscker_config::Spec;
use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};
use totp_rs::{Algorithm, Secret, TOTP};
use tower_cookies::Cookies;

pub const RECOVERY_CODE_COUNT: usize = 10;
pub const RECOVERY_CODE_LEN: usize = 10;
const RECOVERY_ALPHABET: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
pub const DEVICE_COOKIE: &str = "__ruscker_mfa_device";

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
    Ok(verify_totp_step(secret_base32, username, code)?.is_some())
}

/// Verify a TOTP against the current, previous, or next 30-second step and
/// return the exact accepted step for persistent replay prevention.
pub fn verify_totp_step(secret_base32: &str, username: &str, code: &str) -> Result<Option<i64>> {
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(None);
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .context("read system time for TOTP")?
        .as_secs();
    let current = i64::try_from(now / 30).context("TOTP step exceeds i64")?;
    let profile = totp(secret_base32, username)?;
    // Prefer the current step if a six-digit collision occurs across the skew
    // window. The stored monotonic step then rejects every older candidate.
    for offset in [0_i64, -1, 1] {
        let Some(step) = current.checked_add(offset) else {
            continue;
        };
        let Ok(timestamp) = u64::try_from(step.saturating_mul(30)) else {
            continue;
        };
        let expected = profile.generate(timestamp);
        if constant_time_equal(code.as_bytes(), expected.as_bytes()) {
            return Ok(Some(step));
        }
    }
    Ok(None)
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

/// Generate the 32-byte bearer token placed after the grant id in the cookie.
pub fn generate_device_token() -> Result<String> {
    let rng = ring_rand::SystemRandom::new();
    let mut token = [0u8; 32];
    ring_rand::SecureRandom::fill(&rng, &mut token)
        .map_err(|_| anyhow!("generate MFA device token"))?;
    Ok(hex::encode(token))
}

/// Salt and hash a high-entropy trusted-device bearer token.
pub fn hash_device_token(token: &str) -> Result<String> {
    let rng = ring_rand::SystemRandom::new();
    let mut salt = [0u8; 16];
    ring_rand::SecureRandom::fill(&rng, &mut salt)
        .map_err(|_| anyhow!("generate MFA device-token salt"))?;
    let mut input = Vec::with_capacity(salt.len() + token.len());
    input.extend_from_slice(&salt);
    input.extend_from_slice(token.as_bytes());
    let hash = digest::digest(&digest::SHA256, &input);
    Ok(format!("{}:{}", hex::encode(salt), hex::encode(hash.as_ref())))
}

/// Constant-time verification of a trusted-device token against its stored
/// salted hash string.
pub fn verify_device_token(token: &str, stored: &str) -> bool {
    let Some((salt_hex, hash_hex)) = stored.split_once(':') else {
        return false;
    };
    let (Ok(salt), Ok(expected)) = (hex::decode(salt_hex), hex::decode(hash_hex)) else {
        return false;
    };
    if salt.len() != 16 || expected.len() != digest::SHA256_OUTPUT_LEN {
        return false;
    }
    let mut input = Vec::with_capacity(salt.len() + token.len());
    input.extend_from_slice(&salt);
    input.extend_from_slice(token.as_bytes());
    let actual = digest::digest(&digest::SHA256, &input);
    constant_time_equal(actual.as_ref(), &expected)
}

/// One-way binding for the opaque admin-session bearer. The raw session id
/// never enters the grant table.
pub fn session_binding(session_id: &str) -> String {
    hex::encode(digest::digest(&digest::SHA256, session_id.as_bytes()).as_ref())
}

pub fn device_cookie_parts(value: &str) -> Option<(&str, &str)> {
    let (id, token) = value.split_once('.')?;
    if id.is_empty() || token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some((id, token))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MfaDecision {
    Satisfied,
    ChallengeRequired,
    EnrollmentRequired,
}

/// Decide whether this browser's latest device proof satisfies `spec`.
/// Database, cookie, token, factor, expiry, age, and session mismatches all
/// fail closed toward a challenge; only a confirmed-factor absence produces
/// `EnrollmentRequired`.
pub async fn evaluate(
    state: &crate::AppState,
    username: &str,
    session_id: &str,
    cookies: &Cookies,
    spec: &Spec,
) -> MfaDecision {
    if !spec.effective_require_mfa() {
        return MfaDecision::Satisfied;
    }
    let Some(db) = state.db.as_ref() else {
        return MfaDecision::ChallengeRequired;
    };
    let factor = match crate::db::mfa::fetch(db, username).await {
        Ok(Some(row)) if row.confirmed_at.is_some() => row,
        Ok(_) => return MfaDecision::EnrollmentRequired,
        Err(err) => {
            tracing::warn!(error = ?err, %username, "MFA decision factor fetch failed");
            return MfaDecision::ChallengeRequired;
        }
    };
    let Some(cookie) = cookies.get(DEVICE_COOKIE) else {
        return MfaDecision::ChallengeRequired;
    };
    let Some((id, token)) = device_cookie_parts(cookie.value()) else {
        return MfaDecision::ChallengeRequired;
    };
    let grant = match crate::db::mfa_grants::fetch_valid(db, id).await {
        Ok(Some(grant)) => grant,
        Ok(None) => return MfaDecision::ChallengeRequired,
        Err(err) => {
            tracing::warn!(error = ?err, %username, "MFA decision grant fetch failed");
            return MfaDecision::ChallengeRequired;
        }
    };
    let now = Utc::now();
    if !verify_device_token(token, &grant.token_hash)
        || grant.username != crate::db::users::normalize_username(username)
        || grant.expires_at <= now
        || Some(grant.factor_confirmed_at) != factor.confirmed_at
        // Epoch binding (codex review, #1005): a grant that slipped past a
        // racing revocation on pg (READ COMMITTED lets the conditional
        // INSERT…SELECT read the pre-revocation epoch) carries the OLD
        // epoch — the live factor row has the bumped one, so the grant is
        // simply never accepted. Read-time validation makes issuance-time
        // races harmless.
        || grant.security_epoch != factor.security_epoch
    {
        return MfaDecision::ChallengeRequired;
    }

    let validity_days = spec.effective_mfa_validity_days();
    if validity_days == 0 {
        return if constant_time_equal(
            grant.session_binding.as_bytes(),
            session_binding(session_id).as_bytes(),
        ) {
            MfaDecision::Satisfied
        } else {
            MfaDecision::ChallengeRequired
        };
    }
    let age = now.signed_duration_since(grant.mfa_verified_at);
    if age < chrono::Duration::zero()
        || age > chrono::Duration::days(i64::from(validity_days))
    {
        MfaDecision::ChallengeRequired
    } else {
        MfaDecision::Satisfied
    }
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

    /// Atomically reserve one attempt: prune the window, and either count
    /// this attempt and allow it, or refuse. Reserving BEFORE the awaited
    /// argon2/TOTP verification closes the TOCTOU where N concurrent
    /// requests all pass a read-only check ahead of any failure being
    /// recorded (codex review, #1005) — the reservation itself is the
    /// count, under one mutex critical section. A success then clears the
    /// whole entry, so legitimate flows never accumulate.
    pub fn try_reserve(&self, username: &str) -> bool {
        let now = Instant::now();
        let mut all = self.failures.lock().unwrap();
        let attempts = all.entry(username.to_string()).or_default();
        while attempts
            .front()
            .is_some_and(|at| now.duration_since(*at) > self.window)
        {
            attempts.pop_front();
        }
        if attempts.len() >= self.max {
            return false;
        }
        attempts.push_back(now);
        true
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

/// Challenge attempts have an independent budget from enrollment confirms,
/// so setup typos cannot lock an already-enrolled user's app proof flow.
pub static CHALLENGE_LIMITER: std::sync::LazyLock<ConfirmRateLimiter> =
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
