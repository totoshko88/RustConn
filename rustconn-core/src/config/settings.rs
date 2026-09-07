//! Application settings model
//!
//! This module defines the application-wide settings stored in config.toml.

use std::path::PathBuf;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::activity_monitor::ActivityMonitorDefaults;
use crate::models::{HighlightRule, HistorySettings, SmartFolder};
use crate::monitoring::MonitoringSettings;
use crate::secret::CredentialStorage;
use crate::sync::SyncSettings;
use crate::variables::Variable;

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
    /// Application-wide bastion settings, the outermost tier of proxy inheritance
    #[serde(default)]
    pub network: NetworkSettings,
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
    /// Keep the terminal scrollback when a session reconnects in place.
    ///
    /// When `true` (default), the output of the previous session stays readable
    /// after a reconnect — the common case being a session dropped by an idle
    /// timeout, where the user still wants to consult what was on screen
    /// (issue #253). A dim separator marks where the new session begins.
    /// When `false`, the terminal is cleared on reconnect.
    #[serde(default = "default_keep_history_on_reconnect")]
    pub keep_history_on_reconnect: bool,
    /// Maximum scrollback lines to retain after a reconnect.
    ///
    /// When a session reconnects and `keep_history_on_reconnect` is true, VTE
    /// adds the new session's output on top of whatever was already there. Over
    /// many reconnects (e.g. an idle-timeout loop) the buffer can grow without
    /// bound. This cap removes the oldest lines *before* the reconnect rule is
    /// inserted, so the buffer never exceeds `scrollback_lines + this limit`.
    /// `None` (the default) means no cap beyond VTE's own scrollback_lines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_scrollback_on_reconnect: Option<u32>,
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

/// Default terminal theme: follow the desktop's light/dark preference.
///
/// Only reached when `color_theme` is absent from `settings.toml` — a fresh
/// install, or a config written before the field existed. Anyone who has ever
/// saved Preferences has an explicit value stored and keeps it.
fn default_color_theme() -> String {
    crate::terminal_themes::FOLLOW_SYSTEM_THEME.to_string()
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

const fn default_keep_history_on_reconnect() -> bool {
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
            keep_history_on_reconnect: default_keep_history_on_reconnect(),
            max_scrollback_on_reconnect: None,
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
///
/// `Debug` is implemented manually (see below) rather than derived: the
/// `SecretString` fields redact themselves, but the `*_encrypted` fields are
/// plain `String`s holding a machine-key-encrypted credential, and a derived
/// `Debug` would print those ciphertexts verbatim. The manual impl redacts
/// every secret-bearing field, and `secret_settings_debug_does_not_leak` proves
/// it (M-PUBLIC-DEBUG).
#[derive(Clone, Serialize, Deserialize)]
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
    /// Path to the portable encrypted credential file.
    ///
    /// Can point to a cloud-synced directory (Dropbox, Syncthing, etc.) so the
    /// same file is accessible from multiple machines.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portable_file_path: Option<PathBuf>,
    /// Portable file passphrase (NOT serialized — runtime only).
    ///
    /// Set after the user unlocks the portable file at the start of a session.
    #[serde(skip)]
    pub portable_passphrase: Option<SecretString>,
    /// Machine-local encrypted copy of the portable passphrase for convenience.
    ///
    /// Encrypted with the machine key (same as other `*_encrypted` fields) so
    /// the user does not have to re-enter the passphrase every session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub portable_passphrase_encrypted: Option<String>,
    /// Whether to save the portable passphrase to the system keyring.
    #[serde(default)]
    pub portable_save_to_keyring: bool,
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

impl std::fmt::Debug for SecretSettings {
    /// Redacts every secret-bearing field.
    ///
    /// `SecretString` redacts itself, but the `*_encrypted` fields are plain
    /// `String` ciphertexts a derived `Debug` would print verbatim. Both kinds
    /// are shown as a presence marker (`<set>` / `<none>`) so a debug dump still
    /// tells you whether a credential is configured without disclosing it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        /// Presence marker for an `Option` holding a secret, without its value.
        fn redacted<T>(opt: Option<&T>) -> &'static str {
            if opt.is_some() { "<set>" } else { "<none>" }
        }

        f.debug_struct("SecretSettings")
            .field("preferred_backend", &self.preferred_backend)
            .field("enable_fallback", &self.enable_fallback)
            .field("kdbx_path", &self.kdbx_path)
            .field("kdbx_enabled", &self.kdbx_enabled)
            .field("kdbx_password", &redacted(self.kdbx_password.as_ref()))
            .field(
                "kdbx_password_encrypted",
                &redacted(self.kdbx_password_encrypted.as_ref()),
            )
            .field("kdbx_key_file", &self.kdbx_key_file)
            .field("kdbx_use_key_file", &self.kdbx_use_key_file)
            .field("kdbx_use_password", &self.kdbx_use_password)
            .field(
                "bitwarden_password",
                &redacted(self.bitwarden_password.as_ref()),
            )
            .field(
                "bitwarden_password_encrypted",
                &redacted(self.bitwarden_password_encrypted.as_ref()),
            )
            .field("bitwarden_use_api_key", &self.bitwarden_use_api_key)
            .field(
                "bitwarden_client_id",
                &redacted(self.bitwarden_client_id.as_ref()),
            )
            .field(
                "bitwarden_client_id_encrypted",
                &redacted(self.bitwarden_client_id_encrypted.as_ref()),
            )
            .field(
                "bitwarden_client_secret",
                &redacted(self.bitwarden_client_secret.as_ref()),
            )
            .field(
                "bitwarden_client_secret_encrypted",
                &redacted(self.bitwarden_client_secret_encrypted.as_ref()),
            )
            .field("bitwarden_save_to_keyring", &self.bitwarden_save_to_keyring)
            .field("kdbx_save_to_keyring", &self.kdbx_save_to_keyring)
            .field(
                "onepassword_service_account_token",
                &redacted(self.onepassword_service_account_token.as_ref()),
            )
            .field(
                "onepassword_service_account_token_encrypted",
                &redacted(self.onepassword_service_account_token_encrypted.as_ref()),
            )
            .field(
                "onepassword_save_to_keyring",
                &self.onepassword_save_to_keyring,
            )
            .field(
                "passbolt_passphrase",
                &redacted(self.passbolt_passphrase.as_ref()),
            )
            .field(
                "passbolt_passphrase_encrypted",
                &redacted(self.passbolt_passphrase_encrypted.as_ref()),
            )
            .field("passbolt_save_to_keyring", &self.passbolt_save_to_keyring)
            .field("passbolt_server_url", &self.passbolt_server_url)
            .field("pass_store_dir", &self.pass_store_dir)
            .field("portable_file_path", &self.portable_file_path)
            .field(
                "portable_passphrase",
                &redacted(self.portable_passphrase.as_ref()),
            )
            .field(
                "portable_passphrase_encrypted",
                &redacted(self.portable_passphrase_encrypted.as_ref()),
            )
            .field("portable_save_to_keyring", &self.portable_save_to_keyring)
            .finish()
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
            portable_file_path: None,
            portable_passphrase: None,
            portable_passphrase_encrypted: None,
            portable_save_to_keyring: false,
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
            && self.portable_file_path == other.portable_file_path
            && self.portable_passphrase_encrypted == other.portable_passphrase_encrypted
            && self.portable_save_to_keyring == other.portable_save_to_keyring
        // Note: runtime-only SecretString fields (kdbx_password, bitwarden_password,
        // bitwarden_client_id, bitwarden_client_secret, onepassword_service_account_token,
        // passbolt_passphrase, portable_passphrase) are intentionally excluded — they are
        // #[serde(skip)] and not persisted, so they shouldn't affect settings equality.
    }
}

