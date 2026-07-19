//! Shared local (application-managed) credential crypto.
//!
//! This module holds the AES-256-GCM credential encryption used by both the
//! `config.toml` `*_encrypted` master-secret fields (via [`crate::config`]) and
//! the encrypted-file secret backend. It was extracted from
//! `config/settings.rs` so the two callers share one implementation.
//!
//! The on-disk blob format and the machine-key file location are preserved
//! byte-for-byte from the original `settings.rs` implementation:
//!
//! Output format: `RCSC` (4) + version (1) + salt (16) + nonce (12) +
//! ciphertext + tag (16).
//!
//! All intermediate plaintext buffers are wrapped in [`Zeroizing`] so they are
//! wiped on drop. Functions are `pub(crate)` — this is an internal crate helper,
//! not a public API.

use zeroize::Zeroizing;

/// Magic bytes identifying AES-256-GCM encrypted credentials (v1)
pub(crate) const SETTINGS_CRYPTO_MAGIC: &[u8] = b"RCSC";

/// Current settings crypto version
pub(crate) const SETTINGS_CRYPTO_VERSION: u8 = 1;

/// Salt length for Argon2id key derivation
pub(crate) const SETTINGS_SALT_LEN: usize = 16;

/// Nonce length for AES-256-GCM
pub(crate) const SETTINGS_NONCE_LEN: usize = 12;

/// Header length: magic(4) + version(1) + salt(16) + nonce(12)
pub(crate) const SETTINGS_HEADER_LEN: usize = 4 + 1 + SETTINGS_SALT_LEN + SETTINGS_NONCE_LEN;

/// Gets a machine-specific key for encryption.
///
/// Uses an app-specific key file, `/etc/machine-id`, or returns empty.
/// In a Flatpak sandbox `/etc/machine-id` is inaccessible, so we first try an
/// app-specific key file stored in the XDG data directory.
pub(crate) fn get_machine_key() -> Vec<u8> {
    /// Key length type for HKDF output (32 bytes = 256 bits)
    struct HkdfKeyLen;
    impl ring::hkdf::KeyType for HkdfKeyLen {
        fn len(&self) -> usize {
            32
        }
    }

    // 1. Try app-specific key file in XDG data dir (works in Flatpak)
    if let Some(data_dir) = dirs::data_dir() {
        let key_file = data_dir.join("rustconn").join(".machine-key");
        if let Ok(key) = std::fs::read_to_string(&key_file) {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                return trimmed.as_bytes().to_vec();
            }
        }
        // Generate and persist a random key if it doesn't exist
        if std::fs::create_dir_all(data_dir.join("rustconn")).is_ok() {
            let key = uuid::Uuid::new_v4().to_string();
            if std::fs::write(&key_file, &key).is_ok() {
                // Restrict permissions to owner-only (0600)
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    let _ =
                        std::fs::set_permissions(&key_file, std::fs::Permissions::from_mode(0o600));
                }
                return key.into_bytes();
            }
        }
    }

    // 2. Try /etc/machine-id with HKDF derivation (works outside Flatpak)
    if let Ok(machine_id) = std::fs::read_to_string("/etc/machine-id") {
        let trimmed = machine_id.trim();
        if !trimmed.is_empty() {
            // Derive app-specific key via HKDF to avoid sharing raw machine-id
            let salt = ring::hkdf::Salt::new(ring::hkdf::HKDF_SHA256, b"rustconn-machine-key-v1");
            let prk = salt.extract(trimmed.as_bytes());
            if let Ok(okm) = prk.expand(&[b"encryption" as &[u8]], HkdfKeyLen) {
                let mut derived = vec![0u8; 32];
                if okm.fill(&mut derived).is_ok() {
                    return derived;
                }
            }
            // If HKDF fails, use raw machine-id as before
            return trimmed.as_bytes().to_vec();
        }
    }

    // 3. No fallback — refuse to encrypt with predictable key
    tracing::error!(
        "Cannot derive encryption key: no .machine-key file and /etc/machine-id unavailable. \
         Credential encryption will not work."
    );
    Vec::new()
}

