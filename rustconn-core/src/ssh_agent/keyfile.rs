//! Private-key file inspection without a passphrase (A2-lite).
//!
//! The full A2 plan was to pull in `ssh-key` for in-process decryption so an
//! encrypted key could be added to the agent without spawning `ssh-add`. That
//! is blocked by a dependency conflict: `ssh-key 0.7.0-rc.11` pulls
//! `ssh-cipher 0.3.0 -> aes-gcm ^0.11`, and IronRDP's `picky` pins
//! `aes-gcm =0.11.0-rc.4`, which is *below* `0.11.0` by semver, so the two
//! exact RC pins cannot coexist. Decryption therefore stays on `ssh-add` for
//! now (see `.agents/tasks/plan-0.24-ssh-keys.md`).
//!
//! What this module does without any new dependency is the half that needs no
//! crypto: read the **public** part of a key and classify its encryption.
//!
//! - An OpenSSH private key (`-----BEGIN OPENSSH PRIVATE KEY-----`) carries its
//!   public key in the clear, before the (optionally encrypted) private
//!   section — so [`PublicKeyInfo::from_private_key_file`] computes the SHA256
//!   fingerprint of an encrypted key without ever touching the passphrase.
//! - The same format names its cipher in a header field; `"none"` means the
//!   key is unencrypted. [`inspect_key_file`] reports that as
//!   [`KeyEncryption`], which is what the caller needs to decide whether the
//!   native add path can be used or the `ssh-add` fallback is required.
//! - A `.pub` file is just `algo base64 comment`; [`PublicKeyInfo::from_pub_line`]
//!   decodes the blob and fingerprints it.
//!
//! Legacy PEM (`BEGIN RSA/EC PRIVATE KEY`) and PKCS#8
//! (`BEGIN ENCRYPTED PRIVATE KEY`) are recognised only well enough to route
//! them to `ssh-add`: this module does not parse their bodies.
//!
//! Everything here is byte parsing over `data-encoding` (already the project's
//! base64) and `ring` (already the project's SHA-256), so it builds on every
//! platform — unlike `wire.rs`, which is `#[cfg(unix)]` for the agent socket.

use super::{AgentError, AgentResult};

/// The OpenSSH private-key PEM armour.
const OPENSSH_BEGIN: &str = "-----BEGIN OPENSSH PRIVATE KEY-----";
const OPENSSH_END: &str = "-----END OPENSSH PRIVATE KEY-----";
/// The magic prefix of the base64-decoded OpenSSH key body, NUL-terminated.
const OPENSSH_MAGIC: &[u8] = b"openssh-key-v1\0";

/// Whether a key file's private half is encrypted.
///
/// Only the OpenSSH format is classified precisely; the legacy formats are
/// reported as [`KeyEncryption::Unknown`] because this module does not parse
/// their bodies and the caller routes them to `ssh-add` regardless.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyEncryption {
    /// The private key is stored in the clear (`ciphername` is `none`).
    Unencrypted,
    /// The private key is encrypted and needs a passphrase to use.
    Encrypted,
    /// The format is not OpenSSH (legacy PEM, PKCS#8) and was not inspected.
    Unknown,
}

/// The on-disk key-file format, as far as routing needs to tell the formats
/// apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyFormat {
    /// `-----BEGIN OPENSSH PRIVATE KEY-----`: the modern format this module
    /// reads.
    OpenSsh,
    /// Legacy PEM: `BEGIN RSA/DSA/EC PRIVATE KEY` (possibly `Proc-Type:
    /// 4,ENCRYPTED`). Handled by `ssh-add`.
    LegacyPem,
    /// PKCS#8: `BEGIN PRIVATE KEY` / `BEGIN ENCRYPTED PRIVATE KEY`. Handled by
    /// `ssh-add`.
    Pkcs8,
    /// None of the above — not a private key this module recognises.
    Unknown,
}

