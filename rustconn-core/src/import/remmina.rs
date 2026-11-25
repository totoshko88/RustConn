//! Remmina configuration importer.
//!
//! Parses .remmina files from ~/.local/share/remmina/
//! Supports importing passwords from GNOME Keyring via secret-tool.
//! Creates proper group hierarchy from Remmina's group field.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use secrecy::SecretString;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::normalize::parse_host_port;
use super::traits::{ImportResult, ImportSource, ImportWarning, SkippedEntry, read_import_file};
use crate::error::ImportError;
use crate::models::{
    Connection, ConnectionGroup, Credentials, PasswordSource, ProtocolConfig, RdpAudioMode,
    RdpConfig, RdpGateway, Resolution, SpiceConfig, SshAuthMethod, SshConfig, SshKeySource,
    TelnetConfig, VncConfig,
};

/// Ceiling on one `secret-tool lookup`.
///
/// Generous on purpose: a locked keyring shows an unlock prompt the user has to
/// answer, and the first lookup waits for it. The point is to bound a prompt
/// left open or a hung keyring daemon, not to race the user; past it the
/// connection is imported without its password.
const SECRET_TOOL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Importer for Remmina connection files.
///
/// Remmina stores each connection in a separate .remmina file
/// in ~/.local/share/remmina/
pub struct RemminaImporter {
    /// Custom paths to search for Remmina files
    custom_paths: Vec<PathBuf>,
    /// Whether to import passwords from GNOME Keyring
    import_passwords: bool,
}

impl RemminaImporter {
    /// Creates a new Remmina importer with default paths
    #[must_use]
    pub const fn new() -> Self {
        Self {
            custom_paths: Vec::new(),
            import_passwords: true,
        }
    }

