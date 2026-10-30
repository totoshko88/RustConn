//! Microsoft `.rdp` file exporter.
//!
//! Exports RDP connections to the standard `.rdp` file format used by
//! Windows Remote Desktop (mstsc.exe), FreeRDP, and other RDP clients.
//!
//! # Format Reference
//!
//! The `.rdp` format uses `key:type:value` lines where type is:
//! - `s` — string
//! - `i` — integer
//! - `b` — binary (base64, not used here)
//!
//! See: <https://learn.microsoft.com/en-us/windows-server/remote/remote-desktop-services/clients/rdp-files>
//!
//! # Security
//!
//! Passwords are **never** exported. The `.rdp` format supports password hashing
//! (`password 51:b:...`) but it uses reversible encryption with a machine-specific
//! key (DPAPI), making it both insecure and non-portable. The user must enter
//! credentials manually when opening the exported file.

use std::fmt::Write as FmtWrite;
use std::path::Path;

use super::{
    ExportError, ExportFormat, ExportOperationResult, ExportOptions, ExportResult, ExportTarget,
    write_export_file,
};
use crate::models::{Connection, ConnectionGroup, ProtocolConfig, ProtocolType, RdpAudioMode};

/// The `.rdp` performance keys, named so the polarity is visible at the call site.
///
/// A bare tuple of six integers was how this started, and it shipped with two of
/// the three modes wrong: `disable wallpaper` was set for Quality and cleared for
/// Speed's neighbour. Named fields make that class of mistake readable.
struct PerformanceKeys {
    disable_wallpaper: u8,
    allow_font_smoothing: u8,
    allow_desktop_composition: u8,
    disable_full_window_drag: u8,
    disable_menu_anims: u8,
    disable_themes: u8,
    disable_cursor_setting: u8,
}

/// Microsoft `.rdp` file exporter.
///
/// Exports RDP connections to the standard `.rdp` file format.
/// Only RDP connections can be exported; other protocols are skipped.
pub struct RdpFileExporter;

impl RdpFileExporter {
    /// Creates a new `.rdp` file exporter.
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Exports a single RDP connection to `.rdp` file content.
    ///
    /// # Arguments
    ///
    /// * `connection` - The connection to export (must be RDP protocol)
    ///
    /// # Errors
    ///
    /// Returns `ExportError::UnsupportedProtocol` if the connection is not RDP.
    pub fn export_to_rdp_content(connection: &Connection) -> Result<String, ExportError> {
        if connection.protocol != ProtocolType::Rdp {
            return Err(ExportError::UnsupportedProtocol(format!(
                "{:?}",
                connection.protocol
            )));
        }

        let mut output = String::with_capacity(2048);

        // Header comment. The name is sanitized like any other value: it lands on
        // line two, so a line break in it would escape the comment.
        let _ = writeln!(output, "# RustConn RDP Export");
        let _ = writeln!(output, "# Connection: {}", rdp_value(&connection.name));
        let _ = writeln!(output);

        // Required: full address (host:port)
        if connection.port == 3389 {
            let _ = writeln!(output, "full address:s:{}", rdp_value(&connection.host));
        } else {
            let _ = writeln!(
                output,
                "full address:s:{}:{}",
                rdp_value(&connection.host),
                connection.port
            );
        }

        // Username (without domain prefix — domain is separate)
        if let Some(ref username) = connection.username {
            let _ = writeln!(output, "username:s:{}", rdp_value(username));
        }

        // Domain
        if let Some(ref domain) = connection.domain {
            let _ = writeln!(output, "domain:s:{}", rdp_value(domain));
        }

        // RDP-specific settings
        if let ProtocolConfig::Rdp(ref rdp) = connection.protocol_config {
            Self::write_display_settings(&mut output, rdp);
            Self::write_performance_settings(&mut output, rdp);
            Self::write_redirection_settings(&mut output, rdp);
            Self::write_gateway_settings(&mut output, rdp);
            Self::write_security_settings(&mut output, rdp);
            Self::write_remoteapp_settings(&mut output, rdp);
        }

        // Common settings
        let _ = writeln!(output, "autoreconnection enabled:i:1");
        let _ = writeln!(output, "prompt for credentials:i:1");

        Ok(output)
    }

