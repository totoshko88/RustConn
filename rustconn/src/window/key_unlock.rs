//! The check every SSH connect path makes before ssh starts: does a key it
//! passes need unlocking first?
//!
//! The initial connect, the in-place reconnect, an SFTP-type connect and the
//! sidebar's SFTP/mc action each compute their [`UnlockTarget`]s (the key
//! files ssh, the first jump hop or the SFTP add will use, and the agent they
//! go to) and hand the rest of their work to [`unlock_then`]. For an encrypted
//! OpenSSH key the agent does not hold, that shows RustConn's unlock dialog
//! once (`dialogs::key_unlock`) and then goes on; for everything else it goes
//! on at once. Only an encrypted OpenSSH key is a target at all, so a connect
//! with any other key starts synchronously, as before. The decision is
//! `rustconn_core::ssh_agent::check_key_unlock`, run off the GTK thread.
//! Every outcome continues the connect: Cancel, too
//! many wrong passphrases and a refusing agent leave ssh to ask as before.
//! A Cancel is remembered for that key and agent for the rest of the process,
//! so later connects with it do not ask again.
//!
//! A cluster or a restored session can connect several times with one key at
//! once. The second connect does not open a second dialog: it waits for the
//! first one's dialog and goes on when it closes (`InFlight`).

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::path::{Path, PathBuf};

use gtk4::prelude::*;
use rustconn_core::connection::ssh_inheritance;
use rustconn_core::ssh_agent::{
    UnlockDecision, agent_refused_add, check_key_unlock, key_is_encrypted_openssh,
};

use crate::dialogs::key_unlock::{KeyUnlockOutcome, show_key_unlock_dialog};
use crate::i18n::{i18n, i18n_f};
use crate::state::SharedAppState;

/// One key a connect passes, and the agent socket it would be unlocked into.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct UnlockTarget {
    key_path: PathBuf,
    socket: Option<String>,
}

/// The rest of a connect, run once the unlock check is done.
type Continuation = Box<dyn FnOnce()>;

/// The keys an SSH connect of `conn` passes to ssh: its own identity file and
/// that of its first jump hop, resolved exactly as `build_ssh_command_args`
/// resolves them, with the agent socket the session's ssh will use.
pub(super) fn ssh_unlock_targets(
    state: &SharedAppState,
    conn: &rustconn_core::Connection,
) -> Vec<UnlockTarget> {
    let rustconn_core::ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config else {
        return Vec::new();
    };
    let (groups, keys) = {
        let Ok(state_ref) = state.try_borrow() else {
            return Vec::new();
        };
        let groups = state_ref.list_groups_owned();
        let mut keys: Vec<String> = Vec::new();
        keys.extend(super::protocols_ssh::resolved_key_file(conn, &groups));
        if !rustconn_core::ssh_tunnel::has_unmanaged_proxy_route(ssh_config)
            && let Some(jump_id) = super::protocols::resolve_first_hop_id(&state_ref, conn)
            && jump_id != conn.id
            && let Some(jump_conn) = state_ref.get_connection(jump_id)
        {
            keys.extend(super::protocols_ssh::resolved_key_file(jump_conn, &groups));
        }
        (groups, keys)
    };
    if keys.is_empty() {
        return Vec::new();
    }
    // The order the terminal spawn sets `SSH_AUTH_SOCK` in; the jump hop's ssh
    // inherits that environment.
    let socket = ssh_inheritance::resolve_ssh_agent_socket(conn, &groups)
        .filter(|s| !s.is_empty())
        .or_else(rustconn_core::sftp::default_agent_socket);
    retain_askable(targets_from(
        keys.into_iter().map(PathBuf::from),
        socket.as_deref(),
    ))
}

/// The key an SFTP or mc launch of `conn` adds to the agent, and the agent
/// `rustconn_core::sftp::ensure_identity_loaded` adds it to.
pub(super) fn sftp_unlock_targets(
    conn: &rustconn_core::Connection,
    groups: &[rustconn_core::ConnectionGroup],
) -> Vec<UnlockTarget> {
    let key = rustconn_core::sftp::get_ssh_key_path(conn, groups)
        .and_then(|p| rustconn_core::resolve_key_path(&p));
    retain_askable(targets_from(
        key.into_iter(),
        rustconn_core::sftp::default_agent_socket().as_deref(),
    ))
}

/// Keeps the targets the unlock check could ask about: an encrypted OpenSSH
/// key, with an agent that has not refused an add. The rest cannot need a
/// dialog, so a connect left with none starts at once, synchronously, as it
/// did before this check existed.
///
/// Reads each key file here, on the GTK thread: a few hundred bytes from a
/// local file, no agent query. The agent is asked off the thread later.
///
/// A key whose dialog the user cancelled for that agent is dropped too
/// ([`DECLINED`]), so a later connect with it starts at once and ssh asks in
/// the terminal, as it did right after the Cancel.
fn retain_askable(mut targets: Vec<UnlockTarget>) -> Vec<UnlockTarget> {
    targets.retain(|t| {
        t.socket.as_deref().is_some_and(|s| {
            !s.is_empty()
                && !agent_refused_add(s)
                && !DECLINED.with(|d| d.borrow().contains(s, &t.key_path))
        }) && key_is_encrypted_openssh(&t.key_path)
    });
    targets
}

