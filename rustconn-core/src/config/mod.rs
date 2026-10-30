//! Configuration management for `RustConn`
//!
//! This module provides the `ConfigManager` for loading and saving
//! configuration files in TOML format.

pub mod keybindings;
mod manager;
pub mod settings;
mod version_skew;

pub use keybindings::{
    KeybindingCategory, KeybindingDef, KeybindingSettings, MacroKeybindError, default_keybindings,
    default_passthrough_exceptions, is_valid_accelerator, validate_macro_keybind,
};
pub use manager::ConfigManager;
pub use settings::{
    AppSettings, ColorScheme, ConnectionSettings, KeyringRevocations, LoggingSettings,
    NetworkSettings, QuickConnectHistoryItem, RendererPreference, SIDEBAR_ROW_MARGIN_MAX,
    SIDEBAR_ROW_MARGIN_MIN, SIDEBAR_WIDTH_DEFAULT, SIDEBAR_WIDTH_MAX, SIDEBAR_WIDTH_MIN,
    SecretBackendType, SecretSettings, SessionRestoreSettings, SidebarAutoAction, SidebarFontSize,
    StartupAction, TerminalSettings, UiSettings, clamp_sidebar_width,
    sidebar_action_on_session_closed, sidebar_action_on_session_shown,
};
pub use version_skew::is_newer_than_running;
pub(crate) use version_skew::quarantine_file;
// MonitoringSettings is re-exported from the monitoring module, not config
