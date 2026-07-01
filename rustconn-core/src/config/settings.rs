//! Application settings model
//!
//! This module defines the application-wide settings stored in config.toml.

use crate::activity_monitor::ActivityMonitorDefaults;
use crate::models::HighlightRule;
use crate::models::HistorySettings;
use crate::models::SmartFolder;
use crate::monitoring::MonitoringSettings;
use crate::secret::CredentialStorage;
use crate::sync::SyncSettings;
use crate::variables::Variable;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use zeroize::Zeroizing;

/// Application-wide settings
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppSettings {
    /// Terminal settings
    #[serde(default)]
    pub terminal: TerminalSettings,
    /// Logging settings
    #[serde(default)]
    pub logging: LoggingSettings,
    /// Secret storage settings
    #[serde(default)]
    pub secrets: SecretSettings,
    /// UI settings
    #[serde(default)]
    pub ui: UiSettings,
    /// Connection settings
    #[serde(default)]
    pub connection: ConnectionSettings,
    /// Global variables
    #[serde(default)]
    pub global_variables: Vec<Variable>,
    /// Connection history settings
    #[serde(default)]
    pub history: HistorySettings,
    /// Custom keybinding overrides
    #[serde(default)]
    pub keybindings: super::keybindings::KeybindingSettings,
    /// Remote host monitoring settings
    #[serde(default)]
    pub monitoring: MonitoringSettings,
    /// Terminal activity monitor defaults
    #[serde(default)]
    pub activity_monitor: ActivityMonitorDefaults,
    /// Global highlight rules for regex-based text highlighting
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlight_rules: Vec<HighlightRule>,
    /// Saved smart folders for dynamic connection grouping
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub smart_folders: Vec<SmartFolder>,
    /// Global custom SSH agent socket path.
    /// Overrides auto-detected SSH_AUTH_SOCK for all connections.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_agent_socket: Option<String>,
    /// Cloud Sync settings
    #[serde(default)]
    pub sync: SyncSettings,
    /// Standalone SSH tunnels (port forwarding without terminal sessions)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub standalone_tunnels: Vec<crate::models::StandaloneTunnel>,
    /// Quick Connect history (protocol/host/port/username, no secrets)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub quick_connect_history: Vec<QuickConnectHistoryItem>,
}

/// Terminal-related settings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)] // Terminal settings are independent boolean flags
pub struct TerminalSettings {
    /// Font family for terminal
    #[serde(default = "default_font_family")]
    pub font_family: String,
    /// Font size in points
    #[serde(default = "default_font_size")]
    pub font_size: u32,
    /// Scrollback buffer lines
    #[serde(default = "default_scrollback")]
    pub scrollback_lines: u32,
    /// Color theme
    #[serde(default = "default_color_theme")]
    pub color_theme: String,
    /// Cursor shape
    #[serde(default = "default_cursor_shape")]
    pub cursor_shape: String,
    /// Cursor blink mode
    #[serde(default = "default_cursor_blink")]
    pub cursor_blink: String,
    /// Scroll on output
    #[serde(default = "default_scroll_on_output")]
    pub scroll_on_output: bool,
    /// Scroll on keystroke
    #[serde(default = "default_scroll_on_keystroke")]
    pub scroll_on_keystroke: bool,
    /// Allow hyperlinks
    #[serde(default = "default_allow_hyperlinks")]
    pub allow_hyperlinks: bool,
    /// Mouse autohide
    #[serde(default = "default_mouse_autohide")]
    pub mouse_autohide: bool,
    /// Audible bell
    #[serde(default = "default_audible_bell")]
    pub audible_bell: bool,
    /// Prepend timestamps to session log lines
    #[serde(default)]
    pub log_timestamps: bool,
    /// Open SFTP via Midnight Commander in local shell
    ///
    /// Defaults to `true` in Flatpak builds (mc is bundled and avoids
    /// the sandbox/host SSH-agent mismatch with external file managers).
    #[serde(default = "default_sftp_use_mc")]
    pub sftp_use_mc: bool,
    /// Automatically copy selected text to clipboard (X11-style)
    #[serde(default)]
    pub copy_on_select: bool,
    /// Show a scrollbar next to the terminal
    #[serde(default = "default_show_scrollbar")]
    pub show_scrollbar: bool,
    /// Custom command to run in Local Shell instead of the default login shell.
    ///
    /// When set, Local Shell executes this command (e.g. `fish`, `bash --norc`,
    /// `neofetch && bash`, or any custom script) instead of `$SHELL`.
    /// Empty string means use the default shell.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub local_shell_command: String,
    /// Automatically close the tab when the session exits cleanly (exit code 0).
    ///
    /// When `true`, SSH/Telnet/Serial sessions that terminate with exit code 0
    /// (e.g. user typed `exit` or `logout`) will close the tab automatically
    /// instead of showing the reconnect overlay.
    /// Defaults to `false` (show reconnect overlay).
    #[serde(default)]
    pub close_on_clean_exit: bool,
    /// macOS only: treat Option key as Meta/Alt (send ESC prefix).
    ///
    /// When `false` (default on macOS), the Option key produces composed
    /// characters according to the active keyboard layout (e.g. Option+L → @
    /// on German keyboard). When `true`, the Option key sends ESC-prefixed
    /// escape sequences (useful for emacs/vim users).
    ///
    /// On Linux this setting is ignored — Alt always sends ESC sequences.
    #[serde(default)]
    pub option_is_meta: bool,
}

