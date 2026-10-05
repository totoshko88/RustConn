//! Connection model representing a saved remote access configuration.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::custom_property::CustomProperty;
use super::highlight::HighlightRule;
use super::protocol::{
    ProtocolConfig, ProtocolType, RdpClientMode, SshAuthMethod, SshKeySource, VncClientMode,
};
use crate::activity_monitor::ActivityMonitorConfig;
use crate::automation::{ConnectionTask, ExpectRule, KeySequence};
use crate::error::ConfigError;
use crate::monitoring::{MonitoringConfig, MonitoringOverride};
use crate::session::LogConfig;
use crate::variables::Variable;
use crate::variables::{TerminalSubstitution, VariableManager, VariableResult, VariableScope};
use crate::wol::WolConfig;

/// A per-connection command macro: a named command string that can be bound to
/// a keyboard shortcut and sent into the focused terminal of a live session.
///
/// Mirrors asbru-cm's per-connection "exec" entries (`PACExecEntry`). The
/// command is rendered through the variable engine before being sent, so it may
/// contain `${name}` references exactly like other terminal input; a referenced
/// variable whose value is an `@ask:` directive is prompted for by the GUI at
/// fire time. Unlike global snippets (which live in the snippet library and are
/// inserted from the palette), a macro belongs to one connection and fires from
/// its keybind while that connection's terminal has keyboard focus.
///
/// Files written by early 0.23 builds may carry a `prompt_vars` key; it was
/// never read and is ignored on load (this struct does not deny unknown fields).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommandMacro {
    /// Human-readable label shown in the editor.
    pub name: String,
    /// The command text to send. Rendered through the variable engine first.
    pub command: String,
    /// Optional GTK accelerator (e.g. `"<Control><Shift>r"`), pipe-free single
    /// accel. `None` means the macro is stored but not bound to a key, so it
    /// cannot fire. Stored as the same accel string format the keybindings
    /// system already uses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keybind: Option<String>,
    /// Whether to append a newline (Enter) after the command, i.e. run it rather
    /// than just type it. Defaults to true — the common case is "run this".
    #[serde(default = "default_true")]
    pub send_newline: bool,
}

impl Default for CommandMacro {
    /// An empty macro that *runs* its command, agreeing with the serde default
    /// for `send_newline` and the editor switch, which starts on.
    fn default() -> Self {
        Self {
            name: String::new(),
            command: String::new(),
            keybind: None,
            send_newline: true,
        }
    }
}

/// Why a [`CommandMacro`]'s command text cannot be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CommandMacroError {
    /// The command is empty or whitespace only; firing it would send a lone Enter.
    #[error("the macro command is empty")]
    EmptyCommand,
    /// The command contains a control character other than tab (a line break,
    /// ESC, Ctrl+C, …), which would submit or inject input the user never typed.
    #[error("the macro command contains a control character")]
    ControlCharacter,
}

impl CommandMacro {
    /// Whether every field the editor shows is blank — a row the user added and
    /// never filled in, which is dropped on save rather than rejected.
    #[must_use]
    pub fn is_blank(&self) -> bool {
        self.name.trim().is_empty()
            && self.command.trim().is_empty()
            && self.keybind.as_deref().is_none_or(|k| k.trim().is_empty())
    }

    /// Checks that the command text itself is safe to type into a terminal.
    ///
    /// Only tab is allowed among control characters: a literal `\r`/`\n` would
    /// submit a partial line, ESC starts a terminal control sequence and `\x03`
    /// interrupts the remote program. The trailing Enter is the job of
    /// [`Self::send_newline`], never of the text.
    ///
    /// # Errors
    /// Returns [`CommandMacroError::EmptyCommand`] for an empty or
    /// whitespace-only command and [`CommandMacroError::ControlCharacter`] when
    /// it contains a control character other than tab.
    pub fn validate_command(&self) -> Result<(), CommandMacroError> {
        if self.command.trim().is_empty() {
            return Err(CommandMacroError::EmptyCommand);
        }
        if self.command.chars().any(|c| c.is_control() && c != '\t') {
            return Err(CommandMacroError::ControlCharacter);
        }
        Ok(())
    }

    /// Renders the macro's command for sending into a live terminal.
    ///
    /// The command is substituted through the same terminal-input path other
    /// typed input uses (`${name}` / `${ENV_…}` references resolved, secret
    /// values validated and scrubbed on drop). When [`Self::send_newline`] is
    /// set, a carriage return (`\r`) is appended so the command runs rather than
    /// merely being typed — `\r` is what a terminal expects for Enter, matching
    /// the rest of the send paths in this codebase.
    ///
    /// Returns the substitution result (whose `text` is zeroized on drop) and the
    /// list of any `${...}` names that were left unresolved, so the caller can
    /// refuse to send a half-substituted line and name what is missing. Neither
    /// `@ask:` prompting (answers are loaded into `variables` first) nor
    /// [`Self::validate_command`] happens here; the caller does both before
    /// calling this.
    ///
    /// # Errors
    /// Propagates a [`crate::variables::VariableError`] when a reference resolves
    /// to a value rejected for terminal input (e.g. one containing a control
    /// sequence), exactly as `substitute_for_terminal_input` does.
    pub fn render_for_terminal(
        &self,
        variables: &VariableManager,
        scope: VariableScope,
    ) -> VariableResult<TerminalSubstitution> {
        let mut sub = variables.substitute_for_terminal_input(&self.command, scope)?;
        if self.send_newline {
            // Append through a fresh zeroizing buffer so the intermediate is not
            // left unscrubbed; the \r is the Enter a terminal acts on.
            let mut with_cr = zeroize::Zeroizing::new(String::with_capacity(sub.text.len() + 1));
            with_cr.push_str(&sub.text);
            with_cr.push('\r');
            sub.text = with_cr;
        }
        Ok(sub)
    }
}

/// Automation configuration for a connection
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct AutomationConfig {
    /// Expect rules for interactive prompts
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub expect_rules: Vec<ExpectRule>,
    /// Post-login scripts to execute
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub post_login_scripts: Vec<String>,
    /// Expected text of the device's username prompt, for automatic login.
    ///
    /// Matched as a case-insensitive substring of the line under the cursor.
    /// `None` (or blank) uses the built-in matcher, which already covers
    /// `login:`, `Username:` and `>>User name:` (issue #254).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username_prompt: Option<String>,
    /// Expected text of the device's password prompt, for automatic login.
    ///
    /// `None` (or blank) uses the built-in localized password matcher.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_prompt: Option<String>,
    /// How many seconds auto-login waits for a prompt before giving up.
    ///
    /// Defaults to 10 s, which covers a typical SSH handshake or switch banner.
    /// Serial and Telnet connections to network equipment that boots slowly
    /// (Cisco ASR, Huawei MA5800) may need 30–60 s. `None` means "use the
    /// default" so existing configs are unaffected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_timeout_secs: Option<u32>,
    /// Per-connection command macros (named commands, optionally key-bound, sent
    /// into the live terminal). Empty by default; serde-default keeps configs
    /// written before this field loadable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub command_macros: Vec<CommandMacro>,
}

/// Source of password/credentials for a connection
///
/// The `Vault` variant uses whichever secret backend is configured in
/// Settings → Secrets (KeePass, libsecret, Bitwarden, 1Password, Passbolt).
/// Legacy per-backend variants are deserialized as `Vault` for backward
/// compatibility.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PasswordSource {
    /// No password stored
    #[default]
    None,
    /// Password retrieved from the configured secret backend
    /// (replaces KeePass, Keyring, Bitwarden, OnePassword, Passbolt)
    #[serde(
        alias = "kee_pass",
        alias = "keyring",
        alias = "stored",
        alias = "bitwarden",
        alias = "one_password",
        alias = "passbolt"
    )]
    Vault,
    /// Prompt user for password on each connection
    Prompt,
    /// Inherit credentials from parent group
    Inherit,
    /// Password value comes from a named global variable (must be secret)
    Variable(String),
    /// Password retrieved by executing an external command/script
    Script(String),
}

