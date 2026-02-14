//! Ansible inventory importer.
//!
//! Parses Ansible inventory files in INI and YAML formats.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use uuid::Uuid;

use crate::error::ImportError;
use crate::models::{Connection, ConnectionGroup, ProtocolConfig, SshConfig};

use super::traits::{read_import_file, ImportResult, ImportSource, SkippedEntry};

/// Importer for Ansible inventory files.
///
/// Supports both INI-style and YAML inventory formats.
pub struct AnsibleInventoryImporter {
    /// Custom paths to search for inventory files
    custom_paths: Vec<PathBuf>,
}

impl AnsibleInventoryImporter {
    /// Creates a new Ansible inventory importer with default paths
    #[must_use]
    pub const fn new() -> Self {
        Self {
            custom_paths: Vec::new(),
        }
    }

    /// Creates a new Ansible inventory importer with custom paths
    #[must_use]
    pub const fn with_paths(paths: Vec<PathBuf>) -> Self {
        Self {
            custom_paths: paths,
        }
    }

    /// Parses inventory content (auto-detects format)
    #[must_use]
    pub fn parse_inventory(&self, content: &str, source_path: &str) -> ImportResult {
        // Try YAML first, then INI
        if content.trim().starts_with("---") || content.contains("hosts:") {
            self.parse_yaml_inventory(content, source_path)
        } else {
            self.parse_ini_inventory(content, source_path)
        }
    }

    /// Parses INI-style Ansible inventory
    #[must_use]
    pub fn parse_ini_inventory(&self, content: &str, source_path: &str) -> ImportResult {
        let mut result = ImportResult::new();
        let mut current_group: Option<(String, Uuid)> = None;
        let mut group_vars: HashMap<String, HashMap<String, String>> = HashMap::new();
        let mut in_vars_section = false;
        let mut vars_group_name: Option<String> = None;

        for (line_num, line) in content.lines().enumerate() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            // Check for group header
            if line.starts_with('[') && line.ends_with(']') {
                let section = &line[1..line.len() - 1];

                // Check if this is a :vars section
                if section.contains(":vars") {
                    in_vars_section = true;
                    vars_group_name = Some(section.split(':').next().unwrap_or("").to_string());
                    continue;
                }

                // Check if this is a :children section (skip for now)
                if section.contains(":children") {
                    in_vars_section = false;
                    vars_group_name = None;
                    current_group = None;
                    continue;
                }

                in_vars_section = false;
                vars_group_name = None;

                // Create new group
                let group = ConnectionGroup::new(section.to_string());
                let group_id = group.id;
                result.add_group(group);
                current_group = Some((section.to_string(), group_id));
                continue;
            }

            // Handle vars section
            if in_vars_section {
                if let Some(ref group_name) = vars_group_name {
                    if let Some((key, value)) = Self::parse_var_line(line) {
                        group_vars
                            .entry(group_name.clone())
                            .or_default()
                            .insert(key, value);
                    }
                }
                continue;
            }

            // Parse host line
            if let Some(connection) = self.parse_host_line(
                line,
                line_num + 1,
                current_group.as_ref().map(|(_, id)| *id),
                source_path,
                &mut result,
            ) {
                result.add_connection(connection);
            }
        }

