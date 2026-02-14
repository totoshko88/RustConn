//! Telnet protocol handler

use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig};

use super::{Protocol, ProtocolCapabilities, ProtocolResult};

/// Telnet protocol handler
///
/// Implements the Protocol trait for Telnet connections.
/// Telnet sessions are spawned via VTE terminal using an external `telnet` client.
pub struct TelnetProtocol;

impl TelnetProtocol {
    /// Creates a new Telnet protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for TelnetProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for TelnetProtocol {
    fn protocol_id(&self) -> &'static str {
        "telnet"
    }

    fn display_name(&self) -> &'static str {
        "Telnet"
    }

    fn default_port(&self) -> u16 {
        23
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        if connection.host.is_empty() {
            return Err(ProtocolError::InvalidConfig(
                "Host cannot be empty".to_string(),
            ));
        }

        if connection.port == 0 {
            return Err(ProtocolError::InvalidConfig("Port cannot be 0".to_string()));
        }

        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities::terminal()
    }

    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
        let mut cmd = vec!["telnet".to_string()];

        // Add custom args from TelnetConfig
        if let ProtocolConfig::Telnet(ref config) = connection.protocol_config {
            cmd.extend(config.custom_args.clone());
        }

        cmd.push(connection.host.clone());
        cmd.push(connection.port.to_string());

        Some(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProtocolConfig, TelnetConfig};

    fn create_telnet_connection(config: TelnetConfig) -> Connection {
        Connection::new(
            "Test Telnet".to_string(),
            "example.com".to_string(),
            23,
            ProtocolConfig::Telnet(config),
        )
    }

    #[test]
    fn test_telnet_protocol_metadata() {
        let protocol = TelnetProtocol::new();
        assert_eq!(protocol.protocol_id(), "telnet");
        assert_eq!(protocol.display_name(), "Telnet");
        assert_eq!(protocol.default_port(), 23);
    }

    #[test]
    fn test_validate_valid_connection() {
        let protocol = TelnetProtocol::new();
        let connection = create_telnet_connection(TelnetConfig::default());
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_empty_host() {
        let protocol = TelnetProtocol::new();
        let mut connection = create_telnet_connection(TelnetConfig::default());
        connection.host = String::new();
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_zero_port() {
        let protocol = TelnetProtocol::new();
        let mut connection = create_telnet_connection(TelnetConfig::default());
        connection.port = 0;
        assert!(protocol.validate_connection(&connection).is_err());
    }
}
