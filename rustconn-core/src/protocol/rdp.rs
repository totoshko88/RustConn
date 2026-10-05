//! RDP protocol handler

use super::{Protocol, ProtocolCapabilities, ProtocolResult};
use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig, RdpConfig};

/// RDP protocol handler
///
/// Implements the Protocol trait for RDP connections.
/// Native RDP embedding is available via IronRDP (`rdp-embedded` feature flag).
#[derive(Debug)]
pub struct RdpProtocol;

impl RdpProtocol {
    /// Creates a new RDP protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Extracts RDP config from a connection, returning an error if not RDP
    fn get_rdp_config(connection: &Connection) -> ProtocolResult<&RdpConfig> {
        match &connection.protocol_config {
            ProtocolConfig::Rdp(config) => Ok(config),
            _ => Err(ProtocolError::InvalidConfig(
                "Connection is not an RDP connection".to_string(),
            )),
        }
    }
}

impl Default for RdpProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for RdpProtocol {
    fn protocol_id(&self) -> &'static str {
        "rdp"
    }

    fn display_name(&self) -> &'static str {
        "RDP"
    }

    fn default_port(&self) -> u16 {
        3389
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        let rdp_config = Self::get_rdp_config(connection)?;

        // Validate host is not empty
        if connection.host.is_empty() {
            return Err(ProtocolError::InvalidConfig(
                "Host cannot be empty".to_string(),
            ));
        }

        // Validate port is in valid range
        if connection.port == 0 {
            return Err(ProtocolError::InvalidConfig("Port cannot be 0".to_string()));
        }

        // Validate color depth if specified
        if let Some(depth) = rdp_config.color_depth
            && !matches!(depth, 8 | 15 | 16 | 24 | 32)
        {
            return Err(ProtocolError::InvalidConfig(format!(
                "Invalid color depth: {depth}. Must be 8, 15, 16, 24, or 32"
            )));
        }

        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities {
            multi_monitor: true,
            ..ProtocolCapabilities::graphical(true, true, true)
        }
    }

    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
        // The launch order the GUI uses and the connection's pinned client. This
        // used to name `xfreerdp` outright, which a system with only FreeRDP 3 —
        // Debian and Ubuntu's `freerdp3-x11`, and the snap, which bundles
        // `xfreerdp3` — does not have, so `rustconn-cli connect` failed there.
        // With no client installed the unsuffixed name stays, so the error still
        // names something to install.
        let pinned = Self::get_rdp_config(connection)
            .ok()
            .and_then(|rdp| rdp.freerdp_client_override.as_deref());
        let binary = super::detection::resolve_freerdp_client(pinned)
            .unwrap_or_else(|| "xfreerdp".to_string());
        self.build_command_with_binary(&binary, connection)
    }
}