/// The public identity of a key: its SHA256 fingerprint and short type.
///
/// Matches the shape `wire.rs` produces for agent keys, so a file key and an
/// agent key compare by `fingerprint` directly (the A3 "is it already loaded?"
/// check).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKeyInfo {
    /// OpenSSH `SHA256:<base64-no-pad>` fingerprint of the public blob.
    pub fingerprint: String,
    /// Short key type: `ED25519`, `RSA`, `ECDSA`, … (best effort).
    pub key_type: String,
}

/// What [`inspect_key_file`] found about a key file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyFileInfo {
    /// The detected on-disk format.
    pub format: KeyFormat,
    /// Whether the private half is encrypted (precise only for OpenSSH).
    pub encryption: KeyEncryption,
    /// The public identity, when it could be read without a passphrase. `None`
    /// for the legacy formats this module does not parse.
    pub public_key: Option<PublicKeyInfo>,
}

impl KeyFileInfo {
    /// Whether this key can be added to the agent by the native path, or must
    /// fall back to `ssh-add`.
    ///
    /// The native add path (A2 proper) needs in-process decryption for an
    /// encrypted key, which is not available (see the module docs), so an
    /// encrypted key — and any non-OpenSSH format — routes to `ssh-add`. An
    /// unencrypted OpenSSH key needs no decryption.
    #[must_use]
    pub const fn needs_ssh_add_fallback(&self) -> bool {
        !matches!(
            (self.format, self.encryption),
            (KeyFormat::OpenSsh, KeyEncryption::Unencrypted)
        )
    }
}

/// The OpenSSH `SHA256:<base64>` fingerprint of a public-key blob.
///
/// OpenSSH hashes the raw blob with SHA-256 and prints the digest as standard
/// base64 with the padding `=` stripped. Shared with `wire.rs` (the agent
/// read path) so a file key and an agent key fingerprint identically.
pub(super) fn sha256_fingerprint(blob: &[u8]) -> String {
    let digest = ring::digest::digest(&ring::digest::SHA256, blob);
    let b64 = data_encoding::BASE64_NOPAD.encode(digest.as_ref());
    format!("SHA256:{b64}")
}

/// The short UI key type from a public-key blob's leading algorithm string.
///
/// Shared with `wire.rs`. Returns `"UNKNOWN"` for a blob that does not start
/// with a readable algorithm string; no decision keys on this.
pub(super) fn key_type_from_blob(blob: &[u8]) -> String {
    let Some(algo) = read_ssh_string(blob, 0).map(|(s, _)| s) else {
        return "UNKNOWN".to_string();
    };
    let algo = String::from_utf8_lossy(algo);
    match algo.as_ref() {
        "ssh-ed25519" => "ED25519".to_string(),
        "ssh-rsa" => "RSA".to_string(),
        "ssh-dss" => "DSA".to_string(),
        a if a.starts_with("ecdsa-sha2-") => "ECDSA".to_string(),
        a if a.starts_with("sk-ssh-ed25519") => "ED25519-SK".to_string(),
        a if a.starts_with("sk-ecdsa-sha2-") => "ECDSA-SK".to_string(),
        other => other.to_uppercase(),
    }
}

impl PublicKeyInfo {
    /// Builds a [`PublicKeyInfo`] from a raw OpenSSH public-key blob (the bytes
    /// `wire.rs` also fingerprints).
    #[must_use]
    pub(super) fn from_blob(blob: &[u8]) -> Self {
        Self {
            fingerprint: sha256_fingerprint(blob),
            key_type: key_type_from_blob(blob),
        }
    }

    /// Parses a `.pub` line (`algo base64blob [comment]`) and fingerprints the
    /// decoded blob.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::ParseError`] when the line has no base64 field or
    /// the field is not valid base64.
    pub fn from_pub_line(line: &str) -> AgentResult<Self> {
        Ok(Self::from_blob(&pub_line_blob(line)?))
    }

