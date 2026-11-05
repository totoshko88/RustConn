//! Credential migration between the machine-bound and portable file backends.
//!
//! Bulk-transfers credentials between [`super::EncryptedFileBackend`]'s
//! machine-key store and [`super::PortableEncryptedFileBackend`]'s
//! passphrase-protected store, in both directions.
//!
//! Both file formats are read and written through their owning modules'
//! helpers rather than restated here. An earlier version carried its own copy
//! of the portable store struct, which meant the header validation added to the
//! real one (format version, KDF name, key-derivation cost ceilings) did not
//! apply to migration — the one path that reads a file the user just pointed at
//! from a shared folder.
//!
//! Migration is lossless: every entry is decrypted and re-encrypted with the
//! destination's key. Entries that fail are reported, never silently dropped,
//! and `delete_source` only removes what actually arrived.

use std::path::Path;

use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

use super::encrypted_file::{read_map, write_map_atomic};
use super::local_crypto::{decrypt_credential, encrypt_credential, get_machine_key};
use super::portable_encrypted_file::{
    PortableStoreFile, open_entry, read_store, seal_entry, write_store_atomic,
};
use crate::error::{SecretError, SecretResult};

/// Result of a migration operation.
#[derive(Debug, Clone)]
pub struct MigrationResult {
    /// Number of entries successfully migrated.
    pub migrated: usize,
    /// Entries that failed, as `(entry key, error description)`.
    ///
    /// The key is a connection lookup key, never a secret.
    pub failures: Vec<(String, String)>,
}

impl MigrationResult {
    /// Whether all entries were migrated without errors.
    #[must_use]
    pub fn is_complete(&self) -> bool {
        self.failures.is_empty()
    }

    /// Total entries attempted (migrated + failed).
    #[must_use]
    pub fn total(&self) -> usize {
        self.migrated + self.failures.len()
    }
}

/// Counts entries in the machine-bound encrypted store at `path`.
///
/// Needs no key: the map keys are not encrypted. A missing file counts as zero.
/// The Settings page uses this to decide whether there is anything to offer to
/// migrate.
///
/// # Errors
/// Returns [`SecretError::RetrieveFailed`] if the file is unreadable or corrupt.
pub fn encrypted_entry_count(path: &Path) -> SecretResult<usize> {
    Ok(read_map(path)?.len())
}

/// Loads the machine key, refusing to proceed without one.
///
/// # Errors
/// Returns [`SecretError::StoreFailed`] when no machine key can be derived, in
/// which case the machine-bound store can be neither read nor written.
fn require_machine_key() -> SecretResult<Zeroizing<Vec<u8>>> {
    // `get_machine_key` already returns a wiped-on-drop buffer.
    let key = get_machine_key();
    if key.is_empty() {
        return Err(SecretError::StoreFailed(
            "no machine key available — cannot read or write the machine-bound store".to_string(),
        ));
    }
    Ok(key)
}

/// Migrates all credentials from the machine-bound store to a portable one.
///
/// Each entry in `source_path` is decrypted with the machine key and re-encrypted
/// under the portable store's data-encryption key, which `passphrase` unlocks.
/// A destination that does not exist yet is created under `passphrase`; an
/// existing one must open with it, so a mistyped passphrase cannot append
/// entries that the rest of the file's key cannot read.
///
/// `delete_source` removes the migrated entries from `source_path` after the
/// destination has been written successfully. Entries that failed to migrate are
/// always left in place.
///
/// # Errors
/// Returns [`SecretError::IncorrectPassphrase`] if `passphrase` does not open an
/// existing destination, or a `SecretError` if either file cannot be read or
/// written.
pub fn migrate_encrypted_to_portable(
    source_path: &Path,
    dest_path: &Path,
    passphrase: &SecretString,
    delete_source: bool,
) -> SecretResult<MigrationResult> {
    let machine_key = require_machine_key()?;
    let pass = Zeroizing::new(passphrase.expose_secret().as_bytes().to_vec());

    let source_map = read_map(source_path)?;
    if source_map.is_empty() {
        return Ok(MigrationResult {
            migrated: 0,
            failures: Vec::new(),
        });
    }

    // Open (or create) the destination once. The KEK/DEK split means this is the
    // only key derivation the whole batch pays for.
    let (mut dest_store, dek) = PortableStoreFile::open_or_create_for_write(dest_path, &pass)?;

    let mut migrated_keys: Vec<String> = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();

    for (key, encoded_blob) in &source_map {
        match migrate_entry_to_portable(key, encoded_blob, &machine_key, &dek) {
            Ok(new_blob) => {
                dest_store.entries.insert(key.clone(), new_blob);
                migrated_keys.push(key.clone());
            }
            Err(e) => failures.push((key.clone(), e)),
        }
    }

    write_store_atomic(dest_path, &dest_store)?;

    // Only now that the destination is durable may the source shed entries, and
    // only the ones that actually arrived. Removing the failures instead — which
    // an earlier version did, against its own comment — destroyed exactly the
    // credentials that had nowhere else to live.
    if delete_source && !migrated_keys.is_empty() {
        let mut remaining = source_map;
        for key in &migrated_keys {
            remaining.remove(key);
        }
        write_map_atomic(source_path, &remaining)?;
    }

    Ok(MigrationResult {
        migrated: migrated_keys.len(),
        failures,
    })
}