fn default_font_family() -> String {
    "Monospace".to_string()
}

const fn default_font_size() -> u32 {
    12
}

const fn default_scrollback() -> u32 {
    10000
}

fn default_color_theme() -> String {
    "Dark".to_string()
}

fn default_cursor_shape() -> String {
    "Block".to_string()
}

fn default_cursor_blink() -> String {
    "On".to_string()
}

const fn default_scroll_on_output() -> bool {
    false
}

const fn default_scroll_on_keystroke() -> bool {
    true
}

const fn default_allow_hyperlinks() -> bool {
    true
}

const fn default_mouse_autohide() -> bool {
    true
}

const fn default_audible_bell() -> bool {
    false
}

/// Returns `true` when running inside a Flatpak sandbox.
///
/// In Flatpak, external file managers (Dolphin, Nautilus) cannot access
/// the sandbox's SSH agent, so mc is a better default — it runs inside
/// the sandbox and inherits `SSH_AUTH_SOCK` directly.
fn default_sftp_use_mc() -> bool {
    crate::flatpak::is_flatpak()
}

const fn default_show_scrollbar() -> bool {
    true
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_family: default_font_family(),
            font_size: default_font_size(),
            scrollback_lines: default_scrollback(),
            color_theme: default_color_theme(),
            cursor_shape: default_cursor_shape(),
            cursor_blink: default_cursor_blink(),
            scroll_on_output: default_scroll_on_output(),
            scroll_on_keystroke: default_scroll_on_keystroke(),
            allow_hyperlinks: default_allow_hyperlinks(),
            mouse_autohide: default_mouse_autohide(),
            audible_bell: default_audible_bell(),
            log_timestamps: false,
            sftp_use_mc: default_sftp_use_mc(),
            copy_on_select: false,
            show_scrollbar: default_show_scrollbar(),
            local_shell_command: String::new(),
            close_on_clean_exit: false,
            option_is_meta: false,
        }
    }
}

/// Logging settings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)] // Logging modes are independent boolean flags
pub struct LoggingSettings {
    /// Enable session logging
    #[serde(default)]
    pub enabled: bool,
    /// Directory for log files (relative to config dir if not absolute)
    #[serde(default = "default_log_dir")]
    pub log_directory: PathBuf,
    /// Number of days to retain logs
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
    /// Log terminal activity (change counts) - default mode
    #[serde(default = "default_true")]
    pub log_activity: bool,
    /// Log user input (commands)
    #[serde(default)]
    pub log_input: bool,
    /// Log full terminal output (transcript)
    #[serde(default)]
    pub log_output: bool,
}

fn default_log_dir() -> PathBuf {
    PathBuf::from("logs")
}

const fn default_retention_days() -> u32 {
    30
}

impl Default for LoggingSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            log_directory: default_log_dir(),
            retention_days: default_retention_days(),
            log_activity: true,
            log_input: false,
            log_output: false,
        }
    }
}