    /// Reads the public key from a **private** OpenSSH key file, without a
    /// passphrase.
    ///
    /// The OpenSSH private-key format stores the public key in the clear ahead
    /// of the (optionally encrypted) private section, so this works for an
    /// encrypted key too — which is the whole point: a fingerprint identifies
    /// the key before anyone is asked to unlock it.
    ///
    /// # Errors
    ///
    /// Returns [`AgentError::ParseError`] when `contents` is not an OpenSSH
    /// private key or its public section cannot be read.
    pub fn from_private_key_file(contents: &str) -> AgentResult<Self> {
        let body = decode_openssh_body(contents)?;
        let blob = openssh_public_blob(&body)?;
        Ok(Self::from_blob(&blob))
    }
}

/// Decodes the blob of a `.pub` line (`algo base64blob [comment]`).
fn pub_line_blob(line: &str) -> AgentResult<Vec<u8>> {
    let mut fields = line.split_whitespace();
    let _algo = fields.next();
    let b64 = fields
        .next()
        .ok_or_else(|| AgentError::ParseError("public-key line has no base64 field".to_string()))?;
    data_encoding::BASE64
        .decode(b64.as_bytes())
        .map_err(|e| AgentError::ParseError(format!("public-key base64 is invalid: {e}")))
}

/// What the files of a private key say its public-key blob is, read without a
/// passphrase. Public material only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct KeyFileBlobs {
    /// From the private key file itself: an OpenSSH private key carries its
    /// public key in the clear, even when encrypted, so this names the key
    /// `ssh-add` loads from that file.
    pub(super) key_file: Option<Vec<u8>>,
    /// From a sibling `<path>-cert.pub`, the certificate `ssh-add` loads
    /// together with the key when that file exists. Not checked against the
    /// key here: `ssh-add` skips a certificate that does not match, so only a
    /// certificate the agent shows after the add counts.
    pub(super) cert_file: Option<Vec<u8>>,
}

/// Reads [`KeyFileBlobs`] for the private key at `key_path`; a source that
/// cannot be read is `None`.
///
/// A sibling `<path>.pub` is not read: it is not checked against the private
/// key, and a stale one names some other key.
pub(super) fn key_file_blobs(key_path: &std::path::Path) -> KeyFileBlobs {
    let key_file = std::fs::read_to_string(key_path)
        .ok()
        .filter(|contents| detect_format(contents) == KeyFormat::OpenSsh)
        .and_then(|contents| {
            decode_openssh_body(&contents)
                .and_then(|b| openssh_public_blob(&b))
                .ok()
        });
    let mut cert_path = key_path.as_os_str().to_os_string();
    cert_path.push("-cert.pub");
    let cert_file = std::fs::read_to_string(std::path::PathBuf::from(cert_path))
        .ok()
        .and_then(|line| pub_line_blob(line.trim()).ok());
    KeyFileBlobs {
        key_file,
        cert_file,
    }
}

/// The public-key line (`<type> <base64>`, no comment) of an OpenSSH private
/// key, rebuilt from its public section without touching the (possibly
/// encrypted) private section.
///
/// Used for "Use an Existing Key" (K5) when no `.pub` file sits next to the
/// private key: the line this returns is the source of truth, since the
/// OpenSSH format carries the public key in the clear.
///
/// # Errors
///
/// Returns [`AgentError::ParseError`] when `contents` is not an OpenSSH
/// private key, its public section cannot be read, or the key type inside it
/// is missing or not valid UTF-8.
pub fn openssh_public_key_line(contents: &str) -> AgentResult<String> {
    let body = decode_openssh_body(contents)?;
    let blob = openssh_public_blob(&body)?;
    let (algo, _) = read_ssh_string(&blob, 0).ok_or_else(|| {
        AgentError::ParseError("OpenSSH public blob has no type string".to_string())
    })?;
    let algo = std::str::from_utf8(algo).map_err(|_| {
        AgentError::ParseError("OpenSSH public blob type is not valid UTF-8".to_string())
    })?;
    if algo.is_empty() {
        return Err(AgentError::ParseError(
            "OpenSSH public blob type is empty".to_string(),
        ));
    }
    Ok(format!("{algo} {}", data_encoding::BASE64.encode(&blob)))
}

