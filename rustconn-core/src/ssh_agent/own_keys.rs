//! The SSH keys RustConn put into an agent on its own, so it can take exactly
//! those back out.
//!
//! When a connect or an SFTP/mc launch needs a key in the agent, RustConn asks
//! the agent which identities it holds, runs `ssh-add -t 43200`
//! ([`OWN_KEY_LIFETIME`]) only if the key is not among them, asks again, and
//! records the public blobs that appeared in between, together with the agent
//! socket all three went to ([`add_and_record`]). On exit, screen lock and
//! suspend, [`evict_own_keys`] sends one `SSH_AGENTC_REMOVE_IDENTITY` per
//! recorded blob to that same socket.
//!
//! Keys the user loaded are never recorded. A key the agent already holds is
//! not added again, because re-adding would replace its constraints. When the
//! agent cannot say what it holds, or the key's public half is unreadable (a
//! legacy PEM with no `.pub` beside it) and the agent holds keys this one could
//! be, nothing is added. The blob read from the key's files only decides
//! whether to add; what gets recorded comes from the agent itself, so a stale
//! `.pub` cannot make RustConn remove someone else's key. A key the user adds
//! in Settings → SSH Agent is dropped from the record, because from then on it
//! is theirs. Nothing here removes all identities.
//!
//! No passphrase is ever kept: the agent decrypts the key once, during the add.

