//! Session logging functionality
//!
//! This module provides session logging capabilities for recording
//! terminal output to timestamped log files with configurable rotation
//! and retention policies.

use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

use chrono::{Local, Utc};
use thiserror::Error;
use zeroize::Zeroizing;

use crate::variables::{VariableManager, VariableScope};

/// Errors that can occur during logging operations
#[derive(Debug, Error)]
pub enum LogError {
    /// Failed to create log directory
    #[error("Failed to create log directory: {0}")]
    DirectoryCreation(String),

    /// Failed to create or open log file
    #[error("Failed to create/open log file: {0}")]
    FileCreation(String),

    /// Failed to write to log file
    #[error("Failed to write to log: {0}")]
    WriteError(String),

    /// Failed to flush log file
    #[error("Failed to flush log: {0}")]
    FlushError(String),

    /// Failed to rotate log file
    #[error("Failed to rotate log: {0}")]
    RotationError(String),

    /// Invalid path template
    #[error("Invalid path template: {0}")]
    InvalidTemplate(String),

    /// Failed to expand path template
    #[error("Failed to expand path template: {0}")]
    TemplateExpansion(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type for logging operations
pub type LogResult<T> = std::result::Result<T, LogError>;

/// Log configuration for session logging
///
/// Defines how session output should be logged, including file paths,
/// timestamp formatting, and retention policies.
#[derive(Debug, Clone, PartialEq, Eq)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
)] // Logging modes are independent boolean flags
pub struct LogConfig {
    /// Whether logging is enabled
    pub enabled: bool,
    /// Path template for log files (supports variables like `${connection_name}`, `${date}`, `${time}`, `${protocol}`)
    pub path_template: String,
    /// Timestamp format string (strftime format)
    pub timestamp_format: String,
    /// Maximum log file size in megabytes (0 = no limit)
    pub max_size_mb: u32,
    /// Number of days to retain log files (0 = no limit)
    pub retention_days: u32,
    /// Log terminal activity (change counts) - default mode
    pub log_activity: bool,
    /// Log user input (commands typed)
    pub log_input: bool,
    /// Log full terminal output (transcript)
    pub log_output: bool,
    /// Prepend `[HH:MM:SS]` timestamps to each log line
    pub log_timestamps: bool,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path_template: String::from(
                "${HOME}/.local/share/rustconn/logs/${connection_name}_${date}.log",
            ),
            timestamp_format: String::from("%Y-%m-%d %H:%M:%S"),
            max_size_mb: 10,
            retention_days: 30,
            log_activity: true,
            log_input: false,
            log_output: false,
            log_timestamps: false,
        }
    }
}

impl LogConfig {
    /// Creates a new `LogConfig` with the specified path template
    #[must_use]
    pub fn new(path_template: impl Into<String>) -> Self {
        Self {
            enabled: true,
            path_template: path_template.into(),
            ..Default::default()
        }
    }

    /// Sets whether logging is enabled
    #[must_use]
    pub const fn with_enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Sets the timestamp format
    #[must_use]
    pub fn with_timestamp_format(mut self, format: impl Into<String>) -> Self {
        self.timestamp_format = format.into();
        self
    }

    /// Sets the maximum log file size in megabytes
    #[must_use]
    pub const fn with_max_size_mb(mut self, max_size_mb: u32) -> Self {
        self.max_size_mb = max_size_mb;
        self
    }

    /// Sets the retention period in days
    #[must_use]
    pub const fn with_retention_days(mut self, retention_days: u32) -> Self {
        self.retention_days = retention_days;
        self
    }

    /// Sets whether to log terminal activity (change counts)
    #[must_use]
    pub const fn with_log_activity(mut self, enabled: bool) -> Self {
        self.log_activity = enabled;
        self
    }

    /// Sets whether to log user input (commands)
    #[must_use]
    pub const fn with_log_input(mut self, enabled: bool) -> Self {
        self.log_input = enabled;
        self
    }

    /// Sets whether to log full terminal output (transcript)
    #[must_use]
    pub const fn with_log_output(mut self, enabled: bool) -> Self {
        self.log_output = enabled;
        self
    }

    /// Sets whether to prepend timestamps to each log line
    #[must_use]
    pub const fn with_log_timestamps(mut self, enabled: bool) -> Self {
        self.log_timestamps = enabled;
        self
    }

    /// Validates the configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid (e.g., empty path template when enabled).
    pub fn validate(&self) -> LogResult<()> {
        if self.enabled && self.path_template.is_empty() {
            return Err(LogError::InvalidTemplate(
                "Path template cannot be empty".to_string(),
            ));
        }
        Ok(())
    }

    /// Resolves the effective logging configuration for one session.
    ///
    /// A connection's own configuration (connection editor → Logs) wins when it
    /// is enabled; otherwise the global settings (Settings → Terminal →
    /// Logging) apply with `log_dir` as the destination. Returns `None` when
    /// logging is switched off in both places.
    ///
    /// `global_timestamps` comes from `TerminalSettings::log_timestamps`: the
    /// global logging settings have no `LogConfig` of their own to carry it.
    #[must_use]
    pub fn resolve(
        global: &crate::config::LoggingSettings,
        global_timestamps: bool,
        log_dir: &Path,
        per_connection: Option<&Self>,
    ) -> Option<Self> {
        if let Some(per) = per_connection.filter(|c| c.enabled) {
            return Some(Self {
                path_template: anchor_template(&per.path_template, log_dir),
                ..per.clone()
            });
        }

        if !global.enabled {
            return None;
        }

        Some(Self {
            enabled: true,
            // Reproduces the historical global-logging file name
            // `<connection>_<YYYY-MM-DD_HH-MM-SS>.log`.
            path_template: log_dir
                .join("${connection_name}_${datetime}.log")
                .to_string_lossy()
                .into_owned(),
            // The global logging UI exposes no size limit, so nothing rotates
            // by size there; age-based pruning still applies.
            max_size_mb: 0,
            retention_days: global.retention_days,
            log_activity: global.log_activity,
            log_input: global.log_input,
            log_output: global.log_output,
            log_timestamps: global_timestamps,
            ..Self::default()
        })
    }
}

/// Anchors a relative path template to `base_dir`.
///
/// A leading `~/` becomes `${HOME}/` so the free-text path field in the
/// connection editor behaves the way a shell would. Templates that are already
/// absolute — literally, or via a leading `${HOME}` — are returned unchanged;
/// anything else is joined onto `base_dir`, which keeps a bare
/// `${connection_name}.log` inside the configured log directory instead of the
/// process working directory.
fn anchor_template(template: &str, base_dir: &Path) -> String {
    let template = template.trim();
    let normalized = if template == "~" {
        "${HOME}".to_string()
    } else if let Some(rest) = template.strip_prefix("~/") {
        format!("${{HOME}}/{rest}")
    } else {
        template.to_string()
    };

    if normalized.starts_with('/') || normalized.starts_with("${HOME}") {
        normalized
    } else {
        base_dir.join(normalized).to_string_lossy().into_owned()
    }
}

/// Deletes `*.log` files directly inside `dir` that are older than
/// `retention_days`.
///
/// Returns the number of files removed. Does nothing when `retention_days` is
/// `0` (retain forever) or when `dir` cannot be read. Only the directory it is
/// given is scanned — never a parent, never recursively — so callers must pass
/// a directory that belongs to `RustConn`.
pub fn prune_logs(dir: &Path, retention_days: u32) -> usize {
    if retention_days == 0 {
        return 0;
    }

    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(u64::from(retention_days) * 24 * 60 * 60);

    let Ok(entries) = fs::read_dir(dir) else {
        return 0; // Directory might not exist yet
    };

    let mut removed = 0;
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "log")
            && let Ok(metadata) = fs::metadata(&path)
            && metadata.is_file()
            && let Ok(modified) = metadata.modified()
            && modified < cutoff
            && fs::remove_file(&path).is_ok()
        {
            removed += 1;
        }
    }
    removed
}

