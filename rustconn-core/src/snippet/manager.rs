//! Snippet manager for CRUD operations
//!
//! This module provides the `SnippetManager` which handles creating, reading,
//! updating, and deleting command snippets with persistence through `ConfigManager`.

use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

use regex::Regex;
use uuid::Uuid;

use crate::config::ConfigManager;
use crate::error::{ConfigError, ConfigResult};
use crate::models::{Snippet, SnippetVariable};

/// Cached regex for variable extraction: matches `${var_name}` patterns
static VARIABLE_REGEX: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"\$\{([a-zA-Z_][a-zA-Z0-9_]*)\}").expect("VARIABLE_REGEX is a valid regex pattern")
});

/// Manager for snippet CRUD operations
///
/// Provides in-memory storage with persistence through `ConfigManager`.
/// Supports category/tag organization and search functionality.
#[derive(Debug)]
pub struct SnippetManager {
    /// In-memory snippet storage indexed by ID
    snippets: HashMap<Uuid, Snippet>,
    /// Configuration manager for persistence
    config_manager: ConfigManager,
}

impl SnippetManager {
    /// Creates a new `SnippetManager` with the given `ConfigManager`
    ///
    /// Loads existing snippets from storage.
    ///
    /// # Errors
    ///
    /// Returns an error if loading from storage fails.
    pub fn new(config_manager: ConfigManager) -> ConfigResult<Self> {
        let snippets_vec = config_manager.load_snippets()?;

        let snippets = snippets_vec.into_iter().map(|s| (s.id, s)).collect();

        Ok(Self {
            snippets,
            config_manager,
        })
    }

    /// Creates a new `SnippetManager` with empty storage (for testing)
    #[cfg(test)]
    #[must_use]
    pub fn new_empty(config_manager: ConfigManager) -> Self {
        Self {
            snippets: HashMap::new(),
            config_manager,
        }
    }

    // ========== Snippet CRUD Operations ==========

    /// Creates a new snippet and persists it
    ///
    /// # Arguments
    ///
    /// * `name` - Human-readable name for the snippet
    /// * `command` - Command template (may contain ${variable} placeholders)
    ///
    /// # Returns
    ///
    /// The UUID of the newly created snippet
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails or persistence fails.
    pub fn create_snippet(&mut self, name: String, command: String) -> ConfigResult<Uuid> {
        let snippet = Snippet::new(name, command);
        ConfigManager::validate_snippet(&snippet)?;

        let id = snippet.id;
        self.snippets.insert(id, snippet);
        self.persist_snippets()?;

        Ok(id)
    }

    /// Creates a snippet from an existing Snippet object
    ///
    /// Useful for importing snippets or restoring from backup.
    ///
    /// # Errors
    ///
    /// Returns an error if validation fails or persistence fails.
    pub fn create_snippet_from(&mut self, snippet: Snippet) -> ConfigResult<Uuid> {
        ConfigManager::validate_snippet(&snippet)?;

        let id = snippet.id;
        self.snippets.insert(id, snippet);
        self.persist_snippets()?;

        Ok(id)
    }

    /// Updates an existing snippet
    ///
    /// Preserves the original ID.
    ///
    /// # Errors
    ///
    /// Returns an error if the snippet doesn't exist, validation fails,
    /// or persistence fails.
    pub fn update_snippet(&mut self, id: Uuid, mut updated: Snippet) -> ConfigResult<()> {
        if !self.snippets.contains_key(&id) {
            return Err(ConfigError::Validation {
                field: "id".to_string(),
                reason: format!("Snippet with ID {id} not found"),
            });
        }

        // Preserve original ID
        updated.id = id;

        // Preserve original creation timestamp and update modification time
        if let Some(existing) = self.snippets.get(&id) {
            updated.created_at = existing.created_at;
        }
        updated.updated_at = chrono::Utc::now();

        ConfigManager::validate_snippet(&updated)?;

        self.snippets.insert(id, updated);
        self.persist_snippets()?;

        Ok(())
    }

    /// Deletes a snippet by ID
    ///
    /// # Errors
    ///
    /// Returns an error if the snippet doesn't exist or persistence fails.
    pub fn delete_snippet(&mut self, id: Uuid) -> ConfigResult<()> {
        if self.snippets.remove(&id).is_none() {
            return Err(ConfigError::Validation {
                field: "id".to_string(),
                reason: format!("Snippet with ID {id} not found"),
            });
        }

        self.persist_snippets()?;
        Ok(())
    }

