//! VNC protocol handler

use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig, VncConfig};

use super::{Protocol, ProtocolCapabilities, ProtocolResult};

/// VNC protocol handler
///
/// Implements the Protocol trait for VNC connections.
/// Native VNC embedding is available via vnc-rs (`vnc-embedded` feature flag).
pub struct VncProtocol;

impl VncProtocol {
    /// Creates a new VNC protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Extracts VNC config from a connection, returning an error if not VNC
    fn get_vnc_config(connection: &Connection) -> ProtocolResult<&VncConfig> {
        match &connection.protocol_config {
            ProtocolConfig::Vnc(config) => Ok(config),
            _ => Err(ProtocolError::InvalidConfig(
                "Connection is not a VNC connection".to_string(),
            )),
        }
    }
}

impl Default for VncProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for VncProtocol {
    fn protocol_id(&self) -> &'static str {
        "vnc"
    }

    fn display_name(&self) -> &'static str {
        "VNC"
    }

    fn default_port(&self) -> u16 {
        5900
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        let vnc_config = Self::get_vnc_config(connection)?;

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

        // Validate compression level if specified (0-9)
        if let Some(compression) = vnc_config.compression
            && compression > 9
        {
            return Err(ProtocolError::InvalidConfig(format!(
                "Invalid compression level: {compression}. Must be 0-9"
            )));
        }

        // Validate quality level if specified (0-9)
        if let Some(quality) = vnc_config.quality
            && quality > 9
        {
            return Err(ProtocolError::InvalidConfig(format!(
                "Invalid quality level: {quality}. Must be 0-9"
            )));
        }

        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities::graphical(false, false, true)
    }

    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
        let mut args = Vec::new();

        if let ProtocolConfig::Vnc(ref vnc_config) = connection.protocol_config {
            if let Some(ref encoding) = vnc_config.encoding {
                args.push("-encoding".to_string());
                args.push(encoding.clone());
            }
            if let Some(compression) = vnc_config.compression {
                args.push("-compresslevel".to_string());
                args.push(compression.to_string());
            }
            if let Some(quality) = vnc_config.quality {
                args.push("-quality".to_string());
                args.push(quality.to_string());
            }
            for arg in &vnc_config.custom_args {
                if arg.contains('\0') || arg.contains('\n') {
                    tracing::warn!(arg = %arg, "Skipping suspicious VNC custom arg");
                    continue;
                }
                args.push(arg.clone());
            }
        }

        let display = if connection.port >= 5900 {
            connection.port - 5900
        } else {
            connection.port
        };
        args.push(format!("{}:{display}", connection.host));

        let mut cmd = vec!["vncviewer".to_string()];
        cmd.extend(args);
        Some(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ProtocolConfig;

    fn create_vnc_connection(config: VncConfig) -> Connection {
        Connection::new(
            "Test VNC".to_string(),
            "vnc.example.com".to_string(),
            5900,
            ProtocolConfig::Vnc(config),
        )
    }

    #[test]
    fn test_vnc_protocol_metadata() {
        let protocol = VncProtocol::new();
        assert_eq!(protocol.protocol_id(), "vnc");
        assert_eq!(protocol.display_name(), "VNC");
        assert_eq!(protocol.default_port(), 5900);
    }

    #[test]
    fn test_validate_valid_connection() {
        let protocol = VncProtocol::new();
        let connection = create_vnc_connection(VncConfig::default());
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_empty_host() {
        let protocol = VncProtocol::new();
        let mut connection = create_vnc_connection(VncConfig::default());
        connection.host = String::new();
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_zero_port() {
        let protocol = VncProtocol::new();
        let mut connection = create_vnc_connection(VncConfig::default());
        connection.port = 0;
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_valid_compression() {
        let protocol = VncProtocol::new();
        for compression in 0..=9 {
            let config = VncConfig {
                compression: Some(compression),
                ..Default::default()
            };
            let connection = create_vnc_connection(config);
            assert!(protocol.validate_connection(&connection).is_ok());
        }
    }

    #[test]
    fn test_validate_invalid_compression() {
        let protocol = VncProtocol::new();
        let config = VncConfig {
            compression: Some(15), // Invalid: > 9
            ..Default::default()
        };
        let connection = create_vnc_connection(config);
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_valid_quality() {
        let protocol = VncProtocol::new();
        for quality in 0..=9 {
            let config = VncConfig {
                quality: Some(quality),
                ..Default::default()
            };
            let connection = create_vnc_connection(config);
            assert!(protocol.validate_connection(&connection).is_ok());
        }
    }

    #[test]
    fn test_validate_invalid_quality() {
        let protocol = VncProtocol::new();
        let config = VncConfig {
            quality: Some(10), // Invalid: > 9
            ..Default::default()
        };
        let connection = create_vnc_connection(config);
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_with_encoding() {
        let protocol = VncProtocol::new();
        let config = VncConfig {
            encoding: Some("tight".to_string()),
            ..Default::default()
        };
        let connection = create_vnc_connection(config);
        assert!(protocol.validate_connection(&connection).is_ok());
    }
}
