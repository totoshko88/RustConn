//! Import engine for migrating connections from other tools.
//!
//! This module provides functionality to import connections from various sources:
//! - SSH config files (~/.ssh/config)
//! - Asbru-CM configuration
//! - Remmina connection files
//! - Ansible inventory files
//! - Royal TS rJSON files
//! - MobaXterm session files
//!
//! For large imports (more than 10 connections), use `BatchImporter` for
//! efficient batch processing with progress reporting and cancellation support.
//!
//! After importing, use `ImportNormalizer` to ensure consistency:
//! - Deduplicate groups with identical names
//! - Validate SSH key paths
//! - Normalize ports and auth methods
//! - Add import metadata tags
//!
//! # Import Preview and Merge Strategies
//!
//! Use `ImportPreview` to show users what will be imported before applying:
//!
//! ```ignore
//! let result = importer.import_from_path(&path)?;
//! let preview = ImportPreview::from_result(
//!     &result,
//!     &existing_connections,
//!     &existing_groups,
//!     MergeStrategy::SkipExisting,
//!     "source_id",
//!     "path",
//! );
//!
//! // Show preview to user, let them modify actions...
//!
//! let (to_create, to_update, groups) = preview.apply();
//! ```

mod ansible;
mod asbru;
pub mod batch;
mod mobaxterm;
mod normalize;
mod preview;
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
pub use mobaxterm::MobaXtermImporter;
pub use normalize::{
    is_valid_hostname, looks_like_hostname, parse_host_port, ImportNormalizer, NormalizeOptions,
};
pub use preview::{DuplicateAction, ImportPreview, MergeStrategy, PreviewConnection, PreviewGroup};
pub use rdm::RdmImporter;
pub use remmina::RemminaImporter;
pub use royalts::RoyalTsImporter;
pub use ssh_config::SshConfigImporter;
pub use traits::{ImportResult, ImportSource, SkippedEntry};
