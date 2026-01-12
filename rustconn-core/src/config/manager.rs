//! Configuration manager for TOML file operations
//!
//! This module provides the `ConfigManager` which handles loading and saving
//! configuration files for connections, groups, snippets, and application settings.

use std::fs;
use std::path::{Path, PathBuf};

use crate::cluster::Cluster;
use crate::error::{ConfigError, ConfigResult};
use crate::models::{
    Connection, ConnectionGroup, ConnectionHistoryEntry, ConnectionTemplate, Snippet,
};

use super::settings::AppSettings;

/// File names for configuration files
const CONNECTIONS_FILE: &str = "connections.toml";
const GROUPS_FILE: &str = "groups.toml";
const SNIPPETS_FILE: &str = "snippets.toml";
const CLUSTERS_FILE: &str = "clusters.toml";
const TEMPLATES_FILE: &str = "templates.toml";
const HISTORY_FILE: &str = "history.toml";
const CONFIG_FILE: &str = "config.toml";

/// Wrapper for serializing a list of connections
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct ConnectionsFile {
    #[serde(default)]
    connections: Vec<Connection>,
}

/// Wrapper for serializing a list of groups
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct GroupsFile {
    #[serde(default)]
    groups: Vec<ConnectionGroup>,
}

/// Wrapper for serializing a list of snippets
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct SnippetsFile {
    #[serde(default)]
    snippets: Vec<Snippet>,
}

/// Wrapper for serializing a list of clusters
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct ClustersFile {
    #[serde(default)]
    clusters: Vec<Cluster>,
}

/// Wrapper for serializing a list of templates
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct TemplatesFile {
    #[serde(default)]
    templates: Vec<ConnectionTemplate>,
}

/// Wrapper for serializing connection history
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct HistoryFile {
    #[serde(default)]
    entries: Vec<ConnectionHistoryEntry>,
}

/// Configuration manager for `RustConn`
///
/// Handles loading and saving configuration files in TOML format.
/// Configuration is stored in `~/.config/rustconn/` by default.
#[derive(Debug, Clone)]
pub struct ConfigManager {
    /// Base directory for configuration files
    config_dir: PathBuf,
}

impl ConfigManager {
    /// Creates a new `ConfigManager` with the default configuration directory
    ///
    /// The default directory is `~/.config/rustconn/`
    ///
    /// # Errors
    ///
    /// Returns an error if the home directory cannot be determined.
    pub fn new() -> ConfigResult<Self> {
        let config_dir = dirs::config_dir()
            .ok_or_else(|| ConfigError::NotFound(PathBuf::from("~/.config")))?
            .join("rustconn");
        Ok(Self { config_dir })
    }

    /// Creates a new `ConfigManager` with a custom configuration directory
    ///
    /// This is useful for testing or non-standard configurations.
    #[must_use]
    pub const fn with_config_dir(config_dir: PathBuf) -> Self {
        Self { config_dir }
    }

    /// Returns the configuration directory path
    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    /// Ensures the configuration directory exists
    ///
    /// Creates the directory and any parent directories if they don't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created.
    pub fn ensure_config_dir(&self) -> ConfigResult<()> {
        if !self.config_dir.exists() {
            fs::create_dir_all(&self.config_dir).map_err(|e| {
                ConfigError::Write(format!(
                    "Failed to create config directory {}: {}",
                    self.config_dir.display(),
                    e
                ))
            })?;
        }
        Ok(())
    }

    /// Ensures the logs directory exists
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created.
    pub fn ensure_logs_dir(&self) -> ConfigResult<PathBuf> {
        let logs_dir = self.config_dir.join("logs");
        if !logs_dir.exists() {
            fs::create_dir_all(&logs_dir).map_err(|e| {
                ConfigError::Write(format!(
                    "Failed to create logs directory {}: {}",
                    logs_dir.display(),
                    e
                ))
            })?;
        }
        Ok(logs_dir)
    }

    // ========== Connections ==========

