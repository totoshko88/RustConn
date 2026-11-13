//! Connection management module
//!
//! This module provides the `ConnectionManager` for CRUD operations on connections
//! and groups, with persistence through `ConfigManager`. It also provides
//! `LazyGroupLoader` for lazy loading of connection groups to improve startup
//! performance with large connection databases.
//!
//! The module also includes virtual scrolling helpers for efficient rendering of
//! large connection lists.
//!
//! ## Retry Logic
//!
//! The `retry` submodule provides `RetryConfig` and `RetryState` for handling
//! transient connection failures with exponential backoff.

pub mod automation_inheritance;
pub mod copy_field;
mod dial;
pub mod gateway_failover;
pub mod group_copy;
pub mod host_range;
pub mod jump_chain;
pub mod keepalive;
pub mod knock;
mod lazy_loader;
mod login_prompt;
mod manager;
pub mod mdns;
pub mod mptcp;
mod port_check;
mod retry;
pub mod routing_memory;
pub mod spa;
pub mod ssh_inheritance;
mod ssh_prompt;
mod virtual_scroll;

pub use copy_field::{CopyField, copy_fields, copy_text, format_address};
pub use dial::connect_tcp;
pub use jump_chain::{JumpChain, resolve_jump_chain, resolve_proxy_jump_value};
pub use keepalive::{KeepaliveConfig, apply_interactive_keepalive, set_interactive_keepalive};
pub use knock::{
    Knock, KnockError, KnockProtocol, KnockResult, KnockSequence, SpaAllowIp, SpaConfig,
    execute_knock_sequence,
};
pub use lazy_loader::LazyGroupLoader;
pub use login_prompt::{
    DEFAULT_USERNAME_PROMPTS, LoginPrompt, LoginPromptMatcher, looks_like_username_prompt,
};
pub use manager::{ConnectionManager, ReloadOutcome};
pub use mdns::{apply_mdns_fallback, is_mdns_name, resolve_sandboxed_hostname};
pub use mptcp::{
    MptcpError, MptcpResult, connect_mptcp, connect_mptcp_async, is_mptcp_available,
    is_mptcpize_available,
};
pub use port_check::{
    PortCheckError, PortCheckResult, check_port, check_port_async, check_ssh_banner,
};
pub use retry::{DEFAULT_BACKOFF_MULTIPLIER, RetryConfig, RetryState};
pub use routing_memory::{Route, RouteVerdict, RoutingMemory, routing_memory_path};
pub use spa::{SpaError, SpaResult, build_spa_packet, send_spa};
pub use ssh_prompt::looks_like_password_prompt;
pub use virtual_scroll::SelectionState;
