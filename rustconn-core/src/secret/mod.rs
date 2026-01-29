//! Secret management module for `RustConn`
//!
//! This module provides secure credential storage through multiple backends:
//! - `KeePassXC` via browser integration protocol (primary)
//! - libsecret for GNOME Keyring/KDE Wallet integration (fallback)
//! - Direct KDBX file access (compatible with GNOME Secrets, `OneKeePass`, KeePass)
//! - Bitwarden CLI integration
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
mod libsecret;
mod manager;
mod resolver;
mod status;
mod verification;

pub use async_resolver::{
    resolve_with_callback, spawn_credential_resolution, AsyncCredentialResolver,
    AsyncCredentialResult, CancellationToken, PendingCredentialResolution,
};
pub use backend::SecretBackend;
pub use bitwarden::{
    get_bitwarden_version, lock_vault, unlock_vault, BitwardenBackend, BitwardenVersion,
};
pub use detection::{
    detect_bitwarden, detect_gnome_secrets, detect_keepass, detect_keepassxc, detect_libsecret,
    detect_password_managers, get_password_manager_launch_command, open_password_manager,
    PasswordManagerInfo,
};
pub use hierarchy::{
    GroupCreationResult, KeePassHierarchy, GROUPS_SUBFOLDER, KEEPASS_ROOT_GROUP, PATH_SEPARATOR,
};
pub use kdbx::KdbxExporter;
pub use keepassxc::KeePassXcBackend;
pub use libsecret::LibSecretBackend;
pub use manager::SecretManager;
pub use resolver::CredentialResolver;
pub use status::{parse_keepassxc_version, KeePassStatus};
pub use verification::{
    CredentialStatus, CredentialVerificationManager, DialogPreFillData, VerifiedCredentials,
};