    /// Writes display-related settings.
    fn write_display_settings(output: &mut String, rdp: &crate::models::RdpConfig) {
        use crate::models::RdpDisplayMode;

        // Screen mode: 1 = windowed, 2 = fullscreen
        let screen_mode = match rdp.external_display_mode {
            RdpDisplayMode::Fullscreen | RdpDisplayMode::AllMonitors => 2,
            _ => 1,
        };
        let _ = writeln!(output, "screen mode id:i:{screen_mode}");

        // Resolution (only for custom mode)
        if let Some(ref resolution) = rdp.resolution {
            let _ = writeln!(output, "desktopwidth:i:{}", resolution.width);
            let _ = writeln!(output, "desktopheight:i:{}", resolution.height);
        }

        // Color depth
        let color_depth = rdp.effective_color_depth();
        let _ = writeln!(output, "session bpp:i:{color_depth}");

        // Multi-monitor
        if matches!(rdp.external_display_mode, RdpDisplayMode::AllMonitors) {
            let _ = writeln!(output, "use multimon:i:1");
        }

        // Sizing (issue #341). Smart sizing wins over dynamic resolution in
        // RustConn, as FreeRDP refuses the pair, so the file states the
        // combination that is actually used rather than both switches.
        let _ = writeln!(output, "smart sizing:i:{}", i32::from(rdp.smart_sizing));
        let _ = writeln!(
            output,
            "dynamic resolution:i:{}",
            i32::from(rdp.dynamic_resolution && !rdp.smart_sizing)
        );
    }

    /// Writes performance-related settings.
    ///
    /// The three modes mirror `rdp_client::client::connection::build_performance_flags`,
    /// which is the definition RustConn's own sessions use. Note the polarity
    /// difference: `.rdp` spells four of these as `disable …` and two as
    /// `allow …`, so a mode that enables an effect writes `0` to one key and `1`
    /// to the other. Getting that backwards is invisible in a diff, which is why
    /// `performance_modes_match_the_session_flags` pins all three rows.
    fn write_performance_settings(output: &mut String, rdp: &crate::models::RdpConfig) {
        use crate::models::RdpPerformanceMode;

        let flags = match rdp.performance_mode {
            // Font smoothing and desktop composition on, nothing disabled.
            RdpPerformanceMode::Quality => PerformanceKeys {
                disable_wallpaper: 0,
                allow_font_smoothing: 1,
                allow_desktop_composition: 1,
                disable_full_window_drag: 0,
                disable_menu_anims: 0,
                disable_themes: 0,
                disable_cursor_setting: 0,
            },
            // The session default: drag and menu animations off, font smoothing
            // on, composition off.
            RdpPerformanceMode::Balanced => PerformanceKeys {
                disable_wallpaper: 0,
                allow_font_smoothing: 1,
                allow_desktop_composition: 0,
                disable_full_window_drag: 1,
                disable_menu_anims: 1,
                disable_themes: 0,
                disable_cursor_setting: 0,
            },
            // Every visual effect off.
            RdpPerformanceMode::Speed => PerformanceKeys {
                disable_wallpaper: 1,
                allow_font_smoothing: 0,
                allow_desktop_composition: 0,
                disable_full_window_drag: 1,
                disable_menu_anims: 1,
                disable_themes: 1,
                disable_cursor_setting: 1,
            },
        };

        let _ = writeln!(output, "disable wallpaper:i:{}", flags.disable_wallpaper);
        let _ = writeln!(
            output,
            "allow font smoothing:i:{}",
            flags.allow_font_smoothing
        );
        let _ = writeln!(
            output,
            "allow desktop composition:i:{}",
            flags.allow_desktop_composition
        );
        let _ = writeln!(
            output,
            "disable full window drag:i:{}",
            flags.disable_full_window_drag
        );
        let _ = writeln!(output, "disable menu anims:i:{}", flags.disable_menu_anims);
        let _ = writeln!(output, "disable themes:i:{}", flags.disable_themes);
        let _ = writeln!(
            output,
            "disable cursor setting:i:{}",
            flags.disable_cursor_setting
        );

        // Compression and bitmap caching (always enabled for better performance)
        let _ = writeln!(output, "compression:i:1");
        let _ = writeln!(output, "bitmapcachepersistenable:i:1");
    }

