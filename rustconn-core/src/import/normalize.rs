//! Import normalization utilities.
//!
//! Provides post-import processing to ensure consistency across all import sources:
//! - Deduplicates groups with identical names
//! - Validates and normalizes key paths
//! - Sets auth_method based on key_path presence
//! - Normalizes ports to protocol defaults
//! - Adds import metadata tags

use std::collections::HashMap;
use std::path::Path;

use chrono::Utc;
use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup, ProtocolConfig, SshAuthMethod};

use super::ImportResult;

/// Options for import normalization
#[derive(Debug, Clone, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct NormalizeOptions {
    /// Add source tag (e.g., "imported:ssh_config")
    pub add_source_tag: bool,
    /// Add timestamp tag (e.g., "imported_at:2024-01-15")
    pub add_timestamp_tag: bool,
    /// Validate that key paths exist on filesystem
    pub validate_key_paths: bool,
    /// Deduplicate groups with same name and parent
    pub deduplicate_groups: bool,
    /// Set auth_method based on key_path presence
    pub normalize_auth_method: bool,
    /// Replace port 0 with protocol default
    pub normalize_ports: bool,
}

impl NormalizeOptions {
    /// Creates options with all normalizations enabled
    #[must_use]
    pub fn all() -> Self {
        Self {
            add_source_tag: true,
            add_timestamp_tag: true,
            validate_key_paths: true,
            deduplicate_groups: true,
            normalize_auth_method: true,
            normalize_ports: true,
        }
    }

    /// Creates minimal options (only essential normalizations)
    #[must_use]
    pub fn minimal() -> Self {
        Self {
            add_source_tag: false,
            add_timestamp_tag: false,
            validate_key_paths: false,
            deduplicate_groups: true,
            normalize_auth_method: true,
            normalize_ports: true,
        }
    }
}

/// Normalizes an import result for consistency
pub struct ImportNormalizer {
    options: NormalizeOptions,
    source_id: String,
}

impl ImportNormalizer {
    /// Creates a new normalizer with the given options
    #[must_use]
    pub fn new(source_id: impl Into<String>, options: NormalizeOptions) -> Self {
        Self {
            options,
            source_id: source_id.into(),
        }
    }

    /// Creates a normalizer with all options enabled
    #[must_use]
    pub fn with_all_options(source_id: impl Into<String>) -> Self {
        Self::new(source_id, NormalizeOptions::all())
    }

    /// Normalizes the import result in place
    pub fn normalize(&self, result: &mut ImportResult) {
        // Deduplicate groups first (may remap group_ids)
        let group_remap = if self.options.deduplicate_groups {
            self.deduplicate_groups(&mut result.groups)
        } else {
            HashMap::new()
        };

        // Update connection group_ids if groups were deduplicated
        if !group_remap.is_empty() {
            for conn in &mut result.connections {
                if let Some(ref group_id) = conn.group_id
                    && let Some(&new_id) = group_remap.get(group_id)
                {
                    conn.group_id = Some(new_id);
                }
            }
        }

        // Normalize each connection
        for conn in &mut result.connections {
            self.normalize_connection(conn);
        }
    }

    /// Normalizes a single connection
    fn normalize_connection(&self, conn: &mut Connection) {
        // Sanitize name and host — strip trailing escape sequences from
        // INI-style sources (e.g. Remmina files with literal \n in values)
        conn.name = sanitize_imported_value(&conn.name);
        conn.host = sanitize_imported_value(&conn.host);
        if let Some(ref username) = conn.username {
            let clean = sanitize_imported_value(username);
            conn.username = if clean.is_empty() { None } else { Some(clean) };
        }

        // Add source tag
        if self.options.add_source_tag {
            let tag = format!("imported:{}", self.source_id);
            if !conn.tags.contains(&tag) {
                conn.tags.push(tag);
            }
        }

        // Add timestamp tag
        if self.options.add_timestamp_tag {
            let date = Utc::now().format("%Y-%m-%d").to_string();
            let tag = format!("imported_at:{date}");
            if !conn.tags.contains(&tag) {
                conn.tags.push(tag);
            }
        }

        // Normalize port
        if self.options.normalize_ports && conn.port == 0 {
            conn.port = conn.default_port();
        }

        // Normalize SSH config
        if let ProtocolConfig::Ssh(ref mut ssh_config) = conn.protocol_config {
            // Set auth_method based on key_path
            if self.options.normalize_auth_method && ssh_config.key_path.is_some() {
                ssh_config.auth_method = SshAuthMethod::PublicKey;
            }

            // Validate key path exists
            if self.options.validate_key_paths
                && let Some(ref key_path) = ssh_config.key_path
            {
                // Expand ~ to home directory
                let expanded = expand_tilde(key_path);
                if expanded.exists() {
                    // Update to expanded path
                    ssh_config.key_path = Some(expanded);
                } else {
                    // Key doesn't exist, clear it and fall back to password
                    ssh_config.key_path = None;
                    if ssh_config.auth_method == SshAuthMethod::PublicKey {
                        ssh_config.auth_method = SshAuthMethod::Password;
                    }
                }
            }
        }
    }