/// Secret storage settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)]
pub struct SecretSettings {
    /// Preferred secret backend
    #[serde(default = "default_secret_backend")]
    pub preferred_backend: SecretBackendType,
    /// Enable fallback to libsecret if `KeePassXC` unavailable
    #[serde(default = "default_true")]
    pub enable_fallback: bool,
    /// Path to `KeePass` database file (.kdbx)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kdbx_path: Option<PathBuf>,
    /// Whether `KeePass` integration is enabled
    #[serde(default)]
    pub kdbx_enabled: bool,
    /// `KeePass` database password (NOT serialized for security - runtime only)
    #[serde(skip)]
    pub kdbx_password: Option<SecretString>,
    /// Encrypted `KeePass` password for persistence (base64 encoded)
    /// Uses machine-specific key derivation for security
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kdbx_password_encrypted: Option<String>,
    /// Path to `KeePass` key file (.keyx or .key) - alternative to password
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kdbx_key_file: Option<PathBuf>,
    /// Whether to use key file for authentication
    #[serde(default)]
    pub kdbx_use_key_file: bool,
    /// Whether to use password for authentication
    #[serde(default = "default_true")]
    pub kdbx_use_password: bool,
    /// Bitwarden master password (NOT serialized for security - runtime only)
    #[serde(skip)]
    pub bitwarden_password: Option<SecretString>,
    /// Encrypted Bitwarden master password for persistence (hex encoded)
    /// Uses machine-specific key derivation for security
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitwarden_password_encrypted: Option<String>,
    /// Whether to use API key authentication for Bitwarden
    #[serde(default)]
    pub bitwarden_use_api_key: bool,
    /// Bitwarden API client_id (NOT serialized - runtime only)
    #[serde(skip)]
    pub bitwarden_client_id: Option<SecretString>,
    /// Encrypted Bitwarden client_id for persistence
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitwarden_client_id_encrypted: Option<String>,
    /// Bitwarden API client_secret (NOT serialized - runtime only)
    #[serde(skip)]
    pub bitwarden_client_secret: Option<SecretString>,
    /// Encrypted Bitwarden client_secret for persistence
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bitwarden_client_secret_encrypted: Option<String>,
    /// Whether to save Bitwarden master password to libsecret
    #[serde(default)]
    pub bitwarden_save_to_keyring: bool,
    /// Whether to save KeePass password to system keyring (libsecret/KWallet)
    #[serde(default)]
    pub kdbx_save_to_keyring: bool,
    /// 1Password service account token (NOT serialized - runtime only)
    #[serde(skip)]
    pub onepassword_service_account_token: Option<SecretString>,
    /// Encrypted 1Password service account token for persistence
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub onepassword_service_account_token_encrypted: Option<String>,
    /// Whether to save 1Password token to system keyring
    #[serde(default)]
    pub onepassword_save_to_keyring: bool,
    /// Passbolt GPG passphrase (NOT serialized - runtime only)
    #[serde(skip)]
    pub passbolt_passphrase: Option<SecretString>,
    /// Encrypted Passbolt GPG passphrase for persistence
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passbolt_passphrase_encrypted: Option<String>,
    /// Whether to save Passbolt passphrase to system keyring
    #[serde(default)]
    pub passbolt_save_to_keyring: bool,
    /// Passbolt server URL for web vault access
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub passbolt_server_url: Option<String>,
    /// Pass password store directory (defaults to ~/.password-store)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pass_store_dir: Option<PathBuf>,
}

const fn default_true() -> bool {
    true
}

/// Default secret backend, chosen per platform.
///
/// macOS ships the system Keychain (Security.framework) and has no libsecret,
/// so a fresh install defaults to [`SecretBackendType::MacOsKeychain`] there.
/// Every other platform defaults to [`SecretBackendType::LibSecret`].
fn default_secret_backend() -> SecretBackendType {
    #[cfg(target_os = "macos")]
    {
        SecretBackendType::MacOsKeychain
    }
    #[cfg(not(target_os = "macos"))]
    {
        SecretBackendType::LibSecret
    }
}

impl Default for SecretSettings {
    fn default() -> Self {
        Self {
            preferred_backend: default_secret_backend(),
            enable_fallback: true,
            kdbx_path: None,
            kdbx_enabled: false,
            kdbx_password: None,
            kdbx_password_encrypted: None,
            kdbx_key_file: None,
            kdbx_use_key_file: false,
            kdbx_use_password: true,
            bitwarden_password: None,
            bitwarden_password_encrypted: None,
            bitwarden_use_api_key: false,
            bitwarden_client_id: None,
            bitwarden_client_id_encrypted: None,
            bitwarden_client_secret: None,
            bitwarden_client_secret_encrypted: None,
            bitwarden_save_to_keyring: false,
            kdbx_save_to_keyring: false,
            onepassword_service_account_token: None,
            onepassword_service_account_token_encrypted: None,
            onepassword_save_to_keyring: false,
            passbolt_passphrase: None,
            passbolt_passphrase_encrypted: None,
            passbolt_save_to_keyring: false,
            passbolt_server_url: None,
            pass_store_dir: None,
        }
    }
}

