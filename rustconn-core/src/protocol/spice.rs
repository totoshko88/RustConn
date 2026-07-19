//! SPICE protocol handler

use super::{Protocol, ProtocolCapabilities, ProtocolResult};
use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig, SpiceConfig};

/// SPICE protocol handler
///
/// Implements the Protocol trait for SPICE connections.
/// SPICE sessions open in an external viewer (virt-viewer/remote-viewer).
#[derive(Debug)]
pub struct SpiceProtocol;

impl SpiceProtocol {
    /// Creates a new SPICE protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Extracts SPICE config from a connection, returning an error if not SPICE
    fn get_spice_config(connection: &Connection) -> ProtocolResult<&SpiceConfig> {
        match &connection.protocol_config {
            ProtocolConfig::Spice(config) => Ok(config),
            _ => Err(ProtocolError::InvalidConfig(
                "Connection is not a SPICE connection".to_string(),
            )),
        }
    }
}

impl Default for SpiceProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for SpiceProtocol {
    fn protocol_id(&self) -> &'static str {
        "spice"
    }

    fn display_name(&self) -> &'static str {
        "SPICE"
    }

    fn default_port(&self) -> u16 {
        5900
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        let spice_config = Self::get_spice_config(connection)?;

        // Unix socket mode — skip host/port validation
        if let Some(ref socket_path) = spice_config.unix_socket_path {
            if socket_path.as_os_str().is_empty() {
                return Err(ProtocolError::InvalidConfig(
                    "Unix socket path cannot be empty".to_string(),
                ));
            }
            // Validate shared folders even in socket mode
            for folder in &spice_config.shared_folders {
                if folder.local_path.as_os_str().is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "Shared folder local path cannot be empty".to_string(),
                    ));
                }
                if folder.share_name.is_empty() {
                    return Err(ProtocolError::InvalidConfig(
                        "Shared folder share name cannot be empty".to_string(),
                    ));
                }
            }
            return Ok(());
        }

        // TCP mode — validate host and port
        if connection.host.is_empty() {
            return Err(ProtocolError::InvalidConfig(
                "Host cannot be empty".to_string(),
            ));
        }

        if connection.port == 0 {
            return Err(ProtocolError::InvalidConfig("Port cannot be 0".to_string()));
        }

        // Validate CA certificate path exists if TLS is enabled and path is specified
        if spice_config.tls_enabled
            && let Some(ca_path) = &spice_config.ca_cert_path
            && !ca_path.as_os_str().is_empty()
            && !spice_config.skip_cert_verify
            && !ca_path.exists()
        {
            return Err(ProtocolError::InvalidConfig(format!(
                "CA certificate file not found: {}",
                ca_path.display()
            )));
        }

        // Validate shared folders have non-empty paths and names
        for folder in &spice_config.shared_folders {
            if folder.local_path.as_os_str().is_empty() {
                return Err(ProtocolError::InvalidConfig(
                    "Shared folder local path cannot be empty".to_string(),
                ));
            }
            if folder.share_name.is_empty() {
                return Err(ProtocolError::InvalidConfig(
                    "Shared folder share name cannot be empty".to_string(),
                ));
            }
        }

        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities {
            multi_monitor: true,
            usb_redirection: true,
            audio: true,
            ..ProtocolCapabilities::external_only(true)
        }
    }

    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
        // Non-SPICE config: fall back to a plain spice:// URI.
        let ProtocolConfig::Spice(ref spice_config) = connection.protocol_config else {
            let uri = crate::spice_client::build_spice_uri(
                None,
                false,
                &connection.host,
                connection.port,
            );
            return Some(vec!["remote-viewer".to_string(), uri]);
        };

        // URI building is shared with `build_spice_viewer_args` so the socket /
        // TLS / plain schemes never diverge again.
        let uri = crate::spice_client::build_spice_uri(
            spice_config.unix_socket_path.as_deref(),
            spice_config.tls_enabled,
            &connection.host,
            connection.port,
        );
        let mut cmd = vec!["remote-viewer".to_string(), uri];

        // CA cert only applies in TCP+TLS mode (a unix socket has no TLS layer).
        if spice_config.unix_socket_path.is_none()
            && let Some(ref ca_cert) = spice_config.ca_cert_path
        {
            cmd.push(format!("--spice-ca-file={}", ca_cert.display()));
        }
        if spice_config.usb_redirection {
            cmd.push("--spice-usbredir-auto-redirect-filter".to_string());
            cmd.push(crate::spice_client::SPICE_USB_AUTO_REDIRECT_FILTER.to_string());
        }
        for folder in &spice_config.shared_folders {
            cmd.push(format!(
                "--spice-shared-dir={}",
                folder.local_path.display()
            ));
        }
        if let Some(ref proxy) = spice_config.proxy {
            if proxy
                .chars()
                .all(|c| c.is_alphanumeric() || ".-:/@_".contains(c))
            {
                cmd.push(format!("--spice-proxy={proxy}"));
            } else {
                tracing::warn!(proxy = %proxy, "Invalid SPICE proxy format, skipping");
            }
        }

        Some(cmd)
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::models::{ProtocolConfig, SharedFolder, SpiceImageCompression};

    fn create_spice_connection(config: SpiceConfig) -> Connection {
        Connection::new(
            "Test SPICE".to_string(),
            "vm.example.com".to_string(),
            5900,
            ProtocolConfig::Spice(config),
        )
    }

    #[test]
    fn test_spice_protocol_metadata() {
        let protocol = SpiceProtocol::new();
        assert_eq!(protocol.protocol_id(), "spice");
        assert_eq!(protocol.display_name(), "SPICE");
        assert_eq!(protocol.default_port(), 5900);
    }

    #[test]
    fn test_validate_valid_connection() {
        let protocol = SpiceProtocol::new();
        let connection = create_spice_connection(SpiceConfig::default());
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_empty_host() {
        let protocol = SpiceProtocol::new();
        let mut connection = create_spice_connection(SpiceConfig::default());
        connection.host = String::new();
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_zero_port() {
        let protocol = SpiceProtocol::new();
        let mut connection = create_spice_connection(SpiceConfig::default());
        connection.port = 0;
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_with_tls_enabled() {
        let protocol = SpiceProtocol::new();
        let config = SpiceConfig {
            tls_enabled: true,
            skip_cert_verify: true, // Skip verification so we don't need a real cert
            ..Default::default()
        };
        let connection = create_spice_connection(config);
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_missing_ca_cert() {
        let protocol = SpiceProtocol::new();
        let config = SpiceConfig {
            tls_enabled: true,
            ca_cert_path: Some(PathBuf::from("/nonexistent/ca.crt")),
            skip_cert_verify: false,
            ..Default::default()
        };
        let connection = create_spice_connection(config);
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_with_usb_redirection() {
        let protocol = SpiceProtocol::new();
        let config = SpiceConfig {
            usb_redirection: true,
            ..Default::default()
        };
        let connection = create_spice_connection(config);
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_with_shared_folders() {
        let protocol = SpiceProtocol::new();
        let config = SpiceConfig {
            shared_folders: vec![SharedFolder {
                local_path: PathBuf::from("/home/user/share"),
                share_name: "MyShare".to_string(),
            }],
            ..Default::default()
        };
        let connection = create_spice_connection(config);
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_empty_shared_folder_path() {
        let protocol = SpiceProtocol::new();
        let config = SpiceConfig {
            shared_folders: vec![SharedFolder {
                local_path: PathBuf::new(),
                share_name: "MyShare".to_string(),
            }],
            ..Default::default()
        };
        let connection = create_spice_connection(config);
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_empty_shared_folder_name() {
        let protocol = SpiceProtocol::new();
        let config = SpiceConfig {
            shared_folders: vec![SharedFolder {
                local_path: PathBuf::from("/home/user/share"),
                share_name: String::new(),
            }],
            ..Default::default()
        };
        let connection = create_spice_connection(config);
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_with_image_compression() {
        let protocol = SpiceProtocol::new();
        for compression in [
            SpiceImageCompression::Auto,
            SpiceImageCompression::Off,
            SpiceImageCompression::Glz,
            SpiceImageCompression::Lz,
            SpiceImageCompression::Quic,
        ] {
            let config = SpiceConfig {
                image_compression: Some(compression),
                ..Default::default()
            };
            let connection = create_spice_connection(config);
            assert!(protocol.validate_connection(&connection).is_ok());
        }
    }

    #[test]
    fn test_validate_with_clipboard_disabled() {
        let protocol = SpiceProtocol::new();
        let config = SpiceConfig {
            clipboard_enabled: false,
            ..Default::default()
        };
        let connection = create_spice_connection(config);
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_unix_socket_ignores_empty_host_port() {
        let protocol = SpiceProtocol::new();
        let config = SpiceConfig {
            unix_socket_path: Some(PathBuf::from("/run/libvirt/qemu/vm-spice.sock")),
            ..Default::default()
        };
        let mut connection = create_spice_connection(config);
        // Socket mode must be valid even without host/port.
        connection.host = String::new();
        connection.port = 0;
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_unix_socket_empty_path() {
        let protocol = SpiceProtocol::new();
        let config = SpiceConfig {
            unix_socket_path: Some(PathBuf::new()),
            ..Default::default()
        };
        let connection = create_spice_connection(config);
        let err = protocol
            .validate_connection(&connection)
            .expect_err("empty socket path must fail");
        assert!(err.to_string().contains("Unix socket path cannot be empty"));
    }

    #[test]
    fn test_validate_unix_socket_bad_shared_folder() {
        let protocol = SpiceProtocol::new();
        let config = SpiceConfig {
            unix_socket_path: Some(PathBuf::from("/run/spice.sock")),
            shared_folders: vec![SharedFolder {
                local_path: PathBuf::new(),
                share_name: "Share".to_string(),
            }],
            ..Default::default()
        };
        let connection = create_spice_connection(config);
        // Shared folders are validated even in socket mode.
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_build_command_unix_socket_usb_flag() {
        let protocol = SpiceProtocol::new();
        let config = SpiceConfig {
            unix_socket_path: Some(PathBuf::from("/run/spice.sock")),
            usb_redirection: true,
            ..Default::default()
        };
        let connection = create_spice_connection(config);
        let cmd = protocol.build_command(&connection).expect("command");
        assert!(cmd.contains(&"spice+unix:///run/spice.sock".to_string()));
        // USB flag must match build_spice_viewer_args (no divergence).
        assert!(cmd.contains(&"--spice-usbredir-auto-redirect-filter".to_string()));
    }
}
