//! fwknop SPA keys in the secret store (audit item F-08)
//!
//! Until 0.26.0 the Rijndael and HMAC keys of a connection were written to
//! `connections.toml` as typed. They now live in the configured secret backend,
//! one entry per key, addressed by the connection's UUID so that renaming or
//! moving the connection never orphans them. The file keeps only the
//! `*_key_in_vault` flags of [`SpaConfig`].
//!
//! This module is the GUI-free half: key naming, the [`SpaKeyStore`] seam the
//! GUI implements over its vault dispatcher, the one-way move
//! ([`move_keys_to_vault`]) and the connect-time lookup ([`resolve_spa_keys`]).
//! The file side (backup, clearing the plaintext) is
//! [`ConnectionManager::apply_spa_vault_move`](super::ConnectionManager::apply_spa_vault_move).
//!
//! The move never loses a key: a plaintext value is cleared only after the
//! vault accepted it **and** a read-back returned the same value.

use secrecy::{ExposeSecret, SecretString};
use uuid::Uuid;
use zeroize::Zeroizing;

use super::knock::SpaConfig;
use crate::models::Connection;

/// Name of the copy of `connections.toml` made before the first SPA move.
pub const PRE_VAULT_BACKUP: &str = "connections.toml.pre-spa-vault.bak";

/// Deletes the pre-move copy of `connections.toml` in `config_dir`.
///
/// It still holds the plaintext keys. Call it once a key has been read from
/// the vault and used: that proves the vault copy works, so the backup has
/// served its purpose. Returns whether a file was removed.
pub fn remove_pre_vault_backup(config_dir: &std::path::Path) -> bool {
    std::fs::remove_file(config_dir.join(PRE_VAULT_BACKUP)).is_ok()
}

/// Which of the two SPA keys an entry holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaKeyKind {
    /// The AES-256-CBC (Rijndael) key.
    Rijndael,
    /// The HMAC-SHA256 key.
    Hmac,
}

impl SpaKeyKind {
    const ALL: [Self; 2] = [Self::Rijndael, Self::Hmac];

    const fn as_str(self) -> &'static str {
        match self {
            Self::Rijndael => "rijndael",
            Self::Hmac => "hmac",
        }
    }
}

/// Flat vault key of one SPA key: `rustconn/spa/{connection id}/{kind}`.
///
/// Keyed by UUID, not by name or group path: those change on rename and move,
/// the id does not.
#[must_use]
pub fn spa_secret_key(connection_id: Uuid, kind: SpaKeyKind) -> String {
    format!("rustconn/spa/{connection_id}/{}", kind.as_str())
}

/// A secret store that holds SPA keys under flat keys.
///
/// Implemented by the GUI over its vault dispatcher (every backend) and by a
/// map in tests. Error strings must never contain a key.
pub trait SpaKeyStore {
    /// Stores `value` under `key`, replacing an earlier value.
    ///
    /// # Errors
    /// A human-readable reason: backend locked, missing, read-only, timed out.
    fn store(&self, key: &str, value: &SecretString) -> Result<(), String>;

    /// Reads the value under `key`; `Ok(None)` when there is none.
    ///
    /// # Errors
    /// A human-readable reason, as for [`Self::store`].
    fn load(&self, key: &str) -> Result<Option<SecretString>, String>;

    /// Removes the entry under `key`; an entry that is already gone is `Ok`.
    ///
    /// # Errors
    /// A human-readable reason, as for [`Self::store`].
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// Which of the two SPA keys a connection holds in the vault.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VaultFlags {
    /// The Rijndael key entry.
    pub rijndael: bool,
    /// The HMAC key entry.
    pub hmac: bool,
}

impl VaultFlags {
    /// Whether neither key is in play.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        !self.rijndael && !self.hmac
    }

    /// The keys `cfg` says are in the vault.
    #[must_use]
    pub fn of(cfg: Option<&SpaConfig>) -> Self {
        cfg.map_or_else(Self::default, |c| Self {
            rijndael: c.rijndael_key_in_vault,
            hmac: c.hmac_key_in_vault,
        })
    }

    fn has(self, kind: SpaKeyKind) -> bool {
        match kind {
            SpaKeyKind::Rijndael => self.rijndael,
            SpaKeyKind::Hmac => self.hmac,
        }
    }
}

