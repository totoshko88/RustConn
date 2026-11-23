//! Session data structures
//!
//! This module defines the types shared by session restore and workspaces.

use serde::{Deserialize, Serialize};

/// Type of session based on protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionType {
    /// SSH session displayed in embedded terminal
    Embedded,
    /// RDP/VNC session running in external window
    External,
}
