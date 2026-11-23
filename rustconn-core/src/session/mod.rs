//! Session management for `RustConn`
//!
//! This module provides session lifecycle management for active connections,
//! including process handling, logging, and terminal integration.

mod logger;
/// Session recording in `scriptreplay`-compatible format.
pub mod recording;
mod restore;
#[expect(
    clippy::module_inception,
    reason = "internal `mod foo` inside `foo.rs` keeps the file private and re-exports curated items"
)]
mod session;
/// Read-only live session sharing over a local HTTP/SSE relay.
pub mod share;
/// QUIC (UDP) transport for the same relay, with a STUN-discovered endpoint.
pub mod share_quic;

pub use logger::{
    LogConfig, LogContext, LogError, LogResult, SanitizeConfig, SessionLogger,
    contains_sensitive_prompt, prune_logs, sanitize_output, strip_ansi_escapes,
};
pub use restore::{
    PanelRestoreData, RESTORE_STATE_VERSION, SessionRestoreData, SessionRestoreError,
    SessionRestoreState, SplitLayoutRestoreData,
};
pub use session::SessionType;
