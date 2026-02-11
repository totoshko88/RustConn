//! Secret management module for `RustConn`
//!
//! This module provides secure credential storage through multiple backends:
//! - `KeePassXC` via browser integration protocol (primary)
//! - libsecret for GNOME Keyring/KDE Wallet integration (fallback)
//! - Direct KDBX file access (compatible with GNOME Secrets, `OneKeePass`, KeePass)
//! - Bitwarden CLI integration
//! - 1Password CLI integration
//!
//! The `SecretManager` provides a unified interface with automatic fallback
//! when the primary backend is unavailable.

mod async_resolver;
mod backend;
mod bitwarden;
mod detection;
pub mod hierarchy;
mod kdbx;
mod keepassxc;
pub mod keyring;
mod libsecret;
mod manager;
mod onepassword;
mod passbolt;
mod resolver;
mod status;
mod verification;

pub use async_resolver::{
    resolve_with_callback, spawn_credential_resolution, AsyncCredentialResolver,
    AsyncCredentialResult, CancellationToken, PendingCredentialResolution,
};
pub use backend::SecretBackend;
pub use bitwarden::{
    auto_unlock, configure_server, delete_api_credentials_from_keyring,
    delete_master_password_from_keyring, get_api_credentials_from_keyring, get_bitwarden_version,
    get_master_password_from_keyring, lock_vault, login_with_api_key, logout,
    store_api_credentials_in_keyring, store_master_password_in_keyring, unlock_vault,
    BitwardenBackend, BitwardenVersion,
};
pub use detection::{
    detect_bitwarden, detect_gnome_secrets, detect_keepass, detect_keepassxc, detect_libsecret,
    detect_onepassword, detect_passbolt, detect_password_managers,
    get_password_manager_launch_command, open_password_manager, PasswordManagerInfo,
};
pub use hierarchy::{
    GroupCreationResult, KeePassHierarchy, GROUPS_SUBFOLDER, KEEPASS_ROOT_GROUP, PATH_SEPARATOR,
};
pub use kdbx::KdbxExporter;
pub use keepassxc::{
    delete_kdbx_password_from_keyring, get_kdbx_password_from_keyring,
    store_kdbx_password_in_keyring, KeePassXcBackend,
};
pub use libsecret::LibSecretBackend;
pub use manager::{BulkOperationResult, CredentialUpdate, SecretManager};
pub use onepassword::{
    delete_token_from_keyring, get_onepassword_status, get_onepassword_version,
    get_token_from_keyring, signout as onepassword_signout, store_token_in_keyring,
    OnePasswordBackend, OnePasswordStatus, OnePasswordVersion,
};
pub use passbolt::{
    delete_passphrase_from_keyring, get_passbolt_status, get_passbolt_version,
    get_passphrase_from_keyring, store_passphrase_in_keyring, PassboltBackend, PassboltStatus,
    PassboltVersion,
};
pub use resolver::CredentialResolver;
pub use status::{parse_keepassxc_version, KeePassStatus};
pub use verification::{
    CredentialStatus, CredentialVerificationManager, DialogPreFillData, VerifiedCredentials,
};