impl RdpProtocol {
    /// Builds the FreeRDP argument list without a binary name.
    ///
    /// Callers that perform runtime detection (GUI, CLI) should use this
    /// and prepend the detected binary themselves.
    #[must_use]
    pub fn build_args(connection: &Connection) -> Option<Vec<String>> {
        let mut args = vec![format!("/v:{}:{}", connection.host, connection.port)];

        if let Some(ref username) = connection.username {
            args.push(format!("/u:{username}"));
        }
        if let Some(ref domain) = connection.domain {
            args.push(format!("/d:{domain}"));
        }

        if let ProtocolConfig::Rdp(ref rdp_config) = connection.protocol_config {
            if let Some(ref resolution) = rdp_config.resolution {
                args.push(format!("/w:{}", resolution.width));
                args.push(format!("/h:{}", resolution.height));
            }
            if let Some(depth) = rdp_config.color_depth {
                args.push(format!("/bpp:{depth}"));
            }
            // Always explicit: without an audio flag FreeRDP leaves both
            // AudioPlayback and RemoteConsoleAudio false, which the server
            // reads as "no audio device in this session" (issue #245).
            args.push(rdp_config.effective_audio_mode().freerdp_arg().to_string());
            // Security layer selection (FreeRDP /sec: flags)
            if let Some(sec_arg) = rdp_config.security_layer.freerdp_arg() {
                args.push(sec_arg.to_string());
            }
            // TLS security level for legacy server compatibility
            if let Some(level) = rdp_config.tls_security_level {
                args.push(format!("/tls-seclevel:{level}"));
            }
            // KDC proxy for Kerberos, the same rule as `build_freerdp_args`.
            if !rdp_config.disable_nla
                && let Some(kerberos) = super::freerdp::freerdp_kerberos_arg(
                    rdp_config.kerberos_enabled,
                    rdp_config.kdc_proxy_url.as_deref(),
                )
            {
                args.push(kerberos);
            }
            // RD Gateway. FreeRDP 3.x removed the short `/g:` / `/gu:` aliases
            // in favour of the unified `/gateway:` option; the old aliases are
            // rejected as "Unexpected keyword" and the client exits before
            // connecting (issue #187). This path is the CLI/standalone builder
            // and used to still emit the 2.x aliases, so a gateway connection
            // launched through it broke on a modern FreeRDP. It now matches the
            // embedded/external builder in `freerdp::push_gateway_args`: reuse
            // the session credentials, add an explicit gateway user only when it
            // differs from the session user, and skip an empty hostname.
            if let Some(ref gateway) = rdp_config.gateway
                && !gateway.hostname.is_empty()
            {
                let mut value = format!("g:{}:{}", gateway.hostname, gateway.port);
                if let Some(ref gw_user) = gateway.username
                    && !gw_user.is_empty()
                    && connection.username.as_deref() != Some(gw_user.as_str())
                {
                    value.push_str(",u:");
                    value.push_str(gw_user);
                }
                args.push(format!("/gateway:{value}"));
            }
            for folder in &rdp_config.shared_folders {
                if folder.share_name.contains(',') || folder.share_name.contains('/') {
                    tracing::warn!(share_name = %folder.share_name, "Skipping shared folder with invalid share name");
                    continue;
                }
                args.push(format!(
                    "/drive:{},{}",
                    folder.share_name,
                    folder.local_path.display()
                ));
            }
            // Sizing and custom arguments go through the same rules as every
            // other FreeRDP launch: smart sizing wins over dynamic resolution,
            // which FreeRDP refuses beside it (issue #341), and a custom argument
            // carrying a secret field or selecting a shell or proxy is dropped.
            // This builder used to ignore both switches and keep its own copy of
            // the argument filter.
            let sizing = super::freerdp::FreeRdpSizing::resolve(
                rdp_config.dynamic_resolution,
                rdp_config.smart_sizing,
                &rdp_config.custom_args,
            );
            if let Some(flag) = sizing.flag {
                args.push(flag.to_string());
            }
            args.extend(super::freerdp::filter_extra_args(
                &rdp_config.custom_args,
                sizing,
            ));
        }

        Some(args)
    }

