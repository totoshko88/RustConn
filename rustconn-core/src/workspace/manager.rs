//! Workspace profile manager for CRUD operations
//!
//! Provides in-memory storage with TOML persistence through `ConfigManager`.

use std::collections::HashMap;

use uuid::Uuid;

use crate::config::ConfigManager;
use crate::error::{ConfigError, ConfigResult};
use crate::models::WorkspaceProfile;

/// Manages workspace profile CRUD operations
///
/// Workspace profiles are named sets of connections that can be opened
/// together, restoring a user's working context in one action.
#[derive(Debug)]
pub struct WorkspaceProfileManager {
    /// In-memory storage indexed by ID
    profiles: HashMap<Uuid, WorkspaceProfile>,
    /// Configuration manager for persistence
    config_manager: ConfigManager,
}

impl WorkspaceProfileManager {
    /// Creates a new manager, loading existing profiles from storage
    ///
    /// # Errors
    ///
    /// Returns an error if loading from storage fails.
    pub fn new(config_manager: ConfigManager) -> ConfigResult<Self> {
        let profiles_vec = config_manager.load_workspace_profiles()?;
        let profiles = profiles_vec.into_iter().map(|p| (p.id, p)).collect();
        Ok(Self {
            profiles,
            config_manager,
        })
    }

    /// Creates a new manager with empty storage (for fallback/testing)
    #[must_use]
    pub fn new_empty(config_manager: ConfigManager) -> Self {
        Self {
            profiles: HashMap::new(),
            config_manager,
        }
    }

    // ========== CRUD ==========

    /// Creates a new workspace profile
    ///
    /// # Errors
    ///
    /// Returns an error if a profile with the same name already exists
    /// or persistence fails.
    pub fn create(&mut self, profile: WorkspaceProfile) -> ConfigResult<Uuid> {
        // Check for duplicate name
        if self.find_by_name(&profile.name).is_some() {
            return Err(ConfigError::Validation {
                field: "name".to_string(),
                reason: format!("Workspace '{}' already exists", profile.name),
            });
        }
        let id = profile.id;
        self.profiles.insert(id, profile);
        self.persist()?;
        Ok(id)
    }

    /// Updates an existing workspace profile
    ///
    /// # Errors
    ///
    /// Returns an error if the profile doesn't exist or persistence fails.
    pub fn update(&mut self, id: Uuid, mut updated: WorkspaceProfile) -> ConfigResult<()> {
        if !self.profiles.contains_key(&id) {
            return Err(ConfigError::Validation {
                field: "id".to_string(),
                reason: format!("Workspace profile with ID {id} not found"),
            });
        }
        // Check for name conflict with another profile
        if let Some(existing) = self.find_by_name(&updated.name)
            && existing.id != id
        {
            return Err(ConfigError::Validation {
                field: "name".to_string(),
                reason: format!("Workspace '{}' already exists", updated.name),
            });
        }
        updated.id = id;
        if let Some(existing) = self.profiles.get(&id) {
            updated.created_at = existing.created_at;
        }
        updated.touch();
        self.profiles.insert(id, updated);
        self.persist()?;
        Ok(())
    }

    /// Deletes a workspace profile by ID
    ///
    /// # Errors
    ///
    /// Returns an error if the profile doesn't exist or persistence fails.
    pub fn delete(&mut self, id: Uuid) -> ConfigResult<()> {
        if self.profiles.remove(&id).is_none() {
            return Err(ConfigError::Validation {
                field: "id".to_string(),
                reason: format!("Workspace profile with ID {id} not found"),
            });
        }
        self.persist()?;
        Ok(())
    }

    /// Renames a workspace profile
    ///
    /// # Errors
    ///
    /// Returns an error if the profile doesn't exist, the new name conflicts
    /// with another profile, or persistence fails.
    pub fn rename(&mut self, id: Uuid, new_name: String) -> ConfigResult<()> {
        // Check for name conflict with another profile
        if let Some(existing) = self.find_by_name(&new_name)
            && existing.id != id
        {
            return Err(ConfigError::Validation {
                field: "name".to_string(),
                reason: format!("Workspace '{}' already exists", new_name),
            });
        }
        let profile = self
            .profiles
            .get_mut(&id)
            .ok_or_else(|| ConfigError::Validation {
                field: "id".to_string(),
                reason: format!("Workspace profile with ID {id} not found"),
            })?;
        profile.name = new_name;
        profile.touch();
        self.persist()?;
        Ok(())
    }

    /// Gets a workspace profile by ID
    #[must_use]
    pub fn get(&self, id: Uuid) -> Option<&WorkspaceProfile> {
        self.profiles.get(&id)
    }

    /// Gets a mutable reference to a workspace profile
    #[must_use]
    pub fn get_mut(&mut self, id: Uuid) -> Option<&mut WorkspaceProfile> {
        self.profiles.get_mut(&id)
    }

    /// Lists all workspace profiles, sorted by sort_order then name
    #[must_use]
    pub fn list(&self) -> Vec<&WorkspaceProfile> {
        let mut profiles: Vec<&WorkspaceProfile> = self.profiles.values().collect();
        profiles.sort_by(|a, b| a.sort_order.cmp(&b.sort_order).then(a.name.cmp(&b.name)));
        profiles
    }

    /// Returns the total number of workspace profiles
    #[must_use]
    pub fn count(&self) -> usize {
        self.profiles.len()
    }

    // ========== Search ==========

    /// Finds a workspace profile by name (case-insensitive)
    #[must_use]
    pub fn find_by_name(&self, name: &str) -> Option<&WorkspaceProfile> {
        self.profiles
            .values()
            .find(|p| p.name.eq_ignore_ascii_case(name))
    }