    /// Writes redirection settings (clipboard, audio, drives, etc.).
    fn write_redirection_settings(output: &mut String, rdp: &crate::models::RdpConfig) {
        // Clipboard
        let _ = writeln!(
            output,
            "redirectclipboard:i:{}",
            i32::from(rdp.clipboard_enabled)
        );

        // Audio mode: 0 = local, 1 = remote, 2 = none
        let audio_mode = match rdp.effective_audio_mode() {
            RdpAudioMode::Local => 0,
            RdpAudioMode::Remote => 1,
            RdpAudioMode::None => 2,
        };
        let _ = writeln!(output, "audiomode:i:{audio_mode}");

        // Printer redirection
        let _ = writeln!(
            output,
            "redirectprinters:i:{}",
            i32::from(rdp.printer_enabled)
        );

        // Drive redirection
        if !rdp.shared_folders.is_empty() {
            // drivestoredirect format: "C:\;D:\" or "*" for all or "DynamicDrives" for plug-ins
            let drives: Vec<String> = rdp
                .shared_folders
                .iter()
                .map(|f| rdp_value(&f.local_path.display().to_string()))
                .collect();
            let _ = writeln!(output, "drivestoredirect:s:{}", drives.join(";"));
        }

        // Microphone, smart cards and FIDO2/WebAuthn
        let _ = writeln!(
            output,
            "audiocapturemode:i:{}",
            i32::from(rdp.microphone_enabled)
        );
        let _ = writeln!(
            output,
            "redirectsmartcards:i:{}",
            i32::from(rdp.smartcard_enabled)
        );
        let _ = writeln!(
            output,
            "redirectwebauthn:i:{}",
            i32::from(rdp.fido2_enabled)
        );

        // COM ports and POS devices (default off)
        let _ = writeln!(output, "redirectcomports:i:0");
        let _ = writeln!(output, "redirectposdevices:i:0");
    }

    /// Writes RD Gateway settings.
    fn write_gateway_settings(output: &mut String, rdp: &crate::models::RdpConfig) {
        if let Some(ref gateway) = rdp.gateway {
            // Gateway hostname with port if non-default
            if gateway.port == 443 {
                let _ = writeln!(output, "gatewayhostname:s:{}", rdp_value(&gateway.hostname));
            } else {
                let _ = writeln!(
                    output,
                    "gatewayhostname:s:{}:{}",
                    rdp_value(&gateway.hostname),
                    gateway.port
                );
            }

            // Usage method: 1 = always use gateway
            let _ = writeln!(output, "gatewayusagemethod:i:1");

            // Credentials source: 0 = prompt, 4 = use session credentials
            let _ = writeln!(output, "gatewaycredentialssource:i:0");

            // Gateway username if different from session
            if let Some(ref gw_user) = gateway.username {
                let _ = writeln!(output, "gatewayusername:s:{}", rdp_value(gw_user));
            }
        } else {
            // No gateway
            let _ = writeln!(output, "gatewayusagemethod:i:0");
        }

        // RD Connection Broker routing value, which RD Web Access files carry
        // beside the gateway
        if let Some(info) = rdp.load_balance_info.as_deref().map(str::trim)
            && !info.is_empty()
        {
            let _ = writeln!(output, "loadbalanceinfo:s:{}", rdp_value(info));
        }
    }

    /// Writes security settings.
    fn write_security_settings(output: &mut String, rdp: &crate::models::RdpConfig) {
        use crate::models::RdpSecurityLayer;

        // Authentication level: 0 = connect anyway, 2 = warn, 3 = do not connect
        // Default to 2 (warn) for balance between security and usability
        let _ = writeln!(output, "authentication level:i:2");

        // Enable CredSSP support (NLA)
        let enable_credssp = !rdp.disable_nla;
        let _ = writeln!(
            output,
            "enablecredsspsupport:i:{}",
            i32::from(enable_credssp)
        );

        // Security protocol: negotiation, nla, tls, rdp
        let negotiate = match rdp.security_layer {
            RdpSecurityLayer::Negotiate => 1,
            _ => 0,
        };
        let _ = writeln!(output, "negotiate security layer:i:{negotiate}");
    }

    /// Writes RemoteApp (RAIL) settings.
    fn write_remoteapp_settings(output: &mut String, rdp: &crate::models::RdpConfig) {
        if let Some(ref program) = rdp.remote_app_program {
            let _ = writeln!(output, "remoteapplicationmode:i:1");
            let _ = writeln!(output, "remoteapplicationprogram:s:{}", rdp_value(program));

            if let Some(ref args) = rdp.remote_app_args {
                let _ = writeln!(output, "remoteapplicationcmdline:s:{}", rdp_value(args));
            }

            if let Some(ref name) = rdp.remote_app_name {
                let _ = writeln!(output, "remoteapplicationname:s:{}", rdp_value(name));
            }
        } else {
            let _ = writeln!(output, "remoteapplicationmode:i:0");
        }
    }
}

impl Default for RdpFileExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ExportTarget for RdpFileExporter {
    fn format_id(&self) -> ExportFormat {
        ExportFormat::RdpFile
    }

