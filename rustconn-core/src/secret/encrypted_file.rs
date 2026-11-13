//! Application-managed encrypted-file secret backend.
//!
//! A durable, desktop-environment-independent [`SecretBackend`] that needs no
//! system keyring. It reuses the shared AES-256-GCM + Argon2id credential crypto
//! from [`super::local_crypto`] (the same `RCSC` blob format as the `config.toml`
//! `*_encrypted` master-secret fields) and stores one independently encrypted
//! blob per connection in a JSON map.
//!
//! Storage model (Req 6.1, 6.2, 6.4):
//! - Location: `dirs::data_dir()/rustconn/credentials.enc` — under
//!   `$XDG_DATA_HOME`, writable inside Flatpak/Snap sandboxes and headless.
//! - Shape: a JSON object `{ connection_id: base64(RCSC blob) }`. Each value is
//!   an independently encrypted, serialized [`StoredCredentials`] so deleting one
//!   entry leaves the rest intact (Req 6.5).
//! - Writes go to a sibling temp file then `rename` (atomic; no truncation on
//!   crash) and the file is `0600` on unix (Req 8.4).
//!
//! Secret hygiene (Req 8.1–8.3, 8.6): in-memory secrets are [`SecretString`];
//! intermediate plaintext lives only inside [`Zeroizing`]; [`StoredCredentials`]
//! wipes its secret fields on drop and redacts them from `Debug`; no secret
//! value enters a log or an error message.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use async_trait::async_trait;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use zeroize::{Zeroize, Zeroizing};

use super::backend::SecretBackend;
use super::local_crypto::{decrypt_credential, encrypt_credential, get_machine_key};
use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

/// File name (under the XDG data dir) holding the encrypted credential map.
const STORE_FILE_NAME: &str = "credentials.enc";

/// Default location of the machine-bound credential store.
///
/// Exposed so callers that need to *name* the file — the portable-file migration
/// in Settings, for one — do not rebuild the path from a hardcoded literal and
/// drift from where the backend actually reads.
#[must_use]
pub fn default_encrypted_store_path() -> PathBuf {
    dirs::data_dir()
        .map(|dir| dir.join("rustconn").join(STORE_FILE_NAME))
        .unwrap_or_else(|| PathBuf::from(STORE_FILE_NAME))
}

/// Owner-only permission bits (`rw-------`) for the on-disk store (Req 8.4).
#[cfg(unix)]
const STORE_FILE_MODE: u32 = 0o600;

/// Plaintext credential payload that is serialized, encrypted, and stored.
///
/// This type exists only transiently between (de)serialization and the
/// encrypted blob. Its secret fields are wiped on drop and excluded from the
/// `Debug` representation (Req 8.6) so they never leak via logs.
///
/// Shared with [`super::portable_encrypted_file`]: both backends store the same
/// payload and differ only in how the key is obtained, so the wipe-on-drop and
/// `Debug`-redaction guarantees are written once rather than copied.
#[derive(Serialize, Deserialize)]
pub(crate) struct StoredCredentials {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    /// Legacy: an SSH key passphrase stored by a release before 0.24.6.
    ///
    /// Still accepted on read so an old entry decrypts, then wiped and dropped:
    /// [`Self::into_credentials`] does not pass it on, and it is never written
    /// (`skip_serializing`). An old entry keeps it until the entry is next
    /// rewritten: the connection's credentials are saved again, the connection
    /// is renamed or moved (a key move is retrieve + store + delete), or the
    /// entry is migrated between this store and the portable one.
    #[serde(default, skip_serializing)]
    key_passphrase: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    domain: Option<String>,
}

impl Drop for StoredCredentials {
    fn drop(&mut self) {
        // Wipe plaintext secret material; username/domain are non-secret.
        self.password.zeroize();
        self.key_passphrase.zeroize();
    }
}

