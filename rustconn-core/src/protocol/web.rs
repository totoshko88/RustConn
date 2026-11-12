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
    ///
    /// The same holds for the profile: `rustconn-cli connect` replaces itself with
    /// the browser, so nothing is left to delete the profile when it exits. It is
    /// kept, and the sweep at the start of this call removes such leftovers.
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
        remove_stale_browser_profiles();
        let (args, profile) =
            proxied_browser_args(&browser, proxy, &connection.host, config.private_mode).ok()?;
        profile.keep();
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

/// How a browser takes a proxy for one launch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowserFamily {
    /// Chromium and its derivatives: `--proxy-server`.
    Chromium,
    /// Firefox and its derivatives: a profile with proxy preferences.
    Firefox,
}

/// The family of `browser`, a binary name or path, judged by its name.
#[must_use]
pub fn browser_family(browser: &str) -> Option<BrowserFamily> {
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

/// The Chromium-family names among the proxy-capable browsers, most capable
/// first.
pub fn chromium_proxy_browsers() -> impl Iterator<Item = &'static str> {
    PROXY_BROWSERS
        .iter()
        .copied()
        .filter(|browser| browser_family(browser) == Some(BrowserFamily::Chromium))
}

/// The first installed browser that can use a proxy for one launch.
#[must_use]
pub fn first_installed_proxy_browser() -> Option<String> {
    PROXY_BROWSERS
        .iter()
        .find(|browser| crate::which::is_available(browser))
        .map(|browser| (*browser).to_string())
}

/// A browser profile directory that is deleted when this value drops.
///
/// The owner keeps it for as long as the browser runs: the profile holds the
/// session's cookies, history and cache, which must not outlive it.
#[derive(Debug)]
pub struct BrowserProfile {
    /// Empty once [`Self::keep`] has handed the directory over.
    dir: std::path::PathBuf,
}

impl BrowserProfile {
    /// The profile directory.
    #[must_use]
    pub fn path(&self) -> &std::path::Path {
        &self.dir
    }

    /// Leaves the directory on disk, for a browser nothing will wait for.
    /// [`remove_stale_browser_profiles`] deletes it later.
    pub fn keep(mut self) {
        self.dir = std::path::PathBuf::new();
    }
}

impl Drop for BrowserProfile {
    fn drop(&mut self) {
        if self.dir.as_os_str().is_empty() {
            return;
        }
        // ponytail: the removal runs where the value drops, which in the GUI is
        // the GTK main thread. On Linux the profile is on tmpfs and that is quick,
        // but a large Chromium cache on disk ($TMPDIR on macOS) can pause the
        // window for a moment when a browser exits. Upgrade path: hand the path to
        // a worker thread, as the startup sweep in `rustconn/src/app.rs` does. Not
        // now: a thread cut short at quit would break the promise that a profile
        // is deleted when RustConn quits, and
        // `a_dropped_profile_is_deleted_and_a_kept_one_is_not` relies on the
        // removal being done when `drop` returns.
        if let Err(e) = std::fs::remove_dir_all(&self.dir) {
            tracing::debug!(
                profile = %self.dir.display(),
                error = %e,
                "Could not remove the browser profile; a later sweep will"
            );
        }
    }
}

/// Where proxied browser profiles live: tmpfs where the session has one.
fn browser_profile_base() -> std::path::PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|dir| !dir.is_empty())
        .map_or_else(std::env::temp_dir, std::path::PathBuf::from)
}

/// A new owner-only profile directory under [`browser_profile_base`].
fn new_proxy_profile() -> std::io::Result<BrowserProfile> {
    use std::os::unix::fs::DirBuilderExt;

    let dir =
        browser_profile_base().join(format!("rustconn-proxy-browser-{}", uuid::Uuid::new_v4()));
    std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
    Ok(BrowserProfile { dir })
}

/// How old an unused browser profile must be before a sweep deletes it.
///
/// Long enough for any browser left open within a login session, as for the mc
/// wrapper directories.
const STALE_PROFILE_AFTER: std::time::Duration = std::time::Duration::from_hours(12);

