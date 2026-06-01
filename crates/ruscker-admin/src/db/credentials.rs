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
use zeroize::Zeroizing;

use crate::crypto::MasterKey;
use crate::db::ConfigDb;

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
    db: &ConfigDb,
    key: &MasterKey,
    name: &str,
    registry: &str,
    username: &str,
    password: &str,
    actor: Option<&str>,
) -> Result<bool> {
    let now = Utc::now();
    // Two storage modes, one store (#351 unification):
    //   • literal password  → AES-256-GCM at rest (the original model).
    //   • `${VAR}` env-ref   → stored VERBATIM, never encrypted, with an
    //     EMPTY nonce as the discriminator (a real GCM nonce is 12 bytes,
    //     never empty). Resolved from the environment at pull time, so the
    //     plaintext never lands in the DB at all — preserving the "DB never
    //     sees the cleartext" model now that the spec form points only at
    //     the store.
    //
    // The env-ref branch requires the password to be ENTIRELY valid env-ref
    // tokens (#422): a loose `contains("${")` would store a literal like
    // `abc${def` (no valid token) verbatim — i.e. cleartext at rest. Anything
    // that isn't a pure env-ref is encrypted.
    let (ciphertext, nonce): (Vec<u8>, Vec<u8>) =
        if ruscker_config::env::is_pure_env_ref(password) {
            (password.as_bytes().to_vec(), Vec::new())
        } else {
            key.encrypt(password.as_bytes())?
        };
    // Metadata-only audit diff — never the password (see the
    // `audit_log_never_records_the_password` test).
    let diff = serde_json::to_string(&serde_json::json!({
        "registry": registry,
        "username": username,
    }))?;
    let target = format!("credential:{name}");

    // `ON CONFLICT(name) DO UPDATE SET col = excluded.col` is the same
    // on both backends — only the placeholders differ.
    match db {
        ConfigDb::Sqlite(pool) => {
            let mut tx = pool.begin().await.context("begin credential tx")?;
            let existing: Option<(String,)> =
                sqlx::query_as("SELECT name FROM credentials WHERE name = ?")
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
            .bind(if replaced {
                "credential.update"
            } else {
                "credential.create"
            })
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit credential upsert")?;
            tx.commit().await.context("commit credential tx")?;
            Ok(replaced)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin credential tx")?;
            let existing: Option<(String,)> =
                sqlx::query_as("SELECT name FROM credentials WHERE name = $1")
                    .bind(name)
                    .fetch_optional(&mut *tx)
                    .await
                    .with_context(|| format!("lookup credential {name}"))?;
            let replaced = existing.is_some();
            sqlx::query(
                "INSERT INTO credentials
                   (name, registry, username, password_enc, nonce, created_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
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
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(actor)
            .bind(if replaced {
                "credential.update"
            } else {
                "credential.create"
            })
            .bind(&target)
            .bind(&diff)
            .bind(now)
            .execute(&mut *tx)
            .await
            .context("audit credential upsert")?;
            tx.commit().await.context("commit credential tx")?;
            Ok(replaced)
        }
    }
}

/// List every credential — names, usernames, registries only.
pub async fn list_all(db: &ConfigDb) -> Result<Vec<CredentialMeta>> {
    let sql = "SELECT name, registry, username, created_at
           FROM credentials
          ORDER BY name ASC";
    let rows: Vec<(String, String, String, DateTime<Utc>)> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(sql).fetch_all(pool).await,
        ConfigDb::Postgres(pool) => sqlx::query_as(sql).fetch_all(pool).await,
    }
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
    db: &ConfigDb,
    key: &MasterKey,
    name: &str,
) -> Result<Option<ruscker_core::RegistryCredentials>> {
    let row: Option<(String, String, Vec<u8>, Vec<u8>)> = match db {
        ConfigDb::Sqlite(pool) => sqlx::query_as(
            "SELECT registry, username, password_enc, nonce FROM credentials WHERE name = ?",
        )
        .bind(name)
        .fetch_optional(pool)
        .await,
        ConfigDb::Postgres(pool) => sqlx::query_as(
            "SELECT registry, username, password_enc, nonce FROM credentials WHERE name = $1",
        )
        .bind(name)
        .fetch_optional(pool)
        .await,
    }
    .with_context(|| format!("resolve credential {name}"))?;
    match row {
        None => Ok(None),
        Some((registry, username, enc, nonce)) => {
            // Empty nonce ⇒ a `${VAR}` env-ref stored verbatim (see
            // `upsert`): resolve it from the environment at pull time
            // rather than decrypting. Non-empty ⇒ AES-GCM ciphertext.
            let password = if nonce.is_empty() {
                let raw = String::from_utf8(enc)
                    .context("stored credential reference is not valid UTF-8")?;
                ruscker_config::env::interpolate_value(&raw)
                    .with_context(|| format!("resolve ${{VAR}} in credential {name}"))?
            } else {
                let pt = key.decrypt(&enc, &nonce)?;
                String::from_utf8(pt.to_vec())
                    .context("decrypted password is not valid UTF-8")?
            };
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
    db: &ConfigDb,
    key: &MasterKey,
    name: &str,
) -> Result<Option<Zeroizing<String>>> {
    let row: Option<(Vec<u8>, Vec<u8>)> = match db {
        ConfigDb::Sqlite(pool) => {
            sqlx::query_as("SELECT password_enc, nonce FROM credentials WHERE name = ?")
                .bind(name)
                .fetch_optional(pool)
                .await
        }
        ConfigDb::Postgres(pool) => {
            sqlx::query_as("SELECT password_enc, nonce FROM credentials WHERE name = $1")
                .bind(name)
                .fetch_optional(pool)
                .await
        }
    }
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

pub async fn delete_one(db: &ConfigDb, name: &str, actor: Option<&str>) -> Result<bool> {
    let now = Utc::now();
    let target = format!("credential:{name}");
    match db {
        ConfigDb::Sqlite(pool) => {
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
                .bind(&target)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit credential.delete")?;
            }
            tx.commit().await.context("commit credential delete")?;
            Ok(removed)
        }
        ConfigDb::Postgres(pool) => {
            let mut tx = pool.begin().await.context("begin credential delete tx")?;
            let rows = sqlx::query("DELETE FROM credentials WHERE name = $1")
                .bind(name)
                .execute(&mut *tx)
                .await
                .with_context(|| format!("delete credential {name}"))?;
            let removed = rows.rows_affected() > 0;
            if removed {
                sqlx::query(
                    "INSERT INTO audit_log (actor, action, target, diff_json, occurred_at)
                     VALUES ($1, 'credential.delete', $2, '{}', $3)",
                )
                .bind(actor)
                .bind(&target)
                .bind(now)
                .execute(&mut *tx)
                .await
                .context("audit credential.delete")?;
            }
            tx.commit().await.context("commit credential delete")?;
            Ok(removed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::open_memory;

    fn fixed_key() -> MasterKey {
        MasterKey::parse(&"ab".repeat(32)).unwrap()
    }

    #[tokio::test]
    async fn upsert_then_fetch_round_trip() {
        let pool = open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool.clone());
        let key = fixed_key();
        upsert(&db, &key, "docker-hub", "docker.io", "acme", "hunter2", Some("admin"))
            .await
            .unwrap();
        let pw = fetch_password(&db, &key, "docker-hub").await.unwrap();
        assert_eq!(pw.unwrap().as_str(), "hunter2");
    }

    #[tokio::test]
    async fn list_does_not_contain_passwords() {
        let pool = open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool.clone());
        let key = fixed_key();
        upsert(&db, &key, "dh", "docker.io", "acme", "topsecret", None)
            .await
            .unwrap();
        let metas = list_all(&db).await.unwrap();
        assert_eq!(metas.len(), 1);
        assert_eq!(metas[0].name, "dh");
        assert_eq!(metas[0].username, "acme");
        // (No way to access the password field — CredentialMeta
        // doesn't have one. Compile-time guarantee.)
    }

    #[tokio::test]
    async fn audit_log_never_records_the_password() {
        let pool = open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool.clone());
        let key = fixed_key();
        upsert(
            &db,
            &key,
            "dh",
            "docker.io",
            "acme",
            "p@ss-w0rd-SECRET",
            Some("admin"),
        )
        .await
        .unwrap();
        // The audit diff for a credential change records only metadata
        // (registry/username); the plaintext password must appear in no
        // audit_log row.
        let diffs: Vec<Option<String>> = sqlx::query_scalar("SELECT diff_json FROM audit_log")
            .fetch_all(&pool)
            .await
            .unwrap();
        for d in diffs.into_iter().flatten() {
            assert!(
                !d.contains("p@ss-w0rd-SECRET"),
                "password leaked into audit diff_json: {d}"
            );
        }
    }

    #[tokio::test]
    async fn upsert_replaces_password() {
        let pool = open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool.clone());
        let key = fixed_key();
        upsert(&db, &key, "dh", "docker.io", "acme", "old", None).await.unwrap();
        let replaced = upsert(&db, &key, "dh", "docker.io", "acme", "new", None).await.unwrap();
        assert!(replaced);
        let pw = fetch_password(&db, &key, "dh").await.unwrap().unwrap();
        assert_eq!(pw.as_str(), "new");
    }

    #[tokio::test]
    async fn delete_then_fetch_returns_none() {
        let pool = open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool.clone());
        let key = fixed_key();
        upsert(&db, &key, "dh", "docker.io", "acme", "x", None).await.unwrap();
        assert!(delete_one(&db, "dh", None).await.unwrap());
        assert!(fetch_password(&db, &key, "dh").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn resolve_returns_full_credential() {
        let pool = open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool.clone());
        let key = fixed_key();
        upsert(&db, &key, "priv", "registry.example.com", "bot", "hunter2", None)
            .await
            .unwrap();
        let c = resolve(&db, &key, "priv").await.unwrap().expect("resolved");
        assert_eq!(c.username, "bot");
        assert_eq!(c.password, "hunter2");
        assert_eq!(c.server_address.as_deref(), Some("registry.example.com"));
    }

    #[tokio::test]
    async fn env_ref_password_is_stored_verbatim_and_resolved_at_pull() {
        // A `${VAR}` password (#351 unification): stored verbatim with an
        // empty nonce, resolved from the environment at resolve time. Using
        // `${VAR:-default}` with the var unset exercises the path without
        // mutating the process environment.
        let pool = open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool.clone());
        let key = fixed_key();
        upsert(
            &db,
            &key,
            "envcred",
            "registry.example.com",
            "bot",
            "${RUSCKER_TEST_UNSET_CRED:-resolved-pw}",
            None,
        )
        .await
        .unwrap();
        let c = resolve(&db, &key, "envcred").await.unwrap().expect("resolved");
        assert_eq!(c.password, "resolved-pw", "env-ref resolved at pull");

        // Stored verbatim (never AES-encrypted), so a DIFFERENT master key
        // still resolves it — proof the secret never hit the cipher.
        let other = MasterKey::parse(&"cd".repeat(32)).unwrap();
        let c2 = resolve(&db, &other, "envcred")
            .await
            .unwrap()
            .expect("resolved with a different key");
        assert_eq!(
            c2.password, "resolved-pw",
            "verbatim env-ref ignores the master key (not encrypted)"
        );
    }

    #[tokio::test]
    async fn literal_password_containing_dollar_brace_is_encrypted_not_verbatim() {
        // #422: a literal password that merely CONTAINS `${` (not a valid
        // env-ref) must be AES-encrypted, never stored verbatim.
        let pool = open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool.clone());
        let key = fixed_key();
        upsert(&db, &key, "lit", "registry.example.com", "bot", "abc${def", None)
            .await
            .unwrap();
        // Same key round-trips the literal verbatim.
        let c = resolve(&db, &key, "lit").await.unwrap().expect("resolved");
        assert_eq!(c.password, "abc${def");
        // A DIFFERENT key fails to decrypt — proof it was encrypted, not
        // stored in cleartext (a verbatim value would resolve regardless).
        let other = MasterKey::parse(&"cd".repeat(32)).unwrap();
        assert!(
            resolve(&db, &other, "lit").await.is_err(),
            "literal with `${{` must be AES-encrypted, not verbatim"
        );
    }

    #[tokio::test]
    async fn resolve_empty_registry_means_docker_hub() {
        let pool = open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool.clone());
        let key = fixed_key();
        upsert(&db, &key, "hub", "", "bot", "pw", None).await.unwrap();
        let c = resolve(&db, &key, "hub").await.unwrap().unwrap();
        assert!(c.server_address.is_none(), "empty registry -> Docker Hub default");
    }

    #[tokio::test]
    async fn resolve_unknown_name_is_none() {
        let pool = open_memory().await.unwrap();
        let db = ConfigDb::Sqlite(pool.clone());
        let key = fixed_key();
        assert!(resolve(&db, &key, "nope").await.unwrap().is_none());
    }

    // Encrypt/replace/decrypt/resolve/delete through the
    // `ConfigDb::Postgres` arm against a real daemon (BYTEA round-trip +
    // `ON CONFLICT`). Gated on `postgres-it`.
    #[cfg(feature = "postgres-it")]
    #[tokio::test]
    async fn credentials_against_real_postgres() {
        let _guard = crate::db::pg_test_lock().lock().await;
        let url = std::env::var("RUSCKER_TEST_PG_URL")
            .expect("set RUSCKER_TEST_PG_URL to a reachable postgres:// DSN");
        let pool = crate::db::open_pg(&url).await.unwrap();
        sqlx::query("DELETE FROM credentials")
            .execute(&pool)
            .await
            .unwrap();
        let db = ConfigDb::Postgres(pool);
        let key = fixed_key();

        assert!(
            !upsert(&db, &key, "dh", "docker.io", "acme", "s3cret", Some("admin"))
                .await
                .unwrap(),
            "first upsert inserts"
        );
        assert!(
            upsert(&db, &key, "dh", "docker.io", "acme2", "s3cret2", Some("admin"))
                .await
                .unwrap(),
            "second upsert replaces"
        );
        assert_eq!(
            fetch_password(&db, &key, "dh").await.unwrap().unwrap().as_str(),
            "s3cret2"
        );
        let c = resolve(&db, &key, "dh").await.unwrap().unwrap();
        assert_eq!(c.username, "acme2");
        assert_eq!(c.server_address.as_deref(), Some("docker.io"));
        assert_eq!(list_all(&db).await.unwrap().len(), 1);
        assert!(delete_one(&db, "dh", Some("admin")).await.unwrap());
        assert!(fetch_password(&db, &key, "dh").await.unwrap().is_none());
    }
}