/// Where a connection takes its bastion / proxy configuration from.
///
/// The connection's own `proxy_jump` / `jump_host_id` always win when set; this
/// only decides what happens when they are not, and it is the only way to say
/// "nothing" as opposed to "not configured here" (issue
/// [#301](https://github.com/totoshko88/RustConn/issues/301)).
///
/// Deliberately two variants rather than the four network modes the request
/// listed. *Jump* and *Local Proxy* are already expressible — a `jump_host_id`
/// or a `proxy_jump`, and a `ProxyCommand` or a dynamic (`-D`) forward
/// respectively — so an enum variant for each would duplicate state that
/// already exists and need a migration to keep the two copies agreeing. What
/// could not be expressed is the choice between inheriting and refusing to.
///
/// Lives on [`Connection`] rather than on `SshConfig` so it reads the same for
/// every protocol: RDP, VNC and SPICE carry a `jump_host_id` too, and their
/// "proxy" is an SSH tunnel to it. This mirrors [`PasswordSource`], which is
/// also connection-level and also has an `Inherit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NetworkMode {
    /// Take the bastion from the group chain, then from the global setting.
    ///
    /// The default, and what every connection written before this field existed
    /// deserializes to.
    #[default]
    Inherit,
    /// Connect straight to the host, ignoring any inherited bastion.
    Direct,
}

/// Window mode for connection display
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowMode {
    /// Embedded in main window (default)
    #[default]
    Embedded,
    /// Open in separate external window
    External,
    /// Open in fullscreen mode
    Fullscreen,
}

impl WindowMode {
    /// Returns all available window modes
    #[must_use]
    pub const fn all() -> &'static [Self] {
        &[Self::Embedded, Self::External, Self::Fullscreen]
    }

    /// Returns the display name for this window mode
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Embedded => "Embedded",
            Self::External => "External Window",
            Self::Fullscreen => "Fullscreen",
        }
    }

    /// Returns the index of this window mode in the `all()` array
    #[must_use]
    pub const fn index(&self) -> u32 {
        match self {
            Self::Embedded => 0,
            Self::External => 1,
            Self::Fullscreen => 2,
        }
    }

    /// Creates a window mode from an index
    #[must_use]
    pub const fn from_index(index: u32) -> Self {
        match index {
            1 => Self::External,
            2 => Self::Fullscreen,
            _ => Self::Embedded,
        }
    }
}

/// Window geometry for external windows
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowGeometry {
    /// Window X position
    pub x: i32,
    /// Window Y position
    pub y: i32,
    /// Window width
    pub width: i32,
    /// Window height
    pub height: i32,
}

impl WindowGeometry {
    /// Creates a new window geometry
    #[must_use]
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    /// Creates a default window geometry
    #[must_use]
    pub const fn default_geometry() -> Self {
        Self {
            x: 100,
            y: 100,
            width: 800,
            height: 600,
        }
    }

    /// Returns true if the geometry has valid dimensions
    #[must_use]
    pub const fn is_valid(&self) -> bool {
        self.width > 0 && self.height > 0
    }
}

/// Configuration for a command to pipe terminal output through.
///
/// Wraps the session in a shell pipeline that processes output before it is
/// displayed — `chromaterm` for syntax highlighting, `pv` for bandwidth
/// metering, `ccze` for log colouring. The filter reads the session's stdout on
/// its own stdin and writes to the terminal.
///
/// Only the session's *output* is redirected. In a POSIX pipeline the filter's
/// stdin is the pipe, so the session keeps the terminal for input and typing is
/// unaffected — which is what makes this usable for an interactive shell rather
/// than only for a one-shot command.
///
/// # Example
///
/// ```
/// use rustconn_core::models::PostpendCommand;
///
/// let filter = PostpendCommand {
///     command: "chromaterm".to_string(),
///     args: vec!["--config".to_string(), "/home/u/ct.yml".to_string()],
///     enabled: true,
/// };
/// assert_eq!(
///     filter.filter_argv(),
///     Some(vec![
///         "chromaterm".to_string(),
///         "--config".to_string(),
///         "/home/u/ct.yml".to_string(),
///     ])
/// );
/// ```
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PostpendCommand {
    /// The command to execute (e.g., `chromaterm`, `pv`, `ccze`).
    pub command: String,
    /// Arguments to pass to the command.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    /// Whether the postpend command is active.
    ///
    /// When `false`, the connection runs without piping through this command,
    /// allowing quick toggling without deleting the configuration.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

impl PostpendCommand {
    /// Returns the filter's argv, or `None` when no filter should be applied.
    ///
    /// `None` for a disabled filter and for a blank command, so a caller can
    /// treat "configured but off" and "not configured" identically. A leading
    /// `~/` is expanded in the command and in every argument: the values are
    /// quoted before they reach the shell, so nothing else would expand them,
    /// and a path typed with a tilde is the ordinary case for a filter's config
    /// file.
    #[must_use]
    pub fn filter_argv(&self) -> Option<Vec<String>> {
        if !self.enabled {
            return None;
        }
        let command = self.command.trim();
        if command.is_empty() {
            return None;
        }

        let mut argv = Vec::with_capacity(self.args.len() + 1);
        argv.push(expand_leading_tilde(command));
        argv.extend(self.args.iter().map(|arg| expand_leading_tilde(arg)));
        Some(argv)
    }
}

/// Expands a leading `~/` (or a bare `~`) against `$HOME`, leaving the rest alone.
///
/// `~user` is deliberately not handled: resolving another account's home needs
/// the password database, and a filter argument is not where that belongs.
fn expand_leading_tilde(value: &str) -> String {
    let Some(rest) = value.strip_prefix('~') else {
        return value.to_string();
    };
    if !rest.is_empty() && !rest.starts_with('/') {
        return value.to_string();
    }
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() => format!("{home}{rest}"),
        _ => value.to_string(),
    }
}

/// Per-connection terminal color override.
///
/// Stores optional background, foreground, and cursor colors as CSS hex strings
/// (`#RRGGBB` or `#RRGGBBAA`). When set on a [`Connection`], these override the
/// global terminal theme for that connection only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionThemeOverride {
    /// Background color (`#RRGGBB` or `#RRGGBBAA`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Foreground (text) color (`#RRGGBB` or `#RRGGBBAA`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub foreground: Option<String>,
    /// Cursor color (`#RRGGBB` or `#RRGGBBAA`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
}

