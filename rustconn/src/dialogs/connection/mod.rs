//! Connection dialog for creating and editing connections
//!
//! This module provides a GTK4 dialog with protocol-specific fields, input validation,
//! and portal integration for file selection (SSH keys).
//!
//! The dialog is split into submodules for maintainability:
//! - `ssh` - SSH protocol options
//! - `rdp` - RDP protocol options
//! - `vnc` - VNC protocol options
//! - `spice` - SPICE protocol options
//! - `shared_folders` - Shared folders UI (used by RDP and SPICE)
//! - `protocol_layout` - Common layout builder for protocol options
//! - `zerotrust` - Zero Trust provider options
//!
//! Updated for GTK 4.10+ compatibility using `DropDown` instead of `ComboBoxText`
//! and Window instead of Dialog.

mod dialog;
mod logging_tab;
mod protocol_layout;
mod rdp;
mod shared_folders;
mod spice;
mod ssh;
mod telnet;
mod vnc;
pub mod widgets;
pub mod zerotrust;

// Re-export types from parent module for use in submodules
pub use super::{ConnectionCallback, ConnectionDialogResult};

// Re-export the main dialog
pub use dialog::ConnectionDialog;
