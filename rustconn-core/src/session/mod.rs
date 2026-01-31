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
    contains_sensitive_prompt, sanitize_output, LogConfig, LogContext, LogError, LogResult,
    SanitizeConfig, SessionLogger,
};
pub use manager::{
    HealthCheckConfig, HealthCheckEvent, HealthStatus, SessionManager, SessionResult,
    DEFAULT_HEALTH_CHECK_INTERVAL_SECS,
};
pub use restore::{
    PanelRestoreData, SessionRestoreData, SessionRestoreError, SessionRestoreState,
    SplitLayoutRestoreData, RESTORE_STATE_VERSION,
};
pub use session::{Session, SessionState, SessionType};