    fn display_name(&self) -> &'static str {
        "RDP File (.rdp)"
    }

    fn export(
        &self,
        connections: &[Connection],
        _groups: &[ConnectionGroup],
        options: &ExportOptions,
    ) -> ExportOperationResult<ExportResult> {
        let mut result = ExportResult::new();

        // Filter to RDP connections only
        let rdp_connections: Vec<_> = connections
            .iter()
            .filter(|c| c.protocol == ProtocolType::Rdp)
            .collect();

        if rdp_connections.is_empty() {
            result.add_warning("No RDP connections to export");
            return Ok(result);
        }

        // Always a directory with one file per connection, even for a single
        // connection — see `ExportFormat::exports_to_directory` for why the
        // count cannot decide this. `export_rdp_file` is the single-file API.
        std::fs::create_dir_all(&options.output_path)?;

        let mut used_names: std::collections::HashSet<String> = std::collections::HashSet::new();
        for conn in rdp_connections {
            let file_path = options.output_path.join(format!(
                "{}.rdp",
                unique_filename(&conn.name, &mut used_names)
            ));

            match Self::export_to_rdp_content(conn) {
                Ok(content) => {
                    write_export_file(&file_path, &content)?;
                    result.add_output_file(file_path);
                    result.increment_exported();
                }
                Err(e) => {
                    result.add_warning(format!("Failed to export '{}': {}", conn.name, e));
                }
            }
        }

        // Count what did not make it out. Assigned rather than added to, because
        // the per-file loop above already counted its own failures; deriving the
        // total from `exported_count` covers both those and the non-RDP
        // connections in one number, so the two cannot double-count.
        let skipped = connections.len().saturating_sub(result.exported_count);
        if skipped > 0 {
            let non_rdp = connections
                .iter()
                .filter(|c| c.protocol != ProtocolType::Rdp)
                .count();
            result.skipped_count = skipped;
            if non_rdp > 0 {
                result.add_warning(format!(
                    "{non_rdp} non-RDP connection(s) skipped (only RDP can be exported to .rdp format)"
                ));
            }
        }

        Ok(result)
    }

    fn export_connection(&self, connection: &Connection) -> ExportOperationResult<String> {
        Self::export_to_rdp_content(connection)
    }

    fn supports_protocol(&self, protocol: &ProtocolType) -> bool {
        *protocol == ProtocolType::Rdp
    }
}

/// Makes a value safe to interpolate into a `key:type:value` line.
///
/// The `.rdp` format is line-oriented: a value cannot hold a line break, because
/// the next line is parsed as another `key:type:value` pair. An unfiltered value
/// therefore lets whatever produced it append keys of its own — a `username` of
/// `jdoe\nremoteapplicationprogram:s:||cmd` writes a second, honoured key, and
/// `remoteapplicationprogram` launches a program on the RDP server. The same
/// trick reaches `drivestoredirect:s:*` (hands over every local drive),
/// `gatewayhostname` (reroutes the session through someone else's gateway) and
/// `authentication level:i:0` (drops certificate checking silently). The
/// connection *name* matters as much as the rest: it is echoed into the
/// `# Connection:` header, so a newline there escapes the comment on line two of
/// every file this exporter writes.
///
/// A GTK entry will not accept a typed line break, which is why this was not
/// reachable from the UI — but a hand-edited or shared `.rcn`, another import
/// format, or `rustconn-cli add --username $'a\nb'` all carry one.
///
/// Every control character goes, not just `\r` and `\n`: `\x0b` and `\x0c` are
/// line breaks to some parsers, and no control character is representable in this
/// format anyway, so dropping them loses nothing a client could have read. A colon
/// is deliberately left alone — parsers split on the first two, which is exactly
/// how `full address:s:host:3390` expresses a port.
///
/// Stripping rather than refusing, so one odd connection name cannot abort an
/// export that is otherwise fine.
fn rdp_value(value: &str) -> String {
    value.chars().filter(|c| !c.is_control()).collect()
}

/// Sanitizes a connection name for use as a filename.
///
/// Anything outside `[A-Za-z0-9-_.]` becomes `_`. A name that leaves nothing
/// usable — empty, or all separators — falls back to `connection`, because the
/// alternatives are a hidden `.rdp` file (an empty name) or a `...rdp` that reads
/// like a traversal attempt (a name of dots).
fn sanitize_filename(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();

    if sanitized.chars().all(|c| c == '.' || c == '_' || c == '-') {
        return "connection".to_string();
    }
    sanitized
}