    /// Creates a new Remmina importer with custom paths
    #[must_use]
    pub const fn with_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            custom_paths: paths,
            import_passwords: true,
        }
    }

    /// Sets whether to import passwords from GNOME Keyring
    #[must_use]
    pub const fn with_password_import(mut self, import: bool) -> Self {
        self.import_passwords = import;
        self
    }

    /// Retrieves a password from GNOME Keyring for a Remmina connection
    ///
    /// Remmina stores passwords using the `org.remmina.Password` schema with
    /// attributes: `filename` (full path to .remmina file) and `key` (field name,
    /// e.g. "password"). We try the full path first, then fall back to just the
    /// filename for compatibility with older Remmina versions.
    fn get_password_from_keyring(file_path: &str) -> Option<SecretString> {
        // Primary lookup: full path + key="password" (matches Remmina's schema)
        if let Some(stdout) = Self::secret_tool_lookup(file_path)
            && let Some(password) = Self::password_from_lookup_output(stdout)
        {
            return Some(password);
        }

        // Fallback: try with just the basename (older Remmina versions)
        let basename = Path::new(file_path)
            .file_name()
            .and_then(|f| f.to_str())
            .unwrap_or(file_path);

        if basename != file_path {
            return Self::secret_tool_lookup(basename).and_then(Self::password_from_lookup_output);
        }

        None
    }

    /// stdout of a successful `secret-tool lookup filename <filename> key
    /// password`, or `None` if it failed, could not start, or ran past
    /// [`SECRET_TOOL_TIMEOUT`].
    ///
    /// The deadline exists because a locked keyring makes `secret-tool` wait on
    /// an unlock prompt, and without one the import waited with it for as long
    /// as the prompt stayed open.
    fn secret_tool_lookup(filename: &str) -> Option<Vec<u8>> {
        let child = Command::new("secret-tool")
            .args(["lookup", "filename", filename, "key", "password"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let output = crate::proc::wait_bounded(child, SECRET_TOOL_TIMEOUT, "secret-tool lookup")
            .ok()?
            .output()?;
        if output.status.success() {
            Some(output.stdout)
        } else {
            // A failed lookup prints no password, but wipe whatever it printed.
            drop(Zeroizing::new(output.stdout));
            None
        }
    }

    /// The password in `secret-tool lookup` stdout, or `None` when it is empty.
    ///
    /// Only the one trailing newline `secret-tool` may append is removed. The
    /// `.trim()` this replaced also cut spaces a password can legitimately
    /// start or end with, so such a password was imported corrupted. Every
    /// intermediate buffer is wiped on drop.
    fn password_from_lookup_output(stdout: Vec<u8>) -> Option<SecretString> {
        let stdout = Zeroizing::new(stdout);
        let text = Zeroizing::new(String::from_utf8_lossy(&stdout).into_owned());
        let password = text.strip_suffix('\n').unwrap_or(&text);
        (!password.is_empty()).then(|| SecretString::from(password))
    }

    /// Parses a single .remmina file content
    #[must_use]
    pub fn parse_remmina_file(
        &self,
        content: &str,
        source_path: &str,
        group_map: &mut HashMap<String, Uuid>,
    ) -> ImportResult {
        let mut result = ImportResult::new();

        // Parse INI-style format
        let config = Self::parse_ini(content);

        // Get the remmina section
        let Some(remmina_section) = config.get("remmina") else {
            result.add_skipped(SkippedEntry::with_location(
                source_path,
                "No [remmina] section found",
                source_path,
            ));
            return result;
        };

        // Extract connection details
        if let Some((mut connection, group_path)) =
            self.convert_to_connection(remmina_section, source_path, &mut result)
        {
            // Assign group if present
            if let Some(ref path) = group_path
                && !path.is_empty()
            {
                let group_id = Self::get_or_create_group(path, group_map, &mut result);
                connection.group_id = Some(group_id);
            }
            Self::link_ssh_tunnel(remmina_section, &mut connection, group_map, &mut result);
            result.add_connection(connection);
        }

        result
    }

    /// Turns Remmina's "Enable SSH tunnel" into a saved SSH connection and links
    /// it to `connection` as its jump host (#449, #450).
    ///
    /// Remmina builds the tunnel from `ssh_tunnel_server` (empty means the
    /// connection's own host, `:port` keeps that host on another port),
    /// `ssh_tunnel_username`, `ssh_tunnel_privatekey` and `ssh_tunnel_auth`.
    /// `ssh_tunnel_loopback` only picks the local listen address, which RustConn
    /// chooses itself, so it is not read. The tunnel password is never read: it
    /// is a secret and the bastion asks for it on connect.
    ///
    /// A bastion with the same host, user and port is created once and reused by
    /// every profile and every file of the same import. `group_map` is the only
    /// state shared between files, so the bastion ids live in it under a key no
    /// group name can have (it starts with a NUL byte).
    ///
    /// Only RDP, VNC and SPICE carry a jump host; any other protocol with the
    /// tunnel on, or a tunnel server that cannot be parsed, gets a warning.
    fn link_ssh_tunnel(
        config: &HashMap<String, String>,
        connection: &mut Connection,
        group_map: &mut HashMap<String, Uuid>,
        result: &mut ImportResult,
    ) {
        let is_on = |key: &str| {
            config.get(key).is_some_and(|v| {
                v == "1" || v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true")
            })
        };
        if !is_on("ssh_tunnel_enabled") {
            return;
        }
        let warn = |result: &mut ImportResult, connection: &Connection| {
            result.add_warning(ImportWarning::SshTunnelNotImported {
                connection_name: connection.name.clone(),
            });
        };
        if !matches!(
            connection.protocol_config,
            ProtocolConfig::Rdp(_) | ProtocolConfig::Vnc(_) | ProtocolConfig::Spice(_)
        ) {
            warn(result, connection);
            return;
        }

        let text = |key: &str| {
            config
                .get(key)
                .map(String::as_str)
                .map(str::trim)
                .filter(|v| !v.is_empty())
        };
        let server = text("ssh_tunnel_server").unwrap_or("");
        let (parsed_host, parsed_port) = parse_host_port(server);
        let host = if parsed_host.is_empty() {
            connection.host.clone()
        } else {
            parsed_host
        };
        let port = parsed_port.unwrap_or(22);
        // `host:abc` is not a port; parse_host_port leaves it inside the host.
        let bad_port = parsed_port.is_none() && server.contains(':') && !server.starts_with('[');
        if host.is_empty() || host.chars().any(char::is_whitespace) || bad_port {
            warn(result, connection);
            return;
        }
        let username = text("ssh_tunnel_username").map(str::to_string);
        let key_path = text("ssh_tunnel_privatekey")
            .map(|p| PathBuf::from(shellexpand::tilde(p).into_owned()));
        // Remmina's identity-file and automatic public-key modes both read a
        // key; the key file, when named, decides. With none named, a password
        // mode stays a password and every other mode falls back to the agent.
        let auth_method = if key_path.is_some() {
            SshAuthMethod::PublicKey
        } else if matches!(text("ssh_tunnel_auth"), None | Some("0" | "password")) {
            SshAuthMethod::Password
        } else {
            SshAuthMethod::Agent
        };

        let key = format!(
            "\0ssh-tunnel:{}@{}:{port}",
            username.as_deref().unwrap_or(""),
            host.to_lowercase()
        );
        let bastion_id = if let Some(&id) = group_map.get(&key) {
            id
        } else {
            let label = username
                .as_ref()
                .map_or_else(|| host.clone(), |u| format!("{u}@{host}"));
            let mut bastion = Connection::new(
                format!("{label} (SSH tunnel)"),
                host,
                port,
                ProtocolConfig::Ssh(SshConfig {
                    auth_method,
                    key_path,
                    ..Default::default()
                }),
            );
            bastion.username = username;
            bastion.group_id = connection.group_id;
            let id = bastion.id;
            group_map.insert(key, id);
            result.add_connection(bastion);
            id
        };

        match &mut connection.protocol_config {
            ProtocolConfig::Rdp(c) => c.jump_host_id = Some(bastion_id),
            ProtocolConfig::Vnc(c) => c.jump_host_id = Some(bastion_id),
            ProtocolConfig::Spice(c) => c.jump_host_id = Some(bastion_id),
            _ => {}
        }
    }

    /// Parses INI-style content into sections
    fn parse_ini(content: &str) -> HashMap<String, HashMap<String, String>> {
        let mut sections: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut current_section: Option<String> = None;

        for line in content.lines() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            // Check for section header
            if line.starts_with('[') && line.ends_with(']') {
                let section_name = line[1..line.len() - 1].to_lowercase();
                current_section = Some(section_name.clone());
                sections.entry(section_name).or_default();
                continue;
            }

            // Parse key=value
            if let Some(eq_pos) = line.find('=') {
                let key = line[..eq_pos].trim().to_lowercase();
                // Strip trailing literal escape sequences (\n, \r, \t)
                // that Remmina INI files sometimes contain at end of values
                let value = super::normalize::sanitize_imported_value(line[eq_pos + 1..].trim());

                if let Some(ref section) = current_section {
                    sections
                        .entry(section.clone())
                        .or_default()
                        .insert(key, value);
                }
            }
        }

        sections
    }

    /// Converts Remmina config section to a Connection
    /// Returns (Connection, Option<group_path>) for group assignment
    #[expect(
        clippy::too_many_lines,
        reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
    )]
    fn convert_to_connection(
        &self,
        config: &HashMap<String, String>,
        source_path: &str,
        result: &mut ImportResult,
    ) -> Option<(Connection, Option<String>)> {
        // Get protocol
        let protocol = config.get("protocol").map(|s| s.to_uppercase());

        // Get server/host
        let server = config.get("server").or_else(|| config.get("ssh_server"));
        let (host, parsed_port) = match server {
            Some(s) if !s.is_empty() => {
                // Use shared utility for host:port parsing
                parse_host_port(s)
            }
            _ => {
                result.add_skipped(SkippedEntry::with_location(
                    source_path,
                    "No server specified",
                    source_path,
                ));
                return None;
            }
        };

        // Get name
        let name = config.get("name").cloned().unwrap_or_else(|| host.clone());

        // Parse port from dedicated field or use parsed port from server string
        let port = config
            .get("ssh_server_port")
            .and_then(|p| p.parse().ok())
            .or(parsed_port);

        // Create protocol-specific config
        let (protocol_config, default_port) = match protocol.as_deref() {
            Some("SSH" | "SFTP") => {
                let auth_method = match config.get("ssh_auth").map(std::string::String::as_str) {
                    Some("2" | "publickey") => SshAuthMethod::PublicKey,
                    Some("3" | "agent") => SshAuthMethod::Agent,
                    _ => SshAuthMethod::Password,
                };

                let key_path = config
                    .get("ssh_privatekey")
                    .filter(|s| !s.is_empty())
                    .map(|p| PathBuf::from(shellexpand::tilde(p).into_owned()));

                // Check for SSH tunnel options
                let x11_forwarding = config
                    .get("ssh_tunnel_x11")
                    .is_some_and(|v| v == "1" || v.to_lowercase() == "yes");

                let compression = config
                    .get("ssh_compression")
                    .is_some_and(|v| v == "1" || v.to_lowercase() == "yes");

                let agent_forwarding = config
                    .get("ssh_tunnel_agent")
                    .is_some_and(|v| v == "1" || v.to_lowercase() == "yes");

                (
                    ProtocolConfig::Ssh(SshConfig {
                        auth_method,
                        key_path,
                        key_source: SshKeySource::Default,
                        agent_key_fingerprint: None,
                        identities_only: false,
                        jump_host_id: None,
                        proxy_jump: None,
                        proxy_command: None,
                        pkcs11_provider: None,
                        use_control_master: false,
                        agent_forwarding,
                        x11_forwarding,
                        compression,
                        custom_options: HashMap::new(),
                        startup_command: None,
                        port_forwards: Vec::new(),
                        waypipe: false,
                        ssh_agent_socket: None,
                        keep_alive_interval: None,
                        keep_alive_count_max: None,
                        verbose: false,
                        mptcp: false,
                        remote_path: None,
                        // Remmina has no equivalent erase-key setting to carry over.
                        ..Default::default()
                    }),
                    22u16,
                )
            }
            Some("RDP") => {
                let resolution = config.get("resolution").and_then(|r| {
                    let parts: Vec<&str> = r.split('x').collect();
                    if parts.len() == 2 {
                        let width = parts[0].parse().ok()?;
                        let height = parts[1].parse().ok()?;
                        Some(Resolution::new(width, height))
                    } else {
                        None
                    }
                });

                let color_depth = config.get("colordepth").and_then(|d| d.parse().ok());

                // Parse RDP gateway from Remmina fields
                let gateway = config
                    .get("gateway_server")
                    .filter(|s| !s.is_empty())
                    .map(|gw| {
                        let (gw_host, gw_port) = parse_host_port(gw);
                        let gw_username = config
                            .get("gateway_username")
                            .filter(|s| !s.is_empty())
                            .cloned();
                        RdpGateway {
                            hostname: gw_host,
                            port: gw_port.unwrap_or(443),
                            username: gw_username,
                        }
                    });

                let mut rdp_config = RdpConfig {
                    resolution,
                    color_depth,
                    gateway,
                    ..Default::default()
                };
                if let Some(sound) = config.get("sound") {
                    rdp_config.set_audio_mode(remmina_audio_mode(sound));
                }
                (ProtocolConfig::Rdp(rdp_config), 3389u16)
            }
            Some("VNC") => (ProtocolConfig::Vnc(VncConfig::default()), 5900u16),
            Some("SPICE") => (ProtocolConfig::Spice(SpiceConfig::default()), 5900u16),
            Some("TELNET") => (ProtocolConfig::Telnet(TelnetConfig::default()), 23u16),
            Some(p) => {
                result.add_skipped(SkippedEntry::with_location(
                    &name,
                    format!("Unsupported protocol: {p}"),
                    source_path,
                ));
                return None;
            }
            None => {
                result.add_skipped(SkippedEntry::with_location(
                    &name,
                    "No protocol specified",
                    source_path,
                ));
                return None;
            }
        };

        let port = port.unwrap_or(default_port);

        let mut connection = Connection::new(name, host, port, protocol_config);

        // Set username
        if let Some(username) = config
            .get("username")
            .or_else(|| config.get("ssh_username"))
            && !username.is_empty()
        {
            connection.username = Some(username.clone());
        }

        // Set domain for RDP connections
        if let Some(domain) = config.get("domain").filter(|s| !s.is_empty()) {
            connection.domain = Some(domain.clone());
        }

        // Try to import password from GNOME Keyring if enabled
        if self.import_passwords
            && let Some(password) = Self::get_password_from_keyring(source_path)
        {
            // Store credentials in the result for later persistence
            let creds = Credentials {
                password: Some(password),
                ..Credentials::with_username(connection.username.clone().unwrap_or_default())
            };
            result.add_credentials(connection.id, creds);
            connection.password_source = PasswordSource::Vault;
        }

        // Return connection and group name for later processing
        Some((connection, config.get("group").cloned()))
    }

    /// Gets or creates a group from the group map, handling nested paths like "Folder/Subfolder"
    ///
    /// # Preconditions
    ///
    /// `group_path` must not be empty. The caller is responsible for checking this.
    ///
    /// # Panics
    ///
    /// Panics in debug builds if `group_path` is empty.
    fn get_or_create_group(
        group_path: &str,
        group_map: &mut HashMap<String, Uuid>,
        result: &mut ImportResult,
    ) -> Uuid {
        debug_assert!(!group_path.is_empty(), "group_path must not be empty");

        // Check if already exists
        if let Some(&id) = group_map.get(group_path) {
            return id;
        }

        // Handle nested paths (e.g., "Production/Web Servers")
        let parts: Vec<&str> = group_path.split('/').collect();
        let mut parent_id: Option<Uuid> = None;
        let mut current_path = String::new();

        for (idx, part) in parts.iter().enumerate() {
            if idx > 0 {
                current_path.push('/');
            }
            current_path.push_str(part);

            if let Some(&existing_id) = group_map.get(&current_path) {
                parent_id = Some(existing_id);
            } else {
                // Create new group
                let group = if let Some(pid) = parent_id {
                    ConnectionGroup::with_parent((*part).to_string(), pid)
                } else {
                    ConnectionGroup::new((*part).to_string())
                };
                let group_id = group.id;
                group_map.insert(current_path.clone(), group_id);
                result.add_group(group);
                parent_id = Some(group_id);
            }
        }

        // INVARIANT: parent_id is always Some after the loop because:
        // 1. group_path is non-empty (precondition)
        // 2. split('/') on non-empty string always yields at least one part
        // 3. Each part either finds an existing group or creates a new one
        // Justifies the `expect` below as a programming-bug invariant (M-PANIC-ON-BUG).
        parent_id.expect("parent_id is always Some for non-empty group_path")
    }
}