impl Eq for SecretSettings {}

/// System-keyring entries orphaned by a settings change.
///
/// Produced by [`SecretSettings::keyring_revocations`]; each flag marks a
/// credential whose backend no longer stores it in the system keyring, so the
/// stale entry has to be removed. Kept as a plain flag record rather than a set
/// so the caller can name each keyring key explicitly.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "one independent flag per keyring entry; a set would hide which key is meant"
)]
pub struct KeyringRevocations {
    /// The KeePass database password entry is stale.
    pub kdbx_password: bool,
    /// The Bitwarden master password entry is stale.
    pub bitwarden_password: bool,
    /// The Bitwarden API `client_id` / `client_secret` entries are stale.
    pub bitwarden_api_credentials: bool,
    /// The 1Password service account token entry is stale.
    pub onepassword_token: bool,
    /// The Passbolt GPG passphrase entry is stale.
    pub passbolt_passphrase: bool,
    /// The portable encrypted file passphrase entry is stale.
    pub portable_passphrase: bool,
}

impl KeyringRevocations {
    /// Reports whether anything needs revoking at all.
    #[must_use]
    pub const fn any(&self) -> bool {
        self.kdbx_password
            || self.bitwarden_password
            || self.bitwarden_api_credentials
            || self.onepassword_token
            || self.passbolt_passphrase
            || self.portable_passphrase
    }
}

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
    /// Portable encrypted file — passphrase-based, cloud-syncable.
    ///
    /// Same AES-256-GCM per-entry blob format as [`Self::EncryptedFile`], but
    /// the encryption key is derived from a user-supplied passphrase (Argon2id)
    /// instead of a machine-specific key. The file can live in a cloud-synced
    /// directory and be opened on any machine with the same passphrase.
    PortableEncryptedFile,
}

