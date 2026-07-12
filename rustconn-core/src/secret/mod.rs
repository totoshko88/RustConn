//! Secret management module for `RustConn`
//!
//! This module provides secure credential storage through multiple backends:
//! - Direct KDBX file access (compatible with `KeePassXC`, GNOME Secrets, `OneKeePass`)
//! - libsecret / macOS Keychain integration when the `system-keyring` feature is enabled
//! - Bitwarden CLI integration
//! - 1Password CLI integration
//!
//! The `SecretManager` provides a unified interface with automatic fallback
//! when the primary backend is unavailable.

mod async_resolver;
mod backend;
mod bitwarden;
mod detection;
mod encrypted_file;
pub mod hierarchy;
mod kdbx;
mod kdbx_keyring;
pub mod keyring;
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
mod libsecret;
pub(crate) mod local_crypto;
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
pub mod macos_keychain;
mod manager;
mod onepassword;
mod pass;
mod passbolt;
mod resolver;
pub mod script_resolver;
pub mod serde_helpers;
mod status;
mod verification;

pub use async_resolver::{
    AsyncCredentialResolver, AsyncCredentialResult, CancellationToken, PendingCredentialResolution,
    resolve_with_callback, spawn_credential_resolution,
};
pub use backend::{BackendAvailability, SecretBackend};
pub use bitwarden::{
    BitwardenBackend, BitwardenVersion, auto_unlock, clear_session_key, configure_server,
    delete_api_credentials_from_keyring, delete_master_password_from_keyring,
    get_api_credentials_from_keyring, get_bitwarden_version, get_bw_cmd,
    get_master_password_from_keyring, get_session_key, lock_vault, login_with_api_key, logout,
    resolve_bw_cmd, set_bw_cmd, set_session_key, store_api_credentials_in_keyring,
    store_master_password_in_keyring, unlock_vault,
};
pub use detection::{
    PasswordManagerInfo, VERSION_REGEX, detect_bitwarden, detect_gnome_secrets, detect_keepass,
    detect_keepassxc, detect_libsecret, detect_onepassword, detect_pass, detect_passbolt,
    detect_password_managers, get_password_manager_launch_command, open_password_manager,
    url_open_command,
};
pub use encrypted_file::EncryptedFileBackend;
pub use hierarchy::{
    GROUPS_SUBFOLDER, GroupCreationResult, KEEPASS_ROOT_GROUP, KeePassHierarchy, PATH_SEPARATOR,
};
pub use kdbx::KdbxExporter;
pub use kdbx_keyring::{
    delete_kdbx_password_from_keyring, get_kdbx_password_from_keyring,
    store_kdbx_password_in_keyring,
};
#[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
pub use libsecret::LibSecretBackend;
#[cfg(all(feature = "system-keyring", target_os = "macos"))]
pub use macos_keychain::MacOsKeychainBackend;
pub use manager::{
    BulkOperationResult, CACHE_TTL_SECONDS, CredentialUpdate, SecretManager, StoreOutcome,
};
pub use onepassword::{
    OnePasswordBackend, OnePasswordStatus, OnePasswordVersion, delete_token_from_keyring,
    get_onepassword_status, get_onepassword_version, get_token_from_keyring,
    signout as onepassword_signout, store_token_in_keyring,
};
pub use pass::PassBackend;
pub use passbolt::{
    PassboltBackend, PassboltStatus, PassboltVersion, delete_passphrase_from_keyring,
    get_passbolt_status, get_passbolt_version, get_passphrase_from_keyring,
    store_passphrase_in_keyring,
};
pub use resolver::CredentialResolver;
pub use status::{KeePassStatus, parse_keepassxc_version};
pub use verification::{
    CredentialStatus, CredentialVerificationManager, DialogPreFillData, VerifiedCredentials,
};

/// Three-state credential storage choice for backend master credentials.
///
/// Used by Settings → Secrets to consolidate the previous "Save password" and
/// "Save to system keyring" check pair into a single canonical selector.
///
/// Persistence in [`crate::config::SecretSettings`] uses two underlying fields
/// for backward compatibility:
/// - `*_password_encrypted: Option<String>` — set when [`Self::EncryptedFile`]
/// - `*_save_to_keyring: bool`             — set when [`Self::SystemKeyring`]
///
/// The two fields are mutually exclusive; combinations are reduced through
/// [`Self::from_legacy`] so old configs continue to work unchanged.
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
pub enum CredentialStorage {
    /// The credential is not persisted between sessions. The user enters it
    /// each time the application launches.
    #[default]
    None,
    /// The credential is encrypted with a machine-specific key (AES-256-GCM)
    /// and stored in `config.toml`. Convenient but tied to one machine.
    EncryptedFile,
    /// The credential is stored in the system keyring (GNOME Keyring / KDE
    /// Wallet) via libsecret. Recommended; works across machines that share a
    /// keyring backend.
    SystemKeyring,
}

impl CredentialStorage {
    /// Reduces a legacy `(encrypted_present, save_to_keyring)` pair to a single
    /// canonical [`CredentialStorage`]. Conflicting inputs (both true) prefer
    /// the keyring choice — historically the more recent and recommended one.
    #[must_use]
    pub const fn from_legacy(encrypted_present: bool, save_to_keyring: bool) -> Self {
        if save_to_keyring {
            Self::SystemKeyring
        } else if encrypted_present {
            Self::EncryptedFile
        } else {
            Self::None
        }
    }
}