    /// Gets a snippet by ID
    #[must_use]
    pub fn get_snippet(&self, id: Uuid) -> Option<&Snippet> {
        self.snippets.get(&id)
    }

    /// Gets a mutable reference to a snippet by ID
    pub fn get_snippet_mut(&mut self, id: Uuid) -> Option<&mut Snippet> {
        self.snippets.get_mut(&id)
    }

    /// Lists all snippets
    #[must_use]
    pub fn list_snippets(&self) -> Vec<&Snippet> {
        self.snippets.values().collect()
    }

    /// Returns the total number of snippets
    #[must_use]
    pub fn snippet_count(&self) -> usize {
        self.snippets.len()
    }

    // ========== Category/Tag Organization ==========

    /// Gets all snippets in a specific category
    #[must_use]
    pub fn get_by_category(&self, category: &str) -> Vec<&Snippet> {
        let category_lower = category.to_lowercase();
        self.snippets
            .values()
            .filter(|s| {
                s.category
                    .as_ref()
                    .is_some_and(|c| c.to_lowercase() == category_lower)
            })
            .collect()
    }

    /// Gets all snippets without a category
    #[must_use]
    pub fn get_uncategorized(&self) -> Vec<&Snippet> {
        self.snippets
            .values()
            .filter(|s| s.category.is_none())
            .collect()
    }

    /// Gets all unique categories across all snippets
    #[must_use]
    pub fn get_all_categories(&self) -> Vec<String> {
        let mut categories: Vec<String> = self
            .snippets
            .values()
            .filter_map(|s| s.category.clone())
            .collect();

        categories.sort_unstable();
        categories.dedup();
        categories
    }

    /// Filters snippets by tag
    ///
    /// Returns all snippets that have the specified tag.
    /// Case-insensitive matching.
    #[must_use]
    pub fn filter_by_tag(&self, tag: &str) -> Vec<&Snippet> {
        let tag_lower = tag.to_lowercase();

        self.snippets
            .values()
            .filter(|s| s.tags.iter().any(|t| t.to_lowercase() == tag_lower))
            .collect()
    }

    /// Filters snippets by multiple tags (AND logic)
    ///
    /// Returns snippets that have ALL specified tags.
    #[must_use]
    pub fn filter_by_tags(&self, tags: &[String]) -> Vec<&Snippet> {
        let tags_lower: Vec<String> = tags.iter().map(|t| t.to_lowercase()).collect();

        self.snippets
            .values()
            .filter(|s| {
                let snippet_tags_lower: Vec<String> =
                    s.tags.iter().map(|t| t.to_lowercase()).collect();
                tags_lower
                    .iter()
                    .all(|tag| snippet_tags_lower.contains(tag))
            })
            .collect()
    }

    /// Gets all unique tags across all snippets
    #[must_use]
    pub fn get_all_tags(&self) -> Vec<String> {
        let mut tags: Vec<String> = self
            .snippets
            .values()
            .flat_map(|s| s.tags.iter().cloned())
            .collect();

        tags.sort_unstable();
        tags.dedup();
        tags
    }

    // ========== Search Functionality ==========

    /// Searches snippets by query string
    ///
    /// Matches against name, command content, description, and category.
    /// Case-insensitive matching.
    ///
    /// # Arguments
    ///
    /// * `query` - The search query string
    ///
    /// # Returns
    ///
    /// A vector of references to matching snippets
    #[must_use]
    pub fn search(&self, query: &str) -> Vec<&Snippet> {
        let query_lower = query.to_lowercase();

        self.snippets
            .values()
            .filter(|snippet| {
                // Match name
                if snippet.name.to_lowercase().contains(&query_lower) {
                    return true;
                }

                // Match command content
                if snippet.command.to_lowercase().contains(&query_lower) {
                    return true;
                }

                // Match description
                if let Some(ref desc) = snippet.description
                    && desc.to_lowercase().contains(&query_lower)
                {
                    return true;
                }

                // Match category
                if let Some(ref cat) = snippet.category
                    && cat.to_lowercase().contains(&query_lower)
                {
                    return true;
                }

                // Match tags
                if snippet
                    .tags
                    .iter()
                    .any(|tag| tag.to_lowercase().contains(&query_lower))
                {
                    return true;
                }

                false
            })
            .collect()
    }

    // ========== Variable Extraction and Substitution ==========