/// Migrates all credentials from a portable store to the machine-bound store.
///
/// Each entry in `source_path` is decrypted with the portable store's key, which
/// `passphrase` unlocks, and re-encrypted with the machine key.
///
/// `delete_source` removes the migrated entries from `source_path` after the
/// destination has been written successfully. Entries that failed to migrate are
/// always left in place.
///
/// # Errors
/// Returns [`SecretError::IncorrectPassphrase`] if `passphrase` does not open the
/// source, or a `SecretError` if either file cannot be read or written.
pub fn migrate_portable_to_encrypted(
    source_path: &Path,
    dest_path: &Path,
    passphrase: &SecretString,
    delete_source: bool,
) -> SecretResult<MigrationResult> {
    let machine_key = require_machine_key()?;
    let pass = Zeroizing::new(passphrase.expose_secret().as_bytes().to_vec());

    let Some(mut source_store) = read_store(source_path)? else {
        return Ok(MigrationResult {
            migrated: 0,
            failures: Vec::new(),
        });
    };
    if source_store.entries.is_empty() {
        return Ok(MigrationResult {
            migrated: 0,
            failures: Vec::new(),
        });
    }

    let dek = source_store.unlock(&pass)?;
    let mut dest_map = read_map(dest_path)?;

    let mut migrated_keys: Vec<String> = Vec::new();
    let mut failures: Vec<(String, String)> = Vec::new();

    for (key, encoded_blob) in &source_store.entries {
        match migrate_entry_to_encrypted(key, encoded_blob, &dek, &machine_key) {
            Ok(new_blob) => {
                dest_map.insert(key.clone(), new_blob);
                migrated_keys.push(key.clone());
            }
            Err(e) => failures.push((key.clone(), e)),
        }
    }

    write_map_atomic(dest_path, &dest_map)?;

    if delete_source && !migrated_keys.is_empty() {
        for key in &migrated_keys {
            source_store.entries.remove(key);
        }
        write_store_atomic(source_path, &source_store)?;
    }

    Ok(MigrationResult {
        migrated: migrated_keys.len(),
        failures,
    })
}

// ──────────────────────────────────────────────────────────────────────────────
// Per-entry re-encryption
// ──────────────────────────────────────────────────────────────────────────────

/// Re-encrypts one machine-key entry under the portable store's data key.
///
/// Goes through [`Credentials`](crate::models::Credentials) rather than moving
/// the raw plaintext across, so a payload the destination cannot represent fails
/// here instead of being written back as something unreadable.
fn migrate_entry_to_portable(
    key: &str,
    encoded: &str,
    machine_key: &[u8],
    dek: &Zeroizing<[u8; 32]>,
) -> Result<String, String> {
    let blob = data_encoding::BASE64
        .decode(encoded.as_bytes())
        .map_err(|e| format!("base64 decode: {e}"))?;
    let plaintext = decrypt_credential(&blob, machine_key)?;
    // The serde error is dropped rather than reported: this is decrypted
    // plaintext, and serde quotes the offending value, so the message could
    // carry a fragment of the credential into the migration report.
    let stored: super::encrypted_file::StoredCredentials = serde_json::from_slice(&plaintext)
        .map_err(|_| "entry is not in a known format".to_owned())?;
    // The entry keeps its name across the move: the portable store authenticates
    // the name alongside the blob, so sealing it under anything else would
    // produce an entry that cannot be opened where it is filed.
    seal_entry(dek, key, &stored.into_credentials()).map_err(|e| e.to_string())
}