/// Returns a sanitized filename stem that has not been used in this export yet.
///
/// Sanitizing is lossy — `web/prod` and `web prod` both become `web_prod` — so
/// without this a second connection would silently overwrite the first and the
/// export would report more files than it wrote. Collisions get a `-2`, `-3` …
/// suffix.
fn unique_filename(name: &str, used: &mut std::collections::HashSet<String>) -> String {
    let base = sanitize_filename(name);
    if used.insert(base.clone()) {
        return base;
    }
    for suffix in 2..=u32::MAX {
        let candidate = format!("{base}-{suffix}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
    }
    base
}

/// Exports a single RDP connection directly to a file.
///
/// Convenience function for quick single-file export.
///
/// # Arguments
///
/// * `connection` - The RDP connection to export
/// * `path` - Output file path
///
/// # Errors
///
/// Returns an error if the connection is not RDP or if writing fails.
pub fn export_rdp_file(connection: &Connection, path: &Path) -> Result<(), ExportError> {
    let content = RdpFileExporter::export_to_rdp_content(connection)?;
    write_export_file(path, &content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{RdpGateway, Resolution};

    fn make_rdp_connection(name: &str, host: &str, port: u16) -> Connection {
        Connection::new_rdp(name.to_string(), host.to_string(), port)
    }

    #[test]
    fn test_basic_rdp_export() {
        let conn = make_rdp_connection("Test Server", "server.example.com", 3389);
        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();

        assert!(content.contains("full address:s:server.example.com"));
        assert!(!content.contains(":3389")); // Default port should be omitted
        assert!(content.contains("# RustConn RDP Export"));
    }

    #[test]
    fn test_rdp_export_with_custom_port() {
        let conn = make_rdp_connection("Custom Port", "server.example.com", 3390);
        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();

        assert!(content.contains("full address:s:server.example.com:3390"));
    }

    #[test]
    fn test_rdp_export_with_username_and_domain() {
        let mut conn = make_rdp_connection("Corp Server", "server.corp.com", 3389);
        conn.username = Some("jdoe".to_string());
        conn.domain = Some("CORP".to_string());

        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();

        assert!(content.contains("username:s:jdoe"));
        assert!(content.contains("domain:s:CORP"));
    }

    #[test]
    fn test_rdp_export_with_resolution() {
        let mut conn = make_rdp_connection("HD Server", "server.example.com", 3389);
        if let ProtocolConfig::Rdp(ref mut rdp) = conn.protocol_config {
            rdp.resolution = Some(Resolution {
                width: 1920,
                height: 1080,
            });
        }

        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();

        assert!(content.contains("desktopwidth:i:1920"));
        assert!(content.contains("desktopheight:i:1080"));
    }

    #[test]
    fn device_and_broker_settings_are_exported() {
        let conn = make_rdp_connection("Default", "server.example.com", 3389);
        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();
        assert!(content.contains("audiocapturemode:i:0"));
        assert!(content.contains("redirectsmartcards:i:0"));
        assert!(content.contains("redirectwebauthn:i:0"));
        assert!(!content.contains("loadbalanceinfo"));

        let mut conn = make_rdp_connection("Farm", "broker.corp", 3389);
        if let ProtocolConfig::Rdp(ref mut rdp) = conn.protocol_config {
            rdp.microphone_enabled = true;
            rdp.smartcard_enabled = true;
            rdp.fido2_enabled = true;
            rdp.load_balance_info = Some("tsv://MS Terminal Services Plugin.1.Sessions".into());
        }
        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();
        assert!(content.contains("audiocapturemode:i:1"));
        assert!(content.contains("redirectsmartcards:i:1"));
        assert!(content.contains("redirectwebauthn:i:1"));
        assert!(content.contains("loadbalanceinfo:s:tsv://MS Terminal Services Plugin.1.Sessions"));
    }

    /// Defaults: dynamic resolution on, smart sizing off.
    #[test]
    fn default_sizing_is_exported() {
        let conn = make_rdp_connection("Default", "server.example.com", 3389);
        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();

        assert!(content.contains("smart sizing:i:0"));
        assert!(content.contains("dynamic resolution:i:1"));
    }

    /// With both switches on smart sizing wins in RustConn, and the file says so
    /// instead of claiming both (issue #341).
    #[test]
    fn smart_sizing_is_exported_as_the_combination_in_use() {
        let mut conn = make_rdp_connection("Legacy", "w2k8r2.corp", 3389);
        if let ProtocolConfig::Rdp(ref mut rdp) = conn.protocol_config {
            rdp.smart_sizing = true;
            rdp.dynamic_resolution = true;
        }

        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();

        assert!(content.contains("smart sizing:i:1"));
        assert!(content.contains("dynamic resolution:i:0"));
    }

    #[test]
    fn test_rdp_export_with_gateway() {
        let mut conn = make_rdp_connection("Internal Server", "internal.corp.com", 3389);
        if let ProtocolConfig::Rdp(ref mut rdp) = conn.protocol_config {
            rdp.gateway = Some(RdpGateway {
                hostname: "gateway.corp.com".to_string(),
                port: 443,
                username: None,
            });
        }

        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();

        assert!(content.contains("gatewayhostname:s:gateway.corp.com"));
        assert!(content.contains("gatewayusagemethod:i:1"));
    }

    #[test]
    fn test_rdp_export_with_gateway_custom_port() {
        let mut conn = make_rdp_connection("Internal Server", "internal.corp.com", 3389);
        if let ProtocolConfig::Rdp(ref mut rdp) = conn.protocol_config {
            rdp.gateway = Some(RdpGateway {
                hostname: "gateway.corp.com".to_string(),
                port: 8443,
                username: Some("gwadmin".to_string()),
            });
        }

        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();

        assert!(content.contains("gatewayhostname:s:gateway.corp.com:8443"));
        assert!(content.contains("gatewayusername:s:gwadmin"));
    }

    #[test]
    fn test_rdp_export_with_remoteapp() {
        let mut conn = make_rdp_connection("RemoteApp", "rdserver.corp.com", 3389);
        if let ProtocolConfig::Rdp(ref mut rdp) = conn.protocol_config {
            rdp.remote_app_program = Some("||notepad".to_string());
            rdp.remote_app_args = Some("/p readme.txt".to_string());
            rdp.remote_app_name = Some("Notepad".to_string());
        }

        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();

        assert!(content.contains("remoteapplicationmode:i:1"));
        assert!(content.contains("remoteapplicationprogram:s:||notepad"));
        assert!(content.contains("remoteapplicationcmdline:s:/p readme.txt"));
        assert!(content.contains("remoteapplicationname:s:Notepad"));
    }

    /// No key that could carry a credential may be written.
    ///
    /// Asserted on the *keys* rather than as a substring search for "password":
    /// the connection name is echoed into the header comment, so a plain
    /// `contains("password")` both passes for the wrong reason (the fixture had no
    /// password to leak) and fails for the wrong reason (a connection legitimately
    /// named "password vault"). The name here is chosen to prove the difference.
    #[test]
    fn no_credential_key_is_ever_written() {
        let mut conn = make_rdp_connection("password vault", "server.example.com", 3389);
        conn.username = Some("admin".to_string());

        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();

        for line in content.lines() {
            let key = line.split(':').next().unwrap_or("").trim().to_lowercase();
            assert_ne!(
                key, "password 51",
                "the DPAPI password field must not appear"
            );
            assert_ne!(key, "password", "no password key may be written");
            assert_ne!(
                key, "clearpassword",
                "no cleartext password key may be written"
            );
        }
        assert!(
            !content.contains("51:b:"),
            "the binary/DPAPI value form must not appear: {content}"
        );
        // The name still round-trips, which is what makes the assertions above
        // meaningful rather than accidentally satisfied.
        assert!(content.contains("# Connection: password vault"));
    }

    /// A value cannot append a second, honoured key.
    ///
    /// `remoteapplicationprogram` launches a program on the RDP server, so a
    /// newline surviving into the file is remote code execution on the target, not
    /// a formatting bug. The GUI will not accept a typed newline; a shared `.rcn`
    /// or `rustconn-cli add --username $'a\nb'` will.
    #[test]
    fn a_newline_in_a_value_cannot_append_a_key() {
        let mut conn = make_rdp_connection("Injected", "server.example.com", 3389);
        conn.username = Some("jdoe\nremoteapplicationprogram:s:||cmd".to_string());
        conn.domain = Some("CORP\r\ndrivestoredirect:s:*".to_string());

        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();

        // Line-based, because that is what a parser reads: the injected text
        // surviving *inside* the username value is harmless, since only a value
        // at the start of a line is a key. Nothing may start a line with it.
        for line in content.lines() {
            assert!(
                !line.starts_with("remoteapplicationprogram:s:||cmd"),
                "a smuggled RemoteApp key became its own line:\n{content}"
            );
            assert!(
                !line.starts_with("drivestoredirect:s:*"),
                "a smuggled drive redirection became its own line:\n{content}"
            );
        }
        // One line per key that the connection actually configured.
        assert_eq!(
            content
                .lines()
                .filter(|l| l.starts_with("username:"))
                .count(),
            1
        );
        assert_eq!(
            content.lines().filter(|l| l.starts_with("domain:")).count(),
            1
        );
        // The surviving value stays on one line, with the control characters gone.
        assert!(content.contains("username:s:jdoeremoteapplicationprogram:s:||cmd"));
        assert!(content.contains("domain:s:CORPdrivestoredirect:s:*"));
        // RemoteApp is off, which is what the connection actually configured.
        assert!(content.contains("remoteapplicationmode:i:0"));
    }

    /// The connection name lands on line two, inside a comment. A newline there
    /// would escape the comment and everything after it would be parsed as keys.
    #[test]
    fn a_newline_in_the_name_cannot_escape_the_header_comment() {
        let conn =
            make_rdp_connection("prod\nauthentication level:i:0", "server.example.com", 3389);

        let content = RdpFileExporter::export_to_rdp_content(&conn).unwrap();

        assert!(
            !content.contains("\nauthentication level:i:0"),
            "the name escaped its comment:\n{content}"
        );
        // The exporter's own security setting is the only one present.
        assert_eq!(
            content
                .lines()
                .filter(|l| l.starts_with("authentication level:"))
                .count(),
            1
        );
        assert!(content.contains("authentication level:i:2"));
    }

    /// Every control character goes, not only `\r` and `\n`.
    #[test]
    fn other_control_characters_are_dropped_too() {
        assert_eq!(rdp_value("a\u{0b}b\u{0c}c\td"), "abcd");
        assert_eq!(rdp_value("plain value"), "plain value");
        // A colon is left alone: it is how a port is expressed.
        assert_eq!(rdp_value("host:3390"), "host:3390");
    }

    /// The claim the format's directory handling rests on: one connection is still
    /// a directory with one file in it, because the chooser is picked before the
    /// selection is known.
    #[test]
    fn a_single_connection_still_exports_as_a_directory() {
        let dir = tempfile::tempdir().unwrap();
        let connections = vec![make_rdp_connection("only-one", "a.example.com", 3389)];
        let out = dir.path().join("out");
        let options = ExportOptions::new(ExportFormat::RdpFile, out.clone());

        let result = RdpFileExporter::new()
            .export(&connections, &[], &options)
            .unwrap();

        assert_eq!(result.exported_count, 1);
        assert!(
            out.is_dir(),
            "a single connection must still yield a directory"
        );
        assert!(out.join("only-one.rdp").is_file());
    }

    #[test]
    fn test_non_rdp_connection_rejected() {
        let conn = Connection::new_ssh("SSH Server".to_string(), "ssh.example.com".to_string(), 22);
        let result = RdpFileExporter::export_to_rdp_content(&conn);

        assert!(result.is_err());
        assert!(matches!(result, Err(ExportError::UnsupportedProtocol(_))));
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("Simple Name"), "Simple_Name");
        assert_eq!(sanitize_filename("Server/Corp"), "Server_Corp");
        assert_eq!(sanitize_filename("192.168.1.1"), "192.168.1.1");
        assert_eq!(sanitize_filename("test<>:file"), "test___file");
    }

    #[test]
    fn test_export_to_file() {
        let conn = make_rdp_connection("Export Test", "server.example.com", 3389);

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.rdp");

        export_rdp_file(&conn, &path).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("full address:s:server.example.com"));
    }

    #[test]
    fn test_exporter_supports_protocol() {
        let exporter = RdpFileExporter::new();
        assert!(exporter.supports_protocol(&ProtocolType::Rdp));
        assert!(!exporter.supports_protocol(&ProtocolType::Ssh));
        assert!(!exporter.supports_protocol(&ProtocolType::Vnc));
    }

    fn export_with_performance(mode: crate::models::RdpPerformanceMode) -> String {
        let mut conn = make_rdp_connection("Perf", "server.example.com", 3389);
        if let ProtocolConfig::Rdp(ref mut rdp) = conn.protocol_config {
            rdp.performance_mode = mode;
        }
        RdpFileExporter::export_to_rdp_content(&conn).unwrap()
    }

    /// Pins every row of the performance table against the flags a RustConn
    /// session itself uses (`build_performance_flags`). Two of the three rows
    /// disagreed with it on arrival, and `disable …`/`allow …` polarity makes that
    /// unreadable in a diff.
    #[test]
    fn performance_modes_match_the_session_flags() {
        use crate::models::RdpPerformanceMode;

        let quality = export_with_performance(RdpPerformanceMode::Quality);
        for expected in [
            "disable wallpaper:i:0",
            "allow font smoothing:i:1",
            "allow desktop composition:i:1",
            "disable full window drag:i:0",
            "disable menu anims:i:0",
            "disable themes:i:0",
            "disable cursor setting:i:0",
        ] {
            assert!(
                quality.contains(expected),
                "Quality must emit {expected}\n{quality}"
            );
        }

        let balanced = export_with_performance(RdpPerformanceMode::Balanced);
        for expected in [
            "disable wallpaper:i:0",
            "allow font smoothing:i:1",
            "allow desktop composition:i:0",
            "disable full window drag:i:1",
            "disable menu anims:i:1",
            "disable themes:i:0",
        ] {
            assert!(
                balanced.contains(expected),
                "Balanced must emit {expected}\n{balanced}"
            );
        }

        let speed = export_with_performance(RdpPerformanceMode::Speed);
        for expected in [
            "disable wallpaper:i:1",
            "allow font smoothing:i:0",
            "allow desktop composition:i:0",
            "disable full window drag:i:1",
            "disable menu anims:i:1",
            "disable themes:i:1",
            "disable cursor setting:i:1",
        ] {
            assert!(
                speed.contains(expected),
                "Speed must emit {expected}\n{speed}"
            );
        }
    }

    #[test]
    fn a_name_with_nothing_usable_does_not_become_a_hidden_file() {
        assert_eq!(sanitize_filename(""), "connection");
        assert_eq!(sanitize_filename(".."), "connection");
        assert_eq!(sanitize_filename("///"), "connection");
    }

    /// A mixed selection reports the non-RDP entries once, and the two counters
    /// add up to what was handed in.
    #[test]
    fn a_mixed_selection_counts_skipped_once() {
        let dir = tempfile::tempdir().unwrap();
        let connections = vec![
            make_rdp_connection("win-a", "a.example.com", 3389),
            make_rdp_connection("win-b", "b.example.com", 3389),
            Connection::new_ssh("shell".to_string(), "c.example.com".to_string(), 22),
        ];
        let options = ExportOptions::new(ExportFormat::RdpFile, dir.path().join("out"));

        let result = RdpFileExporter::new()
            .export(&connections, &[], &options)
            .unwrap();

        assert_eq!(result.exported_count, 2);
        assert_eq!(result.skipped_count, 1);
        assert_eq!(result.exported_count + result.skipped_count, 3);
        assert_eq!(
            result
                .warnings
                .iter()
                .filter(|w| w.contains("non-RDP"))
                .count(),
            1
        );
        assert!(dir.path().join("out").join("win-a.rdp").exists());
        assert!(dir.path().join("out").join("win-b.rdp").exists());
    }

    /// Sanitizing is lossy, so two different names can collide. Both must survive.
    #[test]
    fn colliding_names_do_not_overwrite_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let connections = vec![
            make_rdp_connection("web/prod", "a.example.com", 3389),
            make_rdp_connection("web prod", "b.example.com", 3389),
        ];
        let options = ExportOptions::new(ExportFormat::RdpFile, dir.path().join("out"));

        let result = RdpFileExporter::new()
            .export(&connections, &[], &options)
            .unwrap();

        assert_eq!(result.exported_count, 2);
        assert_eq!(result.output_files.len(), 2);
        let out = dir.path().join("out");
        assert!(out.join("web_prod.rdp").exists());
        assert!(out.join("web_prod-2.rdp").exists());
        let first = std::fs::read_to_string(out.join("web_prod.rdp")).unwrap();
        let second = std::fs::read_to_string(out.join("web_prod-2.rdp")).unwrap();
        assert!(first.contains("a.example.com"));
        assert!(second.contains("b.example.com"));
    }

    /// A selection with no RDP connection is a warning, not an error, and creates
    /// nothing.
    #[test]
    fn a_selection_without_rdp_exports_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let connections = vec![Connection::new_ssh(
            "shell".to_string(),
            "c.example.com".to_string(),
            22,
        )];
        let options = ExportOptions::new(ExportFormat::RdpFile, dir.path().join("out"));

        let result = RdpFileExporter::new()
            .export(&connections, &[], &options)
            .unwrap();

        assert_eq!(result.exported_count, 0);
        assert!(!dir.path().join("out").exists());
    }
}
