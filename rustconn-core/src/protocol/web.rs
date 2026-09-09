//! Web bookmark protocol handler
//!
//! Web connections open a URL in the user's default browser.
//! They do not create embedded sessions — the browser handles display.
//! Credentials are stored in the configured secret backend for
//! copy-to-clipboard functionality.

use super::{Protocol, ProtocolCapabilities, ProtocolResult};
use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig, WebConfig};

/// Web bookmark protocol handler
///
/// Implements the Protocol trait for web bookmark connections.
/// These connections delegate to the system's default browser via
/// UriLauncher (in the GUI crate) or the platform URL opener in the CLI
/// (`open` on macOS, `xdg-open` on Linux — see
/// [`crate::secret::url_open_command`]).
#[derive(Debug)]
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

        // Validate URL starts with http://, https://, or file://
        let host_lower = connection.host.to_lowercase();
        if !host_lower.starts_with("http://")
            && !host_lower.starts_with("https://")
            && !host_lower.starts_with("file://")
        {
            return Err(ProtocolError::InvalidConfig(
                "URL must start with http://, https://, or file://".to_string(),
            ));
        }

        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities {
            #[cfg(feature = "web-embedded")]
            embedded: true,
            #[cfg(not(feature = "web-embedded"))]
            embedded: false,
            external_fallback: true,
            file_transfer: false,
            audio: false,
            clipboard: false,
            #[cfg(feature = "web-embedded")]
            split_view: true,
            #[cfg(not(feature = "web-embedded"))]
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
        use crate::models::WebBrowserMode;

        let web_config = Self::get_web_config(connection).ok()?;

        match web_config.browser_mode {
            // Embedded mode: the session manager creates the widget directly, so
            // there is no command to build. A build without `web-embedded` has no
            // widget to create and hands the URL to the system browser instead —
            // the fallback lives here, at the point of use, rather than in
            // `WebBrowserMode`, so the stored mode survives a build that cannot
            // honour it (see [`crate::models::WebBrowserMode`]).
            WebBrowserMode::Embedded => {
                #[cfg(feature = "web-embedded")]
                {
                    None
                }
                #[cfg(not(feature = "web-embedded"))]
                {
                    tracing::info!(
                        "browser_mode is embedded but this build has no WebView; \
                         opening the URL in the system browser"
                    );
                    Some(vec![
                        crate::secret::url_open_command().to_string(),
                        connection.host.clone(),
                    ])
                }
            }

            // System mode: delegate to the platform URL opener — `open` on
            // macOS, `xdg-open` on Linux (in the GUI this is intercepted by
            // UriLauncher, but the CLI executes this command literally).
            WebBrowserMode::System => {
                let mut cmd = vec![crate::secret::url_open_command().to_string()];
                cmd.push(connection.host.clone());
                Some(cmd)
            }

            // Custom mode: execute user-specified browser command with URL appended
            WebBrowserMode::Custom => {
                let browser = web_config.browser.as_deref().unwrap_or("").trim();
                if browser.is_empty() {
                    // Empty command — caller displays error notification, no fallback
                    return None;
                }

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
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Connection, ProtocolType, WebBrowserMode, WebConfig};

    fn web_connection(mode: WebBrowserMode) -> Connection {
        let mut conn = Connection::new_ssh("Web".to_string(), "https://example.com".to_string(), 443);
        conn.protocol = ProtocolType::Web;
        conn.protocol_config = ProtocolConfig::Web(WebConfig {
            browser_mode: mode,
            ..WebConfig::default()
        });
        conn
    }

    #[test]
    fn system_mode_uses_the_platform_url_opener() {
        // System mode must not hardcode `xdg-open`: on macOS the opener is
        // `open`. The command's first element is whatever
        // `url_open_command()` returns for the build target, followed by the URL.
        let conn = web_connection(WebBrowserMode::System);
        let cmd = WebProtocol::new()
            .build_command(&conn)
            .expect("system mode builds a command");
        assert_eq!(cmd.first().map(String::as_str), Some(crate::secret::url_open_command()));
        assert_eq!(cmd.last().map(String::as_str), Some("https://example.com"));
    }
}