// Implement serde traits manually to support serialization
impl serde::Serialize for LogConfig {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("LogConfig", 9)?;
        state.serialize_field("enabled", &self.enabled)?;
        state.serialize_field("path_template", &self.path_template)?;
        state.serialize_field("timestamp_format", &self.timestamp_format)?;
        state.serialize_field("max_size_mb", &self.max_size_mb)?;
        state.serialize_field("retention_days", &self.retention_days)?;
        state.serialize_field("log_activity", &self.log_activity)?;
        state.serialize_field("log_input", &self.log_input)?;
        state.serialize_field("log_output", &self.log_output)?;
        state.serialize_field("log_timestamps", &self.log_timestamps)?;
        state.end()
    }
}

impl<'de> serde::Deserialize<'de> for LogConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[expect(
            clippy::struct_excessive_bools,
            reason = "settings/flags struct mirrors persisted config 1:1; bools represent independent toggles, not a state machine"
        )]
        struct LogConfigHelper {
            enabled: bool,
            path_template: String,
            timestamp_format: String,
            max_size_mb: u32,
            retention_days: u32,
            #[serde(default = "default_log_activity")]
            log_activity: bool,
            #[serde(default)]
            log_input: bool,
            #[serde(default)]
            log_output: bool,
            #[serde(default)]
            log_timestamps: bool,
        }

        fn default_log_activity() -> bool {
            true
        }

        let helper = LogConfigHelper::deserialize(deserializer)?;
        Ok(Self {
            enabled: helper.enabled,
            path_template: helper.path_template,
            timestamp_format: helper.timestamp_format,
            max_size_mb: helper.max_size_mb,
            retention_days: helper.retention_days,
            log_activity: helper.log_activity,
            log_input: helper.log_input,
            log_output: helper.log_output,
            log_timestamps: helper.log_timestamps,
        })
    }
}

/// Context for path template expansion
///
/// Contains the variables that can be used in log path templates.
#[derive(Debug, Clone, Default)]
pub struct LogContext {
    /// Connection name
    pub connection_name: String,
    /// Protocol type (ssh, rdp, vnc, spice)
    pub protocol: String,
    /// Additional custom variables
    pub custom_vars: std::collections::HashMap<String, String>,
}

impl LogContext {
    /// Creates a new `LogContext` with the given connection name and protocol
    #[must_use]
    pub fn new(connection_name: impl Into<String>, protocol: impl Into<String>) -> Self {
        Self {
            connection_name: connection_name.into(),
            protocol: protocol.into(),
            custom_vars: std::collections::HashMap::new(),
        }
    }

    /// Adds a custom variable to the context
    #[must_use]
    pub fn with_var(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.custom_vars.insert(name.into(), value.into());
        self
    }
}

/// Session logger for writing terminal output to files
///
/// Handles log file creation, writing with timestamps, rotation,
/// and cleanup based on retention policies.
pub struct SessionLogger {
    /// Log configuration
    config: LogConfig,
    /// Current log file path
    log_path: PathBuf,
    /// Buffered file writer
    writer: Option<BufWriter<File>>,
    /// Bytes written to current log file
    bytes_written: u64,
    /// Rotation counter for current session
    rotation_count: u32,
    /// Redaction applied to everything written through this logger.
    ///
    /// A session log records what the user typed, so a password answered at a
    /// prompt would otherwise land on disk in clear text.
    sanitize: SanitizeConfig,
    /// Whether the last content line written was a sensitive prompt.
    ///
    /// A password typed at a `Password:` prompt is not on the same line as the
    /// prompt — the prompt is one record and the answer the next — and the
    /// answer carries no marker of its own (`INPUT: hunter2` matches nothing in
    /// [`SENSITIVE_PATTERNS`]). Per-line redaction alone therefore lets the
    /// initial password through (issue
    /// [#321](https://github.com/totoshko88/RustConn/issues/321)). This flag is
    /// set when a written line trips [`contains_sensitive_prompt`] and consumed
    /// by the next line, which is redacted whole regardless of its content. It
    /// spans both channels because the prompt travels on the transcript and the
    /// answer on the `INPUT:` records, and both go through this one logger.
    armed_after_prompt: bool,
}

