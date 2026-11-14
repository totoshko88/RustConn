//! Connection history and statistics models
//!
//! This module provides models for tracking connection history and statistics.

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A single connection history entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionHistoryEntry {
    /// Unique identifier for this history entry
    pub id: Uuid,
    /// Connection ID this entry is for
    pub connection_id: Uuid,
    /// Connection name at the time of connection
    pub connection_name: String,
    /// Host connected to
    pub host: String,
    /// Port used
    pub port: u16,
    /// Protocol used (ssh, rdp, vnc, spice)
    pub protocol: String,
    /// Username used (if any)
    pub username: Option<String>,
    /// When the connection was started
    pub started_at: DateTime<Utc>,
    /// When the connection ended (None if still active)
    pub ended_at: Option<DateTime<Utc>>,
    /// Whether the connection was successful
    pub successful: bool,
    /// Error message if connection failed
    pub error_message: Option<String>,
    /// Session duration in seconds (calculated when ended)
    pub duration_seconds: Option<i64>,
}

impl ConnectionHistoryEntry {
    /// Creates a new history entry for a connection start
    #[must_use]
    pub fn new(
        connection_id: Uuid,
        connection_name: String,
        host: String,
        port: u16,
        protocol: String,
        username: Option<String>,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            connection_id,
            connection_name,
            host,
            port,
            protocol,
            username,
            started_at: Utc::now(),
            ended_at: None,
            successful: true,
            error_message: None,
            duration_seconds: None,
        }
    }

    /// Creates a new history entry for a quick connect (no saved connection)
    #[must_use]
    pub fn new_quick_connect(
        host: String,
        port: u16,
        protocol: String,
        username: Option<String>,
    ) -> Self {
        Self::new(
            Uuid::nil(),
            format!("Quick: {host}"),
            host,
            port,
            protocol,
            username,
        )
    }

    /// Marks the connection as ended
    pub fn end(&mut self) {
        let now = Utc::now();
        self.ended_at = Some(now);
        self.duration_seconds = Some((now - self.started_at).num_seconds());
    }

    /// Marks the connection as failed with an error message
    pub fn fail(&mut self, error: impl Into<String>) {
        self.successful = false;
        self.error_message = Some(error.into());
        self.end();
    }

    /// Returns the duration of the connection
    #[must_use]
    pub fn duration(&self) -> Option<Duration> {
        self.duration_seconds.map(Duration::seconds)
    }

    /// Returns true if this is a quick connect entry (no saved connection)
    #[must_use]
    pub fn is_quick_connect(&self) -> bool {
        self.connection_id.is_nil()
    }
}

/// Connection statistics for a single connection
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ConnectionStatistics {
    /// Connection ID these statistics are for
    pub connection_id: Uuid,
    /// Total number of connection attempts
    pub total_connections: u32,
    /// Number of successful connections
    pub successful_connections: u32,
    /// Number of failed connections
    pub failed_connections: u32,
    /// Total time connected in seconds
    pub total_duration_seconds: i64,
    /// Average session duration in seconds
    pub average_duration_seconds: i64,
    /// Longest session duration in seconds
    pub longest_session_seconds: i64,
    /// Shortest session duration in seconds (excluding failed)
    pub shortest_session_seconds: Option<i64>,
    /// Last connection timestamp
    pub last_connected: Option<DateTime<Utc>>,
    /// First connection timestamp
    pub first_connected: Option<DateTime<Utc>>,
}

impl ConnectionStatistics {
    /// Creates new empty statistics for a connection
    #[must_use]
    pub const fn new(connection_id: Uuid) -> Self {
        Self {
            connection_id,
            total_connections: 0,
            successful_connections: 0,
            failed_connections: 0,
            total_duration_seconds: 0,
            average_duration_seconds: 0,
            longest_session_seconds: 0,
            shortest_session_seconds: None,
            last_connected: None,
            first_connected: None,
        }
    }

    /// Updates statistics with a completed history entry
    pub fn update_from_entry(&mut self, entry: &ConnectionHistoryEntry) {
        self.total_connections += 1;

        if entry.successful {
            self.successful_connections += 1;

            if let Some(duration) = entry.duration_seconds {
                self.total_duration_seconds += duration;
                self.longest_session_seconds = self.longest_session_seconds.max(duration);

                self.shortest_session_seconds = Some(
                    self.shortest_session_seconds
                        .map_or(duration, |s| s.min(duration)),
                );

                // Recalculate average
                if self.successful_connections > 0 {
                    self.average_duration_seconds =
                        self.total_duration_seconds / i64::from(self.successful_connections);
                }
            }
        } else {
            self.failed_connections += 1;
        }

        // Update timestamps
        self.last_connected = Some(entry.started_at);
        if self.first_connected.is_none() {
            self.first_connected = Some(entry.started_at);
        }
    }

