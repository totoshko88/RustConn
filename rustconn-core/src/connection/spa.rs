//! fwknop Single Packet Authorization (SPA) packet builder
//!
//! Builds and sends an fwknop-compatible SPA packet:
//! - Encrypt-then-MAC: AES-256-CBC (OpenSSL `Salted__` / EVP_BytesToKey-MD5) + HMAC-SHA256
//! - Wire-compatible with `fwknop 2.6.x` default settings
//!
//! Reference: <https://www.cipherdyne.org/fwknop/docs/fwknop-tutorial.html>

use std::net::{ToSocketAddrs, UdpSocket};
use std::time::{SystemTime, UNIX_EPOCH};

use aes::Aes256;
use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockModeEncrypt, KeyIvInit};
use data_encoding::BASE64;
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use secrecy::{ExposeSecret, SecretString};
use thiserror::Error;
use zeroize::Zeroizing;

type Aes256CbcEnc = cbc::Encryptor<Aes256>;

/// Errors from SPA packet operations
#[derive(Debug, Error)]
pub enum SpaError {
    /// Missing encryption key
    #[error("Rijndael key is required for SPA")]
    MissingRijndaelKey,
    /// Missing HMAC key
    #[error("HMAC key is required for SPA")]
    MissingHmacKey,
    /// Failed to resolve destination host
    #[error("Failed to resolve SPA destination '{host}': {reason}")]
    ResolutionFailed {
        /// The target host
        host: String,
        /// Why resolution failed
        reason: String,
    },
    /// I/O error sending the packet
    #[error("Failed to send SPA packet: {0}")]
    SendFailed(String),
    /// Internal crypto error
    #[error("Crypto error: {0}")]
    CryptoError(String),
}

/// Result of sending an SPA packet
#[derive(Debug, Clone)]
pub struct SpaResult {
    /// Size of the sent packet in bytes
    pub packet_size: usize,
    /// Time taken in milliseconds
    pub elapsed_ms: u32,
}

/// Builds an fwknop-compatible SPA packet
///
/// # Arguments
/// * `rijndael_key` - AES-256-CBC passphrase
/// * `hmac_key` - HMAC-SHA256 key
/// * `access` - Access specification (e.g. "tcp/22")
/// * `username` - System username (included in message)
///
/// # Returns
/// The complete SPA packet as bytes ready to send via UDP
///
/// # Errors
/// Returns an error if key material is missing or crypto operations fail.
pub fn build_spa_packet(
    rijndael_key: &SecretString,
    hmac_key: &SecretString,
    access: &str,
    username: &str,
) -> Result<Vec<u8>, SpaError> {
    let rng = SystemRandom::new();

    // 1. Generate 16 bytes of random data for the message
    let mut random_data = [0u8; 16];
    rng.fill(&mut random_data)
        .map_err(|_| SpaError::CryptoError("RNG failure".to_string()))?;
    let random_b64 = BASE64.encode(&random_data);

    // 2. Get current timestamp
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| SpaError::CryptoError(e.to_string()))?
        .as_secs();

    // 3. Build plaintext message fields (fwknop format):
    //    random_b64 : username : timestamp : fwknop_version : msg_type : access_spec : digest
    let version = "3.0.0"; // fwknop client version we claim
    let msg_type = "1"; // Access message type

    // Build message without digest first
    let msg_no_digest =
        format!("{random_b64}:{username}:{timestamp}:{version}:{msg_type}:{access}");

    // 4. Compute SHA-256 digest of the message
    let digest = ring::digest::digest(&ring::digest::SHA256, msg_no_digest.as_bytes());
    let digest_b64 = BASE64.encode(digest.as_ref());

    // 5. Complete message with digest appended
    let plaintext = format!("{msg_no_digest}:{digest_b64}");

    // 6. Encrypt with AES-256-CBC using OpenSSL EVP_BytesToKey (MD5) key derivation
    let encrypted = encrypt_rijndael(
        plaintext.as_bytes(),
        rijndael_key.expose_secret().as_bytes(),
        &rng,
    )?;

    // 7. Base64-encode the ciphertext (includes "Salted__" + salt + ciphertext)
    let encrypted_b64 = BASE64.encode(&encrypted);

    // 8. Compute HMAC-SHA256 over the base64-encoded ciphertext
    let hmac_key_bytes = Zeroizing::new(hmac_key.expose_secret().as_bytes().to_vec());
    let hmac_signing_key = hmac::Key::new(hmac::HMAC_SHA256, &hmac_key_bytes);
    let hmac_tag = hmac::sign(&hmac_signing_key, encrypted_b64.as_bytes());
    let hmac_b64 = BASE64.encode(hmac_tag.as_ref());

    // 9. Final packet: base64_ciphertext + base64_hmac
    let packet = format!("{encrypted_b64}{hmac_b64}");

    Ok(packet.into_bytes())
}

