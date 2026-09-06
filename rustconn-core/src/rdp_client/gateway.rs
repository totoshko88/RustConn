//! RD Gateway (Remote Desktop Gateway) support
//!
//! This module provides configuration and utilities for connecting through
//! an RD Gateway server. RD Gateway allows secure RDP connections over HTTPS,
//! enabling access to internal RDP servers from outside the corporate network.
//!
//! # Protocol Overview
//!
//! RD Gateway uses HTTP/HTTPS tunneling (MS-TSGU) to encapsulate RDP traffic:
//! 1. Client connects to gateway via HTTPS (port 443)
//! 2. Client authenticates with gateway credentials
//! 3. Gateway establishes connection to target RDP server
//! 4. RDP traffic is tunneled through the HTTPS connection
//!
//! # IronRDP Support Status
//!
//! Embedded RD Gateway tunneling is implemented behind the `rd-gateway` feature
//! via `ironrdp-mstsgu` (see `client/connection.rs`). Two limitations of that
//! crate are load-bearing:
//!
//! - **HTTP Basic authentication only.** The gateway is challenged with a
//!   `user:password` pair; NTLM/Negotiate and smart-card gateway auth are not
//!   available through the embedded tunnel.
//! - **Target port is fixed at 3389.** `ironrdp-mstsgu` hard-codes the tunnel
//!   target port, so a non-standard RDP port cannot be reached this way. GUI
//!   callers route those connections to the external FreeRDP client instead.
//!
//! When the feature is off, `GatewayConfig` is still parsed and stored so that
//! a connection round-trips unchanged; only the connect path is compiled out.

use serde::{Deserialize, Serialize};

/// RD Gateway configuration
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GatewayConfig {
    /// Whether to use RD Gateway
    pub enabled: bool,
    /// Gateway server hostname
    pub hostname: String,
    /// Gateway server port (default: 443)
    pub port: u16,
    /// Username for gateway authentication
    pub username: Option<String>,
    /// Domain for gateway authentication
    pub domain: Option<String>,
    /// Authentication method
    pub auth_method: GatewayAuthMethod,
    /// Whether to bypass gateway for local addresses
    pub bypass_local: bool,
    /// Custom bypass list (hostnames/IPs that don't use gateway)
    pub bypass_list: Vec<String>,
}

impl GatewayConfig {
    /// Default gateway port (HTTPS)
    pub const DEFAULT_PORT: u16 = 443;

    /// Creates a new gateway configuration
    #[must_use]
    pub fn new(hostname: impl Into<String>) -> Self {
        Self {
            enabled: true,
            hostname: hostname.into(),
            port: Self::DEFAULT_PORT,
            username: None,
            domain: None,
            auth_method: GatewayAuthMethod::default(),
            bypass_local: true,
            bypass_list: Vec::new(),
        }
    }

    /// Creates an always-on gateway configuration for a target behind it.
    ///
    /// Unlike [`GatewayConfig::new`], local-address bypass is disabled. Hosts
    /// reached through an RD Gateway are normally RFC 1918 addresses or
    /// `.internal`/`.local` names, which [`GatewayConfig::should_bypass`]
    /// treats as local; bypassing them would dial an address that only exists
    /// on the far side of the gateway ([issue #246]).
    ///
    /// A zero `port` falls back to [`GatewayConfig::DEFAULT_PORT`].
    ///
    /// [issue #246]: https://github.com/totoshko88/RustConn/issues/246
    #[must_use]
    pub fn always_for_target(
        hostname: impl Into<String>,
        port: u16,
        username: Option<String>,
    ) -> Self {
        Self {
            enabled: true,
            hostname: hostname.into(),
            port: if port == 0 { Self::DEFAULT_PORT } else { port },
            username,
            domain: None,
            // `ironrdp-mstsgu` authenticates the tunnel with HTTP Basic.
            auth_method: GatewayAuthMethod::Basic,
            bypass_local: false,
            bypass_list: Vec::new(),
        }
    }

