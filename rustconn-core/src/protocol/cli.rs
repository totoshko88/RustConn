//! CLI connection output feedback utilities
//!
//! This module provides formatting functions for CLI connection feedback messages.
//! These messages are displayed to users when connecting via CLI or `ZeroTrust` protocols.

/// Format a connection start message with emoji prefix
///
/// Returns a formatted string indicating the connection type and target host.
/// Uses the 🔗 emoji prefix for visual identification.
///
/// # Arguments
/// * `protocol` - The protocol name (e.g., "SSH", "AWS SSM", "GCP IAP")
/// * `host` - The target host or identifier
///
/// # Returns
/// A formatted connection message string
///
/// # Example
/// ```
/// use rustconn_core::protocol::format_connection_message;
///
/// let msg = format_connection_message("SSH", "server.example.com");
/// assert!(msg.contains("🔗"));
/// assert!(msg.contains("SSH"));
/// assert!(msg.contains("server.example.com"));
/// ```
#[must_use]
pub fn format_connection_message(protocol: &str, host: &str) -> String {
    format!("🔗 Connecting via {protocol} to {host}...")
}

/// Format a command execution message with emoji prefix
///
/// Returns a formatted string showing the exact command being executed.
/// Uses the ⚡ emoji prefix for visual identification.
///
/// # Arguments
/// * `command` - The full command string being executed
///
/// # Returns
/// A formatted command echo message string
///
/// # Example
/// ```
/// use rustconn_core::protocol::format_command_message;
///
/// let msg = format_command_message("ssh user@host");
/// assert!(msg.contains("⚡"));
/// assert!(msg.contains("ssh user@host"));
/// ```
#[must_use]
pub fn format_command_message(command: &str) -> String {
    format!("⚡ Executing: {command}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_connection_message_contains_emoji() {
        let msg = format_connection_message("SSH", "example.com");
        assert!(msg.contains("🔗"));
    }

    #[test]
    fn test_format_connection_message_contains_protocol() {
        let msg = format_connection_message("SSH", "example.com");
        assert!(msg.contains("SSH"));
    }

    #[test]
    fn test_format_connection_message_contains_host() {
        let msg = format_connection_message("SSH", "example.com");
        assert!(msg.contains("example.com"));
    }

    #[test]
    fn test_format_connection_message_format() {
        let msg = format_connection_message("AWS SSM", "i-0123456789");
        assert_eq!(msg, "🔗 Connecting via AWS SSM to i-0123456789...");
    }

    #[test]
    fn test_format_command_message_contains_emoji() {
        let msg = format_command_message("ssh user@host");
        assert!(msg.contains("⚡"));
    }

    #[test]
    fn test_format_command_message_contains_command() {
        let msg = format_command_message("ssh user@host");
        assert!(msg.contains("ssh user@host"));
    }

    #[test]
    fn test_format_command_message_format() {
        let msg = format_command_message("aws ssm start-session --target i-123");
        assert_eq!(msg, "⚡ Executing: aws ssm start-session --target i-123");
    }

    #[test]
    fn test_format_command_message_preserves_exact_command() {
        let cmd = "ssh -o StrictHostKeyChecking=no user@host";
        let msg = format_command_message(cmd);
        assert!(msg.contains(cmd));
    }
}
