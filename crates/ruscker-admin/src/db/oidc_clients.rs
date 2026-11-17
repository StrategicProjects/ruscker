//! OIDC client registry — the systems allowed to "Sign in with Ruscker".
//!
//! Slice 1 of the OIDC provider (see `docs/adr/0005-oidc-provider.md`):
//! this module only stores and validates clients. The protocol endpoints
//! (`/oauth/authorize`, `/oauth/token`, …) arrive in later slices and
//! read the registry through [`fetch`].
//!
//! Three rules shape everything here:
//!
//! * **The secret is never stored.** It is generated here, returned ONCE
//!   to the caller (who shows it to the admin) and kept only as a salted
//!   SHA-256. Losing it means rotating it, never recovering it.
//! * **Redirect URIs match exactly.** No prefix, no wildcard — a loose
//!   match is the classic way an authorization code leaks to an attacker.
//! * **Access fails closed.** A client restricted to groups never becomes
//!   open by accident: deleting its last group DISABLES it
//!   ([`rewrite_group`]) instead of emptying the list.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use ring::rand as ring_rand;

use crate::db::users::{join_groups, parse_groups};
use crate::db::ConfigDb;

/// Upper bound on registered redirect URIs per client. A real client has
/// one or two (production + local development); a long list is a smell.
pub const MAX_REDIRECT_URIS: usize = 10;

/// One registered client, as stored. The secret hash is private to this
/// module — callers verify through [`OidcClient::verify_secret`].
#[derive(Debug, Clone)]
pub struct OidcClient {
    pub client_id: String,
    pub name: String,
    secret_hash: String,
    /// Bumped on every rotation — the compare-and-set token of
    /// [`rotate_secret`].
    pub secret_version: i64,
    pub redirect_uris: Vec<String>,
    /// Empty = any signed-in user. Otherwise the user must belong to at
    /// least one of these groups (same matching as a spec's
    /// `access-groups`: exact, case-sensitive).
    pub access_groups: Vec<String>,
    pub require_mfa: bool,
    pub enabled: bool,
    /// Media-library filename shown on the sign-in page when a user
    /// arrives from this system.
    pub logo: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl OidcClient {
    /// Whether a user with `user_groups` may sign in through this client.
    pub fn allows_groups(&self, user_groups: &[String]) -> bool {
        self.access_groups.is_empty()
            || self
                .access_groups
                .iter()
                .any(|g| user_groups.iter().any(|u| u == g))
    }

    /// Exact-match check of a redirect URI presented by the client.
    pub fn redirect_uri_registered(&self, uri: &str) -> bool {
        self.redirect_uris.iter().any(|r| r == uri)
    }

    /// Constant-time verification of a presented client secret.
    pub fn verify_secret(&self, presented: &str) -> bool {
        // Same primitive as the MFA trusted-device token: a salted
        // SHA-256 over a high-entropy bearer, compared in constant time.
        crate::mfa::verify_device_token(presented, &self.secret_hash)
    }
}

/// The admin-editable part of a client (everything but id and secret).
#[derive(Debug, Clone)]
pub struct ClientSettings {
    pub name: String,
    pub redirect_uris: Vec<String>,
    pub access_groups: Vec<String>,
    pub require_mfa: bool,
    pub enabled: bool,
    /// Media-library filename, or `None`.
    pub logo: Option<String>,
}

/// A client id is safe iff it is 3–64 chars of lowercase letters, digits,
/// `-`, `_` or `.`. It appears un-encoded in admin URLs and in every
/// protocol request, so the charset is deliberately narrow.
pub fn is_valid_client_id(id: &str) -> bool {
    (3..=64).contains(&id.len())
        && id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '-' | '_' | '.'))
}

/// Why a redirect URI was refused. The admin form maps each variant to a
/// localized message; keeping them distinct makes the message actionable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RedirectUriError {
    /// Not an absolute URL at all.
    NotAbsolute,
    /// Plain `http` is only allowed for a loopback host (development).
    InsecureScheme,
    /// A scheme other than http/https (custom schemes are for native
    /// apps, which this provider does not serve).
    UnsupportedScheme,
    /// `#fragment` — forbidden by OAuth 2.0 (RFC 6749 §3.1.2).
    HasFragment,
    /// `user:pass@host` — never legitimate in a redirect URI.
    HasUserinfo,
    /// `*` anywhere: wildcards are not supported, matching is exact.
    HasWildcard,
    /// Whitespace, a control character, a backslash or a non-ASCII
    /// character. URL parsers silently "repair" these, each in its own
    /// way, and a URI two parsers read differently is how a code ends
    /// up at the wrong host.
    InvalidCharacters,
}

