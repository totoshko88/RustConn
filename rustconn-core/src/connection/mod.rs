//! Connection management module
//!
//! This module provides the `ConnectionManager` for CRUD operations on connections
//! and groups, with persistence through `ConfigManager`. It also provides
//! `LazyGroupLoader` for lazy loading of connection groups to improve startup
//! performance with large connection databases.
//!
//! The module also includes string interning utilities for memory optimization
//! when dealing with large numbers of connections, and virtual scrolling helpers
//! for efficient rendering of large connection lists.
//!
//! ## Retry Logic
//!
//! The `retry` submodule provides `RetryConfig` and `RetryState` for handling
//! transient connection failures with exponential backoff.

pub mod automation_inheritance;
mod interning;
pub mod knock;
mod lazy_loader;
mod manager;
pub mod mptcp;
mod port_check;
mod retry;
pub mod spa;
pub mod ssh_inheritance;
mod ssh_prompt;
mod virtual_scroll;

pub use interning::{
    check_interning_stats, get_interning_stats, intern_connection_strings, intern_hostname,
    intern_protocol_name, intern_username, log_interning_stats, log_interning_stats_with_warning,
};
pub use knock::{
    Knock, KnockError, KnockProtocol, KnockResult, KnockSequence, SpaAllowIp, SpaConfig,
    execute_knock_sequence,
};
pub use lazy_loader::LazyGroupLoader;
pub use manager::ConnectionManager;
pub use mptcp::{MptcpError, MptcpResult, connect_mptcp, connect_mptcp_async, is_mptcp_available};
pub use port_check::{PortCheckError, PortCheckResult, check_port, check_port_async};
pub use retry::{DEFAULT_BACKOFF_MULTIPLIER, RetryConfig, RetryState};
pub use spa::{SpaError, SpaResult, build_spa_packet, send_spa};
pub use ssh_prompt::looks_like_password_prompt;
pub use virtual_scroll::SelectionState;
