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
mod pass;
mod passbolt;
mod resolver;
mod status;
mod verification;

pub use async_resolver::{
    AsyncCredentialResolver, AsyncCredentialResult, CancellationToken, PendingCredentialResolution,
    resolve_with_callback, spawn_credential_resolution,
};
pub use backend::SecretBackend;
pub use bitwarden::{
    BitwardenBackend, BitwardenVersion, auto_unlock, clear_session_key, configure_server,
    delete_api_credentials_from_keyring, delete_master_password_from_keyring,
    get_api_credentials_from_keyring, get_bitwarden_version, get_bw_cmd,
    get_master_password_from_keyring, get_session_key, lock_vault, login_with_api_key, logout,
    resolve_bw_cmd, set_bw_cmd, set_session_key, store_api_credentials_in_keyring,
    store_master_password_in_keyring, unlock_vault,
};
pub use detection::{
    PasswordManagerInfo, detect_bitwarden, detect_gnome_secrets, detect_keepass, detect_keepassxc,
    detect_libsecret, detect_onepassword, detect_pass, detect_passbolt, detect_password_managers,
    get_password_manager_launch_command, open_password_manager,
};
pub use hierarchy::{
    GROUPS_SUBFOLDER, GroupCreationResult, KEEPASS_ROOT_GROUP, KeePassHierarchy, PATH_SEPARATOR,
};
pub use kdbx::KdbxExporter;
pub use keepassxc::{
    KeePassXcBackend, delete_kdbx_password_from_keyring, get_kdbx_password_from_keyring,
    store_kdbx_password_in_keyring,
};
pub use libsecret::LibSecretBackend;
pub use manager::{BulkOperationResult, CACHE_TTL_SECONDS, CredentialUpdate, SecretManager};
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