        result
    }

    /// Parses a variable assignment line
    fn parse_var_line(line: &str) -> Option<(String, String)> {
        if let Some(eq_pos) = line.find('=') {
            let key = line[..eq_pos].trim().to_string();
            let value = line[eq_pos + 1..].trim().to_string();
            if !key.is_empty() {
                return Some((key, value));
            }
        }
        None
    }

    /// Parses a host line from INI inventory
    fn parse_host_line(
        &self,
        line: &str,
        line_num: usize,
        group_id: Option<Uuid>,
        source_path: &str,
        result: &mut ImportResult,
    ) -> Option<Connection> {
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.is_empty() {
            return None;
        }

        let host_pattern = parts[0];

        // Skip patterns with ranges like web[1:10]
        if host_pattern.contains('[') && host_pattern.contains(':') {
            result.add_skipped(SkippedEntry::with_location(
                host_pattern,
                "Host ranges are not supported",
                format!("{source_path}:{line_num}"),
            ));
            return None;
        }

        // Parse inline variables
        let mut vars: HashMap<String, String> = HashMap::new();
        for part in &parts[1..] {
            if let Some((key, value)) = Self::parse_var_line(part) {
                vars.insert(key, value);
            }
        }

        // Determine actual hostname
        let hostname = vars
            .get("ansible_host")
            .cloned()
            .unwrap_or_else(|| host_pattern.to_string());

        // Skip if hostname looks like a pattern
        if hostname.contains('*') || hostname.contains('?') {
            result.add_skipped(SkippedEntry::with_location(
                host_pattern,
                "Wildcard patterns are not supported",
                format!("{source_path}:{line_num}"),
            ));
            return None;
        }

        let port = vars
            .get("ansible_port")
            .or_else(|| vars.get("ansible_ssh_port"))
            .and_then(|p| p.parse().ok())
            .unwrap_or(22);

        let username = vars
            .get("ansible_user")
            .or_else(|| vars.get("ansible_ssh_user"))
            .cloned();

        let key_path = vars
            .get("ansible_ssh_private_key_file")
            .map(|p| PathBuf::from(shellexpand::tilde(p).into_owned()));

        let ssh_config = SshConfig {
            key_path,
            ..Default::default()
        };

        let mut connection = Connection::new(
            host_pattern.to_string(),
            hostname,
            port,
            ProtocolConfig::Ssh(ssh_config),
        );

        connection.username = username;
        connection.group_id = group_id;

        Some(connection)
    }

    /// Parses YAML-style Ansible inventory
    #[must_use]
    pub fn parse_yaml_inventory(&self, content: &str, source_path: &str) -> ImportResult {
        let mut result = ImportResult::new();

        // Parse YAML
        let yaml: serde_yaml::Value = match serde_yaml::from_str(content) {
            Ok(v) => v,
            Err(e) => {
                result.add_error(ImportError::ParseError {
                    source_name: "Ansible inventory".to_string(),
                    reason: format!("Failed to parse YAML: {e}"),
                });
                return result;
            }
        };

        // Process top-level groups
        if let serde_yaml::Value::Mapping(map) = yaml {
            for (group_name, group_value) in map {
                if let serde_yaml::Value::String(name) = group_name {
                    // Skip special keys
                    if name == "all" {
                        // Process 'all' group's children
                        if let serde_yaml::Value::Mapping(all_map) = group_value {
                            if let Some(serde_yaml::Value::Mapping(children)) =
                                all_map.get(serde_yaml::Value::String("children".to_string()))
                            {
                                for (child_name, child_value) in children {
                                    if let serde_yaml::Value::String(child_name_str) = child_name {
                                        self.process_yaml_group(
                                            child_name_str,
                                            child_value,
                                            None,
                                            source_path,
                                            &mut result,
                                        );
                                    }
                                }
                            }
                            // Also process hosts directly under 'all'
                            if let Some(hosts) =
                                all_map.get(serde_yaml::Value::String("hosts".to_string()))
                            {
                                self.process_yaml_hosts(hosts, None, source_path, &mut result);
                            }
                        }
                    } else {
                        self.process_yaml_group(
                            &name,
                            &group_value,
                            None,
                            source_path,
                            &mut result,
                        );
                    }
                }
            }
        }

        result
    }

    /// Processes a YAML group
    fn process_yaml_group(
        &self,
        name: &str,
        value: &serde_yaml::Value,
        parent_id: Option<Uuid>,
        source_path: &str,
        result: &mut ImportResult,
    ) {
        // Create group
        let group = parent_id.map_or_else(
            || ConnectionGroup::new(name.to_string()),
            |parent| ConnectionGroup::with_parent(name.to_string(), parent),
        );
        let group_id = group.id;
        result.add_group(group);

        if let serde_yaml::Value::Mapping(map) = value {
            // Process hosts
            if let Some(hosts) = map.get(serde_yaml::Value::String("hosts".to_string())) {
                self.process_yaml_hosts(hosts, Some(group_id), source_path, result);
            }

            // Process children groups
            if let Some(serde_yaml::Value::Mapping(children)) =
                map.get(serde_yaml::Value::String("children".to_string()))
            {
                for (child_name, child_value) in children {
                    if let serde_yaml::Value::String(child_name_str) = child_name {
                        self.process_yaml_group(
                            child_name_str,
                            child_value,
                            Some(group_id),
                            source_path,
                            result,
                        );
                    }
                }
            }
        }
    }

    /// Processes YAML hosts section
    fn process_yaml_hosts(
        &self,
        hosts: &serde_yaml::Value,
        group_id: Option<Uuid>,
        source_path: &str,
        result: &mut ImportResult,
    ) {
        if let serde_yaml::Value::Mapping(hosts_map) = hosts {
            for (host_name, host_vars) in hosts_map {
                if let serde_yaml::Value::String(name) = host_name {
                    // Skip patterns
                    if name.contains('*') || name.contains('?') || name.contains('[') {
                        result.add_skipped(SkippedEntry::with_location(
                            name,
                            "Patterns are not supported",
                            source_path,
                        ));
                        continue;
                    }

                    let (hostname, port, username, key_path) = match host_vars {
                        serde_yaml::Value::Mapping(vars) => {
                            let hostname = vars
                                .get(serde_yaml::Value::String("ansible_host".to_string()))
                                .and_then(|v| v.as_str())
                                .map_or_else(|| name.clone(), String::from);

                            #[allow(clippy::cast_possible_wrap)]
                            let port = vars
                                .get(serde_yaml::Value::String("ansible_port".to_string()))
                                .and_then(serde_yaml::Value::as_u64)
                                .map_or(22, |p| p as u16);

                            let username = vars
                                .get(serde_yaml::Value::String("ansible_user".to_string()))
                                .and_then(|v| v.as_str())
                                .map(String::from);

                            let key_path = vars
                                .get(serde_yaml::Value::String(
                                    "ansible_ssh_private_key_file".to_string(),
                                ))
                                .and_then(|v| v.as_str())
                                .map(|p| PathBuf::from(shellexpand::tilde(p).into_owned()));

                            (hostname, port, username, key_path)
                        }
                        serde_yaml::Value::Null => (name.clone(), 22, None, None),
                        _ => continue,
                    };

                    let ssh_config = SshConfig {
                        key_path,
                        ..Default::default()
                    };

                    let mut connection = Connection::new(
                        name.clone(),
                        hostname,
                        port,
                        ProtocolConfig::Ssh(ssh_config),
                    );

                    connection.username = username;
                    connection.group_id = group_id;

                    result.add_connection(connection);
                }
            }
        }
    }
}