impl SecretBackendType {
    /// Returns the untranslated name to show a user for this backend.
    ///
    /// Wrap the result in `i18n()` at the call site. The product names are not
    /// translated; the three descriptive ones are, which is why this returns the
    /// English form rather than a localised string.
    ///
    /// This exists because the alternative is `format!("{self:?}")`, and the
    /// startup banner did exactly that — telling users their backend was
    /// `MacOsKeychain` or `LibSecret`, which are Rust variant names and not
    /// anything the interface calls them. Any message naming a backend goes
    /// through here so the name cannot drift per call site.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::KeePassXc => "KeePassXC",
            Self::KdbxFile => "KDBX file",
            Self::LibSecret => "libsecret",
            Self::Bitwarden => "Bitwarden",
            Self::OnePassword => "1Password",
            Self::Passbolt => "Passbolt",
            // Lowercase because that is the program's name: `pass`, the standard
            // unix password manager. Capitalising it made the selector row read
            // as a product called "Pass" while the row's own description, the
            // status label and the documentation all say `pass`.
            Self::Pass => "pass",
            Self::MacOsKeychain => "macOS Keychain",
            Self::EncryptedFile => "Encrypted file",
            Self::PortableEncryptedFile => "Portable encrypted file",
        }
    }

    /// Maps a [`SecretBackend::backend_id`] string back to its configuration variant.
    ///
    /// Needed because [`crate::secret::StoreOutcome::Fallback`] and its retrieve
    /// counterpart report *which backend answered* as a `backend_id`, and a
    /// message that has to name that backend needs a [`Self`] to call
    /// [`Self::display_name`] on. Returns `None` for an unrecognised id rather
    /// than guessing, so a caller can fall back to printing the raw id.
    ///
    /// `keepassxc` and `kdbx_file` are listed but cannot occur today: KDBX is
    /// reached through `KeePassStatus` and the `keepassxc-cli` binary, not
    /// through a [`SecretBackend`] implementation, so no chain ever reports
    /// those ids. They are here so that changing that does not silently start
    /// printing a raw id.
    ///
    /// [`SecretBackend`]: crate::secret::SecretBackend
    /// [`SecretBackend::backend_id`]: crate::secret::SecretBackend::backend_id
    #[must_use]
    pub fn from_backend_id(backend_id: &str) -> Option<Self> {
        match backend_id {
            "keepassxc" => Some(Self::KeePassXc),
            "kdbx_file" => Some(Self::KdbxFile),
            "libsecret" => Some(Self::LibSecret),
            "bitwarden" => Some(Self::Bitwarden),
            "onepassword" => Some(Self::OnePassword),
            "passbolt" => Some(Self::Passbolt),
            "pass" => Some(Self::Pass),
            "macos_keychain" => Some(Self::MacOsKeychain),
            "encrypted_file" => Some(Self::EncryptedFile),
            "portable_encrypted_file" => Some(Self::PortableEncryptedFile),
            _ => None,
        }
    }
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

/// Which GSK renderer the GUI should ask GTK for.
///
/// GTK offers no API for this — the `GSK_RENDERER` environment variable is the
/// only interface, and it must be set before `gtk_init`. The value is therefore
/// read at startup and takes effect on the next launch, not immediately.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RendererPreference {
    /// Let RustConn decide per environment.
    ///
    /// Picks software rendering where the GPU path is known to be worse: X11
    /// sessions whose compositor paints GTK4 popovers blank until hovered
    /// (issue #85), and macOS guests running under a hypervisor, where the
    /// paravirtualised GPU offers Metal but no accelerated OpenGL, so GSK's GL
    /// renderer lands on a software fallback that is slower than Cairo and
    /// burns CPU (issue #274). Everywhere else GTK's own default stands.
    #[default]
    Auto,
    /// Always let GTK choose its default (GPU) renderer.
    ///
    /// The way out for a user whom [`Self::Auto`] downgrades unnecessarily —
    /// an X11 session with a working driver, for instance.
    Gpu,
    /// Always use the Cairo (software) renderer.
    Software,
}