/// Inspects a key file's contents: format, encryption, and the public key when
/// it can be read without a passphrase.
///
/// This never asks for or uses a passphrase and never parses a private key
/// body. For a non-OpenSSH format it reports the format and
/// [`KeyEncryption::Unknown`] with no public key, which is enough for the
/// caller to route it to `ssh-add`.
///
/// # Errors
///
/// Returns [`AgentError::ParseError`] only when the contents *look* like an
/// OpenSSH key (correct armour) but the body is malformed. A legacy or
/// unrecognised format is reported, not an error.
pub fn inspect_key_file(contents: &str) -> AgentResult<KeyFileInfo> {
    let format = detect_format(contents);
    match format {
        KeyFormat::OpenSsh => {
            let body = decode_openssh_body(contents)?;
            let encryption = openssh_encryption(&body)?;
            let public_key = openssh_public_blob(&body)
                .ok()
                .map(|blob| PublicKeyInfo::from_blob(&blob));
            Ok(KeyFileInfo {
                format,
                encryption,
                public_key,
            })
        }
        other => Ok(KeyFileInfo {
            format: other,
            encryption: KeyEncryption::Unknown,
            public_key: None,
        }),
    }
}

/// Classifies a private-key file by its PEM armour line.
fn detect_format(contents: &str) -> KeyFormat {
    let trimmed = contents.trim_start();
    if trimmed.starts_with(OPENSSH_BEGIN) {
        return KeyFormat::OpenSsh;
    }
    // Scan the first few lines for a BEGIN marker; a key file leads with it,
    // but allow a leading comment line or two.
    for line in trimmed.lines().take(4) {
        let line = line.trim();
        if line.starts_with("-----BEGIN ") && line.ends_with("PRIVATE KEY-----") {
            if line.contains("ENCRYPTED PRIVATE KEY") || line == "-----BEGIN PRIVATE KEY-----" {
                return KeyFormat::Pkcs8;
            }
            // BEGIN RSA/DSA/EC PRIVATE KEY — legacy PEM.
            return KeyFormat::LegacyPem;
        }
    }
    KeyFormat::Unknown
}

/// Base64-decodes the body between the OpenSSH armour lines.
fn decode_openssh_body(contents: &str) -> AgentResult<Vec<u8>> {
    let start = contents
        .find(OPENSSH_BEGIN)
        .ok_or_else(|| AgentError::ParseError("not an OpenSSH private key".to_string()))?;
    let after_begin = start + OPENSSH_BEGIN.len();
    let end = contents[after_begin..]
        .find(OPENSSH_END)
        .ok_or_else(|| AgentError::ParseError("OpenSSH key is missing its END line".to_string()))?;
    let b64: String = contents[after_begin..after_begin + end]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    let body = data_encoding::BASE64
        .decode(b64.as_bytes())
        .map_err(|e| AgentError::ParseError(format!("OpenSSH key base64 is invalid: {e}")))?;
    if !body.starts_with(OPENSSH_MAGIC) {
        return Err(AgentError::ParseError(
            "OpenSSH key body lacks the openssh-key-v1 magic".to_string(),
        ));
    }
    Ok(body)
}

/// Reads the `ciphername` field of a decoded OpenSSH key body and maps it to
/// [`KeyEncryption`]. `"none"` is unencrypted; anything else is encrypted.
///
/// Layout after the magic: `string ciphername`, `string kdfname`,
/// `string kdfoptions`, `uint32 nkeys`, then the public keys.
fn openssh_encryption(body: &[u8]) -> AgentResult<KeyEncryption> {
    let offset = OPENSSH_MAGIC.len();
    let (ciphername, _) = read_ssh_string(body, offset).ok_or_else(|| {
        AgentError::ParseError("OpenSSH key truncated before ciphername".to_string())
    })?;
    if ciphername == b"none" {
        Ok(KeyEncryption::Unencrypted)
    } else {
        Ok(KeyEncryption::Encrypted)
    }
}