/// Deletes `rustconn-*-browser-*` profiles left behind by a crash, a killed
/// RustConn or `rustconn-cli connect`.
///
/// Covers the `rustconn-proxy-browser-*` profiles and the
/// `rustconn-tunnel-browser-*` ones from 0.24.10 and older. A profile is
/// deleted once it is [`STALE_PROFILE_AFTER`] old and no browser holds its lock.
pub fn remove_stale_browser_profiles() {
    prune_browser_profiles(&browser_profile_base(), std::time::SystemTime::now());
}

// ponytail: a profile counts as in use only while its browser's lock symlink
// (Chromium `SingletonLock`, Firefox `lock` on Linux) exists. A killed browser
// leaves the lock behind, so its profile stays until logout empties
// XDG_RUNTIME_DIR; macOS Firefox locks `.parentlock` with flock and is guarded by
// the age alone. Read the lock's PID if leftovers ever matter.
fn prune_browser_profiles(base: &std::path::Path, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !(name.starts_with("rustconn-") && name.contains("-browser-")) {
            continue;
        }
        let path = entry.path();
        // `symlink_metadata`: a planted symlink must not lead the sweep elsewhere.
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        let stale = meta.is_dir()
            && meta
                .modified()
                .ok()
                .and_then(|modified| now.duration_since(modified).ok())
                .is_some_and(|age| age > STALE_PROFILE_AFTER);
        let locked = || {
            ["SingletonLock", "lock"]
                .iter()
                .any(|lock| std::fs::symlink_metadata(path.join(lock)).is_ok())
        };
        if !stale || locked() {
            continue;
        }
        match std::fs::remove_dir_all(&path) {
            Ok(()) => tracing::debug!(profile = %path.display(), "Removed a stale browser profile"),
            Err(e) => tracing::debug!(
                profile = %path.display(),
                error = %e,
                "Could not remove a stale browser profile"
            ),
        }
    }
}

/// Chromium's switch that keeps WebRTC behind the proxy: a SOCKS5 proxy carries
/// no UDP, and WebRTC otherwise reaches STUN servers directly and hands the
/// page this computer's own addresses.
const CHROMIUM_WEBRTC_PROXY_ONLY: &str =
    "--force-webrtc-ip-handling-policy=disable_non_proxied_udp";

