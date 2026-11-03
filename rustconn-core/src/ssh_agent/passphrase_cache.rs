//! Session passphrase cache for SSH private keys.
//!
//! When a user unlocks an encrypted private key once in a session — by typing
//! its passphrase into the SSH Agent settings dialog, or on a connect that
//! loads the identity — [`add_key`](super::SshAgentManager::add_key) records
//! the passphrase here, keyed by the key file's path. A later `ensure identity`
//! on the same key (for example a multi-hop `-J` connect) can then load it
//! non-interactively instead of asking again or failing.
//!
//! # Security model
//!
//! - The cache lives only in this process's memory. Nothing is written to disk,
//!   and it does not survive a restart.
//! - Each entry is a [`SecretString`], so the passphrase is held zeroized-on-drop
//!   and never appears in a `Debug` rendering of the cache.
//! - The cache is cleared wholesale by [`clear`] — the lock/suspend listener
//!   (W3 phase 4, second sub-step) calls it so a screen lock drops every cached
//!   passphrase, matching the agent's own key-eviction on lock.
//! - A passphrase is remembered only after `ssh-add` *accepted* it (the caller
//!   checks the exit status first), so a wrong guess is never cached.
//!
//! The key is the key file's path, canonicalized when possible so that two
//! spellings of the same file (`~/.ssh/id_ed25519` vs an absolute path) share
//! one entry; an un-canonicalizable path (the file was removed) falls back to
//! the path as given.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use secrecy::SecretString;

/// The process-global store. `OnceLock` defers allocation until first use, and
/// the `Mutex` guards concurrent access from the GTK thread and any worker that
/// loads a key. A poisoned lock (a panic while held) is recovered rather than
/// propagated: a passphrase cache is best-effort, and refusing to serve it
/// because an unrelated thread panicked would only force an extra prompt.
static CACHE: OnceLock<Mutex<HashMap<PathBuf, SecretString>>> = OnceLock::new();

fn store() -> &'static Mutex<HashMap<PathBuf, SecretString>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Canonicalizes a key path for use as the cache key, falling back to the path
/// as given when it cannot be resolved (e.g. the file no longer exists).
fn normalize(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Records the passphrase that successfully unlocked `key_path` for the rest of
/// the session. Call this only after `ssh-add` accepted the passphrase.
pub fn remember(key_path: &Path, passphrase: &SecretString) {
    let mut guard = store()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.insert(normalize(key_path), passphrase.clone());
}

/// Returns the cached passphrase for `key_path`, if one was remembered this
/// session.
#[must_use]
pub fn get(key_path: &Path) -> Option<SecretString> {
    let guard = store()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.get(&normalize(key_path)).cloned()
}

/// Drops the cached passphrase for a single key (e.g. the user removed it from
/// the agent, or it turned out to be wrong).
pub fn forget(key_path: &Path) {
    let mut guard = store()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.remove(&normalize(key_path));
}

/// Drops every cached passphrase. Called on screen lock / suspend so a locked
/// session holds no key passphrases in memory.
pub fn clear() {
    let mut guard = store()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    guard.clear();
}

/// Number of cached entries. For tests and diagnostics.
#[must_use]
pub fn len() -> usize {
    store()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .len()
}

#[cfg(test)]
mod tests {
    use secrecy::{ExposeSecret, SecretString};

    use super::*;

    /// A unique temp path per test so the process-global cache does not let one
    /// test observe another's entry.
    fn unique_key_path(tag: &str) -> PathBuf {
        let dir = std::env::var("KIROCREW_SCRATCH")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir());
        dir.join(format!(
            "rustconn-passphrase-cache-test-{tag}-{}",
            std::process::id()
        ))
    }

    #[test]
    fn remember_then_get_round_trips() {
        let path = unique_key_path("roundtrip");
        assert!(get(&path).is_none(), "clean slate for this path");
        remember(&path, &SecretString::from("hunter2".to_string()));
        let got = get(&path).expect("passphrase was remembered");
        assert_eq!(got.expose_secret(), "hunter2");
        forget(&path);
        assert!(get(&path).is_none(), "forget removes the entry");
    }

    #[test]
    fn clear_drops_the_entry() {
        let path = unique_key_path("clear");
        remember(&path, &SecretString::from("secret".to_string()));
        assert!(get(&path).is_some());
        clear();
        assert!(get(&path).is_none(), "clear wipes everything");
    }

    /// The cache must not leak the passphrase through its `Debug`: the stored
    /// value is a `SecretString`, which renders redacted.
    #[test]
    fn stored_value_debug_does_not_leak() {
        let secret = SecretString::from("topsecret".to_string());
        let shown = format!("{secret:?}");
        assert!(
            !shown.contains("topsecret"),
            "SecretString must redact its Debug"
        );
    }
}
