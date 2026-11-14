//! Secrets at rest: provider keys, the ntfy token and the proxy password.
//!
//! The key lives in `<db dir>/secret.key`, created on first boot with 0600. That is honest about
//! what this buys: it defends against the database file escaping (a copied `stock-predictor.db`, a
//! backup, `sqlite3` open in a terminal), **not** against someone who has the whole data directory,
//! and it is nothing like authentication. The server still has no login.
//!
//! Values are stored as `enc:v1:<base64 nonce || ciphertext>`. Anything without that prefix is
//! plaintext from an older database and is encrypted in place on boot.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use anyhow::{Context, Result};
use base64::Engine;
use chacha20poly1305::aead::{Aead, KeyInit};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};

/// The system CSPRNG. `getrandom` is what the AEAD crate asks for anyway; naming it here keeps the
/// key and nonce generation in one place.
fn fill_random(bytes: &mut [u8]) -> Result<()> {
    getrandom::fill(bytes).map_err(|e| anyhow::anyhow!("no system randomness: {e}"))
}

fn random_key() -> Result<Key> {
    let mut bytes = [0u8; 32];
    fill_random(&mut bytes)?;
    Ok(Key::from(bytes))
}

fn random_nonce() -> Result<XNonce> {
    let mut bytes = [0u8; 24];
    fill_random(&mut bytes)?;
    Ok(XNonce::from(bytes))
}
use sqlx::SqlitePool;

const PREFIX: &str = "enc:v1:";

/// Settings rows that hold a secret, and must never be stored in the clear.
pub const SECRET_KEYS: &[&str] = &[
    "deepseek_key",
    "openai_key",
    "gemini_key",
    "custom_key",
    "ntfy_token",
    "proxy_pass",
];

static KEY: OnceLock<Vec<u8>> = OnceLock::new();

/// Called once at boot with `SECRET_KEY` or the key file's contents.
pub fn init(key: Vec<u8>) {
    let _ = KEY.set(key);
}

/// Never absent: an uninitialised process (a unit test) gets a random key for its lifetime, so a
/// secret is never written in the clear by accident.
fn key() -> &'static [u8] {
    KEY.get_or_init(|| {
        random_key()
            .map(|k| k.to_vec())
            .unwrap_or_else(|_| vec![0u8; 32])
    })
}

pub fn key_path(db_path: &str) -> PathBuf {
    let db = Path::new(db_path);
    db.parent()
        .unwrap_or_else(|| Path::new("."))
        .join("secret.key")
}

/// Read the key, or create it. 32 random bytes, mode 0600 on unix.
pub fn load_or_create_key(db_path: &str) -> Result<Vec<u8>> {
    let path = key_path(db_path);
    if let Ok(bytes) = std::fs::read(&path) {
        if bytes.len() >= 32 {
            return Ok(bytes[..32].to_vec());
        }
    }
    let key = random_key()?.to_vec();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, &key).with_context(|| format!("write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(key)
}

fn cipher(key: &[u8]) -> Result<XChaCha20Poly1305> {
    if key.len() < 32 {
        anyhow::bail!("secret key is too short");
    }
    let bytes: [u8; 32] = key
        .get(..32)
        .and_then(|k| k.try_into().ok())
        .ok_or_else(|| anyhow::anyhow!("key must be 32 bytes"))?;
    Ok(XChaCha20Poly1305::new(&Key::from(bytes)))
}

/// Encrypt with the process key.
pub fn encrypt(plaintext: &str) -> Result<String> {
    encrypt_with(key(), plaintext)
}

/// Decrypt with the process key.
pub fn decrypt(stored: &str) -> Option<String> {
    decrypt_with(key(), stored)
}

pub fn encrypt_with(key: &[u8], plaintext: &str) -> Result<String> {
    let cipher = cipher(key)?;
    let nonce = random_nonce()?;
    let ct = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|_| anyhow::anyhow!("encrypt failed"))?;
    let mut blob = nonce.to_vec();
    blob.extend_from_slice(&ct);
    Ok(format!(
        "{PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(blob)
    ))
}

