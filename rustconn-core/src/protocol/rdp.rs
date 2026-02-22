//! RDP protocol handler

use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig, RdpConfig};

use super::{Protocol, ProtocolCapabilities, ProtocolResult};

/// RDP protocol handler
///
/// Implements the Protocol trait for RDP connections.
/// Native RDP embedding is available via IronRDP (`rdp-embedded` feature flag).
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
        if let Some(depth) = rdp_config.color_depth
            && !matches!(depth, 8 | 15 | 16 | 24 | 32)
        {
            return Err(ProtocolError::InvalidConfig(format!(
                "Invalid color depth: {depth}. Must be 8, 15, 16, 24, or 32"
            )));
        }

        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities::graphical(true, true, true)
    }

    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
        // Default binary for CLI compatibility; GUI overrides via detect_best_freerdp()
        self.build_command_with_binary("xfreerdp", connection)
    }
}

impl RdpProtocol {
    /// Builds the FreeRDP argument list without a binary name.
    ///
    /// Callers that perform runtime detection (GUI, CLI) should use this
    /// and prepend the detected binary themselves.
    #[must_use]
    pub fn build_args(connection: &Connection) -> Option<Vec<String>> {
        let mut args = vec![format!("/v:{}:{}", connection.host, connection.port)];

        if let Some(ref username) = connection.username {
            args.push(format!("/u:{username}"));
        }
        if let Some(ref domain) = connection.domain {
            args.push(format!("/d:{domain}"));
        }

        if let ProtocolConfig::Rdp(ref rdp_config) = connection.protocol_config {
            if let Some(ref resolution) = rdp_config.resolution {
                args.push(format!("/w:{}", resolution.width));
                args.push(format!("/h:{}", resolution.height));
            }
            if let Some(depth) = rdp_config.color_depth {
                args.push(format!("/bpp:{depth}"));
            }
            if rdp_config.audio_redirect {
                args.push("/sound".to_string());
            }
            if let Some(ref gateway) = rdp_config.gateway {
                args.push(format!("/g:{}:{}", gateway.hostname, gateway.port));
                if let Some(ref gw_user) = gateway.username {
                    args.push(format!("/gu:{gw_user}"));
                }
            }
            for folder in &rdp_config.shared_folders {
                if folder.share_name.contains(',') || folder.share_name.contains('/') {
                    tracing::warn!(share_name = %folder.share_name, "Skipping shared folder with invalid share name");
                    continue;
                }
                args.push(format!(
                    "/drive:{},{}",
                    folder.share_name,
                    folder.local_path.display()
                ));
            }
            let dangerous_prefixes = ["/p:", "/password:", "/shell:", "/proxy:"];
            for arg in &rdp_config.custom_args {
                let lower = arg.to_lowercase();
                if dangerous_prefixes.iter().any(|p| lower.starts_with(p)) {
                    tracing::warn!(arg = %arg, "Blocked dangerous RDP custom arg");
                    continue;
                }
                args.push(arg.clone());
            }
        }

        Some(args)
    }

    /// Builds a full command with the given binary name prepended.
    #[must_use]
    pub fn build_command_with_binary(
        &self,
        binary: &str,
        connection: &Connection,
    ) -> Option<Vec<String>> {
        Self::build_args(connection).map(|args| {
            let mut cmd = vec![binary.to_string()];
            cmd.extend(args);
            cmd
        })
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
