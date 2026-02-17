//! SPICE client configuration

// Allow struct with multiple bools - SPICE has many boolean options
#![allow(clippy::struct_excessive_bools)]

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Configuration for SPICE client connection
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SpiceClientConfig {
    /// Target hostname or IP address
    pub host: String,

    /// Target port (default: 5900)
    pub port: u16,

    /// Password for authentication
    #[serde(skip_serializing)]
    pub password: Option<String>,

    /// Desired screen width
    pub width: u16,

    /// Desired screen height
    pub height: u16,

    /// Enable TLS encryption
    pub tls_enabled: bool,

    /// CA certificate path for TLS verification
    pub ca_cert_path: Option<PathBuf>,

    /// Skip TLS certificate verification (insecure)
    pub skip_cert_verify: bool,

    /// Enable clipboard sharing
    pub clipboard_enabled: bool,

    /// Enable USB redirection
    pub usb_redirection: bool,

    /// Shared folders for webdav
    pub shared_folders: Vec<SpiceSharedFolder>,

    /// Image compression setting
    pub image_compression: SpiceImageCompression,

    /// Enable audio playback
    pub audio_playback: bool,

    /// Enable audio recording
    pub audio_record: bool,

    /// Connection timeout in seconds
    pub timeout_secs: u64,

    /// Security protocol to use
    pub security_protocol: SpiceSecurityProtocol,

    /// SPICE proxy URL (e.g. `http://proxy:3128`) for tunnelled connections
    pub proxy: Option<String>,
}

/// SPICE security protocol options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SpiceSecurityProtocol {
    /// Automatic selection (server decides)
    #[default]
    Auto,
    /// Plain (no encryption)
    Plain,
    /// TLS encryption
    Tls,
    /// SASL authentication
    Sasl,
}

/// SPICE image compression options
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum SpiceImageCompression {
    /// Automatic selection
    #[default]
    Auto,
    /// No compression
    Off,
    /// GLZ compression
    Glz,
    /// LZ compression
    Lz,
    /// QUIC compression
    Quic,
}

/// Shared folder configuration for SPICE webdav
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SpiceSharedFolder {
    /// Local path to share
    pub local_path: PathBuf,
    /// Name visible to the remote system
    pub share_name: String,
    /// Whether the share is read-only
    pub read_only: bool,
}

impl SpiceSharedFolder {
    /// Creates a new shared folder configuration
    #[must_use]
    pub fn new(local_path: impl Into<PathBuf>, share_name: impl Into<String>) -> Self {
        Self {
            local_path: local_path.into(),
            share_name: share_name.into(),
            read_only: false,
        }
    }

    /// Sets the folder as read-only
    #[must_use]
    pub const fn with_read_only(mut self, read_only: bool) -> Self {
        self.read_only = read_only;
        self
    }
}

impl Default for SpiceClientConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 5900,
            password: None,
            width: 1280,
            height: 720,
            tls_enabled: false,
            ca_cert_path: None,
            skip_cert_verify: false,
            clipboard_enabled: true,
            usb_redirection: false,
            shared_folders: Vec::new(),
            image_compression: SpiceImageCompression::default(),
            audio_playback: true,
            audio_record: false,
            timeout_secs: 30,
            security_protocol: SpiceSecurityProtocol::default(),
            proxy: None,
        }
    }
}

impl SpiceClientConfig {
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

