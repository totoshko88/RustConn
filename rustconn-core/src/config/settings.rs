//! Application settings model
//!
//! This module defines the application-wide settings stored in config.toml.

use crate::models::HistorySettings;
use crate::variables::Variable;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
}

/// Terminal-related settings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // Terminal settings are independent boolean flags
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
        }
    }
}

/// Logging settings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)] // Logging modes are independent boolean flags
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
#[allow(clippy::struct_excessive_bools)]
pub struct SecretSettings {
    /// Preferred secret backend
    #[serde(default)]
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
}

const fn default_true() -> bool {
    true
}

impl Default for SecretSettings {
    fn default() -> Self {
        Self {
            preferred_backend: SecretBackendType::default(),
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
            && self.kdbx_save_to_keyring == other.kdbx_save_to_keyring
            && self.bitwarden_save_to_keyring == other.bitwarden_save_to_keyring
            && self.onepassword_save_to_keyring == other.onepassword_save_to_keyring
            && self.passbolt_save_to_keyring == other.passbolt_save_to_keyring
            && self.passbolt_server_url == other.passbolt_server_url
        // Note: kdbx_password is intentionally excluded from equality comparison
        // as it's a runtime-only field that shouldn't affect settings equality
    }
}

impl Eq for SecretSettings {}

/// Secret backend type
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretBackendType {
    /// `KeePassXC` browser integration
    #[default]
    KeePassXc,
    /// Direct KDBX file access (GNOME Secrets, `OneKeePass`, KeePass compatible)
    KdbxFile,
    /// libsecret (GNOME Keyring/KDE Wallet)
    LibSecret,
    /// Bitwarden CLI
    Bitwarden,
    /// 1Password CLI
    OnePassword,
    /// Passbolt CLI
    Passbolt,
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

/// Maximum number of search history entries to persist
const MAX_SEARCH_HISTORY_ENTRIES: usize = 20;

/// UI settings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSettings {
    /// Color scheme preference
    #[serde(default)]
    pub color_scheme: ColorScheme,
    /// Remember window geometry
    #[serde(default = "default_true")]
    pub remember_window_geometry: bool,
    /// Window width
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_width: Option<i32>,
    /// Window height
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_height: Option<i32>,
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

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            color_scheme: ColorScheme::default(),
            remember_window_geometry: true,
            window_width: None,
            window_height: None,
            sidebar_width: None,
            enable_tray_icon: true,
            minimize_to_tray: false,
            expanded_groups: std::collections::HashSet::new(),
            session_restore: SessionRestoreSettings::default(),
            search_history: Vec::new(),
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

/// Password encryption utilities for KDBX password persistence
impl SecretSettings {
    /// Encrypts the KDBX password for storage
    /// Uses a simple XOR cipher with machine-specific key
    pub fn encrypt_password(&mut self) {
        if let Some(ref password) = self.kdbx_password {
            use secrecy::ExposeSecret;
            let key = Self::get_machine_key();
            let encrypted = Self::xor_cipher(password.expose_secret().as_bytes(), &key);
            self.kdbx_password_encrypted = Some(base64_encode(&encrypted));
        }
    }

    /// Decrypts the stored KDBX password
    /// Returns true if decryption was successful
    pub fn decrypt_password(&mut self) -> bool {
        if let Some(ref encrypted) = self.kdbx_password_encrypted {
            if let Some(decoded) = base64_decode(encrypted) {
                let key = Self::get_machine_key();
                let decrypted = Self::xor_cipher(&decoded, &key);
                if let Ok(password_str) = String::from_utf8(decrypted) {
                    self.kdbx_password = Some(SecretString::from(password_str));
                    return true;
                }
            }
        }
        false
    }

    /// Clears both encrypted and runtime password
    pub fn clear_password(&mut self) {
        self.kdbx_password = None;
        self.kdbx_password_encrypted = None;
    }

    /// Encrypts the Bitwarden master password for storage
    /// Uses a simple XOR cipher with machine-specific key
    pub fn encrypt_bitwarden_password(&mut self) {
        if let Some(ref password) = self.bitwarden_password {
            use secrecy::ExposeSecret;
            let key = Self::get_machine_key();
            let encrypted = Self::xor_cipher(password.expose_secret().as_bytes(), &key);
            self.bitwarden_password_encrypted = Some(base64_encode(&encrypted));
        }
    }

    /// Decrypts the stored Bitwarden master password
    /// Returns true if decryption was successful
    pub fn decrypt_bitwarden_password(&mut self) -> bool {
        if let Some(ref encrypted) = self.bitwarden_password_encrypted {
            if let Some(decoded) = base64_decode(encrypted) {
                let key = Self::get_machine_key();
                let decrypted = Self::xor_cipher(&decoded, &key);
                if let Ok(password_str) = String::from_utf8(decrypted) {
                    self.bitwarden_password = Some(SecretString::from(password_str));
                    return true;
                }
            }
        }
        false
    }

    /// Clears both encrypted and runtime Bitwarden password
    pub fn clear_bitwarden_password(&mut self) {
        self.bitwarden_password = None;
        self.bitwarden_password_encrypted = None;
    }

    /// Encrypts the 1Password service account token for storage
    pub fn encrypt_onepassword_token(&mut self) {
        if let Some(ref token) = self.onepassword_service_account_token {
            use secrecy::ExposeSecret;
            let key = Self::get_machine_key();
            let encrypted = Self::xor_cipher(token.expose_secret().as_bytes(), &key);
            self.onepassword_service_account_token_encrypted = Some(base64_encode(&encrypted));
        }
    }

    /// Decrypts the stored 1Password service account token
    /// Returns true if decryption was successful
    pub fn decrypt_onepassword_token(&mut self) -> bool {
        if let Some(ref encrypted) = self.onepassword_service_account_token_encrypted {
            if let Some(decoded) = base64_decode(encrypted) {
                let key = Self::get_machine_key();
                let decrypted = Self::xor_cipher(&decoded, &key);
                if let Ok(token_str) = String::from_utf8(decrypted) {
                    self.onepassword_service_account_token = Some(SecretString::from(token_str));
                    return true;
                }
            }
        }
        false
    }

    /// Encrypts the Passbolt GPG passphrase for storage
    pub fn encrypt_passbolt_passphrase(&mut self) {
        if let Some(ref passphrase) = self.passbolt_passphrase {
            use secrecy::ExposeSecret;
            let key = Self::get_machine_key();
            let encrypted = Self::xor_cipher(passphrase.expose_secret().as_bytes(), &key);
            self.passbolt_passphrase_encrypted = Some(base64_encode(&encrypted));
        }
    }

    /// Decrypts the stored Passbolt GPG passphrase
    /// Returns true if decryption was successful
    pub fn decrypt_passbolt_passphrase(&mut self) -> bool {
        if let Some(ref encrypted) = self.passbolt_passphrase_encrypted {
            if let Some(decoded) = base64_decode(encrypted) {
                let key = Self::get_machine_key();
                let decrypted = Self::xor_cipher(&decoded, &key);
                if let Ok(pass_str) = String::from_utf8(decrypted) {
                    self.passbolt_passphrase = Some(SecretString::from(pass_str));
                    return true;
                }
            }
        }
        false
    }

    /// Gets a machine-specific key for encryption
    /// Uses machine-id or falls back to a default
    fn get_machine_key() -> Vec<u8> {
        // Try to read machine-id (Linux)
        if let Ok(machine_id) = std::fs::read_to_string("/etc/machine-id") {
            return machine_id.trim().as_bytes().to_vec();
        }
        // Fallback to hostname + username
        let hostname = hostname::get().map_or_else(
            |_| "rustconn".to_string(),
            |h| h.to_string_lossy().to_string(),
        );
        let username = std::env::var("USER").unwrap_or_else(|_| "user".to_string());
        format!("{hostname}-{username}-rustconn-key").into_bytes()
    }

    /// Simple XOR cipher for obfuscation
    fn xor_cipher(data: &[u8], key: &[u8]) -> Vec<u8> {
        data.iter()
            .enumerate()
            .map(|(i, &byte)| byte ^ key[i % key.len()])
            .collect()
    }
}

/// Base64 encode helper
fn base64_encode(data: &[u8]) -> String {
    use std::fmt::Write;
    let mut result = String::new();
    for byte in data {
        write!(result, "{byte:02x}").ok();
    }
    result
}

/// Base64 decode helper (hex decode)
fn base64_decode(data: &str) -> Option<Vec<u8>> {
    let mut result = Vec::new();
    let mut chars = data.chars();
    while let (Some(a), Some(b)) = (chars.next(), chars.next()) {
        let byte = u8::from_str_radix(&format!("{a}{b}"), 16).ok()?;
        result.push(byte);
    }
    Some(result)
}
