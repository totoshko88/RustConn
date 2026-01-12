//! Configuration management for `RustConn`
//!
//! This module provides the `ConfigManager` for loading and saving
//! configuration files in TOML format.

mod manager;
pub mod settings;

pub use manager::ConfigManager;
pub use settings::{
    AppSettings, ColorScheme, ConnectionSettings, LoggingSettings, SavedSession, SecretBackendType,
    SecretSettings, SessionRestoreSettings, TerminalSettings, UiSettings,
};
