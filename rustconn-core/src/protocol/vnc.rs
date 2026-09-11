//! VNC protocol handler

use super::{Protocol, ProtocolCapabilities, ProtocolResult};
use crate::error::ProtocolError;
use crate::models::{Connection, ProtocolConfig, VncConfig};

/// VNC protocol handler
///
/// Implements the Protocol trait for VNC connections.
/// Native VNC embedding is available via vnc-rs (`vnc-embedded` feature flag).
#[derive(Debug)]
pub struct VncProtocol;

impl VncProtocol {
    /// Creates a new VNC protocol handler
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Returns whether a custom VNC argument is one RustConn refuses to pass on.
    ///
    /// These options can turn an imported connection or a shared config into an
    /// attack: `via`/`proxyserver` reroute the connection through a host of the
    /// attacker's choice, `passwd`/`passwordfile` point the viewer at an
    /// arbitrary file, `securitytypes` weakens authentication, and `listen` turns
    /// the viewer into a server.
    ///
    /// The option *name* is isolated before comparison, because a viewer accepts
    /// more than one spelling of the same option. Measured on TigerVNC 1.15.0:
    /// `--via=gw` is accepted exactly like `-via gw`, while `--NoSuchParam` is
    /// rejected as an unknown parameter — so the double-dash form is real, and a
    /// prefix test against the literal `"-via"` let it through untouched. An
    /// argument that is not an option at all is never treated as one, so a value
    /// that merely happens to begin with these letters is unaffected.
    #[must_use]
    fn is_dangerous_vnc_arg(arg: &str) -> bool {
        const DANGEROUS_OPTIONS: [&str; 6] = [
            "via",
            "passwd",
            "passwordfile",
            "securitytypes",
            "proxyserver",
            "listen",
        ];

        // Only an option can be a dangerous option. One or two leading dashes are
        // both accepted spellings, and the value may be attached with `=`, which
        // the prefix comparison below covers.
        let Some(name) = arg.strip_prefix('-') else {
            return false;
        };
        let name = name.trim_start_matches('-').to_lowercase();

        DANGEROUS_OPTIONS.iter().any(|opt| name.starts_with(opt))
    }

    /// Filters user-supplied viewer arguments down to those safe to pass on.
    ///
    /// Both argument builders — the external-viewer one the GUI uses and the
    /// fallback `build_command` — call this, so their strictness cannot drift.
    /// It rejects NUL/newline injection and every option
    /// [`is_dangerous_vnc_arg`](Self::is_dangerous_vnc_arg) names, and it drops a
    /// blocked option's separate value as well: `-via attacker` arrives as two
    /// argv entries, and removing only the flag left `attacker` behind as a bare
    /// positional argument — which is exactly how a VNC viewer is told which
    /// server to connect to.
    #[must_use]
    fn filter_custom_args(custom_args: &[String]) -> Vec<String> {
        let mut safe = Vec::with_capacity(custom_args.len());
        let mut drop_next_value = false;

        for arg in custom_args {
            // Consume any pending "the previous option was blocked" state up
            // front, so every branch below either uses it or discards it.
            let value_of_blocked_option = std::mem::take(&mut drop_next_value);

            if arg.contains('\0') || arg.contains('\n') {
                tracing::warn!(arg = %arg, "Skipping VNC custom arg with unsafe characters");
                continue;
            }

            if Self::is_dangerous_vnc_arg(arg) {
                tracing::warn!(arg = %arg, "Blocked dangerous VNC custom arg");
                // `--via=gw` already carries its value; `-via gw` does not, so
                // the next entry has to go too.
                drop_next_value = !arg.contains('=');
                continue;
            }

            if value_of_blocked_option && !arg.starts_with('-') {
                tracing::warn!(arg = %arg, "Dropped the orphaned value of a blocked VNC custom arg");
                continue;
            }

            safe.push(arg.clone());
        }

        safe
    }

    /// Extracts VNC config from a connection, returning an error if not VNC
    fn get_vnc_config(connection: &Connection) -> ProtocolResult<&VncConfig> {
        match &connection.protocol_config {
            ProtocolConfig::Vnc(config) => Ok(config),
            _ => Err(ProtocolError::InvalidConfig(
                "Connection is not a VNC connection".to_string(),
            )),
        }
    }