use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use super::{AgentError, AgentResult};

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
/// The process has one, behind [`add_and_record`], [`forget`] and
/// [`evict_own_keys`]; tests build their own so they never share state.
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
    fn record(&self, socket_path: &str, blob: Vec<u8>) {
        let key = OwnKey {
            socket_path: socket_path.to_owned(),
            blob,
        };
        let mut keys = self.lock();
        if !keys.contains(&key) {
            keys.push(key);
        }
    }

    /// Runs `add` (the `ssh-add` of the key at `key_path` into the agent at
    /// `socket_path`) unless the agent may already hold the key, and records
    /// the identities the add put there.
    ///
    /// `expected` is the key's public blob as read from its files, or `None`
    /// when unreadable. It only decides whether to add: with it, a key the
    /// agent already holds is left alone; without it, the add runs only into
    /// an agent that holds nothing. What is recorded is what appeared in the
    /// agent between the query before the add and the one after it, and
    /// nothing when `expected` is known and did not appear (a stale `.pub`).
    /// `key_path` only names the key in the log.
    #[cfg(unix)]
    pub(crate) fn add_and_record(
        &self,
        socket_path: &str,
        key_path: &std::path::Path,
        expected: Option<&[u8]>,
        add: impl FnOnce() -> AgentResult<()>,
    ) -> AgentResult<()> {
        let before = super::wire::request_identity_blobs(socket_path).map_err(|e| {
            tracing::warn!(?key_path, error = %e, "could not ask the SSH agent which keys it holds; not adding the key");
            AgentError::AddKeyFailed(format!(
                "could not ask the SSH agent which keys it holds, so the key was not added: {e}"
            ))
        })?;
        match expected {
            Some(blob) if before.iter().any(|held| held == blob) => {
                tracing::debug!(?key_path, "SSH key already in the agent; not adding it");
                return Ok(());
            }
            None if !before.is_empty() => {
                tracing::warn!(
                    ?key_path,
                    "the public half of this SSH key is unreadable, so whether the agent \
                     already holds it cannot be told; not adding it"
                );
                return Err(AgentError::AddKeyFailed(
                    "the agent holds other keys and this key has no readable public half \
                     (no .pub file beside it), so RustConn did not add it; add it in \
                     Settings → SSH Agent"
                        .to_owned(),
                ));
            }
            _ => {}
        }

        add()?;

        let after = match super::wire::request_identity_blobs(socket_path) {
            Ok(after) => after,
            Err(e) => {
                tracing::warn!(
                    ?key_path,
                    error = %e,
                    "could not ask the SSH agent what the add put there; only its lifetime will remove the key"
                );
                return Ok(());
            }
        };
        let appeared: Vec<Vec<u8>> = after.into_iter().filter(|b| !before.contains(b)).collect();
        if let Some(blob) = expected
            && !appeared.iter().any(|b| b == blob)
        {
            tracing::warn!(
                ?key_path,
                "the added SSH key did not appear under the public key its files name \
                 (a stale .pub?); only its lifetime will remove it"
            );
            return Ok(());
        }
        // ponytail: an identity someone else adds to the same agent between
        // the two queries is recorded too; the window is one local ssh-add.
        for blob in appeared {
            self.record(socket_path, blob);
        }
        Ok(())
    }

    /// No agent socket protocol off Unix: whether the agent already holds the
    /// key cannot be told, so nothing is added.
    #[cfg(not(unix))]
    pub(crate) fn add_and_record(
        &self,
        _socket_path: &str,
        _key_path: &std::path::Path,
        _expected: Option<&[u8]>,
        _add: impl FnOnce() -> AgentResult<()>,
    ) -> AgentResult<()> {
        Err(AgentError::AddKeyFailed(
            "adding keys on RustConn's own needs a Unix agent socket".to_owned(),
        ))
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

/// Adds the key at `key_path` to the agent at `socket_path` through `add`, on
/// the terms of [`OwnKeys::add_and_record`], and records what it added in the
/// process-wide record.
pub(crate) fn add_and_record(
    socket_path: &str,
    key_path: &std::path::Path,
    add: impl FnOnce() -> AgentResult<()>,
) -> AgentResult<()> {
    let expected = super::keyfile::public_blob_for_key_path(key_path);
    OWN_KEYS.add_and_record(socket_path, key_path, expected.as_deref(), add)
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
    use std::cell::Cell;
    use std::io::{Read as _, Write as _};
    use std::os::unix::net::{UnixListener, UnixStream};
    use std::path::Path;
    use std::sync::Arc;

    use super::*;
    use crate::lock_events::{LockEvent, Subscribers};

    /// `SSH_AGENTC_REQUEST_IDENTITIES`.
    const REQUEST_IDENTITIES: u8 = 11;
    /// `SSH_AGENTC_REMOVE_ALL_IDENTITIES`, which RustConn must never send.
    const REMOVE_ALL_IDENTITIES: u8 = 19;
    /// A key path for log fields only; nothing reads it.
    const KEY_PATH: &str = "/nonexistent/rustconn-test/id_ed25519";

    /// What the fake agent holds and every message type it was sent.
    #[derive(Default)]
    struct AgentState {
        keys: Vec<Vec<u8>>,
        requests: Vec<u8>,
    }

    /// A minimal ssh-agent on a socket in a private temp dir. It understands
    /// `REQUEST_IDENTITIES`, `REMOVE_IDENTITY` and `REMOVE_ALL_IDENTITIES`, and
    /// answers anything else with `FAILURE`.
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

        /// What `ssh-add` does to the agent: one more identity.
        fn insert(&self, blob: &[u8]) {
            self.state.lock().unwrap().keys.push(blob.to_vec());
        }
    }

    fn ssh_string(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
        out.extend_from_slice(bytes);
    }

    fn serve(mut stream: UnixStream, state: &Mutex<AgentState>) {
        loop {
            let mut len = [0u8; 4];
            if stream.read_exact(&mut len).is_err() {
                return;
            }
            let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
            stream.read_exact(&mut body).unwrap();
            let reply = answer(&mut state.lock().unwrap(), &body);
            let mut framed = u32::try_from(reply.len()).unwrap().to_be_bytes().to_vec();
            framed.extend_from_slice(&reply);
            stream.write_all(&framed).unwrap();
        }
    }

    /// The fake agent's reply body (type byte + payload) to one request.
    fn answer(state: &mut AgentState, body: &[u8]) -> Vec<u8> {
        state.requests.push(body[0]);
        let status = |ok: bool| vec![if ok { 6 } else { 5 }];
        match body[0] {
            REQUEST_IDENTITIES => {
                let mut answer = vec![12];
                let count = u32::try_from(state.keys.len()).unwrap();
                answer.extend_from_slice(&count.to_be_bytes());
                for key in &state.keys {
                    ssh_string(&mut answer, key);
                    ssh_string(&mut answer, b"");
                }
                answer
            }
            super::super::wire::REMOVE_IDENTITY => {
                let blob = &body[5..];
                let held = state.keys.iter().position(|k| k == blob);
                status(held.map(|i| state.keys.remove(i)).is_some())
            }
            REMOVE_ALL_IDENTITIES => {
                state.keys.clear();
                status(true)
            }
            _ => status(false),
        }
    }

    /// A distinct ed25519-shaped blob per `tag`.
    fn blob(tag: u8) -> Vec<u8> {
        let mut blob = Vec::new();
        ssh_string(&mut blob, b"ssh-ed25519");
        ssh_string(&mut blob, &[tag; 32]);
        blob
    }

    /// Adds `adds` to `agent` through `own`, as `ssh-add` would, with
    /// `expected` as the blob read from the key's files. Returns the result
    /// and whether the add ran at all.
    fn add_via(
        own: &OwnKeys,
        agent: &FakeAgent,
        expected: Option<&[u8]>,
        adds: &[u8],
    ) -> (AgentResult<()>, bool) {
        let ran = Cell::new(false);
        let result = own.add_and_record(&agent.socket, Path::new(KEY_PATH), expected, || {
            ran.set(true);
            agent.insert(adds);
            Ok(())
        });
        (result, ran.get())
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

    /// The add path end to end against one agent: asked before, added to,
    /// asked after, and on eviction only the added key leaves it.
    #[test]
    fn an_added_key_is_recorded_and_alone_evicted() {
        let (ours, users) = (blob(10), blob(11));
        let agent = FakeAgent::holding(&[&users]);
        let own = OwnKeys::new();

        let (result, ran) = add_via(&own, &agent, Some(&ours), &ours);
        assert!(result.is_ok());
        assert!(ran, "a key the agent lacks is added");
        assert_eq!(own.len(), 1);
        assert_eq!(
            agent.requests(),
            vec![REQUEST_IDENTITIES, REQUEST_IDENTITIES],
            "both queries went to the agent the add wrote to"
        );

        assert_eq!(own.evict(), 1);
        assert_eq!(agent.keys(), vec![users]);
    }

    #[test]
    fn a_key_the_agent_already_holds_is_neither_added_nor_recorded() {
        let users = blob(12);
        let agent = FakeAgent::holding(&[&users]);
        let own = OwnKeys::new();

        let (result, ran) = add_via(&own, &agent, Some(&users), &users);
        assert!(result.is_ok());
        assert!(!ran, "re-adding would replace the user's constraints");
        assert_eq!(own.len(), 0);
        assert_eq!(own.evict(), 0);
        assert_eq!(agent.keys(), vec![users]);
    }

    #[test]
    fn an_unreadable_public_half_is_not_added_beside_other_keys() {
        let users = blob(13);
        let agent = FakeAgent::holding(&[&users]);
        let own = OwnKeys::new();

        let (result, ran) = add_via(&own, &agent, None, &users);
        assert!(matches!(result, Err(AgentError::AddKeyFailed(_))));
        assert!(!ran, "the key might be the user's, so it is not re-added");
        assert_eq!(own.len(), 0);
    }

    #[test]
    fn an_unreadable_public_half_goes_into_an_empty_agent_and_is_recorded() {
        let ours = blob(14);
        let agent = FakeAgent::holding(&[]);
        let own = OwnKeys::new();

        let (result, ran) = add_via(&own, &agent, None, &ours);
        assert!(result.is_ok());
        assert!(ran, "an empty agent cannot hold the user's copy");
        assert_eq!(own.evict(), 1, "the identity that appeared is recorded");
        assert!(agent.keys().is_empty());
    }

    /// A stale `.pub` names a key other than the one `ssh-add` loads. The
    /// blob it names is never recorded, so eviction cannot remove a user's
    /// key that happens to match it, and the key that did appear stays to
    /// its lifetime.
    #[test]
    fn a_stale_public_half_records_nothing() {
        let (named_by_stale_pub, loaded) = (blob(15), blob(16));
        let agent = FakeAgent::holding(&[]);
        let own = OwnKeys::new();

        let (result, ran) = add_via(&own, &agent, Some(&named_by_stale_pub), &loaded);
        assert!(result.is_ok());
        assert!(ran);
        assert_eq!(own.len(), 0);
        assert_eq!(own.evict(), 0);
        assert_eq!(agent.keys(), vec![loaded]);
    }

    #[test]
    fn an_unreachable_agent_is_not_added_to() {
        let own = OwnKeys::new();
        let ran = Cell::new(false);
        let result = own.add_and_record(
            "/nonexistent/rustconn-test/agent.sock",
            Path::new(KEY_PATH),
            Some(&blob(17)),
            || {
                ran.set(true);
                Ok(())
            },
        );
        assert!(matches!(result, Err(AgentError::AddKeyFailed(_))));
        assert!(!ran.get());
        assert_eq!(own.len(), 0);
    }

    #[test]
    fn a_failed_add_records_nothing() {
        let agent = FakeAgent::holding(&[]);
        let own = OwnKeys::new();
        let result =
            own.add_and_record(&agent.socket, Path::new(KEY_PATH), Some(&blob(18)), || {
                Err(AgentError::AddKeyFailed("bad key".to_owned()))
            });
        assert!(matches!(result, Err(AgentError::AddKeyFailed(_))));
        assert_eq!(own.len(), 0);
    }
}