    /// Sets the password
    #[must_use]
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(password.into());
        self
    }

    /// Sets the resolution
    #[must_use]
    pub const fn with_resolution(mut self, width: u16, height: u16) -> Self {
        self.width = width;
        self.height = height;
        self
    }

    /// Enables or disables TLS
    #[must_use]
    pub const fn with_tls(mut self, enabled: bool) -> Self {
        self.tls_enabled = enabled;
        self
    }

    /// Sets the CA certificate path
    #[must_use]
    pub fn with_ca_cert(mut self, path: impl Into<PathBuf>) -> Self {
        self.ca_cert_path = Some(path.into());
        self
    }

    /// Sets whether to skip certificate verification
    #[must_use]
    pub const fn with_skip_cert_verify(mut self, skip: bool) -> Self {
        self.skip_cert_verify = skip;
        self
    }

    /// Enables or disables clipboard sharing
    #[must_use]
    pub const fn with_clipboard(mut self, enabled: bool) -> Self {
        self.clipboard_enabled = enabled;
        self
    }

    /// Enables or disables USB redirection
    #[must_use]
    pub const fn with_usb_redirection(mut self, enabled: bool) -> Self {
        self.usb_redirection = enabled;
        self
    }

    /// Adds a shared folder
    #[must_use]
    pub fn with_shared_folder(mut self, folder: SpiceSharedFolder) -> Self {
        self.shared_folders.push(folder);
        self
    }

    /// Sets the image compression
    #[must_use]
    pub const fn with_image_compression(mut self, compression: SpiceImageCompression) -> Self {
        self.image_compression = compression;
        self
    }

    /// Enables or disables audio playback
    #[must_use]
    pub const fn with_audio_playback(mut self, enabled: bool) -> Self {
        self.audio_playback = enabled;
        self
    }

    /// Enables or disables audio recording
    #[must_use]
    pub const fn with_audio_record(mut self, enabled: bool) -> Self {
        self.audio_record = enabled;
        self
    }

    /// Sets the connection timeout
    #[must_use]
    pub const fn with_timeout(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = timeout_secs;
        self
    }

    /// Sets the security protocol
    #[must_use]
    pub const fn with_security_protocol(mut self, protocol: SpiceSecurityProtocol) -> Self {
        self.security_protocol = protocol;
        self
    }

    /// Sets the SPICE proxy URL for tunnelled connections (e.g. Proxmox VE)
    #[must_use]
    pub fn with_proxy(mut self, proxy: impl Into<String>) -> Self {
        self.proxy = Some(proxy.into());
        self
    }

    /// Returns the server address as "host:port"
    #[must_use]
    pub fn server_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }

    /// Validates the configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.host.is_empty() {
            return Err("Host cannot be empty".to_string());
        }
        if self.port == 0 {
            return Err("Port cannot be 0".to_string());
        }
        if self.tls_enabled && !self.skip_cert_verify && self.ca_cert_path.is_none() {
            return Err(
                "TLS enabled but no CA certificate provided and skip_cert_verify is false"
                    .to_string(),
            );
        }
        for folder in &self.shared_folders {
            if folder.share_name.is_empty() {
                return Err("Shared folder name cannot be empty".to_string());
            }
            if folder.local_path.as_os_str().is_empty() {
                return Err("Shared folder path cannot be empty".to_string());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_builder() {
        let config = SpiceClientConfig::new("192.168.1.100")
            .with_port(5901)
            .with_password("secret")
            .with_resolution(1920, 1080)
            .with_tls(true)
            .with_skip_cert_verify(true)
            .with_clipboard(true)
            .with_usb_redirection(true);

        assert_eq!(config.host, "192.168.1.100");
        assert_eq!(config.port, 5901);
        assert_eq!(config.password, Some("secret".to_string()));
        assert_eq!(config.width, 1920);
        assert_eq!(config.height, 1080);
        assert!(config.tls_enabled);
        assert!(config.skip_cert_verify);
        assert!(config.clipboard_enabled);
        assert!(config.usb_redirection);
    }

    #[test]
    fn test_server_address() {
        let config = SpiceClientConfig::new("localhost").with_port(5900);
        assert_eq!(config.server_address(), "localhost:5900");
    }

    #[test]
    fn test_default_values() {
        let config = SpiceClientConfig::default();
        assert_eq!(config.port, 5900);
        assert!(config.clipboard_enabled);
        assert!(!config.usb_redirection);
        assert!(config.audio_playback);
        assert!(!config.audio_record);
    }

    #[test]
    fn test_shared_folder() {
        let folder = SpiceSharedFolder::new("/home/user/share", "MyShare").with_read_only(true);
        assert_eq!(folder.local_path, PathBuf::from("/home/user/share"));
        assert_eq!(folder.share_name, "MyShare");
        assert!(folder.read_only);
    }

    #[test]
    fn test_config_with_shared_folder() {
        let folder = SpiceSharedFolder::new("/tmp", "TempShare");
        let config = SpiceClientConfig::new("localhost").with_shared_folder(folder);
        assert_eq!(config.shared_folders.len(), 1);
        assert_eq!(config.shared_folders[0].share_name, "TempShare");
    }

    #[test]
    fn test_validate_valid_config() {
        let config = SpiceClientConfig::new("localhost").with_port(5900);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_empty_host() {
        let config = SpiceClientConfig::default();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_zero_port() {
        let config = SpiceClientConfig::new("localhost").with_port(0);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_tls_without_cert() {
        let config = SpiceClientConfig::new("localhost")
            .with_tls(true)
            .with_skip_cert_verify(false);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_validate_tls_with_skip_verify() {
        let config = SpiceClientConfig::new("localhost")
            .with_tls(true)
            .with_skip_cert_verify(true);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_validate_empty_shared_folder_name() {
        let folder = SpiceSharedFolder::new("/tmp", "");
        let config = SpiceClientConfig::new("localhost").with_shared_folder(folder);
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_security_protocol_default() {
        assert_eq!(
            SpiceSecurityProtocol::default(),
            SpiceSecurityProtocol::Auto
        );
    }

    #[test]
    fn test_image_compression_default() {
        assert_eq!(
            SpiceImageCompression::default(),
            SpiceImageCompression::Auto
        );
    }
}