    /// Builds the server address string for an external viewer and port.
    ///
    /// TigerVNC-family viewers expect a display-number form (`host:0`) for the
    /// standard 5900–5999 range and a raw-port form (`host::port`) otherwise;
    /// every other viewer receives plain `host:port`.
    #[must_use]
    fn external_server_address(viewer: &str, host: &str, port: u16) -> String {
        match viewer {
            "vncviewer" | "tigervnc" | "xvnc4viewer" | "gvncviewer" => {
                // These viewers use display-number format for standard ports.
                if port == 5900 {
                    format!("{host}:0")
                } else if port > 5900 && port < 6000 {
                    let display = port - 5900;
                    format!("{host}:{display}")
                } else {
                    // Use :: for raw port numbers.
                    format!("{host}::{port}")
                }
            }
            _ => {
                // Other viewers typically use host:port format.
                format!("{host}:{port}")
            }
        }
    }

    /// Builds the external VNC viewer command as a `(program, args)` pair.
    ///
    /// This is the single source of truth for external VNC viewer argument
    /// construction, shared by the GUI launch paths and the `rustconn` session
    /// fallback, so a `std::process::Command` can be built without a GTK widget.
    /// Viewer *detection* (PATH/filesystem lookup) stays in the caller; this
    /// function assembles arguments only. Passwords are intentionally never
    /// placed on the command line.
    ///
    /// `custom_args` containing NUL or newline bytes are skipped.
    #[must_use]
    pub fn build_external_viewer_command(
        viewer: &str,
        host: &str,
        port: u16,
        config: &VncConfig,
    ) -> (String, Vec<String>) {
        let server = Self::external_server_address(viewer, host, port);
        let mut args: Vec<String> = Vec::new();

        match viewer {
            "vncviewer" | "tigervnc" | "xvnc4viewer" => {
                // TigerVNC/TightVNC/RealVNC style.
                // Only add encoding if it's a single valid value (not comma-separated).
                if let Some(ref encoding) = config.encoding {
                    let enc = encoding.trim();
                    if !enc.is_empty() && !enc.contains(',') {
                        args.push("-PreferredEncoding".to_string());
                        args.push(enc.to_string());
                    }
                }
                if let Some(quality) = config.quality {
                    args.push("-QualityLevel".to_string());
                    args.push(quality.to_string());
                }
                if let Some(compression) = config.compression {
                    args.push("-CompressLevel".to_string());
                    args.push(compression.to_string());
                }
                if config.view_only {
                    args.push("-ViewOnly".to_string());
                }
                // Accept untrusted TLS certificates (VeNCrypt).
                if config.accept_certificate {
                    args.push("-SecurityTypes".to_string());
                    args.push("VeNCrypt,TLSVnc,X509Vnc,VncAuth,None".to_string());
                }
                args.push(server);
            }
            "gvncviewer" => {
                // GTK-VNC viewer.
                args.push(server);
            }
            "remmina" => {
                // Remmina uses a different connection format.
                args.push("-c".to_string());
                args.push(format!("vnc://{host}:{port}"));
            }
            "vinagre" | "krdc" => {
                // Vinagre / KDE Remote Desktop Client.
                args.push(format!("vnc://{host}:{port}"));
            }
            _ => {
                // Generic fallback.
                args.push(server);
            }
        }

        // Custom arguments from config, through the shared filter so this path
        // and `build_command` block exactly the same set.
        args.extend(Self::filter_custom_args(&config.custom_args));

        (viewer.to_string(), args)
    }
}

impl Default for VncProtocol {
    fn default() -> Self {
        Self::new()
    }
}

