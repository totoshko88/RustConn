//! Whether a connect asks the user to unlock an encrypted SSH key first.
//!
//! When a connection passes a key file to ssh, RustConn can ask for the key's
//! passphrase once in its own dialog and add the key to the agent with the
//! own-key lifetime ([`super::SshAgentManager::add_own_key_with_passphrase`]),
//! so later connects in the session find it there. [`check_key_unlock`] says
//! whether to ask: only for an OpenSSH key that is encrypted, that the agent
//! ssh will use is known not to hold, and whose agent has not refused an add
//! before in this process. Every other case keeps today's flow (ssh's own
//! prompt, or the key works without one).
//!
//! Legacy PEM and PKCS#8 keys are never asked about: whether they are
//! encrypted is not parsed here, and whether the agent holds them cannot be
//! told. Neither can an agent that does not answer.
//!
//! An agent that refuses the add (the 1Password agent, for one) is recorded
//! by its socket path for the rest of the process, so the dialog is not shown
//! again for it. Only socket paths are kept. No passphrase passes through this
//! module.

use std::path::Path;
use std::sync::{Mutex, MutexGuard, PoisonError};

use zeroize::Zeroizing;

use super::keyfile::{KeyEncryption, inspect_key_file, key_file_blobs};

/// Whether the agent ssh will use already holds a key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentHolds {
    /// The agent lists the key.
    Yes,
    /// The agent answered and does not list the key.
    No,
    /// The agent did not answer, or the key file does not name its public
    /// key.
    Unknown,
}

/// What a connect does about an encrypted key before ssh starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnlockDecision {
    /// Ask for the passphrase and add the key to the agent.
    AskPassphrase,
    /// Do nothing; ssh runs as it always did.
    Skip(SkipReason),
}

/// Why a connect does not ask for a key's passphrase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// No agent socket is known.
    NoAgent,
    /// The key file cannot be read, or is a malformed OpenSSH key.
    KeyUnreadable,
    /// The key is not encrypted.
    NotEncrypted,
    /// The key is not in the OpenSSH format (legacy PEM, PKCS#8), so whether
    /// it is encrypted is not known.
    EncryptionUnknown,
    /// The agent already holds the key.
    AlreadyInAgent,
    /// Whether the agent holds the key cannot be told.
    PresenceUnknown,
    /// This agent refused an add earlier in this process.
    AgentRefusedBefore,
}

/// How many wrong passphrases the unlock dialog takes before it gives up.
pub const MAX_PASSPHRASE_ATTEMPTS: u8 = 3;

/// What the unlock dialog does after a wrong passphrase.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AfterWrongPassphrase {
    /// Ask again, with an inline error.
    Retry,
    /// Close the dialog; the connect goes on without the key in the agent.
    GiveUp,
}

/// The decision for a key with `encryption`, given whether the agent holds it
/// and whether that agent refused an add before.
///
/// Only an encrypted key the agent is known not to hold, on an agent that has
/// not refused an add, is asked about.
#[must_use]
pub const fn decide(
    encryption: KeyEncryption,
    holds: AgentHolds,
    refused_before: bool,
) -> UnlockDecision {
    use UnlockDecision::{AskPassphrase, Skip};
    match (encryption, holds) {
        (KeyEncryption::Unencrypted, _) => Skip(SkipReason::NotEncrypted),
        (KeyEncryption::Unknown, _) => Skip(SkipReason::EncryptionUnknown),
        (KeyEncryption::Encrypted, AgentHolds::Yes) => Skip(SkipReason::AlreadyInAgent),
        (KeyEncryption::Encrypted, AgentHolds::Unknown) => Skip(SkipReason::PresenceUnknown),
        (KeyEncryption::Encrypted, AgentHolds::No) if refused_before => {
            Skip(SkipReason::AgentRefusedBefore)
        }
        (KeyEncryption::Encrypted, AgentHolds::No) => AskPassphrase,
    }
}

/// What the dialog does after `attempts_made` wrong passphrases.
#[must_use]
pub const fn after_wrong_passphrase(attempts_made: u8) -> AfterWrongPassphrase {
    if attempts_made >= MAX_PASSPHRASE_ATTEMPTS {
        AfterWrongPassphrase::GiveUp
    } else {
        AfterWrongPassphrase::Retry
    }
}

/// The agent sockets that refused an add. Socket paths only.
#[derive(Debug)]
struct RefusedAgents {
    sockets: Mutex<Vec<String>>,
}

