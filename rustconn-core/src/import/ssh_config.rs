//! SSH config file importer.
//!
//! Parses ~/.ssh/config and ~/.ssh/config.d/* files to import SSH connections.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use tracing::{debug, info_span};

use super::traits::{ImportResult, ImportSource, SkippedEntry, read_import_file};
use crate::error::ImportError;
use crate::models::{Connection, ProtocolConfig, SshAuthMethod, SshConfig, SshKeySource};
use crate::tracing::span_names;

/// Maximum `Include` recursion depth. Matches OpenSSH's own limit and guards
/// against include cycles (a glob that matches its own file).
const MAX_INCLUDE_DEPTH: u8 = 16;

/// Importer for SSH config files.
///
/// Parses standard OpenSSH configuration files and extracts connection
/// information including Host, `HostName`, Port, User, `IdentityFile`, and `ProxyJump`.
pub struct SshConfigImporter {
    /// Custom paths to search for SSH config files
    custom_paths: Vec<PathBuf>,
}

impl SshConfigImporter {
    /// Creates a new SSH config importer with default paths
    #[must_use]
    pub const fn new() -> Self {
        Self {
            custom_paths: Vec::new(),
        }
    }

    /// Creates a new SSH config importer with custom paths
    #[must_use]
    pub const fn with_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            custom_paths: paths,
        }
    }

    /// Parses SSH config content and returns an import result.
    ///
    /// `Include` directives are expanded (glob patterns supported; relative
    /// paths resolve against `~/.ssh`, matching OpenSSH), and each physical
    /// file is parsed at most once.
    #[must_use]
    pub fn parse_config(&self, content: &str, source_path: &str) -> ImportResult {
        let mut result = ImportResult::new();
        let mut visited = HashSet::new();
        let base_dir = Self::user_ssh_dir();
        self.parse_into(
            content,
            source_path,
            &base_dir,
            0,
            &mut visited,
            &mut result,
        );
        result
    }

    /// The directory relative `Include` paths resolve against (`~/.ssh`).
    fn user_ssh_dir() -> PathBuf {
        dirs::home_dir().map_or_else(|| PathBuf::from("."), |home| home.join(".ssh"))
    }

    /// Parses config `content` into `result`, recursing into `Include` files.
    ///
    /// `visited` holds canonical paths already parsed so that a file reachable
    /// via several `Include`s (or also enumerated in `config.d`) is parsed once.
    fn parse_into(
        &self,
        content: &str,
        source_path: &str,
        base_dir: &Path,
        depth: u8,
        visited: &mut HashSet<PathBuf>,
        result: &mut ImportResult,
    ) {
        let mut current_host: Option<String> = None;
        let mut current_options: HashMap<String, String> = HashMap::new();
        // Global defaults from `Host *` entries
        let mut global_defaults: HashMap<String, String> = HashMap::new();

        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') {
                continue;
            }

            // Parse key-value pair
            let Some((key, value)) = Self::parse_line(line) else {
                result.add_skipped(SkippedEntry::with_location(
                    format!("line {}", line_num + 1),
                    "Invalid syntax",
                    source_path,
                ));
                continue;
            };

            let key_lower = key.to_lowercase();

            // `Include` is processed at the point it appears, regardless of
            // whether it sits at top level or inside a Host block. Note: an
            // Include nested inside a Host block does not inherit that host's
            // accumulated options — included Host blocks are imported on their
            // own, which is the dominant real-world usage.
            if key_lower == "include" {
                self.process_include(value, base_dir, depth, source_path, visited, result);
                continue;
            }

            if key_lower == "host" {
                // Save previous host if any
                if let Some(host_pattern) = current_host.take() {
                    if host_pattern == "*" {
                        // Collect global defaults (later Host * blocks merge in)
                        for (k, v) in &current_options {
                            if !global_defaults.contains_key(k) {
                                global_defaults.insert(k.clone(), v.clone());
                            }
                        }
                    } else {
                        let merged = Self::merge_with_defaults(&current_options, &global_defaults);
                        self.process_host_entry(&host_pattern, &merged, source_path, result);
                    }
                }

                // Start new host entry
                current_host = Some(value.to_string());
                current_options.clear();
            } else if current_host.is_some() {
                // Add option to current host
                current_options.insert(key_lower, value.to_string());
            }
        }

        // Process last host entry
        if let Some(host_pattern) = current_host {
            if host_pattern == "*" {
                for (k, v) in &current_options {
                    if !global_defaults.contains_key(k) {
                        global_defaults.insert(k.clone(), v.clone());
                    }
                }
            } else {
                let merged = Self::merge_with_defaults(&current_options, &global_defaults);
                self.process_host_entry(&host_pattern, &merged, source_path, result);
            }
        }
    }

    /// Expands an `Include` pattern and parses each matched file once.
    fn process_include(
        &self,
        pattern: &str,
        base_dir: &Path,
        depth: u8,
        source_path: &str,
        visited: &mut HashSet<PathBuf>,
        result: &mut ImportResult,
    ) {
        if depth >= MAX_INCLUDE_DEPTH {
            result.add_skipped(SkippedEntry::with_location(
                format!("Include {pattern}"),
                "Maximum Include depth exceeded",
                source_path,
            ));
            return;
        }

        for path in Self::expand_include(pattern, base_dir) {
            // Canonicalize so the same file reached via different patterns is
            // parsed once; fall back to the raw path if it cannot be resolved.
            let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
            if !visited.insert(canonical) {
                continue;
            }
            match read_import_file(&path, "SSH config") {
                Ok(content) => self.parse_into(
                    &content,
                    &path.display().to_string(),
                    base_dir,
                    depth + 1,
                    visited,
                    result,
                ),
                Err(e) => result.add_error(e),
            }
        }
    }

    /// Resolves an `Include` pattern (possibly several whitespace-separated
    /// globs) to a sorted list of existing files.
    fn expand_include(pattern: &str, base_dir: &Path) -> Vec<PathBuf> {
        let mut out = Vec::new();
        for token in pattern.split_whitespace() {
            let expanded = shellexpand::tilde(token).into_owned();
            let full = if Path::new(&expanded).is_absolute() {
                expanded
            } else {
                base_dir.join(&expanded).to_string_lossy().into_owned()
            };
            if let Ok(paths) = glob::glob(&full) {
                let mut matched: Vec<PathBuf> = paths.flatten().filter(|p| p.is_file()).collect();
                matched.sort();
                out.extend(matched);
            }
        }
        out
    }

    /// Merges host-specific options with global `Host *` defaults.
    /// Host-specific values take priority over defaults.
    fn merge_with_defaults(
        host_options: &HashMap<String, String>,
        defaults: &HashMap<String, String>,
    ) -> HashMap<String, String> {
        if defaults.is_empty() {
            return host_options.clone();
        }
        let mut merged = defaults.clone();
        for (k, v) in host_options {
            merged.insert(k.clone(), v.clone());
        }
        merged
    }

    /// Parses a single line into key-value pair
    fn parse_line(line: &str) -> Option<(&str, &str)> {
        // SSH config supports both "Key Value" and "Key=Value" formats
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim();
            let value = line[eq_pos + 1..].trim();
            if !key.is_empty() && !value.is_empty() {
                return Some((key, value));
            }
        }

        // Space-separated format
        let parts: Vec<&str> = line.splitn(2, char::is_whitespace).collect();
        if parts.len() == 2 {
            let key = parts[0].trim();
            let value = parts[1].trim();
            if !key.is_empty() && !value.is_empty() {
                return Some((key, value));
            }
        }

        None
    }

    /// Processes a host entry and adds it to the result
    fn process_host_entry(
        &self,
        host_pattern: &str,
        options: &HashMap<String, String>,
        source_path: &str,
        result: &mut ImportResult,
    ) {
        // Skip wildcard patterns and special entries
        if host_pattern.contains('*') || host_pattern.contains('?') || host_pattern == "*" {
            result.add_skipped(SkippedEntry::with_location(
                host_pattern,
                "Wildcard patterns are not imported",
                source_path,
            ));
            return;
        }

        // Get the actual hostname (HostName option or use the Host pattern)
        let hostname = options.get("hostname").map_or(host_pattern, String::as_str);

        // Skip if hostname is empty or a pattern
        if hostname.is_empty() || hostname.contains('*') {
            result.add_skipped(SkippedEntry::with_location(
                host_pattern,
                "No valid hostname",
                source_path,
            ));
            return;
        }

        // Parse port
        let port = options
            .get("port")
            .and_then(|p| p.parse::<u16>().ok())
            .unwrap_or(22);

        // Determine auth method and key path
        let (auth_method, key_path) =
            options
                .get("identityfile")
                .map_or((SshAuthMethod::Password, None), |identity_file| {
                    let path = PathBuf::from(shellexpand::tilde(identity_file).into_owned());
                    (SshAuthMethod::PublicKey, Some(path))
                });

        // Check for IdentitiesOnly option
        let identities_only = options
            .get("identitiesonly")
            .is_some_and(|v| v.to_lowercase() == "yes");

        // Check for ForwardAgent option
        let agent_forwarding = options
            .get("forwardagent")
            .is_some_and(|v| v.to_lowercase() == "yes");

        // Check for X11 forwarding option
        let x11_forwarding = options
            .get("forwardx11")
            .is_some_and(|v| v.to_lowercase() == "yes");

        // Check for compression option (also stored in custom_options for CLI args)
        let compression = options
            .get("compression")
            .is_some_and(|v| v.to_lowercase() == "yes");

        // Build SSH config
        let ssh_config = SshConfig {
            auth_method,
            key_path,
            key_source: SshKeySource::Default,
            agent_key_fingerprint: None,
            identities_only,
            jump_host_id: None,
            proxy_jump: options.get("proxyjump").cloned(),
            proxy_command: options.get("proxycommand").cloned(),
            pkcs11_provider: options.get("pkcs11provider").cloned(),
            use_control_master: options
                .get("controlmaster")
                .is_some_and(|v| v.to_lowercase() == "auto" || v.to_lowercase() == "yes"),
            agent_forwarding,
            x11_forwarding,
            compression,
            custom_options: self.extract_recognized_options(options),
            startup_command: None,
            sftp_enabled: false,
            port_forwards: Vec::new(),
            waypipe: false,
            ssh_agent_socket: None,
            keep_alive_interval: options
                .get("serveraliveinterval")
                .and_then(|v| v.parse::<u32>().ok()),
            keep_alive_count_max: options
                .get("serveralivecountmax")
                .and_then(|v| v.parse::<u32>().ok()),
            verbose: false,
            mptcp: options
                .get("tcpmultipath")
                .is_some_and(|v| v.to_lowercase() == "yes"),
            remote_path: None,
        };

        // Create connection
        let mut connection = Connection::new(
            host_pattern.to_string(),
            hostname.to_string(),
            port,
            ProtocolConfig::Ssh(ssh_config),
        );

        // Set username if specified
        if let Some(user) = options.get("user") {
            connection.username = Some(user.clone());
        }

        result.add_connection(connection);
    }
    /// Extracts recognized SSH options as custom_options for the connection
    #[expect(
        clippy::unused_self,
        reason = "method is part of a uniform helper API where most operations need &self; keeping &self preserves the consistent signature"
    )]
    fn extract_recognized_options(
        &self,
        options: &HashMap<String, String>,
    ) -> HashMap<String, String> {
        // Keys that are mapped to dedicated SshConfig fields and should NOT
        // be duplicated in custom_options:
        //   - serveraliveinterval → keep_alive_interval
        //   - serveralivecountmax → keep_alive_count_max
        //   - compression → compression (bool field)
        let dedicated_field_keys = ["serveraliveinterval", "serveralivecountmax", "compression"];

        let recognized_keys = [
            "serveraliveinterval",
            "serveralivecountmax",
            "compression",
            "tcpkeepalive",
            "stricthostkeychecking",
            "userknownhostsfile",
            "loglevel",
            "connecttimeout",
            "connectionattempts",
        ];

        options
            .iter()
            .filter(|(k, _)| recognized_keys.contains(&k.as_str()))
            .filter(|(k, _)| !dedicated_field_keys.contains(&k.as_str()))
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    }

    /// Reads and parses a single SSH config file
    fn import_file(&self, path: &Path) -> Result<ImportResult, ImportError> {
        let mut result = ImportResult::new();
        let mut visited = HashSet::new();
        self.import_file_into(path, &mut visited, &mut result)?;
        Ok(result)
    }

    /// Reads `path` and parses it into `result`, skipping files already in
    /// `visited` and recording the file as visited.
    fn import_file_into(
        &self,
        path: &Path,
        visited: &mut HashSet<PathBuf>,
        result: &mut ImportResult,
    ) -> Result<(), ImportError> {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        if !visited.insert(canonical) {
            return Ok(());
        }
        let content = read_import_file(path, "SSH config")?;
        let base_dir = Self::user_ssh_dir();
        self.parse_into(
            &content,
            &path.display().to_string(),
            &base_dir,
            0,
            visited,
            result,
        );
        Ok(())
    }
}