impl std::fmt::Debug for StoredCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Redact secret fields; only presence is reported (Req 8.6).
        f.debug_struct("StoredCredentials")
            .field("username", &self.username)
            .field("password", &self.password.as_ref().map(|_| "<redacted>"))
            .field(
                "key_passphrase",
                &self.key_passphrase.as_ref().map(|_| "<redacted>"),
            )
            .field("domain", &self.domain)
            .finish()
    }
}

impl StoredCredentials {
    /// Builds the plaintext payload from in-memory [`Credentials`], exposing the
    /// secret values only into this short-lived, drop-wiped struct.
    pub(crate) fn from_credentials(creds: &Credentials) -> Self {
        Self {
            username: creds.username.clone(),
            password: creds.expose_password().map(ToOwned::to_owned),
            key_passphrase: None,
            domain: creds.domain.clone(),
        }
    }

    /// Rebuilds in-memory [`Credentials`], moving secret strings into
    /// [`SecretString`] (which zeroizes on drop) without copying them again.
    ///
    /// A legacy `key_passphrase` stays behind and is wiped when `self` drops.
    pub(crate) fn into_credentials(mut self) -> Credentials {
        Credentials {
            username: self.username.take(),
            password: self.password.take().map(SecretString::from),
            domain: self.domain.take(),
        }
    }
}

/// Encrypted-file credential backend.
///
/// Holds only the (non-secret) path to the credential map; all secret material
/// is encrypted at rest and never kept in the struct.
pub struct EncryptedFileBackend {
    path: PathBuf,
}

impl std::fmt::Debug for EncryptedFileBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // `path` is non-secret; the backend holds no in-memory secrets.
        f.debug_struct("EncryptedFileBackend")
            .field("path", &self.path)
            .finish()
    }
}

impl Default for EncryptedFileBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl EncryptedFileBackend {
    /// Creates a backend storing into `dirs::data_dir()/rustconn/credentials.enc`.
    ///
    /// Falls back to a relative path when no data directory can be resolved; in
    /// that degenerate case store/retrieve still operate relative to the current
    /// directory rather than panicking.
    #[must_use]
    pub fn new() -> Self {
        let path = dirs::data_dir()
            .map(|dir| dir.join("rustconn").join(STORE_FILE_NAME))
            .unwrap_or_else(|| PathBuf::from(STORE_FILE_NAME));
        Self { path }
    }

    /// Creates a backend storing into an explicit path (used by tests).
    #[must_use]
    pub fn with_path(path: PathBuf) -> Self {
        Self { path }
    }
}

/// Reads and parses the on-disk credential map; a missing file is an empty map.
///
/// # Errors
/// Returns [`SecretError::RetrieveFailed`] if the file cannot be read or is not
/// valid JSON.
pub(crate) fn read_map(path: &Path) -> SecretResult<BTreeMap<String, String>> {
    match std::fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map_err(|e| SecretError::RetrieveFailed(format!("encrypted store is corrupt: {e}"))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(SecretError::RetrieveFailed(format!(
            "cannot read encrypted store: {e}"
        ))),
    }
}

/// Atomically writes the credential map: temp file + `0600` + `rename`.
///
/// The map values are ciphertext (base64 `RCSC` blobs), so the serialized JSON
/// holds no plaintext secrets.
///
/// # Errors
/// Returns [`SecretError::StoreFailed`] if any directory creation, serialization,
/// write, permission, or rename step fails.
pub(crate) fn write_map_atomic(path: &Path, map: &BTreeMap<String, String>) -> SecretResult<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent)
            .map_err(|e| SecretError::StoreFailed(format!("cannot create data directory: {e}")))?;
    }

    let json = serde_json::to_vec_pretty(map)
        .map_err(|e| SecretError::StoreFailed(format!("cannot serialize encrypted store: {e}")))?;

    // No chmod on the destination: the temp file is created 0600 and `rename`
    // gives its inode the name, so the mode is right whatever the replaced
    // file's was. The write is fsynced and the temp removed on failure.
    crate::fs_atomic::atomic_write(path, &json, STORE_FILE_MODE)
        .map_err(|e| SecretError::StoreFailed(format!("cannot write encrypted store: {e}")))
}