    /// Creates a disabled gateway configuration
    #[must_use]
    pub const fn disabled() -> Self {
        Self {
            enabled: false,
            hostname: String::new(),
            port: Self::DEFAULT_PORT,
            username: None,
            domain: None,
            auth_method: GatewayAuthMethod::Ntlm,
            bypass_local: true,
            bypass_list: Vec::new(),
        }
    }

    /// Sets the gateway port
    #[must_use]
    pub const fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    /// Sets gateway credentials
    #[must_use]
    pub fn with_credentials(mut self, username: impl Into<String>, domain: Option<String>) -> Self {
        self.username = Some(username.into());
        self.domain = domain;
        self
    }

    /// Sets the authentication method
    #[must_use]
    pub const fn with_auth_method(mut self, method: GatewayAuthMethod) -> Self {
        self.auth_method = method;
        self
    }

    /// Adds a hostname to the bypass list
    #[must_use]
    pub fn with_bypass(mut self, hostname: impl Into<String>) -> Self {
        self.bypass_list.push(hostname.into());
        self
    }

    /// Returns the gateway URL
    #[must_use]
    pub fn url(&self) -> String {
        if self.port == Self::DEFAULT_PORT {
            format!("https://{}", self.hostname)
        } else {
            format!("https://{}:{}", self.hostname, self.port)
        }
    }

    /// Checks if a target host should bypass the gateway
    #[must_use]
    pub fn should_bypass(&self, target_host: &str) -> bool {
        if !self.enabled {
            return true;
        }

        // Check bypass for local addresses
        if self.bypass_local && is_local_address(target_host) {
            return true;
        }

        // Check explicit bypass list
        self.bypass_list
            .iter()
            .any(|h| h.eq_ignore_ascii_case(target_host))
    }

    /// Validates the gateway configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration is invalid.
    pub fn validate(&self) -> Result<(), GatewayError> {
        if !self.enabled {
            return Ok(());
        }

        if self.hostname.is_empty() {
            return Err(GatewayError::InvalidConfig(
                "Gateway hostname is required".to_string(),
            ));
        }

        if self.port == 0 {
            return Err(GatewayError::InvalidConfig(
                "Gateway port cannot be 0".to_string(),
            ));
        }

        Ok(())
    }
}

/// Gateway authentication methods
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GatewayAuthMethod {
    /// NTLM authentication (most common)
    #[default]
    Ntlm,
    /// Kerberos authentication
    Kerberos,
    /// Smart card authentication
    SmartCard,
    /// Basic authentication (username/password over HTTPS)
    Basic,
    /// Cookie-based authentication (for web SSO)
    Cookie,
}

impl GatewayAuthMethod {
    /// Returns a human-readable name for the auth method
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Ntlm => "NTLM",
            Self::Kerberos => "Kerberos",
            Self::SmartCard => "Smart Card",
            Self::Basic => "Basic",
            Self::Cookie => "Cookie/SSO",
        }
    }

    /// Returns whether this method requires a password
    #[must_use]
    pub const fn requires_password(&self) -> bool {
        matches!(self, Self::Ntlm | Self::Basic)
    }
}

/// Gateway-related errors
#[derive(Debug, Clone, thiserror::Error)]
pub enum GatewayError {
    /// Invalid configuration
    #[error("Invalid gateway configuration: {0}")]
    InvalidConfig(String),

    /// Connection to gateway failed
    #[error("Gateway connection failed: {0}")]
    ConnectionFailed(String),

    /// Gateway authentication failed
    #[error("Gateway authentication failed: {0}")]
    AuthenticationFailed(String),

    /// Gateway denied access to target
    #[error("Gateway denied access: {0}")]
    AccessDenied(String),

    /// Gateway protocol error
    #[error("Gateway protocol error: {0}")]
    ProtocolError(String),