/// Encrypts credential data using AES-256-GCM with Argon2id key derivation.
///
/// Output format: `RCSC` (4) + version (1) + salt (16) + nonce (12) +
/// ciphertext + tag (16).
///
/// # Errors
/// Returns an error string if salt/nonce generation, key creation, or the
/// AES-GCM seal operation fails.
pub(crate) fn encrypt_credential(plaintext: &[u8], machine_key: &[u8]) -> Result<Vec<u8>, String> {
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};
    use ring::rand::{SecureRandom, SystemRandom};

    let rng = SystemRandom::new();

    let mut salt = [0u8; SETTINGS_SALT_LEN];
    rng.fill(&mut salt)
        .map_err(|_| "Failed to generate salt".to_string())?;

    let mut nonce_bytes = [0u8; SETTINGS_NONCE_LEN];
    rng.fill(&mut nonce_bytes)
        .map_err(|_| "Failed to generate nonce".to_string())?;

    let key = derive_settings_key(machine_key, &salt)?;

    let unbound_key = UnboundKey::new(&AES_256_GCM, &key)
        .map_err(|_| "Failed to create encryption key".to_string())?;
    let less_safe_key = LessSafeKey::new(unbound_key);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    // Plaintext is encrypted in place; `seal_in_place_append_tag` overwrites it
    // with ciphertext and appends the tag, so the plaintext does not outlive the
    // call. A plain `Vec` is required because ring needs `Extend<&u8>` for the
    // appended tag (which `Zeroizing` does not implement).
    let mut in_out = plaintext.to_vec();
    less_safe_key
        .seal_in_place_append_tag(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| "Encryption failed".to_string())?;

    let mut result = Vec::with_capacity(SETTINGS_HEADER_LEN + in_out.len());
    result.extend_from_slice(SETTINGS_CRYPTO_MAGIC);
    result.push(SETTINGS_CRYPTO_VERSION);
    result.extend_from_slice(&salt);
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&in_out);
    Ok(result)
}

/// Decrypts AES-256-GCM credential data, dispatching on the `RCSC` header.
///
/// # Errors
/// Returns an error if the data lacks the `RCSC` magic header (e.g. the
/// long-removed legacy XOR format) or if AES-GCM decryption fails. The legacy
/// XOR fallback was removed in 0.17.0 — it provided no real protection and the
/// transparent migration window (since v0.12) has long passed.
pub(crate) fn decrypt_credential(
    data: &[u8],
    machine_key: &[u8],
) -> Result<Zeroizing<Vec<u8>>, String> {
    if data.len() >= SETTINGS_HEADER_LEN && data[..4] == *SETTINGS_CRYPTO_MAGIC {
        decrypt_credential_aes(data, machine_key)
    } else {
        tracing::warn!(
            "Stored credential is not in AES-256-GCM (RCSC) format and cannot be \
             decrypted; legacy XOR support was removed in 0.17.0 — re-enter the \
             credential in Settings."
        );
        Err("unrecognized credential format (legacy XOR no longer supported)".to_string())
    }
}

/// Decrypts AES-256-GCM encrypted credential data.
///
/// Returns the recovered plaintext wrapped in [`Zeroizing`].
///
/// # Errors
/// Returns an error if the input is too short, the nonce is malformed, key
/// derivation fails, or AES-GCM authentication fails.
pub(crate) fn decrypt_credential_aes(
    data: &[u8],
    machine_key: &[u8],
) -> Result<Zeroizing<Vec<u8>>, String> {
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};

    if data.len() < SETTINGS_HEADER_LEN + 16 {
        return Err("Encrypted data too short".to_string());
    }

    let _version = data[4];
    let salt = &data[5..5 + SETTINGS_SALT_LEN];
    let nonce_bytes: [u8; SETTINGS_NONCE_LEN] = data
        [5 + SETTINGS_SALT_LEN..5 + SETTINGS_SALT_LEN + SETTINGS_NONCE_LEN]
        .try_into()
        .map_err(|_| "Invalid nonce".to_string())?;
    let ciphertext = &data[SETTINGS_HEADER_LEN..];

    let key = derive_settings_key(machine_key, salt)?;

    let unbound_key = UnboundKey::new(&AES_256_GCM, &key)
        .map_err(|_| "Failed to create decryption key".to_string())?;
    let less_safe_key = LessSafeKey::new(unbound_key);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    // Plaintext-bearing buffer wiped on drop.
    let mut in_out = Zeroizing::new(ciphertext.to_vec());
    less_safe_key
        .open_in_place(nonce, Aad::empty(), &mut in_out)
        .map_err(|_| "Decryption failed (wrong key or corrupted data)".to_string())?;

    // Remove the authentication tag (last 16 bytes). Bind the length first
    // because `Zeroizing`'s `Deref` prevents the two-phase borrow that a plain
    // `Vec` would allow in `truncate(self.len() - 16)`.
    let plaintext_len = in_out.len() - 16;
    in_out.truncate(plaintext_len);
    Ok(in_out)
}

