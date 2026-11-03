//! The SSH keys RustConn put into an agent on its own, so it can take exactly
//! those back out.
//!
//! When a connect or an SFTP/mc launch needs a key in the agent and the agent
//! does not hold it yet, RustConn runs `ssh-add -t 43200` ([`OWN_KEY_LIFETIME`])
//! and records the key's public blob here, together with the agent socket the
//! add went to. On exit, screen lock and suspend, [`evict_own_keys`] sends one
//! `SSH_AGENTC_REMOVE_IDENTITY` per recorded key to that same socket.
//!
//! Keys the user loaded are never recorded: a key the agent already holds is
//! not added again, and a key the user adds in Settings → SSH Agent is dropped
//! from the record, because from then on it is theirs. Nothing here removes all
//! identities. A key whose public half cannot be read without its passphrase (a
//! legacy PEM with no `.pub` beside it) cannot be recorded; its 12 h lifetime is
//! then the only bound.
//!
//! No passphrase is ever kept: the agent decrypts the key once, during the add.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

/// How long a key RustConn adds on its own stays in the agent (`ssh-add -t`).
///
/// The bound for the cases RustConn cannot clean up itself: a crash, a kill, or
/// a key it could not record.
pub const OWN_KEY_LIFETIME: Duration = Duration::from_hours(12);

/// One key RustConn added: where it went and what the agent knows it by.
#[derive(Debug, Clone, PartialEq, Eq)]
struct OwnKey {
    socket_path: String,
    /// The public-key blob. Public material, nothing secret.
    blob: Vec<u8>,
}

/// The keys RustConn added, in the order it added them.
///
/// The process has one, behind [`record`], [`forget`] and [`evict_own_keys`];
/// tests build their own so they never share state.
#[derive(Debug)]
pub(crate) struct OwnKeys {
    keys: Mutex<Vec<OwnKey>>,
}