impl Default for RemminaImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportSource for RemminaImporter {
    fn source_id(&self) -> &'static str {
        "remmina"
    }

    fn display_name(&self) -> &'static str {
        "Remmina"
    }

    fn is_available(&self) -> bool {
        self.default_paths().iter().any(|p| p.exists())
    }

    fn default_paths(&self) -> Vec<PathBuf> {
        if !self.custom_paths.is_empty() {
            return self.custom_paths.clone();
        }

        let mut paths = Vec::new();
        let mut scanned_dirs: Vec<PathBuf> = Vec::new();

        if let Some(data_dir) = dirs::data_local_dir() {
            let remmina_dir = data_dir.join("remmina");
            scanned_dirs.push(remmina_dir);
        }

        // In Flatpak, dirs::data_local_dir() returns the sandbox path
        // (~/.var/app/<app-id>/data). Also check the host path so users
        // can grant access via `flatpak override --filesystem`.
        if crate::flatpak::is_flatpak()
            && let Some(home) = dirs::home_dir()
        {
            let host_remmina = home.join(".local/share/remmina");
            if !scanned_dirs.contains(&host_remmina) {
                scanned_dirs.push(host_remmina);
            }
        }

        for remmina_dir in &scanned_dirs {
            if remmina_dir.is_dir()
                && let Ok(entries) = fs::read_dir(remmina_dir)
            {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.extension().is_some_and(|ext| ext == "remmina") {
                        paths.push(path);
                    }
                }
            }
        }

        paths
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        let paths = self.default_paths();

        if paths.is_empty() {
            return Err(ImportError::FileNotFound(PathBuf::from(
                "~/.local/share/remmina/",
            )));
        }

        let mut combined_result = ImportResult::new();
        let mut group_map: HashMap<String, Uuid> = HashMap::new();

        for path in paths {
            match self.import_single_file(&path, &mut group_map) {
                Ok(result) => combined_result.merge(result),
                Err(e) => combined_result.add_error(e),
            }
        }

        Ok(combined_result)
    }

    fn import_from_path(&self, path: &Path) -> Result<ImportResult, ImportError> {
        let mut group_map: HashMap<String, Uuid> = HashMap::new();
        self.import_single_file(path, &mut group_map)
    }
}