/// Firefox reads a proxy only from its profile. `socks_remote_dns` keeps name
/// lookups inside the proxy, the next two preferences stop localhost and
/// first-run pages from going around it, and the two `peerconnection` ones keep
/// WebRTC on the proxy and off the local interface addresses, as
/// [`CHROMIUM_WEBRTC_PROXY_ONLY`] does for Chromium.
fn firefox_preferences(host: &str, port: &str) -> String {
    format!(
        "user_pref(\"network.proxy.type\", 1);\n\
         user_pref(\"network.proxy.socks\", \"{host}\");\n\
         user_pref(\"network.proxy.socks_port\", {port});\n\
         user_pref(\"network.proxy.socks_version\", 5);\n\
         user_pref(\"network.proxy.socks_remote_dns\", true);\n\
         user_pref(\"network.proxy.no_proxies_on\", \"\");\n\
         user_pref(\"network.proxy.allow_hijacking_localhost\", true);\n\
         user_pref(\"media.peerconnection.ice.proxy_only_if_behind_proxy\", true);\n\
         user_pref(\"media.peerconnection.ice.default_address_only\", true);\n\
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
/// The returned [`BrowserProfile`] deletes the profile when it drops, so the
/// caller keeps it until the browser exits.
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
) -> ProtocolResult<(Vec<String>, BrowserProfile)> {
    let family = browser_family(browser).ok_or_else(|| {
        ProtocolError::InvalidConfig(format!("{browser} cannot use a SOCKS proxy"))
    })?;
    let proxy = normalize_socks_proxy(proxy)?;
    let unwritable =
        |e: std::io::Error| ProtocolError::InvalidConfig(format!("browser profile: {e}"));
    let profile = new_proxy_profile().map_err(unwritable)?;
    let profile_arg = profile.path().to_string_lossy().into_owned();
    let args = match family {
        BrowserFamily::Chromium => vec![
            format!("--user-data-dir={profile_arg}"),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
            format!("--proxy-server={proxy}"),
            CHROMIUM_WEBRTC_PROXY_ONLY.to_string(),
            url.to_string(),
        ],
        BrowserFamily::Firefox => {
            let (host, port) = proxy
                .trim_start_matches("socks5://")
                .rsplit_once(':')
                .ok_or_else(|| ProtocolError::InvalidConfig("SOCKS proxy address".to_string()))?;
            let host = host.trim_matches(['[', ']']);
            std::fs::write(
                profile.path().join("user.js"),
                firefox_preferences(host, port),
            )
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
    };
    Ok((args, profile))
}

/// Checks a Web connection's SOCKS proxy settings: the part of
/// [`WebProtocol::validate_connection`] that does not concern the URL, for
/// callers that store the settings without the rest of that check.
///
/// # Errors
///
/// Returns [`ProtocolError::InvalidConfig`] for an unusable proxy address, a
/// proxy together with an SSH tunnel, or a login (a username or a password
/// variable) that lacks the proxy, the username or the password variable.
pub fn validate_socks_settings(config: &WebConfig) -> ProtocolResult<()> {
    if let Some(proxy) = &config.socks_proxy {
        normalize_socks_proxy(proxy)?;
        if config.tunnel_via.is_some() {
            return Err(ProtocolError::InvalidConfig(
                "Choose either an SSH tunnel or a SOCKS proxy, not both".to_string(),
            ));
        }
    }
    let has_user = config.socks_proxy_username.is_some();
    let has_variable = config.socks_proxy_password_variable.is_some();
    if (has_user || has_variable) && !(config.socks_proxy.is_some() && has_user && has_variable) {
        return Err(ProtocolError::InvalidConfig(
            "A SOCKS proxy login needs a proxy, a username and a password variable".to_string(),
        ));
    }
    Ok(())
}

/// Why a Web connection's SOCKS password variable cannot be used.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SocksPasswordVariableError {
    /// No variable has this name.
    #[error("no global variable is named '{0}'")]
    Missing(String),
    /// The variable is not marked secret, so its value is kept in plain text
    /// in the configuration file instead of the secret backend.
    #[error("the variable '{0}' is not secret")]
    NotSecret(String),
}

/// The variable holding a SOCKS proxy password: the last one named `name`, as
/// substitution picks it, and only when it is marked secret.
///
/// # Errors
///
/// Returns [`SocksPasswordVariableError`] when no variable is named `name` or
/// the one that is is not secret.
pub fn socks_password_variable<'a>(
    variables: &'a [crate::variables::Variable],
    name: &str,
) -> Result<&'a crate::variables::Variable, SocksPasswordVariableError> {
    let variable = variables
        .iter()
        .rev()
        .find(|v| v.name == name)
        .ok_or_else(|| SocksPasswordVariableError::Missing(name.to_string()))?;
    if variable.is_secret {
        Ok(variable)
    } else {
        Err(SocksPasswordVariableError::NotSecret(name.to_string()))
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

        validate_socks_settings(web_config)?;

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
    fn the_chromium_candidates_are_the_proxy_browsers_without_firefox() {
        let chromium: Vec<&str> = chromium_proxy_browsers().collect();
        assert_eq!(
            chromium,
            [
                "chromium",
                "chromium-browser",
                "google-chrome",
                "google-chrome-stable",
                "brave-browser",
                "vivaldi",
                "vivaldi-stable",
                "microsoft-edge",
                "microsoft-edge-stable",
            ]
        );
    }

    #[test]
    fn browser_family_names_the_same_browsers_the_gui_did() {
        for chromium in ["chromium", "brave", "vivaldi", "microsoft-edge", "opera"] {
            assert_eq!(
                browser_family(chromium),
                Some(BrowserFamily::Chromium),
                "{chromium}"
            );
        }
        for firefox in ["firefox", "/usr/bin/librewolf"] {
            assert_eq!(
                browser_family(firefox),
                Some(BrowserFamily::Firefox),
                "{firefox}"
            );
        }
        assert_eq!(browser_family("epiphany"), None);
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
        assert!(
            cmd.iter()
                .any(|a| a == "--force-webrtc-ip-handling-policy=disable_non_proxied_udp"),
            "WebRTC stays behind the proxy: {cmd:?}"
        );
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
        for webrtc in [
            r#""media.peerconnection.ice.proxy_only_if_behind_proxy", true"#,
            r#""media.peerconnection.ice.default_address_only", true"#,
        ] {
            assert!(
                prefs.contains(webrtc),
                "WebRTC stays behind the proxy: {prefs}"
            );
        }
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
    fn a_dropped_profile_is_deleted_and_a_kept_one_is_not() {
        let (_, dropped) =
            proxied_browser_args("firefox", "proxy:1080", "https://example.com", false).unwrap();
        let dropped_dir = dropped.path().to_path_buf();
        assert!(dropped_dir.join("user.js").is_file());
        drop(dropped);
        assert!(!dropped_dir.exists());

        let (_, kept) =
            proxied_browser_args("chromium", "proxy:1080", "https://example.com", false).unwrap();
        let kept_dir = kept.path().to_path_buf();
        kept.keep();
        assert!(kept_dir.is_dir());
        std::fs::remove_dir_all(kept_dir).unwrap();
    }

    #[test]
    fn the_sweep_removes_only_stale_unlocked_browser_profiles() {
        let base = tempfile::tempdir().unwrap();
        let make = |name: &str| {
            let dir = base.path().join(name);
            std::fs::create_dir(&dir).unwrap();
            std::fs::write(dir.join("Local State"), "{}").unwrap();
            dir
        };
        let proxy = make("rustconn-proxy-browser-1");
        let tunnel = make("rustconn-tunnel-browser-2");
        let chromium_running = make("rustconn-proxy-browser-3");
        std::os::unix::fs::symlink("host-4242", chromium_running.join("SingletonLock")).unwrap();
        let firefox_running = make("rustconn-proxy-browser-4");
        std::os::unix::fs::symlink("127.0.0.1:+4243", firefox_running.join("lock")).unwrap();
        let unrelated = make("rustconn-mc-ssh");
        let not_a_dir = base.path().join("rustconn-proxy-browser-file");
        std::fs::write(&not_a_dir, "").unwrap();

        prune_browser_profiles(base.path(), std::time::SystemTime::now());
        for dir in [&proxy, &tunnel, &chromium_running, &firefox_running] {
            assert!(dir.exists(), "a fresh profile is kept: {}", dir.display());
        }

        let later = std::time::SystemTime::now() + STALE_PROFILE_AFTER * 2;
        prune_browser_profiles(base.path(), later);
        assert!(!proxy.exists());
        assert!(!tunnel.exists(), "0.24.10 tunnel profiles are swept too");
        assert!(chromium_running.exists(), "a locked profile is in use");
        assert!(firefox_running.exists(), "a locked profile is in use");
        assert!(unrelated.exists());
        assert!(not_a_dir.exists());
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
    fn a_password_variable_without_a_username_is_refused() {
        let mut config = WebConfig {
            socks_proxy: Some("socks5://proxy:1080".to_string()),
            socks_proxy_password_variable: Some("proxy_pw".to_string()),
            ..WebConfig::default()
        };
        let error = validate_socks_settings(&config).unwrap_err();
        assert!(error.to_string().contains("a username"), "{error}");

        config.socks_proxy = None;
        assert!(validate_socks_settings(&config).is_err());

        config.socks_proxy = Some("socks5://proxy:1080".to_string());
        config.socks_proxy_username = Some("alice".to_string());
        assert!(validate_socks_settings(&config).is_ok());
    }

    #[test]
    fn the_socks_password_must_come_from_a_secret_variable() {
        use crate::variables::Variable;

        let variables = [
            Variable::new("plain_pw", "hunter2"),
            Variable::new_secret("proxy_pw", "old"),
            Variable::new_secret("proxy_pw", "shadowing"),
        ];
        let found = socks_password_variable(&variables, "proxy_pw").unwrap();
        assert_eq!(
            found.value, "shadowing",
            "the last one wins, as in substitution"
        );
        assert_eq!(
            socks_password_variable(&variables, "plain_pw"),
            Err(SocksPasswordVariableError::NotSecret(
                "plain_pw".to_string()
            ))
        );
        assert_eq!(
            socks_password_variable(&variables, "absent"),
            Err(SocksPasswordVariableError::Missing("absent".to_string()))
        );
        // A secret value never lands in the error text.
        let shadowed = [
            Variable::new_secret("plain_pw", "s3cret"),
            Variable::new("plain_pw", "x"),
        ];
        let error = socks_password_variable(&shadowed, "plain_pw").unwrap_err();
        assert!(!error.to_string().contains("s3cret"));
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