fn targets_from(keys: impl Iterator<Item = PathBuf>, socket: Option<&str>) -> Vec<UnlockTarget> {
    let mut targets: Vec<UnlockTarget> = Vec::new();
    for key_path in keys {
        if !targets.iter().any(|t| t.key_path == key_path) {
            targets.push(UnlockTarget {
                key_path,
                socket: socket.map(str::to_owned),
            });
        }
    }
    targets
}

/// Runs `then` once every key in `targets` that needs unlocking has had its
/// dialog, or at once when none does.
///
/// With no targets (a password connection, or keys that cannot need a dialog,
/// see [`retain_askable`]) `then` runs synchronously, so nothing changes for
/// those connects. Otherwise the check runs off the GTK thread and `then`
/// runs from its callback.
pub(super) fn unlock_then(
    parent: &impl IsA<gtk4::Widget>,
    targets: Vec<UnlockTarget>,
    then: impl FnOnce() + 'static,
) {
    if targets.is_empty() {
        then();
        return;
    }
    let parent: gtk4::Widget = parent.upcast_ref::<gtk4::Widget>().clone();
    crate::utils::spawn_blocking_with_callback(
        move || {
            targets
                .into_iter()
                .filter_map(|t| {
                    let socket = t.socket?;
                    (check_key_unlock(Some(&socket), &t.key_path) == UnlockDecision::AskPassphrase)
                        .then_some((socket, t.key_path))
                })
                .collect::<Vec<_>>()
        },
        move |asks| ask_each(parent, asks, Box::new(then)),
    );
}

thread_local! {
    /// The keys whose unlock dialog is open, with the connects waiting on it.
    static IN_FLIGHT: RefCell<InFlight<(String, PathBuf), Continuation>> =
        RefCell::new(InFlight::new());

    /// The keys whose unlock dialog the user cancelled, per agent socket, for
    /// the rest of the process. Later connects with such a key do not ask
    /// again; the user can still add it in Settings → SSH Agent.
    static DECLINED: RefCell<Declined> = RefCell::new(Declined::default());
}

/// Asks for the first of `asks`, then the rest, then runs `then`.
fn ask_each(parent: gtk4::Widget, mut asks: Vec<(String, PathBuf)>, then: Continuation) {
    // Cancelled since this connect's check ran (another connect's dialog).
    DECLINED.with(|d| {
        let declined = d.borrow();
        asks.retain(|(socket, key_path)| !declined.contains(socket, key_path));
    });
    if asks.is_empty() {
        then();
        return;
    }
    let key = asks.remove(0);
    let next_parent = parent.clone();
    let next: Continuation = Box::new(move || ask_each(next_parent, asks, then));

    let first = IN_FLIGHT.with(|f| f.borrow_mut().begin(key.clone()));
    if !first {
        // Another connect has this key's dialog open; go on when it closes.
        let unparked = IN_FLIGHT.with(|f| f.borrow_mut().park(&key, next));
        if let Some(next) = unparked {
            next();
        }
        return;
    }
    let (socket, key_path) = key.clone();
    let dialog_key_path = key_path.clone();
    show_key_unlock_dialog(&parent, &dialog_key_path, &socket, move |outcome| {
        report(&key_path, &outcome);
        if outcome == KeyUnlockOutcome::Cancelled {
            DECLINED.with(|d| d.borrow_mut().insert(key.clone()));
        }
        // Taken out first, so no borrow is held while they run.
        let waiters = IN_FLIGHT.with(|f| f.borrow_mut().finish(&key));
        next();
        for waiter in waiters {
            waiter();
        }
    });
}

/// Tells the user why the key did not go into the agent, when that is not
/// their own choice.
fn report(key_path: &std::path::Path, outcome: &KeyUnlockOutcome) {
    match outcome {
        KeyUnlockOutcome::Unlocked | KeyUnlockOutcome::Cancelled => {}
        KeyUnlockOutcome::Refused => crate::toast::show_warning_toast_on_active_window(&i18n(
            "The SSH agent refused the key, so it was not added.",
        )),
        KeyUnlockOutcome::GaveUp => crate::toast::show_warning_toast_on_active_window(&i18n(
            "Too many incorrect passphrases. The key was not added to the SSH agent.",
        )),
        KeyUnlockOutcome::Failed(detail) => {
            tracing::warn!(?key_path, %detail, "could not add the unlocked SSH key to the agent");
            crate::toast::show_warning_toast_on_active_window(&i18n_f(
                "Could not add the SSH key to the agent: {}",
                &[detail.as_str()],
            ));
        }
    }
}