    /// Returns the success rate as a percentage (0-100)
    #[must_use]
    pub fn success_rate(&self) -> f64 {
        if self.total_connections == 0 {
            return 0.0;
        }
        f64::from(self.successful_connections) / f64::from(self.total_connections) * 100.0
    }

    /// Returns the average duration as a Duration
    #[must_use]
    pub fn average_duration(&self) -> Duration {
        Duration::seconds(self.average_duration_seconds)
    }

    /// Returns the total duration as a Duration
    #[must_use]
    pub fn total_duration(&self) -> Duration {
        Duration::seconds(self.total_duration_seconds)
    }

    /// Formats duration in human-readable format (e.g., "2h 30m")
    #[must_use]
    pub fn format_duration(seconds: i64) -> String {
        if seconds < 60 {
            return format!("{seconds}s");
        }

        let hours = seconds / 3600;
        let minutes = (seconds % 3600) / 60;
        let secs = seconds % 60;

        if hours > 0 {
            if minutes > 0 {
                format!("{hours}h {minutes}m")
            } else {
                format!("{hours}h")
            }
        } else if secs > 0 {
            format!("{minutes}m {secs}s")
        } else {
            format!("{minutes}m")
        }
    }
}

/// Global connection history settings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistorySettings {
    /// Whether history tracking is enabled
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Maximum number of history entries to keep
    #[serde(default = "default_max_entries")]
    pub max_entries: usize,
    /// Number of days to retain history
    #[serde(default = "default_retention_days")]
    pub retention_days: u32,
}

const fn default_true() -> bool {
    true
}

const fn default_max_entries() -> usize {
    1000
}

const fn default_retention_days() -> u32 {
    90
}

impl Default for HistorySettings {
    fn default() -> Self {
        Self {
            enabled: true,
            max_entries: default_max_entries(),
            retention_days: default_retention_days(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_settings_load_a_config_with_the_retired_key() {
        // `track_quick_connect` was never read; configs that still hold it must load.
        let s: HistorySettings = toml::from_str(
            "enabled = false\nmax_entries = 5\nretention_days = 7\ntrack_quick_connect = false\n",
        )
        .unwrap();
        assert!(!s.enabled);
        assert_eq!(s.max_entries, 5);
        assert_eq!(s.retention_days, 7);
    }

    #[test]
    fn test_history_entry_creation() {
        let entry = ConnectionHistoryEntry::new(
            Uuid::new_v4(),
            "Test Server".to_string(),
            "example.com".to_string(),
            22,
            "ssh".to_string(),
            Some("user".to_string()),
        );

        assert!(entry.successful);
        assert!(entry.ended_at.is_none());
        assert!(!entry.is_quick_connect());
    }

    #[test]
    fn test_quick_connect_entry() {
        let entry = ConnectionHistoryEntry::new_quick_connect(
            "example.com".to_string(),
            22,
            "ssh".to_string(),
            None,
        );

        assert!(entry.is_quick_connect());
        assert!(entry.connection_name.starts_with("Quick:"));
    }

    #[test]
    fn test_entry_end() {
        let mut entry = ConnectionHistoryEntry::new(
            Uuid::new_v4(),
            "Test".to_string(),
            "host".to_string(),
            22,
            "ssh".to_string(),
            None,
        );

        entry.end();

        assert!(entry.ended_at.is_some());
        assert!(entry.duration_seconds.is_some());
    }

    #[test]
    fn test_statistics_update() {
        let mut stats = ConnectionStatistics::new(Uuid::new_v4());
        let mut entry = ConnectionHistoryEntry::new(
            stats.connection_id,
            "Test".to_string(),
            "host".to_string(),
            22,
            "ssh".to_string(),
            None,
        );

        entry.duration_seconds = Some(3600); // 1 hour
        entry.ended_at = Some(Utc::now());

        stats.update_from_entry(&entry);

        assert_eq!(stats.total_connections, 1);
        assert_eq!(stats.successful_connections, 1);
        assert_eq!(stats.total_duration_seconds, 3600);
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(ConnectionStatistics::format_duration(30), "30s");
        assert_eq!(ConnectionStatistics::format_duration(90), "1m 30s");
        assert_eq!(ConnectionStatistics::format_duration(3600), "1h");
        assert_eq!(ConnectionStatistics::format_duration(5400), "1h 30m");
    }

    #[test]
    fn test_success_rate() {
        let mut stats = ConnectionStatistics::new(Uuid::new_v4());
        stats.total_connections = 10;
        stats.successful_connections = 8;
        stats.failed_connections = 2;

        assert!((stats.success_rate() - 80.0).abs() < f64::EPSILON);
    }
}
