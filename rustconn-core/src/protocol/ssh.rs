//! SSH protocol handler

use super::{Protocol, ProtocolCapabilities, ProtocolResult};
use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig, SshAuthMethod, SshConfig};

/// SSH protocol handler
///
/// Implements the Protocol trait for SSH connections.
/// SSH sessions are spawned via VTE4 terminal in the GUI layer.
#[derive(Debug)]
pub struct SshProtocol;

impl SshProtocol {
    /// Creates a new SSH protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Extracts SSH config from a connection, returning an error if not SSH
    fn get_ssh_config(connection: &Connection) -> ProtocolResult<&SshConfig> {
        match &connection.protocol_config {
            ProtocolConfig::Ssh(config) => Ok(config),
            _ => Err(ProtocolError::InvalidConfig(
                "Connection is not an SSH connection".to_string(),
            )),
        }
    }
}

impl Default for SshProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for SshProtocol {
    fn protocol_id(&self) -> &'static str {
        "ssh"
    }

    fn display_name(&self) -> &'static str {
        "SSH"
    }

    fn default_port(&self) -> u16 {
        22
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        let ssh_config = Self::get_ssh_config(connection)?;

        // Validate host is not empty
        if connection.host.is_empty() {
            return Err(ProtocolError::InvalidConfig(
                "Host cannot be empty".to_string(),
            ));
        }

        // Validate port is in valid range
        if connection.port == 0 {
            return Err(ProtocolError::InvalidConfig("Port cannot be 0".to_string()));
        }

        // Validate key path exists if using public key or security key auth
        if matches!(
            ssh_config.auth_method,
            SshAuthMethod::PublicKey | SshAuthMethod::SecurityKey
        ) && let Some(key_path) = &ssh_config.key_path
            && !key_path.as_os_str().is_empty()
            && !key_path.exists()
        {
            return Err(ProtocolError::InvalidConfig(format!(
                "SSH key file not found: {}",
                key_path.display()
            )));
        }

        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities {
            port_forwarding: true,
            wayland_forwarding: true,
            x11_forwarding: true,
            ..ProtocolCapabilities::terminal()
        }
    }

    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
        let ssh_config = Self::get_ssh_config(connection).ok()?;

        let mut cmd = vec!["ssh".to_string()];

        // Non-default port
        if connection.port != 22 {
            cmd.push("-p".to_string());
            cmd.push(connection.port.to_string());
        }

        // Delegate SSH-specific args to SshConfig::build_command_args()
        cmd.extend(ssh_config.build_command_args());

        // user@host or just host
        let destination = if let Some(ref user) = connection.username {
            format!("{user}@{}", connection.host)
        } else {
            connection.host.clone()
        };
        cmd.push(destination);

        // Append startup command after destination — SSH executes it on the
        // remote host immediately after login.
        // NOTE: In CLI mode this replaces the interactive shell (intended for scripting).
        // The GUI version (terminal/mod.rs) wraps with `; exec $SHELL -l` to stay interactive.
        if let Some(ref startup_cmd) = ssh_config.startup_command {
            cmd.push(startup_cmd.clone());
        }

        Some(cmd)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::models::ProtocolConfig;

    fn create_ssh_connection(config: SshConfig) -> Connection {
        Connection::new(
            "Test SSH".to_string(),
            "example.com".to_string(),
            22,
            ProtocolConfig::Ssh(config),
        )
    }

    #[test]
    fn test_ssh_protocol_metadata() {
        let protocol = SshProtocol::new();
        assert_eq!(protocol.protocol_id(), "ssh");
        assert_eq!(protocol.display_name(), "SSH");
        assert_eq!(protocol.default_port(), 22);
    }

    #[test]
    fn test_validate_valid_connection() {
        let protocol = SshProtocol::new();
        let connection = create_ssh_connection(SshConfig::default());
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_empty_host() {
        let protocol = SshProtocol::new();
        let mut connection = create_ssh_connection(SshConfig::default());
        connection.host = String::new();
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_zero_port() {
        let protocol = SshProtocol::new();
        let mut connection = create_ssh_connection(SshConfig::default());
        connection.port = 0;
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_with_proxy_jump() {
        let protocol = SshProtocol::new();
        let config = SshConfig {
            proxy_jump: Some("bastion.example.com".to_string()),
            ..Default::default()
        };
        let connection = create_ssh_connection(config);
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_with_control_master() {
        let protocol = SshProtocol::new();
        let config = SshConfig {
            use_control_master: true,
            ..Default::default()
        };
        let connection = create_ssh_connection(config);
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_missing_key_file() {
        let protocol = SshProtocol::new();
        let config = SshConfig {
            auth_method: SshAuthMethod::PublicKey,
            key_path: Some(PathBuf::from("/nonexistent/key")),
            ..Default::default()
        };
        let connection = create_ssh_connection(config);
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_build_command_with_startup_command() {
        let protocol = SshProtocol::new();
        let config = SshConfig {
            startup_command: Some("htop".to_string()),
            ..Default::default()
        };
        let connection = create_ssh_connection(config);
        let cmd = protocol.build_command(&connection).unwrap();
        // startup_command should be the last argument (after user@host)
        assert_eq!(cmd.last().unwrap(), "htop");
        // destination should be second-to-last
        assert_eq!(cmd[cmd.len() - 2], "example.com");
    }

    #[test]
    fn test_build_command_with_proxy_command() {
        let protocol = SshProtocol::new();
        let config = SshConfig {
            proxy_command: Some(
                "ncat --proxy 127.0.0.1:9050 --proxy-type socks5 %h %p".to_string(),
            ),
            ..Default::default()
        };
        let connection = create_ssh_connection(config);
        let cmd = protocol.build_command(&connection).unwrap();
        assert!(cmd.contains(&"-o".to_string()));
        assert!(cmd.contains(
            &"ProxyCommand=ncat --proxy 127.0.0.1:9050 --proxy-type socks5 %h %p".to_string()
        ));
    }

    #[test]
    fn test_build_command_proxy_command_overrides_proxy_jump() {
        let protocol = SshProtocol::new();
        let config = SshConfig {
            proxy_jump: Some("bastion.example.com".to_string()),
            proxy_command: Some("ncat --proxy 127.0.0.1:9050 %h %p".to_string()),
            ..Default::default()
        };
        let connection = create_ssh_connection(config);
        let cmd = protocol.build_command(&connection).unwrap();
        // ProxyCommand should be present
        assert!(cmd.contains(&"ProxyCommand=ncat --proxy 127.0.0.1:9050 %h %p".to_string()));
        // -J should NOT be present (ProxyCommand takes precedence)
        assert!(!cmd.contains(&"-J".to_string()));
    }
}
