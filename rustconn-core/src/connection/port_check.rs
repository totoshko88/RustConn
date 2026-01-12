//! Pre-connect TCP port check utility
//!
//! Provides fast TCP port reachability check before launching external clients
//! (RDP, VNC, SPICE) to give faster feedback when hosts are unreachable.

use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::time::Duration;
use thiserror::Error;

/// Error type for port check operations
#[derive(Debug, Error)]
pub enum PortCheckError {
    /// Host resolution failed
    #[error("Failed to resolve host '{host}': {reason}")]
    ResolutionFailed {
        /// The hostname that failed to resolve
        host: String,
        /// The reason for the failure
        reason: String,
    },
    /// Connection refused or timed out
    #[error("Port {port} on '{host}' is not reachable: {reason}")]
    Unreachable {
        /// The hostname that was unreachable
        host: String,
        /// The port that was unreachable
        port: u16,
        /// The reason for the failure
        reason: String,
    },
}

/// Result of a port check operation
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortCheckResult {
    /// Port is open and accepting connections
    Open,
    /// Port check was skipped (disabled or not applicable)
    Skipped,
}

/// Checks if a TCP port is reachable on the given host
///
/// # Arguments
/// * `host` - Hostname or IP address
/// * `port` - TCP port number
/// * `timeout_secs` - Connection timeout in seconds
///
/// # Returns
/// * `Ok(PortCheckResult::Open)` if the port is reachable
///
/// # Errors
/// * `PortCheckError::ResolutionFailed` if the hostname cannot be resolved
/// * `PortCheckError::Unreachable` if the port is not reachable or connection timed out
pub fn check_port(
    host: &str,
    port: u16,
    timeout_secs: u32,
) -> Result<PortCheckResult, PortCheckError> {
    let timeout = Duration::from_secs(u64::from(timeout_secs));
    let addr_str = format!("{host}:{port}");

    // Resolve hostname to socket addresses
    let addrs: Vec<SocketAddr> = addr_str
        .to_socket_addrs()
        .map_err(|e| PortCheckError::ResolutionFailed {
            host: host.to_string(),
            reason: e.to_string(),
        })?
        .collect();

    if addrs.is_empty() {
        return Err(PortCheckError::ResolutionFailed {
            host: host.to_string(),
            reason: "No addresses found".to_string(),
        });
    }

    // Try each resolved address
    let mut last_error = String::new();
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, timeout) {
            Ok(_stream) => {
                // Connection successful, port is open
                return Ok(PortCheckResult::Open);
            }
            Err(e) => {
                last_error = e.to_string();
                // Continue trying other addresses
            }
        }
    }

    Err(PortCheckError::Unreachable {
        host: host.to_string(),
        port,
        reason: last_error,
    })
}

/// Async version of port check using tokio
///
/// # Arguments
/// * `host` - Hostname or IP address
/// * `port` - TCP port number
/// * `timeout_secs` - Connection timeout in seconds
///
/// # Returns
/// * `Ok(PortCheckResult::Open)` if the port is reachable
///
/// # Errors
/// * `PortCheckError::ResolutionFailed` if the hostname cannot be resolved
/// * `PortCheckError::Unreachable` if the port is not reachable or connection timed out
pub async fn check_port_async(
    host: &str,
    port: u16,
    timeout_secs: u32,
) -> Result<PortCheckResult, PortCheckError> {
    let timeout = Duration::from_secs(u64::from(timeout_secs));
    let addr_str = format!("{host}:{port}");

    // Resolve hostname (blocking, but fast)
    let addrs: Vec<SocketAddr> = addr_str
        .to_socket_addrs()
        .map_err(|e| PortCheckError::ResolutionFailed {
            host: host.to_string(),
            reason: e.to_string(),
        })?
        .collect();

    if addrs.is_empty() {
        return Err(PortCheckError::ResolutionFailed {
            host: host.to_string(),
            reason: "No addresses found".to_string(),
        });
    }

    // Try each resolved address with tokio timeout
    let mut last_error = String::new();
    for addr in addrs {
        match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(addr)).await {
            Ok(Ok(_stream)) => {
                return Ok(PortCheckResult::Open);
            }
            Ok(Err(e)) => {
                last_error = e.to_string();
            }
            Err(_) => {
                last_error = "Connection timed out".to_string();
            }
        }
    }

    Err(PortCheckError::Unreachable {
        host: host.to_string(),
        port,
        reason: last_error,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_port_invalid_host() {
        let result = check_port("invalid.host.that.does.not.exist.local", 22, 1);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PortCheckError::ResolutionFailed { .. }
        ));
    }

    #[test]
    fn test_check_port_localhost_closed() {
        // Port 59999 is unlikely to be open
        let result = check_port("127.0.0.1", 59999, 1);
        assert!(result.is_err());
        assert!(matches!(
            result.unwrap_err(),
            PortCheckError::Unreachable { .. }
        ));
    }
}
