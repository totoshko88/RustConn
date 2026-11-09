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

    /// The command for a browser that must use `proxy`, or `None` when none can.
    ///
    /// Only the Chromium family takes a proxy on its command line, and it needs a
    /// profile of its own: a running instance would otherwise take over the launch
    /// and load the page directly, which is the leak a proxy setting exists to
    /// prevent. A system or Firefox launch therefore builds nothing at all.
    fn proxied_browser_command(
        connection: &Connection,
        config: &WebConfig,
        proxy: &str,
    ) -> Option<Vec<String>> {
        let browser = config.browser.as_deref()?.trim();
        let name = browser.to_lowercase();
        let chromium = ["chrom", "brave", "vivaldi", "edge", "opera"]
            .iter()
            .any(|family| name.contains(family));
        if config.browser_mode != crate::models::WebBrowserMode::Custom || !chromium {
            return None;
        }
        let proxy = normalize_socks_proxy(proxy).ok()?;
        let profile = std::env::temp_dir().join(format!("rustconn-socks-{}", connection.id));
        Some(vec![
            browser.to_string(),
            format!("--user-data-dir={}", profile.display()),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
            format!("--proxy-server={proxy}"),
            connection.host.clone(),
        ])
    }
}

/// Checks a SOCKS proxy address and returns it as `socks5://host:port`.
///
/// Accepts `host:port`, `socks5://host:port` and `socks5h://host:port`; an IPv6
/// host goes in brackets. The value ends up on a browser's command line, so only
/// characters a host name can hold pass. Credentials are refused because neither
/// WebKitGTK's proxy URI nor Chromium's `--proxy-server` can use them safely.
///
/// # Errors
///
/// Returns [`ProtocolError::InvalidConfig`] when the address is not a usable
/// SOCKS5 endpoint.
pub fn normalize_socks_proxy(input: &str) -> ProtocolResult<String> {
    let invalid = |reason: &str| Err(ProtocolError::InvalidConfig(reason.to_string()));
    let input = input.trim();
    let lower = input.to_lowercase();
    let address = if lower.starts_with("socks5://") {
        &input["socks5://".len()..]
    } else if lower.starts_with("socks5h://") {
        &input["socks5h://".len()..]
    } else if input.contains("://") {
        return invalid("Only SOCKS5 proxies are supported");
    } else {
        input
    };
    if address.contains('@') {
        return invalid("SOCKS proxy logins are not supported");
    }
    if !address
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || ".-_:[]".contains(c))
    {
        return invalid("The SOCKS proxy address holds invalid characters");
    }
    let Some((host, port)) = address.rsplit_once(':') else {
        return invalid("The SOCKS proxy must be given as host:port");
    };
    let bare_ipv6 = host.contains(':') && !(host.starts_with('[') && host.ends_with(']'));
    if host.is_empty() || host.starts_with('-') || bare_ipv6 {
        return invalid("The SOCKS proxy host is not valid");
    }
    if !port.parse::<u16>().is_ok_and(|p| p != 0) {
        return invalid("The SOCKS proxy port must be between 1 and 65535");
    }
    Ok(format!("socks5://{address}"))
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
        let web_config = Self::get_web_config(connection)?;

        if let Some(proxy) = &web_config.socks_proxy {
            normalize_socks_proxy(proxy)?;
            if web_config.tunnel_via.is_some() {
                return Err(ProtocolError::InvalidConfig(
                    "Choose either an SSH tunnel or a SOCKS proxy, not both".to_string(),
                ));
            }
        }

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

        if let Some(proxy) = &web_config.socks_proxy {
            return Self::proxied_browser_command(connection, web_config, proxy);
        }

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
        let mut conn =
            Connection::new_ssh("Web".to_string(), "https://example.com".to_string(), 443);
        conn.protocol = ProtocolType::Web;
        conn.protocol_config = ProtocolConfig::Web(WebConfig {
            browser_mode: mode,
            ..WebConfig::default()
        });
        conn
    }

    #[test]
    fn socks_addresses_are_normalized() {
        for (input, expected) in [
            ("proxy.lan:1080", "socks5://proxy.lan:1080"),
            (" socks5://127.0.0.1:9050 ", "socks5://127.0.0.1:9050"),
            ("SOCKS5h://proxy:1", "socks5://proxy:1"),
            ("[::1]:1080", "socks5://[::1]:1080"),
        ] {
            assert_eq!(normalize_socks_proxy(input).unwrap(), expected);
        }
    }

    #[test]
    fn bad_socks_addresses_are_refused() {
        for input in [
            "",
            "proxy",
            "proxy:0",
            "proxy:70000",
            ":1080",
            "::1:1080",
            "http://proxy:80",
            "socks4://proxy:1080",
            "user:secret@proxy:1080",
            "proxy:1080 --disable-web-security",
            "-x:1080",
        ] {
            assert!(normalize_socks_proxy(input).is_err(), "{input}");
        }
    }

    fn proxied(mode: WebBrowserMode, browser: Option<&str>) -> Connection {
        let mut conn = web_connection(mode);
        if let ProtocolConfig::Web(config) = &mut conn.protocol_config {
            config.browser = browser.map(String::from);
            config.socks_proxy = Some("socks5://proxy:1080".to_string());
        }
        conn
    }

    #[test]
    fn a_chromium_browser_is_given_the_proxy_and_its_own_profile() {
        let conn = proxied(WebBrowserMode::Custom, Some("google-chrome"));
        let cmd = WebProtocol::new().build_command(&conn).unwrap();
        assert!(
            cmd.iter()
                .any(|a| a == "--proxy-server=socks5://proxy:1080")
        );
        assert!(cmd.iter().any(|a| a.starts_with("--user-data-dir=")));
        assert_eq!(cmd.last().map(String::as_str), Some("https://example.com"));
    }

    #[test]
    fn a_browser_that_cannot_use_the_proxy_is_not_launched() {
        let handler = WebProtocol::new();
        assert!(
            handler
                .build_command(&proxied(WebBrowserMode::System, None))
                .is_none()
        );
        assert!(
            handler
                .build_command(&proxied(WebBrowserMode::Custom, Some("firefox")))
                .is_none()
        );
    }

    #[test]
    fn a_tunnel_and_a_proxy_cannot_be_combined() {
        let mut conn = proxied(WebBrowserMode::Embedded, None);
        assert!(WebProtocol::new().validate_connection(&conn).is_ok());
        if let ProtocolConfig::Web(config) = &mut conn.protocol_config {
            config.tunnel_via = Some(uuid::Uuid::new_v4());
        }
        assert!(WebProtocol::new().validate_connection(&conn).is_err());
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
        assert_eq!(
            cmd.first().map(String::as_str),
            Some(crate::secret::url_open_command())
        );
        assert_eq!(cmd.last().map(String::as_str), Some("https://example.com"));
    }
}