impl RemminaImporter {
    /// Imports a single .remmina file with shared group map
    fn import_single_file(
        &self,
        path: &Path,
        group_map: &mut HashMap<String, Uuid>,
    ) -> Result<ImportResult, ImportError> {
        let content = read_import_file(path, "Remmina")?;

        Ok(self.parse_remmina_file(&content, &path.display().to_string(), group_map))
    }
}

/// Maps Remmina's RDP `sound` value to an audio mode the way Remmina does.
///
/// Remmina's RDP plugin (`plugins/rdp/rdp_plugin.c`) treats `remote` as audio
/// left on the server, any value starting with `local` as played here, and
/// everything else as off. Reading it as "anything but `off` is local" turned
/// a `remote` profile into local playback.
fn remmina_audio_mode(sound: &str) -> RdpAudioMode {
    if sound == "remote" {
        RdpAudioMode::Remote
    } else if sound.starts_with("local") {
        RdpAudioMode::Local
    } else {
        RdpAudioMode::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    /// `secret-tool` may end its output with a newline; that one character is
    /// not part of the password, but spaces at either end are.
    #[test]
    fn lookup_output_keeps_edge_spaces_and_drops_only_the_newline() {
        let lookup = |out: &str| {
            RemminaImporter::password_from_lookup_output(out.as_bytes().to_vec())
                .map(|p| p.expose_secret().to_owned())
        };

        assert_eq!(lookup("  pass word  \n").as_deref(), Some("  pass word  "));
        assert_eq!(lookup(" no-newline ").as_deref(), Some(" no-newline "));
        // Only one newline is the CLI's; a second belongs to the password.
        assert_eq!(lookup("p\n\n").as_deref(), Some("p\n"));
        assert_eq!(lookup("\n"), None);
        assert_eq!(lookup(""), None);
    }

    /// Every audio mode survives a Remmina export and import, `remote`
    /// included, which the exporter used to write as `off`.
    #[test]
    fn audio_mode_round_trips_through_remmina() {
        use crate::export::remmina::RemminaExporter;

        for mode in RdpAudioMode::all().iter().copied() {
            let mut rdp = RdpConfig::default();
            rdp.set_audio_mode(mode);
            let connection = Connection::new(
                "Audio".to_string(),
                "server.example.com".to_string(),
                3389,
                ProtocolConfig::Rdp(rdp),
            );
            let exported = RemminaExporter::export_connection(&connection).expect("RDP exports");

            let mut group_map = HashMap::new();
            let result = RemminaImporter::new().parse_remmina_file(
                &exported,
                "audio.remmina",
                &mut group_map,
            );
            let ProtocolConfig::Rdp(ref imported) = result.connections[0].protocol_config else {
                panic!("expected an RDP connection");
            };
            assert_eq!(imported.effective_audio_mode(), mode, "{exported}");
        }
    }

    #[test]
    fn remmina_sound_values_map_as_remmina_reads_them() {
        assert_eq!(remmina_audio_mode("remote"), RdpAudioMode::Remote);
        assert_eq!(remmina_audio_mode("local"), RdpAudioMode::Local);
        assert_eq!(
            remmina_audio_mode("local,quality:high"),
            RdpAudioMode::Local
        );
        assert_eq!(remmina_audio_mode("off"), RdpAudioMode::None);
        assert_eq!(remmina_audio_mode(""), RdpAudioMode::None);
    }

    #[test]
    fn test_parse_ssh_connection() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=My SSH Server
protocol=SSH
server=192.168.1.100:22
username=admin
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "My SSH Server");
        assert_eq!(conn.host, "192.168.1.100");
        assert_eq!(conn.port, 22);
        assert_eq!(conn.username, Some("admin".to_string()));
    }

    #[test]
    fn test_parse_rdp_connection() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=Windows Server
