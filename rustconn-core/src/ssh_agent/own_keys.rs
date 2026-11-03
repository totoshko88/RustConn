//! The SSH keys RustConn put into an agent on its own, so it can take exactly
//! those back out.
//!
//! When a connect or an SFTP/mc launch needs a key in the agent, RustConn asks
//! the agent which identities it holds, runs `ssh-add -t 43200`
//! ([`OWN_KEY_LIFETIME`]) only if the key is not among them, asks again, and
//! records the key's public blob together with the agent socket all three went
//! to ([`add_and_record`]). On exit, screen lock and suspend,
//! [`evict_own_keys`] sends one `SSH_AGENTC_REMOVE_IDENTITY` per recorded blob
//! to that same socket.
//!
//! Keys the user loaded are never recorded. A key the agent already holds is
//! not added again, because re-adding would replace its constraints. When the
//! agent cannot say what it holds, or the key's public half is unreadable (a
//! legacy PEM with no `.pub` beside it) and the agent holds keys this one could
//! be, nothing is added.
//!
//! Only a blob read from the private key file itself is ever recorded (an
//! OpenSSH key carries its public key in the clear), and only once the agent
//! shows it after the add. A key whose file does not carry it (a legacy PEM or
//! PKCS#8 key, with or without a `.pub` that may be stale) is added but not
//! recorded: the lifetime is its only expiry, so the record can never cover a
//! key the user adds later. A key the user adds in Settings → SSH Agent is
//! dropped from the record, because from then on it is theirs. Nothing here
//! removes all identities.
//!
//! No passphrase is asked for or kept.

use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use super::keyfile::KeyFileBlobs;
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
    /// the key when its own file names it.
    ///
    /// `blobs` is what the key's files say its public blob is. Their best
    /// guess decides whether to add: a key the agent already holds is left
    /// alone, and with no guess at all the add runs only into an agent that
    /// holds nothing. Only [`KeyFileBlobs::key_file`] is recorded, and only
    /// when the agent shows it after the add; a key without it is left to its
    /// lifetime. `key_path` only names the key in the log.
    #[cfg(unix)]
    pub(crate) fn add_and_record(
        &self,
        socket_path: &str,
        key_path: &Path,
        blobs: &KeyFileBlobs,
        add: impl FnOnce() -> AgentResult<()>,
    ) -> AgentResult<()> {
        let before = super::wire::request_identity_blobs(socket_path).map_err(|e| {
            tracing::warn!(?key_path, error = %e, "could not ask the SSH agent which keys it holds; not adding the key");
            AgentError::AddKeyFailed(format!(
                "could not ask the SSH agent which keys it holds, so the key was not added: {e}"
            ))
        })?;
        match blobs.best() {
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
                return Err(AgentError::KeyPresenceUnknown(
                    "the agent holds other keys and this key has no readable public half \
                     (no .pub file beside it), so RustConn did not add it"
                        .to_owned(),
                ));
            }
            _ => {}
        }

        add()?;

        let Some(blob) = blobs.key_file.as_deref() else {
            tracing::info!(
                ?key_path,
                "this SSH key file does not carry its public key (a legacy PEM or PKCS#8 key), \
                 so RustConn does not record it; only its lifetime will remove it"
            );
            return Ok(());
        };
        match super::wire::request_identity_blobs(socket_path) {
            Ok(after) if after.iter().any(|held| held == blob) => {
                self.record(socket_path, blob.to_vec());
            }
            Ok(_) => tracing::warn!(
                ?key_path,
                "the added SSH key did not appear under the public key its file names; \
                 only its lifetime will remove it"
            ),
            Err(e) => tracing::warn!(
                ?key_path,
                error = %e,
                "could not ask the SSH agent what the add put there; only its lifetime will remove the key"
            ),
        }
        Ok(())
    }

    /// No agent socket protocol off Unix: whether the agent already holds the
    /// key cannot be told, so nothing is added.
    #[cfg(not(unix))]
    pub(crate) fn add_and_record(
        &self,
        _socket_path: &str,
        _key_path: &Path,
        _blobs: &KeyFileBlobs,
        _add: impl FnOnce() -> AgentResult<()>,
    ) -> AgentResult<()> {
        Err(AgentError::KeyPresenceUnknown(
            "adding keys on RustConn's own needs a Unix agent socket".to_owned(),
        ))
    }

    /// Stops tracking a key the user has just added at `socket_path`
    /// themselves: from then on it is theirs.
    ///
    /// With the key file's own blob, only that key is dropped. Without it (a
    /// legacy PEM or PKCS#8 key) the user's key could be any recorded one, as
    /// the same key can sit in two files, so every key recorded for that
    /// socket is dropped; those then leave the agent when their lifetime ends.
    pub(crate) fn forget(&self, socket_path: &str, blobs: &KeyFileBlobs) {
        let mut keys = self.lock();
        match blobs.key_file.as_deref() {
            Some(blob) => keys.retain(|k| !(k.socket_path == socket_path && k.blob == blob)),
            None => keys.retain(|k| k.socket_path != socket_path),
        }
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
    key_path: &Path,
    add: impl FnOnce() -> AgentResult<()>,
) -> AgentResult<()> {
    let blobs = super::keyfile::key_file_blobs(key_path);
    OWN_KEYS.add_and_record(socket_path, key_path, &blobs, add)
}