impl SessionLogger {
    /// Creates a new session logger with the given configuration and context
    ///
    /// # Arguments
    ///
    /// * `config` - Log configuration
    /// * `context` - Context for path template expansion
    /// * `variable_manager` - Optional variable manager for additional substitution
    ///
    /// # Errors
    ///
    /// Returns an error if the log file cannot be created.
    pub fn new(
        config: LogConfig,
        context: &LogContext,
        variable_manager: Option<&VariableManager>,
    ) -> LogResult<Self> {
        config.validate()?;

        if !config.enabled {
            return Ok(Self {
                config,
                log_path: PathBuf::new(),
                writer: None,
                bytes_written: 0,
                rotation_count: 0,
                sanitize: SanitizeConfig::new(),
                armed_after_prompt: false,
            });
        }

        // Expand the path template
        let log_path =
            Self::expand_path_template(&config.path_template, context, variable_manager)?;

        // Create parent directories if needed
        if let Some(parent) = log_path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                LogError::DirectoryCreation(format!("Failed to create {}: {}", parent.display(), e))
            })?;
        }

        // Create the log file. Owner-only (0600) on unix: a session transcript
        // can hold sensitive output even after redaction, and the path may be an
        // absolute location outside the 0700 config dir (the guide itself
        // suggests one for Flatpak), where the process umask would otherwise
        // leave it world-readable.
        let mut open_opts = OpenOptions::new();
        open_opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open_opts.mode(0o600);
        }
        let file = open_opts.open(&log_path).map_err(|e| {
            LogError::FileCreation(format!("Failed to open {}: {}", log_path.display(), e))
        })?;

        let writer = BufWriter::new(file);

        // Get current file size
        let bytes_written = fs::metadata(&log_path).map(|m| m.len()).unwrap_or(0);

        Ok(Self {
            config,
            log_path,
            writer: Some(writer),
            bytes_written,
            rotation_count: 0,
            sanitize: SanitizeConfig::new(),
            armed_after_prompt: false,
        })
    }

    /// Replaces the redaction rules applied to written data.
    #[must_use]
    pub fn with_sanitize(mut self, sanitize: SanitizeConfig) -> Self {
        self.sanitize = sanitize;
        self
    }

    /// Expands a path template with context variables
    ///
    /// Supports the following variables:
    /// - `${connection_name}` - The connection name
    /// - `${protocol}` - The protocol type
    /// - `${date}` - Current date (YYYY-MM-DD)
    /// - `${time}` - Current time (HH-MM-SS)
    /// - `${datetime}` - Current datetime (YYYY-MM-DD_HH-MM-SS)
    /// - `${HOME}` - User's home directory
    /// - Any custom variables from the context
    /// - Any variables from the `VariableManager`
    ///
    /// A leading `~` or `~/` is treated as the user's home directory.
    ///
    /// # Errors
    ///
    /// Returns an error if a variable cannot be expanded or is undefined.
    pub fn expand_path_template(
        template: &str,
        context: &LogContext,
        variable_manager: Option<&VariableManager>,
    ) -> LogResult<PathBuf> {
        let now = Local::now();
        // The path field is free text, so a shell-style `~/` has to be honored
        // here — nothing else in the chain expands it, and a literal `~`
        // directory is never what the user meant.
        let mut result = match template.trim() {
            "~" => "${HOME}".to_string(),
            other => other
                .strip_prefix("~/")
                .map_or_else(|| other.to_string(), |rest| format!("${{HOME}}/{rest}")),
        };

        // Built-in variables
        let builtins = [
            (
                "connection_name",
                sanitize_filename(&context.connection_name),
            ),
            ("protocol", context.protocol.clone()),
            ("date", now.format("%Y-%m-%d").to_string()),
            ("time", now.format("%H-%M-%S").to_string()),
            ("datetime", now.format("%Y-%m-%d_%H-%M-%S").to_string()),
            (
                "HOME",
                dirs::home_dir()
                    .map_or_else(|| ".".to_string(), |p| p.to_string_lossy().to_string()),
            ),
        ];

        for (name, value) in &builtins {
            let pattern = format!("${{{name}}}");
            result = result.replace(&pattern, value);
        }

        // Custom context variables
        for (name, value) in &context.custom_vars {
            let pattern = format!("${{{name}}}");
            result = result.replace(&pattern, value);
        }

        // Variable manager substitution (if provided)
        if let Some(vm) = variable_manager {
            // Try to substitute remaining variables using the variable manager
            result = vm
                .substitute(&result, VariableScope::Global)
                .map_err(|e| LogError::TemplateExpansion(e.to_string()))?;
        }

        // Check for any remaining unsubstituted variables
        if result.contains("${") {
            // Extract the first unsubstituted variable for error message
            if let Some(start) = result.find("${")
                && let Some(end) = result[start..].find('}')
            {
                let var_name = &result[start + 2..start + end];
                return Err(LogError::TemplateExpansion(format!(
                    "Undefined variable: {var_name}"
                )));
            }
        }

        Ok(PathBuf::from(result))
    }

    /// Formats a timestamp according to the configured format
    #[must_use]
    pub fn format_timestamp(&self, format: &str) -> String {
        Local::now().format(format).to_string()
    }

    /// Returns the current timestamp formatted according to config
    #[must_use]
    pub fn current_timestamp(&self) -> String {
        self.format_timestamp(&self.config.timestamp_format)
    }

    /// Returns the log file path
    #[must_use]
    pub fn log_path(&self) -> &Path {
        &self.log_path
    }

    /// Returns the number of bytes written to the current log file
    #[must_use]
    pub const fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Strips escapes, redacts secrets, and applies the after-prompt state.
    ///
    /// Runs before every write so a line following a sensitive prompt is
    /// redacted whole even though it carries no marker of its own — the case
    /// per-line matching misses, and the one that leaks the initial password
    /// typed at a `Password:` prompt (issue
    /// [#321](https://github.com/totoshko88/RustConn/issues/321)).
    ///
    /// Escapes are stripped first so redaction sees plain text — a password
    /// prompt with an embedded colour code would otherwise split the pattern
    /// and slip past both the per-line matcher and the arming decision, which
    /// reads the same plain text and would miss `Password` and `:` on opposite
    /// sides of an escape.
    ///
    /// `consume_arm` distinguishes the two channels. The transcript (`write`)
    /// passes `false`: it *arms* the state when it carries a prompt, but its own
    /// following lines are legitimate output that a no-echo password prompt does
    /// not echo, so blanking them would eat real transcript. The `INPUT:` event
    /// channel (`write_record`) passes `true`: that is where a typed password
    /// actually lands, carrying no marker of its own, so an armed line there is
    /// redacted whole and the state disarmed.
    fn sanitize_with_prompt_state(&mut self, input: &str, consume_arm: bool) -> Zeroizing<String> {
        let stripped = Zeroizing::new(strip_ansi_escapes(input));

        // With sanitization off, credential redaction is disabled by
        // configuration; the after-prompt masking is part of that same
        // protection, so it is off too. The lines still update the arming state
        // so the feature behaves consistently if sanitization is toggled.
        if !self.sanitize.enabled {
            self.arm_from_lines(&stripped, consume_arm);
            return Zeroizing::new(stripped.to_string());
        }

        let ends_with_newline = stripped.ends_with('\n');
        let mut out = Zeroizing::new(String::with_capacity(stripped.len()));
        let mut first = true;
        for line in stripped.lines() {
            if !first {
                out.push('\n');
            }
            first = false;

            if consume_arm && self.armed_after_prompt && !line.trim().is_empty() {
                // The answer to the prompt on the input channel. Redact the
                // whole line — it carries no marker, so nothing below would
                // catch it — and disarm.
                out.push_str(&self.sanitize.replacement);
                self.armed_after_prompt = false;
                continue;
            }

            let sanitized = sanitize_output_zeroizing(line, &self.sanitize);
            out.push_str(&sanitized);

            // Arm from the plain line: the next input line is treated as the
            // secret answer. A blank line does not disarm — a prompt is often
            // followed by an empty transcript flush before the answer arrives.
            if contains_sensitive_prompt(line) {
                self.armed_after_prompt = true;
            } else if !consume_arm && !line.trim().is_empty() {
                // Substantive transcript output means the prompt was already
                // answered elsewhere (a vault password sent straight to the PTY,
                // never through the `INPUT:` channel). Disarm so a later,
                // unrelated command is not blanked. The typed-password case is
                // unaffected: the `INPUT:` record arrives and consumes the arm
                // before the server's response reaches the transcript.
                self.armed_after_prompt = false;
            }
        }
        if ends_with_newline && !out.ends_with('\n') {
            out.push('\n');
        }
        out
    }

    /// Updates the after-prompt arming state from plain text, without writing.
    ///
    /// Used when sanitization is disabled so the state stays consistent.
    fn arm_from_lines(&mut self, stripped: &str, consume_arm: bool) {
        for line in stripped.lines() {
            if consume_arm && self.armed_after_prompt && !line.trim().is_empty() {
                self.armed_after_prompt = false;
            } else if contains_sensitive_prompt(line) {
                self.armed_after_prompt = true;
            } else if !consume_arm && !line.trim().is_empty() {
                self.armed_after_prompt = false;
            }
        }
    }

    /// Returns whether logging is enabled
    #[must_use]
    pub const fn is_enabled(&self) -> bool {
        self.config.enabled
    }

    /// Returns the log configuration
    #[must_use]
    pub const fn config(&self) -> &LogConfig {
        &self.config
    }

    /// Writes data to the log file with a timestamp prefix
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails or rotation fails.
    pub fn write(&mut self, data: &[u8]) -> LogResult<()> {
        if !self.config.enabled {
            return Ok(());
        }

        // Check that writer is available
        if self.writer.is_none() {
            return Err(LogError::WriteError("Log file not open".to_string()));
        }

        // Check if rotation is needed before writing
        self.rotate_if_needed()?;

        // Write lines, optionally with timestamp prefix
        let decoded = Zeroizing::new(String::from_utf8_lossy(data).into_owned());
        // The transcript arms the after-prompt state but does not consume it:
        // its own following lines are non-echoed legitimate output.
        let data_str = self.sanitize_with_prompt_state(&decoded, false);

        for line in data_str.lines() {
            let formatted = Zeroizing::new(if self.config.log_timestamps {
                let timestamp = self.current_timestamp();
                format!("[{timestamp}] {line}\n")
            } else {
                format!("{line}\n")
            });
            let bytes = formatted.as_bytes();

            // Get writer (may have changed after rotation)
            let writer = self.writer.as_mut().ok_or_else(|| {
                LogError::WriteError("Log file not open after rotation".to_string())
            })?;

            writer
                .write_all(bytes)
                .map_err(|e| LogError::WriteError(format!("Failed to write: {e}")))?;

            self.bytes_written += bytes.len() as u64;
        }

        Ok(())
    }

    /// Writes one already-formatted record as a single line.
    ///
    /// Unlike [`Self::write`], the caller owns the formatting — including any
    /// timestamp prefix — so event records such as `INPUT:` lines keep their
    /// own layout. Escape stripping, redaction and size-based rotation still
    /// apply: a record is assembled from terminal content, so it carries the
    /// same escape sequences and the same secrets that [`Self::write`] handles.
    ///
    /// # Errors
    ///
    /// Returns an error if rotation or writing fails.
    pub fn write_record(&mut self, record: &str) -> LogResult<()> {
        if !self.config.enabled {
            return Ok(());
        }

        self.rotate_if_needed()?;

        // Event records include the `INPUT:` channel, where a password typed at
        // a prompt lands with no marker of its own — so this channel consumes
        // the after-prompt arming and redacts that line whole (issue #321).
        let sanitized = self.sanitize_with_prompt_state(record, true);
        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| LogError::WriteError("Log file not open".to_string()))?;

        writeln!(writer, "{}", sanitized.as_str())
            .map_err(|e| LogError::WriteError(format!("Failed to write: {e}")))?;
        self.bytes_written += sanitized.len() as u64 + 1;
        Ok(())
    }

    /// Writes bytes through to the log file exactly as given.
    ///
    /// Nothing is added and nothing is removed: no timestamp, no escape
    /// stripping, no redaction. This exists for formats that must survive
    /// byte-for-byte (a replayable recording keeps its escape sequences,
    /// because they *are* the recording). Anything holding terminal text for a
    /// human to read belongs in [`Self::write`] or [`Self::write_record`].
    ///
    /// # Errors
    ///
    /// Returns an error if writing fails.
    pub fn write_raw(&mut self, data: &[u8]) -> LogResult<()> {
        if !self.config.enabled {
            return Ok(());
        }

        // Check if rotation is needed before writing
        self.rotate_if_needed()?;

        let writer = self
            .writer
            .as_mut()
            .ok_or_else(|| LogError::WriteError("Log file not open".to_string()))?;

        writer
            .write_all(data)
            .map_err(|e| LogError::WriteError(format!("Failed to write: {e}")))?;

        self.bytes_written += data.len() as u64;
        Ok(())
    }

    /// Flushes the log buffer to disk
    ///
    /// # Errors
    ///
    /// Returns an error if flushing fails.
    pub fn flush(&mut self) -> LogResult<()> {
        if let Some(writer) = self.writer.as_mut() {
            writer
                .flush()
                .map_err(|e| LogError::FlushError(format!("Failed to flush: {e}")))?;
        }
        Ok(())
    }

    /// Checks if log rotation is needed and performs it if necessary
    fn rotate_if_needed(&mut self) -> LogResult<()> {
        if self.config.max_size_mb == 0 {
            return Ok(()); // No size limit
        }

        let max_bytes = u64::from(self.config.max_size_mb) * 1024 * 1024;

        if self.bytes_written >= max_bytes {
            self.rotate()?;
        }

        Ok(())
    }

    /// Rotates the log file
    ///
    /// Creates a new log file with a rotation suffix and updates the writer.
    ///
    /// # Errors
    ///
    /// Returns an error if rotation fails.
    pub fn rotate(&mut self) -> LogResult<()> {
        if !self.config.enabled {
            return Ok(());
        }

        // Flush and close current file
        self.flush()?;
        self.writer = None;

        // Generate rotated filename
        self.rotation_count += 1;
        let rotated_path = self.generate_rotated_path();

        // Rename current log to rotated name
        if self.log_path.exists() {
            fs::rename(&self.log_path, &rotated_path).map_err(|e| {
                LogError::RotationError(format!(
                    "Failed to rename {} to {}: {}",
                    self.log_path.display(),
                    rotated_path.display(),
                    e
                ))
            })?;
        }

        // Create new log file, owner-only (0600) on unix — same rationale as the
        // initial open above.
        let mut open_opts = OpenOptions::new();
        open_opts.create(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            open_opts.mode(0o600);
        }
        let file = open_opts.open(&self.log_path).map_err(|e| {
            LogError::FileCreation(format!(
                "Failed to create new log file {}: {}",
                self.log_path.display(),
                e
            ))
        })?;

        self.writer = Some(BufWriter::new(file));
        self.bytes_written = 0;

        // Clean up old rotated files based on retention policy
        self.cleanup_old_logs();

        Ok(())
    }

    /// Generates a path for a rotated log file
    fn generate_rotated_path(&self) -> PathBuf {
        let timestamp = Utc::now().format("%Y%m%d_%H%M%S");
        let stem = self
            .log_path
            .file_stem()
            .map_or_else(|| "log".to_string(), |s| s.to_string_lossy().to_string());
        let ext = self
            .log_path
            .extension()
            .map(|s| format!(".{}", s.to_string_lossy()))
            .unwrap_or_default();

        let rotated_name = format!("{stem}.{timestamp}.{}{ext}", self.rotation_count);

        self.log_path.with_file_name(rotated_name)
    }

    /// Cleans up old log files based on retention policy
    fn cleanup_old_logs(&self) {
        if let Some(parent) = self.log_path.parent() {
            prune_logs(parent, self.config.retention_days);
        }
    }

    /// Closes the log file, flushing any buffered data
    ///
    /// # Errors
    ///
    /// Returns an error if flushing fails.
    pub fn close(&mut self) -> LogResult<()> {
        if let Some(mut writer) = self.writer.take() {
            // Write session end marker
            let timestamp = self.current_timestamp();
            let end_marker = format!("\n[{timestamp}] === Session ended ===\n");
            let _ = writer.write_all(end_marker.as_bytes());

            writer
                .flush()
                .map_err(|e| LogError::FlushError(format!("Failed to flush on close: {e}")))?;
        }
        Ok(())
    }
}

