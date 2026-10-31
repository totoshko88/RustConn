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
    /// The hostname could not be resolved, so no probe was performed.
    ///
    /// Treated as "proceed anyway" by every caller (issue #241): the probe is a
    /// latency optimisation, not an authority on reachability. A name our
    /// resolver cannot see may still be resolvable by the client that actually
    /// connects — the classic case is an mDNS `.local` name inside a Flatpak
    /// sandbox — and a genuinely wrong name produces the client's own, more
    /// accurate error a moment later.
    Unresolved,
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
/// * `Ok(PortCheckResult::Unresolved)` if the hostname cannot be resolved locally
///
/// # Errors
/// * `PortCheckError::Unreachable` if the port is not reachable or connection timed out
pub fn check_port(
    host: &str,
    port: u16,
    timeout_secs: u32,
) -> Result<PortCheckResult, PortCheckError> {
    let timeout = Duration::from_secs(u64::from(timeout_secs));
    let addr_str = format!("{host}:{port}");

    // Resolve hostname to socket addresses. A resolution failure never blocks
    // the connection (issue #241) — see `PortCheckResult::Unresolved`.
    let addrs: Vec<SocketAddr> = match addr_str.to_socket_addrs() {
        Ok(addrs) => addrs.collect(),
        Err(e) => {
            tracing::info!(
                %host,
                port,
                error = %e,
                "Pre-connect probe skipped: hostname not resolvable locally"
            );
            return Ok(PortCheckResult::Unresolved);
        }
    };

    if addrs.is_empty() {
        tracing::info!(
            %host,
            port,
            "Pre-connect probe skipped: hostname resolved to no addresses"
        );
        return Ok(PortCheckResult::Unresolved);
    }

    // Try each resolved address
    // ponytail: one address at a time, so an address that never answers (a
    // VPN's unrouted AAAA record) delays the probe by a full `timeout`; race
    // them like `connection::connect_tcp` if that delay starts to matter.
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
/// * `Ok(PortCheckResult::Unresolved)` if the hostname cannot be resolved locally
///
/// # Errors
/// * `PortCheckError::Unreachable` if the port is not reachable or connection timed out
pub async fn check_port_async(
    host: &str,
    port: u16,
    timeout_secs: u32,
) -> Result<PortCheckResult, PortCheckError> {
    let timeout = Duration::from_secs(u64::from(timeout_secs));
    let addr_str = format!("{host}:{port}");

    // Resolve hostname asynchronously. As in `check_port`, an unresolvable name
    // skips the probe instead of failing the connection (issue #241).
    let addrs: Vec<SocketAddr> = match tokio::net::lookup_host(&addr_str).await {
        Ok(addrs) => addrs.collect(),
        Err(e) => {
            tracing::info!(
                %host,
                port,
                error = %e,
                "Pre-connect probe skipped: hostname not resolvable locally"
            );
            return Ok(PortCheckResult::Unresolved);
        }
    };

    if addrs.is_empty() {
        tracing::info!(
            %host,
            port,
            "Pre-connect probe skipped: hostname resolved to no addresses"
        );
        return Ok(PortCheckResult::Unresolved);
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
    fn test_check_port_invalid_host_is_not_an_error() {
        // Issue #241: an unresolvable name must not fail the probe, otherwise a
        // name only the connecting client can resolve (mDNS `.local` inside a
        // Flatpak sandbox) can never be connected to.
        let result = check_port("invalid.host.that.does.not.exist.example", 22, 1);
        assert_eq!(result.ok(), Some(PortCheckResult::Unresolved));
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
