//! Registry credentials — the encrypted-at-rest password store.
//!
//! Plaintext passwords never touch the disk. The
//! [`MasterKey`](crate::crypto::MasterKey) encrypts on insert,
//! decrypts on fetch. The admin UI lists names + usernames only;
//! the actual password is fetched by the runtime when it needs to
//! pull an image (and never echoed back to the operator's
//! browser).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use sqlx::SqlitePool;
use zeroize::Zeroizing;

use crate::crypto::MasterKey;

/// Public-facing summary — no secrets. What the gallery sees.
#[derive(Debug, Clone)]
pub struct CredentialMeta {
    pub name: String,
    pub registry: String,
    pub username: String,
    pub created_at: DateTime<Utc>,
}

/// Insert (or replace) a credential. Encrypts the password under
/// the master key with a fresh nonce. Audit entry tagged with
/// `actor`. Returns `true` if a prior row was replaced.
pub async fn upsert(
    pool: &SqlitePool,
    key: &MasterKey,
    name: &str,
    registry: &str,
    username: &str,
    password: &str,
    actor: Option<&str>,
) -> Result<bool> {
    let now = Utc::now();
    let (ciphertext, nonce) = key.encrypt(password.as_bytes())?;

    let mut tx = pool.begin().await.context("begin credential tx")?;

    let existing: Option<(i64,)> =
        sqlx::query_as("SELECT 1 FROM credentials WHERE name = ?")
            .bind(name)
            .fetch_optional(&mut *tx)
            .await
            .with_context(|| format!("lookup credential {name}"))?;
    let replaced = existing.is_some();

    sqlx::query(
        "INSERT INTO credentials
           (name, registry, username, password_enc, nonce, created_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT(name) DO UPDATE SET
           registry = excluded.registry,
           username = excluded.username,
           password_enc = excluded.password_enc,
           nonce = excluded.nonce",
    )
    .bind(name)
    .bind(registry)
    .bind(username)
    .bind(&ciphertext)
    .bind(&nonce)
    .bind(now)
    .execute(&mut *tx)
    .await
    .with_context(|| format!("insert credential {name}"))?;

    sqlx::query(
        "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(actor)
    .bind(if replaced { "credential.update" } else { "credential.create" })
    .bind(format!("credential:{name}"))
    .bind(serde_json::to_string(&serde_json::json!({
        "registry": registry,
        "username": username,
    }))?)
    .bind(now)
    .execute(&mut *tx)
    .await
    .context("audit credential upsert")?;

    tx.commit().await.context("commit credential tx")?;
    Ok(replaced)
}

/// List every credential — names, usernames, registries only.
pub async fn list_all(pool: &SqlitePool) -> Result<Vec<CredentialMeta>> {
    let rows: Vec<(String, String, String, DateTime<Utc>)> = sqlx::query_as(
        "SELECT name, registry, username, created_at
           FROM credentials
          ORDER BY name ASC",
    )
    .fetch_all(pool)
    .await
    .context("list credentials")?;
    Ok(rows
        .into_iter()
        .map(|(name, registry, username, created_at)| CredentialMeta {
            name,
            registry,
            username,
            created_at,
        })
        .collect())
}

/// Resolve a named credential to backend-neutral
/// [`RegistryCredentials`] for use at image-pull time: fetch
/// registry + username, decrypt the password. Returns `None`
/// when the name doesn't exist.
///
/// This is the one place where a stored secret is turned back
/// into a usable credential outside the admin's own
/// encrypt/decrypt path; it's called from the spawn path, never
/// from a UI handler.
pub async fn resolve(
    pool: &SqlitePool,
    key: &MasterKey,
    name: &str,
) -> Result<Option<ruscker_core::RegistryCredentials>> {
    let row: Option<(String, String, Vec<u8>, Vec<u8>)> = sqlx::query_as(
        "SELECT registry, username, password_enc, nonce FROM credentials WHERE name = ?",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .with_context(|| format!("resolve credential {name}"))?;
    match row {
        None => Ok(None),
        Some((registry, username, enc, nonce)) => {
            let pt = key.decrypt(&enc, &nonce)?;
            let password = String::from_utf8(pt.to_vec())
                .context("decrypted password is not valid UTF-8")?;
            // An empty `registry` means Docker Hub — keep
            // `server_address` None in that case so bollard
            // doesn't get an empty serveraddress.
            let server_address = if registry.trim().is_empty() {
                None
            } else {
                Some(registry)
            };
            Ok(Some(ruscker_core::RegistryCredentials {
                username,
                password,
                server_address,
            }))
        }
    }
}

/// Decrypt the password for a single credential. Used by the
/// runtime when pulling an image; **not** exposed in the UI.
/// Returns `None` if the name doesn't exist.
pub async fn fetch_password(
    pool: &SqlitePool,
    key: &MasterKey,
    name: &str,
) -> Result<Option<Zeroizing<String>>> {
    let row: Option<(Vec<u8>, Vec<u8>)> =
        sqlx::query_as("SELECT password_enc, nonce FROM credentials WHERE name = ?")
            .bind(name)
            .fetch_optional(pool)
            .await
            .with_context(|| format!("fetch credential {name}"))?;
    match row {
        None => Ok(None),
        Some((enc, nonce)) => {
            let pt = key.decrypt(&enc, &nonce)?;
            let s = String::from_utf8(pt.to_vec())
                .context("decrypted password is not valid UTF-8")?;
            Ok(Some(Zeroizing::new(s)))
        }
    }
}

pub async fn delete_one(pool: &SqlitePool, name: &str, actor: Option<&str>) -> Result<bool> {
    let now = Utc::now();
    let mut tx = pool.begin().await.context("begin credential delete tx")?;
    let rows = sqlx::query("DELETE FROM credentials WHERE name = ?")
        .bind(name)
        .execute(&mut *tx)
        .await
        .with_context(|| format!("delete credential {name}"))?;
    let removed = rows.rows_affected() > 0;
    if removed {
        sqlx::query(
            "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
             VALUES (?, 'credential.delete', ?, '{}', ?)",
        )
        .bind(actor)
        .bind(format!("credential:{name}"))
        .bind(now)
        .execute(&mut *tx)
        .await
        .context("audit credential.delete")?;
    }
    tx.commit().await.context("commit credential delete")?;
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    fn fixed_key() -> MasterKey {
        MasterKey::from_str(&"ab".repeat(32)).unwrap()
    }

    #[tokio::test]
    async fn upsert_then_fetch_round_trip() {
        let pool = open_memory().await.unwrap();
        let key = fixed_key();
        upsert(&pool, &key, "docker-hub", "docker.io", "milkway", "hunter2", Some("admin"))
            .await
            .unwrap();
        let pw = fetch_password(&pool, &key, "docker-hub").await.unwrap();
        assert_eq!(pw.unwrap().as_str(), "hunter2");
    }

    #[tokio::test]
    async fn list_does_not_contain_passwords() {
        let pool = open_memory().await.unwrap();
        let key = fixed_key();
        upsert(&pool, &key, "dh", "docker.io", "milkway", "topsecret", None)
            .await
            .unwrap();
        let metas = list_all(&pool).await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].name, "dh");
        assert_eq!(metas[0].username, "milkway");
        // (No way to access the password field — CredentialMeta
        // doesn't have one. Compile-time guarantee.)
    }

    #[tokio::test]
    async fn upsert_replaces_password() {
        let pool = open_memory().await.unwrap();
        let key = fixed_key();
        upsert(&pool, &key, "dh", "docker.io", "milkway", "old", None).await.unwrap();
        let replaced = upsert(&pool, &key, "dh", "docker.io", "milkway", "new", None).await.unwrap();
        assert!(replaced);
        let pw = fetch_password(&pool, &key, "dh").await.unwrap().unwrap();
        assert_eq!(pw.as_str(), "new");
    }

    #[tokio::test]
    async fn delete_then_fetch_returns_none() {
        let pool = open_memory().await.unwrap();
        let key = fixed_key();
        upsert(&pool, &key, "dh", "docker.io", "milkway", "x", None).await.unwrap();
        assert!(delete_one(&pool, "dh", None).await.unwrap());
        assert!(fetch_password(&pool, &key, "dh").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn resolve_returns_full_credential() {
        let pool = open_memory().await.unwrap();
        let key = fixed_key();
        upsert(&pool, &key, "priv", "registry.example.com", "bot", "hunter2", None)
            .await
            .unwrap();
        let c = resolve(&pool, &key, "priv").await.unwrap().expect("resolved");
        assert_eq!(c.username, "bot");
        assert_eq!(c.password, "hunter2");
        assert_eq!(c.server_address.as_deref(), Some("registry.example.com"));
    }

    #[tokio::test]
    async fn resolve_empty_registry_means_docker_hub() {
        let pool = open_memory().await.unwrap();
        let key = fixed_key();
        upsert(&pool, &key, "hub", "", "bot", "pw", None).await.unwrap();
        let c = resolve(&pool, &key, "hub").await.unwrap().unwrap();
        assert!(c.server_address.is_none(), "empty registry -> Docker Hub default");
    }

    #[tokio::test]
    async fn resolve_unknown_name_is_none() {
        let pool = open_memory().await.unwrap();
        let key = fixed_key();
        assert!(resolve(&pool, &key, "nope").await.unwrap().is_none());
    }
}
