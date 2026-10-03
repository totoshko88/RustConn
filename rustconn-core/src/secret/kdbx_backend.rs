//! KDBX (`KeePass`) [`SecretBackend`] wrapper.
//!
//! `KeePass`/kdbx access historically lived as a set of associated functions on
//! [`KeePassStatus`] (in `status.rs`): the writers `save_password_to_kdbx`,
//! `delete_entry_from_kdbx`, `rename_entry_in_kdbx`, and the readers
//! `get_password_from_kdbx*`. Unlike every other secret store, kdbx was never a
//! [`SecretBackend`] — it was invoked directly by the credential resolver.
//!
//! [`KdbxBackend`] is a **thin wrapper** that makes kdbx a first-class
//! [`SecretBackend`] by *delegating* to those existing [`KeePassStatus`]
//! functions. It introduces **zero behaviour change**:
//!
//! * `store`   → [`KeePassStatus::save_password_to_kdbx`]
//! * `retrieve` → [`KeePassStatus::get_password_from_kdbx_with_key`]
//! * `delete`  → [`KeePassStatus::delete_entry_from_kdbx`]
//!
//! Because it delegates, it reuses the exact `RustConn/`-prefix and hierarchy
//! handling those functions already perform — in particular the issue #327
//! doubled-prefix fix in [`super::hierarchy`]. This wrapper never reimplements
//! prefixing.
//!
//! This struct is **additive**: as of this step nothing in the application is
//! rewired to go through it. The 47 existing call sites keep calling
//! `KeePassStatus::*` directly. Routing credential resolution through this
//! backend is a later step.
//!
//! ## Read-only / root-search capabilities
//!
//! [`Self::is_read_only`] returns the stored read-only flag and read-only is
//! now **enforced**: `store` and `delete` call [`SecretBackend::ensure_writable`]
//! as their first action, so a backend put into read-only mode via
//! [`Self::with_read_only`] refuses every mutation with [`SecretError::ReadOnly`] before touching the vault
//! (mirroring `PassBackend`). The guard runs ahead of the delegated writer's
//! own path validation, so an invalid path still surfaces as `ReadOnly`.
//!
//! [`Self::searches_from_root`] still returns the trait default (`false`);
//! vault-root search reads are a separate, later addition.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use secrecy::SecretString;

use super::backend::SecretBackend;
use super::status::KeePassStatus;
use crate::error::SecretResult;
use crate::models::Credentials;

/// The unlock factors a kdbx database needs, mirroring the parameters the
/// existing [`KeePassStatus`] readers and writers already take.
///
/// Grouped into one struct so [`KdbxBackend`] can hold them as a unit and hand
/// them to the delegated functions unchanged.
#[derive(Clone, Default)]
struct UnlockFactors {
    /// Master password, if the database uses one.
    db_password: Option<SecretString>,
    /// Key file, if the database uses one.
    key_file: Option<PathBuf>,
    /// `YubiKey` Challenge-Response slot as `slot[:serial]`, if configured.
    yubikey_slot: Option<String>,
}

/// A [`SecretBackend`] over a `KeePass`/kdbx database file.
///
/// Thin wrapper that delegates every operation to the pre-existing
/// [`KeePassStatus`] associated functions; see the module docs for the mapping
/// and for the step-2 deferral of read-only enforcement.
pub struct KdbxBackend {
    /// Path to the `.kdbx` database file.
    kdbx_path: PathBuf,
    /// How to unlock the database (password / key file / `YubiKey`).
    unlock: UnlockFactors,
    /// When true, [`Self::is_read_only`] reports read-only and `store`/`delete`
    /// refuse with [`SecretError::ReadOnly`] via `ensure_writable()`.
    read_only: bool,
}

impl KdbxBackend {
    /// Creates a new kdbx backend for the database at `kdbx_path`, with no
    /// unlock factors set yet. Chain the `with_*` builders to supply them.
    #[must_use]
    pub fn new(kdbx_path: impl Into<PathBuf>) -> Self {
        Self {
            kdbx_path: kdbx_path.into(),
            unlock: UnlockFactors::default(),
            read_only: false,
        }
    }

    /// Sets the master password used to unlock the database.
    #[must_use]
    pub fn with_db_password(mut self, db_password: SecretString) -> Self {
        self.unlock.db_password = Some(db_password);
        self
    }

    /// Sets the key file used to unlock the database.
    #[must_use]
    pub fn with_key_file(mut self, key_file: impl Into<PathBuf>) -> Self {
        self.unlock.key_file = Some(key_file.into());
        self
    }

    /// Sets the `YubiKey` Challenge-Response slot (`slot[:serial]`).
    #[must_use]
    pub fn with_yubikey_slot(mut self, yubikey_slot: impl Into<String>) -> Self {
        self.unlock.yubikey_slot = Some(yubikey_slot.into());
        self
    }