    /// Extracts all unique variable names from a command template
    ///
    /// Variables are in the format `${var_name}` where `var_name` can contain
    /// alphanumeric characters and underscores.
    ///
    /// # Arguments
    ///
    /// * `command` - The command template to extract variables from
    ///
    /// # Returns
    ///
    /// A vector of unique variable names found in the template
    ///
    /// # Panics
    ///
    /// Panics if the internal regex pattern is invalid (should never happen).
    #[must_use]
    pub fn extract_variables(command: &str) -> Vec<String> {
        let mut variables: HashSet<String> = HashSet::new();

        for cap in VARIABLE_REGEX.captures_iter(command) {
            if let Some(var_name) = cap.get(1) {
                variables.insert(var_name.as_str().to_string());
            }
        }

        let mut result: Vec<String> = variables.into_iter().collect();
        result.sort(); // Sort for consistent ordering
        result
    }

    /// Creates `SnippetVariable` objects from extracted variable names
    ///
    /// # Arguments
    ///
    /// * `command` - The command template to extract variables from
    ///
    /// # Returns
    ///
    /// A vector of `SnippetVariable` objects for each unique variable
    #[must_use]
    pub fn extract_variable_objects(command: &str) -> Vec<SnippetVariable> {
        Self::extract_variables(command)
            .into_iter()
            .map(SnippetVariable::new)
            .collect()
    }

    /// Substitutes variables in a command template with provided values
    ///
    /// # Arguments
    ///
    /// * `command` - The command template with `${var_name}` placeholders
    /// * `values` - A map of variable names to their replacement values
    ///
    /// # Returns
    ///
    /// The command with all variables substituted. Variables without
    /// provided values are left unchanged.
    ///
    /// # Panics
    ///
    /// Panics if the internal regex pattern is invalid (should never happen).
    #[must_use]
    pub fn substitute_variables(command: &str, values: &HashMap<String, String>) -> String {
        VARIABLE_REGEX
            .replace_all(command, |caps: &regex::Captures| {
                let var_name = caps.get(1).map_or("", |m| m.as_str());
                values
                    .get(var_name)
                    .cloned()
                    .unwrap_or_else(|| caps[0].to_string())
            })
            .to_string()
    }

    /// Substitutes variables using the snippet's defined variables with defaults
    ///
    /// # Arguments
    ///
    /// * `snippet` - The snippet containing the command and variable definitions
    /// * `values` - A map of variable names to their replacement values
    ///
    /// # Returns
    ///
    /// The command with all variables substituted. Variables without
    /// provided values use their default values if defined.
    #[must_use]
    pub fn substitute_with_defaults(snippet: &Snippet, values: &HashMap<String, String>) -> String {
        // Build a combined values map with defaults
        let mut combined_values = HashMap::new();

        // First, add defaults from snippet variables
        for var in &snippet.variables {
            if let Some(ref default) = var.default_value {
                combined_values.insert(var.name.clone(), default.clone());
            }
        }

        // Then, override with provided values
        for (key, value) in values {
            combined_values.insert(key.clone(), value.clone());
        }

        Self::substitute_variables(&snippet.command, &combined_values)
    }

    /// Validates that all required variables have values
    ///
    /// # Arguments
    ///
    /// * `snippet` - The snippet to validate
    /// * `values` - The provided variable values
    ///
    /// # Returns
    ///
    /// A list of variable names that are missing values (no provided value and no default)
    #[must_use]
    pub fn get_missing_variables(
        snippet: &Snippet,
        values: &HashMap<String, String>,
    ) -> Vec<String> {
        let required_vars = Self::extract_variables(&snippet.command);

        required_vars
            .into_iter()
            .filter(|var_name| {
                // Check if value is provided
                if values.contains_key(var_name) {
                    return false;
                }

                // Check if default exists
                let has_default = snippet
                    .variables
                    .iter()
                    .any(|v| &v.name == var_name && v.default_value.is_some());

                !has_default
            })
            .collect()
    }

    // ========== Persistence ==========

    /// Persists all snippets to storage
    fn persist_snippets(&self) -> ConfigResult<()> {
        let snippets: Vec<Snippet> = self.snippets.values().cloned().collect();
        self.config_manager.save_snippets(&snippets)
    }

