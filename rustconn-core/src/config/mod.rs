//! Configuration management for `RustConn`
//!
//! This module provides the `ConfigManager` for loading and saving
//! configuration files in TOML format.

pub mod keybindings;
mod manager;
pub mod settings;

pub use keybindings::{
    KeybindingCategory, KeybindingDef, KeybindingSettings, default_keybindings,
    is_valid_accelerator,
};
pub use manager::ConfigManager;
pub use settings::{
    AppSettings, ColorScheme, ConnectionSettings, LoggingSettings, QuickConnectHistoryItem,
    SavedSession, SecretBackendType, SecretSettings, SessionRestoreSettings, StartupAction,
    TerminalSettings, UiSettings, legacy_migration_count, reset_legacy_migration_count,
};
// MonitoringSettings is re-exported from the monitoring module, not config
