//! Native ssh-agent wire protocol (A1).
//!
//! Speaks the ssh-agent protocol directly over the `SSH_AUTH_SOCK` Unix socket
//! instead of spawning `ssh-add -l` and a per-key `ssh-keygen`. On an SSH
//! connect with N agent identities that shell-out path cost N+1 processes and
//! about (1+N)×5 ms on the GTK main thread (issue, A1); this answers in one
//! round trip with no process at all.
//!
//! Two requests are spoken here: [`request_identities`], which fingerprints
//! each key with the shared helper in [`super::keyfile`], and
//! [`remove_identity`], which takes back one key RustConn added itself (see
//! `super::own_keys`). Adding keys still goes through `ssh-add` in
//! [`super::SshAgentManager`]: adding an *encrypted* key needs an in-process
//! passphrase decrypt, which is blocked by a dependency conflict (see
//! `keyfile`), so the askpass path stays. `SSH_AGENTC_REMOVE_ALL_IDENTITIES`
//! is deliberately not implemented: RustConn never removes keys it did not add.
//!
//! # Protocol
//!
//! Framing is `uint32 length` then `byte type` then the message body
//! (draft-miller-ssh-agent). The client sends `SSH_AGENTC_REQUEST_IDENTITIES`
//! (11) with an empty body; the agent answers `SSH_AGENT_IDENTITIES_ANSWER`
//! (12): `uint32 nkeys`, then for each key a `string` key blob and a `string`
//! comment. `SSH_AGENTC_REMOVE_IDENTITY` (18) carries one `string` key blob and
//! is answered with `SSH_AGENT_SUCCESS` (6) or `SSH_AGENT_FAILURE` (5). A
//! `string` is a `uint32` length followed by that many bytes. All integers are
//! big-endian.

use std::io::{Read as _, Write as _};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use super::{AgentError, AgentKey, AgentResult};

/// `SSH_AGENTC_REQUEST_IDENTITIES`.
const REQUEST_IDENTITIES: u8 = 11;
/// `SSH_AGENT_IDENTITIES_ANSWER`.
const IDENTITIES_ANSWER: u8 = 12;
/// `SSH_AGENTC_REMOVE_IDENTITY`.
pub(super) const REMOVE_IDENTITY: u8 = 18;
/// `SSH_AGENT_FAILURE`.
const FAILURE: u8 = 5;
/// `SSH_AGENT_SUCCESS`.
const SUCCESS: u8 = 6;

/// How long one agent round trip may take.
///
/// The agent is a local Unix socket, so a well-behaved one answers in under a
/// millisecond; this only bounds a wedged or launchd-throttled agent so the
/// caller (often the GTK thread) never parks on it. Matches the spirit of the
/// `ssh-add` timeout the shell-out path already used.
const AGENT_IO_TIMEOUT: Duration = Duration::from_secs(3);

/// A reply larger than this is treated as a broken or hostile agent and
/// refused rather than allocated.
///
/// A real agent holds a handful of keys at a few hundred bytes each; 1 MiB is
/// orders of magnitude above any legitimate reply and still cheap to reject.
const MAX_REPLY_BYTES: u32 = 1024 * 1024;

/// Reads the identities an agent at `socket_path` holds, over the native
/// protocol, computing each key's SHA256 fingerprint in-process.
///
/// Returns the keys in agent order. `AgentKey::bits` is `0` here: the wire
/// format does not carry it and deriving it means parsing every algorithm's
/// blob, which no connect-path decision needs — identities are matched by
/// fingerprint. `key_type` is read from the blob's leading algorithm string.
///
/// # Errors
///
/// Returns [`AgentError::Io`] if the socket cannot be reached or the exchange
/// times out, and [`AgentError::ParseError`] if the reply is not a well-formed
/// `SSH_AGENT_IDENTITIES_ANSWER`.
pub(super) fn request_identities(socket_path: &str) -> AgentResult<Vec<AgentKey>> {
    // Request: length (1 for the type byte) + type, empty body.
    let mut request = Vec::with_capacity(5);
    request.extend_from_slice(&1u32.to_be_bytes());
    request.push(REQUEST_IDENTITIES);
    let body = round_trip(socket_path, &request)?;
    parse_identities_answer(&body)
}

/// Asks the agent at `socket_path` to drop the one key whose public blob is
/// `blob`.
///
/// Returns `Ok(true)` when the agent removed it and `Ok(false)` when it
/// answered `SSH_AGENT_FAILURE` — it no longer holds that key (its lifetime ran
/// out, or the user removed it), which for a caller cleaning up is not an error.
///
/// # Errors
///
/// Returns [`AgentError::Io`] if the socket cannot be reached or the exchange
/// times out, and [`AgentError::ParseError`] if the blob is too large to frame
/// or the reply is neither success nor failure.
pub(super) fn remove_identity(socket_path: &str, blob: &[u8]) -> AgentResult<bool> {
    let request = remove_identity_request(blob)?;
    let body = round_trip(socket_path, &request)?;
    parse_status_reply(&body)
}