    /// Records whether the backend is read-only.
    ///
    /// When `true`, `store` and `delete` are refused with
    /// [`SecretError::ReadOnly`] (via `ensure_writable()`) before the vault is
    /// touched, matching the `with_read_only` shape of the other backends (e.g.
    /// `PassBackend`).
    #[must_use]
    pub const fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }

    /// Borrows the configured kdbx path.
    #[must_use]
    pub fn kdbx_path(&self) -> &Path {
        &self.kdbx_path
    }

    /// Convenience accessors for the delegated calls, keeping the `Option<&T>`
    /// shapes the [`KeePassStatus`] functions expect.
    fn db_password(&self) -> Option<&SecretString> {
        self.unlock.db_password.as_ref()
    }

    fn key_file(&self) -> Option<&Path> {
        self.unlock.key_file.as_deref()
    }

    fn yubikey_slot(&self) -> Option<&str> {
        self.unlock.yubikey_slot.as_deref()
    }
}

#[async_trait]
impl SecretBackend for KdbxBackend {
    /// Stores credentials by delegating to
    /// [`KeePassStatus::save_password_to_kdbx`].
    ///
    /// The connection id is used as the entry name exactly as the existing call
    /// sites pass it; `save_password_to_kdbx` applies the `RustConn/` prefixing
    /// internally, so no prefixing is reimplemented here.
    async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
        // Read-only enforcement: refuse before any kdbx side effect, mirroring
        // `PassBackend::store`. The guard runs ahead of the delegated writer's
        // own path validation, so a read-only backend returns `ReadOnly` even
        // for an invalid path.
        self.ensure_writable()?;
        let username = credentials.username.as_deref().unwrap_or("");
        // Delegate the password write. A `None` password still records the
        // entry/username, matching how the entry would otherwise be created.
        let password = credentials
            .password
            .clone()
            .unwrap_or_else(|| SecretString::from(String::new()));
        KeePassStatus::save_password_to_kdbx(
            &self.kdbx_path,
            self.db_password(),
            self.key_file(),
            connection_id,
            username,
            &password,
            None,
            self.yubikey_slot(),
        )
    }

    /// Retrieves credentials by delegating to
    /// [`KeePassStatus::get_password_from_kdbx_with_key`], which performs the
    /// `RustConn/`-prefixed candidate-path lookup (the issue #327 logic).
    async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>> {
        let password = KeePassStatus::get_password_from_kdbx_with_key(
            &self.kdbx_path,
            self.db_password(),
            self.key_file(),
            connection_id,
            None,
            self.yubikey_slot(),
        )?;

        Ok(password.map(|secret| Credentials {
            username: None,
            password: Some(secret),
            key_passphrase: None,
            domain: None,
        }))
    }

    /// Deletes an entry by delegating to
    /// [`KeePassStatus::delete_entry_from_kdbx`].
    ///
    /// The delete target is the full entry path `RustConn/<connection_id>`,
    /// matching the path `save_password_to_kdbx` writes to.
    async fn delete(&self, connection_id: &str) -> SecretResult<()> {
        // Read-only enforcement: refuse before computing the entry path or
        // delegating, so a read-only backend never touches the vault.
        self.ensure_writable()?;
        let entry_path = format!(
            "{}{}{}",
            super::hierarchy::KEEPASS_ROOT_GROUP,
            super::hierarchy::PATH_SEPARATOR,
            connection_id
        );
        KeePassStatus::delete_entry_from_kdbx(
            &self.kdbx_path,
            self.db_password(),
            self.key_file(),
            &entry_path,
            self.yubikey_slot(),
        )
    }

    async fn is_available(&self) -> bool {
        // The kdbx backend is usable when the database file validates and the
        // keepassxc-cli is present — exactly what the delegated functions
        // require before doing anything. `validate_kdbx_path` is the same check
        // every writer/reader runs first.
        KeePassStatus::validate_kdbx_path(&self.kdbx_path).is_ok()
            && KeePassStatus::detect().keepassxc_installed
    }

    fn backend_id(&self) -> &'static str {
        "kdbx"
    }

    fn display_name(&self) -> &'static str {
        "KeePass (KDBX file)"
    }

    fn is_read_only(&self) -> bool {
        // Reports the stored flag; `store`/`delete` enforce it via
        // `ensure_writable()`.
        self.read_only
    }

    // `searches_from_root()` deliberately keeps the trait default (`false`).
    // Root-search reads are step 2.
}

