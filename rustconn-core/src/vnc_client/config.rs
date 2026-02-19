//! VNC client configuration

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

/// Configuration for VNC client connection
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VncClientConfig {
    /// Target hostname or IP address
    pub host: String,

    /// Target port (default: 5900)
    pub port: u16,

    /// Password for authentication (if required)
    #[serde(skip)]
    pub password: Option<SecretString>,

    /// Preferred pixel format
    pub pixel_format: PixelFormat,

    /// Preferred encodings in order of preference
    pub encodings: Vec<VncEncoding>,

    /// Allow shared session (multiple clients)
    pub shared: bool,

    /// View-only mode (no input forwarding)
    pub view_only: bool,

    /// Connection timeout in seconds
    pub timeout_secs: u64,
}

impl Default for VncClientConfig {
    fn default() -> Self {
        Self {
            host: String::new(),
            port: 5900,
            password: None,
            pixel_format: PixelFormat::Bgra,
            encodings: vec![
                VncEncoding::Tight,
                VncEncoding::Zrle,
                VncEncoding::CopyRect,
                VncEncoding::Raw,
            ],
            shared: true,
            view_only: false,
            timeout_secs: 30,
        }
    }
}

impl VncClientConfig {
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
        self.password = Some(SecretString::from(password.into()));
        self
    }

    /// Sets view-only mode
    #[must_use]
    pub const fn with_view_only(mut self, view_only: bool) -> Self {
        self.view_only = view_only;
        self
    }

    /// Sets shared session mode
    #[must_use]
    pub const fn with_shared(mut self, shared: bool) -> Self {
        self.shared = shared;
        self
    }

    /// Returns the server address as "host:port"
    #[must_use]
    pub fn server_address(&self) -> String {
        format!("{}:{}", self.host, self.port)
    }
}

/// Pixel format for VNC framebuffer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PixelFormat {
    /// BGRA format (blue, green, red, alpha) - native for most displays
    #[default]
    Bgra,
    /// RGBA format (red, green, blue, alpha)
    Rgba,
    /// RGB format (red, green, blue) - no alpha
    Rgb,
}

/// VNC encoding types
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VncEncoding {
    /// Raw encoding (uncompressed)
    Raw,
    /// `CopyRect` encoding (copy rectangle from another location)
    CopyRect,
    /// Tight encoding (compressed)
    Tight,
    /// ZRLE encoding (Zlib Run-Length Encoding)
    Zrle,
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn test_config_builder() {
        let config = VncClientConfig::new("192.168.1.100")
            .with_port(5901)
            .with_password("secret")
            .with_view_only(true)
            .with_shared(false);

        assert_eq!(config.host, "192.168.1.100");
        assert_eq!(config.port, 5901);
        assert_eq!(
            config.password.as_ref().map(ExposeSecret::expose_secret),
            Some("secret")
        );
        assert!(config.view_only);
        assert!(!config.shared);
    }

    #[test]
    fn test_server_address() {
        let config = VncClientConfig::new("localhost").with_port(5900);
        assert_eq!(config.server_address(), "localhost:5900");
    }

    #[test]
    fn test_default_encodings() {
        let config = VncClientConfig::default();
        assert!(config.encodings.contains(&VncEncoding::Tight));
        assert!(config.encodings.contains(&VncEncoding::Raw));
    }
}
