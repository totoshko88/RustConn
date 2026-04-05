//! Host online check module
//!
//! Provides async TCP port probing to check if a remote host is reachable.
//! Used for "Check if online" feature and WoL + auto-connect integration.

use std::time::Duration;
use thiserror::Error;
use tokio::net::TcpStream;
use tokio::time::timeout;

/// Default TCP connect timeout in seconds
const DEFAULT_TIMEOUT_SECS: u64 = 3;

/// Default polling interval in seconds
const DEFAULT_POLL_INTERVAL_SECS: u64 = 5;

/// Maximum polling duration in seconds (10 minutes)
const MAX_POLL_DURATION_SECS: u64 = 600;

/// Host check error types
#[derive(Debug, Error)]
pub enum HostCheckError {
    /// DNS resolution failed
    #[error("DNS resolution failed for {host}: {source}")]
    DnsResolution {
        /// Hostname that failed to resolve
        host: String,
        /// Underlying error
        source: std::io::Error,
    },
    /// Connection timed out
    #[error("Connection to {host}:{port} timed out after {timeout_secs}s")]
    Timeout {
        /// Target host
        host: String,
        /// Target port
        port: u16,
        /// Timeout duration
        timeout_secs: u64,
    },
    /// Connection refused or unreachable
    #[error("Host {host}:{port} is unreachable: {source}")]
    Unreachable {
        /// Target host
        host: String,
        /// Target port
        port: u16,
        /// Underlying error
        source: std::io::Error,
    },
    /// Polling was cancelled
    #[error("Host check cancelled")]
    Cancelled,
}

/// Result type for host check operations
pub type HostCheckResult<T> = Result<T, HostCheckError>;

/// Status of a host
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostStatus {
    /// Host is reachable (TCP connect succeeded)
    Online,
    /// Host is not reachable
    Offline,
    /// Check is in progress
    Checking,
}

/// Configuration for host online check
#[derive(Debug, Clone)]
pub struct HostCheckConfig {
    /// Target hostname or IP
    pub host: String,
    /// Target port to probe
    pub port: u16,
    /// TCP connect timeout in seconds
    pub timeout_secs: u64,
    /// Polling interval in seconds (for continuous monitoring)
    pub poll_interval_secs: u64,
    /// Maximum polling duration in seconds (0 = no limit)
    pub max_poll_duration_secs: u64,
}

impl Default for HostCheckConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 22,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
            max_poll_duration_secs: MAX_POLL_DURATION_SECS,
        }
    }
}

impl HostCheckConfig {
    /// Creates a new config for the given host and port
    #[must_use]
    pub fn new(host: impl Into<String>, port: u16) -> Self {
        Self {
            host: host.into(),
            port,
            ..Default::default()
        }
    }

    /// Sets the TCP connect timeout
    #[must_use]
    pub const fn with_timeout_secs(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Sets the polling interval
    #[must_use]
    pub const fn with_poll_interval_secs(mut self, secs: u64) -> Self {
        self.poll_interval_secs = secs;
        self
    }

    /// Sets the maximum polling duration
    #[must_use]
    pub const fn with_max_poll_duration_secs(mut self, secs: u64) -> Self {
        self.max_poll_duration_secs = secs;
        self
    }
}

/// Performs a single TCP connect probe to check if a host:port is reachable.
///
/// Returns `Ok(true)` if the connection succeeded, `Ok(false)` if it timed out
/// or was refused.
///
/// # Errors
///
/// Returns an error only for DNS resolution failures.
pub async fn check_host_online(host: &str, port: u16, timeout_secs: u64) -> HostCheckResult<bool> {
    let addr = format!("{host}:{port}");
    let connect_timeout = Duration::from_secs(timeout_secs.max(1));

    match timeout(connect_timeout, TcpStream::connect(&addr)).await {
        Ok(Ok(_stream)) => Ok(true),
        Ok(Err(e)) => {
            // Connection refused, network unreachable, etc.
            if e.kind() == std::io::ErrorKind::Other
                || e.to_string().contains("resolve")
                || e.to_string().contains("dns")
            {
                Err(HostCheckError::DnsResolution {
                    host: host.to_string(),
                    source: e,
                })
            } else {
                Ok(false)
            }
        }
        Err(_elapsed) => Ok(false), // Timeout = offline
    }
}

/// Polls a host until it comes online or the maximum duration is reached.
///
/// Calls `on_status` with each probe result. Returns `true` if the host
/// came online within the time limit, `false` if it didn't.
///
/// # Arguments
///
/// * `config` - Host check configuration
/// * `cancel` - Cancellation token (set to `true` to stop polling)
/// * `on_status` - Callback invoked after each probe with `(is_online, elapsed_secs)`
///
/// # Errors
///
/// Returns an error if DNS resolution fails or polling is cancelled.
pub async fn poll_until_online<F>(
    config: &HostCheckConfig,
    cancel: &std::sync::atomic::AtomicBool,
    on_status: F,
) -> HostCheckResult<bool>
where
    F: Fn(bool, u64),
{
    let start = std::time::Instant::now();
    let poll_interval = Duration::from_secs(config.poll_interval_secs.max(1));
    let max_duration = if config.max_poll_duration_secs == 0 {
        Duration::from_secs(u64::MAX)
    } else {
        Duration::from_secs(config.max_poll_duration_secs)
    };

    loop {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(HostCheckError::Cancelled);
        }

        let elapsed = start.elapsed();
        if elapsed >= max_duration {
            on_status(false, elapsed.as_secs());
            return Ok(false);
        }

        let is_online = check_host_online(&config.host, config.port, config.timeout_secs).await?;
        on_status(is_online, elapsed.as_secs());

        if is_online {
            return Ok(true);
        }

        tokio::time::sleep(poll_interval).await;
    }
}

/// Action to take when a host comes online
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnlineAction {
    /// Just notify the user
    Notify,
    /// Automatically connect to the host
    AutoConnect,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_host_check_config_default() {
        let config = HostCheckConfig::default();
        assert_eq!(config.timeout_secs, DEFAULT_TIMEOUT_SECS);
        assert_eq!(config.poll_interval_secs, DEFAULT_POLL_INTERVAL_SECS);
        assert_eq!(config.max_poll_duration_secs, MAX_POLL_DURATION_SECS);
    }

    #[test]
    fn test_host_check_config_builder() {
        let config = HostCheckConfig::new("example.com", 22)
            .with_timeout_secs(5)
            .with_poll_interval_secs(10)
            .with_max_poll_duration_secs(300);
        assert_eq!(config.host, "example.com");
        assert_eq!(config.port, 22);
        assert_eq!(config.timeout_secs, 5);
        assert_eq!(config.poll_interval_secs, 10);
        assert_eq!(config.max_poll_duration_secs, 300);
    }

    #[test]
    fn test_host_status_equality() {
        assert_eq!(HostStatus::Online, HostStatus::Online);
        assert_ne!(HostStatus::Online, HostStatus::Offline);
    }

    #[tokio::test]
    async fn test_check_host_offline() {
        // Connect to a port that's almost certainly not listening
        let result = check_host_online("127.0.0.1", 19999, 1).await;
        assert!(result.is_ok());
        assert!(!result.unwrap_or(true)); // Should be offline
    }
}