    /// Builds a full command with the given binary name prepended.
    #[must_use]
    pub fn build_command_with_binary(
        &self,
        binary: &str,
        connection: &Connection,
    ) -> Option<Vec<String>> {
        Self::build_args(connection).map(|args| {
            let mut cmd = vec![binary.to_string()];
            cmd.extend(args);
            cmd
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProtocolConfig, Resolution};

    fn create_rdp_connection(config: RdpConfig) -> Connection {
        Connection::new(
            "Test RDP".to_string(),
            "windows.example.com".to_string(),
            3389,
            ProtocolConfig::Rdp(config),
        )
    }

    #[test]
    fn test_rdp_protocol_metadata() {
        let protocol = RdpProtocol::new();
        assert_eq!(protocol.protocol_id(), "rdp");
        assert_eq!(protocol.display_name(), "RDP");
        assert_eq!(protocol.default_port(), 3389);
    }

    #[test]
    fn test_validate_valid_connection() {
        let protocol = RdpProtocol::new();
        let connection = create_rdp_connection(RdpConfig::default());
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_empty_host() {
        let protocol = RdpProtocol::new();
        let mut connection = create_rdp_connection(RdpConfig::default());
        connection.host = String::new();
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_zero_port() {
        let protocol = RdpProtocol::new();
        let mut connection = create_rdp_connection(RdpConfig::default());
        connection.port = 0;
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_valid_color_depth() {
        let protocol = RdpProtocol::new();
        for depth in [8, 15, 16, 24, 32] {
            let config = RdpConfig {
                color_depth: Some(depth),
                ..Default::default()
            };
            let connection = create_rdp_connection(config);
            assert!(protocol.validate_connection(&connection).is_ok());
        }
    }

    #[test]
    fn test_validate_invalid_color_depth() {
        let protocol = RdpProtocol::new();
        let config = RdpConfig {
            color_depth: Some(12), // Invalid
            ..Default::default()
        };
        let connection = create_rdp_connection(config);
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_with_resolution() {
        let protocol = RdpProtocol::new();
        let config = RdpConfig {
            resolution: Some(Resolution::new(1920, 1080)),
            ..Default::default()
        };
        let connection = create_rdp_connection(config);
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    fn count_option(args: &[String], name: &str) -> usize {
        args.iter()
            .filter(|arg| arg.trim_start_matches(['/', '+', '-']).starts_with(name))
            .count()
    }

    fn cli_args(config: RdpConfig) -> Vec<String> {
        RdpProtocol::build_args(&create_rdp_connection(config)).expect("RDP arguments")
    }

    /// The CLI builder ignored both sizing switches; it now resolves them like
    /// every other FreeRDP launch (issue #341).
    #[test]
    fn build_args_carries_the_sizing_switches() {
        let default = cli_args(RdpConfig::default());
        assert_eq!(count_option(&default, "dynamic-resolution"), 1);
        assert_eq!(count_option(&default, "smart-sizing"), 0);

        let smart = cli_args(RdpConfig {
            smart_sizing: true,
            ..RdpConfig::default()
        });
        assert!(smart.contains(&"/smart-sizing".to_string()));
        assert_eq!(count_option(&smart, "dynamic-resolution"), 0);

        let off = cli_args(RdpConfig {
            dynamic_resolution: false,
            ..RdpConfig::default()
        });
        assert_eq!(count_option(&off, "dynamic-resolution"), 0);
    }

    /// A custom `/smart-sizing` must not go out beside `/dynamic-resolution`,
    /// the pair FreeRDP refuses.
    #[test]
    fn build_args_lets_a_custom_smart_sizing_win() {
        let args = cli_args(RdpConfig {
            custom_args: vec![
                "/smart-sizing:1920x1080".to_string(),
                "/dynamic-resolution".to_string(),
            ],
            ..RdpConfig::default()
        });
        assert_eq!(count_option(&args, "dynamic-resolution"), 0);
        assert!(args.contains(&"/smart-sizing:1920x1080".to_string()));
        assert_eq!(count_option(&args, "smart-sizing"), 1);
    }
}

#[cfg(test)]
mod custom_argument_security_tests {
    use super::*;

    #[test]
    fn build_args_filters_composite_aliases_and_normalized_blocked_prefixes() {
        let config = RdpConfig {
            custom_args: vec![
                "/gateway:g:host,p:session-secret".to_string(),
                "/gateway:g:host,PASSWORD:password-secret".to_string(),
                "/gateway:g:host, gp:gateway-secret".to_string(),
                "/gateway:g:host,/GATEWAY-PASSWORD:alias-secret".to_string(),
                " //PTH:hash-secret".to_string(),
                "--password".to_string(),
                "split-secret".to_string(),
                "  /SHELL:command-secret".to_string(),
                "\t//PrOxY:proxy-secret".to_string(),
                "/gateway:g:host,u:user".to_string(),
            ],
            ..RdpConfig::default()
        };
        let connection = Connection::new(
            "Test RDP".to_string(),
            "windows.example.com".to_string(),
            3389,
            ProtocolConfig::Rdp(config),
        );

        let args = RdpProtocol::build_args(&connection).expect("RDP arguments");

        assert!(args.iter().any(|arg| arg == "/gateway:g:host,u:user"));
        for secret in [
            "session-secret",
            "password-secret",
            "gateway-secret",
            "alias-secret",
            "hash-secret",
            "split-secret",
            "command-secret",
            "proxy-secret",
        ] {
            assert!(args.iter().all(|arg| !arg.contains(secret)));
        }
    }
}

#[cfg(test)]
mod gateway_syntax_tests {
    use super::*;
    use crate::models::RdpGateway;

    fn rdp_connection_with_user(gateway: RdpGateway, username: &str) -> Connection {
        let config = RdpConfig {
            gateway: Some(gateway),
            ..RdpConfig::default()
        };
        let mut connection = Connection::new(
            "GW RDP".to_string(),
            "target.example.com".to_string(),
            3389,
            ProtocolConfig::Rdp(config),
        );
        connection.username = Some(username.to_string());
        connection
    }

    /// The CLI/standalone builder must emit the FreeRDP 3.x `/gateway:` option,
    /// not the 2.x `/g:` + `/gu:` aliases that a modern client rejects with
    /// "Unexpected keyword" before connecting (issue #187).
    #[test]
    fn build_args_emits_unified_gateway_option() {
        let connection = rdp_connection_with_user(
            RdpGateway {
                hostname: "gw.example.com".to_string(),
                port: 443,
                username: Some("gwuser".to_string()),
            },
            "alice",
        );

        let args = RdpProtocol::build_args(&connection).expect("RDP arguments");

        assert!(args.contains(&"/gateway:g:gw.example.com:443,u:gwuser".to_string()));
        assert!(
            args.iter().all(|arg| !arg.starts_with("/g:")),
            "the removed 2.x /g: alias must not be emitted"
        );
        assert!(
            args.iter().all(|arg| !arg.starts_with("/gu:")),
            "the removed 2.x /gu: alias must not be emitted"
        );
    }

    /// A gateway user identical to the session user is redundant — FreeRDP
    /// reuses the session credentials for the gateway — so it is omitted.
    #[test]
    fn build_args_omits_redundant_gateway_user() {
        let connection = rdp_connection_with_user(
            RdpGateway {
                hostname: "gw.example.com".to_string(),
                port: 443,
                username: Some("alice".to_string()),
            },
            "alice",
        );

        let args = RdpProtocol::build_args(&connection).expect("RDP arguments");

        assert!(args.contains(&"/gateway:g:gw.example.com:443".to_string()));
    }
}

#[cfg(test)]
mod kerberos_kdc_tests {
    use super::*;

    fn kerberos_connection(kerberos_enabled: bool, disable_nla: bool) -> Connection {
        Connection::new(
            "Kerberos RDP".to_string(),
            "server.example.com".to_string(),
            3389,
            ProtocolConfig::Rdp(RdpConfig {
                kerberos_enabled,
                disable_nla,
                kdc_proxy_url: Some("https://gw.example.com/KdcProxy".to_string()),
                ..RdpConfig::default()
            }),
        )
    }

    /// The CLI builder passes the KDC proxy on exactly as `build_freerdp_args`
    /// does: only with Kerberos on and NLA left enabled.
    #[test]
    fn build_args_passes_the_kdc_proxy_only_with_kerberos_and_nla() {
        let args = RdpProtocol::build_args(&kerberos_connection(true, false)).expect("RDP args");
        assert!(args.contains(&"/kerberos:kdc-url:gw.example.com".to_string()));

        for connection in [
            kerberos_connection(false, false),
            kerberos_connection(true, true),
        ] {
            let args = RdpProtocol::build_args(&connection).expect("RDP args");
            assert!(!args.iter().any(|arg| arg.starts_with("/kerberos")));
        }
    }
}