/// Derives a 256-bit key from the machine key using Argon2id.
///
/// Uses lighter parameters than document encryption since settings encryption
/// happens on every save and the key material is already high-entropy
/// (machine-specific UUID or machine-id).
///
/// # Errors
/// Returns an error string if the Argon2 parameters are invalid or key
/// derivation fails.
pub(crate) fn derive_settings_key(machine_key: &[u8], salt: &[u8]) -> Result<[u8; 32], String> {
    use argon2::{Algorithm, Argon2, Params, Version};

    // Lighter params: 16 MiB memory, 2 iterations, 1 thread
    // Appropriate for machine-key derivation (not user passwords)
    let params = Params::new(16 * 1024, 2, 1, Some(32))
        .map_err(|e| format!("Invalid Argon2 params: {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    let mut key = [0u8; 32];
    argon2
        .hash_password_into(machine_key, salt, &mut key)
        .map_err(|e| format!("Key derivation failed: {e}"))?;
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real `RCSC` blob captured ONCE from the pre-refactor `settings.rs`
    /// crypto, encrypting [`FIXTURE_PLAINTEXT`] under [`FIXTURE_MACHINE_KEY`].
    ///
    /// This is a backward-compatibility regression fixture: it locks in the
    /// on-disk format (magic + version + salt + nonce + ciphertext + tag), the
    /// AES-256-GCM cipher, and the Argon2id KDF parameters (16 MiB / 2 iters /
    /// 1 thread / V0x13). It MUST NOT be casually regenerated — re-minting it
    /// with the current code would silently track any format change and defeat
    /// the regression check. If decrypting this fixture ever fails, the stored
    /// credentials of existing users would also fail to decrypt (Req 14.1).
    const FIXTURE_BLOB_HEX: &str = "52435343012418b7ef093afd64be602ec75808fc685158eb4261e2377fdd0e147450a1ec50a38938500ccbb3fdf9c10cc4ac909139d940393a129676f1cc72c9a2f46b6b577ec3457ed8b6389000d115eb0daadd069fcc2854bb";

    /// Fixed machine key the fixture was encrypted under (see [`FIXTURE_BLOB_HEX`]).
    const FIXTURE_MACHINE_KEY: &[u8] = b"rustconn-regression-fixed-machine-key-v1";

    /// Known plaintext the fixture decrypts back to (see [`FIXTURE_BLOB_HEX`]).
    const FIXTURE_PLAINTEXT: &[u8] = b"{\"username\":\"alice\",\"password\":\"hunter2\"}";

    fn hex_to_bytes(hex: &str) -> Vec<u8> {
        (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).expect("valid hex"))
            .collect()
    }

    /// Decrypting a pre-refactor `RCSC` blob still yields the original plaintext.
    ///
    /// Proves the extracted crypto reads blobs produced by the old `settings.rs`
    /// format byte-for-byte (Requirement 14.1 — no silent data loss on upgrade).
    #[test]
    fn decrypts_pre_refactor_fixture_blob() {
        let blob = hex_to_bytes(FIXTURE_BLOB_HEX);

        // Sanity-check the on-disk layout is unchanged before decrypting.
        assert_eq!(&blob[..4], SETTINGS_CRYPTO_MAGIC, "magic must be RCSC");
        assert_eq!(blob[4], SETTINGS_CRYPTO_VERSION, "version must be 1");
        assert!(
            blob.len() >= SETTINGS_HEADER_LEN + 16,
            "blob must contain header + tag"
        );

        let recovered = decrypt_credential(&blob, FIXTURE_MACHINE_KEY)
            .expect("pre-refactor fixture must still decrypt");
        assert_eq!(&recovered[..], FIXTURE_PLAINTEXT);
    }
}
