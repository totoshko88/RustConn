//! RDP client configuration

// Allow struct with multiple bools - RDP has many boolean options
#![allow(clippy::struct_excessive_bools)]

use serde::{Deserialize, Serialize};

/// Configuration for RDP client connection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RdpClientConfig {
    /// Target hostname or IP address
    pub host: String,

    /// Target port (default: 3389)
    pub port: u16,

    /// Username for authentication
    pub username: Option<String>,

    /// Password for authentication
    #[serde(skip_serializing)]
    pub password: Option<String>,

    /// Domain for authentication
    pub domain: Option<String>,

    /// Desired screen width
    pub width: u16,

    /// Desired screen height
    pub height: u16,

    /// Color depth (16, 24, or 32)
    pub color_depth: u8,

    /// Enable clipboard sharing
    pub clipboard_enabled: bool,

    /// Enable audio redirection
    pub audio_enabled: bool,

    /// Connection timeout in seconds
    pub timeout_secs: u64,

    /// Ignore certificate errors (insecure, for testing)
    pub ignore_certificate: bool,

    /// Enable NLA (Network Level Authentication)
    pub nla_enabled: bool,

    /// Security protocol to use
    pub security_protocol: RdpSecurityProtocol,
}

/// RDP security protocol options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum RdpSecurityProtocol {
    /// Automatic selection (server decides)
    #[default]
    Auto,
    /// Standard RDP security
    Rdp,
    /// TLS encryption
    Tls,
    /// Network Level Authentication
    Nla,
    /// Extended NLA (`CredSSP` with early user auth)
    Ext,
}

impl Default for RdpClientConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 3389,
            username: None,
            password: None,
            domain: None,
            width: 1280,
            height: 720,
            color_depth: 32,
            clipboard_enabled: true,
            audio_enabled: false,
            timeout_secs: 30,
            ignore_certificate: true,
            nla_enabled: true,
            security_protocol: RdpSecurityProtocol::default(),
        }
    }
}

impl RdpClientConfig {
    /// Creates a new configuration with the specified host
    #[must_use]
    pub fn new(host: impl Into<String>) -> Self {
        Self {
            host: host.into(),
            ..Default::default()
        }
    }

    /// Sets the port
    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets the username
    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Sets the password
    #[must_use]
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Sets the domain
    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Sets the resolution
    #[must_use]
    pub const fn with_resolution(mut self, width: u16, height: u16) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Sets the color depth
    #[must_use]
    pub const fn with_color_depth(mut self, depth: u8) -> Self {
        self.color_depth = depth;
        self
    }

    /// Enables or disables clipboard sharing
    #[must_use]
    pub const fn with_clipboard(mut self, enabled: bool) -> Self {
        self.clipboard_enabled = enabled;
        self
    }

    /// Enables or disables NLA
    #[must_use]
    pub const fn with_nla(mut self, enabled: bool) -> Self {
        self.nla_enabled = enabled;
        self
    }

    /// Returns the server address as "host:port"
    #[must_use]
    pub fn server_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = RdpClientConfig::new("192.168.1.100")
            .with_port(3390)
            .with_username("admin")
            .with_password("secret")
            .with_domain("CORP")
            .with_resolution(1920, 1080)
            .with_color_depth(24);

        assert_eq!(config.host, "192.168.1.100");
        assert_eq!(config.port, 3390);
        assert_eq!(config.username, Some("admin".to_string()));
        assert_eq!(config.password, Some("secret".to_string()));
        assert_eq!(config.domain, Some("CORP".to_string()));
        assert_eq!(config.width, 1920);
        assert_eq!(config.height, 1080);
        assert_eq!(config.color_depth, 24);
    }

    #[test]
    fn test_server_address() {
        let config = RdpClientConfig::new("localhost").with_port(3389);
        assert_eq!(config.server_address(), "localhost:3389");
    }

    #[test]
    fn test_default_values() {
        let config = RdpClientConfig::default();
        assert_eq!(config.port, 3389);
        assert_eq!(config.color_depth, 32);
        assert!(config.clipboard_enabled);
        assert!(config.nla_enabled);
    }
}