/// Validate one redirect URI. Returns it unchanged on success — the URI
/// is stored and later compared BYTE FOR BYTE with what the client sends,
/// so we must not normalize it here.
pub fn validate_redirect_uri(raw: &str) -> Result<&str, RedirectUriError> {
    // First, the raw bytes — BEFORE any parser gets to repair them. The
    // WHATWG parser behind `url` strips tabs and newlines, trims leading
    // control characters and reads `\` as `/`; an RFC 3986 parser in a
    // client library does none of that. Since the stored string is what
    // later gets compared and sent back in a `Location` header, only
    // printable ASCII without a backslash is accepted (an international
    // host name must be registered in its punycode form).
    if !raw.bytes().all(|b| b.is_ascii_graphic() && b != b'\\') {
        return Err(RedirectUriError::InvalidCharacters);
    }
    if raw.contains('*') {
        return Err(RedirectUriError::HasWildcard);
    }
    let url = url::Url::parse(raw).map_err(|_| RedirectUriError::NotAbsolute)?;
    if url.fragment().is_some() {
        return Err(RedirectUriError::HasFragment);
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(RedirectUriError::HasUserinfo);
    }
    // The parser also accepts `https:host`, `https:/host` and an
    // upper-case scheme; require the canonical `scheme://` spelling so
    // the stored string means the same thing to every reader.
    let canonical = raw.starts_with("https://") || raw.starts_with("http://");
    match url.scheme() {
        "https" | "http" if !canonical => Err(RedirectUriError::NotAbsolute),
        "https" => Ok(raw),
        "http" => {
            let loopback = match url.host() {
                Some(url::Host::Domain(d)) => d == "localhost",
                Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
                Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
                None => false,
            };
            if loopback {
                Ok(raw)
            } else {
                Err(RedirectUriError::InsecureScheme)
            }
        }
        _ => Err(RedirectUriError::UnsupportedScheme),
    }
}

/// Split the admin form's textarea (one URI per line) into a clean list:
/// trimmed, blank lines dropped, duplicates removed (first wins).
pub fn parse_redirect_uris(text: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let t = line.trim();
        if !t.is_empty() && !out.iter().any(|u| u == t) {
            out.push(t.to_string());
        }
    }
    out
}

/// Generate a fresh client secret: 32 random bytes, hex-encoded.
fn generate_secret() -> Result<String> {
    let rng = ring_rand::SystemRandom::new();
    let mut secret = [0u8; 32];
    ring_rand::SecureRandom::fill(&rng, &mut secret)
        .map_err(|_| anyhow!("generate OIDC client secret"))?;
    Ok(hex::encode(secret))
}

/// Storage row. Booleans decode from INTEGER (SQLite) and BOOLEAN (pg).
type Row = (
    String,
    String,
    String,
    i64,
    String,
    String,
    bool,
    bool,
    Option<String>,
    DateTime<Utc>,
    DateTime<Utc>,
);

/// Column list shared by every read, so [`from_row`] has one shape to
/// decode. A macro (not a `const`) because `concat!` needs a literal.
macro_rules! select {
    ($tail:literal) => {
        concat!(
            "SELECT client_id, name, secret_hash, secret_version, redirect_uris, access_groups,
                    require_mfa, enabled, logo, created_at, updated_at
               FROM oidc_clients ",
            $tail
        )
    };
}

fn from_row(r: Row) -> OidcClient {
    OidcClient {
        client_id: r.0,
        name: r.1,
        secret_hash: r.2,
        secret_version: r.3,
        redirect_uris: parse_redirect_uris(&r.4),
        access_groups: parse_groups(&r.5),
        require_mfa: r.6,
        enabled: r.7,
        logo: r.8,
        created_at: r.9,
        updated_at: r.10,
    }
}

/// Metadata-only audit diff — never the secret or its hash.
fn audit_diff(s: &ClientSettings) -> Result<String> {
    Ok(serde_json::to_string(&serde_json::json!({
        "name": s.name,
        "redirect_uris": s.redirect_uris,
        "access_groups": s.access_groups,
        "require_mfa": s.require_mfa,
        "enabled": s.enabled,
        "logo": s.logo,
    }))?)
}