/// Encrypts one connection's credentials into a base64 `RCSC` blob.
///
/// # Errors
/// Returns [`SecretError::StoreFailed`] if no machine key is available or if
/// serialization/encryption fails. No secret value appears in the error.
fn encrypt_entry(creds: &Credentials) -> SecretResult<String> {
    // `get_machine_key` already returns a wiped-on-drop buffer.
    let machine_key = get_machine_key();
    if machine_key.is_empty() {
        return Err(SecretError::StoreFailed(
            "no machine key available to encrypt credentials".to_string(),
        ));
    }

    let stored = StoredCredentials::from_credentials(creds);
    // Plaintext secrets live only inside this wiped-on-drop buffer.
    let plaintext = Zeroizing::new(
        serde_json::to_vec(&stored)
            .map_err(|e| SecretError::StoreFailed(format!("cannot serialize credentials: {e}")))?,
    );
    let blob = encrypt_credential(&plaintext, &machine_key)
        .map_err(|e| SecretError::StoreFailed(format!("encryption failed: {e}")))?;
    Ok(data_encoding::BASE64.encode(&blob))
}

/// Decrypts a base64 `RCSC` blob back into in-memory credentials.
///
/// # Errors
/// Returns [`SecretError::RetrieveFailed`] if the blob is malformed, no machine
/// key is available, or decryption/parsing fails. No secret value appears in the
/// error.
fn decrypt_entry(encoded: &str) -> SecretResult<Credentials> {
    let blob = data_encoding::BASE64
        .decode(encoded.as_bytes())
        .map_err(|e| {
            SecretError::RetrieveFailed(format!("encrypted store entry is malformed: {e}"))
        })?;

    // `get_machine_key` already returns a wiped-on-drop buffer.
    let machine_key = get_machine_key();
    if machine_key.is_empty() {
        return Err(SecretError::RetrieveFailed(
            "no machine key available to decrypt credentials".to_string(),
        ));
    }

    let plaintext = decrypt_credential(&blob, &machine_key)
        .map_err(|e| SecretError::RetrieveFailed(format!("decryption failed: {e}")))?;
    // The serde error is dropped on purpose: it can quote the offending value,
    // which here is decrypted credential text.
    let stored: StoredCredentials = serde_json::from_slice(&plaintext).map_err(|_| {
        SecretError::RetrieveFailed("cannot parse decrypted credentials".to_string())
    })?;
    Ok(stored.into_credentials())
}

#[async_trait]
impl SecretBackend for EncryptedFileBackend {
    async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
        let path = self.path.clone();
        let key = connection_id.to_string();
        let creds = credentials.clone();