/// The vault entries an edit leaves behind.
///
/// A key that was in the vault and is now neither in the vault nor typed as
/// plaintext (SPA switched off, or the key gone). A typed replacement is not an
/// orphan: the move overwrites the same entry.
#[must_use]
pub fn orphaned_after_edit(old: &Connection, new: &Connection) -> VaultFlags {
    let was = VaultFlags::of(old.spa_config.as_ref());
    let now = VaultFlags::of(new.spa_config.as_ref());
    let keeps = |kind: SpaKeyKind| {
        now.has(kind)
            || new
                .spa_config
                .as_ref()
                .is_some_and(|c| plaintext(c, kind).is_some())
    };
    VaultFlags {
        rijndael: was.rijndael && !keeps(SpaKeyKind::Rijndael),
        hmac: was.hmac && !keeps(SpaKeyKind::Hmac),
    }
}

/// Deletes the entries of `id` named by `flags`. Returns one reason per entry
/// that could not be removed (never a key).
pub fn remove_spa_keys(id: Uuid, flags: VaultFlags, store: &dyn SpaKeyStore) -> Vec<String> {
    SpaKeyKind::ALL
        .into_iter()
        .filter(|k| flags.has(*k))
        .filter_map(|k| store.delete(&spa_secret_key(id, k)).err())
        .collect()
}

/// What [`copy_spa_keys`] did.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SpaCopyReport {
    /// Entries now present under the new id.
    pub copied: VaultFlags,
    /// Entries the source flagged that did not arrive: the caller must clear
    /// these flags on the copy. The reasons never contain a key.
    pub missing: VaultFlags,
    /// Why, one per failed entry.
    pub failed: Vec<String>,
}

/// Copies the vault entries `src` flags from `from` to `to`, all or nothing.
///
/// Each entry is stored and read back. If one fails, the entries already
/// written under `to` are removed again and the report says nothing arrived,
/// so the copy never holds one key of a pair. Plaintext keys are not copied:
/// they travel with the connection and the move takes them from there.
pub fn copy_spa_keys(
    from: Uuid,
    src: &SpaConfig,
    to: Uuid,
    store: &dyn SpaKeyStore,
) -> SpaCopyReport {
    let wanted = VaultFlags::of(Some(src));
    let mut report = SpaCopyReport::default();
    let mut written: Vec<SpaKeyKind> = Vec::new();
    for kind in SpaKeyKind::ALL.into_iter().filter(|k| wanted.has(*k)) {
        let outcome = match store.load(&spa_secret_key(from, kind)) {
            Ok(Some(v)) if !v.expose_secret().is_empty() => {
                store_and_confirm(store, &spa_secret_key(to, kind), v.expose_secret())
            }
            Ok(_) => Err(format!(
                "the {} key is not in the secret store",
                kind.as_str()
            )),
            Err(e) => Err(e),
        };
        match outcome {
            Ok(()) => written.push(kind),
            Err(reason) => {
                report.failed.push(reason);
                break;
            }
        }
    }
    if report.failed.is_empty() {
        for kind in written {
            match kind {
                SpaKeyKind::Rijndael => report.copied.rijndael = true,
                SpaKeyKind::Hmac => report.copied.hmac = true,
            }
        }
        return report;
    }
    // Roll back, best effort; the copy holds none of them.
    for kind in written {
        let _ = store.delete(&spa_secret_key(to, kind));
    }
    // The failed entry itself may be half written (stored, read-back failed).
    for kind in SpaKeyKind::ALL.into_iter().filter(|k| wanted.has(*k)) {
        let _ = store.delete(&spa_secret_key(to, kind));
    }
    report.missing = wanted;
    report
}

fn plaintext(cfg: &SpaConfig, kind: SpaKeyKind) -> Option<&str> {
    match kind {
        SpaKeyKind::Rijndael => cfg.rijndael_key_ref.as_deref(),
        SpaKeyKind::Hmac => cfg.hmac_key_ref.as_deref(),
    }
    .filter(|k| !k.is_empty())
}

/// Whether `conn` still has an SPA key as plaintext that should move.
#[must_use]
pub fn has_plaintext_keys(conn: &Connection) -> bool {
    conn.spa_config
        .as_ref()
        .is_some_and(|c| SpaKeyKind::ALL.iter().any(|k| plaintext(c, *k).is_some()))
}