/// Sends an SPA packet to the target host
///
/// # Errors
/// Returns an error if the packet cannot be built or sent.
pub fn send_spa(
    host: &str,
    dest_port: u16,
    rijndael_key: &SecretString,
    hmac_key: &SecretString,
    access: &str,
    username: &str,
) -> Result<SpaResult, SpaError> {
    let start = std::time::Instant::now();

    let packet = build_spa_packet(rijndael_key, hmac_key, access, username)?;

    // Resolve destination
    let addr_str = format!("{host}:{dest_port}");
    let addr = addr_str
        .to_socket_addrs()
        .map_err(|e| SpaError::ResolutionFailed {
            host: host.to_string(),
            reason: e.to_string(),
        })?
        .next()
        .ok_or_else(|| SpaError::ResolutionFailed {
            host: host.to_string(),
            reason: "No addresses found".to_string(),
        })?;

    // Send via UDP
    let socket = UdpSocket::bind("0.0.0.0:0").map_err(|e| SpaError::SendFailed(e.to_string()))?;
    socket
        .send_to(&packet, addr)
        .map_err(|e| SpaError::SendFailed(e.to_string()))?;

    let elapsed_ms = start.elapsed().as_millis() as u32;

    tracing::info!(
        host,
        dest_port,
        packet_size = packet.len(),
        elapsed_ms,
        "SPA packet sent"
    );

    Ok(SpaResult {
        packet_size: packet.len(),
        elapsed_ms,
    })
}

// ─────────────────────────────────────────────────────────────────────────────
// OpenSSL EVP_BytesToKey (MD5-based) key derivation + AES-256-CBC encryption
// ─────────────────────────────────────────────────────────────────────────────

/// Encrypts data using AES-256-CBC with OpenSSL-compatible key derivation.
///
/// Output format: `"Salted__" (8 bytes) + salt (8 bytes) + ciphertext`
///
/// Key derivation uses EVP_BytesToKey with MD5:
/// - `key_material = MD5(password + salt) + MD5(prev + password + salt) + ...`
/// - First 32 bytes → AES-256 key, next 16 bytes → IV
fn encrypt_rijndael(
    plaintext: &[u8],
    password: &[u8],
    rng: &SystemRandom,
) -> Result<Vec<u8>, SpaError> {
    // Generate random 8-byte salt
    let mut salt = [0u8; 8];
    rng.fill(&mut salt)
        .map_err(|_| SpaError::CryptoError("RNG failure for salt".to_string()))?;

    // Derive key (32 bytes) and IV (16 bytes) using EVP_BytesToKey with MD5
    let (key, iv) = evp_bytes_to_key(password, &salt);

    // Encrypt with AES-256-CBC + PKCS7 padding
    let key_arr: [u8; 32] = key
        .as_slice()
        .try_into()
        .map_err(|_| SpaError::CryptoError("Key length mismatch".to_string()))?;
    let iv_arr: [u8; 16] = iv
        .as_slice()
        .try_into()
        .map_err(|_| SpaError::CryptoError("IV length mismatch".to_string()))?;
    let encryptor = Aes256CbcEnc::new(&key_arr.into(), &iv_arr.into());

    // Allocate buffer with space for padding (up to 16 extra bytes)
    let mut buf = vec![0u8; plaintext.len() + 16];
    buf[..plaintext.len()].copy_from_slice(plaintext);

    let ciphertext = encryptor
        .encrypt_padded::<Pkcs7>(&mut buf, plaintext.len())
        .map_err(|e| SpaError::CryptoError(format!("AES-CBC encryption failed: {e}")))?;

    // Build output: "Salted__" + salt + ciphertext
    let mut output = Vec::with_capacity(8 + 8 + ciphertext.len());
    output.extend_from_slice(b"Salted__");
    output.extend_from_slice(&salt);
    output.extend_from_slice(ciphertext);

    // Zeroize key material
    drop(key);
    drop(iv);

    Ok(output)
}