impl RendererPreference {
    /// Returns the untranslated label for this preference.
    ///
    /// Wrap the result in `i18n()` at the call site.
    #[must_use]
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Auto => "Automatic",
            Self::Gpu => "Hardware (GPU)",
            Self::Software => "Software (Cairo)",
        }
    }
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
    /// Which GSK renderer to ask GTK for; applies from the next start.
    #[serde(default)]
    pub renderer: RendererPreference,
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
    /// Show Welcome tab when the application starts (and no startup action opens a session)
    ///
    /// Default `true`. When `false`, the Welcome tab is never shown — neither at
    /// startup nor when all sessions are closed. Users can disable this via
    /// Settings or the "Don't show again" toggle on the Welcome tab itself
    /// (issue #232).
    #[serde(default = "default_true")]
    pub show_welcome_on_startup: bool,
    /// Reveal the floating session toolbar when the pointer touches its handle
    ///
    /// Default `true`, the long-standing behaviour. The handle sits at the top
    /// centre of an embedded RDP/VNC view, so moving the pointer towards the
    /// top of the remote screen — reaching for the remote window's own title
    /// bar, its close button, or a maximised app's menu — opens the toolbar
    /// over exactly the area being aimed at. Set to `false` to require a click
    /// on the handle instead: the handle and every action stay where they are,
    /// only the accidental trigger goes away.
    #[serde(default = "default_true")]
    pub reveal_session_toolbar_on_hover: bool,
    /// Color tab indicators by protocol type
    #[serde(default)]
    pub color_tabs_by_protocol: bool,
    /// Show protocol filter bar in sidebar
    #[serde(default)]
    pub show_protocol_filters: bool,
    /// Show Smart Folders section in sidebar
    #[serde(default)]
    pub show_smart_folders: bool,
    /// Compact interface — denser chrome across the whole window
    ///
    /// Reduces vertical chrome (header bar, tab bar, monitoring bar, banners,
    /// split panel margins, playback toolbar, button padding) so more space is
    /// available for the active session content. Especially useful on small
    /// laptop screens (≤14"), macOS, and KDE Plasma where the default Adwaita
    /// chrome looks taller than native Qt/AppKit apps.
    #[serde(default)]
    pub compact_ui: bool,
    /// Automatically enable compact interface when the window is small.
    ///
    /// When `true`, the `.compact` chrome engages on its own once the window
    /// drops below a size threshold (short and/or narrow), and relaxes again
    /// when the window grows — independent of the manual [`Self::compact_ui`]
    /// switch (which, when on, always forces compact). Off by default so
    /// existing behavior is unchanged.
    #[serde(default)]
    pub compact_auto: bool,
    /// Send single-Ctrl terminal control shortcuts (Ctrl+F/P/N/W/H/M/I) to the
    /// focused terminal/viewer instead of the application accelerators.
    ///
    /// Default `true`: while the terminal or an embedded viewer has focus, the
    /// colliding single-Ctrl accelerators are temporarily suspended so readline
    /// chords reach the shell (issue #197). When `false`, accelerators stay active
    /// (the old behavior).
    #[serde(default = "default_true")]
    pub terminal_passthrough_ctrl: bool,
    /// Show the active connection name in the window title bar.
    ///
    /// Default `false` for privacy: connection names would otherwise be visible
    /// in the taskbar, window list, and screen shares. When `true`, the title
    /// becomes `"RustConn - <active tab>"`, which lets time-tracking tools such
    /// as ManicTime attribute usage per connection by reading the window title
    /// (issue #211).
    #[serde(default)]
    pub window_title_shows_connection: bool,
    /// Make a double-click in the sidebar always start another session.
    ///
    /// Default `false`: a double-click focuses the connection's already-open
    /// session instead of duplicating it (the smart double-click introduced in
    /// 0.18.3), and a new session needs Shift/Ctrl or "Open new session". When
    /// `true`, every double-click launches a new session, as it did before
    /// 0.18.3 — the workflow of users who keep several concurrent sessions per
    /// host (issue #242).
    #[serde(default)]
    pub double_click_opens_new_session: bool,
    /// Show connection name as a compact header on each split-view pane.
    ///
    /// Default `false`. When enabled, a thin colored banner with the connection
    /// name appears at the top of every pane in a split layout, making it easy
    /// to identify which pane belongs to which connection at a glance — useful
    /// with 3+ panes side by side (issue #277).
    #[serde(default)]
    pub show_split_pane_labels: bool,
    /// Remember keyboard passthrough state across restarts.
    ///
    /// Default `false`. When `true` at startup the global passthrough mode
    /// (`win.toggle-passthrough`) is re-engaged, disabling all accelerators
    /// except quit, fullscreen, and the toggle itself. Stored on window close
    /// so the mode survives a restart (issue #274 follow-up).
    #[serde(default)]
    pub keyboard_passthrough: bool,
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
        }
    }
}

