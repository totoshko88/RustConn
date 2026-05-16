//! Web bookmark protocol handler
//!
//! Web connections open a URL in the user's default browser.
//! They do not create embedded sessions — the browser handles display.
//! Credentials are stored in the configured secret backend for
//! copy-to-clipboard functionality.

use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig, WebConfig};

use super::{Protocol, ProtocolCapabilities, ProtocolResult};

/// Web bookmark protocol handler
///
/// Implements the Protocol trait for web bookmark connections.
/// These connections delegate to the system's default browser via
/// UriLauncher (in the GUI crate) or xdg-open (in the CLI crate).
pub struct WebProtocol;

impl WebProtocol {
    /// Creates a new Web protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Extracts Web config from a connection, returning an error if not Web
    fn get_web_config(connection: &Connection) -> ProtocolResult<&WebConfig> {
        match &connection.protocol_config {
            ProtocolConfig::Web(config) => Ok(config),
            _ => Err(ProtocolError::InvalidConfig(
                "Connection is not a Web connection".to_string(),
            )),
        }
    }
}

impl Default for WebProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for WebProtocol {
    fn protocol_id(&self) -> &'static str {
        "web"
    }

    fn display_name(&self) -> &'static str {
        "Web"
    }

    fn default_port(&self) -> u16 {
        443
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        let _web_config = Self::get_web_config(connection)?;

        // Validate URL is not empty
        if connection.host.is_empty() {
            return Err(ProtocolError::InvalidConfig(
                "URL cannot be empty".to_string(),
            ));
        }

        // Validate URL starts with http:// or https://
        let host_lower = connection.host.to_lowercase();
        if !host_lower.starts_with("http://") && !host_lower.starts_with("https://") {
            return Err(ProtocolError::InvalidConfig(
                "URL must start with http:// or https://".to_string(),
            ));
        }

        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities {
            embedded: false,
            external_fallback: true,
            file_transfer: false,
            audio: false,
            clipboard: false,
            split_view: false,
            terminal_based: false,
            multi_monitor: false,
            usb_redirection: false,
            port_forwarding: false,
            wayland_forwarding: false,
            x11_forwarding: false,
            session_recording: false,
            remote_monitoring: false,
            command_snippets: false,
            wake_on_lan: false,
        }
    }

    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
        let web_config = Self::get_web_config(connection).ok()?;

        let browser = web_config.browser.as_deref().unwrap_or("xdg-open");

        let mut cmd = vec![browser.to_string()];

        // Add private mode flag for known browsers
        if web_config.private_mode {
            let browser_lower = browser.to_lowercase();
            if browser_lower.contains("firefox") {
                cmd.push("--private-window".to_string());
            } else if browser_lower.contains("chrom") || browser_lower.contains("brave") {
                cmd.push("--incognito".to_string());
            }
        }

        cmd.push(connection.host.clone());

        Some(cmd)
    }
}