impl std::fmt::Debug for KdbxBackend {
    /// Never renders unlock secrets; shows only their presence.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KdbxBackend")
            .field("kdbx_path", &self.kdbx_path)
            .field("has_db_password", &self.unlock.db_password.is_some())
            .field("has_key_file", &self.unlock.key_file.is_some())
            .field("has_yubikey_slot", &self.unlock.yubikey_slot.is_some())
            .field("read_only", &self.read_only)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::SecretError;

    #[test]
    fn backend_identity_and_display_name() {
        let backend = KdbxBackend::new("/tmp/vault.kdbx");
        assert_eq!(backend.backend_id(), "kdbx");
        assert_eq!(backend.display_name(), "KeePass (KDBX file)");
        assert_eq!(backend.kdbx_path(), Path::new("/tmp/vault.kdbx"));
    }

    #[test]
    fn capabilities_default_to_false() {
        // Mirrors the trait defaults: writable, scoped (not root-search).
        let backend = KdbxBackend::new("/tmp/vault.kdbx");
        assert!(!backend.is_read_only());
        assert!(!backend.searches_from_root());
    }

    #[test]
    fn with_read_only_stores_the_flag() {
        // The flag is recorded and reported by is_read_only(); enforcement in
        // store/delete is covered by the read_only_* tests below.
        assert!(!KdbxBackend::new("/tmp/vault.kdbx").is_read_only());
        assert!(
            KdbxBackend::new("/tmp/vault.kdbx")
                .with_read_only(true)
                .is_read_only()
        );
        // Setting it back to false is honoured too.
        assert!(
            !KdbxBackend::new("/tmp/vault.kdbx")
                .with_read_only(true)
                .with_read_only(false)
                .is_read_only()
        );
    }

    #[test]
    fn builder_records_unlock_factors_without_leaking_them() {
        let backend = KdbxBackend::new("/tmp/vault.kdbx")
            .with_db_password(SecretString::from("hunter2".to_string()))
            .with_key_file("/tmp/vault.key")
            .with_yubikey_slot("2:12345678");

        // The delegated-call accessors see what the builder stored.
        assert!(backend.db_password().is_some());
        assert_eq!(backend.key_file(), Some(Path::new("/tmp/vault.key")));
        assert_eq!(backend.yubikey_slot(), Some("2:12345678"));

        // Debug must not render the secret password material.
        let rendered = format!("{backend:?}");
        assert!(rendered.contains("KdbxBackend"));
        assert!(rendered.contains("has_db_password: true"));
        assert!(!rendered.contains("hunter2"));
    }

    /// Delegation smoke test: against a path that is not a real kdbx file,
    /// `retrieve` must surface the delegated validator's error rather than
    /// succeeding or panicking — proving the call is actually wired through
    /// [`KeePassStatus`] and reuses its path validation (no reimplementation).
    #[tokio::test]
    async fn retrieve_delegates_through_keepass_status_validation() {
        let backend = KdbxBackend::new("/nonexistent/not-a-vault.txt")
            .with_db_password(SecretString::from("x".to_string()));
        // Non-`.kdbx` extension is rejected by KeePassStatus::validate_kdbx_path,
        // which the delegated reader calls first. If this wrapper reimplemented
        // anything, this error would not be the delegated one.
        let err = backend.retrieve("conn-1").await.unwrap_err();
        assert!(
            matches!(err, crate::error::SecretError::KeePassXC(ref m) if m.contains(".kdbx")),
            "expected delegated KeePassXC .kdbx validation error, got {err:?}"
        );
    }

    /// Read-only enforcement: `store` must return [`SecretError::ReadOnly`]
    /// naming this backend, and must do so BEFORE the delegated writer's path
    /// validation — proving the `ensure_writable()` guard is the first action.
    /// `/nonexistent/x.kdbx` would otherwise yield a `KeePassXC` validation
    /// error; getting `ReadOnly` instead proves the guard short-circuits first.
    #[tokio::test]
    async fn read_only_refuses_store_before_delegating() {
        let backend = KdbxBackend::new("/nonexistent/x.kdbx").with_read_only(true);
        let err = backend
            .store("conn-1", &Credentials::default())
            .await
            .unwrap_err();
        assert!(
            matches!(err, SecretError::ReadOnly(ref name) if name == "KeePass (KDBX file)"),
            "expected ReadOnly(KeePass (KDBX file)), got {err:?}"
        );
    }

    /// Read-only enforcement: `delete` must likewise return
    /// [`SecretError::ReadOnly`] before touching the vault.
    #[tokio::test]
    async fn read_only_refuses_delete_before_delegating() {
        let backend = KdbxBackend::new("/nonexistent/x.kdbx").with_read_only(true);
        let err = backend.delete("conn-1").await.unwrap_err();
        assert!(
            matches!(err, SecretError::ReadOnly(ref name) if name == "KeePass (KDBX file)"),
            "expected ReadOnly(KeePass (KDBX file)), got {err:?}"
        );
    }
}