/// Extracts the first public-key blob from a decoded OpenSSH key body.
///
/// The public keys sit in the clear regardless of private-section encryption.
/// After the magic: ciphername, kdfname, kdfoptions, `uint32 nkeys`, then for
/// each key a `string` public blob. We take the first blob.
fn openssh_public_blob(body: &[u8]) -> AgentResult<Vec<u8>> {
    let mut offset = OPENSSH_MAGIC.len();
    // ciphername, kdfname, kdfoptions
    for field in ["ciphername", "kdfname", "kdfoptions"] {
        let (_, next) = read_ssh_string(body, offset).ok_or_else(|| {
            AgentError::ParseError(format!("OpenSSH key truncated before {field} end"))
        })?;
        offset = next;
    }
    // uint32 nkeys
    let nkeys_bytes = body.get(offset..offset + 4).ok_or_else(|| {
        AgentError::ParseError("OpenSSH key truncated before key count".to_string())
    })?;
    let nkeys = u32::from_be_bytes([
        nkeys_bytes[0],
        nkeys_bytes[1],
        nkeys_bytes[2],
        nkeys_bytes[3],
    ]);
    offset += 4;
    if nkeys == 0 {
        return Err(AgentError::ParseError(
            "OpenSSH key declares zero public keys".to_string(),
        ));
    }
    let (blob, _) = read_ssh_string(body, offset).ok_or_else(|| {
        AgentError::ParseError("OpenSSH key truncated before public blob".to_string())
    })?;
    Ok(blob.to_vec())
}

/// Reads an SSH `string` (uint32 length + bytes) at `offset`, returning the
/// bytes and the offset just past them, or `None` on a short buffer.
fn read_ssh_string(buf: &[u8], offset: usize) -> Option<(&[u8], usize)> {
    let len_bytes = buf.get(offset..offset + 4)?;
    let len = u32::from_be_bytes([len_bytes[0], len_bytes[1], len_bytes[2], len_bytes[3]]) as usize;
    let start = offset + 4;
    let end = start.checked_add(len)?;
    let bytes = buf.get(start..end)?;
    Some((bytes, end))
}

