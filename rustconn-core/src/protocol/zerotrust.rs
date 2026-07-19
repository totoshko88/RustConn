//! Zero Trust protocol handler
//!
//! Delegates command building to provider-specific CLIs (aws, gcloud, az, oci,
//! cloudflared, tsh, tailscale, boundary, hoop) via `ZeroTrustConfig::build_command()`.

use super::{Protocol, ProtocolCapabilities, ProtocolResult};
use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig};

/// Zero Trust protocol handler
///
/// Zero Trust connections use cloud provider CLIs to establish secure connections
/// through identity-aware proxies. The actual command is determined by the
/// provider configuration (AWS SSM, GCP IAP, Azure Bastion, etc.).
#[derive(Debug)]
pub struct ZeroTrustProtocol;

impl ZeroTrustProtocol {
    /// Creates a new Zero Trust protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }
}

impl Default for ZeroTrustProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for ZeroTrustProtocol {
    fn protocol_id(&self) -> &'static str {
        "zerotrust"
    }

    fn display_name(&self) -> &'static str {
        "Zero Trust"
    }

    fn default_port(&self) -> u16 {
        22 // Most ZT providers tunnel SSH
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        let ProtocolConfig::ZeroTrust(ref zt_config) = connection.protocol_config else {
            return Err(ProtocolError::InvalidConfig(
                "Expected ZeroTrust protocol config".to_string(),
            ));
        };

        zt_config.validate()
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities::terminal()
    }

    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
        let ProtocolConfig::ZeroTrust(ref zt_config) = connection.protocol_config else {
            return None;
        };

        let (program, mut args) = zt_config.build_command(connection.username.as_deref());
        args.extend(zt_config.custom_args.clone());

        let mut cmd = vec![program];
        cmd.append(&mut args);
        Some(cmd)
    }
}