/// The plaintext values that reached the vault for one connection.
///
/// Kept so the file side clears a key only if it still holds exactly this text
/// (an edit made while the move ran wins and is moved on the next run).
pub struct MovedKeys {
    /// The connection.
    pub id: Uuid,
    pub(super) rijndael: Option<Zeroizing<String>>,
    pub(super) hmac: Option<Zeroizing<String>>,
}

impl std::fmt::Debug for MovedKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MovedKeys")
            .field("id", &self.id)
            .field("rijndael", &self.rijndael.as_ref().map(|_| "<redacted>"))
            .field("hmac", &self.hmac.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// What one pass of [`move_keys_to_vault`] did.
#[derive(Debug, Default)]
pub struct MoveReport {
    /// Keys confirmed in the vault (store plus matching read-back).
    pub moved: Vec<MovedKeys>,
    /// Connections with a key that stayed in the file, with the reason.
    /// The reason never contains a key.
    pub failed: Vec<(Uuid, String)>,
}

/// Copies every plaintext SPA key of `connections` into `store` and confirms it.
///
/// Touches nothing but the store. A key whose store or read-back fails is
/// reported in [`MoveReport::failed`] and stays plaintext in the connection, so
/// the next run retries it. Safe to repeat: storing the same value again is
/// harmless, and a connection with no plaintext is skipped.
#[must_use]
pub fn move_keys_to_vault(connections: &[Connection], store: &dyn SpaKeyStore) -> MoveReport {
    let mut report = MoveReport::default();
    for conn in connections {
        let Some(cfg) = conn.spa_config.as_ref() else {
            continue;
        };
        let mut moved = MovedKeys {
            id: conn.id,
            rijndael: None,
            hmac: None,
        };
        for kind in SpaKeyKind::ALL {
            let Some(text) = plaintext(cfg, kind) else {
                continue;
            };
            match store_and_confirm(store, &spa_secret_key(conn.id, kind), text) {
                Ok(()) => {
                    let kept = Some(Zeroizing::new(text.to_owned()));
                    match kind {
                        SpaKeyKind::Rijndael => moved.rijndael = kept,
                        SpaKeyKind::Hmac => moved.hmac = kept,
                    }
                }
                Err(reason) => report.failed.push((conn.id, reason)),
            }
        }
        if moved.rijndael.is_some() || moved.hmac.is_some() {
            report.moved.push(moved);
        }
    }
    report
}

fn store_and_confirm(store: &dyn SpaKeyStore, key: &str, text: &str) -> Result<(), String> {
    let secret = SecretString::from(text.to_owned());
    store.store(key, &secret)?;
    match store.load(key)? {
        Some(back) if back.expose_secret() == text => Ok(()),
        Some(_) => Err("the vault returned a different value than was stored".to_owned()),
        None => Err("the vault did not return the stored value".to_owned()),
    }
}

impl MovedKeys {
    /// A move record for tests of the file side.
    #[cfg(test)]
    pub(super) fn for_test(id: Uuid, rijndael: &str, hmac: &str) -> Self {
        Self {
            id,
            rijndael: Some(Zeroizing::new(rijndael.to_owned())),
            hmac: Some(Zeroizing::new(hmac.to_owned())),
        }
    }

    /// Clears the plaintext in `cfg` that this move confirmed and sets the
    /// matching flag. Returns whether anything changed.
    pub(super) fn apply_to(&self, cfg: &mut SpaConfig) -> bool {
        let rij = self
            .rijndael
            .as_ref()
            .is_some_and(|m| cfg.rijndael_key_ref.as_deref() == Some(m.as_str()));
        if rij {
            cfg.rijndael_key_ref = None;
            cfg.rijndael_key_in_vault = true;
        }
        let hmac = self
            .hmac
            .as_ref()
            .is_some_and(|m| cfg.hmac_key_ref.as_deref() == Some(m.as_str()));
        if hmac {
            cfg.hmac_key_ref = None;
            cfg.hmac_key_in_vault = true;
        }
        rij || hmac
    }

    /// Whether [`Self::apply_to`] would change `cfg`.
    pub(super) fn would_change(&self, cfg: &SpaConfig) -> bool {
        self.apply_to(&mut cfg.clone())
    }
}

/// Why the keys of a connection could not be produced.
#[derive(Debug, thiserror::Error)]
pub enum SpaKeyError {
    /// The file says the key is in the vault, but the vault has no entry.
    #[error("the {0} key is not in the secret store; open the connection and enter it again")]
    Missing(&'static str),
    /// The secret store could not be read (locked, missing, timed out).
    #[error("the secret store could not be read: {0}")]
    Unavailable(String),
}

/// The pair of keys to send an SPA packet with, from the vault or the file.
///
/// `Ok(None)` means no complete pair is configured and the packet is skipped,
/// as before. A key flagged as in the vault is **never** replaced by a
/// plaintext value; a vault that cannot be read is an error, not a silent skip.
///
/// # Errors
/// [`SpaKeyError::Missing`] when a flagged key has no vault entry,
/// [`SpaKeyError::Unavailable`] when the store fails.
pub fn resolve_spa_keys(
    connection_id: Uuid,
    cfg: &SpaConfig,
    store: &dyn SpaKeyStore,
) -> Result<Option<(SecretString, SecretString)>, SpaKeyError> {
    let one = |kind: SpaKeyKind, in_vault: bool| -> Result<Option<SecretString>, SpaKeyError> {
        if in_vault {
            return match store.load(&spa_secret_key(connection_id, kind)) {
                Ok(Some(v)) if !v.expose_secret().is_empty() => Ok(Some(v)),
                Ok(_) => Err(SpaKeyError::Missing(kind.as_str())),
                Err(e) => Err(SpaKeyError::Unavailable(e)),
            };
        }
        Ok(plaintext(cfg, kind).map(|k| SecretString::from(k.to_owned())))
    };
    let rij = one(SpaKeyKind::Rijndael, cfg.rijndael_key_in_vault)?;
    let hmac = one(SpaKeyKind::Hmac, cfg.hmac_key_in_vault)?;
    Ok(rij.zip(hmac))
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::collections::HashMap;

    use super::*;
    use crate::models::Connection;

    #[derive(Default)]
    struct MapStore {
        map: RefCell<HashMap<String, String>>,
        /// Keys whose store fails.
        fail_on: RefCell<Vec<String>>,
        locked: std::cell::Cell<bool>,
    }

    impl SpaKeyStore for MapStore {
        fn store(&self, key: &str, value: &SecretString) -> Result<(), String> {
            if self.locked.get() || self.fail_on.borrow().iter().any(|k| key.ends_with(k)) {
                return Err("secret store is locked".to_owned());
            }
            self.map
                .borrow_mut()
                .insert(key.to_owned(), value.expose_secret().to_owned());
            Ok(())
        }
        fn load(&self, key: &str) -> Result<Option<SecretString>, String> {
            if self.locked.get() {
                return Err("secret store is locked".to_owned());
            }
            Ok(self
                .map
                .borrow()
                .get(key)
                .map(|v| SecretString::from(v.clone())))
        }
        fn delete(&self, key: &str) -> Result<(), String> {
            if self.locked.get() {
                return Err("secret store is locked".to_owned());
            }
            self.map.borrow_mut().remove(key);
            Ok(())
        }
    }

    fn conn(rij: Option<&str>, hmac: Option<&str>) -> Connection {
        let mut c = Connection::new_ssh("h".into(), "h.example".into(), 22);
        let mut cfg = SpaConfig::new();
        cfg.rijndael_key_ref = rij.map(str::to_owned);
        cfg.hmac_key_ref = hmac.map(str::to_owned);
        c.spa_config = Some(cfg);
        c
    }

    #[test]
    fn moves_both_keys_and_confirms_them() {
        let store = MapStore::default();
        let c = conn(Some("rij-secret"), Some("hmac-secret"));
        let report = move_keys_to_vault(std::slice::from_ref(&c), &store);
        assert!(report.failed.is_empty());
        assert_eq!(report.moved.len(), 1);
        let map = store.map.borrow();
        assert_eq!(
            map[&spa_secret_key(c.id, SpaKeyKind::Rijndael)],
            "rij-secret"
        );
        assert_eq!(map[&spa_secret_key(c.id, SpaKeyKind::Hmac)], "hmac-secret");
    }

    #[test]
    fn no_spa_no_keys_and_empty_keys_are_skipped() {
        let store = MapStore::default();
        let mut none = conn(None, None);
        none.spa_config = None;
        let report =
            move_keys_to_vault(&[none, conn(None, None), conn(Some(""), Some(""))], &store);
        assert!(report.moved.is_empty() && report.failed.is_empty());
        assert!(store.map.borrow().is_empty());
        assert!(!has_plaintext_keys(&conn(Some(""), None)));
    }

    #[test]
    fn locked_store_keeps_the_value() {
        let store = MapStore::default();
        store.locked.set(true);
        let c = conn(Some("a"), Some("b"));
        let report = move_keys_to_vault(std::slice::from_ref(&c), &store);
        assert!(report.moved.is_empty());
        assert_eq!(report.failed.len(), 2);
        assert!(
            report
                .failed
                .iter()
                .all(|(_, why)| why == "secret store is locked")
        );
        assert!(has_plaintext_keys(&c));
    }

    #[test]
    fn partial_failure_moves_only_the_confirmed_key() {
        let store = MapStore::default();
        store.fail_on.borrow_mut().push("/hmac".to_owned());
        let mut c = conn(Some("rij"), Some("hmac"));
        let report = move_keys_to_vault(std::slice::from_ref(&c), &store);
        assert_eq!(report.failed.len(), 1);
        let cfg = c.spa_config.as_mut().unwrap();
        assert!(report.moved[0].apply_to(cfg));
        assert_eq!(cfg.rijndael_key_ref, None);
        assert!(cfg.rijndael_key_in_vault);
        assert_eq!(cfg.hmac_key_ref.as_deref(), Some("hmac"));
        assert!(!cfg.hmac_key_in_vault);
    }

    #[test]
    fn repeated_run_is_idempotent() {
        let store = MapStore::default();
        let mut c = conn(Some("rij"), Some("hmac"));
        let first = move_keys_to_vault(std::slice::from_ref(&c), &store);
        assert!(first.moved[0].apply_to(c.spa_config.as_mut().unwrap()));
        let before = store.map.borrow().clone();
        let second = move_keys_to_vault(std::slice::from_ref(&c), &store);
        assert!(second.moved.is_empty() && second.failed.is_empty());
        assert_eq!(*store.map.borrow(), before);
        // Applying the first report again changes nothing.
        assert!(!first.moved[0].apply_to(c.spa_config.as_mut().unwrap()));
    }

    #[test]
    fn edit_during_the_move_is_not_cleared() {
        let store = MapStore::default();
        let mut c = conn(Some("old"), Some("hmac"));
        let report = move_keys_to_vault(std::slice::from_ref(&c), &store);
        c.spa_config.as_mut().unwrap().rijndael_key_ref = Some("new".to_owned());
        assert!(report.moved[0].apply_to(c.spa_config.as_mut().unwrap()));
        let cfg = c.spa_config.as_ref().unwrap();
        assert_eq!(cfg.rijndael_key_ref.as_deref(), Some("new"));
        assert!(!cfg.rijndael_key_in_vault);
        assert!(cfg.hmac_key_in_vault);
    }

    #[test]
    fn round_trip_resolves_from_the_vault_after_the_move() {
        let store = MapStore::default();
        let mut c = conn(Some("rij"), Some("hmac"));
        let report = move_keys_to_vault(std::slice::from_ref(&c), &store);
        report.moved[0].apply_to(c.spa_config.as_mut().unwrap());
        let cfg = c.spa_config.as_ref().unwrap();
        assert_eq!(cfg.rijndael_key_ref, None);
        assert_eq!(cfg.hmac_key_ref, None);
        let (rij, hmac) = resolve_spa_keys(c.id, cfg, &store).unwrap().unwrap();
        assert_eq!(rij.expose_secret(), "rij");
        assert_eq!(hmac.expose_secret(), "hmac");
        // The serialized file text holds no key.
        let text = toml::to_string(cfg).unwrap();
        assert!(
            !text.contains("\"rij\"") && !text.contains("\"hmac\""),
            "{text}"
        );
        assert!(!text.contains("_key_ref"), "{text}");
    }

    #[test]
    fn resolve_reports_missing_and_locked_and_uses_plaintext_when_not_moved() {
        let store = MapStore::default();
        let mut cfg = SpaConfig::new();
        cfg.rijndael_key_in_vault = true;
        cfg.hmac_key_in_vault = true;
        let id = Uuid::new_v4();
        assert!(matches!(
            resolve_spa_keys(id, &cfg, &store),
            Err(SpaKeyError::Missing("rijndael"))
        ));
        store.locked.set(true);
        assert!(matches!(
            resolve_spa_keys(id, &cfg, &store),
            Err(SpaKeyError::Unavailable(_))
        ));
        let legacy = conn(Some("r"), Some("h"));
        let got = resolve_spa_keys(legacy.id, legacy.spa_config.as_ref().unwrap(), &store);
        assert!(got.unwrap().is_some());
        let half = conn(Some("r"), None);
        assert!(
            resolve_spa_keys(half.id, half.spa_config.as_ref().unwrap(), &store)
                .unwrap()
                .is_none()
        );
    }

    fn vaulted(rij: bool, hmac: bool) -> Connection {
        let mut c = conn(None, None);
        let cfg = c.spa_config.as_mut().unwrap();
        cfg.rijndael_key_in_vault = rij;
        cfg.hmac_key_in_vault = hmac;
        c
    }

    fn seed(store: &MapStore, id: Uuid) {
        let mut m = store.map.borrow_mut();
        m.insert(
            spa_secret_key(id, SpaKeyKind::Rijndael),
            "rij-secret".into(),
        );
        m.insert(spa_secret_key(id, SpaKeyKind::Hmac), "hmac-secret".into());
    }

    #[test]
    fn duplicate_copies_both_entries_and_the_copy_resolves() {
        let store = MapStore::default();
        let src = vaulted(true, true);
        seed(&store, src.id);
        let dup = src.duplicate_as("copy".into());
        let report = copy_spa_keys(src.id, src.spa_config.as_ref().unwrap(), dup.id, &store);
        assert_eq!(
            report.copied,
            VaultFlags {
                rijndael: true,
                hmac: true
            }
        );
        assert!(report.failed.is_empty() && report.missing.is_empty());
        let (r, h) = resolve_spa_keys(dup.id, dup.spa_config.as_ref().unwrap(), &store)
            .unwrap()
            .unwrap();
        assert_eq!(
            (r.expose_secret(), h.expose_secret()),
            ("rij-secret", "hmac-secret")
        );
        // The original keeps its own entries.
        assert_eq!(store.map.borrow().len(), 4);
    }

    #[test]
    fn repeated_duplicate_gives_each_copy_its_own_entries() {
        let store = MapStore::default();
        let src = vaulted(true, true);
        seed(&store, src.id);
        let a = src.duplicate_as("a".into());
        let b = src.duplicate_as("b".into());
        for d in [&a, &b] {
            let r = copy_spa_keys(src.id, src.spa_config.as_ref().unwrap(), d.id, &store);
            assert!(r.failed.is_empty());
        }
        assert_eq!(store.map.borrow().len(), 6);
        // Deleting one copy leaves the other and the original intact.
        assert!(remove_spa_keys(a.id, VaultFlags::of(a.spa_config.as_ref()), &store).is_empty());
        assert_eq!(store.map.borrow().len(), 4);
    }

    #[test]
    fn duplicate_without_spa_or_with_one_key_copies_only_what_is_flagged() {
        let store = MapStore::default();
        let mut none = conn(None, None);
        none.spa_config = None;
        assert!(VaultFlags::of(none.spa_config.as_ref()).is_empty());
        let plain = vaulted(false, false);
        let r = copy_spa_keys(
            plain.id,
            plain.spa_config.as_ref().unwrap(),
            Uuid::new_v4(),
            &store,
        );
        assert_eq!(r, SpaCopyReport::default());
        assert!(store.map.borrow().is_empty());

        let one = vaulted(true, false);
        seed(&store, one.id);
        let to = Uuid::new_v4();
        let r = copy_spa_keys(one.id, one.spa_config.as_ref().unwrap(), to, &store);
        assert_eq!(
            r.copied,
            VaultFlags {
                rijndael: true,
                hmac: false
            }
        );
        let map = store.map.borrow();
        assert!(map.contains_key(&spa_secret_key(to, SpaKeyKind::Rijndael)));
        assert!(!map.contains_key(&spa_secret_key(to, SpaKeyKind::Hmac)));
    }

    #[test]
    fn locked_store_copies_nothing_and_reports_the_flags_to_clear() {
        let store = MapStore::default();
        let src = vaulted(true, true);
        seed(&store, src.id);
        store.locked.set(true);
        let r = copy_spa_keys(
            src.id,
            src.spa_config.as_ref().unwrap(),
            Uuid::new_v4(),
            &store,
        );
        assert_eq!(r.copied, VaultFlags::default());
        assert_eq!(
            r.missing,
            VaultFlags {
                rijndael: true,
                hmac: true
            }
        );
        assert_eq!(r.failed, vec!["secret store is locked".to_owned()]);
    }

    #[test]
    fn copy_failure_midway_rolls_back_the_first_entry() {
        let store = MapStore::default();
        let src = vaulted(true, true);
        seed(&store, src.id);
        store.fail_on.borrow_mut().push("/hmac".to_owned());
        let to = Uuid::new_v4();
        let r = copy_spa_keys(src.id, src.spa_config.as_ref().unwrap(), to, &store);
        assert_eq!(r.copied, VaultFlags::default());
        assert_eq!(
            r.missing,
            VaultFlags {
                rijndael: true,
                hmac: true
            }
        );
        // Only the source entries remain: no half pair under the new id.
        let map = store.map.borrow();
        assert_eq!(map.len(), 2);
        assert!(map.keys().all(|k| !k.contains(&to.to_string())));
    }

    #[test]
    fn source_entry_missing_in_the_vault_is_a_failed_copy() {
        let store = MapStore::default();
        let src = vaulted(true, true);
        let r = copy_spa_keys(
            src.id,
            src.spa_config.as_ref().unwrap(),
            Uuid::new_v4(),
            &store,
        );
        assert_eq!(r.copied, VaultFlags::default());
        assert_eq!(r.failed.len(), 1);
        assert_eq!(r.failed[0], "the rijndael key is not in the secret store");
    }

    #[test]
    fn edit_that_disables_spa_orphans_both_entries() {
        let old = vaulted(true, true);
        let mut new = old.clone();
        new.spa_config = None;
        assert_eq!(
            orphaned_after_edit(&old, &new),
            VaultFlags {
                rijndael: true,
                hmac: true
            }
        );
        let store = MapStore::default();
        seed(&store, old.id);
        let errs = remove_spa_keys(old.id, orphaned_after_edit(&old, &new), &store);
        assert!(errs.is_empty() && store.map.borrow().is_empty());
    }

    #[test]
    fn edit_that_keeps_or_replaces_a_key_orphans_nothing() {
        let old = vaulted(true, true);
        assert!(orphaned_after_edit(&old, &old).is_empty());
        // One key retyped: the move overwrites the same entry.
        let mut retyped = old.clone();
        let cfg = retyped.spa_config.as_mut().unwrap();
        cfg.rijndael_key_in_vault = false;
        cfg.rijndael_key_ref = Some("new".into());
        assert!(orphaned_after_edit(&old, &retyped).is_empty());
        // No SPA before: nothing to remove.
        let mut none = conn(None, None);
        none.spa_config = None;
        assert!(orphaned_after_edit(&none, &none).is_empty());
        assert!(orphaned_after_edit(&none, &old).is_empty());
    }

    #[test]
    fn delete_of_a_flagged_connection_removes_only_flagged_entries() {
        let store = MapStore::default();
        let c = vaulted(true, false);
        seed(&store, c.id);
        let other = vaulted(true, true);
        seed(&store, other.id);
        let errs = remove_spa_keys(c.id, VaultFlags::of(c.spa_config.as_ref()), &store);
        assert!(errs.is_empty());
        let map = store.map.borrow();
        assert!(!map.contains_key(&spa_secret_key(c.id, SpaKeyKind::Rijndael)));
        assert!(map.contains_key(&spa_secret_key(c.id, SpaKeyKind::Hmac)));
        assert_eq!(map.len(), 3);
        drop(map);
        // A locked store reports, never panics, and keeps the entries.
        store.locked.set(true);
        assert_eq!(
            remove_spa_keys(other.id, VaultFlags::of(other.spa_config.as_ref()), &store).len(),
            2
        );
    }

    #[test]
    fn debug_never_prints_a_key() {
        let c = conn(Some("TOPSECRET-RIJ"), Some("TOPSECRET-HMAC"));
        let shown = format!("{:?} {:?}", c.spa_config, c);
        assert!(!shown.contains("TOPSECRET"), "{shown}");
        let store = MapStore::default();
        let report = move_keys_to_vault(std::slice::from_ref(&c), &store);
        assert!(!format!("{report:?}").contains("TOPSECRET"));
    }
}