impl Drop for SessionLogger {
    fn drop(&mut self) {
        // Attempt to close gracefully, ignoring errors
        let _ = self.close();
    }
}

/// Sanitizes a filename by removing or replacing invalid characters
fn sanitize_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .chars()
        .take(64) // Limit length
        .collect()
}

/// Patterns that indicate sensitive data in terminal output.
/// Only lowercase variants are needed — the matching code already
/// calls `to_lowercase()` on both sides before comparison.
const SENSITIVE_PATTERNS: &[&str] = &[
    "password:",
    "pass:",
    "passphrase:",
    "secret:",
    "token:",
    "api_key:",
    "apikey:",
    "private_key:",
    "ssh_pass:",
    "sudo password",
    "enter passphrase",
    "enter pin",
    "otp:",
    "2fa:",
    "mfa:",
    "client_secret:",
    "authorization:",
];

/// Regex patterns for detecting sensitive data values
/// These match common password/key formats that follow a prompt
const SENSITIVE_VALUE_PATTERNS: &[&str] = &[
    // Password prompts followed by input (masked in most terminals but may leak)
    r"(?i)password[:\s]+\S+",
    r"(?i)pass[:\s]+\S+",
    // API keys and tokens (common formats)
    r"(?i)api[_-]?key[:\s=]+[a-zA-Z0-9_\-]{16,}",
    r"(?i)token[:\s=]+[a-zA-Z0-9_\-\.]{16,}",
    r"(?i)bearer\s+[a-zA-Z0-9_\-\.]+",
    // AWS credentials
    r"AKIA[0-9A-Z]{16}",
    r"(?i)aws[_-]?secret[_-]?access[_-]?key[:\s=]+\S+",
    // Private keys (PEM format markers)
    r"-----BEGIN\s+(RSA\s+)?PRIVATE\s+KEY-----",
    r"-----BEGIN\s+OPENSSH\s+PRIVATE\s+KEY-----",
    // SSH key fingerprints (not sensitive but may indicate key operations)
    r"SHA256:[a-zA-Z0-9+/]{43}",
    // GitHub personal access tokens
    r"ghp_[a-zA-Z0-9]{36}",
    // GitLab personal access tokens
    r"glpat-[a-zA-Z0-9\-_]{20,}",
    // JWT tokens (header.payload.signature)
    r"eyJ[a-zA-Z0-9_-]{10,}\.[a-zA-Z0-9_-]{10,}\.[a-zA-Z0-9_-]{10,}",
];