impl PartialEq for SecretSettings {
    fn eq(&self, other: &Self) -> bool {
        self.preferred_backend == other.preferred_backend
            && self.enable_fallback == other.enable_fallback
            && self.kdbx_path == other.kdbx_path
            && self.kdbx_enabled == other.kdbx_enabled
            && self.kdbx_key_file == other.kdbx_key_file
            && self.kdbx_use_key_file == other.kdbx_use_key_file
            && self.kdbx_use_password == other.kdbx_use_password
            && self.kdbx_password_encrypted == other.kdbx_password_encrypted
            && self.kdbx_save_to_keyring == other.kdbx_save_to_keyring
            && self.bitwarden_password_encrypted == other.bitwarden_password_encrypted
            && self.bitwarden_use_api_key == other.bitwarden_use_api_key
            && self.bitwarden_client_id_encrypted == other.bitwarden_client_id_encrypted
            && self.bitwarden_client_secret_encrypted == other.bitwarden_client_secret_encrypted
            && self.bitwarden_save_to_keyring == other.bitwarden_save_to_keyring
            && self.onepassword_service_account_token_encrypted
                == other.onepassword_service_account_token_encrypted
            && self.onepassword_save_to_keyring == other.onepassword_save_to_keyring
            && self.passbolt_passphrase_encrypted == other.passbolt_passphrase_encrypted
            && self.passbolt_save_to_keyring == other.passbolt_save_to_keyring
            && self.passbolt_server_url == other.passbolt_server_url
            && self.pass_store_dir == other.pass_store_dir
        // Note: runtime-only SecretString fields (kdbx_password, bitwarden_password,
        // bitwarden_client_id, bitwarden_client_secret, onepassword_service_account_token,
        // passbolt_passphrase) are intentionally excluded — they are #[serde(skip)]
        // and not persisted, so they shouldn't affect settings equality.
    }
}

impl Eq for SecretSettings {}

/// Secret backend type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretBackendType {
    /// `KeePassXC` browser integration
    KeePassXc,
    /// Direct KDBX file access (GNOME Secrets, `OneKeePass`, KeePass compatible)
    KdbxFile,
    /// libsecret (GNOME Keyring/KDE Wallet)
    #[default]
    LibSecret,
    /// Bitwarden CLI
    Bitwarden,
    /// 1Password CLI
    OnePassword,
    /// Passbolt CLI
    Passbolt,
    /// Pass (Unix Password Manager)
    Pass,
    /// macOS Keychain (Security.framework)
    MacOsKeychain,
    /// Application-managed encrypted file (no system keyring required).
    ///
    /// Per-entry AES-256-GCM blobs stored under the user data dir; serialized
    /// as `"encrypted_file"` via `#[serde(rename_all = "snake_case")]`. Kept
    /// last so existing configs round-trip unchanged.
    EncryptedFile,
}

/// Color scheme preference
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorScheme {
    /// Follow system preference
    #[default]
    System,
    /// Force light theme
    Light,
    /// Force dark theme
    Dark,
}

/// Action to perform when the application starts
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StartupAction {
    /// Do nothing (show empty session area)
    #[default]
    None,
    /// Open a local shell terminal
    LocalShell,
    /// Connect to a specific saved connection by UUID
    Connection(uuid::Uuid),
    /// Open and connect from an `.rdp` file
    RdpFile(std::path::PathBuf),
    /// Open and connect from a virt-viewer `.vv` file (SPICE/VNC)
    VvFile(std::path::PathBuf),
}

/// Maximum number of search history entries to persist
const MAX_SEARCH_HISTORY_ENTRIES: usize = 20;

