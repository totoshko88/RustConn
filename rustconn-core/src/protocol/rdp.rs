//! RDP protocol handler

use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig, RdpConfig};

use super::{Protocol, ProtocolCapabilities, ProtocolResult};

/// RDP protocol handler
///
/// Implements the Protocol trait for RDP connections.
/// Native embedding via gtk-frdp will be implemented in Phase 6.
pub struct RdpProtocol;

impl RdpProtocol {
    /// Creates a new RDP protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Extracts RDP config from a connection, returning an error if not RDP
    fn get_rdp_config(connection: &Connection) -> ProtocolResult<&RdpConfig> {
        match &connection.protocol_config {
            ProtocolConfig::Rdp(config) => Ok(config),
            _ => Err(ProtocolError::InvalidConfig(
                "Connection is not an RDP connection".to_string(),
            )),
        }
    }
}

impl Default for RdpProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for RdpProtocol {
    fn protocol_id(&self) -> &'static str {
        "rdp"
    }

    fn display_name(&self) -> &'static str {
        "RDP"
    }

    fn default_port(&self) -> u16 {
        3389
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        let rdp_config = Self::get_rdp_config(connection)?;

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

        // Validate color depth if specified
        if let Some(depth) = rdp_config.color_depth {
            if !matches!(depth, 8 | 15 | 16 | 24 | 32) {
                return Err(ProtocolError::InvalidConfig(format!(
                    "Invalid color depth: {depth}. Must be 8, 15, 16, 24, or 32"
                )));
            }
        }

        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities::graphical(true, true, true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProtocolConfig, Resolution};

    fn create_rdp_connection(config: RdpConfig) -> Connection {
        Connection::new(
            "Test RDP".to_string(),
            "windows.example.com".to_string(),
            3389,
            ProtocolConfig::Rdp(config),
        )
    }

    #[test]
    fn test_rdp_protocol_metadata() {
        let protocol = RdpProtocol::new();
        assert_eq!(protocol.protocol_id(), "rdp");
        assert_eq!(protocol.display_name(), "RDP");
        assert_eq!(protocol.default_port(), 3389);
    }

    #[test]
    fn test_validate_valid_connection() {
        let protocol = RdpProtocol::new();
        let connection = create_rdp_connection(RdpConfig::default());
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_empty_host() {
        let protocol = RdpProtocol::new();
        let mut connection = create_rdp_connection(RdpConfig::default());
        connection.host = String::new();
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_zero_port() {
        let protocol = RdpProtocol::new();
        let mut connection = create_rdp_connection(RdpConfig::default());
        connection.port = 0;
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_valid_color_depth() {
        let protocol = RdpProtocol::new();
        for depth in [8, 15, 16, 24, 32] {
            let config = RdpConfig {
                color_depth: Some(depth),
                ..Default::default()
            };
            let connection = create_rdp_connection(config);
            assert!(protocol.validate_connection(&connection).is_ok());
        }
    }

    #[test]
    fn test_validate_invalid_color_depth() {
        let protocol = RdpProtocol::new();
        let config = RdpConfig {
            color_depth: Some(12), // Invalid
            ..Default::default()
        };
        let connection = create_rdp_connection(config);
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_with_resolution() {
        let protocol = RdpProtocol::new();
        let config = RdpConfig {
            resolution: Some(Resolution::new(1920, 1080)),
            ..Default::default()
        };
        let connection = create_rdp_connection(config);
        assert!(protocol.validate_connection(&connection).is_ok());
    }
}