/// `None` when the value cannot be opened — a wrong key must not stop the app from running.
pub fn decrypt_with(key: &[u8], stored: &str) -> Option<String> {
    let body = stored.strip_prefix(PREFIX)?;
    let blob = base64::engine::general_purpose::STANDARD
        .decode(body)
        .ok()?;
    if blob.len() <= 24 {
        return None;
    }
    let (nonce, ct) = blob.split_at(24);
    let cipher = cipher(key).ok()?;
    let nonce: [u8; 24] = nonce.try_into().ok()?;
    let plain = cipher.decrypt(&XNonce::from(nonce), ct).ok()?;
    String::from_utf8(plain).ok()
}

pub fn is_encrypted(value: &str) -> bool {
    value.starts_with(PREFIX)
}

/// Encrypt any secret still sitting in the clear. Idempotent; runs on every boot.
pub async fn migrate(pool: &SqlitePool) -> Result<usize> {
    let rows: Vec<(String, String)> = sqlx::query_as("SELECT key, value FROM settings")
        .fetch_all(pool)
        .await?;
    let mut changed = 0;
    for (name, value) in rows {
        if !SECRET_KEYS.contains(&name.as_str()) || value.trim().is_empty() || is_encrypted(&value)
        {
            continue;
        }
        let sealed = encrypt(&value)?;
        sqlx::query("UPDATE settings SET value = ? WHERE key = ?")
            .bind(sealed)
            .bind(&name)
            .execute(pool)
            .await?;
        changed += 1;
    }
    Ok(changed)
}

/// Read a secret, decrypting when needed. Old plaintext rows still work.
/// True when a value is stored for this key but cannot be opened — the SECRET_KEY changed.
pub async fn unreadable(pool: &SqlitePool, name: &str) -> Result<bool> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(name)
        .fetch_optional(pool)
        .await?;
    Ok(row
        .map(|(v,)| is_encrypted(&v) && decrypt(&v).is_none())
        .unwrap_or(false))
}

pub async fn get_secret(pool: &SqlitePool, name: &str) -> Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as("SELECT value FROM settings WHERE key = ?")
        .bind(name)
        .fetch_optional(pool)
        .await?;
    let Some((value,)) = row else { return Ok(None) };
    if value.trim().is_empty() {
        return Ok(None);
    }
    if !is_encrypted(&value) {
        return Ok(Some(value));
    }
    match decrypt(&value) {
        Some(plain) if !plain.trim().is_empty() => Ok(Some(plain)),
        _ => {
            tracing::warn!(key = name, "cannot be opened with the current secret key");
            Ok(None)
        }
    }
}

pub async fn put_secret(pool: &SqlitePool, name: &str, plaintext: &str) -> Result<()> {
    let sealed = encrypt(plaintext)?;
    sqlx::query(
        "INSERT INTO settings (key, value, updated_at) VALUES (?, ?, ?)
         ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
    )
    .bind(name)
    .bind(sealed)
    .bind(chrono::Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_secret_round_trips_and_never_appears_in_the_stored_form() {
        let key = vec![7u8; 32];
        let sealed = encrypt_with(&key, "sk-live-hunter2").unwrap();
        assert!(sealed.starts_with(PREFIX));
        assert!(!sealed.contains("hunter2"));
        assert_eq!(
            decrypt_with(&key, &sealed).as_deref(),
            Some("sk-live-hunter2")
        );
    }

    #[test]
    fn the_wrong_key_yields_nothing_rather_than_garbage() {
        let sealed = encrypt_with(&[1u8; 32], "secret").unwrap();
        assert_eq!(decrypt_with(&[2u8; 32], &sealed), None);
    }

    #[test]
    fn the_same_plaintext_encrypts_differently_each_time() {
        let key = vec![3u8; 32];
        assert_ne!(
            encrypt_with(&key, "x").unwrap(),
            encrypt_with(&key, "x").unwrap()
        );
    }

    #[test]
    fn the_key_file_sits_beside_the_database() {
        assert_eq!(
            key_path("data/stock-predictor.db"),
            PathBuf::from("data/secret.key")
        );
        assert_eq!(key_path("/srv/app.db"), PathBuf::from("/srv/secret.key"));
    }
}