/// Pre-compiled regexes for `SENSITIVE_VALUE_PATTERNS` — compiled once via `LazyLock`.
static COMPILED_SENSITIVE_PATTERNS: LazyLock<Vec<regex::Regex>> = LazyLock::new(|| {
    SENSITIVE_VALUE_PATTERNS
        .iter()
        .filter_map(|p| regex::Regex::new(p).ok())
        .collect()
});

/// Configuration for log sanitization
#[derive(Debug, Clone)]
pub struct SanitizeConfig {
    /// Whether sanitization is enabled
    pub enabled: bool,
    /// Replacement text for sensitive data
    pub replacement: String,
    /// Additional custom patterns to sanitize (regex strings)
    pub custom_patterns: Vec<String>,
    /// Whether to sanitize entire lines containing sensitive prompts
    pub sanitize_full_lines: bool,
    /// Pre-compiled custom regex patterns (built from `custom_patterns`)
    compiled_custom: Vec<regex::Regex>,
}

impl PartialEq for SanitizeConfig {
    fn eq(&self, other: &Self) -> bool {
        self.enabled == other.enabled
            && self.replacement == other.replacement
            && self.custom_patterns == other.custom_patterns
            && self.sanitize_full_lines == other.sanitize_full_lines
    }
}

impl Eq for SanitizeConfig {}

impl Default for SanitizeConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            replacement: String::from("[REDACTED]"),
            custom_patterns: Vec::new(),
            sanitize_full_lines: true,
            compiled_custom: Vec::new(),
        }
    }
}

impl SanitizeConfig {
    /// Creates a new sanitize config with sanitization enabled
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a disabled sanitize config
    #[must_use]
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            ..Default::default()
        }
    }

    /// Sets the replacement text
    #[must_use]
    pub fn with_replacement(mut self, replacement: impl Into<String>) -> Self {
        self.replacement = replacement.into();
        self
    }

    /// Adds a custom pattern to sanitize and pre-compiles it
    #[must_use]
    pub fn with_custom_pattern(mut self, pattern: impl Into<String>) -> Self {
        let p = pattern.into();
        if let Ok(re) = regex::Regex::new(&p) {
            self.compiled_custom.push(re);
        }
        self.custom_patterns.push(p);
        self
    }

    /// Sets whether to sanitize full lines containing sensitive prompts
    #[must_use]
    pub const fn with_full_line_sanitization(mut self, enabled: bool) -> Self {
        self.sanitize_full_lines = enabled;
        self
    }
}

/// Sanitizes terminal output by removing or masking sensitive data
///
/// This function detects and redacts:
/// - Password prompts and their values
/// - API keys and tokens
/// - Private key content
/// - AWS credentials
/// - Custom patterns specified in config
///
/// # Arguments
///
/// * `output` - The terminal output to sanitize
/// * `config` - Sanitization configuration
///
/// # Returns
///
/// The sanitized output with sensitive data replaced
#[must_use]
pub fn sanitize_output(output: &str, config: &SanitizeConfig) -> String {
    sanitize_output_zeroizing(output, config).to_string()
}

/// Internal sanitizer used by session logging so transcript copies are scrubbed.
fn sanitize_output_zeroizing(output: &str, config: &SanitizeConfig) -> Zeroizing<String> {
    if !config.enabled {
        return Zeroizing::new(output.to_string());
    }

    let mut result = Zeroizing::new(output.to_string());

    // Check for sensitive prompt patterns and optionally sanitize full lines.
    if config.sanitize_full_lines {
        let mut sanitized = Zeroizing::new(String::with_capacity(result.len()));
        for (index, line) in result.lines().enumerate() {
            if index > 0 {
                sanitized.push('\n');
            }
            let line_lower = Zeroizing::new(line.to_lowercase());
            if SENSITIVE_PATTERNS
                .iter()
                .any(|pattern| line_lower.contains(pattern))
            {
                sanitized.push_str(&config.replacement);
            } else {
                sanitized.push_str(line);
            }
        }
        // Preserve trailing newline if original had one.
        if output.ends_with('\n') && !sanitized.ends_with('\n') {
            sanitized.push('\n');
        }
        result = sanitized;
    }

    // Apply pre-compiled regex patterns for sensitive values.
    for re in COMPILED_SENSITIVE_PATTERNS.iter() {
        result = Zeroizing::new(
            re.replace_all(&result, config.replacement.as_str())
                .into_owned(),
        );
    }

    // Apply pre-compiled custom patterns.
    for re in &config.compiled_custom {
        result = Zeroizing::new(
            re.replace_all(&result, config.replacement.as_str())
                .into_owned(),
        );
    }

    result
}

/// Checks if a line contains sensitive data prompts
///
/// This is a quick check that doesn't perform full sanitization,
/// useful for deciding whether to log a line at all.
#[must_use]
pub fn contains_sensitive_prompt(line: &str) -> bool {
    let line_lower = line.to_lowercase();
    SENSITIVE_PATTERNS
        .iter()
        .any(|pattern| line_lower.contains(pattern))
}

/// Pre-compiled regex for stripping ANSI escape sequences from PTY output.
///
/// Matches:
/// - CSI sequences: `ESC [ ... <final byte>` (colors, cursor movement, etc.)
/// - OSC sequences: `ESC ] ... ST` (window titles, hyperlinks)
/// - Simple escapes: `ESC <char>` (alternate screen, keypad mode, etc.)
/// - C1 control codes (rare, 8-bit): `0x9B .. <final>`
static ANSI_ESCAPE_RE: LazyLock<regex::Regex> = LazyLock::new(|| {
    // Matches ANSI escape sequences and control characters:
    // - CSI: ESC [ <params> <intermediate> <final>
    // - OSC: ESC ] <text> (BEL | ST)
    // - Character set selection: ESC ( | ) | # followed by one char
    // - Other ESC sequences: ESC <intermediate>* <final>
    // - 8-bit CSI: 0x9B <params> <final>
    // - Control chars (except tab, newline, CR)
    regex::Regex::new(concat!(
        r"\x1b\[[0-9;?]*[ -/]*[@-~]",          // CSI sequences
        r"|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)", // OSC (BEL or ST terminated)
        r"|\x1b[()#].",                        // Character set selection
        r"|\x1b[ -/]*[0-~]",                   // Other ESC sequences
        r"|\x9b[0-9;]*[@-~]",                  // 8-bit CSI
        r"|[\x00-\x08\x0b\x0c\x0e-\x1f]",      // Control chars (not \t\n\r)
    ))
    .expect("ANSI escape regex must compile")
});