/// Register a new client. Returns the freshly generated secret — the ONLY
/// time it exists in cleartext — or `None` when `client_id` is taken
/// (nothing is changed; an existing client's secret is never replaced by
/// a create).
pub async fn create(
    db: &ConfigDb,
    client_id: &str,
    settings: &ClientSettings,
    actor: Option<&str>,
) -> Result<Option<String>> {
    let secret = generate_secret()?;
    let secret_hash = crate::mfa::hash_device_token(&secret)?;
    let now = Utc::now();
    let uris = settings.redirect_uris.join("\n");
    let groups = join_groups(&settings.access_groups);
    let diff = audit_diff(settings)?;
    let target = format!("oidc-client:{client_id}");

    let inserted = match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin oidc client tx")?;
            let n = sqlx::query(
                "INSERT INTO oidc_clients
                   (client_id, name, secret_hash, redirect_uris, access_groups,
                    require_mfa, enabled, logo, created_at, updated_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
                 ON CONFLICT(client_id) DO NOTHING",
            )
            .bind(client_id)
            .bind(&settings.name)
            .bind(&secret_hash)
            .bind(&uris)
            .bind(&groups)
            .bind(settings.require_mfa)
            .bind(settings.enabled)
            .bind(&settings.logo)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("insert oidc client {client_id}"))?
            .rows_affected();
            if n == 1 {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES (?, 'oidc-client.create', ?, ?, ?)",
                )
                .bind(actor)
                .bind(&target)
                .bind(&diff)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit oidc client create")?;
            }
            tx.commit().await.context("commit oidc client tx")?;
            n == 1
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin oidc client tx")?;
            let n = sqlx::query(
                "INSERT INTO oidc_clients
                   (client_id, name, secret_hash, redirect_uris, access_groups,
                    require_mfa, enabled, logo, created_at, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
                 ON CONFLICT(client_id) DO NOTHING",
            )
            .bind(client_id)
            .bind(&settings.name)
            .bind(&secret_hash)
            .bind(&uris)
            .bind(&groups)
            .bind(settings.require_mfa)
            .bind(settings.enabled)
            .bind(&settings.logo)
            .bind(now)
            .bind(now)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("insert oidc client {client_id}"))?
            .rows_affected();
            if n == 1 {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES ($1, 'oidc-client.create', $2, $3, $4)",
                )
                .bind(actor)
                .bind(&target)
                .bind(&diff)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit oidc client create")?;
            }
            tx.commit().await.context("commit oidc client tx")?;
            n == 1
        }
    };
    Ok(inserted.then_some(secret))
}

/// Replace a client's editable settings. The secret is untouched (use
/// [`rotate_secret`]). Returns `false` when the client does not exist.
pub async fn update(
    db: &ConfigDb,
    client_id: &str,
    settings: &ClientSettings,
    actor: Option<&str>,
) -> Result<bool> {
    let now = Utc::now();
    let uris = settings.redirect_uris.join("\n");
    let groups = join_groups(&settings.access_groups);
    let diff = audit_diff(settings)?;
    let target = format!("oidc-client:{client_id}");

    let updated = match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin oidc client tx")?;
            let n = sqlx::query(
                "UPDATE oidc_clients
                    SET name = ?, redirect_uris = ?, access_groups = ?,
                        require_mfa = ?, enabled = ?, logo = ?, updated_at = ?
                  WHERE client_id = ?",
            )
            .bind(&settings.name)
            .bind(&uris)
            .bind(&groups)
            .bind(settings.require_mfa)
            .bind(settings.enabled)
            .bind(&settings.logo)
            .bind(now)
            .bind(client_id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("update oidc client {client_id}"))?
            .rows_affected();
            if n == 1 {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES (?, 'oidc-client.update', ?, ?, ?)",
                )
                .bind(actor)
                .bind(&target)
                .bind(&diff)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit oidc client update")?;
            }
            tx.commit().await.context("commit oidc client tx")?;
            n == 1
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin oidc client tx")?;
            let n = sqlx::query(
                "UPDATE oidc_clients
                    SET name = $1, redirect_uris = $2, access_groups = $3,
                        require_mfa = $4, enabled = $5, logo = $6, updated_at = $7
                  WHERE client_id = $8",
            )
            .bind(&settings.name)
            .bind(&uris)
            .bind(&groups)
            .bind(settings.require_mfa)
            .bind(settings.enabled)
            .bind(&settings.logo)
            .bind(now)
            .bind(client_id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("update oidc client {client_id}"))?
            .rows_affected();
            if n == 1 {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES ($1, 'oidc-client.update', $2, $3, $4)",
                )
                .bind(actor)
                .bind(&target)
                .bind(&diff)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit oidc client update")?;
            }
            tx.commit().await.context("commit oidc client tx")?;
            n == 1
        }
    };
    Ok(updated)
}

