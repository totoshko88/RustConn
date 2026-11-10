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
    /// A proxy with a login is left out: it needs a relay that stays alive beside
    /// the browser, which a launcher that replaces its own process cannot offer.
    fn proxied_browser_command(
        connection: &Connection,
        config: &WebConfig,
        proxy: &str,
    ) -> Option<Vec<String>> {
        use crate::models::WebBrowserMode;

        if config.socks_proxy_username.is_some() {
            return None;
        }
        let browser = match config.browser_mode {
            WebBrowserMode::Custom => config.browser.as_deref()?.trim().to_string(),
            WebBrowserMode::Embedded if cfg!(feature = "web-embedded") => return None,
            _ => first_installed_proxy_browser()?,
        };
        let args =
            proxied_browser_args(&browser, proxy, &connection.host, config.private_mode).ok()?;
        let mut cmd = vec![browser];
        cmd.extend(args);
        Some(cmd)
    }
}

/// Browsers that can be pointed at a proxy for one launch, most capable first.
const PROXY_BROWSERS: &[&str] = &[
    "chromium",
    "chromium-browser",
    "google-chrome",
    "google-chrome-stable",
    "brave-browser",
    "vivaldi",
    "vivaldi-stable",
    "microsoft-edge",
    "microsoft-edge-stable",
    "firefox",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum BrowserFamily {
    Chromium,
    Firefox,
}

fn browser_family(browser: &str) -> Option<BrowserFamily> {
    let name = browser.to_lowercase();
    if ["chrom", "brave", "vivaldi", "edge", "opera"]
        .iter()
        .any(|family| name.contains(family))
    {
        Some(BrowserFamily::Chromium)
    } else if ["firefox", "librewolf"]
        .iter()
        .any(|family| name.contains(family))
    {
        Some(BrowserFamily::Firefox)
    } else {
        None
    }
}

/// Whether `browser` can be pointed at a proxy for one launch.
#[must_use]
pub fn can_use_socks_proxy(browser: &str) -> bool {
    browser_family(browser).is_some()
}

/// The first installed browser that can use a proxy for one launch.
#[must_use]
pub fn first_installed_proxy_browser() -> Option<String> {
    PROXY_BROWSERS
        .iter()
        .find(|browser| crate::which::is_available(browser))
        .map(|browser| (*browser).to_string())
}

/// A new owner-only profile directory, on tmpfs where the session has one.
fn new_proxy_profile() -> std::io::Result<std::path::PathBuf> {
    use std::os::unix::fs::DirBuilderExt;

    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from);
    let dir = base.join(format!("rustconn-proxy-browser-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(dir)
}

/// Firefox reads a proxy only from its profile. `socks_remote_dns` keeps name
/// lookups inside the proxy, and the last two preferences stop localhost and
/// first-run pages from going around it.
fn firefox_preferences(host: &str, port: &str) -> String {
    format!(
        "user_pref(\"network.proxy.type\", 1);\n\
         user_pref(\"network.proxy.socks\", \"{host}\");\n\
         user_pref(\"network.proxy.socks_port\", {port});\n\
         user_pref(\"network.proxy.socks_version\", 5);\n\
         user_pref(\"network.proxy.socks_remote_dns\", true);\n\
         user_pref(\"network.proxy.no_proxies_on\", \"\");\n\
         user_pref(\"network.proxy.allow_hijacking_localhost\", true);\n\
         user_pref(\"browser.shell.checkDefaultBrowser\", false);\n\
         user_pref(\"browser.aboutwelcome.enabled\", false);\n\
         user_pref(\"datareporting.policy.dataSubmissionEnabled\", false);\n"
    )
}

/// The arguments that make `browser` load `url` through `proxy` and nothing else.
///
/// Each launch gets a profile of its own: a running instance would otherwise take
/// over and load the page directly, which is the leak a proxy setting exists to
/// prevent.
///
/// `private_mode` opens Firefox with `--private-window`, which works inside the
/// separate `--no-remote` instance, so history and cookies stay out of the
/// profile. Chromium gets no `--incognito`: its profile is already a fresh one.
///
/// # Errors
///
/// Returns [`ProtocolError::InvalidConfig`] when `browser` cannot use a proxy for
/// one launch, `proxy` is not valid, or the profile cannot be written.
pub fn proxied_browser_args(
    browser: &str,
    proxy: &str,
    url: &str,
    private_mode: bool,
) -> ProtocolResult<Vec<String>> {
    let family = browser_family(browser).ok_or_else(|| {
        ProtocolError::InvalidConfig(format!("{browser} cannot use a SOCKS proxy"))
    })?;
    let proxy = normalize_socks_proxy(proxy)?;
    let unwritable =
        |e: std::io::Error| ProtocolError::InvalidConfig(format!("browser profile: {e}"));
    let profile = new_proxy_profile().map_err(unwritable)?;
    let profile_arg = profile.to_string_lossy().into_owned();
    Ok(match family {
        BrowserFamily::Chromium => vec![
            format!("--user-data-dir={profile_arg}"),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
            format!("--proxy-server={proxy}"),
            url.to_string(),
        ],
        BrowserFamily::Firefox => {
            let (host, port) = proxy
                .trim_start_matches("socks5://")
                .rsplit_once(':')
                .ok_or_else(|| ProtocolError::InvalidConfig("SOCKS proxy address".to_string()))?;
            let host = host.trim_matches(['[', ']']);
            std::fs::write(profile.join("user.js"), firefox_preferences(host, port))
                .map_err(unwritable)?;
            let mut args = vec![
                "--no-remote".to_string(),
                "--new-instance".to_string(),
                "--profile".to_string(),
                profile_arg,
            ];
            if private_mode {
                args.push("--private-window".to_string());
            }
            args.push(url.to_string());
            args
        }
    })
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
        if web_config.socks_proxy_username.is_some()
            && (web_config.socks_proxy.is_none()
                || web_config.socks_proxy_password_variable.is_none())
        {
            return Err(ProtocolError::InvalidConfig(
                "A SOCKS proxy login needs a proxy and a password variable".to_string(),
            ));
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
    fn firefox_gets_the_proxy_through_a_profile_of_its_own() {
        let conn = proxied(WebBrowserMode::Custom, Some("firefox"));
        let cmd = WebProtocol::new().build_command(&conn).unwrap();
        assert!(cmd.iter().any(|a| a == "--no-remote"));
        let at = cmd.iter().position(|a| a == "--profile").unwrap();
        let profile = std::path::Path::new(&cmd[at + 1]);
        let prefs = std::fs::read_to_string(profile.join("user.js")).unwrap();
        assert!(
            prefs.contains(r#""network.proxy.socks", "proxy""#),
            "{prefs}"
        );
        assert!(
            prefs.contains(r#""network.proxy.socks_port", 1080"#),
            "{prefs}"
        );
        assert!(
            prefs.contains(r#""network.proxy.socks_remote_dns", true"#),
            "{prefs}"
        );
        std::fs::remove_dir_all(profile).unwrap();
    }

    #[test]
    fn private_mode_keeps_firefox_private_behind_the_proxy() {
        let mut conn = proxied(WebBrowserMode::Custom, Some("firefox"));
        let profile_of = |cmd: &[String]| {
            let at = cmd.iter().position(|a| a == "--profile").unwrap();
            std::path::PathBuf::from(&cmd[at + 1])
        };

        let public = WebProtocol::new().build_command(&conn).unwrap();
        assert!(
            !public.iter().any(|a| a == "--private-window"),
            "{public:?}"
        );
        std::fs::remove_dir_all(profile_of(&public)).unwrap();

        if let ProtocolConfig::Web(config) = &mut conn.protocol_config {
            config.private_mode = true;
        }
        let private = WebProtocol::new().build_command(&conn).unwrap();
        let flag = private.iter().position(|a| a == "--private-window");
        assert_eq!(flag, Some(private.len() - 2), "{private:?}");
        assert_eq!(
            private.last().map(String::as_str),
            Some("https://example.com")
        );
        std::fs::remove_dir_all(profile_of(&private)).unwrap();
    }

    #[test]
    fn a_browser_that_cannot_use_the_proxy_is_not_launched() {
        let conn = proxied(WebBrowserMode::Custom, Some("lynx"));
        assert!(WebProtocol::new().build_command(&conn).is_none());
        assert!(!can_use_socks_proxy("lynx"));
    }

    #[test]
    fn a_proxy_with_a_login_is_not_launched_without_a_relay() {
        let mut conn = proxied(WebBrowserMode::Custom, Some("google-chrome"));
        if let ProtocolConfig::Web(config) = &mut conn.protocol_config {
            config.socks_proxy_username = Some("alice".to_string());
        }
        assert!(WebProtocol::new().build_command(&conn).is_none());
    }

    #[test]
    fn a_login_needs_a_password_variable() {
        let mut conn = proxied(WebBrowserMode::Embedded, None);
        if let ProtocolConfig::Web(config) = &mut conn.protocol_config {
            config.socks_proxy_username = Some("alice".to_string());
        }
        assert!(WebProtocol::new().validate_connection(&conn).is_err());
        if let ProtocolConfig::Web(config) = &mut conn.protocol_config {
            config.socks_proxy_password_variable = Some("proxy_pw".to_string());
        }
        assert!(WebProtocol::new().validate_connection(&conn).is_ok());
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