/// Strips ANSI escape sequences and non-printable control characters from
/// terminal output, producing clean text suitable for log files.
///
/// Preserves tab, newline and carriage return; removes CSI sequences (colour,
/// cursor movement), OSC sequences (window titles, hyperlinks) and the
/// remaining control characters.
///
/// Session logs go through this so a transcript opened in a text editor is
/// readable, and so redaction matches against plain text (issue
/// [#247](https://github.com/totoshko88/RustConn/issues/247)).
#[must_use]
pub fn strip_ansi_escapes(input: &str) -> String {
    ANSI_ESCAPE_RE.replace_all(input, "").into_owned()
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn test_log_config_default() {
        let config = LogConfig::default();
        assert!(!config.enabled);
        assert!(!config.path_template.is_empty());
        assert_eq!(config.max_size_mb, 10);
        assert_eq!(config.retention_days, 30);
    }

    #[test]
    fn test_log_config_builder() {
        let config = LogConfig::new("/tmp/test.log")
            .with_enabled(true)
            .with_timestamp_format("%H:%M:%S")
            .with_max_size_mb(5)
            .with_retention_days(7);

        assert!(config.enabled);
        assert_eq!(config.path_template, "/tmp/test.log");
        assert_eq!(config.timestamp_format, "%H:%M:%S");
        assert_eq!(config.max_size_mb, 5);
        assert_eq!(config.retention_days, 7);
    }

    #[test]
    fn test_log_config_validation() {
        let valid_config = LogConfig::new("/tmp/test.log").with_enabled(true);
        assert!(valid_config.validate().is_ok());

        let invalid_config = LogConfig::new("").with_enabled(true);
        assert!(invalid_config.validate().is_err());

        let disabled_config = LogConfig::new("").with_enabled(false);
        assert!(disabled_config.validate().is_ok());
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("test-server"), "test-server");
        assert_eq!(sanitize_filename("test server"), "test_server");
        assert_eq!(sanitize_filename("test/server"), "test_server");
        assert_eq!(sanitize_filename("test:server"), "test_server");
        assert_eq!(sanitize_filename("test@server.com"), "test_server.com");
    }

    #[test]
    fn test_log_context() {
        let context = LogContext::new("my-server", "ssh").with_var("custom", "value");

        assert_eq!(context.connection_name, "my-server");
        assert_eq!(context.protocol, "ssh");
        assert_eq!(
            context.custom_vars.get("custom"),
            Some(&"value".to_string())
        );
    }

    #[test]
    fn test_expand_path_template_basic() {
        let context = LogContext::new("test-server", "ssh");
        let template = "/tmp/${connection_name}_${protocol}.log";

        let result = SessionLogger::expand_path_template(template, &context, None).unwrap();
        assert_eq!(result, PathBuf::from("/tmp/test-server_ssh.log"));
    }

    #[test]
    fn test_expand_path_template_with_date() {
        let context = LogContext::new("server", "vnc");
        let template = "/tmp/${connection_name}_${date}.log";

        let result = SessionLogger::expand_path_template(template, &context, None).unwrap();
        let result_str = result.to_string_lossy();

        assert!(result_str.starts_with("/tmp/server_"));
        assert!(result_str.ends_with(".log"));
        // Date format: YYYY-MM-DD
        assert!(result_str.contains('-'));
    }

    #[test]
    fn test_expand_path_template_undefined_var() {
        let context = LogContext::new("server", "ssh");
        let template = "/tmp/${undefined_var}.log";

        let result = SessionLogger::expand_path_template(template, &context, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_session_logger_disabled() {
        let config = LogConfig::default(); // disabled by default
        let context = LogContext::new("test", "ssh");

        let logger = SessionLogger::new(config, &context, None).unwrap();
        assert!(!logger.is_enabled());
        assert!(logger.writer.is_none());
    }

    #[test]
    fn test_session_logger_creation() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("test.log");

        let config = LogConfig::new(log_path.to_string_lossy().to_string()).with_enabled(true);
        let context = LogContext::new("test", "ssh");

        let logger = SessionLogger::new(config, &context, None).unwrap();
        assert!(logger.is_enabled());
        assert!(logger.writer.is_some());
        assert!(log_path.exists());
    }

    #[test]
    fn test_session_logger_write() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("test.log");

        let config = LogConfig::new(log_path.to_string_lossy().to_string())
            .with_enabled(true)
            .with_log_timestamps(true);
        let log_ctx = LogContext::new("test", "ssh");

        let mut logger = SessionLogger::new(config, &log_ctx, None).unwrap();
        logger.write(b"Hello, World!").unwrap();
        logger.flush().unwrap();

        let log_content = fs::read_to_string(&log_path).unwrap();
        assert!(log_content.contains("Hello, World!"));
        assert!(log_content.contains('[') && log_content.contains(']')); // Has timestamp
    }

    #[test]
    fn test_session_logger_close() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("test.log");

        let config = LogConfig::new(log_path.to_string_lossy().to_string()).with_enabled(true);
        let log_ctx = LogContext::new("test", "ssh");

        let mut logger = SessionLogger::new(config, &log_ctx, None).unwrap();
        logger.write(b"Test data").unwrap();
        logger.close().unwrap();

        let log_content = fs::read_to_string(&log_path).unwrap();
        assert!(log_content.contains("Session ended"));
    }

    #[test]
    fn test_log_config_serialization() {
        let config = LogConfig::new("/tmp/test.log")
            .with_enabled(true)
            .with_timestamp_format("%H:%M:%S")
            .with_max_size_mb(5)
            .with_retention_days(7);

        let json = serde_json::to_string(&config).unwrap();
        let parsed: LogConfig = serde_json::from_str(&json).unwrap();

        assert_eq!(config, parsed);
    }

    #[test]
    fn test_format_timestamp() {
        let temp_dir = TempDir::new().unwrap();
        let log_path = temp_dir.path().join("test.log");

        let config = LogConfig::new(log_path.to_string_lossy().to_string())
            .with_enabled(true)
            .with_timestamp_format("%Y-%m-%d");
        let context = LogContext::new("test", "ssh");

        let logger = SessionLogger::new(config, &context, None).unwrap();
        let timestamp = logger.current_timestamp();

        // Should be in YYYY-MM-DD format
        assert_eq!(timestamp.len(), 10);
        assert_eq!(timestamp.chars().nth(4), Some('-'));
        assert_eq!(timestamp.chars().nth(7), Some('-'));
    }

    #[test]
    fn test_sanitize_output_disabled() {
        let config = SanitizeConfig::disabled();
        let input = "password: secret123";
        let result = sanitize_output(input, &config);
        assert_eq!(result, input);
    }

    #[test]
    fn test_sanitize_output_password_prompt() {
        let config = SanitizeConfig::new();
        let input = "password: mysecretpassword";
        let result = sanitize_output(input, &config);
        assert!(!result.contains("mysecretpassword"));
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_sanitize_output_api_key() {
        let config = SanitizeConfig::new();
        let input = "api_key: abcdef1234567890abcdef";
        let result = sanitize_output(input, &config);
        assert!(!result.contains("abcdef1234567890abcdef"));
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_sanitize_output_aws_key() {
        let config = SanitizeConfig::new();
        let input = "Found key: AKIAIOSFODNN7EXAMPLE";
        let result = sanitize_output(input, &config);
        assert!(!result.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_sanitize_output_private_key() {
        let config = SanitizeConfig::new();
        let input = "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEA...";
        let result = sanitize_output(input, &config);
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_sanitize_output_bearer_token() {
        let config = SanitizeConfig::new();
        let input = "Authorization: Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.test";
        let result = sanitize_output(input, &config);
        assert!(!result.contains("eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"));
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_sanitize_output_full_line() {
        let config = SanitizeConfig::new().with_full_line_sanitization(true);
        let input = "Enter password: \nNext line";
        let result = sanitize_output(input, &config);
        assert!(result.contains("[REDACTED]"));
        assert!(result.contains("Next line"));
    }

    #[test]
    fn test_sanitize_output_custom_pattern() {
        let config = SanitizeConfig::new().with_custom_pattern(r"secret_\d+");
        let input = "Found secret_12345 in config";
        let result = sanitize_output(input, &config);
        assert!(!result.contains("secret_12345"));
        assert!(result.contains("[REDACTED]"));
    }

    #[test]
    fn test_sanitize_output_custom_replacement() {
        let config = SanitizeConfig::new().with_replacement("***HIDDEN***");
        let input = "password: test123";
        let result = sanitize_output(input, &config);
        assert!(result.contains("***HIDDEN***"));
    }

    #[test]
    fn test_contains_sensitive_prompt() {
        assert!(contains_sensitive_prompt("Enter password:"));
        assert!(contains_sensitive_prompt("Password: "));
        assert!(contains_sensitive_prompt("Enter passphrase for key"));
        assert!(contains_sensitive_prompt("sudo password for root:"));
        assert!(!contains_sensitive_prompt("Hello, world!"));
        assert!(!contains_sensitive_prompt("Connection established"));
    }

    #[test]
    fn test_sanitize_preserves_newlines() {
        let config = SanitizeConfig::new();
        let input = "line1\npassword: secret\nline3\n";
        let result = sanitize_output(input, &config);
        assert!(result.ends_with('\n'));
        assert!(result.contains("line1"));
        assert!(result.contains("line3"));
    }

    // ===== Effective configuration resolution (issue #247) =====

    fn logging_settings(enabled: bool) -> crate::config::LoggingSettings {
        crate::config::LoggingSettings {
            enabled,
            ..Default::default()
        }
    }

    /// Creates `path` with a modification time `days` in the past.
    fn write_aged_file(path: &Path, days: u64) {
        let mut file = File::create(path).expect("create");
        file.write_all(b"x").ok();
        let when = std::time::SystemTime::now() - std::time::Duration::from_secs(days * 24 * 3600);
        file.set_modified(when).expect("backdate");
    }

    #[test]
    fn resolve_returns_none_when_logging_is_off_everywhere() {
        let dir = TempDir::new().expect("temp dir");
        assert!(
            LogConfig::resolve(&logging_settings(false), false, dir.path(), None).is_none(),
            "no logging configured anywhere must not open a log file"
        );
    }

    #[test]
    fn resolve_ignores_a_disabled_connection_config() {
        let dir = TempDir::new().expect("temp dir");
        let per_connection = LogConfig::new("/tmp/never.log").with_enabled(false);
        assert!(
            LogConfig::resolve(
                &logging_settings(false),
                false,
                dir.path(),
                Some(&per_connection)
            )
            .is_none()
        );
    }

    #[test]
    fn resolve_uses_the_connection_config_even_when_the_global_switch_is_off() {
        let dir = TempDir::new().expect("temp dir");
        let per_connection = LogConfig::new("${HOME}/logs/${connection_name}.log")
            .with_log_output(true)
            .with_log_timestamps(true);
        let resolved = LogConfig::resolve(
            &logging_settings(false),
            false,
            dir.path(),
            Some(&per_connection),
        )
        .expect("connection logging must win over the global switch");
        assert_eq!(
            resolved.path_template,
            "${HOME}/logs/${connection_name}.log"
        );
        assert!(resolved.log_output);
        assert!(
            resolved.log_timestamps,
            "the connection's own timestamp choice must survive"
        );
    }

    #[test]
    fn resolve_anchors_a_relative_connection_template_to_the_log_directory() {
        let dir = TempDir::new().expect("temp dir");
        let per_connection = LogConfig::new("${connection_name}.log");
        let resolved = LogConfig::resolve(
            &logging_settings(false),
            false,
            dir.path(),
            Some(&per_connection),
        )
        .expect("enabled connection config resolves");
        assert_eq!(
            resolved.path_template,
            dir.path()
                .join("${connection_name}.log")
                .to_string_lossy()
                .into_owned()
        );
    }

    #[test]
    fn resolve_falls_back_to_the_global_settings() {
        let dir = TempDir::new().expect("temp dir");
        let mut global = logging_settings(true);
        global.log_output = true;
        global.retention_days = 7;
        let resolved = LogConfig::resolve(&global, true, dir.path(), None)
            .expect("the global switch alone must arm logging");
        assert!(
            resolved
                .path_template
                .starts_with(&*dir.path().to_string_lossy())
        );
        assert!(
            resolved
                .path_template
                .ends_with("${connection_name}_${datetime}.log")
        );
        assert!(resolved.log_output);
        assert!(
            resolved.log_timestamps,
            "global timestamp switch is honored"
        );
        assert_eq!(resolved.retention_days, 7);
        assert_eq!(
            resolved.max_size_mb, 0,
            "the global UI has no size limit, so nothing rotates by size"
        );
    }

    #[test]
    fn expand_path_template_expands_a_leading_tilde() {
        let context = LogContext::new("host", "ssh");
        let home = dirs::home_dir().expect("home dir");
        let path = SessionLogger::expand_path_template("~/logs/session.log", &context, None)
            .expect("tilde expands");
        assert_eq!(path, home.join("logs/session.log"));
    }

    // ===== Retention (issue #247) =====

    #[test]
    fn prune_logs_removes_only_expired_log_files() {
        let dir = TempDir::new().expect("temp dir");
        let old = dir.path().join("old.log");
        let fresh = dir.path().join("fresh.log");
        let unrelated = dir.path().join("notes.txt");
        write_aged_file(&old, 10);
        write_aged_file(&fresh, 0);
        write_aged_file(&unrelated, 10);

        assert_eq!(prune_logs(dir.path(), 3), 1);
        assert!(!old.exists(), "an expired log is deleted");
        assert!(fresh.exists(), "a recent log is kept");
        assert!(unrelated.exists(), "non-log files are never touched");
    }

    #[test]
    fn prune_logs_keeps_everything_when_retention_is_disabled() {
        let dir = TempDir::new().expect("temp dir");
        let path = dir.path().join("ancient.log");
        write_aged_file(&path, 400);

        assert_eq!(prune_logs(dir.path(), 0), 0);
        assert!(path.exists(), "retention 0 means keep forever");
    }

    // ===== Redaction of what lands on disk (issue #247) =====

    #[test]
    fn write_record_redacts_a_credential_line() {
        let dir = TempDir::new().expect("temp dir");
        let config = LogConfig::new(dir.path().join("s.log").to_string_lossy().into_owned());
        let mut logger = SessionLogger::new(config, &LogContext::new("host", "ssh"), None)
            .expect("logger opens");
        logger
            .write_record("[10:00:00] INPUT: password: hunter2")
            .expect("write");
        logger.flush().expect("flush");

        let written = fs::read_to_string(logger.log_path()).expect("read back");
        assert!(
            !written.contains("hunter2"),
            "a typed password must not reach the log file: {written}"
        );
        assert!(written.contains("[REDACTED]"));
    }

    // ===== Redaction of the password typed at a prompt (issue #321) =====

    /// Builds an enabled logger over a fresh file in `dir`.
    fn prompt_logger(dir: &TempDir) -> SessionLogger {
        let config = LogConfig::new(dir.path().join("prompt.log").to_string_lossy().into_owned());
        SessionLogger::new(config, &LogContext::new("host", "ssh"), None).expect("logger opens")
    }

    /// The typed answer to a password prompt carries no marker of its own, yet
    /// must not reach the log — the prompt on the transcript arms the next
    /// `INPUT:` record for redaction.
    #[test]
    fn a_password_typed_after_a_prompt_is_redacted() {
        let dir = TempDir::new().expect("temp dir");
        let mut logger = prompt_logger(&dir);

        // Prompt travels on the transcript channel (`write`).
        logger.write(b"Password: ").expect("write prompt");
        // Answer travels on the `INPUT:` event channel (`write_record`).
        logger
            .write_record("INPUT: hunter2")
            .expect("write input record");
        logger.flush().expect("flush");

        let written = fs::read_to_string(logger.log_path()).expect("read back");
        assert!(
            !written.contains("hunter2"),
            "the password typed at the prompt must not reach the log: {written}"
        );
        assert!(written.contains("[REDACTED]"));
    }

    /// Only the first input line after the prompt is treated as the secret; a
    /// later command must be logged normally.
    #[test]
    fn only_the_line_immediately_after_the_prompt_is_redacted() {
        let dir = TempDir::new().expect("temp dir");
        let mut logger = prompt_logger(&dir);

        logger.write(b"Password: ").expect("write prompt");
        logger.write_record("INPUT: hunter2").expect("write secret");
        logger.write_record("INPUT: ls -la").expect("write command");
        logger.flush().expect("flush");

        let written = fs::read_to_string(logger.log_path()).expect("read back");
        assert!(!written.contains("hunter2"), "secret leaked: {written}");
        assert!(
            written.contains("ls -la"),
            "an ordinary command after the prompt must still be logged: {written}"
        );
    }

    /// A no-echo password prompt does not echo the password onto the transcript,
    /// so the transcript's own following line is legitimate output and must not
    /// be blanked by the arming state.
    #[test]
    fn transcript_output_after_a_prompt_is_not_over_redacted() {
        let dir = TempDir::new().expect("temp dir");
        let mut logger = prompt_logger(&dir);

        logger.write(b"Password: ").expect("write prompt");
        logger
            .write(b"Welcome to Ubuntu 24.04 LTS\n")
            .expect("write banner");
        logger.flush().expect("flush");

        let written = fs::read_to_string(logger.log_path()).expect("read back");
        assert!(
            written.contains("Welcome to Ubuntu 24.04 LTS"),
            "post-login output must survive: {written}"
        );
    }

    /// When a prompt is answered off-channel (a vault password sent straight to
    /// the PTY), transcript output flows and disarms the state, so the user's
    /// next typed command is not mistaken for the secret.
    #[test]
    fn a_command_after_an_off_channel_answer_is_not_redacted() {
        let dir = TempDir::new().expect("temp dir");
        let mut logger = prompt_logger(&dir);

        logger.write(b"Password: ").expect("write prompt");
        // The vault answer never reaches the logger; the server's response does.
        logger
            .write(b"Last login: Mon Sep  7\n")
            .expect("write output");
        logger.write_record("INPUT: ls -la").expect("write command");
        logger.flush().expect("flush");

        let written = fs::read_to_string(logger.log_path()).expect("read back");
        assert!(
            written.contains("ls -la"),
            "a command after an off-channel prompt answer must be logged: {written}"
        );
    }

    /// An input line with no preceding prompt is logged verbatim.
    #[test]
    fn input_without_a_preceding_prompt_is_not_redacted() {
        let dir = TempDir::new().expect("temp dir");
        let mut logger = prompt_logger(&dir);

        logger.write_record("INPUT: whoami").expect("write input");
        logger.flush().expect("flush");

        let written = fs::read_to_string(logger.log_path()).expect("read back");
        assert!(
            written.contains("whoami"),
            "ordinary input must be logged: {written}"
        );
    }

    // ===== ANSI stripping (issue #247, PTY relay) =====

    #[test]
    fn strip_ansi_removes_color_codes() {
        let input = "\x1b[32mHello\x1b[0m World";
        let result = super::strip_ansi_escapes(input);
        assert_eq!(result, "Hello World");
    }

    #[test]
    fn strip_ansi_removes_cursor_movement() {
        let input = "\x1b[2J\x1b[H\x1b[1;1Huser@host:~$";
        let result = super::strip_ansi_escapes(input);
        assert_eq!(result, "user@host:~$");
    }

    #[test]
    fn strip_ansi_preserves_newlines_and_tabs() {
        let input = "line1\n\tindented\r\nline3";
        let result = super::strip_ansi_escapes(input);
        assert_eq!(result, "line1\n\tindented\r\nline3");
    }

    #[test]
    fn strip_ansi_removes_osc_sequences() {
        // Window title: ESC ] 0 ; title BEL
        let input = "\x1b]0;user@host: ~\x07user@host:~$ ";
        let result = super::strip_ansi_escapes(input);
        assert_eq!(result, "user@host:~$ ");
    }

    #[test]
    fn strip_ansi_handles_ssh_debug_output() {
        // Typical ssh -v output doesn't have ANSI codes but may have control chars
        let input = "debug1: Connecting to host [192.168.1.1] port 22.\n";
        let result = super::strip_ansi_escapes(input);
        assert_eq!(result, input);
    }

    #[test]
    fn strip_ansi_removes_bold_and_underline() {
        let input = "\x1b[1mBold\x1b[4mUnderline\x1b[0m Normal";
        let result = super::strip_ansi_escapes(input);
        assert_eq!(result, "BoldUnderline Normal");
    }

    #[test]
    fn write_record_strips_ansi_from_output() {
        // The default configuration has `log_timestamps` off, which routes the
        // transcript through `write_record` rather than `write` — so this is the
        // path most session logs actually take.
        let dir = TempDir::new().expect("temp dir");
        let config = LogConfig::new(
            dir.path()
                .join("ansi-record.log")
                .to_string_lossy()
                .into_owned(),
        );
        let mut logger = SessionLogger::new(config, &LogContext::new("host", "ssh"), None)
            .expect("logger opens");
        logger
            .write_record("[10:00:00] OUTPUT:\n  \x1b[32mgreen text\x1b[0m normal")
            .expect("write record");
        logger.flush().expect("flush");

        let written = fs::read_to_string(logger.log_path()).expect("read back");
        assert!(
            !written.contains('\x1b'),
            "ANSI escapes must be stripped from a record too: {written}"
        );
        assert!(written.contains("green text normal"));
    }

    #[test]
    fn write_record_redacts_a_credential_hidden_behind_an_escape() {
        // Redaction runs after stripping precisely so a colour code inside the
        // prompt cannot split the pattern it looks for.
        let dir = TempDir::new().expect("temp dir");
        let config = LogConfig::new(
            dir.path()
                .join("ansi-redact.log")
                .to_string_lossy()
                .into_owned(),
        );
        let mut logger = SessionLogger::new(config, &LogContext::new("host", "ssh"), None)
            .expect("logger opens");
        logger
            .write_record("[10:00:00] INPUT: pass\x1b[0mword: hunter2")
            .expect("write record");
        logger.flush().expect("flush");

        let written = fs::read_to_string(logger.log_path()).expect("read back");
        assert!(
            !written.contains("hunter2"),
            "the credential must not survive: {written}"
        );
    }

    #[test]
    fn write_raw_keeps_escape_sequences() {
        // `write_raw` is the byte-for-byte path a replayable recording needs.
        let dir = TempDir::new().expect("temp dir");
        let config = LogConfig::new(dir.path().join("raw.log").to_string_lossy().into_owned());
        let mut logger = SessionLogger::new(config, &LogContext::new("host", "ssh"), None)
            .expect("logger opens");
        logger
            .write_raw(b"\x1b[32mgreen\x1b[0m")
            .expect("write raw");
        logger.flush().expect("flush");

        let written = fs::read_to_string(logger.log_path()).expect("read back");
        assert!(written.contains("\x1b[32m"), "raw writes must pass through");
    }

    #[test]
    fn write_strips_ansi_from_output() {
        let dir = TempDir::new().expect("temp dir");
        let config = LogConfig::new(dir.path().join("ansi.log").to_string_lossy().into_owned());
        let mut logger = SessionLogger::new(config, &LogContext::new("host", "ssh"), None)
            .expect("logger opens");
        logger
            .write(b"\x1b[32mgreen text\x1b[0m normal")
            .expect("write");
        logger.flush().expect("flush");

        let written = fs::read_to_string(logger.log_path()).expect("read back");
        assert!(
            !written.contains("\x1b["),
            "ANSI escapes must be stripped from log: {written}"
        );
        assert!(written.contains("green text"));
        assert!(written.contains("normal"));
    }
}
