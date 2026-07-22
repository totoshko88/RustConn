//! Multipath TCP (MPTCP) support utilities
//!
//! Provides runtime detection of kernel MPTCP availability and helper
//! functions to create MPTCP-enabled TCP sockets that gracefully fall back
//! to regular TCP when MPTCP is unavailable.
//!
//! ## Background
//!
//! MPTCP allows using multiple network paths simultaneously for a single
//! TCP connection. Benefits include seamless mobility (switching between
//! Wi-Fi and Ethernet without connection drops) and bandwidth aggregation.
//!
//! On Linux, MPTCP requires kernel 5.6+ with `CONFIG_MPTCP=y`. Most modern
//! distributions ship with MPTCP enabled. The protocol is negotiated during
//! the TCP handshake — if the server does not support MPTCP, the connection
//! transparently falls back to regular TCP.

use std::net::SocketAddr;

use socket2::{Domain, Protocol, Socket, Type};
use thiserror::Error;

/// IPPROTO_MPTCP value (262) as defined in the Linux kernel.
// 262 = IPPROTO_MPTCP per include/uapi/linux/in.h
const IPPROTO_MPTCP: i32 = 262;

/// Errors from MPTCP operations.
#[derive(Debug, Error)]
pub enum MptcpError {
    /// MPTCP is not available on this system.
    #[error("MPTCP is not available: {reason}")]
    NotAvailable {
        /// Explanation of why MPTCP is not available.
        reason: String,
    },
    /// Socket creation failed.
    #[error("Failed to create MPTCP socket: {0}")]
    SocketCreation(#[source] std::io::Error),
    /// Connection failed.
    #[error("MPTCP connection to {addr} failed: {source}")]
    ConnectionFailed {
        /// The target address.
        addr: SocketAddr,
        /// The underlying I/O error.
        source: std::io::Error,
    },
}

/// Result type for MPTCP operations.
pub type MptcpResult<T> = Result<T, MptcpError>;

/// Checks whether the kernel has MPTCP support enabled.
///
/// Reads `/proc/sys/net/mptcp/enabled` — returns `true` if the file
/// exists and contains `1`. Returns `false` on non-Linux systems or
/// when MPTCP is disabled/unavailable.
#[must_use]
pub fn is_mptcp_available() -> bool {
    std::fs::read_to_string("/proc/sys/net/mptcp/enabled")
        .map(|s| s.trim() == "1")
        .unwrap_or(false)
}

/// Creates a connected TCP socket using MPTCP protocol.
///
/// Falls back to a regular TCP socket if MPTCP socket creation fails
/// (e.g., kernel does not support MPTCP). The fallback is transparent
/// to the caller.
///
/// # Errors
///
/// Returns `MptcpError::ConnectionFailed` if the connection itself fails
/// (regardless of whether MPTCP or TCP was used).
pub fn connect_mptcp(addr: SocketAddr) -> MptcpResult<std::net::TcpStream> {
    let domain = match addr {
        SocketAddr::V4(_) => Domain::IPV4,
        SocketAddr::V6(_) => Domain::IPV6,
    };

    // Try MPTCP first; fall back to regular TCP if the protocol is unsupported.
    let socket = match Socket::new(domain, Type::STREAM, Some(Protocol::from(IPPROTO_MPTCP))) {
        Ok(s) => {
            tracing::debug!(%addr, "Created MPTCP socket");
            s
        }
        Err(e) => {
            tracing::debug!(%addr, error = %e, "MPTCP socket creation failed, falling back to regular TCP");
            Socket::new(domain, Type::STREAM, Some(Protocol::TCP))
                .map_err(MptcpError::SocketCreation)?
        }
    };

    socket
        .connect(&addr.into())
        .map_err(|e| MptcpError::ConnectionFailed { addr, source: e })?;

    Ok(std::net::TcpStream::from(socket))
}

/// Creates a non-blocking MPTCP socket suitable for async use with Tokio.
///
/// Returns the raw `socket2::Socket` in non-blocking mode so the caller
/// can convert it to `tokio::net::TcpStream` via `TcpStream::from_std`.
///
/// Falls back to regular TCP if MPTCP socket creation fails.
///
/// # Errors
///
/// Returns `MptcpError::SocketCreation` if neither MPTCP nor TCP socket
/// creation succeeds.
pub fn create_mptcp_socket(addr: SocketAddr) -> MptcpResult<socket2::Socket> {
    let domain = match addr {
        SocketAddr::V4(_) => Domain::IPV4,
        SocketAddr::V6(_) => Domain::IPV6,
    };

    let socket = match Socket::new(domain, Type::STREAM, Some(Protocol::from(IPPROTO_MPTCP))) {
        Ok(s) => {
            tracing::debug!(%addr, "Created async MPTCP socket");
            s
        }
        Err(e) => {
            tracing::debug!(%addr, error = %e, "Async MPTCP socket creation failed, falling back to regular TCP");
            Socket::new(domain, Type::STREAM, Some(Protocol::TCP))
                .map_err(MptcpError::SocketCreation)?
        }
    };

    socket
        .set_nonblocking(true)
        .map_err(MptcpError::SocketCreation)?;

    Ok(socket)
}

/// Creates and connects an async MPTCP `TcpStream` for use with Tokio.
///
/// This is the primary entry point for embedded clients (RDP, VNC) that
/// need an async MPTCP connection. Falls back to regular TCP if MPTCP
/// is unavailable.
///
/// # Errors
///
/// Returns `MptcpError::SocketCreation` if socket creation fails, or
/// `MptcpError::ConnectionFailed` if connecting to the address fails.
pub async fn connect_mptcp_async(addr: SocketAddr) -> MptcpResult<tokio::net::TcpStream> {
    let socket = create_mptcp_socket(addr)?;

    // Initiate non-blocking connect
    match socket.connect(&addr.into()) {
        Ok(()) => {}
        // EINPROGRESS (115 on Linux) means the non-blocking connect is in progress
        Err(e) if e.raw_os_error() == Some(nix::libc::EINPROGRESS) => {}
        Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {}
        Err(e) => {
            return Err(MptcpError::ConnectionFailed { addr, source: e });
        }
    }

    // Convert to tokio TcpStream and wait for connection to complete
    let std_stream = std::net::TcpStream::from(socket);
    let tokio_stream = tokio::net::TcpStream::from_std(std_stream)
        .map_err(|e| MptcpError::ConnectionFailed { addr, source: e })?;

    // Wait for the socket to become writable (connection established)
    tokio_stream
        .writable()
        .await
        .map_err(|e| MptcpError::ConnectionFailed { addr, source: e })?;

    // Check for connection errors
    if let Some(err) = tokio_stream
        .take_error()
        .map_err(|e| MptcpError::ConnectionFailed { addr, source: e })?
    {
        return Err(MptcpError::ConnectionFailed { addr, source: err });
    }

    Ok(tokio_stream)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mptcp_availability_does_not_panic() {
        // Just verify the function runs without panicking.
        // Result depends on kernel configuration.
        let _ = is_mptcp_available();
    }

    #[test]
    fn create_mptcp_socket_ipv4() {
        let addr: SocketAddr = "127.0.0.1:0".parse().expect("valid addr");
        // Should succeed — either MPTCP or TCP fallback
        let result = create_mptcp_socket(addr);
        assert!(result.is_ok());
    }

    #[test]
    fn create_mptcp_socket_ipv6() {
        let addr: SocketAddr = "[::1]:0".parse().expect("valid addr");
        let result = create_mptcp_socket(addr);
        assert!(result.is_ok());
    }

    #[test]
    fn connect_mptcp_unreachable() {
        // Port 1 on localhost should be unreachable
        let addr: SocketAddr = "127.0.0.1:1".parse().expect("valid addr");
        let result = connect_mptcp(addr);
        assert!(result.is_err());
    }
}