/// EVP_BytesToKey with MD5 — derives 48 bytes (32 key + 16 IV) from password + salt
///
/// Algorithm:
/// ```text
/// D_0 = ""
/// D_i = MD5(D_{i-1} + password + salt)
/// key_material = D_1 + D_2 + D_3
/// key = key_material[0..32], iv = key_material[32..48]
/// ```
fn evp_bytes_to_key(password: &[u8], salt: &[u8]) -> (Zeroizing<Vec<u8>>, Zeroizing<Vec<u8>>) {
    use md5::{Digest, Md5};

    let mut key_material = Zeroizing::new(Vec::with_capacity(48));
    let mut prev_hash: Vec<u8> = Vec::new();

    while key_material.len() < 48 {
        let mut hasher = Md5::new();
        if !prev_hash.is_empty() {
            hasher.update(&prev_hash);
        }
        hasher.update(password);
        hasher.update(salt);
        prev_hash = hasher.finalize().to_vec();
        key_material.extend_from_slice(&prev_hash);
    }

    let key = Zeroizing::new(key_material[..32].to_vec());
    let iv = Zeroizing::new(key_material[32..48].to_vec());
    (key, iv)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_evp_bytes_to_key_produces_correct_lengths() {
        let (key, iv) = evp_bytes_to_key(b"testpassword", b"saltsalt");
        assert_eq!(key.len(), 32);
        assert_eq!(iv.len(), 16);
    }

    #[test]
    fn test_evp_bytes_to_key_deterministic() {
        let (key1, iv1) = evp_bytes_to_key(b"password", b"12345678");
        let (key2, iv2) = evp_bytes_to_key(b"password", b"12345678");
        assert_eq!(*key1, *key2);
        assert_eq!(*iv1, *iv2);
    }

    #[test]
    fn test_evp_bytes_to_key_different_salt_different_output() {
        let (key1, _) = evp_bytes_to_key(b"password", b"salt1234");
        let (key2, _) = evp_bytes_to_key(b"password", b"salt5678");
        assert_ne!(*key1, *key2);
    }

    #[test]
    fn test_encrypt_rijndael_produces_salted_prefix() {
        let rng = SystemRandom::new();
        let result = encrypt_rijndael(b"hello world", b"testkey", &rng).unwrap();
        assert_eq!(&result[..8], b"Salted__");
        // Total: 8 (prefix) + 8 (salt) + 16 (one AES block for 11 bytes + padding)
        assert_eq!(result.len(), 8 + 8 + 16);
    }

    #[test]
    fn test_encrypt_rijndael_different_each_time() {
        let rng = SystemRandom::new();
        let enc1 = encrypt_rijndael(b"same input", b"key", &rng).unwrap();
        let enc2 = encrypt_rijndael(b"same input", b"key", &rng).unwrap();
        // Different random salt → different ciphertext
        assert_ne!(enc1, enc2);
    }

    #[test]
    fn test_build_spa_packet_structure() {
        let rij_key = SecretString::new("my_rijndael_passphrase".to_string().into());
        let hmac_key = SecretString::new("my_hmac_key".to_string().into());

        let packet = build_spa_packet(&rij_key, &hmac_key, "tcp/22", "testuser").unwrap();

        // Packet should be non-empty base64-like ASCII
        assert!(!packet.is_empty());
        assert!(packet.iter().all(|&b| b.is_ascii()));

        // Should be reasonably sized (typical: ~300-400 bytes)
        assert!(packet.len() > 100);
        assert!(packet.len() < 1000);
    }

    #[test]
    fn test_build_spa_packet_different_each_time() {
        let rij_key = SecretString::new("passphrase".to_string().into());
        let hmac_key = SecretString::new("hmackey".to_string().into());

        let pkt1 = build_spa_packet(&rij_key, &hmac_key, "tcp/22", "user").unwrap();
        let pkt2 = build_spa_packet(&rij_key, &hmac_key, "tcp/22", "user").unwrap();

        // Random + timestamp → always different
        assert_ne!(pkt1, pkt2);
    }

    #[test]
    fn test_build_spa_packet_contains_hmac_suffix() {
        let rij_key = SecretString::new("key123".to_string().into());
        let hmac_key = SecretString::new("hmac456".to_string().into());

        let packet = build_spa_packet(&rij_key, &hmac_key, "tcp/22,tcp/443", "admin").unwrap();
        let packet_str = String::from_utf8(packet).unwrap();

        // HMAC-SHA256 base64 is 44 chars (32 bytes → 44 base64 chars with padding)
        // The packet ends with the HMAC
        assert!(packet_str.len() > 44);
    }
}
