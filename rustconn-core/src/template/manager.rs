//! Template manager for CRUD operations
//!
//! This module provides the `TemplateManager` which handles creating, reading,
//! updating, and deleting connection templates with persistence through `ConfigManager`.

use std::collections::HashMap;

use uuid::Uuid;

use crate::config::ConfigManager;
use crate::error::{ConfigError, ConfigResult};
use crate::models::{ConnectionTemplate, ProtocolType};

/// Manager for template CRUD operations
///
/// Provides in-memory storage with persistence through `ConfigManager`.
/// Supports protocol filtering and search functionality.
#[derive(Debug)]
pub struct TemplateManager {
    /// In-memory template storage indexed by ID
    templates: HashMap<Uuid, ConnectionTemplate>,
    /// Configuration manager for persistence
    config_manager: ConfigManager,
}

impl TemplateManager {
    /// Creates a new `TemplateManager` with the given `ConfigManager`
    ///
    /// Loads existing templates from storage.
    ///
    /// # Errors
    ///
    /// Returns an error if loading from storage fails.
    pub fn new(config_manager: ConfigManager) -> ConfigResult<Self> {
        let templates_vec = config_manager.load_templates()?;
        let templates = templates_vec.into_iter().map(|t| (t.id, t)).collect();
        Ok(Self {
            templates,
            config_manager,
        })
    }

    /// Creates a new `TemplateManager` with empty storage (for testing)
    #[cfg(test)]
    #[must_use]
    pub fn new_empty(config_manager: ConfigManager) -> Self {
        Self {
            templates: HashMap::new(),
            config_manager,
        }
    }

    // ========== Template CRUD Operations ==========

    /// Creates a new template and persists it
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails or persistence fails.
    pub fn create_template(&mut self, template: ConnectionTemplate) -> ConfigResult<Uuid> {
        ConfigManager::validate_template(&template)?;
        let id = template.id;
        self.templates.insert(id, template);
        self.persist_templates()?;
        Ok(id)
    }

    /// Updates an existing template
    ///
    /// Preserves the original ID and creation timestamp.
    ///
    /// # Errors
    ///
    /// Returns an error if the template doesn't exist, validation fails,
    /// or persistence fails.
    pub fn update_template(
        &mut self,
        id: Uuid,
        mut updated: ConnectionTemplate,
    ) -> ConfigResult<()> {
        if !self.templates.contains_key(&id) {
            return Err(ConfigError::Validation {
                field: "id".to_string(),
                reason: format!("Template with ID {id} not found"),
            });
        }

        updated.id = id;
        if let Some(existing) = self.templates.get(&id) {
            updated.created_at = existing.created_at;
        }
        updated.touch();

        ConfigManager::validate_template(&updated)?;
        self.templates.insert(id, updated);
        self.persist_templates()?;
        Ok(())
    }

    /// Deletes a template by ID
    ///
    /// # Errors
    ///
    /// Returns an error if the template doesn't exist or persistence fails.
    pub fn delete_template(&mut self, id: Uuid) -> ConfigResult<()> {
        if self.templates.remove(&id).is_none() {
            return Err(ConfigError::Validation {
                field: "id".to_string(),
                reason: format!("Template with ID {id} not found"),
            });
        }
        self.persist_templates()?;
        Ok(())
    }

    /// Gets a template by ID
    #[must_use]
    pub fn get_template(&self, id: Uuid) -> Option<&ConnectionTemplate> {
        self.templates.get(&id)
    }

    /// Lists all templates
    #[must_use]
    pub fn list_templates(&self) -> Vec<&ConnectionTemplate> {
        self.templates.values().collect()
    }

    /// Returns the total number of templates
    #[must_use]
    pub fn template_count(&self) -> usize {
        self.templates.len()
    }

    // ========== Protocol Filtering ==========

    /// Gets all templates for a specific protocol
    #[must_use]
    pub fn get_by_protocol(&self, protocol: ProtocolType) -> Vec<&ConnectionTemplate> {
        self.templates
            .values()
            .filter(|t| t.protocol == protocol)
            .collect()
    }