impl Default for SshConfigImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportSource for SshConfigImporter {
    fn source_id(&self) -> &'static str {
        "ssh_config"
    }

    fn display_name(&self) -> &'static str {
        "SSH Config"
    }

    fn is_available(&self) -> bool {
        self.default_paths().iter().any(|p| p.exists())
    }

    fn default_paths(&self) -> Vec<PathBuf> {
        if !self.custom_paths.is_empty() {
            return self.custom_paths.clone();
        }

        let mut paths = Vec::new();

        if let Some(home) = dirs::home_dir() {
            let ssh_dir = home.join(".ssh");
            let config_path = ssh_dir.join("config");
            if config_path.exists() {
                paths.push(config_path);
            }

            let config_d = ssh_dir.join("config.d");
            if config_d.is_dir()
                && let Ok(entries) = fs::read_dir(&config_d)
            {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_file() {
                        paths.push(path);
                    }
                }
            }
        }

        paths
    }

    fn import(&self) -> Result<ImportResult, ImportError> {
        let _span = info_span!(span_names::IMPORT_EXECUTE, format = "ssh_config").entered();

        let paths = self.default_paths();

        if paths.is_empty() {
            return Err(ImportError::FileNotFound(PathBuf::from("~/.ssh/config")));
        }

        debug!(path_count = paths.len(), "Importing from SSH config files");

        let mut combined_result = ImportResult::new();
        // Shared across all default paths so a file pulled in by `Include` is
        // not also imported a second time when enumerated directly (e.g.
        // `config.d/*` that the main config also includes).
        let mut visited = HashSet::new();

        for path in paths {
            if let Err(e) = self.import_file_into(&path, &mut visited, &mut combined_result) {
                combined_result.add_error(e);
            }
        }

        debug!(
            imported = combined_result.connections.len(),
            skipped = combined_result.skipped.len(),
            "SSH config import completed"
        );

        Ok(combined_result)
    }

    fn import_from_path(&self, path: &Path) -> Result<ImportResult, ImportError> {
        let _span = info_span!(
            span_names::IMPORT_EXECUTE,
            format = "ssh_config",
            path = %path.display()
        )
        .entered();

        if !path.exists() {
            return Err(ImportError::FileNotFound(path.to_path_buf()));
        }

        self.import_file(path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_simple_host() {
        let importer = SshConfigImporter::new();
        let config = r"
Host myserver
    HostName 192.168.1.100
    User admin
    Port 2222
";

        let result = importer.parse_config(config, "test");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        assert_eq!(conn.name, "myserver");
        assert_eq!(conn.host, "192.168.1.100");
        assert_eq!(conn.port, 2222);
        assert_eq!(conn.username, Some("admin".to_string()));
    }

    #[test]
    fn test_parse_with_identity_file() {
        let importer = SshConfigImporter::new();
        let config = r"
Host secure-server
    HostName secure.example.com
    User deploy
    IdentityFile ~/.ssh/id_ed25519
";

        let result = importer.parse_config(config, "test");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        if let ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config {
            assert_eq!(ssh_config.auth_method, SshAuthMethod::PublicKey);
            assert!(ssh_config.key_path.is_some());
        } else {
            panic!("Expected SSH config");
        }
    }

    #[test]
    fn test_skip_wildcard_hosts() {
        let importer = SshConfigImporter::new();
        let config = r"
Host *
    ServerAliveInterval 60

Host *.example.com
    User admin
";

        let result = importer.parse_config(config, "test");
        // Host * is consumed as defaults (not skipped), *.example.com is skipped
        assert_eq!(result.connections.len(), 0);
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn test_host_star_defaults_applied() {
        let importer = SshConfigImporter::new();
        let config = r"
Host *
    ServerAliveInterval 60
    ServerAliveCountMax 5
    Compression yes

Host myserver
    HostName 192.168.1.100
    User admin
    ServerAliveCountMax 10
";

        let result = importer.parse_config(config, "test");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        if let ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config {
            // Inherited from Host *
            assert_eq!(ssh_config.keep_alive_interval, Some(60));
            assert!(ssh_config.compression);
            // Overridden by host-specific value
            assert_eq!(ssh_config.keep_alive_count_max, Some(10));
        } else {
            panic!("Expected SSH config");
        }
    }

    #[test]
    fn test_parse_proxy_jump() {
        let importer = SshConfigImporter::new();
        let config = r"
Host internal
    HostName 10.0.0.5
    ProxyJump bastion.example.com
";

        let result = importer.parse_config(config, "test");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        if let ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config {
            assert_eq!(
                ssh_config.proxy_jump,
                Some("bastion.example.com".to_string())
            );
        } else {
            panic!("Expected SSH config");
        }
    }

    #[test]
    fn test_parse_multiple_hosts() {
        let importer = SshConfigImporter::new();
        let config = r"
Host server1
    HostName 192.168.1.1
    User user1

Host server2
    HostName 192.168.1.2
    User user2
    Port 22022
";

        let result = importer.parse_config(config, "test");
        assert_eq!(result.connections.len(), 2);
    }

    #[test]
    fn test_parse_forward_agent() {
        let importer = SshConfigImporter::new();
        let config = r"
Host bastion
    HostName bastion.example.com
    ForwardAgent yes
";

        let result = importer.parse_config(config, "test");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        if let ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config {
            assert!(ssh_config.agent_forwarding);
        } else {
            panic!("Expected SSH config");
        }
    }

    #[test]
    fn test_include_directive_expands_files() {
        // Main config Includes another file via an absolute path; both the
        // included host and the inline host must be imported.
        let dir = std::env::temp_dir().join(format!("rc-ssh-inc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).expect("create temp dir");
        let included = dir.join("extra_hosts");
        std::fs::write(
            &included,
            "Host included-host\n    HostName 10.9.8.7\n    User inc\n",
        )
        .expect("write included file");

        let main = format!(
            "Include {}\n\nHost main-host\n    HostName 1.2.3.4\n",
            included.display()
        );
        let importer = SshConfigImporter::new();
        let result = importer.parse_config(&main, "test");

        let names: Vec<&str> = result.connections.iter().map(|c| c.name.as_str()).collect();
        assert!(
            names.contains(&"included-host"),
            "included host missing: {names:?}"
        );
        assert!(names.contains(&"main-host"), "main host missing: {names:?}");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_parse_keepalive_options() {
        let importer = SshConfigImporter::new();
        let config = r"
Host keepalive-server
    HostName server.example.com
    ServerAliveInterval 60
    ServerAliveCountMax 3
    TCPKeepAlive yes
    Compression yes
    ConnectTimeout 30
";

        let result = importer.parse_config(config, "test");
        assert_eq!(result.connections.len(), 1);

        let conn = &result.connections[0];
        if let ProtocolConfig::Ssh(ssh_config) = &conn.protocol_config {
            // keep-alive and compression are mapped to dedicated fields
            assert_eq!(ssh_config.keep_alive_interval, Some(60));
            assert_eq!(ssh_config.keep_alive_count_max, Some(3));
            assert!(ssh_config.compression);

            // These should NOT be in custom_options (mapped to dedicated fields)
            assert!(
                !ssh_config
                    .custom_options
                    .contains_key("serveraliveinterval")
            );
            assert!(
                !ssh_config
                    .custom_options
                    .contains_key("serveralivecountmax")
            );
            assert!(!ssh_config.custom_options.contains_key("compression"));

            // These should still be in custom_options
            assert_eq!(
                ssh_config.custom_options.get("tcpkeepalive"),
                Some(&"yes".to_string())
            );
            assert_eq!(
                ssh_config.custom_options.get("connecttimeout"),
                Some(&"30".to_string())
            );
        } else {
            panic!("Expected SSH config");
        }
    }
}