    /// Gateway not supported
    #[error("Gateway not supported: {0}")]
    NotSupported(String),
}

/// Resolves the account name sent to the RD Gateway.
///
/// Falls back to the session user when no dedicated gateway user is configured,
/// and qualifies a bare name with the domain (`DOMAIN\user`) because gateways
/// backed by Active Directory reject an unqualified account. Names that already
/// carry a domain prefix or a UPN suffix (`user@domain`) are passed through
/// unchanged. Returns an empty string when no user is known at all.
#[must_use]
pub fn resolve_gateway_user(
    gateway_username: Option<&str>,
    session_username: Option<&str>,
    domain: Option<&str>,
) -> String {
    fn non_empty(value: Option<&str>) -> Option<&str> {
        value.map(str::trim).filter(|v| !v.is_empty())
    }

    let Some(user) = non_empty(gateway_username).or_else(|| non_empty(session_username)) else {
        return String::new();
    };

    if user.contains('\\') || user.contains('@') {
        return user.to_string();
    }

    non_empty(domain).map_or_else(|| user.to_string(), |domain| format!("{domain}\\{user}"))
}

/// Checks if an address is a local/private address
#[expect(
    clippy::case_sensitive_file_extension_comparisons,
    reason = "extension comes from a controlled allow-list, not user input - case is already normalised upstream"
)]
fn is_local_address(host: &str) -> bool {
    // Check for localhost variants
    if host.eq_ignore_ascii_case("localhost") || host == "127.0.0.1" || host == "::1" {
        return true;
    }

    // Check for private IP ranges
    if let Ok(ip) = host.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(ipv4) => {
                ipv4.is_private() || ipv4.is_loopback() || ipv4.is_link_local()
            }
            std::net::IpAddr::V6(ipv6) => ipv6.is_loopback(),
        };
    }

    // Check for .local/.internal domain suffix (already case-insensitive via to_lowercase)
    let host_lower = host.to_lowercase();
    if host_lower.ends_with(".local") || host_lower.ends_with(".internal") {
        return true;
    }

    false
}

/// Gateway connection state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GatewayState {
    /// Not using gateway
    #[default]
    Disabled,
    /// Connecting to gateway
    Connecting,
    /// Authenticating with gateway
    Authenticating,
    /// Connected through gateway
    Connected,
    /// Gateway connection failed
    Failed,
}

impl GatewayState {
    /// Returns whether the gateway is in a connected state
    #[must_use]
    pub const fn is_connected(&self) -> bool {
        matches!(self, Self::Connected)
    }