protocol=RDP
server=192.168.1.50
username=Administrator
resolution=1920x1080
colordepth=32
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "Windows Server");
        assert!(matches!(conn.protocol_config, ProtocolConfig::Rdp(_)));

        if let ProtocolConfig::Rdp(rdp_config) = &conn.protocol_config {
            assert!(rdp_config.resolution.is_some());
            assert_eq!(rdp_config.color_depth, Some(32));
        }
    }

    #[test]
    fn test_parse_vnc_connection() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=VNC Desktop
protocol=VNC
server=192.168.1.75:5901
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert!(matches!(conn.protocol_config, ProtocolConfig::Vnc(_)));
        assert_eq!(conn.port, 5901);
    }

    #[test]
    fn test_parse_spice_connection() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=SPICE VM
protocol=SPICE
server=192.168.1.100:5900
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert!(matches!(conn.protocol_config, ProtocolConfig::Spice(_)));
        assert_eq!(conn.port, 5900);
    }

    #[test]
    fn test_parse_with_group() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=Production Server
protocol=SSH
server=10.0.0.1
group=Production
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.groups.len(), 1);

        let conn = &result.connections[0];
        assert!(conn.group_id.is_some());
        assert_eq!(result.groups[0].name, "Production");
    }

    #[test]
    fn test_parse_with_nested_group() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=Web Server
