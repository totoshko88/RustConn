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
///
/// The result is cached in a process-wide `OnceLock` so the key is stable for
/// the lifetime of the process, avoiding race conditions when the key file is
/// first generated (parallel threads would otherwise each generate a different
/// UUID, with the last writer winning and earlier encryptions becoming
/// undecryptable).
/// The cached copy lives for the process lifetime — that is the point of the
/// cache, and it is why the *clone* handed to each caller is what gets wiped.
/// Returning `Zeroizing` means a caller cannot accidentally leave key bytes in a
/// buffer that outlives the operation; previously every call site had to
/// remember to wrap it, and two of them did not.
pub(crate) fn get_machine_key() -> Zeroizing<Vec<u8>> {
    static CACHED_KEY: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    Zeroizing::new(CACHED_KEY.get_or_init(derive_machine_key_inner).clone())
}

/// Inner implementation of machine key derivation (called once per process).
fn derive_machine_key_inner() -> Vec<u8> {
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

    let unbound_key = UnboundKey::new(&AES_256_GCM, key.as_ref())
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

    let unbound_key = UnboundKey::new(&AES_256_GCM, key.as_ref())
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
/// Uses lighter Argon2 parameters than credential hashing since settings
/// encryption happens on every save and the key material is already high-entropy
/// (machine-specific UUID or machine-id).
///
/// # Errors
/// Returns an error string if the Argon2 parameters are invalid or key
/// derivation fails.
pub(crate) fn derive_settings_key(
    machine_key: &[u8],
    salt: &[u8],
) -> Result<Zeroizing<[u8; 32]>, String> {
    use argon2::{Algorithm, Argon2, Params, Version};

    // Lighter params: 16 MiB memory, 2 iterations, 1 thread
    // Appropriate for machine-key derivation (not user passwords)
    let params = Params::new(16 * 1024, 2, 1, Some(32))
        .map_err(|e| format!("Invalid Argon2 params: {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);

    // Wiped on drop, matching `derive_passphrase_key` below. Both produce key
    // material of the same sensitivity; only one of them used to say so.
    let mut key = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(machine_key, salt, key.as_mut())
        .map_err(|e| format!("Key derivation failed: {e}"))?;
    Ok(key)
}

/// Parameters for passphrase-based key derivation (portable encrypted file).
///
/// Stronger than [`derive_settings_key`] because the input is a user-chosen
/// passphrase with limited entropy rather than a machine-specific UUID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[expect(
    clippy::struct_field_names,
    reason = "m_cost/t_cost/p_cost are Argon2's own parameter names and are serialised \
              under them; renaming would break the on-disk header"
)]
pub(crate) struct PassphraseKdfParams {
    /// Memory cost in KiB (default: 64 MiB = 65536 KiB).
    pub m_cost: u32,
    /// Time cost / iterations (default: 3).
    pub t_cost: u32,
    /// Parallelism (default: 4).
    pub p_cost: u32,
}

impl Default for PassphraseKdfParams {
    fn default() -> Self {
        Self {
            // 64 MiB memory, 3 iterations, 4 threads — balance between
            // security (brute-force resistance) and unlock latency (~0.5 s on
            // modern hardware). These defaults are stored in the file header so
            // future versions can raise them without breaking existing files.
            m_cost: 64 * 1024,
            t_cost: 3,
            p_cost: 4,
        }
    }
}

/// Derives a 256-bit AES key from a user passphrase using Argon2id.
///
/// Uses stronger parameters than [`derive_settings_key`] because the input is
/// a human-chosen passphrase rather than a high-entropy machine key.
///
/// # Errors
/// Returns an error string if the Argon2 parameters are invalid or key
/// derivation fails.
pub(crate) fn derive_passphrase_key(
    passphrase: &[u8],
    salt: &[u8],
    params: &PassphraseKdfParams,
) -> Result<Zeroizing<[u8; 32]>, String> {
    use argon2::{Algorithm, Argon2, Params, Version};

    let argon_params = Params::new(params.m_cost, params.t_cost, params.p_cost, Some(32))
        .map_err(|e| format!("Invalid Argon2 passphrase params: {e}"))?;
    let argon2 = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);

    let mut key = Zeroizing::new([0u8; 32]);
    argon2
        .hash_password_into(passphrase, salt, key.as_mut())
        .map_err(|e| format!("Passphrase key derivation failed: {e}"))?;
    Ok(key)
}