/// UI settings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)]
pub struct UiSettings {
    /// Color scheme preference
    #[serde(default)]
    pub color_scheme: ColorScheme,
    /// Language override (locale code like "uk", "de", "fr", or "system" for auto-detect)
    #[serde(default = "default_language")]
    pub language: String,
    /// Remember window geometry
    #[serde(default = "default_true")]
    pub remember_window_geometry: bool,
    /// Window width
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_width: Option<i32>,
    /// Window height
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_height: Option<i32>,
    /// Whether the window was maximized at last close (restored on startup, #202)
    #[serde(default)]
    pub window_maximized: bool,
    /// Sidebar width
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidebar_width: Option<i32>,
    /// Enable tray icon
    #[serde(default = "default_true")]
    pub enable_tray_icon: bool,
    /// Minimize to tray instead of quitting when closing window
    #[serde(default)]
    pub minimize_to_tray: bool,
    /// IDs of groups that are expanded in the sidebar (for state persistence)
    #[serde(default, skip_serializing_if = "std::collections::HashSet::is_empty")]
    pub expanded_groups: std::collections::HashSet<uuid::Uuid>,
    /// Session restore settings
    #[serde(default)]
    pub session_restore: SessionRestoreSettings,
    /// Search history for sidebar (persisted across sessions)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub search_history: Vec<String>,
    /// Action to perform on application startup
    #[serde(default)]
    pub startup_action: StartupAction,
    /// Color tab indicators by protocol type
    #[serde(default)]
    pub color_tabs_by_protocol: bool,
    /// Show protocol filter bar in sidebar
    #[serde(default)]
    pub show_protocol_filters: bool,
    /// Show Smart Folders section in sidebar
    #[serde(default)]
    pub show_smart_folders: bool,
    /// Compact interface — denser header bar, tabs and buttons
    ///
    /// Reduces vertical chrome (header bar, tab bar, button padding) so more
    /// space is available for the active session content. Especially useful on
    /// small laptop screens (≤14") and on KDE Plasma where the default Adwaita
    /// chrome looks taller than native Qt apps.
    #[serde(default)]
    pub compact_ui: bool,
    /// Send single-Ctrl terminal control shortcuts (Ctrl+F/P/N/W/H/M/I) to the
    /// focused terminal/viewer instead of the application accelerators.
    ///
    /// Default `true`: while the terminal or an embedded viewer has focus, the
    /// colliding single-Ctrl accelerators are temporarily suspended so readline
    /// chords reach the shell (issue #197). When `false`, accelerators stay active
    /// (the old behavior).
    #[serde(default = "default_true")]
    pub terminal_passthrough_ctrl: bool,
}

impl UiSettings {
    /// Adds a search query to the persisted history
    ///
    /// Moves existing entries to front and limits to max entries.
    pub fn add_search_history(&mut self, query: &str) {
        let query = query.trim();
        if query.is_empty() {
            return;
        }

        // Remove if already exists (to move to front)
        self.search_history.retain(|q| q != query);

        // Add to front
        self.search_history.insert(0, query.to_string());

        // Trim to max size
        self.search_history.truncate(MAX_SEARCH_HISTORY_ENTRIES);
    }

    /// Clears the search history
    pub fn clear_search_history(&mut self) {
        self.search_history.clear();
    }
}

/// Session restore settings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionRestoreSettings {
    /// Whether to restore sessions on startup
    #[serde(default)]
    pub enabled: bool,
    /// Whether to prompt before restoring
    #[serde(default = "default_true")]
    pub prompt_on_restore: bool,
    /// Maximum age of sessions to restore (in hours, 0 = no limit)
    #[serde(default = "default_session_max_age")]
    pub max_age_hours: u32,
    /// Sessions to restore (connection IDs)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saved_sessions: Vec<SavedSession>,
}

const fn default_session_max_age() -> u32 {
    24
}

impl Default for SessionRestoreSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            prompt_on_restore: true,
            max_age_hours: default_session_max_age(),
            saved_sessions: Vec::new(),
        }
    }
}

/// A saved session for restore
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSession {
    /// Connection ID
    pub connection_id: uuid::Uuid,
    /// Connection name (for display if connection deleted)
    pub connection_name: String,
    /// Protocol type
    pub protocol: String,
    /// Host
    pub host: String,
    /// Port
    pub port: u16,
    /// When the session was saved
    pub saved_at: chrono::DateTime<chrono::Utc>,
}

/// Default language value (system auto-detect)
fn default_language() -> String {
    "system".to_string()
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            color_scheme: ColorScheme::default(),
            language: default_language(),
            remember_window_geometry: true,
            window_width: None,
            window_height: None,
            window_maximized: false,
            sidebar_width: None,
            enable_tray_icon: true,
            minimize_to_tray: false,
            expanded_groups: std::collections::HashSet::new(),
            session_restore: SessionRestoreSettings::default(),
            search_history: Vec::new(),
            startup_action: StartupAction::default(),
            color_tabs_by_protocol: false,
            show_protocol_filters: false,
            show_smart_folders: false,
            compact_ui: cfg!(target_os = "macos"),
            terminal_passthrough_ctrl: true,
        }
    }
}

/// Connection settings for pre-connect checks and timeouts
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionSettings {
    /// Enable TCP port check before connecting (faster failure detection)
    #[serde(default = "default_true")]
    pub pre_connect_port_check: bool,
    /// Timeout in seconds for port check (default: 3)
    #[serde(default = "default_port_check_timeout")]
    pub port_check_timeout_secs: u32,
}

const fn default_port_check_timeout() -> u32 {
    3
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            pre_connect_port_check: true,
            port_check_timeout_secs: default_port_check_timeout(),
        }
    }
}