/// Stops tracking the key at `key_path` in the agent at `socket_path`, on the
/// terms of [`OwnKeys::forget`].
pub(crate) fn forget(socket_path: &str, key_path: &Path) {
    OWN_KEYS.forget(socket_path, &super::keyfile::key_file_blobs(key_path));
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

    /// What an OpenSSH private key file says: its own public blob.
    fn from_key_file(blob: &[u8]) -> KeyFileBlobs {
        KeyFileBlobs {
            key_file: Some(blob.to_vec()),
            pub_file: None,
        }
    }

    /// What a legacy PEM key with a `.pub` beside it says.
    fn from_pub_file(blob: &[u8]) -> KeyFileBlobs {
        KeyFileBlobs {
            key_file: None,
            pub_file: Some(blob.to_vec()),
        }
    }

    /// Adds `adds` to `agent` through `own`, as `ssh-add` would, with
    /// `blobs` as what the key's files say. Returns the result and whether
    /// the add ran at all.
    fn add_via(
        own: &OwnKeys,
        agent: &FakeAgent,
        blobs: &KeyFileBlobs,
        adds: &[u8],
    ) -> (AgentResult<()>, bool) {
        let ran = Cell::new(false);
        let result = own.add_and_record(&agent.socket, Path::new(KEY_PATH), blobs, || {
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
        own.forget(&agent.socket, &from_key_file(&key));

        assert_eq!(own.evict(), 0);
        assert_eq!(agent.keys(), vec![key]);
        assert!(agent.requests().is_empty(), "nothing was sent to the agent");
    }

    /// The user adds a key whose file does not carry its public key. It could
    /// be any recorded key (the same key can sit in two files), so nothing
    /// recorded for that agent is evicted any more; another agent's record
    /// stays.
    #[test]
    fn forgetting_without_the_key_files_blob_drops_that_agents_whole_record() {
        let (first, second, elsewhere) = (blob(19), blob(20), blob(21));
        let agent = FakeAgent::holding(&[&first, &second]);
        let own = OwnKeys::new();
        own.record(&agent.socket, first.clone());
        own.record(&agent.socket, second.clone());
        own.record("/nonexistent/rustconn-test/other.sock", elsewhere);

        own.forget(&agent.socket, &from_pub_file(&first));
        assert_eq!(own.len(), 1, "only the other agent's key is still recorded");

        own.evict();
        assert_eq!(agent.keys(), vec![first, second]);
        assert!(
            agent.requests().is_empty(),
            "nothing was sent to this agent"
        );
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

        let (result, ran) = add_via(&own, &agent, &from_key_file(&ours), &ours);
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

        let (result, ran) = add_via(&own, &agent, &from_key_file(&users), &users);
        assert!(result.is_ok());
        assert!(!ran, "re-adding would replace the user's constraints");
        assert_eq!(own.len(), 0);
        assert_eq!(own.evict(), 0);
        assert_eq!(agent.keys(), vec![users]);
    }

    #[test]
    fn a_key_its_pub_file_names_as_held_is_not_added() {
        let users = blob(22);
        let agent = FakeAgent::holding(&[&users]);
        let own = OwnKeys::new();

        let (result, ran) = add_via(&own, &agent, &from_pub_file(&users), &users);
        assert!(result.is_ok());
        assert!(!ran);
        assert_eq!(own.len(), 0);
    }

    /// Whether the agent holds a key with no readable public half cannot be
    /// told; that is its own outcome, so a launcher can warn and go on.
    #[test]
    fn an_unreadable_public_half_is_not_added_beside_other_keys() {
        let users = blob(13);
        let agent = FakeAgent::holding(&[&users]);
        let own = OwnKeys::new();

        let (result, ran) = add_via(&own, &agent, &KeyFileBlobs::default(), &users);
        assert!(matches!(result, Err(AgentError::KeyPresenceUnknown(_))));
        assert!(!ran, "the key might be the user's, so it is not re-added");
        assert_eq!(own.len(), 0);
    }

    /// A key whose public blob does not come from its own file goes into an
    /// empty agent, but is not recorded: its lifetime is its only expiry.
    #[test]
    fn a_blobless_automatic_add_records_nothing() {
        let ours = blob(14);
        let agent = FakeAgent::holding(&[]);
        let own = OwnKeys::new();

        let (result, ran) = add_via(&own, &agent, &KeyFileBlobs::default(), &ours);
        assert!(result.is_ok());
        assert!(ran, "an empty agent cannot hold the user's copy");
        assert_eq!(own.len(), 0);
        assert_eq!(own.evict(), 0);
        assert_eq!(agent.keys(), vec![ours]);
        assert!(
            !agent
                .requests()
                .contains(&super::super::wire::REMOVE_IDENTITY)
        );
    }

    /// The case the record must never cover: RustConn added a blob-less key
    /// on its own, then the user added the same file in Settings. Lock,
    /// suspend and exit leave the user's key in the agent.
    #[test]
    fn a_later_settings_add_of_a_blobless_key_leaves_nothing_to_evict() {
        let key = blob(23);
        let agent = FakeAgent::holding(&[]);
        let own = OwnKeys::new();

        let (result, ran) = add_via(&own, &agent, &KeyFileBlobs::default(), &key);
        assert!(result.is_ok() && ran);
        // Settings → SSH Agent → Add key of the same file.
        own.forget(&agent.socket, &KeyFileBlobs::default());

        assert_eq!(own.evict(), 0);
        assert_eq!(agent.keys(), vec![key]);
        assert!(
            !agent
                .requests()
                .contains(&super::super::wire::REMOVE_IDENTITY)
        );
    }

    /// A stale `.pub` names a key other than the one `ssh-add` loads. Only a
    /// blob from the key file itself is ever recorded, so neither the key it
    /// names nor the key that did appear is: the latter stays to its lifetime.
    #[test]
    fn a_stale_public_half_records_nothing() {
        let (named_by_stale_pub, loaded) = (blob(15), blob(16));
        let agent = FakeAgent::holding(&[]);
        let own = OwnKeys::new();

        let (result, ran) = add_via(&own, &agent, &from_pub_file(&named_by_stale_pub), &loaded);
        assert!(result.is_ok());
        assert!(ran);
        assert_eq!(own.len(), 0);
        assert_eq!(own.evict(), 0);
        assert_eq!(agent.keys(), vec![loaded]);
    }

    /// The key file's blob did not show up after the add: nothing else that
    /// appeared is recorded in its place.
    #[test]
    fn a_key_file_blob_that_did_not_appear_records_nothing() {
        let (named, loaded) = (blob(24), blob(25));
        let agent = FakeAgent::holding(&[]);
        let own = OwnKeys::new();

        let (result, ran) = add_via(&own, &agent, &from_key_file(&named), &loaded);
        assert!(result.is_ok() && ran);
        assert_eq!(own.len(), 0);
        assert_eq!(agent.keys(), vec![loaded]);
    }

    #[test]
    fn an_unreachable_agent_is_not_added_to() {
        let own = OwnKeys::new();
        let ran = Cell::new(false);
        let result = own.add_and_record(
            "/nonexistent/rustconn-test/agent.sock",
            Path::new(KEY_PATH),
            &from_key_file(&blob(17)),
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
        let result = own.add_and_record(
            &agent.socket,
            Path::new(KEY_PATH),
            &from_key_file(&blob(18)),
            || Err(AgentError::AddKeyFailed("bad key".to_owned())),
        );
        assert!(matches!(result, Err(AgentError::AddKeyFailed(_))));
        assert_eq!(own.len(), 0);
    }
}