    /// Loads connections from the configuration file
    ///
    /// Returns an empty vector if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_connections(&self) -> ConfigResult<Vec<Connection>> {
        let path = self.config_dir.join(CONNECTIONS_FILE);
        Self::load_toml_file::<ConnectionsFile>(&path).map(|f| f.connections)
    }

    /// Saves connections to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_connections(&self, connections: &[Connection]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(CONNECTIONS_FILE);
        let file = ConnectionsFile {
            connections: connections.to_vec(),
        };
        Self::save_toml_file(&path, &file)
    }

    // ========== Groups ==========

    /// Loads connection groups from the configuration file
    ///
    /// Returns an empty vector if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_groups(&self) -> ConfigResult<Vec<ConnectionGroup>> {
        let path = self.config_dir.join(GROUPS_FILE);
        Self::load_toml_file::<GroupsFile>(&path).map(|f| f.groups)
    }

    /// Saves connection groups to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_groups(&self, groups: &[ConnectionGroup]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(GROUPS_FILE);
        let file = GroupsFile {
            groups: groups.to_vec(),
        };
        Self::save_toml_file(&path, &file)
    }

    // ========== Snippets ==========

    /// Loads snippets from the configuration file
    ///
    /// Returns an empty vector if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_snippets(&self) -> ConfigResult<Vec<Snippet>> {
        let path = self.config_dir.join(SNIPPETS_FILE);
        Self::load_toml_file::<SnippetsFile>(&path).map(|f| f.snippets)
    }

    /// Saves snippets to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_snippets(&self, snippets: &[Snippet]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(SNIPPETS_FILE);
        let file = SnippetsFile {
            snippets: snippets.to_vec(),
        };
        Self::save_toml_file(&path, &file)
    }

    // ========== Clusters ==========

    /// Loads clusters from the configuration file
    ///
    /// Returns an empty vector if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_clusters(&self) -> ConfigResult<Vec<Cluster>> {
        let path = self.config_dir.join(CLUSTERS_FILE);
        Self::load_toml_file::<ClustersFile>(&path).map(|f| f.clusters)
    }

    /// Saves clusters to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_clusters(&self, clusters: &[Cluster]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(CLUSTERS_FILE);
        let file = ClustersFile {
            clusters: clusters.to_vec(),
        };
        Self::save_toml_file(&path, &file)
    }

    // ========== Templates ==========

    /// Loads templates from the configuration file
    ///
    /// Returns an empty vector if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_templates(&self) -> ConfigResult<Vec<ConnectionTemplate>> {
        let path = self.config_dir.join(TEMPLATES_FILE);
        Self::load_toml_file::<TemplatesFile>(&path).map(|f| f.templates)
    }

    /// Saves templates to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_templates(&self, templates: &[ConnectionTemplate]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(TEMPLATES_FILE);
        let file = TemplatesFile {
            templates: templates.to_vec(),
        };
        Self::save_toml_file(&path, &file)
    }

    // ========== Connection History ==========

    /// Loads connection history from the configuration file
    ///
    /// Returns an empty list if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_history(&self) -> ConfigResult<Vec<ConnectionHistoryEntry>> {
        let path = self.config_dir.join(HISTORY_FILE);
        Self::load_toml_file::<HistoryFile>(&path).map(|f| f.entries)
    }

    /// Saves connection history to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_history(&self, entries: &[ConnectionHistoryEntry]) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(HISTORY_FILE);
        let file = HistoryFile {
            entries: entries.to_vec(),
        };
        Self::save_toml_file(&path, &file)
    }

    // ========== Application Settings ==========

    /// Loads application settings from the configuration file
    ///
    /// Returns default settings if the file doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file exists but cannot be parsed.
    pub fn load_settings(&self) -> ConfigResult<AppSettings> {
        let path = self.config_dir.join(CONFIG_FILE);
        if !path.exists() {
            return Ok(AppSettings::default());
        }
        Self::load_toml_file(&path)
    }

    /// Saves application settings to the configuration file
    ///
    /// Creates the configuration directory if it doesn't exist.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be written.
    pub fn save_settings(&self, settings: &AppSettings) -> ConfigResult<()> {
        self.ensure_config_dir()?;
        let path = self.config_dir.join(CONFIG_FILE);
        Self::save_toml_file(&path, settings)
    }

    // ========== Global Variables ==========

    /// Loads global variables from the settings file
    ///
    /// Returns an empty vector if no variables are configured.
    ///
    /// # Errors
    ///
    /// Returns an error if the settings file cannot be read.
    pub fn load_variables(&self) -> ConfigResult<Vec<crate::variables::Variable>> {
        let settings = self.load_settings()?;
        Ok(settings.global_variables)
    }

    /// Saves global variables to the settings file
    ///
    /// # Errors
    ///
    /// Returns an error if the settings file cannot be written.
    pub fn save_variables(&self, variables: &[crate::variables::Variable]) -> ConfigResult<()> {
        let mut settings = self.load_settings()?;
        settings.global_variables = variables.to_vec();
        self.save_settings(&settings)
    }

    // ========== Generic TOML Operations ==========

    /// Loads and parses a TOML file
    ///
    /// Returns the default value if the file doesn't exist.
    fn load_toml_file<T>(path: &Path) -> ConfigResult<T>
    where
        T: serde::de::DeserializeOwned + Default,
    {
        if !path.exists() {
            return Ok(T::default());
        }

        let content = fs::read_to_string(path)
            .map_err(|e| ConfigError::Parse(format!("Failed to read {}: {}", path.display(), e)))?;

        Self::parse_toml(&content, path)
    }

    /// Parses TOML content with validation
    fn parse_toml<T>(content: &str, path: &Path) -> ConfigResult<T>
    where
        T: serde::de::DeserializeOwned,
    {
        toml::from_str(content).map_err(|e| {
            ConfigError::Deserialize(format!("Failed to parse {}: {}", path.display(), e))
        })
    }

    /// Saves data to a TOML file
    fn save_toml_file<T>(path: &Path, data: &T) -> ConfigResult<()>
    where
        T: serde::Serialize,
    {
        let content = toml::to_string_pretty(data)
            .map_err(|e| ConfigError::Serialize(format!("Failed to serialize: {e}")))?;

        fs::write(path, content)
            .map_err(|e| ConfigError::Write(format!("Failed to write {}: {}", path.display(), e)))
    }

    // ========== Validation ==========

    /// Validates a connection configuration
    ///
    /// # Errors
    ///
    /// Returns an error if the connection is invalid.
    pub fn validate_connection(connection: &Connection) -> ConfigResult<()> {
        use crate::models::ProtocolConfig;

        if connection.name.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "name".to_string(),
                reason: "Connection name cannot be empty".to_string(),
            });
        }

        // Host and port are optional for Zero Trust connections
        // (the target is defined in the provider config)
        let is_zerotrust = matches!(connection.protocol_config, ProtocolConfig::ZeroTrust(_));

        if !is_zerotrust && connection.host.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "host".to_string(),
                reason: "Host cannot be empty".to_string(),
            });
        }

        if !is_zerotrust && connection.port == 0 {
            return Err(ConfigError::Validation {
                field: "port".to_string(),
                reason: "Port must be greater than 0".to_string(),
            });
        }

        Ok(())
    }

    /// Validates a connection group
    ///
    /// # Errors
    ///
    /// Returns an error if the group is invalid.
    pub fn validate_group(group: &ConnectionGroup) -> ConfigResult<()> {
        if group.name.is_empty() {
            return Err(ConfigError::Validation {
                field: "name".to_string(),
                reason: "Group name cannot be empty".to_string(),
            });
        }

        Ok(())
    }

    /// Validates a snippet
    ///
    /// # Errors
    ///
    /// Returns an error if the snippet is invalid.
    pub fn validate_snippet(snippet: &Snippet) -> ConfigResult<()> {
        if snippet.name.is_empty() {
            return Err(ConfigError::Validation {
                field: "name".to_string(),
                reason: "Snippet name cannot be empty".to_string(),
            });
        }

        if snippet.command.is_empty() {
            return Err(ConfigError::Validation {
                field: "command".to_string(),
                reason: "Snippet command cannot be empty".to_string(),
            });
        }

        Ok(())
    }

    /// Validates a cluster
    ///
    /// # Errors
    ///
    /// Returns an error if the cluster is invalid.
    pub fn validate_cluster(cluster: &Cluster) -> ConfigResult<()> {
        if cluster.name.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "name".to_string(),
                reason: "Cluster name cannot be empty".to_string(),
            });
        }

        Ok(())
    }

    /// Validates all connections and returns errors for invalid ones
    #[must_use]
    pub fn validate_connections(connections: &[Connection]) -> Vec<(usize, ConfigError)> {
        connections
            .iter()
            .enumerate()
            .filter_map(|(i, conn)| Self::validate_connection(conn).err().map(|e| (i, e)))
            .collect()
    }

    /// Validates all groups and returns errors for invalid ones
    #[must_use]
    pub fn validate_groups(groups: &[ConnectionGroup]) -> Vec<(usize, ConfigError)> {
        groups
            .iter()
            .enumerate()
            .filter_map(|(i, group)| Self::validate_group(group).err().map(|e| (i, e)))
            .collect()
    }

    /// Validates all snippets and returns errors for invalid ones
    #[must_use]
    pub fn validate_snippets(snippets: &[Snippet]) -> Vec<(usize, ConfigError)> {
        snippets
            .iter()
            .enumerate()
            .filter_map(|(i, snippet)| Self::validate_snippet(snippet).err().map(|e| (i, e)))
            .collect()
    }

    /// Validates all clusters and returns errors for invalid ones
    #[must_use]
    pub fn validate_clusters(clusters: &[Cluster]) -> Vec<(usize, ConfigError)> {
        clusters
            .iter()
            .enumerate()
            .filter_map(|(i, cluster)| Self::validate_cluster(cluster).err().map(|e| (i, e)))
            .collect()
    }

    /// Validates a template
    ///
    /// # Errors
    ///
    /// Returns an error if the template is invalid.
    pub fn validate_template(template: &ConnectionTemplate) -> ConfigResult<()> {
        if template.name.trim().is_empty() {
            return Err(ConfigError::Validation {
                field: "name".to_string(),
                reason: "Template name cannot be empty".to_string(),
            });
        }

        Ok(())
    }

    /// Validates all templates and returns errors for invalid ones
    #[must_use]
    pub fn validate_templates(templates: &[ConnectionTemplate]) -> Vec<(usize, ConfigError)> {
        templates
            .iter()
            .enumerate()
            .filter_map(|(i, template)| Self::validate_template(template).err().map(|e| (i, e)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{ProtocolConfig, SshConfig};
    use tempfile::TempDir;

    fn create_test_manager() -> (ConfigManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        (manager, temp_dir)
    }

    #[test]
    fn test_ensure_config_dir() {
        let (manager, _temp) = create_test_manager();
        assert!(manager.ensure_config_dir().is_ok());
        assert!(manager.config_dir().exists());
    }

    #[test]
    fn test_load_empty_connections() {
        let (manager, _temp) = create_test_manager();
        let connections = manager.load_connections().unwrap();
        assert!(connections.is_empty());
    }

    #[test]
    fn test_save_and_load_connections() {
        let (manager, _temp) = create_test_manager();

        let conn = Connection::new(
            "Test Server".to_string(),
            "example.com".to_string(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );

        manager
            .save_connections(std::slice::from_ref(&conn))
            .unwrap();
        let loaded = manager.load_connections().unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, conn.name);
        assert_eq!(loaded[0].host, conn.host);
        assert_eq!(loaded[0].port, conn.port);
    }

    #[test]
    fn test_save_and_load_groups() {
        let (manager, _temp) = create_test_manager();

        let group = ConnectionGroup::new("Production".to_string());

        manager.save_groups(std::slice::from_ref(&group)).unwrap();
        let loaded = manager.load_groups().unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, group.name);
    }

    #[test]
    fn test_save_and_load_snippets() {
        let (manager, _temp) = create_test_manager();

        let snippet = Snippet::new("List files".to_string(), "ls -la".to_string());

        manager
            .save_snippets(std::slice::from_ref(&snippet))
            .unwrap();
        let loaded = manager.load_snippets().unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, snippet.name);
        assert_eq!(loaded[0].command, snippet.command);
    }

    #[test]
    fn test_save_and_load_settings() {
        let (manager, _temp) = create_test_manager();

        let mut settings = AppSettings::default();
        settings.terminal.font_size = 14;
        settings.logging.enabled = true;

        manager.save_settings(&settings).unwrap();
        let loaded = manager.load_settings().unwrap();

        assert_eq!(loaded.terminal.font_size, 14);
        assert!(loaded.logging.enabled);
    }

    #[test]
    fn test_validate_connection_empty_name() {
        let conn = Connection::new(
            String::new(),
            "example.com".to_string(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );

        let result = ConfigManager::validate_connection(&conn);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_connection_empty_host() {
        let conn = Connection::new(
            "Test".to_string(),
            String::new(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );

        let result = ConfigManager::validate_connection(&conn);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_group_empty_name() {
        let mut group = ConnectionGroup::new("Test".to_string());
        group.name = String::new();

        let result = ConfigManager::validate_group(&group);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_snippet_empty_command() {
        let mut snippet = Snippet::new("Test".to_string(), "ls".to_string());
        snippet.command = String::new();

        let result = ConfigManager::validate_snippet(&snippet);
        assert!(result.is_err());
    }

    #[test]
    fn test_save_and_load_clusters() {
        use crate::cluster::Cluster;
        use uuid::Uuid;

        let (manager, _temp) = create_test_manager();

        let mut cluster = Cluster::new("Production Servers".to_string());
        cluster.add_connection(Uuid::new_v4());
        cluster.add_connection(Uuid::new_v4());
        cluster.broadcast_enabled = true;

        manager
            .save_clusters(std::slice::from_ref(&cluster))
            .unwrap();
        let loaded = manager.load_clusters().unwrap();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].name, cluster.name);
        assert_eq!(loaded[0].id, cluster.id);
        assert_eq!(loaded[0].connection_ids.len(), 2);
        assert!(loaded[0].broadcast_enabled);
    }

    #[test]
    fn test_validate_cluster_empty_name() {
        use crate::cluster::Cluster;

        let mut cluster = Cluster::new("Test".to_string());
        cluster.name = String::new();

        let result = ConfigManager::validate_cluster(&cluster);
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_cluster_whitespace_name() {
        use crate::cluster::Cluster;

        let mut cluster = Cluster::new("Test".to_string());
        cluster.name = "   ".to_string();

        let result = ConfigManager::validate_cluster(&cluster);
        assert!(result.is_err());
    }
}
