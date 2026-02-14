//! Dashboard data types for session monitoring
//!
//! This module provides core data types for the connection dashboard,
//! including session statistics and filtering capabilities.
//!
//! **Validates: Requirements 13.1, 13.2, 13.5**

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::session::SessionState;

/// Session statistics for dashboard display
/// **Validates: Requirements 13.2**
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStats {
    /// Session ID
    pub session_id: Uuid,
    /// Connection ID
    pub connection_id: Uuid,
    /// Connection name
    pub connection_name: String,
    /// Protocol (ssh, rdp, vnc, spice)
    pub protocol: String,
    /// Session state
    pub state: SessionState,
    /// When the session started
    pub started_at: DateTime<Utc>,
    /// Bytes sent
    pub bytes_sent: u64,
    /// Bytes received
    pub bytes_received: u64,
    /// Host address
    pub host: String,
    /// Group ID (if any)
    pub group_id: Option<Uuid>,
}

impl SessionStats {
    /// Creates new session stats
    #[must_use]
    pub fn new(
        session_id: Uuid,
        connection_id: Uuid,
        connection_name: String,
        protocol: String,
        host: String,
    ) -> Self {
        Self {
            session_id,
            connection_id,
            connection_name,
            protocol,
            state: SessionState::Active,
            started_at: Utc::now(),
            bytes_sent: 0,
            bytes_received: 0,
            host,
            group_id: None,
        }
    }

    /// Returns the connection duration
    /// **Validates: Requirements 13.2**
    #[must_use]
    pub fn duration(&self) -> Duration {
        Utc::now().signed_duration_since(self.started_at)
    }

    /// Returns the duration in seconds (for testing)
    #[must_use]
    pub fn duration_seconds(&self) -> i64 {
        self.duration().num_seconds().max(0)
    }

    /// Formats the duration as a human-readable string
    /// **Validates: Requirements 13.2**
    #[must_use]
    pub fn format_duration(&self) -> String {
        let total_seconds = self.duration_seconds();

        let hours = total_seconds / 3600;
        let minutes = (total_seconds % 3600) / 60;
        let seconds = total_seconds % 60;

        if hours > 0 {
            format!("{hours}h {minutes}m {seconds}s")
        } else if minutes > 0 {
            format!("{minutes}m {seconds}s")
        } else {
            format!("{seconds}s")
        }
    }

    /// Formats bytes as human-readable string
    /// **Validates: Requirements 13.2**
    #[must_use]
    pub fn format_bytes(bytes: u64) -> String {
        const KB: u64 = 1024;
        const MB: u64 = KB * 1024;
        const GB: u64 = MB * 1024;

        if bytes >= GB {
            format!("{:.2} GB", bytes as f64 / GB as f64)
        } else if bytes >= MB {
            format!("{:.2} MB", bytes as f64 / MB as f64)
        } else if bytes >= KB {
            format!("{:.2} KB", bytes as f64 / KB as f64)
        } else {
            format!("{bytes} B")
        }
    }

    /// Returns the state as a display string
    #[must_use]
    pub const fn state_display(&self) -> &'static str {
        match self.state {
            SessionState::Starting => "Starting",
            SessionState::Active => "Connected",
            SessionState::Disconnecting => "Disconnecting",
            SessionState::Terminated => "Disconnected",
            SessionState::Error => "Error",
        }
    }

    /// Updates bytes sent
    pub const fn add_bytes_sent(&mut self, bytes: u64) {
        self.bytes_sent = self.bytes_sent.saturating_add(bytes);
    }

    /// Updates bytes received
    pub const fn add_bytes_received(&mut self, bytes: u64) {
        self.bytes_received = self.bytes_received.saturating_add(bytes);
    }
}

/// Dashboard filter criteria
/// **Validates: Requirements 13.5**
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct DashboardFilter {
    /// Filter by protocol (None = all protocols)
    pub protocol: Option<String>,
    /// Filter by group ID (None = all groups)
    pub group_id: Option<Uuid>,
    /// Filter by status (None = all statuses)
    pub status: Option<SessionState>,
}

impl DashboardFilter {
    /// Creates a new empty filter (shows all sessions)
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the protocol filter
    #[must_use]
    pub fn with_protocol(mut self, protocol: Option<String>) -> Self {
        self.protocol = protocol;
        self
    }

    /// Sets the group filter
    #[must_use]
    pub const fn with_group(mut self, group_id: Option<Uuid>) -> Self {
        self.group_id = group_id;
        self
    }

    /// Sets the status filter
    #[must_use]
    pub const fn with_status(mut self, status: Option<SessionState>) -> Self {
        self.status = status;
        self
    }

    /// Checks if a session matches this filter
    /// **Validates: Requirements 13.5**
    #[must_use]
    pub fn matches(&self, stats: &SessionStats) -> bool {
        // Check protocol filter
        if let Some(ref protocol) = self.protocol {
            if &stats.protocol != protocol {
                return false;
            }
        }

        // Check group filter
        if let Some(group_id) = self.group_id {
            if stats.group_id != Some(group_id) {
                return false;
            }
        }

        // Check status filter
        if let Some(status) = self.status {
            if stats.state != status {
                return false;
            }
        }

        true
    }

    /// Filters a list of session stats
    /// **Validates: Requirements 13.5**
    #[must_use]
    pub fn apply(&self, sessions: &[SessionStats]) -> Vec<SessionStats> {
        sessions
            .iter()
            .filter(|s| self.matches(s))
            .cloned()
            .collect()
    }

    /// Returns the count of sessions matching this filter
    #[must_use]
    pub fn count_matches(&self, sessions: &[SessionStats]) -> usize {
        sessions.iter().filter(|s| self.matches(s)).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_bytes() {
        assert_eq!(SessionStats::format_bytes(0), "0 B");
        assert_eq!(SessionStats::format_bytes(512), "512 B");
        assert_eq!(SessionStats::format_bytes(1024), "1.00 KB");
        assert_eq!(SessionStats::format_bytes(1536), "1.50 KB");
        assert_eq!(SessionStats::format_bytes(1_048_576), "1.00 MB");
        assert_eq!(SessionStats::format_bytes(1_073_741_824), "1.00 GB");
    }

    #[test]
    fn test_filter_matches_all() {
        let filter = DashboardFilter::new();
        let stats = SessionStats::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "Test".to_string(),
            "ssh".to_string(),
            "localhost".to_string(),
        );
        assert!(filter.matches(&stats));
    }

    #[test]
    fn test_filter_by_protocol() {
        let filter = DashboardFilter::new().with_protocol(Some("ssh".to_string()));

        let ssh_stats = SessionStats::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "SSH Test".to_string(),
            "ssh".to_string(),
            "localhost".to_string(),
        );

        let rdp_stats = SessionStats::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            "RDP Test".to_string(),
            "rdp".to_string(),
            "localhost".to_string(),
        );

        assert!(filter.matches(&ssh_stats));
        assert!(!filter.matches(&rdp_stats));
    }
}