    /// Reloads snippets from storage
    ///
    /// # Errors
    ///
    /// Returns an error if loading fails.
    pub fn reload(&mut self) -> ConfigResult<()> {
        let snippets_vec = self.config_manager.load_snippets()?;

        self.snippets = snippets_vec.into_iter().map(|s| (s.id, s)).collect();

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn create_test_manager() -> (SnippetManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        let manager = SnippetManager::new_empty(config_manager);
        (manager, temp_dir)
    }

    #[test]
    fn test_create_snippet() {
        let (mut manager, _temp) = create_test_manager();

        let id = manager
            .create_snippet("List files".to_string(), "ls -la".to_string())
            .unwrap();

        assert_eq!(manager.snippet_count(), 1);
        let snippet = manager.get_snippet(id).unwrap();
        assert_eq!(snippet.name, "List files");
        assert_eq!(snippet.command, "ls -la");
    }

    #[test]
    fn test_update_snippet() {
        let (mut manager, _temp) = create_test_manager();

        let id = manager
            .create_snippet("List files".to_string(), "ls -la".to_string())
            .unwrap();

        let mut updated = manager.get_snippet(id).unwrap().clone();
        updated.name = "List all files".to_string();
        updated.command = "ls -la --color".to_string();

        manager.update_snippet(id, updated).unwrap();

        let snippet = manager.get_snippet(id).unwrap();
        assert_eq!(snippet.name, "List all files");
        assert_eq!(snippet.command, "ls -la --color");
    }

    #[test]
    fn test_delete_snippet() {
        let (mut manager, _temp) = create_test_manager();

        let id = manager
            .create_snippet("List files".to_string(), "ls -la".to_string())
            .unwrap();

        assert_eq!(manager.snippet_count(), 1);
        manager.delete_snippet(id).unwrap();
        assert_eq!(manager.snippet_count(), 0);
        assert!(manager.get_snippet(id).is_none());
    }

    #[test]
    fn test_get_by_category() {
        let (mut manager, _temp) = create_test_manager();

        let snippet1 = Snippet::new("List files".to_string(), "ls -la".to_string())
            .with_category("filesystem");
        let snippet2 =
            Snippet::new("Disk usage".to_string(), "df -h".to_string()).with_category("filesystem");
        let snippet3 = Snippet::new("Network info".to_string(), "ip addr".to_string())
            .with_category("network");

        manager.create_snippet_from(snippet1).unwrap();
        manager.create_snippet_from(snippet2).unwrap();
        manager.create_snippet_from(snippet3).unwrap();

        let fs_snippets = manager.get_by_category("filesystem");
        assert_eq!(fs_snippets.len(), 2);

        let net_snippets = manager.get_by_category("network");
        assert_eq!(net_snippets.len(), 1);
    }

    #[test]
    fn test_filter_by_tag() {
        let (mut manager, _temp) = create_test_manager();

        let snippet1 = Snippet::new("List files".to_string(), "ls -la".to_string())
            .with_tags(vec!["common".to_string(), "filesystem".to_string()]);
        let snippet2 = Snippet::new("Disk usage".to_string(), "df -h".to_string())
            .with_tags(vec!["common".to_string()]);

        manager.create_snippet_from(snippet1).unwrap();
        manager.create_snippet_from(snippet2).unwrap();

        let common_snippets = manager.filter_by_tag("common");
        assert_eq!(common_snippets.len(), 2);

        let fs_snippets = manager.filter_by_tag("filesystem");
        assert_eq!(fs_snippets.len(), 1);
    }

    #[test]
    fn test_search_by_name() {
        let (mut manager, _temp) = create_test_manager();

        manager
            .create_snippet("List files".to_string(), "ls -la".to_string())
            .unwrap();
        manager
            .create_snippet("Disk usage".to_string(), "df -h".to_string())
            .unwrap();

        let results = manager.search("list");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "List files");
    }