/// Test fixture: an OpenSSH private-key body carrying `pub_blob` as its single
/// public key, armoured as PEM text. `ciphername` is the OpenSSH `ciphername`
/// field (`"none"` for unencrypted); the private section is a dummy that no
/// reader here ever parses.
///
/// Shared with `ssh_key_setup`'s tests, which need a realistic private-key
/// file without running `ssh-keygen`.
#[cfg(test)]
pub(crate) fn openssh_key_fixture(ciphername: &str, pub_blob: &[u8]) -> String {
    fn ssh_string(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
        out.extend_from_slice(bytes);
    }
    let mut body = Vec::new();
    body.extend_from_slice(OPENSSH_MAGIC);
    ssh_string(&mut body, ciphername.as_bytes());
    ssh_string(
        &mut body,
        if ciphername == "none" {
            b"none"
        } else {
            b"bcrypt"
        },
    );
    ssh_string(&mut body, b""); // kdfoptions (empty for this fixture)
    body.extend_from_slice(&1u32.to_be_bytes()); // nkeys
    ssh_string(&mut body, pub_blob); // the public blob
    ssh_string(&mut body, b"encrypted-or-plain-private-section-ignored");

    let b64 = data_encoding::BASE64.encode(&body);
    format!("{OPENSSH_BEGIN}\n{b64}\n{OPENSSH_END}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    // A real ed25519 OpenSSH public key line (algo + base64 + comment).
    const ED25519_PUB: &str = "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAINnFst2q3Z6rJ0r5wqT3o8n5oQ9m9g2k0o7w8b6c1d2e tester@host";

    #[test]
    fn pub_line_fingerprints_and_types() {
        let info = PublicKeyInfo::from_pub_line(ED25519_PUB).unwrap();
        assert_eq!(info.key_type, "ED25519");
        assert!(info.fingerprint.starts_with("SHA256:"));
        assert!(!info.fingerprint.contains('='));
    }

    #[test]
    fn pub_line_without_base64_is_a_parse_error() {
        assert!(matches!(
            PublicKeyInfo::from_pub_line("ssh-ed25519"),
            Err(AgentError::ParseError(_))
        ));
    }

    #[test]
    fn pub_line_with_bad_base64_is_a_parse_error() {
        assert!(matches!(
            PublicKeyInfo::from_pub_line("ssh-ed25519 not!valid!base64!"),
            Err(AgentError::ParseError(_))
        ));
    }

    #[test]
    fn from_blob_matches_the_wire_fingerprint() {
        // The same blob shape wire.rs builds: algo string + 32-byte key.
        let mut blob = Vec::new();
        blob.extend_from_slice(&(11u32).to_be_bytes());
        blob.extend_from_slice(b"ssh-ed25519");
        blob.extend_from_slice(&(32u32).to_be_bytes());
        blob.extend_from_slice(&[7u8; 32]);
        let info = PublicKeyInfo::from_blob(&blob);
        assert_eq!(info.key_type, "ED25519");
        assert_eq!(info.fingerprint, sha256_fingerprint(&blob));
    }

    #[test]
    fn detect_format_recognises_each_armour() {
        assert_eq!(
            detect_format(
                "-----BEGIN OPENSSH PRIVATE KEY-----\nAAAA\n-----END OPENSSH PRIVATE KEY-----"
            ),
            KeyFormat::OpenSsh
        );
        assert_eq!(
            detect_format("-----BEGIN RSA PRIVATE KEY-----\nMII\n-----END RSA PRIVATE KEY-----"),
            KeyFormat::LegacyPem
        );
        assert_eq!(
            detect_format("-----BEGIN EC PRIVATE KEY-----\nMHc\n-----END EC PRIVATE KEY-----"),
            KeyFormat::LegacyPem
        );
        assert_eq!(
            detect_format(
                "-----BEGIN ENCRYPTED PRIVATE KEY-----\nMII\n-----END ENCRYPTED PRIVATE KEY-----"
            ),
            KeyFormat::Pkcs8
        );
        assert_eq!(
            detect_format("-----BEGIN PRIVATE KEY-----\nMII\n-----END PRIVATE KEY-----"),
            KeyFormat::Pkcs8
        );
        assert_eq!(detect_format("not a key at all"), KeyFormat::Unknown);
    }

    /// An ed25519 fixture: [`openssh_key_fixture`] with a fixed public blob.
    fn make_openssh_key(ciphername: &str) -> String {
        fn ssh_string(out: &mut Vec<u8>, bytes: &[u8]) {
            out.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
            out.extend_from_slice(bytes);
        }
        let mut pub_blob = Vec::new();
        ssh_string(&mut pub_blob, b"ssh-ed25519");
        ssh_string(&mut pub_blob, &[9u8; 32]);
        openssh_key_fixture(ciphername, &pub_blob)
    }

    #[test]
    fn unencrypted_openssh_key_is_classified_and_fingerprinted() {
        let pem = make_openssh_key("none");
        let info = inspect_key_file(&pem).unwrap();
        assert_eq!(info.format, KeyFormat::OpenSsh);
        assert_eq!(info.encryption, KeyEncryption::Unencrypted);
        let pk = info.public_key.as_ref().expect("public key read");
        assert_eq!(pk.key_type, "ED25519");
        assert!(pk.fingerprint.starts_with("SHA256:"));
        assert!(!info.needs_ssh_add_fallback());
    }

    #[test]
    fn encrypted_openssh_key_fingerprints_without_a_passphrase() {
        let pem = make_openssh_key("aes256-ctr");
        let info = inspect_key_file(&pem).unwrap();
        assert_eq!(info.encryption, KeyEncryption::Encrypted);
        // The public key is still readable — the point of A2-lite.
        let pk = info
            .public_key
            .as_ref()
            .expect("public key read from encrypted key");
        assert_eq!(pk.key_type, "ED25519");
        // An encrypted key routes to ssh-add (no in-process decrypt).
        assert!(info.needs_ssh_add_fallback());

        // And from_private_key_file gives the same fingerprint, no passphrase.
        let pk2 = PublicKeyInfo::from_private_key_file(&pem).unwrap();
        assert_eq!(pk2.fingerprint, pk.fingerprint);
    }

    #[test]
    fn legacy_pem_routes_to_ssh_add_without_a_public_key() {
        let info =
            inspect_key_file("-----BEGIN RSA PRIVATE KEY-----\nMII\n-----END RSA PRIVATE KEY-----")
                .unwrap();
        assert_eq!(info.format, KeyFormat::LegacyPem);
        assert_eq!(info.encryption, KeyEncryption::Unknown);
        assert!(info.public_key.is_none());
        assert!(info.needs_ssh_add_fallback());
    }

    #[test]
    fn public_key_line_matches_the_private_key_fingerprint() {
        let pem = make_openssh_key("none");
        let line = openssh_public_key_line(&pem).unwrap();
        assert!(line.starts_with("ssh-ed25519 "));
        let from_line = PublicKeyInfo::from_pub_line(&line).unwrap();
        let from_private = PublicKeyInfo::from_private_key_file(&pem).unwrap();
        assert_eq!(from_line.fingerprint, from_private.fingerprint);
    }

    #[test]
    fn public_key_line_works_on_an_encrypted_key_without_a_passphrase() {
        let pem = make_openssh_key("aes256-ctr");
        let line = openssh_public_key_line(&pem).unwrap();
        assert!(line.starts_with("ssh-ed25519 "));
        let from_line = PublicKeyInfo::from_pub_line(&line).unwrap();
        let from_private = PublicKeyInfo::from_private_key_file(&pem).unwrap();
        assert_eq!(from_line.fingerprint, from_private.fingerprint);
    }

    #[test]
    fn public_key_line_of_non_openssh_text_is_a_parse_error() {
        assert!(matches!(
            openssh_public_key_line("not a key at all"),
            Err(AgentError::ParseError(_))
        ));
    }

    #[test]
    fn public_blob_comes_from_an_encrypted_openssh_key_itself() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("id_ed25519");
        std::fs::write(&key, make_openssh_key("aes256-ctr")).unwrap();
        let blobs = key_file_blobs(&key);
        let blob = blobs
            .key_file
            .clone()
            .expect("blob read without a passphrase");
        assert_eq!(key_type_from_blob(&blob), "ED25519");
        assert_eq!(blobs.cert_file, None, "no certificate beside it");
    }

    /// A `.pub` beside a key is never read: it may be stale. Only the key
    /// file and its `-cert.pub` count.
    #[test]
    fn a_pub_sibling_is_ignored_and_a_cert_sibling_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let key = dir.path().join("legacy_rsa");
        std::fs::write(
            &key,
            "-----BEGIN RSA PRIVATE KEY-----\nMII\n-----END RSA PRIVATE KEY-----\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("legacy_rsa.pub"), ED25519_PUB).unwrap();
        assert_eq!(
            key_file_blobs(&key),
            KeyFileBlobs::default(),
            "a legacy PEM has no public section, and the .pub is not read"
        );

        // The certificate line's shape is all that matters here: its base64
        // field is what the agent lists the certificate by.
        let cert_line = ED25519_PUB.replacen("ssh-ed25519", "ssh-ed25519-cert-v01@openssh.com", 1);
        std::fs::write(dir.path().join("legacy_rsa-cert.pub"), &cert_line).unwrap();
        let blobs = key_file_blobs(&key);
        assert_eq!(blobs.key_file, None);
        let cert = blobs.cert_file.expect("blob from the -cert.pub sibling");
        assert_eq!(
            sha256_fingerprint(&cert),
            PublicKeyInfo::from_pub_line(ED25519_PUB)
                .unwrap()
                .fingerprint
        );
    }

    #[test]
    fn openssh_armour_with_a_broken_body_is_a_parse_error() {
        let bad = format!("{OPENSSH_BEGIN}\n!!!notbase64!!!\n{OPENSSH_END}\n");
        assert!(matches!(
            inspect_key_file(&bad),
            Err(AgentError::ParseError(_))
        ));
    }
}