    /// Gets all unique protocol types across all templates
    #[must_use]
    pub fn get_all_protocols(&self) -> Vec<ProtocolType> {
        let mut protocols: Vec<ProtocolType> = self
            .templates
            .values()
            .map(|t| t.protocol)
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect();
        protocols.sort_by_key(|p| p.as_str().to_string());
        protocols
    }

    // ========== Search ==========

    /// Searches templates by query string
    ///
    /// Matches against name, description, host, and tags.
    /// Case-insensitive matching.
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&ConnectionTemplate> {
        let query_lower = query.to_lowercase();
        self.templates
            .values()
            .filter(|t| {
                if t.name.to_lowercase().contains(&query_lower) {
                    return true;
                }
                if let Some(ref desc) = t.description
                    && desc.to_lowercase().contains(&query_lower)
                {
                    return true;
                }
                if t.host.to_lowercase().contains(&query_lower) {
                    return true;
                }
                if t.tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&query_lower))
                {
                    return true;
                }
                false
            })
            .collect()
    }

    /// Finds a template by name (case-insensitive)
    ///
    /// Returns `None` if no match or multiple matches found.
    #[must_use]
    pub fn find_by_name(&self, name: &str) -> Option<&ConnectionTemplate> {
        let matches: Vec<_> = self
            .templates
            .values()
            .filter(|t| t.name.eq_ignore_ascii_case(name))
            .collect();
        if matches.len() == 1 {
            Some(matches[0])
        } else {
            None
        }
    }

    // ========== Import / Export ==========

    /// Imports templates, skipping duplicates by name+protocol
    ///
    /// Returns the number of templates actually imported.
    ///
    /// # Errors
    ///
    /// Returns an error if validation or persistence fails.
    pub fn import_templates(&mut self, templates: Vec<ConnectionTemplate>) -> ConfigResult<usize> {
        let mut imported = 0usize;
        for template in templates {
            let is_duplicate = self.templates.values().any(|existing| {
                existing.name == template.name && existing.protocol == template.protocol
            });
            if is_duplicate {
                continue;
            }
            ConfigManager::validate_template(&template)?;
            self.templates.insert(template.id, template);
            imported += 1;
        }
        if imported > 0 {
            self.persist_templates()?;
        }
        Ok(imported)
    }

    /// Exports all templates as a vector (for serialization)
    #[must_use]
    pub fn export_templates(&self) -> Vec<ConnectionTemplate> {
        self.templates.values().cloned().collect()
    }

    // ========== Persistence ==========

    /// Persists all templates to storage
    fn persist_templates(&self) -> ConfigResult<()> {
        let templates: Vec<ConnectionTemplate> = self.templates.values().cloned().collect();
        self.config_manager.save_templates(&templates)
    }

    /// Reloads templates from storage
    ///
    /// # Errors
    ///
    /// Returns an error if loading fails.
    pub fn reload(&mut self) -> ConfigResult<()> {
        let templates_vec = self.config_manager.load_templates()?;
        self.templates = templates_vec.into_iter().map(|t| (t.id, t)).collect();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::ConnectionTemplate;
    use tempfile::TempDir;

    fn create_test_manager() -> (TemplateManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        let manager = TemplateManager::new_empty(config_manager);
        (manager, temp_dir)
    }

    #[test]
    fn test_create_template() {
        let (mut manager, _temp) = create_test_manager();
        let template = ConnectionTemplate::new_ssh("SSH Server".to_string());
        let id = manager.create_template(template).unwrap();
        assert_eq!(manager.template_count(), 1);
        let t = manager.get_template(id).unwrap();
        assert_eq!(t.name, "SSH Server");
    }

    #[test]
    fn test_update_template() {
        let (mut manager, _temp) = create_test_manager();
        let template = ConnectionTemplate::new_ssh("SSH".to_string());
        let id = manager.create_template(template).unwrap();

        let mut updated = manager.get_template(id).unwrap().clone();
        updated.name = "SSH Updated".to_string();
        manager.update_template(id, updated).unwrap();

        let t = manager.get_template(id).unwrap();
        assert_eq!(t.name, "SSH Updated");
    }

    #[test]
    fn test_delete_template() {
        let (mut manager, _temp) = create_test_manager();
        let template = ConnectionTemplate::new_ssh("SSH".to_string());
        let id = manager.create_template(template).unwrap();
        assert_eq!(manager.template_count(), 1);
        manager.delete_template(id).unwrap();
        assert_eq!(manager.template_count(), 0);
    }

    #[test]
    fn test_get_by_protocol() {
        let (mut manager, _temp) = create_test_manager();
        manager
            .create_template(ConnectionTemplate::new_ssh("SSH 1".to_string()))
            .unwrap();
        manager
            .create_template(ConnectionTemplate::new_ssh("SSH 2".to_string()))
            .unwrap();
        manager
            .create_template(ConnectionTemplate::new_rdp("RDP 1".to_string()))
            .unwrap();

        assert_eq!(manager.get_by_protocol(ProtocolType::Ssh).len(), 2);
        assert_eq!(manager.get_by_protocol(ProtocolType::Rdp).len(), 1);
        assert_eq!(manager.get_by_protocol(ProtocolType::Vnc).len(), 0);
    }

    #[test]
    fn test_search() {
        let (mut manager, _temp) = create_test_manager();
        manager
            .create_template(
                ConnectionTemplate::new_ssh("Production SSH".to_string())
                    .with_host("prod.example.com"),
            )
            .unwrap();
        manager
            .create_template(ConnectionTemplate::new_rdp("Dev RDP".to_string()))
            .unwrap();

        assert_eq!(manager.search("prod").len(), 1);
        assert_eq!(manager.search("example").len(), 1);
        assert_eq!(manager.search("dev").len(), 1);
        assert_eq!(manager.search("nonexistent").len(), 0);
    }

    #[test]
    fn test_find_by_name() {
        let (mut manager, _temp) = create_test_manager();
        manager
            .create_template(ConnectionTemplate::new_ssh("My SSH".to_string()))
            .unwrap();

        assert!(manager.find_by_name("my ssh").is_some());
        assert!(manager.find_by_name("MY SSH").is_some());
        assert!(manager.find_by_name("nonexistent").is_none());
    }

    #[test]
    fn test_import_skips_duplicates() {
        let (mut manager, _temp) = create_test_manager();
        manager
            .create_template(ConnectionTemplate::new_ssh("SSH Server".to_string()))
            .unwrap();

        let imports = vec![
            ConnectionTemplate::new_ssh("SSH Server".to_string()), // duplicate
            ConnectionTemplate::new_rdp("RDP Server".to_string()), // new
        ];
        let count = manager.import_templates(imports).unwrap();
        assert_eq!(count, 1);
        assert_eq!(manager.template_count(), 2);
    }

    #[test]
    fn test_export_templates() {
        let (mut manager, _temp) = create_test_manager();
        manager
            .create_template(ConnectionTemplate::new_ssh("SSH".to_string()))
            .unwrap();
        manager
            .create_template(ConnectionTemplate::new_rdp("RDP".to_string()))
            .unwrap();

        let exported = manager.export_templates();
        assert_eq!(exported.len(), 2);
    }

    #[test]
    fn test_get_all_protocols() {
        let (mut manager, _temp) = create_test_manager();
        manager
            .create_template(ConnectionTemplate::new_ssh("SSH".to_string()))
            .unwrap();
        manager
            .create_template(ConnectionTemplate::new_rdp("RDP".to_string()))
            .unwrap();
        manager
            .create_template(ConnectionTemplate::new_ssh("SSH 2".to_string()))
            .unwrap();

        let protocols = manager.get_all_protocols();
        assert_eq!(protocols.len(), 2);
    }
}