/// The (agent socket, key path) pairs whose unlock the user cancelled.
#[derive(Debug, Default)]
struct Declined {
    keys: HashSet<(String, PathBuf)>,
}

impl Declined {
    fn insert(&mut self, key: (String, PathBuf)) {
        self.keys.insert(key);
    }

    fn contains(&self, socket: &str, key_path: &Path) -> bool {
        self.keys
            .contains(&(socket.to_owned(), key_path.to_path_buf()))
    }
}

/// Which keys have a dialog open, and who waits for each.
struct InFlight<K, W> {
    asking: HashMap<K, Vec<W>>,
}

impl<K: Eq + Hash, W> InFlight<K, W> {
    fn new() -> Self {
        Self {
            asking: HashMap::new(),
        }
    }

    /// `true` when nobody is asking for `key` yet; the caller asks now.
    fn begin(&mut self, key: K) -> bool {
        if self.asking.contains_key(&key) {
            return false;
        }
        self.asking.insert(key, Vec::new());
        true
    }

    /// Waits for the dialog open for `key`. With none open, the waiter is
    /// handed back.
    fn park(&mut self, key: &K, waiter: W) -> Option<W> {
        match self.asking.get_mut(key) {
            Some(waiters) => {
                waiters.push(waiter);
                None
            }
            None => Some(waiter),
        }
    }

    /// Ends the dialog for `key` and returns its waiters, in order.
    fn finish(&mut self, key: &K) -> Vec<W> {
        self.asking.remove(key).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_connect_asks_and_the_second_waits() {
        let mut in_flight: InFlight<&str, u8> = InFlight::new();
        assert!(in_flight.begin("key"));
        assert!(!in_flight.begin("key"));
        assert_eq!(in_flight.park(&"key", 1), None);
        assert_eq!(in_flight.park(&"key", 2), None);
        assert_eq!(in_flight.finish(&"key"), vec![1, 2], "waiters in order");
        assert!(in_flight.begin("key"), "the entry is cleared");
    }

    #[test]
    fn different_keys_are_independent() {
        let mut in_flight: InFlight<&str, u8> = InFlight::new();
        assert!(in_flight.begin("a"));
        assert!(in_flight.begin("b"));
        assert_eq!(in_flight.park(&"a", 1), None);
        assert_eq!(in_flight.finish(&"b"), Vec::<u8>::new());
        assert_eq!(in_flight.finish(&"a"), vec![1]);
    }

    #[test]
    fn a_cancel_is_remembered_for_that_key_and_agent_only() {
        let mut declined = Declined::default();
        declined.insert(("/run/a.sock".to_owned(), PathBuf::from("/k/a")));
        assert!(declined.contains("/run/a.sock", Path::new("/k/a")));
        assert!(
            !declined.contains("/run/b.sock", Path::new("/k/a")),
            "another agent still asks"
        );
        assert!(
            !declined.contains("/run/a.sock", Path::new("/k/b")),
            "another key still asks"
        );
    }

    #[test]
    fn a_waiter_with_no_dialog_open_is_handed_back() {
        let mut in_flight: InFlight<&str, u8> = InFlight::new();
        assert_eq!(in_flight.park(&"key", 7), Some(7));
    }

    /// A target that cannot need a dialog is dropped, so the connect stays
    /// synchronous. The encrypted case is covered in `rustconn-core`
    /// (`key_is_encrypted_openssh`), which owns the key fixtures.
    #[test]
    fn a_key_that_cannot_need_a_dialog_is_not_a_target() {
        let dir = tempfile::tempdir().unwrap();
        let unencrypted = dir.path().join("id_plain");
        std::fs::write(
            &unencrypted,
            "-----BEGIN RSA PRIVATE KEY-----\nAAAA\n-----END RSA PRIVATE KEY-----\n",
        )
        .unwrap();
        let missing = dir.path().join("id_missing");
        let targets = vec![
            UnlockTarget {
                key_path: unencrypted,
                socket: Some("/run/agent.sock".to_owned()),
            },
            UnlockTarget {
                key_path: missing,
                socket: Some("/run/agent.sock".to_owned()),
            },
            UnlockTarget {
                key_path: dir.path().join("id_any"),
                socket: None,
            },
        ];
        assert!(retain_askable(targets).is_empty());
    }

    #[test]
    fn a_key_passed_twice_is_one_target() {
        let targets = targets_from(
            [
                PathBuf::from("/k/a"),
                PathBuf::from("/k/b"),
                PathBuf::from("/k/a"),
            ]
            .into_iter(),
            Some("/run/agent.sock"),
        );
        assert_eq!(
            targets
                .iter()
                .map(|t| t.key_path.clone())
                .collect::<Vec<_>>(),
            vec![PathBuf::from("/k/a"), PathBuf::from("/k/b")]
        );
        assert!(
            targets
                .iter()
                .all(|t| t.socket.as_deref() == Some("/run/agent.sock"))
        );
    }
}