/// Fills `buf` with cryptographically secure random bytes.
///
/// # Errors
/// Returns an error string if the system RNG refuses to produce bytes.
pub(crate) fn fill_random(buf: &mut [u8]) -> Result<(), String> {
    use ring::rand::{SecureRandom, SystemRandom};
    SystemRandom::new()
        .fill(buf)
        .map_err(|_| "Failed to generate random bytes".to_string())
}

/// AAD context label for the wrapped data-encryption key in a portable store.
pub(crate) const AAD_WRAPPED_KEY: &[u8] = b"rustconn-portable-dek-v1";

/// AAD context label for a credential entry in a portable store.
pub(crate) const AAD_ENTRY: &[u8] = b"rustconn-portable-entry-v1";

/// Seals `plaintext` under a caller-supplied 256-bit key.
///
/// Output layout is `nonce (12) + ciphertext + tag (16)` — no magic, no salt and
/// no KDF, because the key is already a key. This is the primitive the portable
/// store uses for both the wrapped data-encryption key and every entry, so a
/// store operation costs one AES-GCM pass rather than an Argon2 derivation.
///
/// `aad` binds the blob to its role ([`AAD_WRAPPED_KEY`] or [`AAD_ENTRY`]).
/// Without it the two blob kinds are structurally identical, so anyone able to
/// edit the JSON could move a wrapped key into the entry map (or the reverse)
/// and the ciphertext would still authenticate.
///
/// # Errors
/// Returns an error if nonce generation or AES-GCM sealing fails.
pub(crate) fn seal_with_key(
    key: &[u8; 32],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};

    let mut nonce_bytes = [0u8; SETTINGS_NONCE_LEN];
    fill_random(&mut nonce_bytes)?;

    let unbound_key = UnboundKey::new(&AES_256_GCM, key)
        .map_err(|_| "Failed to create encryption key".to_string())?;
    let less_safe_key = LessSafeKey::new(unbound_key);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    // Sealed in place: `seal_in_place_append_tag` overwrites the plaintext copy
    // with ciphertext, so it does not outlive the call. `Zeroizing` covers the
    // path where it does *not* overwrite anything — on a seal error the buffer
    // would otherwise be dropped still holding the plaintext, which here is a
    // data-encryption key or a credential.
    let mut in_out = Zeroizing::new(plaintext.to_vec());
    less_safe_key
        .seal_in_place_append_tag(nonce, Aad::from(aad), &mut *in_out)
        .map_err(|_| "Encryption failed".to_string())?;

    let mut result = Vec::with_capacity(SETTINGS_NONCE_LEN + in_out.len());
    result.extend_from_slice(&nonce_bytes);
    result.extend_from_slice(&in_out);
    Ok(result)
}

