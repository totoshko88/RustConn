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
//! - `zerotrust` - Zero Trust provider options
//!
//! Updated for GTK 4.10+ compatibility using `DropDown` instead of `ComboBoxText`
//! and Window instead of Dialog.

mod dialog;
mod rdp;
mod spice;
mod ssh;
mod vnc;

// Re-export types from parent module for use in submodules
pub use super::{ConnectionCallback, ConnectionDialogResult};

// Re-export the main dialog
pub use dialog::ConnectionDialog;