/// What [`rotate_secret`] did.
#[derive(Debug)]
pub enum RotateOutcome {
    /// Rotated; carries the new secret (shown once).
    Rotated(String),
    /// The client exists but `expected_version` is no longer current:
    /// the secret was already rotated since the form was rendered.
    /// Nothing changed.
    Stale,
    /// No such client.
    Missing,
}

/// Replace the client secret with a fresh one, invalidating the old one
/// immediately.
///
/// Compare-and-set on `secret_version`: the caller passes the version
/// the admin was looking at. A re-submitted request (F5 on the result
/// page, a double click, two tabs) carries the OLD version and is
/// refused as [`RotateOutcome::Stale`] — otherwise it would silently
/// kill the secret the admin just copied into the client system.
pub async fn rotate_secret(
    db: &ConfigDb,
    client_id: &str,
    expected_version: i64,
    actor: Option<&str>,
) -> Result<RotateOutcome> {
    let secret = generate_secret()?;
    let secret_hash = crate::mfa::hash_device_token(&secret)?;
    let now = Utc::now();
    let target = format!("oidc-client:{client_id}");

    let rotated = match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin oidc client tx")?;
            let n = sqlx::query(
                "UPDATE oidc_clients
                    SET secret_hash = ?, secret_version = secret_version + 1, updated_at = ?
                  WHERE client_id = ? AND secret_version = ?",
            )
            .bind(&secret_hash)
            .bind(now)
            .bind(client_id)
            .bind(expected_version)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("rotate oidc client secret {client_id}"))?
            .rows_affected();
            if n == 1 {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES (?, 'oidc-client.rotate-secret', ?, NULL, ?)",
                )
                .bind(actor)
                .bind(&target)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit oidc client rotate")?;
            }
            tx.commit().await.context("commit oidc client tx")?;
            n == 1
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin oidc client tx")?;
            let n = sqlx::query(
                "UPDATE oidc_clients
                    SET secret_hash = $1, secret_version = secret_version + 1, updated_at = $2
                  WHERE client_id = $3 AND secret_version = $4",
            )
            .bind(&secret_hash)
            .bind(now)
            .bind(client_id)
            .bind(expected_version)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("rotate oidc client secret {client_id}"))?
            .rows_affected();
            if n == 1 {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES ($1, 'oidc-client.rotate-secret', $2, NULL, $3)",
                )
                .bind(actor)
                .bind(&target)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit oidc client rotate")?;
            }
            tx.commit().await.context("commit oidc client tx")?;
            n == 1
        }
    };
    if rotated {
        return Ok(RotateOutcome::Rotated(secret));
    }
    // Nothing matched: tell "wrong version" from "no such client".
    Ok(match fetch(db, client_id).await? {
        Some(_) => RotateOutcome::Stale,
        None => RotateOutcome::Missing,
    })
}

/// Remove a client. Returns `false` when it did not exist.
pub async fn delete(db: &ConfigDb, client_id: &str, actor: Option<&str>) -> Result<bool> {
    let now = Utc::now();
    let target = format!("oidc-client:{client_id}");
    let deleted = match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin oidc client tx")?;
            let n = sqlx::query("DELETE FROM oidc_clients WHERE client_id = ?")
                .bind(client_id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("delete oidc client {client_id}"))?
                .rows_affected();
            if n == 1 {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES (?, 'oidc-client.delete', ?, NULL, ?)",
                )
                .bind(actor)
                .bind(&target)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit oidc client delete")?;
            }
            tx.commit().await.context("commit oidc client tx")?;
            n == 1
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin oidc client tx")?;
            let n = sqlx::query("DELETE FROM oidc_clients WHERE client_id = $1")
                .bind(client_id)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("delete oidc client {client_id}"))?
                .rows_affected();
            if n == 1 {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES ($1, 'oidc-client.delete', $2, NULL, $3)",
                )
                .bind(actor)
                .bind(&target)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit oidc client delete")?;
            }
            tx.commit().await.context("commit oidc client tx")?;
            n == 1
        }
    };
    Ok(deleted)
}

/// Every client, ordered by id.
pub async fn list_all(db: &ConfigDb) -> Result<Vec<OidcClient>> {
    let sql = select!("ORDER BY client_id ASC");
    let rows: Vec<Row> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    }
    .context("list oidc clients")?;
    Ok(rows.into_iter().map(from_row).collect())
}