protocol=SSH
server=10.0.0.1
group=Production/Web Servers
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);
        assert_eq!(result.groups.len(), 2);

        // Check hierarchy
        let production = result.groups.iter().find(|g| g.name == "Production");
        let web_servers = result.groups.iter().find(|g| g.name == "Web Servers");
        assert!(production.is_some());
        assert!(web_servers.is_some());
        assert!(
            production
                .as_ref()
                .map(|g| g.parent_id.is_none())
                .unwrap_or(false)
        );
        assert_eq!(
            web_servers.as_ref().and_then(|g| g.parent_id),
            production.map(|g| g.id)
        );
    }

    #[test]
    fn test_skip_unsupported_protocol() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=Unknown
protocol=X2GO
server=192.168.1.100
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 0);
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn test_skip_no_server() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        let content = r"
[remmina]
name=No Server
protocol=SSH
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 0);
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn test_parse_name_with_trailing_escape() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();
        // Remmina files can have literal \n at end of name values
        let content = r"
[remmina]
name=ec2-3-250-166-202.eu-west-1.compute.amazonaws.com\n
protocol=RDP
server=ec2-3-250-166-202.eu-west-1.compute.amazonaws.com
username=Administrator
";

        let result = importer.parse_remmina_file(content, "test.remmina", &mut group_map);
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        // parse_ini now strips trailing \n at the source
        assert_eq!(
            conn.name,
            "ec2-3-250-166-202.eu-west-1.compute.amazonaws.com"
        );
    }

    #[test]
    fn test_shared_group_map() {
        let importer = RemminaImporter::new();
        let mut group_map = HashMap::new();

        // First file
        let content1 = r"
[remmina]
name=Server 1
protocol=SSH
server=10.0.0.1
group=Production
";
        let result1 = importer.parse_remmina_file(content1, "test1.remmina", &mut group_map);

        // Second file with same group
        let content2 = r"
[remmina]
name=Server 2
protocol=SSH
server=10.0.0.2
group=Production
";
        let result2 = importer.parse_remmina_file(content2, "test2.remmina", &mut group_map);

        // Group should be reused, not duplicated
        assert_eq!(result1.groups.len(), 1);
        assert_eq!(result2.groups.len(), 0); // No new group created
        assert_eq!(
            result1.connections[0].group_id,
            result2.connections[0].group_id
        );
    }

    fn jump_id(conn: &Connection) -> Option<Uuid> {
        match &conn.protocol_config {
            ProtocolConfig::Rdp(c) => c.jump_host_id,
            ProtocolConfig::Vnc(c) => c.jump_host_id,
            ProtocolConfig::Spice(c) => c.jump_host_id,
            _ => None,
        }
    }

    fn import_one(content: &str) -> ImportResult {
        RemminaImporter::new()
            .with_password_import(false)
            .parse_remmina_file(content, "t.remmina", &mut HashMap::new())
    }

    #[test]
    fn ssh_tunnel_on_vnc_creates_linked_bastion() {
        let result = import_one(
            r"
[remmina]
name=Behind bastion
protocol=VNC
server=127.0.0.1:9336
ssh_tunnel_enabled=1
ssh_tunnel_loopback=1
ssh_tunnel_server=bastion.example.com:2222
ssh_tunnel_username=ops
ssh_tunnel_auth=1
ssh_tunnel_privatekey=/home/u/.ssh/id_ed25519
ssh_tunnel_password=SECRET
",
        );
        assert_eq!(result.connections.len(), 2);
        let vnc = result
            .connections
            .iter()
            .find(|c| matches!(c.protocol_config, ProtocolConfig::Vnc(_)))
            .unwrap();
        let bastion = result
            .connections
            .iter()
            .find(|c| matches!(c.protocol_config, ProtocolConfig::Ssh(_)))
            .unwrap();
        assert_eq!(jump_id(vnc), Some(bastion.id));
        assert_eq!(vnc.host, "127.0.0.1");
        assert_eq!(bastion.host, "bastion.example.com");
        assert_eq!(bastion.port, 2222);
        assert_eq!(bastion.username.as_deref(), Some("ops"));
        let ProtocolConfig::Ssh(ssh) = &bastion.protocol_config else {
            unreachable!()
        };
        assert_eq!(ssh.auth_method, SshAuthMethod::PublicKey);
        assert_eq!(ssh.key_path, Some(PathBuf::from("/home/u/.ssh/id_ed25519")));
        // The tunnel password is never imported.
        assert!(result.credentials.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn empty_tunnel_server_means_the_connection_host_on_port_22() {
        let result = import_one(
            "[remmina]\nname=w\nprotocol=RDP\nserver=win.example.com\nssh_tunnel_enabled=1\nssh_tunnel_username=u\n",
        );
        let bastion = result
            .connections
            .iter()
            .find(|c| matches!(c.protocol_config, ProtocolConfig::Ssh(_)))
            .unwrap();
        assert_eq!(bastion.host, "win.example.com");
        assert_eq!(bastion.port, 22);
        let ProtocolConfig::Ssh(ssh) = &bastion.protocol_config else {
            unreachable!()
        };
        assert_eq!(ssh.auth_method, SshAuthMethod::Password);
    }

    #[test]
    fn same_bastion_is_reused_across_profiles_and_files() {
        let importer = RemminaImporter::new().with_password_import(false);
        let mut group_map = HashMap::new();
        let tunnel =
            "ssh_tunnel_enabled=1\nssh_tunnel_server=b.example.com:22\nssh_tunnel_username=u\n";
        let a = importer.parse_remmina_file(
            &format!("[remmina]\nname=a\nprotocol=RDP\nserver=10.0.0.1\n{tunnel}"),
            "a.remmina",
            &mut group_map,
        );
        let b = importer.parse_remmina_file(
            &format!("[remmina]\nname=b\nprotocol=VNC\nserver=10.0.0.2\n{tunnel}"),
            "b.remmina",
            &mut group_map,
        );
        assert_eq!(a.connections.len(), 2);
        assert_eq!(b.connections.len(), 1);
        let first = jump_id(a.connections.iter().find(|c| c.name == "a").unwrap());
        assert!(first.is_some());
        assert_eq!(first, jump_id(&b.connections[0]));
    }

    #[test]
    fn tunnel_disabled_or_absent_keeps_the_connection_as_it_was() {
        for extra in ["", "ssh_tunnel_enabled=0\nssh_tunnel_server=b:22\n"] {
            let result = import_one(&format!(
                "[remmina]\nname=a\nprotocol=RDP\nserver=10.0.0.1\n{extra}"
            ));
            assert_eq!(result.connections.len(), 1);
            assert_eq!(jump_id(&result.connections[0]), None);
            assert!(result.warnings.is_empty());
        }
    }

    #[test]
    fn unmappable_tunnel_warns_and_imports_without_it() {
        for content in [
            "[remmina]\nname=a\nprotocol=RDP\nserver=h\nssh_tunnel_enabled=1\nssh_tunnel_server=b:notaport\n",
            "[remmina]\nname=a\nprotocol=TELNET\nserver=h\nssh_tunnel_enabled=1\n",
        ] {
            let result = import_one(content);
            assert_eq!(result.connections.len(), 1);
            assert_eq!(jump_id(&result.connections[0]), None);
            assert_eq!(
                result.warnings,
                vec![ImportWarning::SshTunnelNotImported {
                    connection_name: "a".to_string()
                }]
            );
        }
    }
}
