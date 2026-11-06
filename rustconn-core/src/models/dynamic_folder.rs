//! Dynamic folder model for script-generated connections.
//!
//! A dynamic folder executes a user-defined script that returns a JSON array
//! of connection definitions. The connections are read-only and refreshed
//! on demand or at a configurable interval.

use std::path::PathBuf;
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A dynamic folder configuration attached to a [`super::ConnectionGroup`].
///
/// When a group has a `DynamicFolderConfig`, its child connections are generated
/// by executing `script` and parsing the JSON output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DynamicFolderConfig {
    /// Shell script or command to execute (run via `sh -c`)
    pub script: String,

    /// Optional working directory for the script
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub working_directory: Option<PathBuf>,

    /// Auto-refresh interval in seconds (None = manual only)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_interval_secs: Option<u64>,

    /// Maximum time to wait for the script to complete
    #[serde(default = "DynamicFolderConfig::default_timeout_secs")]
    pub timeout_secs: u64,

    /// Timestamp of the last successful refresh
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refreshed_at: Option<DateTime<Utc>>,

    /// Error message from the last failed refresh (cleared on success)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

impl DynamicFolderConfig {
    /// Creates a new dynamic folder config with the given script
    #[must_use]
    pub fn new(script: String) -> Self {
        Self {
            script,
            working_directory: None,
            refresh_interval_secs: None,
            timeout_secs: Self::default_timeout_secs(),
            last_refreshed_at: None,
            last_error: None,
        }
    }

    /// Default script timeout: 30 seconds
    #[must_use]
    pub const fn default_timeout_secs() -> u64 {
        30
    }

    /// Returns the refresh interval as a `Duration`, if configured
    #[must_use]
    pub fn refresh_interval(&self) -> Option<Duration> {
        self.refresh_interval_secs.map(Duration::from_secs)
    }

    /// Returns the timeout as a `Duration`
    #[must_use]
    pub fn timeout(&self) -> Duration {
        Duration::from_secs(self.timeout_secs)
    }
}

/// A single connection entry returned by a dynamic folder script.
///
/// Scripts output a JSON array of these objects. Only `name` and `host`
/// are required; everything else has sensible defaults.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DynamicConnectionEntry {
    /// Connection display name (required)
    pub name: String,

    /// Hostname or IP address (required)
    pub host: String,

    /// Port number (defaults to protocol default)
    #[serde(default)]
    pub port: Option<u16>,

    /// Protocol: "ssh", "rdp", "vnc", "telnet", etc. (default: "ssh")
    #[serde(default = "DynamicConnectionEntry::default_protocol")]
    pub protocol: String,

    /// Username for authentication
    #[serde(default)]
    pub username: Option<String>,

    /// Sub-group path within the dynamic folder (e.g. "web-servers/production")
    #[serde(default)]
    pub group: Option<String>,

    /// Tags for filtering
    #[serde(default)]
    pub tags: Vec<String>,

    /// Optional description
    #[serde(default)]
    pub description: Option<String>,

    /// Full protocol-specific configuration, in the native RustConn
    /// [`ProtocolConfig`] shape (issue
    /// [#410](https://github.com/totoshko88/RustConn/issues/410)).
    ///
    /// When present it is the base configuration the generated connection is
    /// built from, so a script can set anything the connection editor can — an
    /// SSH `startup_command` (e.g. `tmux new-session -A -s my-session`), an
    /// `auth_method`, RDP display options, and so on — instead of being limited
    /// to host/port/username. The JSON is the same tagged shape the config file
    /// uses, e.g. `{"type": "Ssh", "startup_command": "...", "auth_method":
    /// "agent"}`.
    ///
    /// When absent (the default, and every entry written before this field
    /// existed), the connection is built from the protocol defaults exactly as
    /// before. When present, its tag decides the protocol; the `protocol`
    /// string is then only used to warn on a mismatch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol_config: Option<super::ProtocolConfig>,
}

impl DynamicConnectionEntry {
    fn default_protocol() -> String {
        "ssh".to_string()
    }
}

/// Result of executing a dynamic folder script
#[derive(Debug, Clone)]
pub struct DynamicFolderResult {
    /// Successfully parsed connection entries
    pub entries: Vec<DynamicConnectionEntry>,

    /// Warnings encountered during parsing (non-fatal)
    pub warnings: Vec<String>,

    /// Execution duration
    pub duration: Duration,
}