/// One client by id, or `None`.
pub async fn fetch(db: &ConfigDb, client_id: &str) -> Result<Option<OidcClient>> {
    let row: Option<Row> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as(select!("WHERE client_id = ?"))
                .bind(client_id)
                .fetch_optional(pool)
                .await
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as(select!("WHERE client_id = $1"))
                .bind(client_id)
                .fetch_optional(pool)
                .await
        }
    }
    .with_context(|| format!("fetch oidc client {client_id}"))?;
    Ok(row.map(from_row))
}

/// Write ONLY a client's group gate (and its enabled flag). Used by
/// [`rewrite_group`], which must not touch the other columns: it works
/// from a snapshot, and rewriting the whole row would silently revert
/// an edit an admin saved in between (re-enabling a client they had
/// just disabled, or restoring a redirect URI they had just removed).
async fn set_gate(
    db: &ConfigDb,
    client_id: &str,
    groups: &[String],
    enabled: bool,
    action: &str,
    actor: Option<&str>,
) -> Result<()> {
    let now = Utc::now();
    let joined = join_groups(groups);
    let target = format!("oidc-client:{client_id}");
    let diff = serde_json::to_string(&serde_json::json!({
        "access_groups": groups,
        "enabled": enabled,
    }))?;
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin oidc client tx")?;
            sqlx::query(
                "UPDATE oidc_clients SET access_groups = ?, enabled = ?, updated_at = ?
                  WHERE client_id = ?",
            )
            .bind(&joined)
            .bind(enabled)
            .bind(now)
            .bind(client_id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("rewrite oidc client gate {client_id}"))?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(actor)
            .bind(action)
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit oidc client gate")?;
            tx.commit().await.context("commit oidc client tx")?;
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin oidc client tx")?;
            sqlx::query(
                "UPDATE oidc_clients SET access_groups = $1, enabled = $2, updated_at = $3
                  WHERE client_id = $4",
            )
            .bind(&joined)
            .bind(enabled)
            .bind(now)
            .bind(client_id)
            .execute(&mut *tx)
            .await
            .with_context(|| format!("rewrite oidc client gate {client_id}"))?;
            sqlx::query(
                "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(actor)
            .bind(action)
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit oidc client gate")?;
            tx.commit().await.context("commit oidc client tx")?;
        }
    }
    Ok(())
}