/// The framed `SSH_AGENTC_REMOVE_IDENTITY` request for `blob`.
fn remove_identity_request(blob: &[u8]) -> AgentResult<Vec<u8>> {
    let too_large = || AgentError::ParseError("key blob too large to frame".to_string());
    let blob_len = u32::try_from(blob.len()).map_err(|_| too_large())?;
    // type byte + uint32 string length + the blob
    let frame_len = blob_len.checked_add(5).ok_or_else(too_large)?;
    let mut request = Vec::with_capacity(blob.len() + 9);
    request.extend_from_slice(&frame_len.to_be_bytes());
    request.push(REMOVE_IDENTITY);
    request.extend_from_slice(&blob_len.to_be_bytes());
    request.extend_from_slice(blob);
    Ok(request)
}

/// Maps a one-byte `SSH_AGENT_SUCCESS` / `SSH_AGENT_FAILURE` reply to a bool.
fn parse_status_reply(body: &[u8]) -> AgentResult<bool> {
    match body.first() {
        Some(&SUCCESS) => Ok(true),
        Some(&FAILURE) => Ok(false),
        Some(other) => Err(AgentError::ParseError(format!(
            "expected SUCCESS or FAILURE, got message type {other}"
        ))),
        None => Err(AgentError::ParseError("empty agent reply".to_string())),
    }
}

/// Sends one framed request and reads one framed reply, returning the reply
/// body (type byte + payload). Every read and write is bounded by
/// [`AGENT_IO_TIMEOUT`].
fn round_trip(socket_path: &str, request: &[u8]) -> AgentResult<Vec<u8>> {
    let mut stream = UnixStream::connect(Path::new(socket_path))?;
    stream.set_read_timeout(Some(AGENT_IO_TIMEOUT))?;
    stream.set_write_timeout(Some(AGENT_IO_TIMEOUT))?;

    stream.write_all(request)?;
    stream.flush()?;

    // Reply: uint32 length, then that many bytes (type + body).
    let mut len_buf = [0u8; 4];
    stream.read_exact(&mut len_buf)?;
    let reply_len = u32::from_be_bytes(len_buf);
    if reply_len == 0 || reply_len > MAX_REPLY_BYTES {
        return Err(AgentError::ParseError(format!(
            "agent reply length {reply_len} out of range"
        )));
    }
    let mut body = vec![0u8; reply_len as usize];
    stream.read_exact(&mut body)?;
    Ok(body)
}

/// Parses the body of an `SSH_AGENT_IDENTITIES_ANSWER` (type byte + payload).
///
/// Split from the I/O so it can be tested against captured bytes without a live
/// agent.
fn parse_identities_answer(body: &[u8]) -> AgentResult<Vec<AgentKey>> {
    let mut reader = Reader::new(body);

    let msg_type = reader.u8()?;
    if msg_type != IDENTITIES_ANSWER {
        return Err(AgentError::ParseError(format!(
            "expected IDENTITIES_ANSWER ({IDENTITIES_ANSWER}), got message type {msg_type}"
        )));
    }

    let nkeys = reader.u32()?;
    // A sane agent holds few keys; bound the count against a truncated length
    // field claiming millions and driving a huge pre-allocation.
    if nkeys > 4096 {
        return Err(AgentError::ParseError(format!(
            "agent reports {nkeys} keys, which is implausible"
        )));
    }

    let mut keys = Vec::with_capacity(nkeys as usize);
    for _ in 0..nkeys {
        let blob = reader.string()?;
        let comment = reader.string()?;
        keys.push(AgentKey {
            fingerprint: super::keyfile::sha256_fingerprint(blob),
            bits: 0,
            key_type: super::keyfile::key_type_from_blob(blob),
            comment: String::from_utf8_lossy(comment).into_owned(),
        });
    }

    Ok(keys)
}