/// Re-encrypts one portable entry under the machine key.
fn migrate_entry_to_encrypted(
    key: &str,
    encoded: &str,
    dek: &Zeroizing<[u8; 32]>,
    machine_key: &[u8],
) -> Result<String, String> {
    let creds = open_entry(dek, key, encoded).map_err(|e| e.to_string())?;
    let stored = super::encrypted_file::StoredCredentials::from_credentials(&creds);
    let plaintext = Zeroizing::new(
        serde_json::to_vec(&stored).map_err(|e| format!("cannot serialize entry: {e}"))?,
    );
    let new_blob = encrypt_credential(&plaintext, machine_key)?;
    Ok(data_encoding::BASE64.encode(&new_blob))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::Credentials;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    /// Writes a machine-key store containing one entry per supplied name.
    fn seed_encrypted_store(path: &Path, names: &[&str]) -> Zeroizing<Vec<u8>> {
        let machine_key = get_machine_key();
        let mut map = BTreeMap::new();
        for name in names {
            let creds = Credentials {
                username: Some(format!("user-{name}")),
                password: Some(SecretString::from(format!("pw-{name}"))),
                domain: None,
            };
            let stored = super::super::encrypted_file::StoredCredentials::from_credentials(&creds);
            let plaintext = serde_json::to_vec(&stored).unwrap();
            let blob = encrypt_credential(&plaintext, &machine_key).unwrap();
            map.insert((*name).to_string(), data_encoding::BASE64.encode(&blob));
        }
        write_map_atomic(path, &map).unwrap();
        machine_key
    }

    #[test]
    fn round_trip_encrypted_to_portable_and_back() {
        if get_machine_key().is_empty() {
            // No XDG data dir and no /etc/machine-id: the machine-bound half of
            // this test cannot run.
            return;
        }
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("credentials.enc");
        let portable = dir.path().join("credentials-portable.enc");
        seed_encrypted_store(&source, &["rustconn/test-server"]);

        let passphrase = SecretString::from("test-passphrase".to_owned());

        let result = migrate_encrypted_to_portable(&source, &portable, &passphrase, false).unwrap();
        assert_eq!(result.migrated, 1);
        assert!(result.is_complete());
        assert_eq!(
            super::super::portable_encrypted_file::entry_count(&portable).unwrap(),
            1
        );

        // The source is untouched when delete_source is false.
        assert_eq!(encrypted_entry_count(&source).unwrap(), 1);

        let dest2 = dir.path().join("credentials2.enc");
        let back = migrate_portable_to_encrypted(&portable, &dest2, &passphrase, false).unwrap();
        assert_eq!(back.migrated, 1);
        assert!(back.is_complete());

        // The credential survived both hops intact.
        let map = read_map(&dest2).unwrap();
        let blob = data_encoding::BASE64
            .decode(map["rustconn/test-server"].as_bytes())
            .unwrap();
        let plaintext = decrypt_credential(&blob, &get_machine_key()).unwrap();
        let stored: super::super::encrypted_file::StoredCredentials =
            serde_json::from_slice(&plaintext).unwrap();
        let creds = stored.into_credentials();
        assert_eq!(creds.username.as_deref(), Some("user-rustconn/test-server"));
        assert_eq!(creds.expose_password(), Some("pw-rustconn/test-server"));
    }

    /// `delete_source` must remove exactly what reached the destination. The
    /// regression this guards: an earlier version removed the *failures*, so a
    /// credential that could not be migrated was deleted from the only file that
    /// still held it.
    #[test]
    fn delete_source_keeps_entries_that_failed_to_migrate() {
        if get_machine_key().is_empty() {
            return;
        }
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("credentials.enc");
        let portable = dir.path().join("portable.enc");
        seed_encrypted_store(&source, &["good-one", "good-two"]);

        // Add an entry that cannot be decrypted, standing in for any per-entry
        // failure (corruption, a blob from another machine).
        let mut map = read_map(&source).unwrap();
        map.insert(
            "broken".to_owned(),
            data_encoding::BASE64.encode(b"not an RCSC blob at all"),
        );
        write_map_atomic(&source, &map).unwrap();

        let passphrase = SecretString::from("pass".to_owned());
        let result = migrate_encrypted_to_portable(&source, &portable, &passphrase, true).unwrap();

        assert_eq!(result.migrated, 2);
        assert_eq!(result.failures.len(), 1);
        assert_eq!(result.failures[0].0, "broken");
        assert_eq!(result.total(), 3);

        let remaining = read_map(&source).unwrap();
        assert!(
            remaining.contains_key("broken"),
            "the entry that failed to migrate must survive in the source"
        );
        assert!(
            !remaining.contains_key("good-one") && !remaining.contains_key("good-two"),
            "migrated entries should be gone from the source, got {:?}",
            remaining.keys().collect::<Vec<_>>()
        );
    }

    /// Appending to an existing portable store under the wrong passphrase would
    /// leave a file half of which cannot be read with either passphrase.
    #[test]
    fn migrating_into_an_existing_store_requires_the_right_passphrase() {
        if get_machine_key().is_empty() {
            return;
        }
        let dir = TempDir::new().unwrap();
        let source = dir.path().join("credentials.enc");
        let portable = dir.path().join("portable.enc");
        seed_encrypted_store(&source, &["first"]);

        let right = SecretString::from("right-passphrase".to_owned());
        migrate_encrypted_to_portable(&source, &portable, &right, false).unwrap();

        seed_encrypted_store(&source, &["second"]);
        let wrong = SecretString::from("wrong-passphrase".to_owned());
        let result = migrate_encrypted_to_portable(&source, &portable, &wrong, false);

        assert!(
            matches!(result, Err(SecretError::IncorrectPassphrase)),
            "expected the wrong passphrase to be refused, got {result:?}"
        );
    }

    #[test]
    fn empty_source_is_a_no_op() {
        let dir = TempDir::new().unwrap();
        let missing = dir.path().join("absent.enc");
        let portable = dir.path().join("portable.enc");
        let passphrase = SecretString::from("pass".to_owned());

        if get_machine_key().is_empty() {
            return;
        }
        let result = migrate_encrypted_to_portable(&missing, &portable, &passphrase, true).unwrap();
        assert_eq!(result.migrated, 0);
        assert!(result.is_complete());
        assert!(!portable.exists(), "no destination is created for no work");

        let back = migrate_portable_to_encrypted(&missing, &portable, &passphrase, true).unwrap();
        assert_eq!(back.migrated, 0);
    }
}
