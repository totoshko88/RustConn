//! Session management for `RustConn`
//!
//! This module provides session lifecycle management for active connections,
//! including process handling, logging, and terminal integration.

mod logger;
mod manager;
mod restore;
#[allow(clippy::module_inception)]
mod session;

pub use logger::{
    LogConfig, LogContext, LogError, LogResult, SanitizeConfig, SessionLogger,
    contains_sensitive_prompt, sanitize_output,
};
pub use manager::{
    DEFAULT_HEALTH_CHECK_INTERVAL_SECS, HealthCheckConfig, HealthCheckEvent, HealthStatus,
    SessionManager,
};
pub use restore::{
    PanelRestoreData, RESTORE_STATE_VERSION, SessionRestoreData, SessionRestoreError,
    SessionRestoreState, SplitLayoutRestoreData,
};
pub use session::{Session, SessionState, SessionType};
