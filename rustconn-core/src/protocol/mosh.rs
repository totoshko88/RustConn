//! MOSH protocol handler

use super::{Protocol, ProtocolCapabilities, ProtocolResult};
use crate::error::ProtocolError;
use crate::models::{Connection, MoshPredictMode, ProtocolConfig};

/// MOSH protocol handler
///
/// Implements the Protocol trait for MOSH (mobile shell) connections.
/// MOSH sessions are spawned via VTE terminal using the external `mosh` client.
#[derive(Debug)]
pub struct MoshProtocol;

impl MoshProtocol {
    /// Creates a new MOSH protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for MoshProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for MoshProtocol {
    fn protocol_id(&self) -> &'static str {
        "mosh"
    }

    fn display_name(&self) -> &'static str {
        "MOSH"
    }

    fn default_port(&self) -> u16 {
        22
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        if connection.host.is_empty() {
            return Err(ProtocolError::InvalidConfig(
                "Host cannot be empty".to_string(),
            ));
        }
        if connection.port == 0 {
            return Err(ProtocolError::InvalidConfig(
                "Port cannot be zero".to_string(),
            ));
        }
        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities::terminal()
    }

    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
        let mut cmd = vec!["mosh".to_string()];

        if let ProtocolConfig::Mosh(ref config) = connection.protocol_config {
            // --ssh "ssh -p PORT"
            if let Some(ssh_port) = config.ssh_port {
                cmd.push("--ssh".to_string());
                cmd.push(format!("ssh -p {ssh_port}"));
            }

            // --predict=MODE
            match config.predict_mode {
                MoshPredictMode::Adaptive => {} // default, no flag needed
                MoshPredictMode::Always => cmd.push("--predict=always".to_string()),
                MoshPredictMode::Never => cmd.push("--predict=never".to_string()),
            }

            // --server=PATH
            if let Some(ref server) = config.server_binary {
                cmd.push(format!("--server={server}"));
            }

            // -p PORT_RANGE
            if let Some(ref port_range) = config.port_range {
                cmd.push("-p".to_string());
                cmd.push(port_range.clone());
            }

            // Custom args (sanitized)
            for arg in &config.custom_args {
                if arg.contains('\0') || arg.contains('\n') {
                    tracing::warn!(arg = %arg, "Skipping MOSH custom arg with unsafe characters");
                    continue;
                }
                cmd.push(arg.clone());
            }
        }

        // [user@]host
        if let Some(ref username) = connection.username {
            cmd.push(format!("{username}@{}", connection.host));
        } else {
            cmd.push(connection.host.clone());
        }

        Some(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{MoshConfig, ProtocolConfig};

    fn create_mosh_connection(config: MoshConfig) -> Connection {
        Connection::new(
            "Test Mosh".to_string(),
            "example.com".to_string(),
            22,
            ProtocolConfig::Mosh(config),
        )
    }

    #[test]
    fn test_mosh_protocol_metadata() {
        let protocol = MoshProtocol::new();
        assert_eq!(protocol.protocol_id(), "mosh");
        assert_eq!(protocol.display_name(), "MOSH");
        assert_eq!(protocol.default_port(), 22);
    }

    #[test]
    fn test_validate_valid_connection() {
        let protocol = MoshProtocol::new();
        let connection = create_mosh_connection(MoshConfig::default());
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_empty_host() {
        let protocol = MoshProtocol::new();
        let mut connection = create_mosh_connection(MoshConfig::default());
        connection.host = String::new();
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_build_command_default() {
        let protocol = MoshProtocol::new();
        let connection = create_mosh_connection(MoshConfig::default());
        let cmd = protocol.build_command(&connection);
        assert!(cmd.is_some());
        let cmd = cmd.unwrap();
        assert_eq!(cmd[0], "mosh");
        assert_eq!(cmd[1], "example.com");
    }

    #[test]
    fn test_build_command_with_username() {
        let protocol = MoshProtocol::new();
        let mut connection = create_mosh_connection(MoshConfig::default());
        connection.username = Some("admin".to_string());
        let cmd = protocol.build_command(&connection).unwrap();
        assert_eq!(cmd[0], "mosh");
        assert_eq!(cmd[1], "admin@example.com");
    }

    #[test]
    fn test_build_command_with_options() {
        let protocol = MoshProtocol::new();
        let config = MoshConfig {
            ssh_port: Some(2222),
            port_range: Some("60000:60010".to_string()),
            server_binary: Some("/usr/local/bin/mosh-server".to_string()),
            predict_mode: MoshPredictMode::Always,
            custom_args: vec![],
        };
        let connection = create_mosh_connection(config);
        let cmd = protocol.build_command(&connection).unwrap();
        assert_eq!(cmd[0], "mosh");
        assert!(cmd.contains(&"--ssh".to_string()));
        assert!(cmd.contains(&"ssh -p 2222".to_string()));
        assert!(cmd.contains(&"--predict=always".to_string()));
        assert!(cmd.contains(&"--server=/usr/local/bin/mosh-server".to_string()));
        assert!(cmd.contains(&"-p".to_string()));
        assert!(cmd.contains(&"60000:60010".to_string()));
    }
}