impl OwnKeys {
    pub(crate) const fn new() -> Self {
        Self {
            keys: Mutex::new(Vec::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<OwnKey>> {
        // The data is a plain list that no panic can leave half-written.
        self.keys.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records that RustConn added `blob` to the agent at `socket_path`.
    pub(crate) fn record(&self, socket_path: &str, blob: Vec<u8>) {
        let key = OwnKey {
            socket_path: socket_path.to_owned(),
            blob,
        };
        let mut keys = self.lock();
        if !keys.contains(&key) {
            keys.push(key);
        }
    }

    /// Stops tracking `blob` at `socket_path`: the user has made it their key.
    pub(crate) fn forget(&self, socket_path: &str, blob: &[u8]) {
        self.lock()
            .retain(|k| !(k.socket_path == socket_path && k.blob == blob));
    }

    /// Removes every recorded key from its agent and clears the record.
    ///
    /// Returns how many the agents actually removed. A key the agent no longer
    /// holds (its lifetime ran out, or the user removed it) is not an error. An
    /// agent that cannot be reached is logged; the record is cleared anyway,
    /// because a vanished agent took its keys with it.
    pub(crate) fn evict(&self) -> usize {
        // Taken out under the lock, removed outside it: each removal is a
        // socket round trip of up to a few seconds.
        let keys = std::mem::take(&mut *self.lock());
        keys.iter().filter(|key| remove_one(key)).count()
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.lock().len()
    }
}

/// Sends `SSH_AGENTC_REMOVE_IDENTITY` for one key; `true` when it was removed.
#[cfg(unix)]
fn remove_one(key: &OwnKey) -> bool {
    let fingerprint = super::keyfile::sha256_fingerprint(&key.blob);
    match super::wire::remove_identity(&key.socket_path, &key.blob) {
        Ok(true) => {
            tracing::debug!(%fingerprint, "removed an SSH key RustConn added");
            true
        }
        Ok(false) => {
            tracing::debug!(%fingerprint, "the agent no longer held an SSH key RustConn added");
            false
        }
        Err(e) => {
            tracing::warn!(%fingerprint, error = %e, "could not remove an SSH key RustConn added");
            false
        }
    }
}

/// No agent socket protocol off Unix; the lifetime is the only bound there.
#[cfg(not(unix))]
fn remove_one(_key: &OwnKey) -> bool {
    false
}

/// The process-wide record.
static OWN_KEYS: OwnKeys = OwnKeys::new();

/// Records the key at `key_path`, just added to the agent at `socket_path`.
pub(crate) fn record(socket_path: &str, key_path: &std::path::Path) {
    if let Some(blob) = super::keyfile::public_blob_for_key_path(key_path) {
        OWN_KEYS.record(socket_path, blob);
    } else {
        tracing::debug!(
            ?key_path,
            "public half of an added SSH key is unreadable; only its lifetime will remove it"
        );
    }
}

/// Stops tracking the key at `key_path` in the agent at `socket_path`.
pub(crate) fn forget(socket_path: &str, key_path: &std::path::Path) {
    if let Some(blob) = super::keyfile::public_blob_for_key_path(key_path) {
        OWN_KEYS.forget(socket_path, &blob);
    }
}

/// Removes from their agents the SSH keys RustConn added on its own this run,
/// and only those.
///
/// Call it on exit, screen lock and suspend. Each key costs one Unix-socket
/// round trip bounded to a few seconds; no process is spawned. Keys the user
/// added are left alone. Returns how many keys the agents removed.
pub fn evict_own_keys() -> usize {
    OWN_KEYS.evict()
}

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use std::io::{Read as _, Write as _};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::sync::Arc;

    use super::*;
    use crate::lock_events::{LockEvent, Subscribers};

    /// `SSH_AGENTC_REMOVE_ALL_IDENTITIES`, which RustConn must never send.
    const REMOVE_ALL_IDENTITIES: u8 = 19;

    /// What the fake agent holds and every message type it was sent.
    #[derive(Default)]
    struct AgentState {
        keys: Vec<Vec<u8>>,
        requests: Vec<u8>,
    }

    /// A minimal ssh-agent on a socket in a private temp dir. It understands
    /// `REMOVE_IDENTITY` and `REMOVE_ALL_IDENTITIES` and answers anything else
    /// with `FAILURE`.
    struct FakeAgent {
        _dir: tempfile::TempDir,
        socket: String,
        state: Arc<Mutex<AgentState>>,
    }

    impl FakeAgent {
        fn holding(keys: &[&[u8]]) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("agent.sock");
            let listener = UnixListener::bind(&path).unwrap();
            let state = Arc::new(Mutex::new(AgentState {
                keys: keys.iter().map(|k| k.to_vec()).collect(),
                requests: Vec::new(),
            }));
            let served = Arc::clone(&state);
            // Leaks one thread blocked in `accept` per test; it ends with the
            // test process.
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { return };
                    serve(stream, &served);
                }
            });
            Self {
                _dir: dir,
                socket: path.to_string_lossy().into_owned(),
                state,
            }
        }

        fn keys(&self) -> Vec<Vec<u8>> {
            self.state.lock().unwrap().keys.clone()
        }