/// Opens a blob produced by [`seal_with_key`] under the same key and `aad`.
///
/// # Errors
/// Returns an error if the blob is shorter than a nonce plus tag, or if AES-GCM
/// authentication fails (wrong key, wrong `aad`, or corrupted data).
pub(crate) fn open_with_key(
    key: &[u8; 32],
    aad: &[u8],
    data: &[u8],
) -> Result<Zeroizing<Vec<u8>>, String> {
    use ring::aead::{AES_256_GCM, Aad, LessSafeKey, Nonce, UnboundKey};

    /// AES-GCM authentication tag length.
    const TAG_LEN: usize = 16;

    if data.len() < SETTINGS_NONCE_LEN + TAG_LEN {
        return Err("Encrypted data too short".to_string());
    }

    let nonce_bytes: [u8; SETTINGS_NONCE_LEN] = data[..SETTINGS_NONCE_LEN]
        .try_into()
        .map_err(|_| "Invalid nonce".to_string())?;
    let ciphertext = &data[SETTINGS_NONCE_LEN..];

    let unbound_key = UnboundKey::new(&AES_256_GCM, key)
        .map_err(|_| "Failed to create decryption key".to_string())?;
    let less_safe_key = LessSafeKey::new(unbound_key);
    let nonce = Nonce::assume_unique_for_key(nonce_bytes);

    // Plaintext-bearing buffer wiped on drop.
    let mut in_out = Zeroizing::new(ciphertext.to_vec());
    less_safe_key
        .open_in_place(nonce, Aad::from(aad), &mut in_out)
        .map_err(|_| "Decryption failed (wrong key or corrupted data)".to_string())?;

    // Drop the authentication tag. The length is bound first because
    // `Zeroizing`'s `Deref` blocks the two-phase borrow a plain `Vec` allows.
    let plaintext_len = in_out.len() - TAG_LEN;
    in_out.truncate(plaintext_len);
    Ok(in_out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The key `derive_passphrase_key` produces for fixed inputs, captured under
    /// argon2 0.5.3 before the crate was moved to 0.6.
    ///
    /// `derive_settings_key` has had a backward-compatibility fixture for a long
    /// time — the `RCSC` blob below — and `derive_passphrase_key`, which is what
    /// opens the portable credential store, had none. That gap mattered at exactly
    /// one moment: an argon2 upgrade. The round-trip tests create a store and open
    /// it with the same build, so they pass whether or not the derivation changed,
    /// while every store already on a user's disk would have become unopenable
    /// with no test going red. This vector is the missing half.
    ///
    /// Cheap parameters on purpose — 1 MiB, one pass, one lane. Argon2id does not
    /// special-case cost, so a change in the algorithm's output shows up here just
    /// as clearly as at 64 MiB, and the test costs milliseconds instead of half a
    /// second. `defaults_are_unchanged` covers the other half of the format
    /// contract, the cost parameters themselves.
    ///
    /// Do not regenerate this value. Re-minting it with the current code is how a
    /// format change gets tracked silently instead of caught.
    const PASSPHRASE_KDF_VECTOR: [u8; 32] = [
        0xee, 0x09, 0xf3, 0x9a, 0x5e, 0xe7, 0x59, 0x9e, 0xbb, 0x37, 0x69, 0x25, 0x77, 0x39, 0x1c,
        0x87, 0xb9, 0x6e, 0x84, 0x5b, 0x4c, 0x53, 0xa9, 0xbf, 0x1e, 0x0d, 0xdf, 0xc6, 0xc1, 0x59,
        0xc5, 0xba,
    ];

    /// The passphrase KDF still derives the key it derived before.
    ///
    /// If this fails after a dependency bump, the bump changes the on-disk format
    /// and needs a migration path rather than a version number.
    #[test]
    fn passphrase_key_derivation_is_unchanged() {
        let params = PassphraseKdfParams {
            m_cost: 1024,
            t_cost: 1,
            p_cost: 1,
        };
        let key = derive_passphrase_key(
            b"correct horse battery staple",
            b"rustconn-kdf-vec",
            &params,
        )
        .expect("derivation with valid parameters cannot fail");

        assert_eq!(
            *key, PASSPHRASE_KDF_VECTOR,
            "Argon2id output changed: every existing portable store is now unopenable"
        );
    }

    /// The default cost parameters are part of the format too.
    ///
    /// They are written into each store's header, so changing them does not break
    /// an existing file — it is read back from there. What it does change is what
    /// a *new* store costs to open, and the 64 MiB figure is quoted in
    /// `docs/USER_GUIDE.md` and in the unlock-latency reasoning above.
    #[test]
    fn default_kdf_parameters_are_unchanged() {
        let d = PassphraseKdfParams::default();
        assert_eq!((d.m_cost, d.t_cost, d.p_cost), (65536, 3, 4));
    }

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
    /// format byte-for-byte (no silent data loss on upgrade).
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
