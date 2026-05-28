//! Session management for `RustConn`
//!
//! This module provides session lifecycle management for active connections,
//! including process handling, logging, and terminal integration.

mod logger;
mod manager;
/// Session recording in `scriptreplay`-compatible format.
pub mod recording;
mod restore;
#[expect(
    clippy::module_inception,
    reason = "internal `mod foo` inside `foo.rs` keeps the file private and re-exports curated items"
)]
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