        fn requests(&self) -> Vec<u8> {
            self.state.lock().unwrap().requests.clone()
        }
    }

    fn serve(mut stream: UnixStream, state: &Mutex<AgentState>) {
        loop {
            let mut len = [0u8; 4];
            if stream.read_exact(&mut len).is_err() {
                return;
            }
            let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
            stream.read_exact(&mut body).unwrap();
            let ok = {
                let mut state = state.lock().unwrap();
                state.requests.push(body[0]);
                match body[0] {
                    super::super::wire::REMOVE_IDENTITY => {
                        let blob = &body[5..];
                        let held = state.keys.iter().position(|k| k == blob);
                        held.map(|i| state.keys.remove(i)).is_some()
                    }
                    REMOVE_ALL_IDENTITIES => {
                        state.keys.clear();
                        true
                    }
                    _ => false,
                }
            };
            let reply = [0, 0, 0, 1, if ok { 6 } else { 5 }];
            stream.write_all(&reply).unwrap();
        }
    }

    /// A distinct ed25519-shaped blob per `tag`.
    fn blob(tag: u8) -> Vec<u8> {
        let mut blob = Vec::new();
        blob.extend_from_slice(&11u32.to_be_bytes());
        blob.extend_from_slice(b"ssh-ed25519");
        blob.extend_from_slice(&32u32.to_be_bytes());
        blob.extend_from_slice(&[tag; 32]);
        blob
    }

    #[test]
    fn evict_removes_only_the_recorded_keys() {
        let (ours, users) = (blob(1), blob(2));
        let agent = FakeAgent::holding(&[&ours, &users]);
        let own = OwnKeys::new();
        own.record(&agent.socket, ours);

        assert_eq!(own.evict(), 1);
        assert_eq!(agent.keys(), vec![users], "the user's key stays");
        assert_eq!(agent.requests(), vec![super::super::wire::REMOVE_IDENTITY]);
        assert_eq!(own.len(), 0, "the record is cleared");
        assert_eq!(own.evict(), 0, "a second eviction has nothing to do");
    }

    #[test]
    fn a_forgotten_key_is_left_in_the_agent() {
        let key = blob(3);
        let agent = FakeAgent::holding(&[&key]);
        let own = OwnKeys::new();
        own.record(&agent.socket, key.clone());
        // The user added the same key in Settings: it is theirs now.
        own.forget(&agent.socket, &key);

        assert_eq!(own.evict(), 0);
        assert_eq!(agent.keys(), vec![key]);
        assert!(agent.requests().is_empty(), "nothing was sent to the agent");
    }

    #[test]
    fn recording_a_key_twice_keeps_one_entry() {
        let own = OwnKeys::new();
        own.record("/nonexistent/agent.sock", blob(4));
        own.record("/nonexistent/agent.sock", blob(4));
        own.record("/nonexistent/other.sock", blob(4));
        assert_eq!(own.len(), 2, "one entry per (socket, key)");
    }

    #[test]
    fn a_key_the_agent_no_longer_holds_is_not_an_error() {
        let agent = FakeAgent::holding(&[]);
        let own = OwnKeys::new();
        own.record(&agent.socket, blob(5));
        assert_eq!(own.evict(), 0);
        assert_eq!(own.len(), 0);
    }

    #[test]
    fn an_unreachable_agent_clears_the_record() {
        let own = OwnKeys::new();
        own.record("/nonexistent/rustconn-test/agent.sock", blob(6));
        assert_eq!(own.evict(), 0);
        assert_eq!(own.len(), 0);
    }

    /// The wiring the GUI does: a lock-event handler that evicts. Lock and
    /// suspend each remove the keys recorded since the last event and nothing
    /// else, and nothing ever asks the agent to remove all identities.
    #[test]
    fn lock_and_suspend_remove_recorded_keys_and_never_all_keys() {
        let (first, second, users) = (blob(7), blob(8), blob(9));
        let agent = FakeAgent::holding(&[&first, &second, &users]);
        let own = Arc::new(OwnKeys::new());
        let subscribers = Subscribers::new();
        let handler_keys = Arc::clone(&own);
        subscribers.add(Arc::new(move |_event| {
            handler_keys.evict();
        }));

        own.record(&agent.socket, first);
        subscribers.dispatch(LockEvent::ScreenLocked);
        assert_eq!(agent.keys(), vec![second.clone(), users.clone()]);

        own.record(&agent.socket, second);
        subscribers.dispatch(LockEvent::Suspending);
        assert_eq!(agent.keys(), vec![users]);

        let requests = agent.requests();
        assert!(!requests.contains(&REMOVE_ALL_IDENTITIES));
        assert_eq!(
            requests,
            vec![super::super::wire::REMOVE_IDENTITY; 2],
            "one REMOVE_IDENTITY per recorded key"
        );
    }
}
