//! Secret management module for `RustConn`
//!
//! This module provides secure credential storage through multiple backends:
//! - `KeePassXC` via browser integration protocol (primary)
//! - libsecret for GNOME Keyring/KDE Wallet integration (fallback)
//! - Direct KDBX file access
//!
//! The `SecretManager` provides a unified interface with automatic fallback
//! when the primary backend is unavailable.

mod async_resolver;
mod backend;
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
pub use hierarchy::{GroupCreationResult, KeePassHierarchy, KEEPASS_ROOT_GROUP, PATH_SEPARATOR};
pub use kdbx::KdbxExporter;
pub use keepassxc::KeePassXcBackend;
pub use libsecret::LibSecretBackend;
pub use manager::SecretManager;
pub use resolver::CredentialResolver;
pub use status::{parse_keepassxc_version, KeePassStatus};
pub use verification::{
    CredentialStatus, CredentialVerificationManager, DialogPreFillData, VerifiedCredentials,
};
