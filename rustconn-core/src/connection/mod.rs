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

mod interning;
mod lazy_loader;
mod manager;
mod port_check;
mod virtual_scroll;

pub use interning::{
    check_interning_stats, get_interning_stats, intern_connection_strings, intern_hostname,
    intern_protocol_name, intern_username, log_interning_stats, log_interning_stats_with_warning,
};
pub use lazy_loader::LazyGroupLoader;
pub use manager::ConnectionManager;
pub use port_check::{check_port, check_port_async, PortCheckError, PortCheckResult};
pub use virtual_scroll::{SelectionState, VirtualScrollConfig};
