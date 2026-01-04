//! Import engine for migrating connections from other tools.
//!
//! This module provides functionality to import connections from various sources:
//! - SSH config files (~/.ssh/config)
//! - Asbru-CM configuration
//! - Remmina connection files
//! - Ansible inventory files
//! - Royal TS rJSON files
//!
//! For large imports (more than 10 connections), use `BatchImporter` for
//! efficient batch processing with progress reporting and cancellation support.

mod ansible;
mod asbru;
pub mod batch;
mod rdm;
mod remmina;
mod royalts;
mod ssh_config;
mod traits;

pub use ansible::AnsibleInventoryImporter;
pub use asbru::AsbruImporter;
pub use batch::{
    BatchCancelHandle, BatchImportResult, BatchImporter, BATCH_IMPORT_THRESHOLD,
    DEFAULT_IMPORT_BATCH_SIZE,
};
pub use rdm::RdmImporter;
pub use remmina::RemminaImporter;
pub use royalts::RoyalTsImporter;
pub use ssh_config::SshConfigImporter;
pub use traits::{ImportResult, ImportSource, SkippedEntry};