/// Follow a group rename (`new = Some`) or delete (`new = None`) across
/// every client's `access_groups`, mirroring what the Groups page does
/// for users and specs.
///
/// One deliberate difference from specs: when a delete would leave a
/// restricted client with NO groups, the client is **disabled** and keeps
/// the stale group name (audited as `oidc-client.auto-disable`, so "why
/// is this client off?" has an answer). Emptying the list would silently
/// turn "only this team" into "everyone", which is the wrong direction to
/// fail for a login gate. The admin re-enables it after choosing new
/// groups.
///
/// Limit worth knowing: groups are derived from membership, so a group
/// can also vanish WITHOUT passing through here (its last member is
/// removed or deleted). The client then keeps the orphan name and stays
/// enabled — nobody can sign in, which is safe — but anyone later put in
/// a group of that same name regains access. Same as a spec's
/// `access-groups` today.
pub async fn rewrite_group(
    db: &ConfigDb,
    old: &str,
    new: Option<&str>,
    actor: Option<&str>,
) -> Result<()> {
    for client in list_all(db).await? {
        if !client.access_groups.iter().any(|g| g == old) {
            continue;
        }
        let mut groups: Vec<String> = client
            .access_groups
            .iter()
            .filter(|g| *g != old)
            .cloned()
            .collect();
        if let Some(n) = new {
            if !groups.iter().any(|g| g == n) {
                groups.push(n.to_string());
            }
        }
        if groups.is_empty() {
            // Keep the gate, close the client.
            let kept = &client.access_groups;
            set_gate(db, &client.client_id, kept, false, "oidc-client.auto-disable", actor).await?;
        } else {
            let on = client.enabled;
            set_gate(db, &client.client_id, &groups, on, "oidc-client.group-rewrite", actor).await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn mem_db() -> ConfigDb {
        ConfigDb::Sqlite(crate::db::open_memory().await.expect("open in-memory"))
    }

    fn settings(groups: &[&str]) -> ClientSettings {
        ClientSettings {
            name: "Radar Fiscal".into(),
            redirect_uris: vec!["https://radar.example/callback".into()],
            access_groups: groups.iter().map(|g| g.to_string()).collect(),
            require_mfa: false,
            enabled: true,
            logo: None,
        }
    }

    #[test]
    fn client_id_charset_and_length() {
        assert!(is_valid_client_id("radar-fiscal"));
        assert!(is_valid_client_id("norma_api.v2"));
        assert!(!is_valid_client_id("ab"), "too short");
        assert!(!is_valid_client_id(&"a".repeat(65)), "too long");
        assert!(!is_valid_client_id("Radar"), "uppercase");
        assert!(!is_valid_client_id("a/b"));
        assert!(!is_valid_client_id("with space"));
        assert!(!is_valid_client_id(""));
    }

    #[test]
    fn redirect_uri_rules() {
        use RedirectUriError::*;
        assert!(validate_redirect_uri("https://app.example/cb").is_ok());
        assert!(validate_redirect_uri("https://app.example/cb?x=1").is_ok());
        // Plain http only for loopback development.
        assert!(validate_redirect_uri("http://localhost:5173/cb").is_ok());
        assert!(validate_redirect_uri("http://127.0.0.1:8080/cb").is_ok());
        assert!(validate_redirect_uri("http://[::1]:8080/cb").is_ok());
        assert_eq!(validate_redirect_uri("http://app.example/cb"), Err(InsecureScheme));
        // A hostname that merely STARTS with localhost is not loopback.
        assert_eq!(
            validate_redirect_uri("http://localhost.evil.example/cb"),
            Err(InsecureScheme)
        );
        assert_eq!(validate_redirect_uri("/relative/cb"), Err(NotAbsolute));
        assert_eq!(validate_redirect_uri("myapp://cb"), Err(UnsupportedScheme));
        assert_eq!(validate_redirect_uri("javascript:alert(1)"), Err(UnsupportedScheme));
        assert_eq!(validate_redirect_uri("https://app.example/cb#x"), Err(HasFragment));
        assert_eq!(validate_redirect_uri("https://u:p@app.example/cb"), Err(HasUserinfo));
        assert_eq!(validate_redirect_uri("https://*.example/cb"), Err(HasWildcard));
    }

    /// Inputs a WHATWG parser "repairs" and would otherwise be stored
    /// verbatim. Every one of them parses fine with `url`, so this test
    /// fails if the raw-byte check is removed.
    #[test]
    fn redirect_uri_rejects_what_parsers_read_differently() {
        use RedirectUriError::*;
        for raw in [
            "https://a.example/cb\tx",              // tab stripped by the parser
            "https://a.example/cb\rSet-Cookie: x",  // bare CR: header injection shape
            "\u{1}https://a.example/cb",            // leading control char
            "https://a.example/c b",                // inner space
            "https://good.example\\@evil.example/cb", // `\` read as `/` vs userinfo
            "https://exämple.org/cb",               // raw IDN; register punycode
        ] {
            assert_eq!(validate_redirect_uri(raw), Err(InvalidCharacters), "{raw:?}");
        }
        for raw in ["https:a.example/cb", "https:/a.example/cb", "HTTPS://a.example/cb"] {
            assert_eq!(validate_redirect_uri(raw), Err(NotAbsolute), "{raw:?}");
        }
        // Punycode and an explicit port are fine.
        assert!(validate_redirect_uri("https://xn--exmple-cua.org:8443/cb").is_ok());
    }

    #[test]
    fn redirect_uris_textarea_is_cleaned() {
        assert_eq!(
            parse_redirect_uris("  https://a/cb \n\nhttps://b/cb\r\nhttps://a/cb\n"),
            vec!["https://a/cb".to_string(), "https://b/cb".to_string()]
        );
    }

    #[tokio::test]
    async fn create_returns_secret_once_and_stores_only_its_hash() {
        let db = mem_db().await;
        let secret = create(&db, "radar", &settings(&["fiscal"]), Some("root"))
            .await
            .unwrap()
            .expect("created");
        assert_eq!(secret.len(), 64);

        let c = fetch(&db, "radar").await.unwrap().expect("stored");
        assert!(c.verify_secret(&secret));
        assert!(!c.verify_secret("wrong"));
        assert!(!c.secret_hash.contains(&secret), "cleartext never stored");
        assert_eq!(c.access_groups, vec!["fiscal".to_string()]);
        assert!(c.enabled && !c.require_mfa);

        // A second create with the same id changes nothing — in
        // particular it must not replace the secret.
        assert!(create(&db, "radar", &settings(&[]), Some("root"))
            .await
            .unwrap()
            .is_none());
        let again = fetch(&db, "radar").await.unwrap().unwrap();
        assert!(again.verify_secret(&secret));
        assert_eq!(again.access_groups, vec!["fiscal".to_string()]);
    }

    #[tokio::test]
    async fn rotate_invalidates_the_old_secret() {
        let db = mem_db().await;
        let old = create(&db, "radar", &settings(&[]), None).await.unwrap().unwrap();
        let v1 = fetch(&db, "radar").await.unwrap().unwrap().secret_version;
        let RotateOutcome::Rotated(new) =
            rotate_secret(&db, "radar", v1, Some("root")).await.unwrap()
        else {
            panic!("expected a rotation");
        };
        assert_ne!(old, new);
        let c = fetch(&db, "radar").await.unwrap().unwrap();
        assert!(c.verify_secret(&new));
        assert!(!c.verify_secret(&old));
        assert_eq!(c.secret_version, v1 + 1);
        assert!(matches!(
            rotate_secret(&db, "nope", 1, None).await.unwrap(),
            RotateOutcome::Missing
        ));
    }

    /// The F5 case: the same rotate request arriving twice must rotate
    /// ONCE. The second one carries the version the form was rendered
    /// with, which is no longer current.
    #[tokio::test]
    async fn replayed_rotation_is_refused_and_keeps_the_new_secret() {
        let db = mem_db().await;
        create(&db, "radar", &settings(&[]), None).await.unwrap();
        let v1 = fetch(&db, "radar").await.unwrap().unwrap().secret_version;
        let RotateOutcome::Rotated(new) = rotate_secret(&db, "radar", v1, None).await.unwrap()
        else {
            panic!("expected a rotation");
        };
        assert!(matches!(
            rotate_secret(&db, "radar", v1, None).await.unwrap(),
            RotateOutcome::Stale
        ));
        let c = fetch(&db, "radar").await.unwrap().unwrap();
        assert!(c.verify_secret(&new), "the secret just handed out survives the replay");
        assert_eq!(c.secret_version, v1 + 1);
    }

    #[tokio::test]
    async fn update_and_delete_report_missing_clients() {
        let db = mem_db().await;
        assert!(!update(&db, "nope", &settings(&[]), None).await.unwrap());
        assert!(!delete(&db, "nope", None).await.unwrap());

        create(&db, "radar", &settings(&[]), None).await.unwrap();
        let mut s = settings(&["a", "b"]);
        s.require_mfa = true;
        s.redirect_uris.push("http://localhost:5173/cb".into());
        assert!(update(&db, "radar", &s, Some("root")).await.unwrap());
        let c = fetch(&db, "radar").await.unwrap().unwrap();
        assert!(c.require_mfa);
        assert_eq!(c.redirect_uris.len(), 2);
        assert!(c.redirect_uri_registered("http://localhost:5173/cb"));
        // Exact match only: no prefix, no trailing-slash tolerance.
        assert!(!c.redirect_uri_registered("http://localhost:5173/cb/"));
        assert!(!c.redirect_uri_registered("http://localhost:5173/cb?x=1"));

        assert!(delete(&db, "radar", Some("root")).await.unwrap());
        assert!(fetch(&db, "radar").await.unwrap().is_none());
    }

    #[test]
    fn group_gate_is_open_only_when_no_groups_are_listed() {
        let mut c = OidcClient {
            client_id: "x".into(),
            name: "x".into(),
            secret_hash: String::new(),
            secret_version: 1,
            redirect_uris: vec![],
            access_groups: vec![],
            require_mfa: false,
            enabled: true,
            logo: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(c.allows_groups(&[]));
        c.access_groups = vec!["fiscal".into()];
        assert!(!c.allows_groups(&[]));
        assert!(!c.allows_groups(&["Fiscal".into()]), "case-sensitive");
        assert!(c.allows_groups(&["outro".into(), "fiscal".into()]));
    }

    #[tokio::test]
    async fn group_rename_follows_and_delete_of_last_group_disables() {
        let db = mem_db().await;
        create(&db, "two", &settings(&["a", "b"]), None).await.unwrap();
        create(&db, "one", &settings(&["a"]), None).await.unwrap();
        create(&db, "open", &settings(&[]), None).await.unwrap();

        // Rename a → b, folding when the target is already present.
        rewrite_group(&db, "a", Some("b"), Some("root")).await.unwrap();
        assert_eq!(fetch(&db, "two").await.unwrap().unwrap().access_groups, vec!["b"]);
        assert_eq!(fetch(&db, "one").await.unwrap().unwrap().access_groups, vec!["b"]);

        // Deleting the group: "two"/"one" would both become empty, so
        // they are disabled and keep the gate; "open" is untouched.
        rewrite_group(&db, "b", None, Some("root")).await.unwrap();
        for id in ["two", "one"] {
            let c = fetch(&db, id).await.unwrap().unwrap();
            assert!(!c.enabled, "{id} must be disabled, not opened");
            assert_eq!(c.access_groups, vec!["b"], "{id} keeps its gate");
        }
        let open = fetch(&db, "open").await.unwrap().unwrap();
        assert!(open.enabled && open.access_groups.is_empty());

        // The auto-disable is its own audit action.
        let audits = crate::db::audit::list(&db, &crate::db::audit::AuditFilter::new())
            .await
            .unwrap();
        assert_eq!(
            audits.iter().filter(|e| e.action == "oidc-client.auto-disable").count(),
            2
        );
    }

    /// The rewrite writes the gate columns only (see [`set_gate`]); the
    /// rest of the row comes out exactly as it went in.
    #[tokio::test]
    async fn group_rewrite_leaves_other_columns_alone() {
        let db = mem_db().await;
        create(&db, "radar", &settings(&["a"]), None).await.unwrap();
        let secret_before = fetch(&db, "radar").await.unwrap().unwrap().secret_hash;
        rewrite_group(&db, "a", Some("z"), None).await.unwrap();
        let c = fetch(&db, "radar").await.unwrap().unwrap();
        assert_eq!(c.access_groups, vec!["z"]);
        assert_eq!(c.name, "Radar Fiscal");
        assert_eq!(c.redirect_uris, vec!["https://radar.example/callback".to_string()]);
        assert_eq!(c.secret_hash, secret_before);
        assert!(c.enabled && !c.require_mfa);
    }

    #[tokio::test]
    async fn audit_rows_never_carry_the_secret() {
        let db = mem_db().await;
        let secret = create(&db, "radar", &settings(&[]), Some("root"))
            .await
            .unwrap()
            .unwrap();
        let RotateOutcome::Rotated(rotated) =
            rotate_secret(&db, "radar", 1, Some("root")).await.unwrap()
        else {
            panic!("expected a rotation");
        };
        update(&db, "radar", &settings(&["a"]), Some("root")).await.unwrap();
        rewrite_group(&db, "a", None, Some("root")).await.unwrap();
        let hash = fetch(&db, "radar").await.unwrap().unwrap().secret_hash;
        let audits = crate::db::audit::list(&db, &crate::db::audit::AuditFilter::new())
            .await
            .unwrap();
        let ours: Vec<_> = audits
            .iter()
            .filter(|e| e.action.starts_with("oidc-client."))
            .collect();
        assert_eq!(ours.len(), 4, "create, rotate, update, auto-disable");
        for e in ours {
            let diff = format!("{:?}", e.diff);
            assert!(!diff.contains(&secret) && !diff.contains(&rotated));
            assert!(!diff.contains(&hash), "not even the hash is audited");
        }
    }

    // The dual-dialect statements against a real Postgres. Gated on
    // `postgres-it` (see the workspace CLAUDE.md for the one-liner).
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn registry_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let db = ConfigDb::Postgres(crate::db::open_pg(&url).await.expect("open pg"));
        let id = format!("pg-it-{}", std::process::id());
        let _ = delete(&db, &id, None).await;

        let secret = create(&db, &id, &settings(&["a"]), Some("root"))
            .await
            .unwrap()
            .expect("created");
        assert!(create(&db, &id, &settings(&[]), None).await.unwrap().is_none());
        let mut s = settings(&["a", "b"]);
        s.require_mfa = true;
        assert!(update(&db, &id, &s, Some("root")).await.unwrap());
        let c = fetch(&db, &id).await.unwrap().unwrap();
        assert!(c.verify_secret(&secret) && c.require_mfa && c.enabled);
        let RotateOutcome::Rotated(new) =
            rotate_secret(&db, &id, c.secret_version, None).await.unwrap()
        else {
            panic!("expected a rotation");
        };
        assert!(matches!(
            rotate_secret(&db, &id, c.secret_version, None).await.unwrap(),
            RotateOutcome::Stale
        ));
        assert!(fetch(&db, &id).await.unwrap().unwrap().verify_secret(&new));
        rewrite_group(&db, "a", None, None).await.unwrap();
        assert_eq!(fetch(&db, &id).await.unwrap().unwrap().access_groups, vec!["b"]);
        assert!(list_all(&db).await.unwrap().iter().any(|c| c.client_id == id));
        assert!(delete(&db, &id, None).await.unwrap());
    }
}