    /// Deduplicates groups with same name and parent, returns remap of old->new IDs
    fn deduplicate_groups(&self, groups: &mut Vec<ConnectionGroup>) -> HashMap<Uuid, Uuid> {
        let mut remap: HashMap<Uuid, Uuid> = HashMap::new();
        let mut seen: HashMap<(String, Option<Uuid>), Uuid> = HashMap::new();
        let mut to_remove: Vec<usize> = Vec::new();

        for (idx, group) in groups.iter().enumerate() {
            let key = (group.name.clone(), group.parent_id);
            if let Some(&existing_id) = seen.get(&key) {
                // Duplicate found, map old ID to existing
                remap.insert(group.id, existing_id);
                to_remove.push(idx);
            } else {
                seen.insert(key, group.id);
            }
        }

        // Remove duplicates in reverse order to preserve indices
        for idx in to_remove.into_iter().rev() {
            groups.remove(idx);
        }

        remap
    }
}

/// Expands ~ to home directory in a path
fn expand_tilde(path: &Path) -> std::path::PathBuf {
    let path_str = path.to_string_lossy();
    if let Some(stripped) = path_str.strip_prefix('~')
        && let Some(home) = dirs::home_dir()
    {
        let suffix = stripped.strip_prefix('/').unwrap_or(stripped);
        return home.join(suffix);
    }
    path.to_path_buf()
}

/// Parses a host:port string, returning (host, Option<port>)
#[must_use]
pub fn parse_host_port(server: &str) -> (String, Option<u16>) {
    // Handle IPv6 addresses like [::1]:22
    if server.starts_with('[')
        && let Some(bracket_end) = server.find(']')
    {
        let host = &server[1..bracket_end];
        let rest = &server[bracket_end + 1..];
        if let Some(port_str) = rest.strip_prefix(':')
            && let Ok(port) = port_str.parse::<u16>()
        {
            return (host.to_string(), Some(port));
        }
        return (host.to_string(), None);
    }

    // Handle regular host:port
    if let Some(colon_pos) = server.rfind(':') {
        let host = &server[..colon_pos];
        let port_str = &server[colon_pos + 1..];
        // Only treat as port if it's numeric
        if let Ok(port) = port_str.parse::<u16>() {
            return (host.to_string(), Some(port));
        }
    }

    (server.to_string(), None)
}

/// Strips trailing INI escape sequences and control characters from imported
/// string values.
///
/// Some sources (e.g. Remmina `.remmina` files) store literal `\n`, `\r`, or
/// `\t` escape sequences at the end of values. These are not real whitespace
/// — they are the two-character sequences backslash + letter — and
/// `str::trim()` does not remove them.
#[must_use]
pub fn sanitize_imported_value(value: &str) -> String {
    let mut s = value.trim().to_string();
    // Strip trailing literal escape sequences (\\n, \\r, \\t)
    loop {
        let trimmed = s
            .strip_suffix("\\n")
            .or_else(|| s.strip_suffix("\\r"))
            .or_else(|| s.strip_suffix("\\t"));
        match trimmed {
            Some(rest) => s = rest.trim_end().to_string(),
            None => break,
        }
    }
    s
}

