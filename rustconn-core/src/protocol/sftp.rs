//! SFTP protocol handler
//!
//! SFTP connections reuse SSH configuration but open a file manager
//! instead of a terminal session. Validation mirrors SSH.

use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig, SshAuthMethod, SshConfig};

use super::{Protocol, ProtocolCapabilities, ProtocolResult};

/// SFTP protocol handler
///
/// Implements the Protocol trait for SFTP file transfer connections.
/// Uses SSH transport but opens a file manager (via `sftp://` URI)
/// instead of a terminal session.
pub struct SftpProtocol;

impl SftpProtocol {
    /// Creates a new SFTP protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Extracts SSH config from an SFTP connection
    fn get_ssh_config(connection: &Connection) -> ProtocolResult<&SshConfig> {
        match &connection.protocol_config {
            ProtocolConfig::Sftp(config) => Ok(config),
            _ => Err(ProtocolError::InvalidConfig(
                "Connection is not an SFTP connection".to_string(),
            )),
        }
    }
}

impl Default for SftpProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for SftpProtocol {
    fn protocol_id(&self) -> &'static str {
        "sftp"
    }

    fn display_name(&self) -> &'static str {
        "SFTP"
    }

    fn default_port(&self) -> u16 {
        22
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        let ssh_config = Self::get_ssh_config(connection)?;

        if connection.host.is_empty() {
            return Err(ProtocolError::InvalidConfig(
                "Host cannot be empty".to_string(),
            ));
        }

        if connection.port == 0 {
            return Err(ProtocolError::InvalidConfig("Port cannot be 0".to_string()));
        }

        // Validate key path if using public key or security key auth
        if matches!(
            ssh_config.auth_method,
            SshAuthMethod::PublicKey | SshAuthMethod::SecurityKey
        ) {
            if let Some(key_path) = &ssh_config.key_path {
                if !key_path.as_os_str().is_empty() && !key_path.exists() {
                    return Err(ProtocolError::InvalidConfig(format!(
                        "SSH key file not found: {}",
                        key_path.display()
                    )));
                }
            }
        }

        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities {
            embedded: false,
            external_fallback: true,
            file_transfer: true,
            audio: false,
            clipboard: false,
            split_view: false,
            terminal_based: false,
        }
    }

    fn build_command(&self, _connection: &Connection) -> Option<Vec<String>> {
        // SFTP connections open a file manager via URI, not a CLI command
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_sftp_connection(config: SshConfig) -> Connection {
        Connection::new(
            "Test SFTP".to_string(),
            "example.com".to_string(),
            22,
            ProtocolConfig::Sftp(config),
        )
    }

    #[test]
    fn test_sftp_protocol_metadata() {
        let protocol = SftpProtocol::new();
        assert_eq!(protocol.protocol_id(), "sftp");
        assert_eq!(protocol.display_name(), "SFTP");
        assert_eq!(protocol.default_port(), 22);
    }

    #[test]
    fn test_validate_valid_connection() {
        let protocol = SftpProtocol::new();
        let conn = create_sftp_connection(SshConfig::default());
        assert!(protocol.validate_connection(&conn).is_ok());
    }

    #[test]
    fn test_validate_empty_host() {
        let protocol = SftpProtocol::new();
        let mut conn = create_sftp_connection(SshConfig::default());
        conn.host = String::new();
        assert!(protocol.validate_connection(&conn).is_err());
    }

    #[test]
    fn test_validate_zero_port() {
        let protocol = SftpProtocol::new();
        let mut conn = create_sftp_connection(SshConfig::default());
        conn.port = 0;
        assert!(protocol.validate_connection(&conn).is_err());
    }

    #[test]
    fn test_capabilities_file_transfer() {
        let protocol = SftpProtocol::new();
        let caps = protocol.capabilities();
        assert!(caps.file_transfer);
        assert!(!caps.embedded);
        assert!(!caps.terminal_based);
        assert!(caps.external_fallback);
    }

    #[test]
    fn test_build_command_returns_none() {
        let protocol = SftpProtocol::new();
        let conn = create_sftp_connection(SshConfig::default());
        assert!(protocol.build_command(&conn).is_none());
    }
}