        tokio::task::spawn_blocking(move || {
            let encoded = encrypt_entry(&creds)?;
            let mut map = read_map(&path)?;
            map.insert(key, encoded);
            write_map_atomic(&path, &map)?;
            tracing::debug!(backend = "encrypted_file", "stored credential entry");
            Ok(())
        })
        .await
        .map_err(|e| SecretError::StoreFailed(format!("encrypted store task panicked: {e}")))?
    }

    async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>> {
        let path = self.path.clone();
        let key = connection_id.to_string();

        tokio::task::spawn_blocking(move || {
            let map = read_map(&path)?;
            match map.get(&key) {
                Some(encoded) => decrypt_entry(encoded).map(Some),
                None => Ok(None),
            }
        })
        .await
        .map_err(|e| SecretError::RetrieveFailed(format!("encrypted store task panicked: {e}")))?
    }

    async fn delete(&self, connection_id: &str) -> SecretResult<()> {
        let path = self.path.clone();
        let key = connection_id.to_string();

        tokio::task::spawn_blocking(move || {
            let mut map = read_map(&path)?;
            // Remove only this connection's entry; leave the rest intact (Req 6.5).
            if map.remove(&key).is_some() {
                write_map_atomic(&path, &map)?;
                tracing::debug!(backend = "encrypted_file", "deleted credential entry");
            }
            Ok(())
        })
        .await
        .map_err(|e| SecretError::DeleteFailed(format!("encrypted store task panicked: {e}")))?
    }

    async fn is_available(&self) -> bool {
        // Available whenever a machine key can be derived/created (Req 5.3) —
        // holds on Linux, BSD, macOS, headless, and inside Flatpak/Snap.
        tokio::task::spawn_blocking(|| !get_machine_key().is_empty())
            .await
            .unwrap_or(false)
    }

    fn backend_id(&self) -> &'static str {
        "encrypted_file"
    }

    fn display_name(&self) -> &'static str {
        "Encrypted file — no system keyring required"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The private `StoredCredentials` redacts its secret fields in `Debug`: a
    /// sentinel password/passphrase must not appear in the rendered string, while
    /// non-secret fields (username/domain) remain visible.
    #[test]
    fn stored_credentials_debug_redacts_secrets() {
        const SENTINEL_PW: &str = "StoredCreds-LEAK-pw-001";
        const SENTINEL_PASS: &str = "StoredCreds-LEAK-pass-002";

        let stored = StoredCredentials {
            username: Some("carol".to_string()),
            password: Some(SENTINEL_PW.to_string()),
            key_passphrase: Some(SENTINEL_PASS.to_string()),
            domain: Some("EXAMPLE".to_string()),
        };

        let rendered = format!("{stored:?}");
        assert!(
            !rendered.contains(SENTINEL_PW),
            "StoredCredentials Debug leaked the password: {rendered}"
        );
        assert!(
            !rendered.contains(SENTINEL_PASS),
            "StoredCredentials Debug leaked the passphrase: {rendered}"
        );
        // Non-secret fields and the redaction marker are present.
        assert!(rendered.contains("carol"));
        assert!(rendered.contains("EXAMPLE"));
        assert!(rendered.contains("<redacted>"));
    }

    /// The plaintext an entry is encrypted from never carries a key
    /// passphrase field.
    #[test]
    fn stored_payload_has_no_key_passphrase_field() {
        let creds = Credentials::with_password("carol", "pw");
        let json = serde_json::to_string(&StoredCredentials::from_credentials(&creds)).unwrap();
        assert!(!json.contains("key_passphrase"), "{json}");
        assert!(json.contains("carol"), "{json}");
    }

    /// An entry written by a release that stored a key passphrase still
    /// decrypts; the passphrase does not reach the credentials, and saving
    /// them again writes an entry without it.
    #[test]
    fn a_legacy_key_passphrase_is_read_past_and_not_written_back() {
        const SENTINEL_PASS: &str = "StoredCreds-LEGACY-pass-003";
        let legacy = format!(
            r#"{{"username":"carol","password":"pw","key_passphrase":"{SENTINEL_PASS}","domain":"EXAMPLE"}}"#
        );

        let stored: StoredCredentials = serde_json::from_str(&legacy).unwrap();
        let creds = stored.into_credentials();

        assert_eq!(creds.username.as_deref(), Some("carol"));
        assert_eq!(creds.expose_password(), Some("pw"));
        assert_eq!(creds.domain.as_deref(), Some("EXAMPLE"));
        assert!(!format!("{creds:?}").contains(SENTINEL_PASS));
        let rewritten =
            serde_json::to_string(&StoredCredentials::from_credentials(&creds)).unwrap();
        assert!(!rewritten.contains("key_passphrase"), "{rewritten}");
        assert!(!rewritten.contains(SENTINEL_PASS), "{rewritten}");
    }
}