impl RefusedAgents {
    const fn new() -> Self {
        Self {
            sockets: Mutex::new(Vec::new()),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Vec<String>> {
        // A plain list that no panic can leave half-written.
        self.sockets.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn record(&self, socket_path: &str) {
        let mut sockets = self.lock();
        if !sockets.iter().any(|s| s == socket_path) {
            sockets.push(socket_path.to_owned());
        }
    }

    fn contains(&self, socket_path: &str) -> bool {
        self.lock().iter().any(|s| s == socket_path)
    }
}

/// The process-wide record of refusing agents.
static REFUSED: RefusedAgents = RefusedAgents::new();

/// Records that the agent at `socket_path` refused to add a key.
pub(super) fn record_add_refused(socket_path: &str) {
    tracing::info!(
        socket_path,
        "the SSH agent refused to add a key; RustConn will not ask to unlock keys for it again"
    );
    REFUSED.record(socket_path);
}

/// Whether the agent at `socket_path` refused to add a key earlier in this
/// process.
#[must_use]
pub fn agent_refused_add(socket_path: &str) -> bool {
    REFUSED.contains(socket_path)
}

/// Whether a connect that passes the key at `key_path` to ssh, with ssh using
/// the agent at `socket_path`, asks for the key's passphrase first.
///
/// Reads the key file (wiped from memory afterwards) and, only when the key is
/// an encrypted OpenSSH key, asks the agent which keys it holds: one Unix
/// socket round trip, bounded to a few seconds. Never asks for or reads a
/// passphrase. Call it off the GTK thread.
#[must_use]
pub fn check_key_unlock(socket_path: Option<&str>, key_path: &Path) -> UnlockDecision {
    let decision = check(socket_path, key_path);
    tracing::debug!(?key_path, ?decision, "SSH key unlock check");
    decision
}

fn check(socket_path: Option<&str>, key_path: &Path) -> UnlockDecision {
    let Some(socket_path) = socket_path.filter(|s| !s.is_empty()) else {
        return UnlockDecision::Skip(SkipReason::NoAgent);
    };
    // The private section of an unencrypted key is in this file in the clear,
    // so the text is wiped when it drops.
    let Ok(contents) = std::fs::read_to_string(key_path).map(Zeroizing::new) else {
        return UnlockDecision::Skip(SkipReason::KeyUnreadable);
    };
    let Ok(info) = inspect_key_file(&contents) else {
        return UnlockDecision::Skip(SkipReason::KeyUnreadable);
    };
    drop(contents);
    if info.encryption != KeyEncryption::Encrypted {
        // No agent query for a key that needs no passphrase.
        return decide(info.encryption, AgentHolds::Unknown, false);
    }
    let holds = key_file_blobs(key_path)
        .key_file
        .map_or(AgentHolds::Unknown, |blob| agent_holds(socket_path, &blob));
    decide(info.encryption, holds, agent_refused_add(socket_path))
}

#[cfg(unix)]
fn agent_holds(socket_path: &str, blob: &[u8]) -> AgentHolds {
    match super::wire::request_identity_blobs(socket_path) {
        Ok(held) if held.iter().any(|h| h == blob) => AgentHolds::Yes,
        Ok(_) => AgentHolds::No,
        Err(e) => {
            tracing::debug!(error = %e, "could not ask the SSH agent which keys it holds");
            AgentHolds::Unknown
        }
    }
}

/// No agent socket protocol off Unix.
#[cfg(not(unix))]
fn agent_holds(_socket_path: &str, _blob: &[u8]) -> AgentHolds {
    AgentHolds::Unknown
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENCRYPTIONS: [KeyEncryption; 3] = [
        KeyEncryption::Unencrypted,
        KeyEncryption::Encrypted,
        KeyEncryption::Unknown,
    ];
    const HOLDS: [AgentHolds; 3] = [AgentHolds::Yes, AgentHolds::No, AgentHolds::Unknown];

    fn ed25519_blob(tag: u8) -> Vec<u8> {
        let mut blob = Vec::new();
        for part in [b"ssh-ed25519".as_slice(), &[tag; 32]] {
            blob.extend_from_slice(&u32::try_from(part.len()).unwrap().to_be_bytes());
            blob.extend_from_slice(part);
        }
        blob
    }

    fn key_file(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("id_test");
        std::fs::write(&path, contents).unwrap();
        (dir, path)
    }

    /// A socket path nothing listens on.
    const NO_AGENT: &str = "/nonexistent/rustconn-test/unlock-agent.sock";

    #[test]
    fn only_an_encrypted_key_the_agent_lacks_is_asked_about() {
        let mut asks = 0;
        for encryption in ENCRYPTIONS {
            for holds in HOLDS {
                for refused in [false, true] {
                    let decision = decide(encryption, holds, refused);
                    let expected = match (encryption, holds, refused) {
                        (KeyEncryption::Unencrypted, _, _) => {
                            UnlockDecision::Skip(SkipReason::NotEncrypted)
                        }
                        (KeyEncryption::Unknown, _, _) => {
                            UnlockDecision::Skip(SkipReason::EncryptionUnknown)
                        }
                        (_, AgentHolds::Yes, _) => UnlockDecision::Skip(SkipReason::AlreadyInAgent),
                        (_, AgentHolds::Unknown, _) => {
                            UnlockDecision::Skip(SkipReason::PresenceUnknown)
                        }
                        (_, AgentHolds::No, true) => {
                            UnlockDecision::Skip(SkipReason::AgentRefusedBefore)
                        }
                        (_, AgentHolds::No, false) => UnlockDecision::AskPassphrase,
                    };
                    assert_eq!(decision, expected, "{encryption:?} {holds:?} {refused}");
                    asks += usize::from(decision == UnlockDecision::AskPassphrase);
                }
            }
        }
        assert_eq!(asks, 1, "exactly one combination asks");
    }

    #[test]
    fn the_dialog_gives_up_after_three_wrong_passphrases() {
        let after: Vec<_> = (0..=4).map(after_wrong_passphrase).collect();
        assert_eq!(
            after,
            vec![
                AfterWrongPassphrase::Retry,
                AfterWrongPassphrase::Retry,
                AfterWrongPassphrase::Retry,
                AfterWrongPassphrase::GiveUp,
                AfterWrongPassphrase::GiveUp,
            ]
        );
    }

    #[test]
    fn a_refusal_is_remembered_for_that_socket_only() {
        let refused = RefusedAgents::new();
        refused.record("/run/a.sock");
        refused.record("/run/a.sock");
        assert!(refused.contains("/run/a.sock"));
        assert!(!refused.contains("/run/b.sock"));
        assert_eq!(refused.lock().len(), 1, "a duplicate is kept once");
    }

    #[test]
    fn no_agent_socket_means_no_dialog() {
        let (_dir, path) = key_file(&crate::ssh_agent::openssh_key_fixture(
            "aes256-ctr",
            &ed25519_blob(1),
        ));
        for socket in [None, Some("")] {
            assert_eq!(
                check_key_unlock(socket, &path),
                UnlockDecision::Skip(SkipReason::NoAgent)
            );
        }
    }

    #[test]
    fn a_missing_key_file_means_no_dialog() {
        assert_eq!(
            check_key_unlock(Some(NO_AGENT), Path::new("/nonexistent/rustconn-test/id_x")),
            UnlockDecision::Skip(SkipReason::KeyUnreadable)
        );
    }

    /// The agent socket is unreachable, so `NotEncrypted` (rather than
    /// `PresenceUnknown`) also shows the agent was not asked.
    #[test]
    fn an_unencrypted_key_is_not_asked_about_and_the_agent_is_not_queried() {
        let (_dir, path) = key_file(&crate::ssh_agent::openssh_key_fixture(
            "none",
            &ed25519_blob(2),
        ));
        assert_eq!(
            check_key_unlock(Some(NO_AGENT), &path),
            UnlockDecision::Skip(SkipReason::NotEncrypted)
        );
    }

    #[test]
    fn a_legacy_pem_key_is_not_asked_about() {
        let (_dir, path) = key_file(
            "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\n\nAAAA\n-----END RSA PRIVATE KEY-----\n",
        );
        assert_eq!(
            check_key_unlock(Some(NO_AGENT), &path),
            UnlockDecision::Skip(SkipReason::EncryptionUnknown)
        );
    }

    #[test]
    fn an_agent_that_does_not_answer_means_no_dialog() {
        let (_dir, path) = key_file(&crate::ssh_agent::openssh_key_fixture(
            "aes256-ctr",
            &ed25519_blob(3),
        ));
        assert_eq!(
            check_key_unlock(Some(NO_AGENT), &path),
            UnlockDecision::Skip(SkipReason::PresenceUnknown)
        );
    }
}
