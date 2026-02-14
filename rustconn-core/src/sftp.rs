//! SFTP URI and command builder
//!
//! Provides utilities for building SFTP URIs and CLI commands
//! for SSH connections with SFTP enabled.

use crate::models::Connection;

/// Builds an SFTP URI for the given connection.
///
/// Format: `sftp://[user@]host[:port]`
///
/// Used by GUI (`gtk4::UriLauncher`) and CLI (`xdg-open`) to open
/// the host file manager's SFTP browser.
#[must_use]
pub fn build_sftp_uri(username: Option<&str>, host: &str, port: u16) -> String {
    let user_part = username.map_or_else(String::new, |u| format!("{u}@"));
    if port == 22 {
        format!("sftp://{user_part}{host}")
    } else {
        format!("sftp://{user_part}{host}:{port}")
    }
}

/// Builds an SFTP URI from a `Connection`.
///
/// Returns `None` if the connection is not SSH or SFTP is not enabled.
#[must_use]
pub fn build_sftp_uri_from_connection(connection: &Connection) -> Option<String> {
    if let crate::models::ProtocolConfig::Ssh(ref config) = connection.protocol_config {
        if !config.sftp_enabled {
            return None;
        }
    } else {
        return None;
    }

    Some(build_sftp_uri(
        connection.username.as_deref(),
        &connection.host,
        connection.port,
    ))
}

/// Builds an `sftp` CLI command for the given connection.
///
/// Returns `None` if the connection is not SSH or SFTP is not enabled.
///
/// The returned `Vec` has the program name as the first element,
/// followed by arguments: `["sftp", "-P", "port", "user@host"]`.
#[must_use]
pub fn build_sftp_command(connection: &Connection) -> Option<Vec<String>> {
    if let crate::models::ProtocolConfig::Ssh(ref config) = connection.protocol_config {
        if !config.sftp_enabled {
            return None;
        }
    } else {
        return None;
    }

    let mut cmd = vec!["sftp".to_string()];

    if connection.port != 22 {
        cmd.push("-P".to_string());
        cmd.push(connection.port.to_string());
    }

    let target = if let Some(ref user) = connection.username {
        format!("{user}@{}", connection.host)
    } else {
        connection.host.clone()
    };
    cmd.push(target);

    Some(cmd)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_sftp_uri_with_user_default_port() {
        let uri = build_sftp_uri(Some("admin"), "server.example.com", 22);
        assert_eq!(uri, "sftp://admin@server.example.com");
    }

    #[test]
    fn test_build_sftp_uri_without_user() {
        let uri = build_sftp_uri(None, "10.0.0.1", 22);
        assert_eq!(uri, "sftp://10.0.0.1");
    }

    #[test]
    fn test_build_sftp_uri_custom_port() {
        let uri = build_sftp_uri(Some("root"), "host.local", 2222);
        assert_eq!(uri, "sftp://root@host.local:2222");
    }

    #[test]
    fn test_build_sftp_command_default_port() {
        let mut conn =
            Connection::new_ssh("Test".to_string(), "server.example.com".to_string(), 22);
        conn.username = Some("admin".to_string());
        if let crate::models::ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.sftp_enabled = true;
        }

        let cmd = build_sftp_command(&conn).unwrap();
        assert_eq!(cmd, vec!["sftp", "admin@server.example.com"]);
    }

    #[test]
    fn test_build_sftp_command_custom_port() {
        let mut conn = Connection::new_ssh("Test".to_string(), "host.local".to_string(), 2222);
        conn.username = Some("root".to_string());
        if let crate::models::ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.sftp_enabled = true;
        }

        let cmd = build_sftp_command(&conn).unwrap();
        assert_eq!(cmd, vec!["sftp", "-P", "2222", "root@host.local"]);
    }

    #[test]
    fn test_build_sftp_command_disabled() {
        let conn = Connection::new_ssh("Test".to_string(), "server.example.com".to_string(), 22);
        // sftp_enabled defaults to false
        assert!(build_sftp_command(&conn).is_none());
    }

    #[test]
    fn test_build_sftp_uri_from_connection_enabled() {
        let mut conn =
            Connection::new_ssh("Test".to_string(), "server.example.com".to_string(), 22);
        conn.username = Some("admin".to_string());
        if let crate::models::ProtocolConfig::Ssh(ref mut cfg) = conn.protocol_config {
            cfg.sftp_enabled = true;
        }

        let uri = build_sftp_uri_from_connection(&conn).unwrap();
        assert_eq!(uri, "sftp://admin@server.example.com");
    }

    #[test]
    fn test_build_sftp_uri_from_non_ssh() {
        let conn = Connection::new_rdp("Test".to_string(), "server.example.com".to_string(), 3389);
        assert!(build_sftp_uri_from_connection(&conn).is_none());
    }
}