/// Default language value (system auto-detect)
fn default_language() -> String {
    "system".to_string()
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            color_scheme: ColorScheme::default(),
            renderer: RendererPreference::default(),
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
            show_welcome_on_startup: true,
            reveal_session_toolbar_on_hover: true,
            color_tabs_by_protocol: false,
            show_protocol_filters: false,
            show_smart_folders: false,
            compact_ui: cfg!(target_os = "macos"),
            compact_auto: false,
            terminal_passthrough_ctrl: true,
            window_title_shows_connection: false,
            double_click_opens_new_session: false,
            show_split_pane_labels: false,
            keyboard_passthrough: false,
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

/// Application-wide bastion settings — the outermost tier of proxy inheritance.
///
/// Resolution order for a connection's bastion is: its own `proxy_jump` /
/// `jump_host_id`, then the group chain (`ConnectionGroup::ssh_proxy_jump` /
/// `ssh_jump_host_id`), then this. A connection with
/// [`NetworkMode::Direct`](crate::models::NetworkMode) stops before the group
/// chain and never reaches here.
///
/// This tier exists because a group cannot express "everything". There is no
/// single implicit root group — `parent_id: None` just means top level, and
/// there can be many — and an ungrouped connection has `group_id: None`, so its
/// chain is empty and it inherits nothing at all (issue
/// [#301](https://github.com/totoshko88/RustConn/issues/301)).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkSettings {
    /// `ProxyJump` applied to every connection that inherits, in OpenSSH syntax
    /// (`user@host`, or several hops comma-separated, client-first).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy_jump: Option<String>,
    /// ID of a saved SSH connection to use as the bastion for every connection
    /// that inherits.
    ///
    /// Offered alongside [`Self::proxy_jump`] rather than instead of it because a
    /// saved connection also carries its port, its identity file and its own
    /// bastion chain — none of which fit in the text field.
    ///
    /// Setting both is not a conflict: the two are resolved independently and end
    /// up as two hops of one chain, exactly as they do at the connection and
    /// group levels. This one is the hop nearer the *client* — the free-text
    /// [`Self::proxy_jump`] is pushed first and
    /// [`JumpChain::hops`](crate::connection::JumpChain::hops) is ordered
    /// target-first — so `ssh -J` contacts this bastion first and reaches the
    /// free-text one through it. See
    /// [`resolve_jump_chain`](crate::connection::resolve_jump_chain).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_host_id: Option<Uuid>,
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

    /// Reads the canonical credential storage choice for the portable
    /// encrypted file passphrase.
    #[must_use]
    pub fn portable_storage(&self) -> CredentialStorage {
        CredentialStorage::from_legacy(
            self.portable_passphrase_encrypted.is_some(),
            self.portable_save_to_keyring,
        )
    }

    /// Writes the canonical credential storage choice for the portable
    /// encrypted file passphrase.
    pub fn set_portable_storage(&mut self, storage: CredentialStorage) {
        match storage {
            CredentialStorage::None => {
                self.portable_save_to_keyring = false;
                self.portable_passphrase_encrypted = None;
            }
            CredentialStorage::EncryptedFile => {
                self.portable_save_to_keyring = false;
            }
            CredentialStorage::SystemKeyring => {
                self.portable_save_to_keyring = true;
                self.portable_passphrase_encrypted = None;
            }
        }
    }

    /// Reports whether `self` carries a runtime secret that `previous` lacks.
    ///
    /// The runtime `SecretString` fields are `#[serde(skip)]` and deliberately
    /// excluded from [`PartialEq`], so a password freshly typed into the
    /// Settings dialog looks like "nothing changed" to the dirty check that
    /// decides whether the save path runs at all. For a keyring-backed backend
    /// that save is the only thing that ever reaches the keyring, so the typed
    /// password would be dropped on dialog close (issue #272).
    ///
    /// Only values that are *newly present or different* count. A field left
    /// `None` is never a change: the dialog intentionally leaves password
    /// entries it could not load empty, and an untouched open/close round trip
    /// must stay a no-op.
    #[must_use]
    pub fn has_new_runtime_secret(&self, previous: &Self) -> bool {
        fn is_new(current: Option<&SecretString>, previous: Option<&SecretString>) -> bool {
            use secrecy::ExposeSecret;
            current.is_some_and(|current| {
                previous.is_none_or(|previous| previous.expose_secret() != current.expose_secret())
            })
        }

        is_new(self.kdbx_password.as_ref(), previous.kdbx_password.as_ref())
            || is_new(
                self.bitwarden_password.as_ref(),
                previous.bitwarden_password.as_ref(),
            )
            || is_new(
                self.bitwarden_client_id.as_ref(),
                previous.bitwarden_client_id.as_ref(),
            )
            || is_new(
                self.bitwarden_client_secret.as_ref(),
                previous.bitwarden_client_secret.as_ref(),
            )
            || is_new(
                self.onepassword_service_account_token.as_ref(),
                previous.onepassword_service_account_token.as_ref(),
            )
            || is_new(
                self.passbolt_passphrase.as_ref(),
                previous.passbolt_passphrase.as_ref(),
            )
            || is_new(
                self.portable_passphrase.as_ref(),
                previous.portable_passphrase.as_ref(),
            )
    }

    /// Applies every backend's storage choice to its persisted blob.
    ///
    /// One rule, identical for all five backends:
    /// [`CredentialStorage::SystemKeyring`] makes the keyring the persistence
    /// layer, so no encrypted blob is written — duplicating the secret on disk
    /// would contradict the user's explicit choice (issue #272);
    /// [`CredentialStorage::EncryptedFile`] encrypts whatever runtime secret was
    /// collected and otherwise leaves the blob already on disk alone;
    /// [`CredentialStorage::None`] clears both copies.
    ///
    /// Meant to run immediately before the settings are written to disk.
    /// Clearing a runtime secret here is safe for the GUI caller: it restores
    /// runtime-only fields from the previous settings afterwards, so the session
    /// keeps working while nothing lands on disk.
    ///
    /// Returns the backends whose requested local encryption failed, so the
    /// caller can say so. An empty result means every "remember this on the
    /// machine" request was honoured. Local encryption fails when no machine key
    /// can be derived, and the user's request is then silently not carried out —
    /// which for the portable store means the key to every credential in the
    /// file is gone at the next start, with nothing on screen to explain it.
    pub fn apply_storage_persistence(&mut self) -> Vec<&'static str> {
        // Read every choice up front — the per-backend steps below mutate the
        // very blobs the `*_storage()` helpers derive their answer from.
        let kdbx = self.kdbx_storage();
        let bitwarden = self.bitwarden_storage();
        let onepassword = self.onepassword_storage();
        let passbolt = self.passbolt_storage();
        let portable = self.portable_storage();

        self.apply_kdbx_persistence(kdbx);
        self.apply_bitwarden_persistence(bitwarden);
        self.apply_onepassword_persistence(onepassword);
        self.apply_passbolt_persistence(passbolt);

        let mut failed = Vec::new();
        if !self.apply_portable_persistence(portable) {
            failed.push("Portable encrypted file");
        }
        failed
    }

    /// KDBX half of [`Self::apply_storage_persistence`].
    fn apply_kdbx_persistence(&mut self, storage: CredentialStorage) {
        if !self.kdbx_enabled {
            self.clear_password();
            return;
        }
        match storage {
            CredentialStorage::None => self.clear_password(),
            CredentialStorage::EncryptedFile => {
                if self.kdbx_password.is_some() {
                    self.encrypt_password();
                }
            }
            CredentialStorage::SystemKeyring => self.kdbx_password_encrypted = None,
        }
    }

    /// Bitwarden half of [`Self::apply_storage_persistence`], covering both the
    /// master password and the API key pair.
    fn apply_bitwarden_persistence(&mut self, storage: CredentialStorage) {
        match storage {
            CredentialStorage::None => self.clear_bitwarden_password(),
            CredentialStorage::EncryptedFile => {
                if self.bitwarden_password.is_some() {
                    self.encrypt_bitwarden_password();
                }
            }
            CredentialStorage::SystemKeyring => self.bitwarden_password_encrypted = None,
        }

        // The API key pair has no selector of its own — "Save master password"
        // is the only storage choice the Bitwarden section offers, so the
        // keyring case follows it and the credentials go to the keyring instead
        // of to disk. `EncryptedFile` and `None` deliberately keep encrypting:
        // an API key is an *alternative* to the master password, and users who
        // authenticate with one while leaving the selector at its "Don't save"
        // default still expect it to survive a restart.
        if self.bitwarden_use_api_key {
            if storage == CredentialStorage::SystemKeyring {
                self.bitwarden_client_id_encrypted = None;
                self.bitwarden_client_secret_encrypted = None;
            } else if self.bitwarden_client_id.is_some() || self.bitwarden_client_secret.is_some() {
                self.encrypt_bitwarden_api_credentials();
            }
        }
    }

    /// 1Password half of [`Self::apply_storage_persistence`].
    fn apply_onepassword_persistence(&mut self, storage: CredentialStorage) {
        match storage {
            CredentialStorage::None => {
                self.onepassword_service_account_token = None;
                self.onepassword_service_account_token_encrypted = None;
            }
            CredentialStorage::EncryptedFile => {
                if self.onepassword_service_account_token.is_some() {
                    self.encrypt_onepassword_token();
                }
            }
            CredentialStorage::SystemKeyring => {
                self.onepassword_service_account_token_encrypted = None;
            }
        }
    }

    /// Passbolt half of [`Self::apply_storage_persistence`].
    fn apply_passbolt_persistence(&mut self, storage: CredentialStorage) {
        match storage {
            CredentialStorage::None => {
                self.passbolt_passphrase = None;
                self.passbolt_passphrase_encrypted = None;
            }
            CredentialStorage::EncryptedFile => {
                if self.passbolt_passphrase.is_some() {
                    self.encrypt_passbolt_passphrase();
                }
            }
            CredentialStorage::SystemKeyring => self.passbolt_passphrase_encrypted = None,
        }
    }

    /// Portable-file half of [`Self::apply_storage_persistence`].
    ///
    /// The passphrase this persists is the key to every credential in the
    /// portable store, so "Don't save" has to clear both copies: leaving a blob
    /// behind after the user asked for no persistence would keep the whole file
    /// openable from disk alone.
    /// Returns `false` if an encrypted copy was requested and could not be
    /// written — the one outcome the caller has to report rather than absorb.
    fn apply_portable_persistence(&mut self, storage: CredentialStorage) -> bool {
        match storage {
            CredentialStorage::None => {
                self.portable_passphrase = None;
                self.portable_passphrase_encrypted = None;
                true
            }
            CredentialStorage::EncryptedFile => {
                if self.portable_passphrase.is_none() {
                    return true;
                }
                if self.encrypt_portable_passphrase() {
                    return true;
                }
                // Drop the placeholder the dialog planted. Leaving it would make
                // `portable_storage()` keep answering "EncryptedFile" for a blob
                // that is not there, so the next start would try to decrypt a
                // placeholder and report a corrupt store rather than a missing
                // one.
                self.portable_passphrase_encrypted = None;
                false
            }
            CredentialStorage::SystemKeyring => {
                self.portable_passphrase_encrypted = None;
                true
            }
        }
    }

    /// Carries runtime secrets the dialog could not collect over from
    /// `previous`, so changing the storage mode alone never strands a secret.
    ///
    /// Only [`CredentialStorage::SystemKeyring`] needs this. There the runtime
    /// `SecretString` is the *only* copy that ever reaches the keyring, and the
    /// dialog leaves a password entry blank whenever the keyring could not
    /// pre-fill it. Migrating "Encrypted file" → "System keyring" without
    /// retyping therefore used to drop the blob from disk *and* write nothing to
    /// the keyring, so the password was gone at the next restart. The
    /// encrypted-file choice keeps its own blob, and "Don't save" is a request
    /// for no persistence at all, so neither is touched.
    pub fn carry_over_runtime_secrets(&mut self, previous: &Self) {
        if self.kdbx_enabled
            && self.kdbx_use_password
            && self.kdbx_storage() == CredentialStorage::SystemKeyring
            && self.kdbx_password.is_none()
        {
            self.kdbx_password = previous.kdbx_password.clone();
        }

        if self.bitwarden_storage() == CredentialStorage::SystemKeyring {
            if self.bitwarden_password.is_none() {
                self.bitwarden_password = previous.bitwarden_password.clone();
            }
            if self.bitwarden_use_api_key {
                if self.bitwarden_client_id.is_none() {
                    self.bitwarden_client_id = previous.bitwarden_client_id.clone();
                }
                if self.bitwarden_client_secret.is_none() {
                    self.bitwarden_client_secret = previous.bitwarden_client_secret.clone();
                }
            }
        }

        if self.onepassword_storage() == CredentialStorage::SystemKeyring
            && self.onepassword_service_account_token.is_none()
        {
            self.onepassword_service_account_token =
                previous.onepassword_service_account_token.clone();
        }

        if self.passbolt_storage() == CredentialStorage::SystemKeyring
            && self.passbolt_passphrase.is_none()
        {
            self.passbolt_passphrase = previous.passbolt_passphrase.clone();
        }

        if self.portable_storage() == CredentialStorage::SystemKeyring
            && self.portable_passphrase.is_none()
        {
            self.portable_passphrase = previous.portable_passphrase.clone();
        }
    }

    /// Reports which system-keyring entries the move from `previous` to `self`
    /// has orphaned.
    ///
    /// Switching a backend away from "System keyring" (or turning the backend
    /// off) used to leave its keyring entry behind forever, so there was no way
    /// to revoke a stored secret — the mirror image of the leak issue #272
    /// fixed. Emptying a password entry deliberately does *not* count: a blank
    /// field means "I did not retype it", not "delete it".
    #[must_use]
    pub fn keyring_revocations(&self, previous: &Self) -> KeyringRevocations {
        let bitwarden_left_keyring = previous.bitwarden_save_to_keyring
            && !(self.bitwarden_save_to_keyring && self.bitwarden_use_api_key);
        KeyringRevocations {
            kdbx_password: previous.kdbx_save_to_keyring
                && !(self.kdbx_save_to_keyring && self.kdbx_enabled && self.kdbx_use_password),
            bitwarden_password: previous.bitwarden_save_to_keyring
                && !self.bitwarden_save_to_keyring,
            bitwarden_api_credentials: previous.bitwarden_use_api_key && bitwarden_left_keyring,
            onepassword_token: previous.onepassword_save_to_keyring
                && !self.onepassword_save_to_keyring,
            passbolt_passphrase: previous.passbolt_save_to_keyring
                && !self.passbolt_save_to_keyring,
            portable_passphrase: previous.portable_save_to_keyring
                && !self.portable_save_to_keyring,
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
    fn get_machine_key() -> zeroize::Zeroizing<Vec<u8>> {
        crate::secret::local_crypto::get_machine_key()
    }

    /// Encrypts the portable file passphrase for local storage using AES-256-GCM.
    ///
    /// The passphrase itself is encrypted with the machine key so it can be
    /// restored without user interaction on this machine. The portable
    /// credential file remains decryptable on other machines because it uses
    /// the passphrase (not the machine key) as input to its own KDF.
    /// Returns whether the encrypted copy was written.
    pub fn encrypt_portable_passphrase(&mut self) -> bool {
        use secrecy::ExposeSecret;
        let Some(ref passphrase) = self.portable_passphrase else {
            return false;
        };
        match encrypt_credential(
            passphrase.expose_secret().as_bytes(),
            &Self::get_machine_key(),
        ) {
            Ok(encrypted) => {
                self.portable_passphrase_encrypted = Some(hex_encode(&encrypted));
                true
            }
            Err(e) => {
                // Logged *and* returned: the log is for the maintainer, the
                // return value is what reaches the user. A warning alone left
                // the next session prompting with nothing on screen to explain
                // why the passphrase it was told to remember was gone.
                tracing::warn!(
                    error = %e,
                    "Could not encrypt the portable file passphrase for local storage"
                );
                false
            }
        }
    }

    /// Decrypts the stored portable file passphrase.
    ///
    /// Returns true if decryption was successful.
    pub fn decrypt_portable_passphrase(&mut self) -> bool {
        if let Some(ref encrypted) = self.portable_passphrase_encrypted
            && let Some(decoded) = hex_decode(encrypted)
        {
            let key = Self::get_machine_key();
            if let Ok(plaintext) = decrypt_credential(&decoded, &key)
                && let Ok(pass_str) = std::str::from_utf8(&plaintext)
            {
                self.portable_passphrase = Some(SecretString::from(pass_str.to_owned()));
                return true;
            }
        }
        false
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

#[cfg(test)]
mod tests {
    use super::{RendererPreference, UiSettings};

    /// A config written before the renderer preference existed must still load,
    /// and must load as `Auto` — the behaviour those users already have.
    #[test]
    fn ui_settings_without_renderer_field_default_to_auto() {
        let older_config = r#"
            color_scheme = "system"
            language = "system"
        "#;

        let settings: UiSettings =
            toml::from_str(older_config).expect("a config predating the field must still parse");

        assert_eq!(settings.renderer, RendererPreference::Auto);
    }

    /// The persisted spelling is part of the config format: renaming a variant
    /// would silently reset the preference of everyone who set it.
    #[test]
    fn renderer_preference_round_trips_through_toml() {
        for (preference, expected) in [
            (RendererPreference::Auto, "auto"),
            (RendererPreference::Gpu, "gpu"),
            (RendererPreference::Software, "software"),
        ] {
            let settings = UiSettings {
                renderer: preference,
                ..Default::default()
            };

            let encoded = toml::to_string(&settings).expect("UiSettings is serializable");
            assert!(
                encoded.contains(&format!("renderer = \"{expected}\"")),
                "expected `renderer = \"{expected}\"` in:\n{encoded}"
            );

            let decoded: UiSettings = toml::from_str(&encoded).expect("round trip");
            assert_eq!(decoded.renderer, preference);
        }
    }

    /// Every variant has a label, and no two share one — the settings combo
    /// maps positions to variants by this list.
    #[test]
    fn every_preference_has_a_distinct_label() {
        let labels: Vec<&str> = [
            RendererPreference::Auto,
            RendererPreference::Gpu,
            RendererPreference::Software,
        ]
        .iter()
        .map(|preference| preference.display_name())
        .collect();

        assert!(labels.iter().all(|label| !label.is_empty()));
        let mut unique = labels.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), labels.len(), "labels must be distinct");
    }

    /// `SecretSettings` has a manual `Debug` because a derived one would print
    /// the `*_encrypted` ciphertexts in the clear. Prove nothing sensitive —
    /// neither a `SecretString` value nor a stored ciphertext — reaches the
    /// output (M-PUBLIC-DEBUG).
    #[test]
    fn secret_settings_debug_does_not_leak() {
        use super::SecretSettings;
        use secrecy::SecretString;

        const RUNTIME_SECRET: &str = "runtime-secret-sentinel";
        const CIPHERTEXT_SENTINEL: &str = "encrypted-ciphertext-sentinel";

        let settings = SecretSettings {
            kdbx_password: Some(SecretString::from(RUNTIME_SECRET)),
            bitwarden_password: Some(SecretString::from(RUNTIME_SECRET)),
            bitwarden_client_id: Some(SecretString::from(RUNTIME_SECRET)),
            bitwarden_client_secret: Some(SecretString::from(RUNTIME_SECRET)),
            onepassword_service_account_token: Some(SecretString::from(RUNTIME_SECRET)),
            passbolt_passphrase: Some(SecretString::from(RUNTIME_SECRET)),
            portable_passphrase: Some(SecretString::from(RUNTIME_SECRET)),
            kdbx_password_encrypted: Some(CIPHERTEXT_SENTINEL.to_string()),
            bitwarden_password_encrypted: Some(CIPHERTEXT_SENTINEL.to_string()),
            bitwarden_client_id_encrypted: Some(CIPHERTEXT_SENTINEL.to_string()),
            bitwarden_client_secret_encrypted: Some(CIPHERTEXT_SENTINEL.to_string()),
            onepassword_service_account_token_encrypted: Some(CIPHERTEXT_SENTINEL.to_string()),
            passbolt_passphrase_encrypted: Some(CIPHERTEXT_SENTINEL.to_string()),
            portable_passphrase_encrypted: Some(CIPHERTEXT_SENTINEL.to_string()),
            ..Default::default()
        };

        let rendered = format!("{settings:?}");

        assert!(
            !rendered.contains(RUNTIME_SECRET),
            "runtime secret leaked into Debug: {rendered}"
        );
        assert!(
            !rendered.contains(CIPHERTEXT_SENTINEL),
            "stored ciphertext leaked into Debug: {rendered}"
        );
        // The presence marker must still be there so a debug dump is useful.
        assert!(
            rendered.contains("<set>"),
            "expected a `<set>` presence marker in: {rendered}"
        );
    }
}