impl Default for AnsibleInventoryImporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ImportSource for AnsibleInventoryImporter {
    fn source_id(&self) -> &'static str {
        "ansible"
    }

    fn display_name(&self) -> &'static str {
        "Ansible Inventory"
    }

    fn is_available(&self) -> bool {
        // Ansible inventory can be anywhere, so we just check common locations
        self.default_paths().iter().any(|p| p.exists())
    }

    fn default_paths(&self) -> Vec<PathBuf> {
        if !self.custom_paths.is_empty() {
            return self.custom_paths.clone();
        }

        let mut paths = Vec::new();

        // Check common inventory locations
        let common_paths = [
            "/etc/ansible/hosts",
            "inventory",
            "inventory.yml",
            "inventory.yaml",
            "inventory.ini",
            "hosts",
            "hosts.yml",
            "hosts.yaml",
        ];

        for path_str in &common_paths {
            let path = PathBuf::from(path_str);
            if path.exists() {
                paths.push(path);
            }
        }

        // Check home directory
        if let Some(home) = dirs::home_dir() {
            let ansible_dir = home.join(".ansible");
            if ansible_dir.is_dir() {
                for name in ["hosts", "inventory", "inventory.yml"] {
                    let path = ansible_dir.join(name);
                    if path.exists() {
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
            return Err(ImportError::FileNotFound(PathBuf::from("inventory")));
        }

        let mut combined_result = ImportResult::new();

        for path in paths {
            match self.import_from_path(&path) {
                Ok(result) => combined_result.merge(result),
                Err(e) => combined_result.add_error(e),
            }
        }

        Ok(combined_result)
    }

    fn import_from_path(&self, path: &Path) -> Result<ImportResult, ImportError> {
        let content = read_import_file(path, "Ansible inventory")?;

        Ok(self.parse_inventory(&content, &path.display().to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ini_simple() {
        let importer = AnsibleInventoryImporter::new();
        let content = r"
[webservers]
web1.example.com
web2.example.com ansible_host=192.168.1.2 ansible_port=2222

[dbservers]
db1.example.com ansible_user=postgres
";

        let result = importer.parse_ini_inventory(content, "test");
        assert_eq!(result.groups.len(), 2);
        assert_eq!(result.connections.len(), 3);

        // Check web2 has custom host and port
        let web2 = result
            .connections
            .iter()
            .find(|c| c.name == "web2.example.com")
            .unwrap();
        assert_eq!(web2.host, "192.168.1.2");
        assert_eq!(web2.port, 2222);

        // Check db1 has username
        let db1 = result
            .connections
            .iter()
            .find(|c| c.name == "db1.example.com")
            .unwrap();
        assert_eq!(db1.username, Some("postgres".to_string()));
    }

    #[test]
    fn test_parse_yaml_simple() {
        let importer = AnsibleInventoryImporter::new();
        let content = r"
all:
  children:
    webservers:
      hosts:
        web1.example.com:
          ansible_host: 192.168.1.1
        web2.example.com:
          ansible_host: 192.168.1.2
          ansible_port: 2222
    dbservers:
      hosts:
        db1.example.com:
          ansible_user: postgres
";

        let result = importer.parse_yaml_inventory(content, "test");
        assert_eq!(result.groups.len(), 2);
        assert_eq!(result.connections.len(), 3);
    }

    #[test]
    fn test_skip_host_ranges() {
        let importer = AnsibleInventoryImporter::new();
        let content = r"
[webservers]
web[1:10].example.com
";

        let result = importer.parse_ini_inventory(content, "test");
        assert_eq!(result.connections.len(), 0);
        assert_eq!(result.skipped.len(), 1);
    }

    #[test]
    fn test_auto_detect_format() {
        let importer = AnsibleInventoryImporter::new();

        // INI format
        let ini_content = "[servers]\nserver1";
        let result = importer.parse_inventory(ini_content, "test");
        assert_eq!(result.connections.len(), 1);

        // YAML format
        let yaml_content = "---\nall:\n  hosts:\n    server1:";
        let result = importer.parse_inventory(yaml_content, "test");
        assert_eq!(result.connections.len(), 1);
    }
}