    /// Returns whether the gateway is in a transitional state
    #[must_use]
    pub const fn is_transitioning(&self) -> bool {
        matches!(self, Self::Connecting | Self::Authenticating)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gateway_config_new() {
        let config = GatewayConfig::new("gateway.example.com");
        assert!(config.enabled);
        assert_eq!(config.hostname, "gateway.example.com");
        assert_eq!(config.port, 443);
    }

    #[test]
    fn test_gateway_config_disabled() {
        let config = GatewayConfig::disabled();
        assert!(!config.enabled);
    }

    #[test]
    fn always_for_target_never_bypasses_internal_hosts() {
        let config = GatewayConfig::always_for_target("gw.example.com", 443, None);
        assert!(config.enabled);
        assert!(!config.bypass_local);
        // Exactly the hosts an RD Gateway exists for (issue #246): private
        // addresses and internal names must still go through the tunnel.
        for target in [
            "10.10.0.5",
            "192.168.1.20",
            "172.16.4.9",
            "server.internal",
            "server.local",
        ] {
            assert!(
                !config.should_bypass(target),
                "target must not bypass the gateway: {target}"
            );
        }
    }

    #[test]
    fn always_for_target_falls_back_to_default_port() {
        let config = GatewayConfig::always_for_target("gw.example.com", 0, None);
        assert_eq!(config.port, GatewayConfig::DEFAULT_PORT);

        let custom = GatewayConfig::always_for_target("gw.example.com", 444, None);
        assert_eq!(custom.port, 444);
    }

    #[test]
    fn gateway_user_prefers_dedicated_account() {
        assert_eq!(
            resolve_gateway_user(Some("gwadmin"), Some("alice"), None),
            "gwadmin"
        );
    }

    #[test]
    fn gateway_user_falls_back_to_session_account() {
        assert_eq!(resolve_gateway_user(None, Some("alice"), None), "alice");
        assert_eq!(
            resolve_gateway_user(Some("  "), Some("alice"), None),
            "alice"
        );
    }

    #[test]
    fn gateway_user_is_domain_qualified() {
        assert_eq!(
            resolve_gateway_user(None, Some("alice"), Some("CORP")),
            "CORP\\alice"
        );
    }

    #[test]
    fn gateway_user_keeps_existing_qualification() {
        assert_eq!(
            resolve_gateway_user(None, Some("CORP\\alice"), Some("OTHER")),
            "CORP\\alice"
        );
        assert_eq!(
            resolve_gateway_user(None, Some("alice@corp.example"), Some("CORP")),
            "alice@corp.example"
        );
    }

    #[test]
    fn gateway_user_is_empty_without_any_account() {
        assert!(resolve_gateway_user(None, None, Some("CORP")).is_empty());
        assert!(resolve_gateway_user(Some(""), Some(" "), None).is_empty());
    }

    #[test]
    fn test_gateway_url() {
        let config = GatewayConfig::new("gateway.example.com");
        assert_eq!(config.url(), "https://gateway.example.com");

        let config_custom_port = config.with_port(8443);
        assert_eq!(config_custom_port.url(), "https://gateway.example.com:8443");
    }

    #[test]
    fn test_gateway_bypass_local() {
        let config = GatewayConfig::new("gateway.example.com");

        assert!(config.should_bypass("localhost"));
        assert!(config.should_bypass("127.0.0.1"));
        assert!(config.should_bypass("192.168.1.100"));
        assert!(config.should_bypass("10.0.0.1"));
        assert!(!config.should_bypass("external.example.com"));
    }

    #[test]
    fn test_gateway_bypass_list() {
        let config = GatewayConfig::new("gateway.example.com").with_bypass("internal.example.com");

        assert!(config.should_bypass("internal.example.com"));
        assert!(!config.should_bypass("external.example.com"));
    }

    #[test]
    fn test_gateway_validate() {
        let valid = GatewayConfig::new("gateway.example.com");
        assert!(valid.validate().is_ok());

        let invalid = GatewayConfig {
            enabled: true,
            hostname: String::new(),
            ..Default::default()
        };
        assert!(invalid.validate().is_err());

        let disabled = GatewayConfig::disabled();
        assert!(disabled.validate().is_ok());
    }

    #[test]
    fn test_is_local_address() {
        assert!(is_local_address("localhost"));
        assert!(is_local_address("127.0.0.1"));
        assert!(is_local_address("::1"));
        assert!(is_local_address("192.168.1.1"));
        assert!(is_local_address("10.0.0.1"));
        assert!(is_local_address("172.16.0.1"));
        assert!(is_local_address("server.local"));
        assert!(!is_local_address("8.8.8.8"));
        assert!(!is_local_address("example.com"));
    }

    #[test]
    fn test_auth_method_display() {
        assert_eq!(GatewayAuthMethod::Ntlm.display_name(), "NTLM");
        assert_eq!(GatewayAuthMethod::Kerberos.display_name(), "Kerberos");
    }

    #[test]
    fn test_auth_method_requires_password() {
        assert!(GatewayAuthMethod::Ntlm.requires_password());
        assert!(GatewayAuthMethod::Basic.requires_password());
        assert!(!GatewayAuthMethod::SmartCard.requires_password());
    }
}