/// A persisted Quick Connect history entry (no secrets)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuickConnectHistoryItem {
    /// Protocol name: "SSH", "RDP", "VNC", "Telnet"
    pub protocol: String,
    /// Host or IP address
    pub host: String,
    /// Port number
    pub port: u16,
    /// Username (if provided)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
}

// The `RCSC` blob format and machine-key crypto live in
// [`crate::secret::local_crypto`]; this module delegates to it so the
// encrypted-file backend and the `*_encrypted` settings fields share one
// implementation. The on-disk format is preserved byte-for-byte.

/// Password encryption utilities for credential persistence
///
/// Uses AES-256-GCM with Argon2id key derivation from a machine-specific key.
impl SecretSettings {
    /// Reads the canonical credential storage choice for the KDBX backend
    /// from the legacy `kdbx_password_encrypted` + `kdbx_save_to_keyring`
    /// pair.
    #[must_use]
    pub fn kdbx_storage(&self) -> CredentialStorage {
        CredentialStorage::from_legacy(
            self.kdbx_password_encrypted.is_some(),
            self.kdbx_save_to_keyring,
        )
    }

    /// Writes the canonical credential storage choice for the KDBX backend
    /// back to the legacy `kdbx_password_encrypted` + `kdbx_save_to_keyring`
    /// fields. The encrypted blob is preserved when switching to
    /// [`CredentialStorage::EncryptedFile`] so a later
    /// [`Self::encrypt_password`] call can populate it; switching to any
    /// other state clears both.
    pub fn set_kdbx_storage(&mut self, storage: CredentialStorage) {
        match storage {
            CredentialStorage::None => {
                self.kdbx_save_to_keyring = false;
                self.kdbx_password_encrypted = None;
            }
            CredentialStorage::EncryptedFile => {
                self.kdbx_save_to_keyring = false;
                // Leave kdbx_password_encrypted untouched: encrypt_password()
                // will populate it from kdbx_password before save.
            }
            CredentialStorage::SystemKeyring => {
                self.kdbx_save_to_keyring = true;
                self.kdbx_password_encrypted = None;
            }
        }
    }

    /// Reads the canonical credential storage choice for the Bitwarden
    /// backend.
    #[must_use]
    pub fn bitwarden_storage(&self) -> CredentialStorage {
        CredentialStorage::from_legacy(
            self.bitwarden_password_encrypted.is_some(),
            self.bitwarden_save_to_keyring,
        )
    }

    /// Writes the canonical credential storage choice for the Bitwarden
    /// backend.
    pub fn set_bitwarden_storage(&mut self, storage: CredentialStorage) {
        match storage {
            CredentialStorage::None => {
                self.bitwarden_save_to_keyring = false;
                self.bitwarden_password_encrypted = None;
            }
            CredentialStorage::EncryptedFile => {
                self.bitwarden_save_to_keyring = false;
            }
            CredentialStorage::SystemKeyring => {
                self.bitwarden_save_to_keyring = true;
                self.bitwarden_password_encrypted = None;
            }
        }
    }

    /// Reads the canonical credential storage choice for the 1Password
    /// backend.
    #[must_use]
    pub fn onepassword_storage(&self) -> CredentialStorage {
        CredentialStorage::from_legacy(
            self.onepassword_service_account_token_encrypted.is_some(),
            self.onepassword_save_to_keyring,
        )
    }

    /// Writes the canonical credential storage choice for the 1Password
    /// backend.
    pub fn set_onepassword_storage(&mut self, storage: CredentialStorage) {
        match storage {
            CredentialStorage::None => {
                self.onepassword_save_to_keyring = false;
                self.onepassword_service_account_token_encrypted = None;
            }
            CredentialStorage::EncryptedFile => {
                self.onepassword_save_to_keyring = false;
            }
            CredentialStorage::SystemKeyring => {
                self.onepassword_save_to_keyring = true;
                self.onepassword_service_account_token_encrypted = None;
            }
        }
    }

    /// Reads the canonical credential storage choice for the Passbolt
    /// backend.
    #[must_use]
    pub fn passbolt_storage(&self) -> CredentialStorage {
        CredentialStorage::from_legacy(
            self.passbolt_passphrase_encrypted.is_some(),
            self.passbolt_save_to_keyring,
        )
    }

    /// Writes the canonical credential storage choice for the Passbolt
    /// backend.
    pub fn set_passbolt_storage(&mut self, storage: CredentialStorage) {
        match storage {
            CredentialStorage::None => {
                self.passbolt_save_to_keyring = false;
                self.passbolt_passphrase_encrypted = None;
            }
            CredentialStorage::EncryptedFile => {
                self.passbolt_save_to_keyring = false;
            }
            CredentialStorage::SystemKeyring => {
                self.passbolt_save_to_keyring = true;
                self.passbolt_passphrase_encrypted = None;
            }
        }
    }