    #[test]
    fn test_search_by_command() {
        let (mut manager, _temp) = create_test_manager();

        manager
            .create_snippet("List files".to_string(), "ls -la".to_string())
            .unwrap();
        manager
            .create_snippet("Disk usage".to_string(), "df -h".to_string())
            .unwrap();

        let results = manager.search("df");
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].name, "Disk usage");
    }

    #[test]
    fn test_extract_variables_simple() {
        let command = "ssh ${user}@${host}";
        let vars = SnippetManager::extract_variables(command);

        assert_eq!(vars.len(), 2);
        assert!(vars.contains(&"host".to_string()));
        assert!(vars.contains(&"user".to_string()));
    }

    #[test]
    fn test_extract_variables_with_underscores() {
        let command = "curl -u ${api_user}:${api_key} ${base_url}/endpoint";
        let vars = SnippetManager::extract_variables(command);

        assert_eq!(vars.len(), 3);
        assert!(vars.contains(&"api_user".to_string()));
        assert!(vars.contains(&"api_key".to_string()));
        assert!(vars.contains(&"base_url".to_string()));
    }

    #[test]
    fn test_extract_variables_duplicates() {
        let command = "echo ${var} && echo ${var}";
        let vars = SnippetManager::extract_variables(command);

        assert_eq!(vars.len(), 1);
        assert!(vars.contains(&"var".to_string()));
    }

    #[test]
    fn test_extract_variables_no_variables() {
        let command = "ls -la";
        let vars = SnippetManager::extract_variables(command);

        assert!(vars.is_empty());
    }

    #[test]
    fn test_extract_variables_invalid_format() {
        // These should NOT be extracted as variables
        let command = "$var ${} ${123} ${}";
        let vars = SnippetManager::extract_variables(command);

        assert!(vars.is_empty());
    }

    #[test]
    fn test_substitute_variables() {
        let command = "ssh ${user}@${host}";
        let mut values = HashMap::new();
        values.insert("user".to_string(), "admin".to_string());
        values.insert("host".to_string(), "example.com".to_string());

        let result = SnippetManager::substitute_variables(command, &values);
        assert_eq!(result, "ssh admin@example.com");
    }

    #[test]
    fn test_substitute_variables_partial() {
        let command = "ssh ${user}@${host}";
        let mut values = HashMap::new();
        values.insert("user".to_string(), "admin".to_string());
        // host is not provided

        let result = SnippetManager::substitute_variables(command, &values);
        assert_eq!(result, "ssh admin@${host}");
    }

    #[test]
    fn test_substitute_with_defaults() {
        let snippet = Snippet::new(
            "SSH".to_string(),
            "ssh ${user}@${host} -p ${port}".to_string(),
        )
        .with_variables(vec![
            SnippetVariable::new("user".to_string()),
            SnippetVariable::new("host".to_string()),
            SnippetVariable::new("port".to_string()).with_default("22"),
        ]);

        let mut values = HashMap::new();
        values.insert("user".to_string(), "admin".to_string());
        values.insert("host".to_string(), "example.com".to_string());
        // port uses default

        let result = SnippetManager::substitute_with_defaults(&snippet, &values);
        assert_eq!(result, "ssh admin@example.com -p 22");
    }

    #[test]
    fn test_get_missing_variables() {
        let snippet = Snippet::new(
            "SSH".to_string(),
            "ssh ${user}@${host} -p ${port}".to_string(),
        )
        .with_variables(vec![
            SnippetVariable::new("user".to_string()),
            SnippetVariable::new("host".to_string()),
            SnippetVariable::new("port".to_string()).with_default("22"),
        ]);

        let mut values = HashMap::new();
        values.insert("user".to_string(), "admin".to_string());
        // host and port are not provided, but port has a default

        let missing = SnippetManager::get_missing_variables(&snippet, &values);
        assert_eq!(missing.len(), 1);
        assert!(missing.contains(&"host".to_string()));
    }

    #[test]
    fn test_get_all_categories() {
        let (mut manager, _temp) = create_test_manager();

        let snippet1 = Snippet::new("s1".to_string(), "cmd1".to_string()).with_category("cat_a");
        let snippet2 = Snippet::new("s2".to_string(), "cmd2".to_string()).with_category("cat_b");
        let snippet3 = Snippet::new("s3".to_string(), "cmd3".to_string()).with_category("cat_a");

        manager.create_snippet_from(snippet1).unwrap();
        manager.create_snippet_from(snippet2).unwrap();
        manager.create_snippet_from(snippet3).unwrap();

        let categories = manager.get_all_categories();
        assert_eq!(categories.len(), 2);
        assert!(categories.contains(&"cat_a".to_string()));
        assert!(categories.contains(&"cat_b".to_string()));
    }

    #[test]
    fn test_get_all_tags() {
        let (mut manager, _temp) = create_test_manager();

        let snippet1 =
            Snippet::new("s1".to_string(), "cmd1".to_string()).with_tags(vec!["tag1".to_string()]);
        let snippet2 = Snippet::new("s2".to_string(), "cmd2".to_string())
            .with_tags(vec!["tag1".to_string(), "tag2".to_string()]);

        manager.create_snippet_from(snippet1).unwrap();
        manager.create_snippet_from(snippet2).unwrap();

        let tags = manager.get_all_tags();
        assert_eq!(tags.len(), 2);
        assert!(tags.contains(&"tag1".to_string()));
        assert!(tags.contains(&"tag2".to_string()));
    }
}