/// A bounds-checked cursor over the reply bytes.
///
/// Every read is length-checked and returns [`AgentError::ParseError`] on a
/// short buffer rather than panicking (M-PANIC-ON-BUG: a malformed reply is a
/// recoverable condition, not a bug in this program).
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    const fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    fn take(&mut self, n: usize) -> AgentResult<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or_else(|| {
            AgentError::ParseError("length overflow reading agent reply".to_string())
        })?;
        if end > self.buf.len() {
            return Err(AgentError::ParseError(
                "agent reply ended mid-field".to_string(),
            ));
        }
        let slice = &self.buf[self.pos..end];
        self.pos = end;
        Ok(slice)
    }

    fn u8(&mut self) -> AgentResult<u8> {
        Ok(self.take(1)?[0])
    }

    fn u32(&mut self) -> AgentResult<u32> {
        let bytes = self.take(4)?;
        Ok(u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    /// An SSH `string`: a `uint32` length then that many bytes.
    fn string(&mut self) -> AgentResult<&'a [u8]> {
        let len = self.u32()? as usize;
        self.take(len)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds an SSH `string` (uint32 length + bytes).
    fn ssh_string(bytes: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + bytes.len());
        #[expect(clippy::cast_possible_truncation, reason = "test inputs are tiny")]
        out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
        out.extend_from_slice(bytes);
        out
    }

    /// A minimal valid ed25519 key blob: algorithm string + 32-byte pubkey.
    fn ed25519_blob() -> Vec<u8> {
        let mut blob = ssh_string(b"ssh-ed25519");
        blob.extend(ssh_string(&[7u8; 32]));
        blob
    }

    /// An IDENTITIES_ANSWER body for the given (blob, comment) pairs.
    fn answer_body(keys: &[(Vec<u8>, &str)]) -> Vec<u8> {
        let mut body = vec![IDENTITIES_ANSWER];
        #[expect(clippy::cast_possible_truncation, reason = "test inputs are tiny")]
        body.extend_from_slice(&(keys.len() as u32).to_be_bytes());
        for (blob, comment) in keys {
            body.extend(ssh_string(blob));
            body.extend(ssh_string(comment.as_bytes()));
        }
        body
    }

    #[test]
    fn empty_agent_parses_to_no_keys() {
        let body = answer_body(&[]);
        let keys = parse_identities_answer(&body).unwrap();
        assert!(keys.is_empty());
    }

    #[test]
    fn one_ed25519_key_round_trips() {
        let body = answer_body(&[(ed25519_blob(), "user@host")]);
        let keys = parse_identities_answer(&body).unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].key_type, "ED25519");
        assert_eq!(keys[0].comment, "user@host");
        assert!(keys[0].fingerprint.starts_with("SHA256:"));
        // No padding in an OpenSSH SHA256 fingerprint.
        assert!(!keys[0].fingerprint.contains('='));
    }

    #[test]
    fn fingerprint_matches_known_vector() {
        // SHA256 of the 11 bytes b"hello world", base64 no-pad, is a fixed
        // value — this pins the encoding (standard base64, padding stripped,
        // SHA256: prefix) against drift. The helper now lives in `keyfile`,
        // shared with the key-file read path so both fingerprint identically.
        let fp = super::super::keyfile::sha256_fingerprint(b"hello world");
        assert_eq!(fp, "SHA256:uU0nuZNNPgilLlLX2n2r+sSE7+N6U4DukIj3rOLvzek");
    }

    #[test]
    fn rsa_and_ecdsa_types_are_recognised() {
        let rsa = ssh_string(b"ssh-rsa");
        assert_eq!(super::super::keyfile::key_type_from_blob(&rsa), "RSA");
        let ecdsa = ssh_string(b"ecdsa-sha2-nistp256");
        assert_eq!(super::super::keyfile::key_type_from_blob(&ecdsa), "ECDSA");
    }

    #[test]
    fn wrong_message_type_is_a_parse_error() {
        // A SUCCESS (6) where an IDENTITIES_ANSWER was expected.
        let body = vec![6u8, 0, 0, 0, 0];
        assert!(matches!(
            parse_identities_answer(&body),
            Err(AgentError::ParseError(_))
        ));
    }

    #[test]
    fn truncated_reply_is_a_parse_error_not_a_panic() {
        // Claims one key, then ends immediately.
        let mut body = vec![IDENTITIES_ANSWER];
        body.extend_from_slice(&1u32.to_be_bytes());
        assert!(matches!(
            parse_identities_answer(&body),
            Err(AgentError::ParseError(_))
        ));
    }

    #[test]
    fn remove_identity_request_frames_one_blob() {
        let blob = ed25519_blob();
        let request = remove_identity_request(&blob).unwrap();
        let mut expected = Vec::new();
        #[expect(clippy::cast_possible_truncation, reason = "test inputs are tiny")]
        expected.extend_from_slice(&((blob.len() + 5) as u32).to_be_bytes());
        expected.push(REMOVE_IDENTITY);
        expected.extend(ssh_string(&blob));
        assert_eq!(request, expected);
    }

    #[test]
    fn status_reply_maps_success_and_failure() {
        assert!(parse_status_reply(&[SUCCESS]).unwrap());
        assert!(!parse_status_reply(&[FAILURE]).unwrap());
        assert!(matches!(
            parse_status_reply(&[IDENTITIES_ANSWER]),
            Err(AgentError::ParseError(_))
        ));
        assert!(matches!(
            parse_status_reply(&[]),
            Err(AgentError::ParseError(_))
        ));
    }

    #[test]
    fn implausible_key_count_is_refused() {
        let mut body = vec![IDENTITIES_ANSWER];
        body.extend_from_slice(&5000u32.to_be_bytes());
        assert!(matches!(
            parse_identities_answer(&body),
            Err(AgentError::ParseError(_))
        ));
    }
}