    /// Encrypts the KDBX password for storage using AES-256-GCM
    pub fn encrypt_password(&mut self) {
        if let Some(ref password) = self.kdbx_password {
            use secrecy::ExposeSecret;
            if let Ok(encrypted) = encrypt_credential(
                password.expose_secret().as_bytes(),
                &Self::get_machine_key(),
            ) {
                self.kdbx_password_encrypted = Some(hex_encode(&encrypted));
            }
        }
    }

    /// Decrypts the stored KDBX password
    ///
    /// Decrypts AES-256-GCM (RCSC) credentials.
    /// Returns true if decryption was successful.
    pub fn decrypt_password(&mut self) -> bool {
        if let Some(ref encrypted) = self.kdbx_password_encrypted
            && let Some(decoded) = hex_decode(encrypted)
        {
            let key = Self::get_machine_key();
            if let Ok(plaintext) = decrypt_credential(&decoded, &key)
                && let Ok(password_str) = std::str::from_utf8(&plaintext)
            {
                self.kdbx_password = Some(SecretString::from(password_str.to_owned()));
                return true;
            }
        }
        false
    }

    /// Clears both encrypted and runtime password
    pub fn clear_password(&mut self) {
        self.kdbx_password = None;
        self.kdbx_password_encrypted = None;
    }

    /// Encrypts the Bitwarden master password for storage using AES-256-GCM
    pub fn encrypt_bitwarden_password(&mut self) {
        if let Some(ref password) = self.bitwarden_password {
            use secrecy::ExposeSecret;
            if let Ok(encrypted) = encrypt_credential(
                password.expose_secret().as_bytes(),
                &Self::get_machine_key(),
            ) {
                self.bitwarden_password_encrypted = Some(hex_encode(&encrypted));
            }
        }
    }

    /// Decrypts the stored Bitwarden master password
    ///
    /// Decrypts AES-256-GCM (RCSC) credentials.
    /// Returns true if decryption was successful.
    pub fn decrypt_bitwarden_password(&mut self) -> bool {
        if let Some(ref encrypted) = self.bitwarden_password_encrypted
            && let Some(decoded) = hex_decode(encrypted)
        {
            let key = Self::get_machine_key();
            if let Ok(plaintext) = decrypt_credential(&decoded, &key)
                && let Ok(password_str) = std::str::from_utf8(&plaintext)
            {
                self.bitwarden_password = Some(SecretString::from(password_str.to_owned()));
                return true;
            }
        }
        false
    }

    /// Clears both encrypted and runtime Bitwarden password
    pub fn clear_bitwarden_password(&mut self) {
        self.bitwarden_password = None;
        self.bitwarden_password_encrypted = None;
    }

    /// Encrypts the Bitwarden API credentials (client_id + client_secret) for storage
    pub fn encrypt_bitwarden_api_credentials(&mut self) {
        use secrecy::ExposeSecret;
        let key = Self::get_machine_key();
        if let Some(ref client_id) = self.bitwarden_client_id
            && let Ok(encrypted) = encrypt_credential(client_id.expose_secret().as_bytes(), &key)
        {
            self.bitwarden_client_id_encrypted = Some(hex_encode(&encrypted));
        }
        if let Some(ref client_secret) = self.bitwarden_client_secret
            && let Ok(encrypted) =
                encrypt_credential(client_secret.expose_secret().as_bytes(), &key)
        {
            self.bitwarden_client_secret_encrypted = Some(hex_encode(&encrypted));
        }
    }

    /// Decrypts the stored Bitwarden API credentials (client_id + client_secret)
    ///
    /// Decrypts AES-256-GCM (RCSC) credentials.
    /// Returns true if at least one credential was decrypted successfully.
    pub fn decrypt_bitwarden_api_credentials(&mut self) -> bool {
        let key = Self::get_machine_key();
        let id_ok = if let Some(ref encrypted) = self.bitwarden_client_id_encrypted
            && let Some(decoded) = hex_decode(encrypted)
            && let Ok(plaintext) = decrypt_credential(&decoded, &key)
            && let Ok(s) = std::str::from_utf8(&plaintext)
        {
            self.bitwarden_client_id = Some(SecretString::from(s.to_owned()));
            true
        } else {
            false
        };
        let secret_ok = if let Some(ref encrypted) = self.bitwarden_client_secret_encrypted
            && let Some(decoded) = hex_decode(encrypted)
            && let Ok(plaintext) = decrypt_credential(&decoded, &key)
            && let Ok(s) = std::str::from_utf8(&plaintext)
        {
            self.bitwarden_client_secret = Some(SecretString::from(s.to_owned()));
            true
        } else {
            false
        };
        id_ok || secret_ok
    }