impl Protocol for VncProtocol {
    fn protocol_id(&self) -> &'static str {
        "vnc"
    }

    fn display_name(&self) -> &'static str {
        "VNC"
    }

    fn default_port(&self) -> u16 {
        5900
    }

    fn validate_connection(&self, connection: &Connection) -> ProtocolResult<()> {
        let vnc_config = Self::get_vnc_config(connection)?;

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

        // Validate compression level if specified (0-9)
        if let Some(compression) = vnc_config.compression
            && compression > 9
        {
            return Err(ProtocolError::InvalidConfig(format!(
                "Invalid compression level: {compression}. Must be 0-9"
            )));
        }

        // Validate quality level if specified (0-9)
        if let Some(quality) = vnc_config.quality
            && quality > 9
        {
            return Err(ProtocolError::InvalidConfig(format!(
                "Invalid quality level: {quality}. Must be 0-9"
            )));
        }

        Ok(())
    }

    fn capabilities(&self) -> ProtocolCapabilities {
        ProtocolCapabilities::graphical(false, false, true)
    }

    fn build_command(&self, connection: &Connection) -> Option<Vec<String>> {
        let mut args = Vec::new();

        if let ProtocolConfig::Vnc(ref vnc_config) = connection.protocol_config {
            if let Some(ref encoding) = vnc_config.encoding {
                args.push("-encoding".to_string());
                args.push(encoding.clone());
            }
            if let Some(compression) = vnc_config.compression {
                args.push("-compresslevel".to_string());
                args.push(compression.to_string());
            }
            if let Some(quality) = vnc_config.quality {
                args.push("-quality".to_string());
                args.push(quality.to_string());
            }
            // Accept untrusted TLS certificates (VeNCrypt)
            if vnc_config.accept_certificate {
                args.push("-SecurityTypes".to_string());
                args.push("VeNCrypt,TLSVnc,X509Vnc,VncAuth,None".to_string());
            }
            // Same shared filter as the external-viewer path: NUL/newline
            // injection, dangerous rerouting/auth-weakening options, and the
            // orphaned value of anything blocked.
            args.extend(Self::filter_custom_args(&vnc_config.custom_args));
        }

        let display = if connection.port >= 5900 {
            connection.port - 5900
        } else {
            connection.port
        };
        args.push(format!("{}:{display}", connection.host));

        let mut cmd = vec!["vncviewer".to_string()];
        cmd.extend(args);
        Some(cmd)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ProtocolConfig;

    fn create_vnc_connection(config: VncConfig) -> Connection {
        Connection::new(
            "Test VNC".to_string(),
            "vnc.example.com".to_string(),
            5900,
            ProtocolConfig::Vnc(config),
        )
    }

    #[test]
    fn test_vnc_protocol_metadata() {
        let protocol = VncProtocol::new();
        assert_eq!(protocol.protocol_id(), "vnc");
        assert_eq!(protocol.display_name(), "VNC");
        assert_eq!(protocol.default_port(), 5900);
    }

    #[test]
    fn test_validate_valid_connection() {
        let protocol = VncProtocol::new();
        let connection = create_vnc_connection(VncConfig::default());
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_validate_empty_host() {
        let protocol = VncProtocol::new();
        let mut connection = create_vnc_connection(VncConfig::default());
        connection.host = String::new();
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_zero_port() {
        let protocol = VncProtocol::new();
        let mut connection = create_vnc_connection(VncConfig::default());
        connection.port = 0;
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_valid_compression() {
        let protocol = VncProtocol::new();
        for compression in 0..=9 {
            let config = VncConfig {
                compression: Some(compression),
                ..Default::default()
            };
            let connection = create_vnc_connection(config);
            assert!(protocol.validate_connection(&connection).is_ok());
        }
    }

    #[test]
    fn test_validate_invalid_compression() {
        let protocol = VncProtocol::new();
        let config = VncConfig {
            compression: Some(15), // Invalid: > 9
            ..Default::default()
        };
        let connection = create_vnc_connection(config);
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_valid_quality() {
        let protocol = VncProtocol::new();
        for quality in 0..=9 {
            let config = VncConfig {
                quality: Some(quality),
                ..Default::default()
            };
            let connection = create_vnc_connection(config);
            assert!(protocol.validate_connection(&connection).is_ok());
        }
    }

    #[test]
    fn test_validate_invalid_quality() {
        let protocol = VncProtocol::new();
        let config = VncConfig {
            quality: Some(10), // Invalid: > 9
            ..Default::default()
        };
        let connection = create_vnc_connection(config);
        assert!(protocol.validate_connection(&connection).is_err());
    }

    #[test]
    fn test_validate_with_encoding() {
        let protocol = VncProtocol::new();
        let config = VncConfig {
            encoding: Some("tight".to_string()),
            ..Default::default()
        };
        let connection = create_vnc_connection(config);
        assert!(protocol.validate_connection(&connection).is_ok());
    }

    #[test]
    fn test_external_server_address_display_number() {
        // Standard port maps to display 0.
        assert_eq!(
            VncProtocol::external_server_address("vncviewer", "host", 5900),
            "host:0"
        );
        // Port within the 5900 range maps to the display offset.
        assert_eq!(
            VncProtocol::external_server_address("tigervnc", "host", 5901),
            "host:1"
        );
        // Raw port outside the range uses the double-colon form.
        assert_eq!(
            VncProtocol::external_server_address("xvnc4viewer", "host", 6100),
            "host::6100"
        );
    }

    #[test]
    fn test_external_server_address_other_viewers() {
        // Non-TigerVNC viewers use plain host:port.
        assert_eq!(
            VncProtocol::external_server_address("remmina", "host", 5901),
            "host:5901"
        );
    }

    #[test]
    fn test_build_external_viewer_command_tigervnc_options() {
        let config = VncConfig {
            encoding: Some("tight".to_string()),
            quality: Some(8),
            compression: Some(6),
            view_only: true,
            accept_certificate: true,
            ..Default::default()
        };
        let (program, args) =
            VncProtocol::build_external_viewer_command("vncviewer", "host", 5901, &config);

        assert_eq!(program, "vncviewer");
        assert!(args.contains(&"-PreferredEncoding".to_string()));
        assert!(args.contains(&"tight".to_string()));
        assert!(args.contains(&"-QualityLevel".to_string()));
        assert!(args.contains(&"8".to_string()));
        assert!(args.contains(&"-CompressLevel".to_string()));
        assert!(args.contains(&"6".to_string()));
        assert!(args.contains(&"-ViewOnly".to_string()));
        assert!(args.contains(&"-SecurityTypes".to_string()));
        // Server address is the last positional argument for TigerVNC.
        assert_eq!(args.last().map(String::as_str), Some("host:1"));
    }

    #[test]
    fn test_build_external_viewer_command_skips_comma_encoding() {
        let config = VncConfig {
            encoding: Some("tight,zrle".to_string()),
            ..Default::default()
        };
        let (_program, args) =
            VncProtocol::build_external_viewer_command("vncviewer", "host", 5900, &config);
        assert!(!args.contains(&"-PreferredEncoding".to_string()));
    }

    #[test]
    fn test_build_external_viewer_command_remmina_uri() {
        let config = VncConfig::default();
        let (program, args) =
            VncProtocol::build_external_viewer_command("remmina", "host", 5901, &config);
        assert_eq!(program, "remmina");
        assert_eq!(args, vec!["-c".to_string(), "vnc://host:5901".to_string()]);
    }

    #[test]
    fn test_build_external_viewer_command_vinagre_krdc_uri() {
        let config = VncConfig::default();
        for viewer in ["vinagre", "krdc"] {
            let (program, args) =
                VncProtocol::build_external_viewer_command(viewer, "host", 5902, &config);
            assert_eq!(program, viewer);
            assert_eq!(args, vec!["vnc://host:5902".to_string()]);
        }
    }

    #[test]
    fn test_build_external_viewer_command_filters_unsafe_custom_args() {
        let config = VncConfig {
            custom_args: vec!["-Fullscreen".to_string(), "bad\narg".to_string()],
            ..Default::default()
        };
        let (_program, args) =
            VncProtocol::build_external_viewer_command("vncviewer", "host", 5900, &config);
        assert!(args.contains(&"-Fullscreen".to_string()));
        assert!(!args.iter().any(|a| a.contains('\n')));
    }

    /// The external-viewer path must block the same dangerous rerouting/auth
    /// arguments the fallback `build_command` does — previously it did not, so a
    /// malicious imported connection could smuggle `-via`/`-passwordfile` into
    /// the GUI viewer launch (the strictness asymmetry the 0.21.11 audit found).
    #[test]
    fn external_viewer_blocks_dangerous_custom_args() {
        let config = VncConfig {
            custom_args: vec![
                "-Fullscreen".to_string(),
                "-via".to_string(),
                "attacker:22".to_string(),
                "-passwordfile".to_string(),
                "/etc/shadow".to_string(),
                "-Listen".to_string(),
            ],
            ..Default::default()
        };
        let (_program, args) =
            VncProtocol::build_external_viewer_command("vncviewer", "host", 5900, &config);
        assert!(args.contains(&"-Fullscreen".to_string()), "benign arg kept");
        for blocked in ["-via", "-passwordfile", "-Listen"] {
            assert!(
                !args.iter().any(|a| a.eq_ignore_ascii_case(blocked)),
                "{blocked} must be blocked on the external-viewer path"
            );
        }
    }

    /// A viewer accepts an option with two leading dashes just as readily as
    /// with one — verified on TigerVNC 1.15.0, where `--via=gw` is accepted and
    /// `--NoSuchParam` is refused. The prefix test against the literal `"-via"`
    /// therefore let the whole double-dash family through.
    #[test]
    fn dangerous_args_cannot_be_smuggled_with_a_double_dash() {
        for smuggled in [
            "--via=attacker",
            "--Via=attacker",
            "--passwordfile=/etc/shadow",
            "--securitytypes=None",
            "--listen",
            "--proxyserver=attacker:5900",
        ] {
            assert!(
                VncProtocol::is_dangerous_vnc_arg(smuggled),
                "{smuggled} must be recognised as dangerous"
            );
        }
    }

    /// Blocking a flag but keeping its value is not blocking it: a VNC viewer
    /// reads a bare positional argument as the server to connect to, so a
    /// leftover `attacker:22` still reroutes the session.
    #[test]
    fn the_value_of_a_blocked_option_is_dropped_too() {
        let filtered = VncProtocol::filter_custom_args(&[
            "-via".to_string(),
            "attacker:22".to_string(),
            "-Fullscreen".to_string(),
        ]);
        assert_eq!(
            filtered,
            vec!["-Fullscreen".to_string()],
            "only the benign option survives; the orphaned value is gone"
        );
    }

    /// A blocked option that carries its own value must not then eat the next,
    /// unrelated argument.
    #[test]
    fn an_attached_value_does_not_consume_the_following_argument() {
        let filtered = VncProtocol::filter_custom_args(&[
            "--via=attacker".to_string(),
            "-Fullscreen".to_string(),
        ]);
        assert_eq!(filtered, vec!["-Fullscreen".to_string()]);
    }

    /// The blocked set applies to options, not to values. A benign argument that
    /// merely starts with the same letters is not an option and stays.
    #[test]
    fn a_non_option_value_is_not_mistaken_for_a_dangerous_option() {
        assert!(!VncProtocol::is_dangerous_vnc_arg("viaduct.example.com"));
        let filtered =
            VncProtocol::filter_custom_args(&["-encoding".to_string(), "listenable".to_string()]);
        assert_eq!(
            filtered,
            vec!["-encoding".to_string(), "listenable".to_string()]
        );
    }

    /// The two builders must agree, for the same input, on exactly what survives.
    #[test]
    fn both_builders_filter_identically() {
        let custom = vec![
            "-Fullscreen".to_string(),
            "--via=attacker".to_string(),
            "-passwd".to_string(),
            "/tmp/pw".to_string(),
        ];
        let config = VncConfig {
            custom_args: custom.clone(),
            ..Default::default()
        };

        let (_program, external) =
            VncProtocol::build_external_viewer_command("vncviewer", "host", 5900, &config);
        let mut connection = Connection::new(
            "vnc".to_string(),
            "host".to_string(),
            5900,
            ProtocolConfig::Vnc(config),
        );
        connection.username = None;
        let fallback = VncProtocol.build_command(&connection).expect("command");

        let survived = |args: &[String]| -> Vec<String> {
            args.iter()
                .filter(|a| custom.iter().any(|c| c == *a))
                .cloned()
                .collect()
        };
        assert_eq!(survived(&external), survived(&fallback));
        assert_eq!(survived(&external), vec!["-Fullscreen".to_string()]);
    }
}
