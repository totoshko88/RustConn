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
    /// Global variables
    #[serde(default)]
    pub global_variables: Vec<Variable>,
    /// Connection history settings
    #[serde(default)]
    pub history: HistorySettings,
}

/// Terminal-related settings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_family: default_font_family(),
            font_size: default_font_size(),
            scrollback_lines: default_scrollback(),
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
    /// Whether to use key file instead of password
    #[serde(default)]
    pub kdbx_use_key_file: bool,
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
    /// Direct KDBX file access
    KdbxFile,
    /// libsecret (GNOME Keyring/KDE Wallet)
    LibSecret,
}

/// UI settings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UiSettings {
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
            remember_window_geometry: true,
            window_width: None,
            window_height: None,
            sidebar_width: None,
            enable_tray_icon: true,
            minimize_to_tray: false,
            expanded_groups: std::collections::HashSet::new(),
            session_restore: SessionRestoreSettings::default(),
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