impl ConnectionThemeOverride {
    /// Validates that all non-`None` color fields are valid CSS hex colors.
    ///
    /// Accepted formats: `#RRGGBB` (6 hex digits) or `#RRGGBBAA` (8 hex digits).
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::Validation`] if any color value is invalid.
    pub fn validate(&self) -> Result<(), ConfigError> {
        fn is_valid_hex_color(s: &str) -> bool {
            let bytes = s.as_bytes();
            let len = bytes.len();
            (len == 7 || len == 9)
                && bytes[0] == b'#'
                && bytes[1..].iter().all(u8::is_ascii_hexdigit)
        }

        for (field, value) in [
            ("background", &self.background),
            ("foreground", &self.foreground),
            ("cursor", &self.cursor),
        ] {
            if let Some(color) = value
                && !is_valid_hex_color(color)
            {
                return Err(ConfigError::Validation {
                    field: field.to_string(),
                    reason: format!("Invalid color value '{color}': expected #RRGGBB or #RRGGBBAA"),
                });
            }
        }
        Ok(())
    }

    /// Returns `true` if all color fields are `None`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.background.is_none() && self.foreground.is_none() && self.cursor.is_none()
    }
}

/// A saved remote connection configuration
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)]
pub struct Connection {
    /// Unique identifier for the connection
    pub id: Uuid,
    /// Human-readable name for the connection
    pub name: String,
    /// Optional description for the connection
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Protocol type (SSH, RDP, VNC)
    pub protocol: ProtocolType,
    /// Remote host address (hostname or IP)
    pub host: String,
    /// Remote port number
    pub port: u16,
    /// Username for authentication
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    /// Group this connection belongs to
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group_id: Option<Uuid>,
    /// Tags for organization and filtering
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Timestamp when the connection was created
    pub created_at: DateTime<Utc>,
    /// Timestamp when the connection was last modified
    pub updated_at: DateTime<Utc>,
    /// Protocol-specific configuration
    pub protocol_config: ProtocolConfig,
    /// Automation configuration
    #[serde(default)]
    pub automation: AutomationConfig,
    /// Sort order for manual ordering (lower values appear first)
    #[serde(default)]
    pub sort_order: i32,
    /// Timestamp when the connection was last used
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_connected: Option<DateTime<Utc>>,
    /// Source of password for this connection
    #[serde(default)]
    pub password_source: PasswordSource,
    /// Where this connection takes its bastion / proxy configuration from.
    ///
    /// `Inherit` (the default) walks the group chain and then the global
    /// network settings; `Direct` refuses an inherited bastion. A `proxy_jump`
    /// or `jump_host_id` set on the connection itself outranks both.
    #[serde(default)]
    pub network_mode: NetworkMode,
    /// Domain for RDP/Windows authentication
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// Custom properties for additional metadata
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub custom_properties: Vec<CustomProperty>,
    /// Pre-connect task to execute before establishing the connection
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_connect_task: Option<ConnectionTask>,
    /// Post-disconnect task to execute after the connection is terminated
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub post_disconnect_task: Option<ConnectionTask>,
    /// Wake On LAN configuration for waking sleeping machines
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wol_config: Option<WolConfig>,
    /// Local variables that override global variables for this connection
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub local_variables: HashMap<String, Variable>,
    /// Session logging configuration
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_config: Option<LogConfig>,
    /// Key sequence to send after connection is established
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_sequence: Option<KeySequence>,
    /// Window mode for connection display (embedded, external, fullscreen)
    #[serde(default)]
    pub window_mode: WindowMode,
    /// Whether to remember window position for external windows
    #[serde(default)]
    pub remember_window_position: bool,
    /// Saved window geometry for external windows
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub window_geometry: Option<WindowGeometry>,
    /// Skip pre-connect port check for this connection (overrides global setting)
    #[serde(default)]
    pub skip_port_check: bool,
    /// Whether this connection is pinned to favorites
    #[serde(default)]
    pub is_pinned: bool,
    /// Sort order within pinned connections (lower values appear first)
    #[serde(default)]
    pub pin_order: i32,
    /// Custom icon for the connection (emoji/unicode character or GTK icon name)
    ///
    /// When `None`, the default protocol-based icon is used.
    /// Examples: `"🇺🇦"`, `"🏢"`, `"starred-symbolic"`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Per-connection remote monitoring override
    ///
    /// When `None`, the global `MonitoringSettings` from `AppSettings` apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub monitoring_config: Option<MonitoringConfig>,
    /// Per-connection activity monitor override
    ///
    /// When `None`, the global `ActivityMonitorDefaults` from `AppSettings` apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activity_monitor_config: Option<ActivityMonitorConfig>,
    /// Per-connection terminal theme override
    ///
    /// When `None`, the global terminal theme settings apply.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub theme_override: Option<ConnectionThemeOverride>,
    /// Whether session recording is enabled for this connection
    #[serde(default)]
    pub session_recording_enabled: bool,
    /// Per-connection highlight rules for regex-based text highlighting
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlight_rules: Vec<HighlightRule>,
    /// Whether this connection was generated by a dynamic folder script.
    /// Dynamic connections are read-only and regenerated on refresh.
    #[serde(default)]
    pub is_dynamic: bool,
    /// Retry configuration for automatic reconnection on failure.
    ///
    /// When `None`, auto-reconnect uses the default polling behavior.
    /// When `Some`, the configured retry policy (max attempts, backoff) is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retry_config: Option<crate::connection::RetryConfig>,
    /// Port knock sequence to execute before connecting
    ///
    /// Sends TCP SYN / UDP packets to open a firewall before the real connection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub knock_sequence: Option<crate::connection::knock::KnockSequence>,
    /// fwknop Single Packet Authorization configuration
    ///
    /// Sends an encrypted UDP packet to open a firewall rule for this client.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spa_config: Option<crate::connection::knock::SpaConfig>,
    /// Command to pipe terminal output through (e.g., ChromaTerm for syntax highlighting).
    ///
    /// When set and enabled, the terminal session is wrapped in a pipeline where
    /// the session's stdout is piped through this command before display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub postpend: Option<PostpendCommand>,
    /// The id this connection has on the Group Sync Master it was imported
    /// from.
    ///
    /// Set only on connections inside a Group Sync Import tree. Import matches
    /// a remote connection on it first, so a rename or move on the Master
    /// updates this connection in place — same `id`, same vault link (issue
    /// #263) — instead of deleting and recreating it. It is a reference, not
    /// an identity: the connection keeps an `id` of its own, because the
    /// Master's connection can exist on the same device (a Master importing
    /// its own file, or a Simple Sync peer) and two entities must never share
    /// one `id`. Absent in files written before 0.23, which load with `None`
    /// and are linked on the next sync.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_origin_id: Option<Uuid>,
}

impl Connection {
    /// Whether the user should be told that no stored password was found.
    ///
    /// Answers a presentation question, not an authentication one: nothing reads
    /// this to decide whether to look a credential up or which one to use. It
    /// exists because "no vault entry, you will be prompted for a password" is
    /// false and alarming for a connection that authenticates with a key — `ssh`
    /// prompts for a key passphrase there, if anything, never for the account
    /// password — and the reporter of issue
    /// [#307](https://github.com/totoshko88/RustConn/issues/307) saw that notice
    /// before every single connection.
    ///
    /// Deliberately keyed on the *key* configuration rather than on
    /// `auth_method` alone. [`SshAuthMethod`] defaults to `Password`, so a
    /// connection imported from an `ssh_config` or created without touching that
    /// dropdown reads as password auth however it actually connects; a check on
    /// `auth_method` by itself would therefore keep showing the notice to
    /// precisely the users who complained about it. A key path or an agent key
    /// source is the stronger signal, so any of the three is enough.
    ///
    /// Errs towards showing the notice: an unknown or half-configured protocol
    /// returns `true`, because a missing password the user does need to know
    /// about costs a failed connection, while one they do not costs a toast.
    ///
    /// [`SshAuthMethod`]: crate::models::SshAuthMethod
    #[must_use]
    pub fn expects_password_prompt(&self) -> bool {
        let ssh = match &self.protocol_config {
            ProtocolConfig::Ssh(cfg) | ProtocolConfig::Sftp(cfg) => cfg,
            // A Web bookmark never prompts for an account password the way a
            // shell or remote-desktop session does: the browser (embedded or
            // external) collects any credentials the page itself asks for, and a
            // configured SOCKS tunnel authenticates to its *jump host*, not to
            // the site. So "no vault entry, you will be prompted for a password"
            // is meaningless here — and, when the tunnel's bastion is down, it
            // fires before the connection has even failed. Never announce it for
            // Web.
            ProtocolConfig::Web(_) => return false,
            _ => return true,
        };

        let key_configured = ssh.key_path.is_some()
            || matches!(
                ssh.key_source,
                SshKeySource::File { .. } | SshKeySource::Agent { .. }
            )
            || matches!(
                ssh.auth_method,
                SshAuthMethod::PublicKey | SshAuthMethod::Agent | SshAuthMethod::SecurityKey
            );

        !key_configured
    }

    /// Creates a new connection with the given parameters
    #[must_use]
    pub fn new(name: String, host: String, port: u16, protocol_config: ProtocolConfig) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            description: None,
            protocol: protocol_config.protocol_type(),
            host,
            port,
            username: None,
            group_id: None,
            tags: Vec::new(),
            created_at: now,
            updated_at: now,
            protocol_config,
            sort_order: 0,
            last_connected: None,
            password_source: PasswordSource::None,
            network_mode: NetworkMode::default(),
            domain: None,
            custom_properties: Vec::new(),
            pre_connect_task: None,
            post_disconnect_task: None,
            wol_config: None,
            local_variables: HashMap::new(),
            log_config: None,
            key_sequence: None,
            automation: AutomationConfig::default(),
            window_mode: WindowMode::default(),
            remember_window_position: false,
            window_geometry: None,
            skip_port_check: false,
            is_pinned: false,
            pin_order: 0,
            icon: None,
            monitoring_config: None,
            activity_monitor_config: None,
            theme_override: None,
            session_recording_enabled: false,
            highlight_rules: Vec::new(),
            is_dynamic: false,
            retry_config: None,
            knock_sequence: None,
            spa_config: None,
            postpend: None,
            sync_origin_id: None,
        }
    }

    /// Creates a new SSH connection with default configuration
    #[must_use]
    pub fn new_ssh(name: String, host: String, port: u16) -> Self {
        Self::new(
            name,
            host,
            port,
            ProtocolConfig::Ssh(super::protocol::SshConfig::default()),
        )
    }

    /// Creates a new RDP connection with default configuration
    #[must_use]
    pub fn new_rdp(name: String, host: String, port: u16) -> Self {
        Self::new(
            name,
            host,
            port,
            ProtocolConfig::Rdp(super::protocol::RdpConfig::default()),
        )
    }

    /// Creates a new VNC connection with default configuration
    #[must_use]
    pub fn new_vnc(name: String, host: String, port: u16) -> Self {
        Self::new(
            name,
            host,
            port,
            ProtocolConfig::Vnc(super::protocol::VncConfig::default()),
        )
    }

    /// Creates a new SPICE connection with default configuration
    #[must_use]
    pub fn new_spice(name: String, host: String, port: u16) -> Self {
        Self::new(
            name,
            host,
            port,
            ProtocolConfig::Spice(super::protocol::SpiceConfig::default()),
        )
    }

    /// Creates a new Telnet connection with default settings
    #[must_use]
    pub fn new_telnet(name: String, host: String, port: u16) -> Self {
        Self::new(
            name,
            host,
            port,
            ProtocolConfig::Telnet(super::protocol::TelnetConfig::default()),
        )
    }

    /// Creates a new Serial connection with default settings
    #[must_use]
    pub fn new_serial(name: String, device: String) -> Self {
        let config = super::protocol::SerialConfig {
            device,
            ..Default::default()
        };
        Self::new(name, String::new(), 0, ProtocolConfig::Serial(config))
    }

    /// Creates a new SFTP connection with default SSH config
    #[must_use]
    pub fn new_sftp(name: String, host: String, port: u16) -> Self {
        Self::new(
            name,
            host,
            port,
            ProtocolConfig::Sftp(super::protocol::SshConfig::default()),
        )
    }

    /// Creates a new Kubernetes connection with default config
    #[must_use]
    pub fn new_kubernetes(name: String) -> Self {
        Self::new(
            name,
            String::new(),
            0,
            ProtocolConfig::Kubernetes(super::protocol::KubernetesConfig::default()),
        )
    }

    /// Creates a new MOSH connection with default config
    #[must_use]
    pub fn new_mosh(name: String, host: String, port: u16) -> Self {
        Self::new(
            name,
            host,
            port,
            ProtocolConfig::Mosh(super::protocol::MoshConfig::default()),
        )
    }

    /// Sets the username for this connection
    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Sets the group for this connection
    #[must_use]
    pub const fn with_group(mut self, group_id: Uuid) -> Self {
        self.group_id = Some(group_id);
        self
    }

    /// Adds tags to this connection
    #[must_use]
    pub fn with_tags(mut self, tags: Vec<String>) -> Self {
        self.tags = tags;
        self
    }

    /// Sets the description for this connection
    #[must_use]
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Updates the `updated_at` timestamp to now
    pub fn touch(&mut self) {
        self.updated_at = Utc::now();
    }

    /// Reads whether this connection follows the global monitoring switch.
    ///
    /// The three-way view of `monitoring_config.enabled`; see
    /// [`MonitoringOverride::from_config`].
    #[must_use]
    pub fn monitoring_override(&self) -> MonitoringOverride {
        MonitoringOverride::from_config(self.monitoring_config.as_ref())
    }

    /// Makes this connection follow the global monitoring switch.
    ///
    /// Clears the connection's own on/off value and keeps an interval override;
    /// a connection left with nothing to override stores no monitoring config at
    /// all. Returns whether anything changed, and touches `updated_at` only then.
    pub fn reset_monitoring_override(&mut self) -> bool {
        if self.monitoring_override() == MonitoringOverride::Inherit {
            return false;
        }
        let kept = MonitoringOverride::Inherit.apply(self.monitoring_config.as_ref());
        self.monitoring_config = kept;
        self.touch();
        true
    }

    /// Returns the default port for this connection's protocol
    #[must_use]
    pub const fn default_port(&self) -> u16 {
        self.protocol.default_port()
    }

    /// Gets a custom property by name
    ///
    /// # Arguments
    /// * `name` - The name of the property to retrieve
    ///
    /// # Returns
    /// A reference to the property if found, `None` otherwise
    #[must_use]
    pub fn get_custom_property(&self, name: &str) -> Option<&CustomProperty> {
        self.custom_properties.iter().find(|p| p.name == name)
    }

    /// Gets a mutable reference to a custom property by name
    ///
    /// # Arguments
    /// * `name` - The name of the property to retrieve
    ///
    /// # Returns
    /// A mutable reference to the property if found, `None` otherwise
    #[must_use]
    pub fn get_custom_property_mut(&mut self, name: &str) -> Option<&mut CustomProperty> {
        self.custom_properties.iter_mut().find(|p| p.name == name)
    }

    /// Sets a custom property, replacing any existing property with the same name
    ///
    /// # Arguments
    /// * `property` - The property to set
    pub fn set_custom_property(&mut self, property: CustomProperty) {
        if let Some(existing) = self.get_custom_property_mut(&property.name) {
            *existing = property;
        } else {
            self.custom_properties.push(property);
        }
        self.touch();
    }

    /// Removes a custom property by name
    ///
    /// # Arguments
    /// * `name` - The name of the property to remove
    ///
    /// # Returns
    /// `true` if a property was removed, `false` otherwise
    pub fn remove_custom_property(&mut self, name: &str) -> bool {
        let len_before = self.custom_properties.len();
        self.custom_properties.retain(|p| p.name != name);
        let removed = self.custom_properties.len() < len_before;
        if removed {
            self.touch();
        }
        removed
    }

    /// Adds custom properties to this connection (builder pattern)
    #[must_use]
    pub fn with_custom_properties(mut self, properties: Vec<CustomProperty>) -> Self {
        self.custom_properties = properties;
        self
    }

    /// Sets the pre-connect task for this connection
    #[must_use]
    pub fn with_pre_connect_task(mut self, task: ConnectionTask) -> Self {
        self.pre_connect_task = Some(task);
        self
    }

    /// Sets the post-disconnect task for this connection
    #[must_use]
    pub fn with_post_disconnect_task(mut self, task: ConnectionTask) -> Self {
        self.post_disconnect_task = Some(task);
        self
    }

    /// Returns true if this connection has a pre-connect task
    #[must_use]
    pub const fn has_pre_connect_task(&self) -> bool {
        self.pre_connect_task.is_some()
    }

    /// Returns true if this connection has a post-disconnect task
    #[must_use]
    pub const fn has_post_disconnect_task(&self) -> bool {
        self.post_disconnect_task.is_some()
    }

    /// Sets the Wake On LAN configuration for this connection
    #[must_use]
    pub fn with_wol_config(mut self, config: WolConfig) -> Self {
        self.wol_config = Some(config);
        self
    }

    /// Returns true if this connection has Wake On LAN configured
    #[must_use]
    pub const fn has_wol_config(&self) -> bool {
        self.wol_config.is_some()
    }

    /// Gets a reference to the WOL configuration if present
    #[must_use]
    pub const fn get_wol_config(&self) -> Option<&WolConfig> {
        self.wol_config.as_ref()
    }

    /// Sets the WOL configuration, updating the timestamp
    pub fn set_wol_config(&mut self, config: Option<WolConfig>) {
        self.wol_config = config;
        self.touch();
    }

    /// Gets a local variable by name
    ///
    /// # Arguments
    /// * `name` - The name of the variable to retrieve
    ///
    /// # Returns
    /// A reference to the variable if found, `None` otherwise
    #[must_use]
    pub fn get_local_variable(&self, name: &str) -> Option<&Variable> {
        self.local_variables.get(name)
    }

    /// Sets a local variable, replacing any existing variable with the same name
    ///
    /// # Arguments
    /// * `variable` - The variable to set
    pub fn set_local_variable(&mut self, variable: Variable) {
        self.local_variables.insert(variable.name.clone(), variable);
        self.touch();
    }

    /// Removes a local variable by name
    ///
    /// # Arguments
    /// * `name` - The name of the variable to remove
    ///
    /// # Returns
    /// The removed variable if it existed, `None` otherwise
    pub fn remove_local_variable(&mut self, name: &str) -> Option<Variable> {
        let removed = self.local_variables.remove(name);
        if removed.is_some() {
            self.touch();
        }
        removed
    }

    /// Returns true if this connection has local variables
    #[must_use]
    pub fn has_local_variables(&self) -> bool {
        !self.local_variables.is_empty()
    }

    /// Sets local variables for this connection (builder pattern)
    #[must_use]
    pub fn with_local_variables(mut self, variables: HashMap<String, Variable>) -> Self {
        self.local_variables = variables;
        self
    }

    /// Sets the session logging configuration for this connection
    #[must_use]
    pub fn with_log_config(mut self, config: LogConfig) -> Self {
        self.log_config = Some(config);
        self
    }

    /// Returns true if this connection has session logging configured
    #[must_use]
    pub const fn has_log_config(&self) -> bool {
        self.log_config.is_some()
    }

    /// Gets a reference to the log configuration if present
    #[must_use]
    pub const fn get_log_config(&self) -> Option<&LogConfig> {
        self.log_config.as_ref()
    }

    /// Sets the log configuration, updating the timestamp
    pub fn set_log_config(&mut self, config: Option<LogConfig>) {
        self.log_config = config;
        self.touch();
    }

    /// Returns true if session logging is enabled for this connection
    #[must_use]
    pub fn is_logging_enabled(&self) -> bool {
        self.log_config.as_ref().is_some_and(|c| c.enabled)
    }

    /// Sets the key sequence for this connection
    #[must_use]
    pub fn with_key_sequence(mut self, sequence: KeySequence) -> Self {
        self.key_sequence = Some(sequence);
        self
    }

    /// Returns true if this connection has a key sequence configured
    #[must_use]
    pub const fn has_key_sequence(&self) -> bool {
        self.key_sequence.is_some()
    }

    /// Gets a reference to the key sequence if present
    #[must_use]
    pub const fn get_key_sequence(&self) -> Option<&KeySequence> {
        self.key_sequence.as_ref()
    }

    /// Sets the key sequence, updating the timestamp
    pub fn set_key_sequence(&mut self, sequence: Option<KeySequence>) {
        self.key_sequence = sequence;
        self.touch();
    }

    /// Sets the expect rules for this connection
    #[must_use]
    pub fn with_expect_rules(mut self, rules: Vec<ExpectRule>) -> Self {
        self.automation.expect_rules = rules;
        self
    }

    /// Returns true if this connection has expect rules configured
    #[must_use]
    pub fn has_expect_rules(&self) -> bool {
        !self.automation.expect_rules.is_empty()
    }

    /// Gets a reference to the expect rules
    #[must_use]
    pub fn get_expect_rules(&self) -> &[ExpectRule] {
        &self.automation.expect_rules
    }

    /// Adds an expect rule to this connection
    pub fn add_expect_rule(&mut self, rule: ExpectRule) {
        self.automation.expect_rules.push(rule);
        self.touch();
    }

    /// Removes an expect rule by ID
    ///
    /// # Returns
    /// `true` if a rule was removed, `false` otherwise
    pub fn remove_expect_rule(&mut self, id: uuid::Uuid) -> bool {
        let len_before = self.automation.expect_rules.len();
        self.automation.expect_rules.retain(|r| r.id != id);
        let removed = self.automation.expect_rules.len() < len_before;
        if removed {
            self.touch();
        }
        removed
    }

    /// Sets the expect rules, updating the timestamp
    pub fn set_expect_rules(&mut self, rules: Vec<ExpectRule>) {
        self.automation.expect_rules = rules;
        self.touch();
    }

    /// Sets the window mode for this connection
    #[must_use]
    pub const fn with_window_mode(mut self, mode: WindowMode) -> Self {
        self.window_mode = mode;
        self
    }

    /// Gets the window mode for this connection
    #[must_use]
    pub const fn get_window_mode(&self) -> WindowMode {
        self.window_mode
    }

    /// Sets the window mode, updating the timestamp
    pub fn set_window_mode(&mut self, mode: WindowMode) {
        self.window_mode = mode;
        self.touch();
    }

    /// Returns true if this connection should open in an external window
    #[must_use]
    pub const fn is_external_window(&self) -> bool {
        matches!(self.window_mode, WindowMode::External)
    }

    /// Returns true if this connection should open in fullscreen mode
    #[must_use]
    pub const fn is_fullscreen(&self) -> bool {
        matches!(self.window_mode, WindowMode::Fullscreen)
    }

    /// Returns true if this connection's protocol honours `window_mode`.
    ///
    /// Only RDP and VNC honour the setting; for every other protocol the value
    /// is ignored at connect time. SPICE is deliberately excluded: it always
    /// uses an external viewer (see [`uses_external_viewer`](Self::uses_external_viewer)),
    /// so `window_mode` has no observable effect on a SPICE connection.
    #[must_use]
    pub const fn supports_window_mode(&self) -> bool {
        matches!(self.protocol, ProtocolType::Rdp | ProtocolType::Vnc)
    }

    /// Returns `true` when the display is fully delegated to an external viewer.
    ///
    /// Such connections render in a separate operating-system process (TigerVNC,
    /// xfreerdp, remote-viewer) and get no embedded notebook tab. SPICE is always
    /// external (the embedded SPICE client was removed in 0.18.0); VNC and RDP are
    /// external when either `window_mode == External` or the protocol
    /// `client_mode == External`. All other protocols are never external.
    ///
    /// The result is pure and deterministic: the same inputs always yield the same
    /// value, with no hidden state or I/O.
    #[must_use]
    pub fn uses_external_viewer(&self) -> bool {
        match &self.protocol_config {
            ProtocolConfig::Spice(_) => true,
            ProtocolConfig::Vnc(c) => {
                self.window_mode == WindowMode::External || c.client_mode == VncClientMode::External
            }
            ProtocolConfig::Rdp(c) => {
                self.window_mode == WindowMode::External || c.client_mode == RdpClientMode::External
            }
            _ => false,
        }
    }

    /// Sets whether to remember window position for external windows
    #[must_use]
    pub const fn with_remember_window_position(mut self, remember: bool) -> Self {
        self.remember_window_position = remember;
        self
    }

    /// Gets whether to remember window position
    #[must_use]
    pub const fn should_remember_window_position(&self) -> bool {
        self.remember_window_position
    }

    /// Sets remember window position, updating the timestamp
    pub fn set_remember_window_position(&mut self, remember: bool) {
        self.remember_window_position = remember;
        self.touch();
    }

    /// Sets the window geometry for this connection
    #[must_use]
    pub const fn with_window_geometry(mut self, geometry: WindowGeometry) -> Self {
        self.window_geometry = Some(geometry);
        self
    }

    /// Gets the window geometry if set
    #[must_use]
    pub const fn get_window_geometry(&self) -> Option<&WindowGeometry> {
        self.window_geometry.as_ref()
    }

    /// Sets the window geometry, updating the timestamp
    pub fn set_window_geometry(&mut self, geometry: Option<WindowGeometry>) {
        self.window_geometry = geometry;
        self.touch();
    }

    /// Updates the window geometry from current window state
    pub fn update_window_geometry(&mut self, x: i32, y: i32, width: i32, height: i32) {
        if self.remember_window_position {
            self.window_geometry = Some(WindowGeometry::new(x, y, width, height));
            self.touch();
        }
    }

    /// Returns `true` if this connection bypasses a direct TCP probe.
    ///
    /// Connections routed through a jump host, RDP Gateway, SSH ProxyCommand,
    /// or SPICE proxy are not directly reachable, so a pre-connect port check
    /// would always time out.
    ///
    /// A bastion set as free text in `proxy_jump` counts as much as one picked
    /// from the connection list: both mean the target is not reachable directly.
    #[must_use]
    pub fn bypasses_direct_probe(&self) -> bool {
        match &self.protocol_config {
            ProtocolConfig::Ssh(c) | ProtocolConfig::Sftp(c) => {
                c.jump_host_id.is_some()
                    || c.proxy_command.is_some()
                    || c.proxy_jump
                        .as_deref()
                        .is_some_and(|p| !p.trim().is_empty())
            }
            ProtocolConfig::Rdp(c) => c.jump_host_id.is_some() || c.gateway.is_some(),
            ProtocolConfig::Vnc(c) => c.jump_host_id.is_some(),
            ProtocolConfig::Spice(c) => c.jump_host_id.is_some() || c.proxy.is_some(),
            ProtocolConfig::ZeroTrust(_) | ProtocolConfig::Web(_) => true,
            _ => false,
        }
    }

    /// Returns `true` if a pre-connect TCP port check should be performed.
    ///
    /// Checks the global setting, per-connection override, and whether the
    /// connection bypasses direct probing (jump host, gateway, proxy, etc.).
    #[must_use]
    pub fn should_pre_connect_check(&self, settings: &crate::config::ConnectionSettings) -> bool {
        settings.pre_connect_port_check && !self.skip_port_check && !self.bypasses_direct_probe()
    }

    /// Toggles the pinned state of this connection
    pub fn toggle_pin(&mut self) {
        self.is_pinned = !self.is_pinned;
        if !self.is_pinned {
            self.pin_order = 0;
        }
        self.touch();
    }

    /// Sets the pinned state and order
    pub fn set_pinned(&mut self, pinned: bool, order: i32) {
        self.is_pinned = pinned;
        self.pin_order = order;
        self.touch();
    }

    /// Returns a copy of this connection under a new id and `name`.
    ///
    /// The copy keeps every setting, but not what belongs to the original as an
    /// entry the user chose or used: it is not a favorite (`is_pinned`,
    /// `pin_order`) and has never been connected (`last_connected`). Favorites
    /// are a per-connection choice, so a duplicate of a favorite landing in
    /// Favorites on its own was wrong. Duplicate, paste and the CLI's
    /// `duplicate` all go through here so they cannot drift apart again.
    ///
    /// The copy is also never dynamic (`is_dynamic`): a dynamic folder's
    /// refresh deletes every dynamic connection under it, so a duplicate made
    /// to customise a generated entry would vanish on the next refresh. Nor
    /// does it keep `sync_origin_id`: it is not the copy of the Master's
    /// connection that a Group Sync Import matches.
    #[must_use]
    pub fn duplicate_as(&self, name: String) -> Self {
        let now = Utc::now();
        Self {
            id: Uuid::new_v4(),
            name,
            created_at: now,
            updated_at: now,
            last_connected: None,
            is_pinned: false,
            pin_order: 0,
            is_dynamic: false,
            sync_origin_id: None,
            ..self.clone()
        }
    }
}

/// Helper for serde defaults.
const fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::custom_property::PropertyType;

    fn create_test_connection() -> Connection {
        Connection::new_ssh("Test Server".to_string(), "example.com".to_string(), 22)
    }

    /// A duplicate of a favorite must not be a favorite itself.
    #[test]
    fn duplicate_is_not_a_favorite_and_never_connected() {
        let mut original = create_test_connection();
        original.set_pinned(true, 3);
        original.last_connected = Some(Utc::now());

        let copy = original.duplicate_as("Test Server (copy)".to_string());

        assert!(!copy.is_pinned, "a copy of a favorite must not be pinned");
        assert_eq!(copy.pin_order, 0);
        assert_eq!(copy.last_connected, None);
        assert!(original.is_pinned, "the original stays a favorite");
        assert_eq!(original.pin_order, 3);
    }

    /// The copy keeps the settings and gets an identity of its own.
    #[test]
    fn duplicate_keeps_settings_under_a_new_identity() {
        let mut original = create_test_connection();
        original.group_id = Some(Uuid::new_v4());
        original.username = Some("admin".to_string());
        original.tags = vec!["prod".to_string()];

        let copy = original.duplicate_as("Copy".to_string());

        assert_ne!(copy.id, original.id);
        assert_eq!(copy.name, "Copy");
        assert_eq!(copy.host, original.host);
        assert_eq!(copy.port, original.port);
        assert_eq!(copy.username, original.username);
        assert_eq!(copy.group_id, original.group_id);
        assert_eq!(copy.tags, original.tags);
        assert_eq!(copy.protocol_config, original.protocol_config);
    }

    /// A duplicate of a dynamic-folder entry survives the folder's refresh.
    #[test]
    fn duplicate_of_a_dynamic_connection_is_not_dynamic() {
        let mut original = create_test_connection();
        original.is_dynamic = true;

        let copy = original.duplicate_as("Copy".to_string());

        assert!(!copy.is_dynamic);
        assert!(original.is_dynamic);
    }

    /// Returns the SSH config of a test connection for mutation.
    fn ssh_of(conn: &mut Connection) -> &mut super::super::protocol::SshConfig {
        match &mut conn.protocol_config {
            ProtocolConfig::Ssh(cfg) => cfg,
            _ => unreachable!("create_test_connection builds an SSH connection"),
        }
    }

    /// The `~/` expansion an output filter's config path relies on. Previously
    /// untested, including the two cases it deliberately refuses.
    mod tilde {
        use super::super::expand_leading_tilde;

        /// `$HOME` is process-global, so these assertions are grouped into one
        /// test rather than racing each other across parallel test threads.
        #[test]
        fn a_leading_tilde_expands_and_nothing_else_does() {
            // A read of the real environment, never a write: whatever HOME is, the
            // expected value is expressed relative to it.
            let home = std::env::var("HOME").unwrap_or_default();
            assert!(
                !home.is_empty(),
                "HOME must be set for this test to mean anything"
            );

            assert_eq!(expand_leading_tilde("~/ct.yml"), format!("{home}/ct.yml"));
            assert_eq!(expand_leading_tilde("~"), home);

            // Another account's home needs the password database; refused.
            assert_eq!(expand_leading_tilde("~root/x"), "~root/x");
            // Only a *leading* tilde is a home reference.
            assert_eq!(
                expand_leading_tilde("--config=~/ct.yml"),
                "--config=~/ct.yml"
            );
            assert_eq!(expand_leading_tilde("/abs/path"), "/abs/path");
            assert_eq!(expand_leading_tilde("chromaterm"), "chromaterm");
            assert_eq!(expand_leading_tilde(""), "");
        }
    }

    /// A disabled or blank filter is indistinguishable from no filter, and a
    /// non-empty `argv[0]` is the invariant the spawn path indexes on.
    #[test]
    fn filter_argv_refuses_disabled_and_blank_commands() {
        let disabled = PostpendCommand {
            command: "ccze".to_string(),
            args: vec![],
            enabled: false,
        };
        assert_eq!(disabled.filter_argv(), None);

        let blank = PostpendCommand {
            command: "   ".to_string(),
            args: vec![],
            enabled: true,
        };
        assert_eq!(blank.filter_argv(), None, "a blank command is not a filter");

        let real = PostpendCommand {
            command: "  ccze  ".to_string(),
            args: vec!["-A".to_string()],
            enabled: true,
        };
        let argv = real.filter_argv().expect("an enabled filter yields argv");
        assert_eq!(argv, vec!["ccze".to_string(), "-A".to_string()]);
        assert!(!argv[0].is_empty(), "argv[0] must never be empty");
    }

    #[test]
    fn a_plain_ssh_connection_expects_a_password_prompt() {
        // Nothing configured: SshAuthMethod defaults to Password, and there is
        // no key. The notice is correct here.
        let conn = create_test_connection();
        assert!(conn.expects_password_prompt());
    }

    #[test]
    fn a_key_path_alone_suppresses_the_prompt_notice() {
        // The case the issue reporter is in: a key is configured but the auth
        // method was never touched, so it still reads as Password. Keying the
        // check on auth_method alone would keep showing the notice.
        let mut conn = create_test_connection();
        ssh_of(&mut conn).key_path = Some(std::path::PathBuf::from("/home/u/.ssh/id_ed25519"));
        assert_eq!(ssh_of(&mut conn).auth_method, SshAuthMethod::Password);
        assert!(!conn.expects_password_prompt());
    }

    #[test]
    fn an_agent_key_source_alone_suppresses_the_prompt_notice() {
        let mut conn = create_test_connection();
        ssh_of(&mut conn).key_source = SshKeySource::Agent {
            fingerprint: "SHA256:abc".to_string(),
            comment: "u@host".to_string(),
        };
        assert!(!conn.expects_password_prompt());
    }

    #[test]
    fn a_file_key_source_alone_suppresses_the_prompt_notice() {
        let mut conn = create_test_connection();
        ssh_of(&mut conn).key_source = SshKeySource::File {
            path: std::path::PathBuf::from("/home/u/.ssh/id_rsa"),
        };
        assert!(!conn.expects_password_prompt());
    }

    #[test]
    fn every_key_auth_method_suppresses_the_prompt_notice() {
        for method in [
            SshAuthMethod::PublicKey,
            SshAuthMethod::Agent,
            SshAuthMethod::SecurityKey,
        ] {
            let mut conn = create_test_connection();
            ssh_of(&mut conn).auth_method = method.clone();
            assert!(
                !conn.expects_password_prompt(),
                "{method:?} should not expect a password prompt"
            );
        }
    }

    #[test]
    fn password_and_keyboard_interactive_still_expect_the_prompt() {
        for method in [SshAuthMethod::Password, SshAuthMethod::KeyboardInteractive] {
            let mut conn = create_test_connection();
            ssh_of(&mut conn).auth_method = method.clone();
            assert!(
                conn.expects_password_prompt(),
                "{method:?} should expect a password prompt"
            );
        }
    }

    #[test]
    fn a_non_ssh_connection_always_expects_the_prompt() {
        // RDP, VNC and the rest (except Web, tested below) have no key
        // configuration to reason about, so the notice stays. Erring towards
        // showing it is deliberate.
        let conn = Connection::new_rdp("Win".to_string(), "example.com".to_string(), 3389);
        assert!(conn.expects_password_prompt());
    }

    #[test]
    fn a_web_connection_never_expects_the_prompt() {
        // A Web bookmark's browser handles any page credential itself, and a
        // SOCKS tunnel authenticates to the jump host, not the site — so the
        // "you will be prompted for a password" notice is meaningless and was
        // firing before a dead-bastion tunnel had even failed.
        let mut conn = Connection::new_ssh("2ip".to_string(), "https://2ip.io".to_string(), 443);
        conn.protocol_config = ProtocolConfig::Web(crate::models::WebConfig::default());
        assert!(!conn.expects_password_prompt());
    }

    #[test]
    fn sftp_is_treated_like_ssh() {
        // SFTP shares SshConfig, so the same reasoning has to reach it.
        let mut conn = create_test_connection();
        let mut cfg = match &conn.protocol_config {
            ProtocolConfig::Ssh(cfg) => cfg.clone(),
            _ => unreachable!(),
        };
        cfg.key_path = Some(std::path::PathBuf::from("/home/u/.ssh/id_ed25519"));
        conn.protocol_config = ProtocolConfig::Sftp(cfg);
        assert!(!conn.expects_password_prompt());
    }

    #[test]
    fn resetting_the_monitoring_override_clears_only_the_switch() {
        let mut conn = create_test_connection();
        conn.monitoring_config = Some(MonitoringConfig {
            enabled: Some(false),
            interval_secs: Some(15),
        });
        assert_eq!(conn.monitoring_override(), MonitoringOverride::Disabled);

        assert!(conn.reset_monitoring_override());
        assert_eq!(conn.monitoring_override(), MonitoringOverride::Inherit);
        assert_eq!(
            conn.monitoring_config,
            Some(MonitoringConfig {
                enabled: None,
                interval_secs: Some(15),
            }),
            "the interval override survives the reset"
        );

        // Nothing left to clear: reported as no change, and not touched.
        let touched_at = conn.updated_at;
        assert!(!conn.reset_monitoring_override());
        assert_eq!(conn.updated_at, touched_at);
    }

    #[test]
    fn resetting_a_switch_only_override_drops_the_config() {
        let mut conn = create_test_connection();
        conn.monitoring_config = Some(MonitoringConfig {
            enabled: Some(true),
            interval_secs: None,
        });
        assert!(conn.reset_monitoring_override());
        assert!(conn.monitoring_config.is_none());
    }

    #[test]
    fn test_get_custom_property_not_found() {
        let conn = create_test_connection();
        assert!(conn.get_custom_property("nonexistent").is_none());
    }

    #[test]
    fn test_set_and_get_custom_property() {
        let mut conn = create_test_connection();
        let prop = CustomProperty::new_text("notes", "Test notes");
        conn.set_custom_property(prop);

        let retrieved = conn.get_custom_property("notes");
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.name, "notes");
        assert_eq!(retrieved.value, "Test notes");
        assert_eq!(retrieved.property_type, PropertyType::Text);
    }

    #[test]
    fn test_set_custom_property_replaces_existing() {
        let mut conn = create_test_connection();

        // Set initial property
        conn.set_custom_property(CustomProperty::new_text("notes", "Initial"));
        assert_eq!(conn.custom_properties.len(), 1);

        // Replace with new value
        conn.set_custom_property(CustomProperty::new_text("notes", "Updated"));
        assert_eq!(conn.custom_properties.len(), 1);

        let retrieved = conn.get_custom_property("notes").unwrap();
        assert_eq!(retrieved.value, "Updated");
    }

    #[test]
    fn test_remove_custom_property() {
        let mut conn = create_test_connection();
        conn.set_custom_property(CustomProperty::new_text("notes", "Test"));

        assert!(conn.remove_custom_property("notes"));
        assert!(conn.get_custom_property("notes").is_none());
        assert!(conn.custom_properties.is_empty());
    }

    #[test]
    fn test_remove_nonexistent_property() {
        let mut conn = create_test_connection();
        assert!(!conn.remove_custom_property("nonexistent"));
    }

    #[test]
    fn test_with_custom_properties_builder() {
        let props = vec![
            CustomProperty::new_text("notes", "Some notes"),
            CustomProperty::new_url("docs", "https://example.com"),
            CustomProperty::new_protected("api_key", "secret"),
        ];

        let conn = create_test_connection().with_custom_properties(props);

        assert_eq!(conn.custom_properties.len(), 3);
        assert!(conn.get_custom_property("notes").is_some());
        assert!(conn.get_custom_property("docs").is_some());
        assert!(conn.get_custom_property("api_key").is_some());
    }

    #[test]
    fn test_all_property_types() {
        let mut conn = create_test_connection();

        // Test Text type
        conn.set_custom_property(CustomProperty::new_text("text_prop", "text value"));
        let text_prop = conn.get_custom_property("text_prop").unwrap();
        assert_eq!(text_prop.property_type, PropertyType::Text);
        assert!(!text_prop.is_protected());
        assert!(!text_prop.is_url());

        // Test URL type
        conn.set_custom_property(CustomProperty::new_url("url_prop", "https://example.com"));
        let url_prop = conn.get_custom_property("url_prop").unwrap();
        assert_eq!(url_prop.property_type, PropertyType::Url);
        assert!(!url_prop.is_protected());
        assert!(url_prop.is_url());

        // Test Protected type
        conn.set_custom_property(CustomProperty::new_protected("protected_prop", "secret"));
        let protected_prop = conn.get_custom_property("protected_prop").unwrap();
        assert_eq!(protected_prop.property_type, PropertyType::Protected);
        assert!(protected_prop.is_protected());
        assert!(!protected_prop.is_url());
    }

    #[test]
    fn test_get_custom_property_mut() {
        let mut conn = create_test_connection();
        conn.set_custom_property(CustomProperty::new_text("notes", "Initial"));

        // Modify through mutable reference
        if let Some(prop) = conn.get_custom_property_mut("notes") {
            prop.value = "Modified".to_string();
        }

        let retrieved = conn.get_custom_property("notes").unwrap();
        assert_eq!(retrieved.value, "Modified");
    }

    #[test]
    fn test_set_custom_property_updates_timestamp() {
        let mut conn = create_test_connection();
        let initial_updated_at = conn.updated_at;

        // Small delay to ensure timestamp difference
        std::thread::sleep(std::time::Duration::from_millis(10));

        conn.set_custom_property(CustomProperty::new_text("notes", "Test"));

        assert!(conn.updated_at > initial_updated_at);
    }

    #[test]
    fn test_remove_custom_property_updates_timestamp() {
        let mut conn = create_test_connection();
        conn.custom_properties
            .push(CustomProperty::new_text("notes", "Test"));
        let initial_updated_at = conn.updated_at;

        // Small delay to ensure timestamp difference
        std::thread::sleep(std::time::Duration::from_millis(10));

        conn.remove_custom_property("notes");

        assert!(conn.updated_at > initial_updated_at);
    }

    #[test]
    fn command_macro_defaults_send_newline_true() {
        // serde-default: a macro deserialized without `send_newline` runs its
        // command (true), matching the "run this" common case.
        let json = r#"{"name":"root shell","command":"sudo -i"}"#;
        let m: CommandMacro = serde_json::from_str(json).unwrap();
        assert_eq!(m.name, "root shell");
        assert_eq!(m.command, "sudo -i");
        assert!(m.send_newline, "send_newline must default to true");
        assert!(m.keybind.is_none());
    }

    #[test]
    fn command_macro_rust_default_runs_the_command() {
        // The editor's "Add Macro" pushes `CommandMacro::default()` and only
        // writes `send_newline` when the switch changes, so the Rust default
        // must agree with the serde default and the switch (both on).
        let m = CommandMacro::default();
        assert!(m.send_newline, "Default must append Enter, like serde does");
        assert!(m.name.is_empty() && m.command.is_empty() && m.keybind.is_none());
        assert!(m.is_blank());
    }

    #[test]
    fn command_macro_ignores_legacy_prompt_vars_key() {
        // Early 0.23 builds wrote `prompt_vars`; it must not break loading.
        let json = r#"{"name":"n","command":"ls","prompt_vars":true,"send_newline":false}"#;
        let m: CommandMacro = serde_json::from_str(json).unwrap();
        assert_eq!(m.command, "ls");
        assert!(!m.send_newline);
        assert!(!serde_json::to_string(&m).unwrap().contains("prompt_vars"));
    }

    #[test]
    fn command_macro_validate_command_rejects_empty_and_control_chars() {
        let with = |command: &str| CommandMacro {
            command: command.into(),
            ..CommandMacro::default()
        };
        assert_eq!(
            with("").validate_command(),
            Err(CommandMacroError::EmptyCommand)
        );
        assert_eq!(
            with("   ").validate_command(),
            Err(CommandMacroError::EmptyCommand)
        );
        for bad in [
            "ls\r",
            "ls\nrm -rf /",
            "\x1b[201~",
            "a\x03",
            "x\0y",
            "del\x7f",
        ] {
            assert_eq!(
                with(bad).validate_command(),
                Err(CommandMacroError::ControlCharacter),
                "{bad:?} must be rejected"
            );
        }
        assert_eq!(with("printf 'a\tb'").validate_command(), Ok(()));
        assert_eq!(
            with("sudo systemctl restart ${svc}").validate_command(),
            Ok(())
        );
    }

    #[test]
    fn command_macro_keybind_requires_ctrl_alt_or_super() {
        use crate::config::{KeybindingSettings, MacroKeybindError, validate_macro_keybind};
        let kb = KeybindingSettings::default();
        for ok in [
            "<Control><Alt>r",
            "<Alt>F3",
            "<Super>m",
            "<Mod1>y",
            "<Primary><Alt>z",
            "F5",
            "<Shift>F24",
        ] {
            assert_eq!(
                validate_macro_keybind(ok, &kb),
                Ok(()),
                "{ok} must be accepted"
            );
        }
        for bare in ["r", "<Shift>r", "Return", "F25", "F0", "<Shift>Tab"] {
            assert_eq!(
                validate_macro_keybind(bare, &kb),
                Err(MacroKeybindError::MissingModifier),
                "{bare} must need a modifier"
            );
        }
        for bad in ["", "<Control>", "<Control"] {
            assert_eq!(
                validate_macro_keybind(bad, &kb),
                Err(MacroKeybindError::Invalid),
                "{bad:?} must be invalid"
            );
        }
    }

    #[test]
    fn command_macro_keybind_rejects_app_shortcuts_including_overrides() {
        use crate::config::{
            KeybindingSettings, MacroKeybindError, default_keybindings, validate_macro_keybind,
        };
        let mut kb = KeybindingSettings::default();
        // Every default accel is taken, whatever the modifier order or alias.
        let def = default_keybindings()
            .into_iter()
            .find(|d| d.default_accels.contains("<Control><Shift>"))
            .expect("at least one Ctrl+Shift default exists");
        let accel = def.default_accel_list()[0].replace("<Control><Shift>", "<Shift><Primary>");
        assert!(matches!(
            validate_macro_keybind(&accel, &kb),
            Err(MacroKeybindError::AppShortcut { ref action, .. }) if *action == def.action
        ));

        // A user override moves the binding: the old default becomes free and
        // the new chord becomes taken.
        let freed = def.default_accel_list()[0].to_string();
        kb.overrides
            .insert(def.action, "<Control><Alt><Super>k".into());
        assert_eq!(validate_macro_keybind(&freed, &kb), Ok(()));
        assert!(matches!(
            validate_macro_keybind("<Super><Alt><Control>k", &kb),
            Err(MacroKeybindError::AppShortcut { .. })
        ));
    }

    #[test]
    fn command_macro_is_blank_counts_the_keybind() {
        let keybind_only = CommandMacro {
            keybind: Some("<Control>F5".into()),
            ..CommandMacro::default()
        };
        assert!(
            !keybind_only.is_blank(),
            "a keybind-only row must be validated, not dropped"
        );
        let whitespace = CommandMacro {
            name: " ".into(),
            keybind: Some("  ".into()),
            ..CommandMacro::default()
        };
        assert!(whitespace.is_blank());
    }

    #[test]
    fn command_macro_round_trips() {
        let m = CommandMacro {
            name: "restart nginx".into(),
            command: "sudo systemctl restart nginx".into(),
            keybind: Some("<Control><Shift>r".into()),
            send_newline: true,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: CommandMacro = serde_json::from_str(&json).unwrap();
        assert_eq!(m, back);
    }

    #[test]
    fn automation_config_without_macros_still_loads() {
        // Backward compat: a config written before command_macros existed has no
        // such key; serde-default must yield an empty vec, not an error.
        let json = r#"{"expect_rules":[],"post_login_scripts":[]}"#;
        let cfg: AutomationConfig = serde_json::from_str(json).unwrap();
        assert!(cfg.command_macros.is_empty());
    }

    #[test]
    fn automation_config_empty_macros_not_serialized() {
        // skip_serializing_if keeps the on-disk form clean for the common case.
        let cfg = AutomationConfig::default();
        let json = serde_json::to_string(&cfg).unwrap();
        assert!(
            !json.contains("command_macros"),
            "empty command_macros must be omitted, got: {json}"
        );
    }

    #[test]
    fn macro_render_appends_carriage_return_when_send_newline() {
        let vars = VariableManager::new();
        let m = CommandMacro {
            name: "ls".into(),
            command: "ls -la".into(),
            keybind: None,
            send_newline: true,
        };
        let out = m.render_for_terminal(&vars, VariableScope::Global).unwrap();
        assert_eq!(&*out.text, "ls -la\r");
        assert!(out.unresolved.is_empty());
    }

    #[test]
    fn macro_render_no_newline_when_disabled() {
        let vars = VariableManager::new();
        let m = CommandMacro {
            name: "type".into(),
            command: "partial".into(),
            keybind: None,
            send_newline: false,
        };
        let out = m.render_for_terminal(&vars, VariableScope::Global).unwrap();
        assert_eq!(
            &*out.text, "partial",
            "no trailing CR when send_newline=false"
        );
    }

    #[test]
    fn macro_render_substitutes_variables() {
        let mut vars = VariableManager::new();
        vars.set_global(Variable::new("svc", "nginx"));
        let m = CommandMacro {
            name: "restart".into(),
            command: "sudo systemctl restart ${svc}".into(),
            keybind: None,
            send_newline: true,
        };
        let out = m.render_for_terminal(&vars, VariableScope::Global).unwrap();
        assert_eq!(&*out.text, "sudo systemctl restart nginx\r");
        assert!(out.unresolved.is_empty());
    }

    #[test]
    fn macro_render_reports_unresolved_variable() {
        let vars = VariableManager::new();
        let m = CommandMacro {
            name: "x".into(),
            command: "echo ${missing}".into(),
            keybind: None,
            send_newline: false,
        };
        let out = m.render_for_terminal(&vars, VariableScope::Global).unwrap();
        // Unresolved placeholder is kept verbatim and reported, not silently dropped.
        assert!(out.text.contains("${missing}"));
        assert_eq!(out.unresolved, vec!["missing".to_string()]);
    }
}