    /// Encrypts the 1Password service account token for storage using AES-256-GCM
    pub fn encrypt_onepassword_token(&mut self) {
        if let Some(ref token) = self.onepassword_service_account_token {
            use secrecy::ExposeSecret;
            if let Ok(encrypted) =
                encrypt_credential(token.expose_secret().as_bytes(), &Self::get_machine_key())
            {
                self.onepassword_service_account_token_encrypted = Some(hex_encode(&encrypted));
            }
        }
    }

    /// Decrypts the stored 1Password service account token
    ///
    /// Decrypts AES-256-GCM (RCSC) credentials.
    /// Returns true if decryption was successful.
    pub fn decrypt_onepassword_token(&mut self) -> bool {
        if let Some(ref encrypted) = self.onepassword_service_account_token_encrypted
            && let Some(decoded) = hex_decode(encrypted)
        {
            let key = Self::get_machine_key();
            if let Ok(plaintext) = decrypt_credential(&decoded, &key)
                && let Ok(token_str) = std::str::from_utf8(&plaintext)
            {
                self.onepassword_service_account_token =
                    Some(SecretString::from(token_str.to_owned()));
                return true;
            }
        }
        false
    }

    /// Encrypts the Passbolt GPG passphrase for storage using AES-256-GCM
    pub fn encrypt_passbolt_passphrase(&mut self) {
        if let Some(ref passphrase) = self.passbolt_passphrase {
            use secrecy::ExposeSecret;
            if let Ok(encrypted) = encrypt_credential(
                passphrase.expose_secret().as_bytes(),
                &Self::get_machine_key(),
            ) {
                self.passbolt_passphrase_encrypted = Some(hex_encode(&encrypted));
            }
        }
    }

    /// Decrypts the stored Passbolt GPG passphrase
    ///
    /// Decrypts AES-256-GCM (RCSC) credentials.
    /// Returns true if decryption was successful.
    pub fn decrypt_passbolt_passphrase(&mut self) -> bool {
        if let Some(ref encrypted) = self.passbolt_passphrase_encrypted
            && let Some(decoded) = hex_decode(encrypted)
        {
            let key = Self::get_machine_key();
            if let Ok(plaintext) = decrypt_credential(&decoded, &key)
                && let Ok(pass_str) = std::str::from_utf8(&plaintext)
            {
                self.passbolt_passphrase = Some(SecretString::from(pass_str.to_owned()));
                return true;
            }
        }
        false
    }

    /// Gets a machine-specific key for encryption.
    ///
    /// Delegates to [`crate::secret::local_crypto::get_machine_key`].
    fn get_machine_key() -> Vec<u8> {
        crate::secret::local_crypto::get_machine_key()
    }
}

/// Encrypts credential data using AES-256-GCM with Argon2id key derivation.
///
/// Output format: `RCSC` (4) + version (1) + salt (16) + nonce (12) +
/// ciphertext + tag (16). Delegates to
/// [`crate::secret::local_crypto::encrypt_credential`].
fn encrypt_credential(plaintext: &[u8], machine_key: &[u8]) -> Result<Vec<u8>, String> {
    crate::secret::local_crypto::encrypt_credential(plaintext, machine_key)
}

/// Decrypts AES-256-GCM (`RCSC`) credential data.
///
/// Returns the recovered plaintext wrapped in [`Zeroizing`] so it is wiped on
/// drop. Delegates to [`crate::secret::local_crypto::decrypt_credential`].
fn decrypt_credential(data: &[u8], machine_key: &[u8]) -> Result<Zeroizing<Vec<u8>>, String> {
    crate::secret::local_crypto::decrypt_credential(data, machine_key)
}

/// Hex-encodes binary data to a string
fn hex_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::with_capacity(data.len() * 2);
    for byte in data {
        write!(result, "{byte:02x}").ok();
    }
    result
}

/// Hex-decodes a string to binary data
fn hex_decode(data: &str) -> Option<Vec<u8>> {
    let mut result = Vec::with_capacity(data.len() / 2);
    let mut chars = data.chars();
    while let (Some(a), Some(b)) = (chars.next(), chars.next()) {
        let byte = u8::from_str_radix(&format!("{a}{b}"), 16).ok()?;
        result.push(byte);
    }
    Some(result)
}