    // ========== Connection cleanup ==========

    /// Removes a deleted connection from all workspace profiles that reference it
    ///
    /// # Errors
    ///
    /// Returns an error if persistence fails.
    pub fn on_connection_deleted(&mut self, connection_id: Uuid) -> ConfigResult<()> {
        let mut changed = false;
        for profile in self.profiles.values_mut() {
            if profile.remove_connection(connection_id) > 0 {
                changed = true;
            }
        }
        if changed {
            self.persist()?;
        }
        Ok(())
    }

    // ========== Persistence ==========

    /// Persists all profiles to storage
    fn persist(&self) -> ConfigResult<()> {
        let profiles: Vec<WorkspaceProfile> = self.profiles.values().cloned().collect();
        self.config_manager.save_workspace_profiles(&profiles)
    }

    /// Saves after external mutation (e.g. after `get_mut()` + modify)
    ///
    /// # Errors
    ///
    /// Returns an error if persistence fails.
    pub fn save(&self) -> ConfigResult<()> {
        self.persist()
    }

    /// Reloads profiles from storage
    ///
    /// # Errors
    ///
    /// Returns an error if loading fails.
    pub fn reload(&mut self) -> ConfigResult<()> {
        let profiles_vec = self.config_manager.load_workspace_profiles()?;
        self.profiles = profiles_vec.into_iter().map(|p| (p.id, p)).collect();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;
    use crate::models::{WorkspaceEntry, WorkspaceProfile};
    use crate::session::SessionType;

    fn create_test_manager() -> (WorkspaceProfileManager, TempDir) {
        let temp_dir = TempDir::new().unwrap();
        let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());
        let manager = WorkspaceProfileManager::new_empty(config_manager);
        (manager, temp_dir)
    }

    #[test]
    fn test_create_workspace() {
        let (mut mgr, _tmp) = create_test_manager();
        let ws = WorkspaceProfile::new("Production");
        let id = mgr.create(ws).unwrap();
        assert_eq!(mgr.count(), 1);
        assert_eq!(mgr.get(id).unwrap().name, "Production");
    }

    #[test]
    fn test_duplicate_name_rejected() {
        let (mut mgr, _tmp) = create_test_manager();
        mgr.create(WorkspaceProfile::new("Test")).unwrap();
        let result = mgr.create(WorkspaceProfile::new("Test"));
        assert!(result.is_err());
    }

    #[test]
    fn test_update_workspace() {
        let (mut mgr, _tmp) = create_test_manager();
        let ws = WorkspaceProfile::new("Old Name");
        let id = mgr.create(ws).unwrap();

        let mut updated = mgr.get(id).unwrap().clone();
        updated.name = "New Name".to_string();
        mgr.update(id, updated).unwrap();

        assert_eq!(mgr.get(id).unwrap().name, "New Name");
    }

    #[test]
    fn test_delete_workspace() {
        let (mut mgr, _tmp) = create_test_manager();
        let id = mgr.create(WorkspaceProfile::new("ToDelete")).unwrap();
        mgr.delete(id).unwrap();
        assert_eq!(mgr.count(), 0);
        assert!(mgr.get(id).is_none());
    }

    #[test]
    fn test_list_sorted() {
        let (mut mgr, _tmp) = create_test_manager();
        let mut ws_b = WorkspaceProfile::new("Beta");
        ws_b.sort_order = 2;
        let mut ws_a = WorkspaceProfile::new("Alpha");
        ws_a.sort_order = 1;
        mgr.create(ws_b).unwrap();
        mgr.create(ws_a).unwrap();

        let list = mgr.list();
        assert_eq!(list[0].name, "Alpha");
        assert_eq!(list[1].name, "Beta");
    }

    #[test]
    fn test_on_connection_deleted() {
        let (mut mgr, _tmp) = create_test_manager();
        let conn_id = Uuid::new_v4();
        let mut ws = WorkspaceProfile::new("Workspace");
        ws.add_entry(WorkspaceEntry::new(
            conn_id,
            "server".to_string(),
            "ssh".to_string(),
            SessionType::Embedded,
            0,
        ));
        let id = mgr.create(ws).unwrap();

        mgr.on_connection_deleted(conn_id).unwrap();
        assert!(mgr.get(id).unwrap().is_empty());
    }

    #[test]
    fn test_find_by_name() {
        let (mut mgr, _tmp) = create_test_manager();
        mgr.create(WorkspaceProfile::new("MyWorkspace")).unwrap();
        assert!(mgr.find_by_name("myworkspace").is_some());
        assert!(mgr.find_by_name("nonexistent").is_none());
    }

    #[test]
    fn test_persistence_roundtrip() {
        let temp_dir = TempDir::new().unwrap();
        let config_manager = ConfigManager::with_config_dir(temp_dir.path().to_path_buf());

        // Create and persist
        {
            let mut mgr = WorkspaceProfileManager::new_empty(config_manager.clone());
            let mut ws = WorkspaceProfile::new("Persistent");
            ws.add_entry(WorkspaceEntry::new(
                Uuid::new_v4(),
                "host".to_string(),
                "ssh".to_string(),
                SessionType::Embedded,
                0,
            ));
            mgr.create(ws).unwrap();
        }

        // Reload and verify
        let mgr = WorkspaceProfileManager::new(config_manager).unwrap();
        assert_eq!(mgr.count(), 1);
        assert_eq!(mgr.list()[0].name, "Persistent");
        assert_eq!(mgr.list()[0].entry_count(), 1);
    }
}
