//! Monitoring settings for remote host metrics collection
//!
//! Global settings live in `AppSettings.monitoring` and control defaults.
//! Per-connection overrides use `MonitoringConfig` on the `Connection` struct.

use serde::{Deserialize, Serialize};

/// Global monitoring settings (stored in `config.toml` under `[monitoring]`)
#[allow(clippy::struct_excessive_bools)] // Settings struct — bools are independent toggles, not a state machine
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitoringSettings {
    /// Whether remote monitoring is enabled globally (default: true)
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Polling interval in seconds (1–60, default: 3)
    #[serde(default = "default_interval_secs")]
    pub interval_secs: u8,
    /// Show CPU usage in the monitoring bar
    #[serde(default = "default_true")]
    pub show_cpu: bool,
    /// Show memory usage in the monitoring bar
    #[serde(default = "default_true")]
    pub show_memory: bool,
    /// Show disk usage in the monitoring bar
    #[serde(default = "default_true")]
    pub show_disk: bool,
    /// Show network throughput in the monitoring bar
    #[serde(default = "default_true")]
    pub show_network: bool,
    /// Show load average in the monitoring bar
    #[serde(default = "default_true")]
    pub show_load: bool,
    /// Show system info (distro, kernel, uptime) in the monitoring bar
    #[serde(default = "default_true")]
    pub show_system_info: bool,
}

const fn default_interval_secs() -> u8 {
    3
}

const fn default_true() -> bool {
    true
}

impl Default for MonitoringSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            interval_secs: default_interval_secs(),
            show_cpu: true,
            show_memory: true,
            show_disk: true,
            show_network: true,
            show_load: true,
            show_system_info: true,
        }
    }
}

impl MonitoringSettings {
    /// Returns the interval clamped to the valid range (1–60 seconds)
    #[must_use]
    pub const fn effective_interval_secs(&self) -> u8 {
        if self.interval_secs == 0 {
            1
        } else if self.interval_secs > 60 {
            60
        } else {
            self.interval_secs
        }
    }
}

/// Per-connection monitoring override (stored on `Connection`)
///
/// When `None` on a connection, the global `MonitoringSettings` apply.
/// When `Some`, these values override the global defaults.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonitoringConfig {
    /// Override the global enabled flag for this connection
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    /// Override the polling interval for this connection
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub interval_secs: Option<u8>,
}

impl MonitoringConfig {
    /// Returns whether monitoring is enabled, falling back to global setting
    #[must_use]
    pub fn is_enabled(&self, global: &MonitoringSettings) -> bool {
        self.enabled.unwrap_or(global.enabled)
    }

    /// Returns the effective interval, falling back to global setting
    #[must_use]
    pub fn effective_interval(&self, global: &MonitoringSettings) -> u8 {
        let secs = self
            .interval_secs
            .unwrap_or_else(|| global.effective_interval_secs());
        secs.clamp(1, 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let s = MonitoringSettings::default();
        assert!(s.enabled);
        assert_eq!(s.interval_secs, 3);
        assert!(s.show_cpu);
        assert!(s.show_memory);
        assert!(s.show_disk);
        assert!(s.show_network);
        assert!(s.show_load);
        assert!(s.show_system_info);
    }

    #[test]
    fn test_effective_interval_clamping() {
        let s = MonitoringSettings {
            interval_secs: 0,
            ..Default::default()
        };
        assert_eq!(s.effective_interval_secs(), 1);

        let s = MonitoringSettings {
            interval_secs: 255,
            ..Default::default()
        };
        assert_eq!(s.effective_interval_secs(), 60);

        let s = MonitoringSettings {
            interval_secs: 5,
            ..Default::default()
        };
        assert_eq!(s.effective_interval_secs(), 5);
    }

    #[test]
    fn test_per_connection_override() {
        let global = MonitoringSettings {
            enabled: true,
            interval_secs: 5,
            ..Default::default()
        };
        let config = MonitoringConfig {
            enabled: Some(false),
            interval_secs: Some(10),
        };
        assert!(!config.is_enabled(&global));
        assert_eq!(config.effective_interval(&global), 10);
    }

    #[test]
    fn test_per_connection_fallback() {
        let global = MonitoringSettings {
            enabled: true,
            interval_secs: 7,
            ..Default::default()
        };
        let config = MonitoringConfig {
            enabled: None,
            interval_secs: None,
        };
        assert!(config.is_enabled(&global));
        assert_eq!(config.effective_interval(&global), 7);
    }

    #[test]
    fn test_serde_roundtrip() {
        let settings = MonitoringSettings {
            enabled: true,
            interval_secs: 10,
            show_cpu: true,
            show_memory: false,
            show_disk: true,
            show_network: false,
            show_load: true,
            show_system_info: false,
        };
        let json = serde_json::to_string(&settings).unwrap();
        let deserialized: MonitoringSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(settings, deserialized);
    }
}