/// Checks if a string looks like a valid hostname or IP
#[must_use]
pub fn is_valid_hostname(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Reject common placeholder values (localhost is intentionally allowed —
    // it is a valid target for SSH tunnels, port-forwarding, and local services)
    let lower = trimmed.to_lowercase();
    if lower == "tmp" || lower == "placeholder" || lower == "none" {
        return false;
    }

    // Reject hostnames with invalid characters
    if !trimmed
        .chars()
        .all(|c| c.is_alphanumeric() || ".-:[]_".contains(c))
    {
        return false;
    }

    true
}

/// Checks if a string looks like a hostname (contains dots, is IP, or has variables)
#[must_use]
pub fn looks_like_hostname(s: &str) -> bool {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return false;
    }

    // Contains dynamic variable syntax ($VAR or ${VAR})
    if trimmed.contains("${")
        || (trimmed.contains('$') && trimmed.chars().any(|c| c.is_alphabetic()))
    {
        return true;
    }

    // Contains dots (like a FQDN)
    if trimmed.contains('.') {
        return true;
    }

    // Looks like an IP address
    if trimmed.parse::<std::net::IpAddr>().is_ok() {
        return true;
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProtocolConfig, SshConfig};

    #[test]
    fn test_parse_host_port_simple() {
        let (host, port) = parse_host_port("example.com:22");
        assert_eq!(host, "example.com");
        assert_eq!(port, Some(22));
    }

    #[test]
    fn test_parse_host_port_no_port() {
        let (host, port) = parse_host_port("example.com");
        assert_eq!(host, "example.com");
        assert_eq!(port, None);
    }

    #[test]
    fn test_parse_host_port_ipv6() {
        let (host, port) = parse_host_port("[::1]:22");
        assert_eq!(host, "::1");
        assert_eq!(port, Some(22));
    }

    #[test]
    fn test_parse_host_port_ipv6_no_port() {
        let (host, port) = parse_host_port("[2001:db8::1]");
        assert_eq!(host, "2001:db8::1");
        assert_eq!(port, None);
    }

    #[test]
    fn test_is_valid_hostname() {
        assert!(is_valid_hostname("example.com"));
        assert!(is_valid_hostname("192.168.1.1"));
        assert!(!is_valid_hostname(""));
        assert!(!is_valid_hostname("tmp"));
        assert!(!is_valid_hostname("placeholder"));
    }

    #[test]
    fn test_looks_like_hostname() {
        assert!(looks_like_hostname("example.com"));
        assert!(looks_like_hostname("192.168.1.1"));
        assert!(looks_like_hostname("${HOST}"));
        assert!(looks_like_hostname("$HOSTNAME"));
        assert!(!looks_like_hostname("myserver"));
        assert!(!looks_like_hostname(""));
    }

    #[test]
    fn test_normalize_port() {
        let mut result = ImportResult::new();
        let conn = Connection::new_ssh("test".to_string(), "host".to_string(), 0);
        result.add_connection(conn);

        let normalizer = ImportNormalizer::new("test", NormalizeOptions::minimal());
        normalizer.normalize(&mut result);

        assert_eq!(result.connections[0].port, 22);
    }

    #[test]
    fn test_normalize_auth_method() {
        let mut result = ImportResult::new();
        let ssh_config = SshConfig {
            key_path: Some(std::path::PathBuf::from("/tmp/test_key")),
            auth_method: SshAuthMethod::Password, // Wrong, should be PublicKey
            ..Default::default()
        };
        let conn = Connection::new(
            "test".to_string(),
            "host".to_string(),
            22,
            ProtocolConfig::Ssh(ssh_config),
        );
        result.add_connection(conn);

        let mut options = NormalizeOptions::minimal();
        options.validate_key_paths = false; // Don't check if file exists
        let normalizer = ImportNormalizer::new("test", options);
        normalizer.normalize(&mut result);

        if let ProtocolConfig::Ssh(ref ssh) = result.connections[0].protocol_config {
            assert_eq!(ssh.auth_method, SshAuthMethod::PublicKey);
        } else {
            panic!("Expected SSH config");
        }
    }

    #[test]
    fn test_deduplicate_groups() {
        let mut result = ImportResult::new();
        let group1 = ConnectionGroup::new("Production".to_string());
        let group1_id = group1.id;
        let group2 = ConnectionGroup::new("Production".to_string()); // Duplicate
        let group2_id = group2.id;
        result.add_group(group1);
        result.add_group(group2);

        // Add connection pointing to duplicate group
        let mut conn = Connection::new_ssh("test".to_string(), "host".to_string(), 22);
        conn.group_id = Some(group2_id);
        result.add_connection(conn);

        let normalizer = ImportNormalizer::new("test", NormalizeOptions::minimal());
        normalizer.normalize(&mut result);

        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.connections[0].group_id, Some(group1_id));
    }

    #[test]
    fn test_add_source_tag() {
        let mut result = ImportResult::new();
        let conn = Connection::new_ssh("test".to_string(), "host".to_string(), 22);
        result.add_connection(conn);

        let options = NormalizeOptions {
            add_source_tag: true,
            ..Default::default()
        };
        let normalizer = ImportNormalizer::new("ssh_config", options);
        normalizer.normalize(&mut result);

        assert!(
            result.connections[0]
                .tags
                .contains(&"imported:ssh_config".to_string())
        );
    }

    #[test]
    fn test_sanitize_imported_value_trailing_newline() {
        assert_eq!(
            sanitize_imported_value("hostname.example.com\\n"),
            "hostname.example.com"
        );
    }

    #[test]
    fn test_sanitize_imported_value_trailing_return() {
        assert_eq!(
            sanitize_imported_value("hostname.example.com\\r"),
            "hostname.example.com"
        );
    }

    #[test]
    fn test_sanitize_imported_value_trailing_tab() {
        assert_eq!(
            sanitize_imported_value("hostname.example.com\\t"),
            "hostname.example.com"
        );
    }

    #[test]
    fn test_sanitize_imported_value_multiple_escapes() {
        assert_eq!(sanitize_imported_value("hostname\\n\\r\\t"), "hostname");
    }

    #[test]
    fn test_sanitize_imported_value_clean() {
        assert_eq!(
            sanitize_imported_value("hostname.example.com"),
            "hostname.example.com"
        );
    }

    #[test]
    fn test_sanitize_imported_value_whitespace() {
        assert_eq!(sanitize_imported_value("  hostname  "), "hostname");
    }

    #[test]
    fn test_sanitize_imported_value_empty() {
        assert_eq!(sanitize_imported_value(""), "");
    }

    #[test]
    fn test_normalize_sanitizes_name() {
        let mut result = ImportResult::new();
        let conn = Connection::new_ssh(
            "myhost.example.com\\n".to_string(),
            "myhost.example.com".to_string(),
            22,
        );
        result.add_connection(conn);

        let normalizer = ImportNormalizer::new("test", NormalizeOptions::minimal());
        normalizer.normalize(&mut result);

        assert_eq!(result.connections[0].name, "myhost.example.com");
    }
}
