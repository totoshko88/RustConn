//! Session management for `RustConn`
//!
//! This module provides session lifecycle management for active connections,
//! including process handling, logging, and terminal integration.

mod logger;
mod manager;
#[allow(clippy::module_inception)]
mod session;

pub use logger::{LogConfig, LogContext, LogError, LogResult, SessionLogger};
pub use manager::SessionManager;
pub use session::{Session, SessionState, SessionType};
