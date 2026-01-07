//! Application state management
//!
//! This module provides the central application state that holds all managers
//! and provides thread-safe access to core functionality.

use chrono::Utc;
use rustconn_core::models::PasswordSource;
use rustconn_core::models::{ConnectionHistoryEntry, ConnectionStatistics};
use rustconn_core::{
    AppSettings, AsyncCredentialResolver, AsyncCredentialResult, CancellationToken, Cluster,
    ClusterManager, ConfigManager, Connection, ConnectionGroup, ConnectionManager,
    CredentialResolver, CredentialVerificationManager, Credentials, Document, DocumentManager,
    ImportResult, SecretManager, Session, SessionManager, Snippet, SnippetManager,
};
use secrecy::SecretString;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

// Thread-local tokio runtime for blocking async operations
// This avoids creating a new runtime for each credential operation
thread_local! {
    static TOKIO_RUNTIME: RefCell<Option<tokio::runtime::Runtime>> = const { RefCell::new(None) };
}

/// Gets or creates the thread-local tokio runtime
fn with_runtime<F, R>(f: F) -> Result<R, String>
where
    F: FnOnce(&tokio::runtime::Runtime) -> R,
{
    TOKIO_RUNTIME.with(|rt| {
        let mut rt_ref = rt.borrow_mut();
        if rt_ref.is_none() {
            *rt_ref = Some(
                tokio::runtime::Runtime::new()
                    .map_err(|e| format!("Failed to create runtime: {e}"))?,
            );
        }
        Ok(f(rt_ref.as_ref().expect("runtime should be initialized")))
    })
}

/// Internal clipboard for connection copy/paste operations
///
/// Stores a copied connection and its source group for paste operations.
/// The clipboard is session-only and not persisted.
#[derive(Debug, Clone, Default)]
pub struct ConnectionClipboard {
    /// Copied connection data
    connection: Option<Connection>,
    /// Source group ID where the connection was copied from
    source_group: Option<Uuid>,
}

impl ConnectionClipboard {
    /// Creates a new empty clipboard
    #[must_use]
    pub const fn new() -> Self {
        Self {
            connection: None,
            source_group: None,
        }
    }

    /// Copies a connection to the clipboard
    ///
    /// # Arguments
    /// * `connection` - The connection to copy
    /// * `group_id` - The source group ID (if any)
    pub fn copy(&mut self, connection: &Connection, group_id: Option<Uuid>) {
        self.connection = Some(connection.clone());
        self.source_group = group_id;
    }

    /// Pastes the connection from the clipboard, creating a duplicate
    ///
    /// Returns a new connection with:
    /// - A new unique ID
    /// - "(Copy)" suffix appended to the name
    /// - Updated timestamps
    ///
    /// # Returns
    /// `Some(Connection)` if there's content in the clipboard, `None` otherwise
    #[must_use]
    pub fn paste(&self) -> Option<Connection> {
        self.connection.as_ref().map(|conn| {
            let mut new_conn = conn.clone();
            new_conn.id = Uuid::new_v4();
            new_conn.name = format!("{} (Copy)", conn.name);
            let now = Utc::now();
            new_conn.created_at = now;
            new_conn.updated_at = now;
            new_conn.last_connected = None;
            new_conn
        })
    }

    /// Checks if the clipboard has content
    #[must_use]
    pub const fn has_content(&self) -> bool {
        self.connection.is_some()
    }

    /// Gets the source group ID where the connection was copied from
    #[must_use]
    pub const fn source_group(&self) -> Option<Uuid> {
        self.source_group
    }

    /// Clears the clipboard
    #[allow(dead_code)] // May be used in future for clipboard management
    pub fn clear(&mut self) {
        self.connection = None;
        self.source_group = None;
    }
}

/// Cached credentials for a connection (session-only, not persisted)
#[derive(Clone)]
pub struct CachedCredentials {
    /// Username
    pub username: String,
    /// Password (stored securely in memory)
    pub password: SecretString,
    /// Domain for Windows authentication
    pub domain: String,
}

/// Application state holding all managers
///
/// This struct provides centralized access to all core functionality
/// and is shared across the application using Rc<`RefCell`<>>.
pub struct AppState {
    /// Connection manager for CRUD operations
    connection_manager: ConnectionManager,
    /// Session manager for active connections
    session_manager: SessionManager,
    /// Snippet manager for command snippets
    snippet_manager: SnippetManager,
    /// Secret manager for credentials
    secret_manager: SecretManager,
    /// Configuration manager for persistence
    config_manager: ConfigManager,
    /// Document manager for multi-document support
    document_manager: DocumentManager,
    /// Cluster manager for connection clusters
    cluster_manager: ClusterManager,
    /// Credential verification manager for tracking verified credentials
    verification_manager: CredentialVerificationManager,
    /// Currently active document ID
    active_document_id: Option<Uuid>,
    /// Application settings
    settings: AppSettings,
    /// Session-level password cache (cleared on app exit)
    password_cache: HashMap<Uuid, CachedCredentials>,
    /// Connection clipboard for copy/paste operations
    clipboard: ConnectionClipboard,
    /// Connection history entries
    history_entries: Vec<ConnectionHistoryEntry>,
}

impl AppState {
    /// Creates a new application state
    ///
    /// Initializes all managers and loads configuration from disk.
    ///
    /// # Errors
    ///
    /// Returns an error if initialization fails.
    pub fn new() -> Result<Self, String> {
        // Initialize config manager
        let config_manager = ConfigManager::new()
            .map_err(|e| format!("Failed to initialize config manager: {e}"))?;

        // Load settings
        let mut settings = config_manager
            .load_settings()
            .unwrap_or_else(|_| AppSettings::default());

        // Validate KDBX integration at startup
        if settings.secrets.kdbx_enabled {
            let mut disable_integration = false;

            // Check if KDBX file exists
            if let Some(ref kdbx_path) = settings.secrets.kdbx_path {
                if !kdbx_path.exists() {
                    eprintln!(
                        "KeePass database file not found: {}. Disabling integration.",
                        kdbx_path.display()
                    );
                    disable_integration = true;
                }
            } else {
                eprintln!(
                    "KeePass integration enabled but no database path configured. Disabling."
                );
                disable_integration = true;
            }

            if disable_integration {
                settings.secrets.kdbx_enabled = false;
                settings.secrets.clear_password();
                // Save updated settings
                if let Err(e) = config_manager.save_settings(&settings) {
                    eprintln!("Failed to save settings after disabling KDBX: {e}");
                }
            } else {
                // Try to decrypt stored password
                if settings.secrets.decrypt_password() {
                    eprintln!("KeePass password restored from encrypted storage");
                }
            }
        }

        // Initialize connection manager
        let connection_manager = ConnectionManager::new(config_manager.clone())
            .map_err(|e| format!("Failed to initialize connection manager: {e}"))?;

        // Initialize session manager with logging if enabled
        let session_manager = if settings.logging.enabled {
            let log_dir = if settings.logging.log_directory.is_absolute() {
                settings.logging.log_directory.clone()
            } else {
                config_manager
                    .config_dir()
                    .join(&settings.logging.log_directory)
            };
            SessionManager::with_logging(&log_dir).unwrap_or_else(|_| SessionManager::new())
        } else {
            SessionManager::new()
        };

        // Initialize snippet manager
        let snippet_manager = SnippetManager::new(config_manager.clone())
            .map_err(|e| format!("Failed to initialize snippet manager: {e}"))?;

        // Initialize secret manager (empty for now, backends added later)
        let secret_manager = SecretManager::empty();

        // Initialize document manager
        let document_manager = DocumentManager::new();

        // Initialize cluster manager and load clusters
        let mut cluster_manager = ClusterManager::new();
        if let Ok(clusters) = config_manager.load_clusters() {
            cluster_manager.load_clusters(clusters);
        }

        // Load connection history
        let history_entries = config_manager.load_history().unwrap_or_default();

        Ok(Self {
            connection_manager,
            session_manager,
            snippet_manager,
            secret_manager,
            config_manager,
            document_manager,
            cluster_manager,
            verification_manager: CredentialVerificationManager::new(),
            active_document_id: None,
            settings,
            password_cache: HashMap::new(),
            clipboard: ConnectionClipboard::new(),
            history_entries,
        })
    }

    // ========== Password Cache Operations ==========

    /// Caches credentials for a connection (session-only)
    pub fn cache_credentials(
        &mut self,
        connection_id: Uuid,
        username: &str,
        password: &str,
        domain: &str,
    ) {
        self.password_cache.insert(
            connection_id,
            CachedCredentials {
                username: username.to_string(),
                password: SecretString::from(password.to_string()),
                domain: domain.to_string(),
            },
        );
    }

    /// Gets cached credentials for a connection
    pub fn get_cached_credentials(&self, connection_id: Uuid) -> Option<&CachedCredentials> {
        self.password_cache.get(&connection_id)
    }

    /// Checks if credentials are cached for a connection
    ///
    /// Note: Part of credential caching API.
    #[allow(dead_code)]
    pub fn has_cached_credentials(&self, connection_id: Uuid) -> bool {
        self.password_cache.contains_key(&connection_id)
    }

    /// Clears cached credentials for a connection
    ///
    /// Note: Part of credential caching API.
    #[allow(dead_code)]
    pub fn clear_cached_credentials(&mut self, connection_id: Uuid) {
        self.password_cache.remove(&connection_id);
    }

    /// Clears all cached credentials
    ///
    /// Note: Part of credential caching API.
    #[allow(dead_code)]
    pub fn clear_all_cached_credentials(&mut self) {
        self.password_cache.clear();
    }

    // ========== Credential Verification Operations ==========

    /// Marks credentials as verified for a connection after successful authentication
    ///
    /// Note: Part of credential verification API.
    #[allow(dead_code)]
    pub fn mark_credentials_verified(&mut self, connection_id: Uuid) {
        self.verification_manager.mark_verified(connection_id);
    }

    /// Marks credentials as unverified for a connection after failed authentication
    ///
    /// Note: Part of credential verification API.
    #[allow(dead_code)]
    pub fn mark_credentials_unverified(&mut self, connection_id: Uuid, error: Option<String>) {
        self.verification_manager
            .mark_unverified(connection_id, error);
    }

    /// Checks if credentials are verified for a connection
    ///
    /// Note: Part of credential verification API.
    #[allow(dead_code)]
    pub fn are_credentials_verified(&self, connection_id: Uuid) -> bool {
        self.verification_manager.is_verified(connection_id)
    }

    /// Checks if we can skip the password dialog for a connection
    ///
    /// Returns true if:
    /// - Credentials are verified (previously successful auth)
    /// - AND we have cached credentials
    ///
    /// Note: This method only checks the in-memory cache to avoid blocking the UI.
    /// KeePass credential resolution is done asynchronously when needed.
    pub fn can_skip_password_dialog(&self, connection_id: Uuid) -> bool {
        if !self.verification_manager.is_verified(connection_id) {
            return false;
        }

        // Check if we have cached credentials (fast, non-blocking)
        self.password_cache.contains_key(&connection_id)
    }

    // ========== Connection Operations ==========

    /// Creates a new connection
    ///
    /// If a connection with the same name already exists, automatically generates
    /// a unique name by appending the protocol suffix (e.g., "server (RDP)").
    pub fn create_connection(&mut self, mut connection: Connection) -> Result<Uuid, String> {
        // Auto-generate unique name if duplicate exists (Bug 4 fix)
        if self.connection_exists_by_name(&connection.name) {
            let protocol_type = connection.protocol_config.protocol_type();
            connection.name = self.generate_unique_connection_name(&connection.name, protocol_type);
        }

        self.connection_manager
            .create_connection_from(connection)
            .map_err(|e| format!("Failed to create connection: {e}"))
    }

    /// Checks if a connection with the given name exists
    pub fn connection_exists_by_name(&self, name: &str) -> bool {
        self.connection_manager
            .list_connections()
            .iter()
            .any(|c| c.name.eq_ignore_ascii_case(name))
    }

    /// Checks if a group with the given name exists
    pub fn group_exists_by_name(&self, name: &str) -> bool {
        self.connection_manager
            .list_groups()
            .iter()
            .any(|g| g.name.eq_ignore_ascii_case(name))
    }

    /// Generates a unique name by appending a protocol suffix and/or number if needed
    ///
    /// Uses the `ConnectionManager::generate_unique_name` method which follows the pattern:
    /// 1. If base name is unique, return it as-is
    /// 2. If duplicate, append protocol suffix (e.g., "server (RDP)")
    /// 3. If still duplicate, append numeric suffix (e.g., "server (RDP) 2")
    pub fn generate_unique_connection_name(
        &self,
        base_name: &str,
        protocol: rustconn_core::ProtocolType,
    ) -> String {
        self.connection_manager
            .generate_unique_name(base_name, protocol)
    }

    /// Generates a unique name without protocol suffix (legacy method for backward compatibility)
    ///
    /// This method is kept for cases where protocol is not known or not relevant.
    ///
    /// Note: Part of connection naming API.
    #[allow(dead_code)]
    pub fn generate_unique_connection_name_simple(&self, base_name: &str) -> String {
        if !self.connection_exists_by_name(base_name) {
            return base_name.to_string();
        }

        let mut counter = 1;
        loop {
            let new_name = format!("{base_name} ({counter})");
            if !self.connection_exists_by_name(&new_name) {
                return new_name;
            }
            counter += 1;
        }
    }

    /// Normalizes a connection name by removing auto-generated suffixes if the base name is now unique
    ///
    /// This should be called when renaming a connection to potentially simplify the name.
    ///
    /// Note: Part of connection naming API.
    #[allow(dead_code)]
    pub fn normalize_connection_name(&self, name: &str, connection_id: Uuid) -> String {
        self.connection_manager.normalize_name(name, connection_id)
    }

    /// Generates a unique group name by appending a number if needed
    pub fn generate_unique_group_name(&self, base_name: &str) -> String {
        if !self.group_exists_by_name(base_name) {
            return base_name.to_string();
        }

        let mut counter = 1;
        loop {
            let new_name = format!("{base_name} ({counter})");
            if !self.group_exists_by_name(&new_name) {
                return new_name;
            }
            counter += 1;
        }
    }

    /// Updates an existing connection
    pub fn update_connection(&mut self, id: Uuid, connection: Connection) -> Result<(), String> {
        self.connection_manager
            .update_connection(id, connection)
            .map_err(|e| format!("Failed to update connection: {e}"))
    }

    /// Deletes a connection
    pub fn delete_connection(&mut self, id: Uuid) -> Result<(), String> {
        self.connection_manager
            .delete_connection(id)
            .map_err(|e| format!("Failed to delete connection: {e}"))
    }

    /// Gets a connection by ID
    pub fn get_connection(&self, id: Uuid) -> Option<&Connection> {
        self.connection_manager.get_connection(id)
    }

    /// Lists all connections
    pub fn list_connections(&self) -> Vec<&Connection> {
        self.connection_manager.list_connections()
    }

    /// Searches connections
    ///
    /// Note: Part of connection search API.
    #[allow(dead_code)]
    pub fn search_connections(&self, query: &str) -> Vec<&Connection> {
        self.connection_manager.search(query)
    }

    /// Gets connections by group
    pub fn get_connections_by_group(&self, group_id: Uuid) -> Vec<&Connection> {
        self.connection_manager.get_by_group(group_id)
    }

    /// Gets ungrouped connections
    pub fn get_ungrouped_connections(&self) -> Vec<&Connection> {
        self.connection_manager.get_ungrouped()
    }

    // ========== Group Operations ==========

    /// Creates a new group
    pub fn create_group(&mut self, name: String) -> Result<Uuid, String> {
        // Check for duplicate name
        if self.group_exists_by_name(&name) {
            return Err(format!("Group with name '{name}' already exists"));
        }

        self.connection_manager
            .create_group(name)
            .map_err(|e| format!("Failed to create group: {e}"))
    }

    /// Creates a group with a parent
    pub fn create_group_with_parent(
        &mut self,
        name: String,
        parent_id: Uuid,
    ) -> Result<Uuid, String> {
        self.connection_manager
            .create_group_with_parent(name, parent_id)
            .map_err(|e| format!("Failed to create group: {e}"))
    }

    /// Deletes a group (connections become ungrouped)
    pub fn delete_group(&mut self, id: Uuid) -> Result<(), String> {
        self.connection_manager
            .delete_group(id)
            .map_err(|e| format!("Failed to delete group: {e}"))
    }

    /// Deletes a group and all connections within it (cascade delete)
    pub fn delete_group_cascade(&mut self, id: Uuid) -> Result<(), String> {
        self.connection_manager
            .delete_group_cascade(id)
            .map_err(|e| format!("Failed to delete group: {e}"))
    }

    /// Moves a group to a new parent group
    pub fn move_group_to_parent(
        &mut self,
        group_id: Uuid,
        new_parent_id: Option<Uuid>,
    ) -> Result<(), String> {
        self.connection_manager
            .move_group(group_id, new_parent_id)
            .map_err(|e| format!("Failed to move group: {e}"))
    }

    /// Counts connections in a group (including child groups)
    pub fn count_connections_in_group(&self, group_id: Uuid) -> usize {
        self.connection_manager.count_connections_in_group(group_id)
    }

    /// Gets a group by ID
    pub fn get_group(&self, id: Uuid) -> Option<&ConnectionGroup> {
        self.connection_manager.get_group(id)
    }

    /// Lists all groups
    pub fn list_groups(&self) -> Vec<&ConnectionGroup> {
        self.connection_manager.list_groups()
    }

    /// Gets root-level groups
    pub fn get_root_groups(&self) -> Vec<&ConnectionGroup> {
        self.connection_manager.get_root_groups()
    }

    /// Gets child groups
    pub fn get_child_groups(&self, parent_id: Uuid) -> Vec<&ConnectionGroup> {
        self.connection_manager.get_child_groups(parent_id)
    }

    /// Moves a connection to a group
    pub fn move_connection_to_group(
        &mut self,
        connection_id: Uuid,
        group_id: Option<Uuid>,
    ) -> Result<(), String> {
        self.connection_manager
            .move_connection_to_group(connection_id, group_id)
            .map_err(|e| format!("Failed to move connection: {e}"))
    }

    /// Gets the group path
    pub fn get_group_path(&self, group_id: Uuid) -> Option<String> {
        self.connection_manager.get_group_path(group_id)
    }

    /// Updates the sort order of a connection
    ///
    /// Note: Part of connection ordering API.
    #[allow(dead_code)]
    pub fn update_connection_sort_order(
        &mut self,
        connection_id: Uuid,
        sort_order: i32,
    ) -> Result<(), String> {
        if let Some(conn) = self.connection_manager.get_connection(connection_id) {
            let mut updated = conn.clone();
            updated.sort_order = sort_order;
            self.connection_manager
                .update_connection(connection_id, updated)
                .map_err(|e| format!("Failed to update sort order: {e}"))
        } else {
            Err(format!("Connection not found: {connection_id}"))
        }
    }

    /// Updates the sort order of a group
    ///
    /// Note: Part of group ordering API.
    #[allow(dead_code)]
    pub fn update_group_sort_order(
        &mut self,
        group_id: Uuid,
        sort_order: i32,
    ) -> Result<(), String> {
        if let Some(group) = self.connection_manager.get_group(group_id) {
            let mut updated = group.clone();
            updated.sort_order = sort_order;
            self.connection_manager
                .update_group(group_id, updated)
                .map_err(|e| format!("Failed to update sort order: {e}"))
        } else {
            Err(format!("Group not found: {group_id}"))
        }
    }

    /// Reorders connections by updating their `sort_order` values
    ///
    /// Note: Part of connection ordering API.
    #[allow(dead_code)]
    pub fn reorder_connections(&mut self, connection_ids: &[Uuid]) -> Result<(), String> {
        for (index, &id) in connection_ids.iter().enumerate() {
            #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
            self.update_connection_sort_order(id, index as i32)?;
        }
        Ok(())
    }

    /// Sorts connections within a specific group alphabetically
    pub fn sort_group(&mut self, group_id: Uuid) -> Result<(), String> {
        self.connection_manager
            .sort_group(group_id)
            .map_err(|e| format!("Failed to sort group: {e}"))
    }

    /// Sorts all groups and connections alphabetically
    pub fn sort_all(&mut self) -> Result<(), String> {
        self.connection_manager
            .sort_all()
            .map_err(|e| format!("Failed to sort all: {e}"))
    }

    /// Reorders a connection to be positioned after another connection
    pub fn reorder_connection(
        &mut self,
        connection_id: Uuid,
        target_id: Uuid,
    ) -> Result<(), String> {
        self.connection_manager
            .reorder_connection(connection_id, target_id)
            .map_err(|e| format!("Failed to reorder connection: {e}"))
    }

    /// Reorders a group to be positioned after another group
    pub fn reorder_group(&mut self, group_id: Uuid, target_id: Uuid) -> Result<(), String> {
        self.connection_manager
            .reorder_group(group_id, target_id)
            .map_err(|e| format!("Failed to reorder group: {e}"))
    }

    /// Updates the `last_connected` timestamp for a connection
    pub fn update_last_connected(&mut self, connection_id: Uuid) -> Result<(), String> {
        self.connection_manager
            .update_last_connected(connection_id)
            .map_err(|e| format!("Failed to update last connected: {e}"))
    }

    /// Sorts all connections by `last_connected` timestamp (most recent first)
    pub fn sort_by_recent(&mut self) -> Result<(), String> {
        self.connection_manager
            .sort_by_recent()
            .map_err(|e| format!("Failed to sort by recent: {e}"))
    }

    // ========== Session Operations ==========

    /// Starts a session for a connection
    ///
    /// Note: Part of session management API.
    #[allow(dead_code)]
    pub fn start_session(
        &mut self,
        connection_id: Uuid,
        _credentials: Option<&Credentials>,
    ) -> Result<Uuid, String> {
        let connection = self
            .connection_manager
            .get_connection(connection_id)
            .ok_or_else(|| format!("Connection not found: {connection_id}"))?
            .clone();

        self.session_manager
            .start_session(&connection)
            .map_err(|e| format!("Failed to start session: {e}"))
    }

    /// Terminates a session
    pub fn terminate_session(&mut self, session_id: Uuid) -> Result<(), String> {
        self.session_manager
            .terminate_session(session_id)
            .map_err(|e| format!("Failed to terminate session: {e}"))
    }

    /// Gets a session by ID
    ///
    /// Note: Part of session management API.
    #[allow(dead_code)]
    pub fn get_session(&self, session_id: Uuid) -> Option<&Session> {
        self.session_manager.get_session(session_id)
    }

    /// Gets active sessions
    pub fn active_sessions(&self) -> Vec<&Session> {
        self.session_manager.active_sessions()
    }

    /// Gets the session manager (for building commands)
    ///
    /// Note: Part of session management API.
    #[allow(dead_code)]
    pub const fn session_manager(&self) -> &SessionManager {
        &self.session_manager
    }

    /// Gets mutable session manager
    ///
    /// Note: Part of session management API.
    #[allow(dead_code)]
    pub fn session_manager_mut(&mut self) -> &mut SessionManager {
        &mut self.session_manager
    }

    // ========== Snippet Operations ==========

    /// Creates a new snippet
    pub fn create_snippet(&mut self, snippet: Snippet) -> Result<Uuid, String> {
        self.snippet_manager
            .create_snippet_from(snippet)
            .map_err(|e| format!("Failed to create snippet: {e}"))
    }

    /// Updates a snippet
    pub fn update_snippet(&mut self, id: Uuid, snippet: Snippet) -> Result<(), String> {
        self.snippet_manager
            .update_snippet(id, snippet)
            .map_err(|e| format!("Failed to update snippet: {e}"))
    }

    /// Deletes a snippet
    pub fn delete_snippet(&mut self, id: Uuid) -> Result<(), String> {
        self.snippet_manager
            .delete_snippet(id)
            .map_err(|e| format!("Failed to delete snippet: {e}"))
    }

    /// Gets a snippet by ID
    pub fn get_snippet(&self, id: Uuid) -> Option<&Snippet> {
        self.snippet_manager.get_snippet(id)
    }

    /// Lists all snippets
    pub fn list_snippets(&self) -> Vec<&Snippet> {
        self.snippet_manager.list_snippets()
    }

    /// Searches snippets
    pub fn search_snippets(&self, query: &str) -> Vec<&Snippet> {
        self.snippet_manager.search(query)
    }

    // ========== Secret/Credential Operations ==========

    /// Gets a reference to the secret manager
    ///
    /// Note: Part of secret management API.
    #[allow(dead_code)]
    pub const fn secret_manager(&self) -> &SecretManager {
        &self.secret_manager
    }

    /// Gets a mutable reference to the secret manager
    ///
    /// Note: Part of secret management API.
    #[allow(dead_code)]
    pub fn secret_manager_mut(&mut self) -> &mut SecretManager {
        &mut self.secret_manager
    }

    /// Stores credentials for a connection (blocking wrapper for async operation)
    ///
    /// Note: This uses a cached tokio runtime to avoid creating a new one each time.
    /// For better performance in async contexts, use `secret_manager()` directly.
    ///
    /// Note: Part of secret management API.
    #[allow(dead_code)]
    pub fn store_credentials(
        &self,
        connection_id: Uuid,
        credentials: &Credentials,
    ) -> Result<(), String> {
        let secret_manager = self.secret_manager.clone();
        let id_str = connection_id.to_string();
        let creds = credentials.clone();

        with_runtime(|rt| {
            rt.block_on(async {
                secret_manager
                    .store(&id_str, &creds)
                    .await
                    .map_err(|e| format!("Failed to store credentials: {e}"))
            })
        })?
    }

    /// Retrieves credentials for a connection (blocking wrapper for async operation)
    ///
    /// Note: This uses a cached tokio runtime to avoid creating a new one each time.
    /// For better performance in async contexts, use `secret_manager()` directly.
    ///
    /// Note: Part of secret management API.
    #[allow(dead_code)]
    pub fn retrieve_credentials(&self, connection_id: Uuid) -> Result<Option<Credentials>, String> {
        let secret_manager = self.secret_manager.clone();
        let id_str = connection_id.to_string();

        with_runtime(|rt| {
            rt.block_on(async {
                secret_manager
                    .retrieve(&id_str)
                    .await
                    .map_err(|e| format!("Failed to retrieve credentials: {e}"))
            })
        })?
    }

    /// Deletes credentials for a connection (blocking wrapper for async operation)
    ///
    /// Note: This uses a cached tokio runtime to avoid creating a new one each time.
    /// For better performance in async contexts, use `secret_manager()` directly.
    ///
    /// Note: Part of secret management API.
    #[allow(dead_code)]
    pub fn delete_credentials(&self, connection_id: Uuid) -> Result<(), String> {
        let secret_manager = self.secret_manager.clone();
        let id_str = connection_id.to_string();

        with_runtime(|rt| {
            rt.block_on(async {
                secret_manager
                    .delete(&id_str)
                    .await
                    .map_err(|e| format!("Failed to delete credentials: {e}"))
            })
        })?
    }

    /// Checks if any secret backend is available (blocking wrapper)
    pub fn has_secret_backend(&self) -> bool {
        let secret_manager = self.secret_manager.clone();

        with_runtime(|rt| rt.block_on(async { secret_manager.is_available().await }))
            .unwrap_or(false)
    }

    /// Resolves credentials for a connection using the credential resolution chain
    ///
    /// This method implements the credential resolution flow based on the connection's
    /// `password_source` setting:
    /// - `PasswordSource::KeePass` - Try `KeePass` first, fallback if enabled
    /// - `PasswordSource::Keyring` - Try system keyring (libsecret)
    /// - `PasswordSource::Stored` - Return None (caller uses stored password)
    /// - `PasswordSource::Prompt` - Return None (caller prompts user)
    /// - `PasswordSource::None` - Try fallback chain if enabled
    ///
    /// # Returns
    /// - `Ok(Some(Credentials))` - Credentials found from a backend
    /// - `Ok(None)` - No credentials found, caller should prompt user or use stored
    /// - `Err(String)` - Error during resolution
    pub fn resolve_credentials(
        &self,
        connection: &Connection,
    ) -> Result<Option<Credentials>, String> {
        use rustconn_core::secret::KeePassStatus;
        use secrecy::ExposeSecret;

        // For KeePass password source, directly use KeePassStatus to retrieve password
        // This bypasses the SecretManager which requires registered backends
        if connection.password_source == PasswordSource::KeePass
            && self.settings.secrets.kdbx_enabled
        {
            if let Some(ref kdbx_path) = self.settings.secrets.kdbx_path {
                // Get the lookup key with protocol for uniqueness
                // Format: "name (protocol)" or "host (protocol)" if name is empty
                let protocol = connection.protocol_config.protocol_type();
                let protocol_str = protocol.as_str();
                let base_name = if connection.name.trim().is_empty() {
                    connection.host.clone()
                } else {
                    connection.name.clone()
                };
                let lookup_key = format!("{base_name} ({protocol_str})");

                // Get credentials - password and key file can be used together
                let db_password = self
                    .settings
                    .secrets
                    .kdbx_password
                    .as_ref()
                    .map(|p| p.expose_secret());

                let key_file = self.settings.secrets.kdbx_key_file.as_deref();

                tracing::debug!(
                    "[resolve_credentials] KeePass lookup: key='{}', has_password={}, has_key_file={}",
                    lookup_key,
                    db_password.is_some(),
                    key_file.is_some()
                );

                match KeePassStatus::get_password_from_kdbx_with_key(
                    kdbx_path,
                    db_password,
                    key_file,
                    &lookup_key,
                    None, // Protocol already included in lookup_key
                ) {
                    Ok(Some(password)) => {
                        tracing::debug!("[resolve_credentials] Found password in KeePass");
                        // Build credentials with optional username and password
                        let mut creds = if let Some(ref username) = connection.username {
                            Credentials::with_password(username, &password)
                        } else {
                            Credentials {
                                username: None,
                                password: Some(SecretString::from(password)),
                                key_passphrase: None,
                            }
                        };
                        // Preserve key_passphrase if needed
                        creds.key_passphrase = None;
                        return Ok(Some(creds));
                    }
                    Ok(None) => {
                        tracing::debug!("[resolve_credentials] No password found in KeePass");
                    }
                    Err(e) => {
                        tracing::error!("[resolve_credentials] KeePass error: {}", e);
                    }
                }
            }
        }

        // Fall back to the standard resolver for other password sources
        let secret_manager = self.secret_manager.clone();
        let resolver =
            CredentialResolver::new(Arc::new(secret_manager), self.settings.secrets.clone());
        let connection = connection.clone();

        with_runtime(|rt| {
            rt.block_on(async {
                resolver
                    .resolve(&connection)
                    .await
                    .map_err(|e| format!("Failed to resolve credentials: {e}"))
            })
        })?
    }

    /// Resolves credentials for a connection by ID
    ///
    /// Convenience method that looks up the connection and resolves credentials.
    pub fn resolve_credentials_for_connection(
        &self,
        connection_id: Uuid,
    ) -> Result<Option<Credentials>, String> {
        let connection = self
            .get_connection(connection_id)
            .ok_or_else(|| format!("Connection not found: {connection_id}"))?
            .clone();

        self.resolve_credentials(&connection)
    }

    /// Determines if credentials should be prompted for a connection
    ///
    /// Returns `true` if the connection's password source requires user input
    /// and no credentials are available from other sources.
    pub fn should_prompt_for_credentials(&self, connection: &Connection) -> bool {
        match connection.password_source {
            PasswordSource::Prompt => true,
            PasswordSource::Stored => false, // Use stored password
            PasswordSource::None => {
                // Check if fallback is enabled and backends are available
                !self.settings.secrets.enable_fallback || !self.has_secret_backend()
            }
            PasswordSource::KeePass => {
                // Prompt if KeePass is not enabled
                !self.settings.secrets.kdbx_enabled
            }
            PasswordSource::Keyring => {
                // Prompt if no backend available
                !self.has_secret_backend()
            }
        }
    }

    // ========== Async Credential Operations ==========

    /// Creates an async credential resolver for non-blocking credential resolution
    ///
    /// This method creates a resolver that can be used for async credential
    /// resolution without blocking the UI thread.
    ///
    /// # Returns
    /// An `AsyncCredentialResolver` configured with current settings
    #[must_use]
    pub fn create_async_resolver(&self) -> AsyncCredentialResolver {
        AsyncCredentialResolver::new(
            Arc::new(SecretManager::empty()),
            self.settings.secrets.clone(),
        )
    }

    /// Resolves credentials asynchronously without blocking the UI
    ///
    /// This method spawns an async task to resolve credentials and returns
    /// immediately. The result is delivered via the provided callback.
    ///
    /// # Arguments
    /// * `connection` - The connection to resolve credentials for
    /// * `callback` - Function called with the result when resolution completes
    ///
    /// # Returns
    /// A `CancellationToken` that can be used to cancel the operation
    ///
    /// # Requirements Coverage
    /// - Requirement 9.1: Async operations instead of blocking calls
    /// - Requirement 9.2: Avoid `block_on()` in GUI code
    ///
    /// Note: Part of async credential resolution API.
    #[allow(dead_code)]
    pub fn resolve_credentials_with_callback<F>(
        &self,
        connection: Connection,
        callback: F,
    ) -> CancellationToken
    where
        F: FnOnce(AsyncCredentialResult) + Send + 'static,
    {
        let resolver = Arc::new(self.create_async_resolver());
        rustconn_core::resolve_with_callback(resolver, connection, callback)
    }

    /// Resolves credentials asynchronously with timeout
    ///
    /// This method spawns an async task to resolve credentials with a timeout.
    /// The result is delivered via the provided callback.
    ///
    /// # Arguments
    /// * `connection` - The connection to resolve credentials for
    /// * `timeout` - Maximum time to wait for resolution
    /// * `callback` - Function called with the result when resolution completes
    ///
    /// # Returns
    /// A `CancellationToken` that can be used to cancel the operation
    ///
    /// # Requirements Coverage
    /// - Requirement 9.1: Async operations instead of blocking calls
    /// - Requirement 9.5: Support cancellation of pending requests
    ///
    /// Note: Part of async credential resolution API.
    #[allow(dead_code)]
    pub fn resolve_credentials_with_timeout<F>(
        &self,
        connection: Connection,
        timeout: Duration,
        callback: F,
    ) -> CancellationToken
    where
        F: FnOnce(AsyncCredentialResult) + Send + 'static,
    {
        let resolver = Arc::new(self.create_async_resolver());
        let cancel_token = CancellationToken::new();
        let token_clone = cancel_token.clone();

        tokio::spawn(async move {
            let result = resolver
                .resolve_with_cancellation_and_timeout(&connection, &token_clone, timeout)
                .await;
            callback(result);
        });

        cancel_token
    }

    /// Resolves credentials asynchronously and returns a future
    ///
    /// This method is for use in async contexts where you want to await
    /// the result directly rather than using a callback.
    ///
    /// # Arguments
    /// * `connection` - The connection to resolve credentials for
    ///
    /// # Returns
    /// A `PendingCredentialResolution` that can be awaited or cancelled
    ///
    /// # Requirements Coverage
    /// - Requirement 9.1: Async operations instead of blocking calls
    /// - Requirement 9.5: Support cancellation of pending requests
    ///
    /// Note: Part of async credential resolution API.
    #[allow(dead_code)]
    pub fn resolve_credentials_async(
        &self,
        connection: Connection,
    ) -> rustconn_core::PendingCredentialResolution {
        let resolver = Arc::new(self.create_async_resolver());
        rustconn_core::spawn_credential_resolution(resolver, connection, None)
    }

    /// Resolves credentials asynchronously with timeout and returns a future
    ///
    /// # Arguments
    /// * `connection` - The connection to resolve credentials for
    /// * `timeout` - Maximum time to wait for resolution
    ///
    /// # Returns
    /// A `PendingCredentialResolution` that can be awaited or cancelled
    ///
    /// Note: Part of async credential resolution API.
    #[allow(dead_code)]
    pub fn resolve_credentials_async_with_timeout(
        &self,
        connection: Connection,
        timeout: Duration,
    ) -> rustconn_core::PendingCredentialResolution {
        let resolver = Arc::new(self.create_async_resolver());
        rustconn_core::spawn_credential_resolution(resolver, connection, Some(timeout))
    }

    /// Checks if `KeePass` integration is currently active
    ///
    /// Note: Part of KeePass integration API.
    #[allow(dead_code)]
    pub const fn is_keepass_active(&self) -> bool {
        self.settings.secrets.kdbx_enabled && self.settings.secrets.kdbx_path.is_some()
    }

    // ========== Settings Operations ==========

    /// Gets the current settings
    pub const fn settings(&self) -> &AppSettings {
        &self.settings
    }

    /// Updates and saves settings
    pub fn update_settings(&mut self, mut settings: AppSettings) -> Result<(), String> {
        // Encrypt KDBX password before saving if integration is enabled
        if settings.secrets.kdbx_enabled && settings.secrets.kdbx_password.is_some() {
            settings.secrets.encrypt_password();
        } else if !settings.secrets.kdbx_enabled {
            // Clear encrypted password if integration is disabled
            settings.secrets.clear_password();
        }

        self.config_manager
            .save_settings(&settings)
            .map_err(|e| format!("Failed to save settings: {e}"))?;

        // Update session manager logging
        if settings.logging.enabled != self.settings.logging.enabled {
            self.session_manager
                .set_logging_enabled(settings.logging.enabled);
        }

        self.settings = settings;
        Ok(())
    }

    /// Gets the config manager
    pub const fn config_manager(&self) -> &ConfigManager {
        &self.config_manager
    }

    /// Updates the expanded groups in settings and saves
    pub fn update_expanded_groups(
        &mut self,
        expanded: std::collections::HashSet<uuid::Uuid>,
    ) -> Result<(), String> {
        self.settings.ui.expanded_groups = expanded;
        self.config_manager
            .save_settings(&self.settings)
            .map_err(|e| format!("Failed to save settings: {e}"))
    }

    /// Gets the expanded groups from settings
    #[must_use]
    pub fn expanded_groups(&self) -> &std::collections::HashSet<uuid::Uuid> {
        &self.settings.ui.expanded_groups
    }

    /// Gets the connection manager
    pub fn connection_manager(&mut self) -> &mut ConnectionManager {
        &mut self.connection_manager
    }

    // ========== Import Operations ==========

    /// Imports connections from an import result with automatic group creation
    ///
    /// Creates a parent group for the import source (e.g., "Remmina Import", "SSH Config Import")
    /// and organizes connections into subgroups based on their original grouping.
    pub fn import_connections_with_source(
        &mut self,
        result: &ImportResult,
        source_name: &str,
    ) -> Result<usize, String> {
        let mut imported = 0;

        // Create parent group for this import source
        // Use generate_unique_group_name to handle duplicate names
        let base_group_name = format!("{source_name} Import");
        let parent_group_name = self.generate_unique_group_name(&base_group_name);
        let parent_group_id = match self.connection_manager.create_group(parent_group_name) {
            Ok(id) => Some(id),
            Err(_) => {
                // Group might already exist, try to find it
                self.connection_manager
                    .list_groups()
                    .iter()
                    .find(|g| g.name == base_group_name)
                    .map(|g| g.id)
            }
        };

        // Create a map for subgroups - maps OLD group UUID to NEW group UUID
        let mut group_uuid_map: std::collections::HashMap<Uuid, Uuid> =
            std::collections::HashMap::new();
        // Also keep name-based map for Remmina groups
        let mut subgroup_map: std::collections::HashMap<String, Uuid> =
            std::collections::HashMap::new();

        // Import groups from result preserving hierarchy
        // First pass: identify root groups (no parent or parent not in import)
        let imported_group_ids: std::collections::HashSet<Uuid> =
            result.groups.iter().map(|g| g.id).collect();

        // Sort groups by hierarchy level (root groups first, then children)
        let mut sorted_groups: Vec<&ConnectionGroup> = result.groups.iter().collect();
        sorted_groups.sort_by(|a, b| {
            let a_is_root = a.parent_id.is_none()
                || !imported_group_ids.contains(&a.parent_id.unwrap_or(Uuid::nil()));
            let b_is_root = b.parent_id.is_none()
                || !imported_group_ids.contains(&b.parent_id.unwrap_or(Uuid::nil()));
            b_is_root.cmp(&a_is_root) // Root groups first
        });

        // Create groups preserving hierarchy
        for group in sorted_groups {
            // Determine the actual parent for this group
            let actual_parent_id = if let Some(orig_parent_id) = group.parent_id {
                // Check if original parent is in the import
                if let Some(&new_parent_id) = group_uuid_map.get(&orig_parent_id) {
                    // Parent was already created, use its new ID
                    Some(new_parent_id)
                } else {
                    // Parent not in import, use import root group
                    parent_group_id
                }
            } else {
                // Root group in import, make it child of import root
                parent_group_id
            };

            let new_group_id = if let Some(parent_id) = actual_parent_id {
                match self
                    .connection_manager
                    .create_group_with_parent(group.name.clone(), parent_id)
                {
                    Ok(id) => Some(id),
                    Err(_) => {
                        // Try to find existing
                        self.connection_manager
                            .get_child_groups(parent_id)
                            .iter()
                            .find(|g| g.name == group.name)
                            .map(|g| g.id)
                    }
                }
            } else {
                self.connection_manager
                    .create_group(group.name.clone())
                    .ok()
            };

            if let Some(new_id) = new_group_id {
                // Map old group UUID to new group UUID
                group_uuid_map.insert(group.id, new_id);
                subgroup_map.insert(group.name.clone(), new_id);
            }
        }

        // Import connections with automatic conflict resolution
        for conn in &result.connections {
            let mut connection = conn.clone();

            // Check for Remmina group tag (format: "remmina:group_name")
            let remmina_group = connection
                .tags
                .iter()
                .find(|t| t.starts_with("remmina:"))
                .map(|t| t.strip_prefix("remmina:").unwrap_or("").to_string());

            // Remove the remmina group tag from tags
            connection.tags.retain(|t| !t.starts_with("remmina:"));

            // Determine target group
            let target_group_id = if let Some(group_name) = remmina_group {
                // Create subgroup for Remmina group if not exists
                if !subgroup_map.contains_key(&group_name) {
                    if let Some(parent_id) = parent_group_id {
                        if let Ok(id) = self
                            .connection_manager
                            .create_group_with_parent(group_name.clone(), parent_id)
                        {
                            subgroup_map.insert(group_name.clone(), id);
                        }
                    }
                }
                subgroup_map.get(&group_name).copied()
            } else if let Some(existing_group_id) = connection.group_id {
                // Connection has a group from import - map to new UUID
                group_uuid_map
                    .get(&existing_group_id)
                    .copied()
                    .or(parent_group_id)
            } else {
                // Use parent import group
                parent_group_id
            };

            // Set the group
            connection.group_id = target_group_id;

            // Auto-resolve name conflicts using protocol-aware naming
            if self.connection_exists_by_name(&connection.name) {
                connection.name =
                    self.generate_unique_connection_name(&connection.name, connection.protocol);
            }

            match self.connection_manager.create_connection_from(connection) {
                Ok(_) => imported += 1,
                Err(e) => eprintln!("Warning: Failed to import connection {}: {}", conn.name, e),
            }
        }

        Ok(imported)
    }

    /// Imports connections from an import result (legacy method)
    ///
    /// Note: Prefer `import_connections_with_source` for better organization.
    #[allow(dead_code)]
    pub fn import_connections(&mut self, result: &ImportResult) -> Result<usize, String> {
        self.import_connections_with_source(result, "Unknown")
    }

    // ========== Document Operations ==========

    /// Creates a new document
    pub fn create_document(&mut self, name: String) -> Uuid {
        let id = self.document_manager.create(name);
        // Set as active if no active document
        if self.active_document_id.is_none() {
            self.active_document_id = Some(id);
        }
        id
    }

    /// Opens a document from a file
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed
    pub fn open_document(&mut self, path: &Path, password: Option<&str>) -> Result<Uuid, String> {
        self.document_manager
            .load(path, password)
            .map_err(|e| format!("Failed to open document: {e}"))
    }

    /// Saves a document to a file
    ///
    /// # Errors
    ///
    /// Returns an error if the document cannot be saved
    pub fn save_document(
        &mut self,
        id: Uuid,
        path: &Path,
        password: Option<&str>,
    ) -> Result<(), String> {
        self.document_manager
            .save(id, path, password)
            .map_err(|e| format!("Failed to save document: {e}"))
    }

    /// Closes a document
    ///
    /// Returns the document if it was removed
    pub fn close_document(&mut self, id: Uuid) -> Option<Document> {
        let doc = self.document_manager.remove(id);
        // Update active document if needed
        if self.active_document_id == Some(id) {
            self.active_document_id = self.document_manager.document_ids().first().copied();
        }
        doc
    }

    /// Gets a document by ID
    pub fn get_document(&self, id: Uuid) -> Option<&Document> {
        self.document_manager.get(id)
    }

    /// Gets a mutable reference to a document by ID
    ///
    /// Note: Part of document management API.
    #[allow(dead_code)]
    pub fn get_document_mut(&mut self, id: Uuid) -> Option<&mut Document> {
        self.document_manager.get_mut(id)
    }

    /// Lists all document IDs
    ///
    /// Note: Part of document management API.
    #[allow(dead_code)]
    pub fn list_document_ids(&self) -> Vec<Uuid> {
        self.document_manager.document_ids()
    }

    /// Returns the number of loaded documents
    ///
    /// Note: Part of document management API.
    #[allow(dead_code)]
    pub fn document_count(&self) -> usize {
        self.document_manager.document_count()
    }

    /// Returns true if the document has unsaved changes
    pub fn is_document_dirty(&self, id: Uuid) -> bool {
        self.document_manager.is_dirty(id)
    }

    /// Marks a document as dirty
    ///
    /// Note: Part of document management API.
    #[allow(dead_code)]
    pub fn mark_document_dirty(&mut self, id: Uuid) {
        self.document_manager.mark_dirty(id);
    }

    /// Returns true if any document has unsaved changes
    ///
    /// Note: Part of document management API.
    #[allow(dead_code)]
    pub fn has_dirty_documents(&self) -> bool {
        self.document_manager.has_dirty_documents()
    }

    /// Returns IDs of all dirty documents
    ///
    /// Note: Part of document management API.
    #[allow(dead_code)]
    pub fn dirty_document_ids(&self) -> Vec<Uuid> {
        self.document_manager.dirty_document_ids()
    }

    /// Gets the file path for a document if it has been saved
    pub fn get_document_path(&self, id: Uuid) -> Option<&Path> {
        self.document_manager.get_path(id)
    }

    /// Gets the currently active document ID
    pub const fn active_document_id(&self) -> Option<Uuid> {
        self.active_document_id
    }

    /// Sets the active document
    ///
    /// Note: Part of document management API.
    #[allow(dead_code)]
    pub fn set_active_document(&mut self, id: Option<Uuid>) {
        self.active_document_id = id;
    }

    /// Gets the currently active document
    pub fn active_document(&self) -> Option<&Document> {
        self.active_document_id
            .and_then(|id| self.document_manager.get(id))
    }

    /// Exports a document to a portable file
    ///
    /// # Errors
    ///
    /// Returns an error if the document cannot be exported
    pub fn export_document(&self, id: Uuid, path: &Path) -> Result<(), String> {
        self.document_manager
            .export(id, path)
            .map_err(|e| format!("Failed to export document: {e}"))
    }

    /// Imports a document from a file
    ///
    /// # Errors
    ///
    /// Returns an error if the document cannot be imported
    pub fn import_document(&mut self, path: &Path) -> Result<Uuid, String> {
        self.document_manager
            .import(path)
            .map_err(|e| format!("Failed to import document: {e}"))
    }

    /// Gets the document manager
    ///
    /// Note: Part of document management API.
    #[allow(dead_code)]
    pub const fn document_manager(&self) -> &DocumentManager {
        &self.document_manager
    }

    /// Gets a mutable reference to the document manager
    ///
    /// Note: Part of document management API.
    #[allow(dead_code)]
    pub fn document_manager_mut(&mut self) -> &mut DocumentManager {
        &mut self.document_manager
    }

    // ========== Cluster Operations ==========

    /// Gets the cluster manager
    ///
    /// Note: Part of cluster management API.
    #[allow(dead_code)]
    pub const fn cluster_manager(&self) -> &ClusterManager {
        &self.cluster_manager
    }

    /// Gets a mutable reference to the cluster manager
    ///
    /// Note: Part of cluster management API.
    #[allow(dead_code)]
    pub fn cluster_manager_mut(&mut self) -> &mut ClusterManager {
        &mut self.cluster_manager
    }

    /// Creates a new cluster
    pub fn create_cluster(&mut self, cluster: Cluster) -> Result<Uuid, String> {
        let id = cluster.id;
        self.cluster_manager.add_cluster(cluster);
        self.save_clusters()?;
        Ok(id)
    }

    /// Updates an existing cluster
    pub fn update_cluster(&mut self, cluster: Cluster) -> Result<(), String> {
        self.cluster_manager
            .update_cluster(cluster.id, cluster)
            .map_err(|e| format!("Failed to update cluster: {e}"))?;
        self.save_clusters()
    }

    /// Deletes a cluster
    pub fn delete_cluster(&mut self, cluster_id: Uuid) -> Result<(), String> {
        self.cluster_manager.remove_cluster(cluster_id);
        self.save_clusters()
    }

    /// Gets a cluster by ID
    pub fn get_cluster(&self, cluster_id: Uuid) -> Option<&Cluster> {
        self.cluster_manager.get_cluster(cluster_id)
    }

    /// Gets all clusters
    pub fn get_all_clusters(&self) -> Vec<&Cluster> {
        self.cluster_manager.get_all_clusters()
    }

    /// Saves clusters to disk
    fn save_clusters(&self) -> Result<(), String> {
        let clusters = self.cluster_manager.clusters_to_vec();
        self.config_manager
            .save_clusters(&clusters)
            .map_err(|e| format!("Failed to save clusters: {e}"))
    }

    // ========== Template Operations ==========

    /// Loads templates from disk
    pub fn load_templates(&self) -> Result<Vec<rustconn_core::ConnectionTemplate>, String> {
        self.config_manager
            .load_templates()
            .map_err(|e| format!("Failed to load templates: {e}"))
    }

    /// Saves templates to disk
    pub fn save_templates(
        &self,
        templates: &[rustconn_core::ConnectionTemplate],
    ) -> Result<(), String> {
        self.config_manager
            .save_templates(templates)
            .map_err(|e| format!("Failed to save templates: {e}"))
    }

    /// Adds a template and saves to disk
    pub fn add_template(
        &mut self,
        template: rustconn_core::ConnectionTemplate,
    ) -> Result<(), String> {
        // Add to active document if one exists
        if let Some(doc_id) = self.active_document_id {
            if let Some(doc) = self.document_manager.get_mut(doc_id) {
                doc.add_template(template.clone());
            }
        }

        // Also save to config file for persistence
        let mut templates = self.load_templates().unwrap_or_default();
        templates.push(template);
        self.save_templates(&templates)
    }

    /// Updates a template and saves to disk
    pub fn update_template(
        &mut self,
        template: rustconn_core::ConnectionTemplate,
    ) -> Result<(), String> {
        let id = template.id;

        // Update in active document if one exists
        if let Some(doc_id) = self.active_document_id {
            if let Some(doc) = self.document_manager.get_mut(doc_id) {
                doc.remove_template(id);
                doc.add_template(template.clone());
            }
        }

        // Also update in config file
        let mut templates = self.load_templates().unwrap_or_default();
        if let Some(pos) = templates.iter().position(|t| t.id == id) {
            templates[pos] = template;
        } else {
            templates.push(template);
        }
        self.save_templates(&templates)
    }

    /// Deletes a template and saves to disk
    pub fn delete_template(&mut self, template_id: uuid::Uuid) -> Result<(), String> {
        // Remove from active document if one exists
        if let Some(doc_id) = self.active_document_id {
            if let Some(doc) = self.document_manager.get_mut(doc_id) {
                doc.remove_template(template_id);
            }
        }

        // Also remove from config file
        let mut templates = self.load_templates().unwrap_or_default();
        templates.retain(|t| t.id != template_id);
        self.save_templates(&templates)
    }

    /// Gets all templates (from config file and active document)
    pub fn get_all_templates(&self) -> Vec<rustconn_core::ConnectionTemplate> {
        let mut templates = self.load_templates().unwrap_or_default();

        // Also include templates from active document
        if let Some(doc) = self.active_document() {
            for doc_template in &doc.templates {
                if !templates.iter().any(|t| t.id == doc_template.id) {
                    templates.push(doc_template.clone());
                }
            }
        }

        templates
    }

    // ========== Connection History Operations ==========

    /// Gets all history entries
    #[must_use]
    pub fn history_entries(&self) -> &[ConnectionHistoryEntry] {
        &self.history_entries
    }

    /// Adds a new history entry for a connection start
    pub fn record_connection_start(
        &mut self,
        connection: &Connection,
        username: Option<&str>,
    ) -> Uuid {
        let entry = ConnectionHistoryEntry::new(
            connection.id,
            connection.name.clone(),
            connection.host.clone(),
            connection.port,
            format!("{:?}", connection.protocol).to_lowercase(),
            username.map(String::from),
        );
        let entry_id = entry.id;
        self.history_entries.push(entry);
        self.trim_history();
        let _ = self.save_history();
        entry_id
    }

    /// Adds a new history entry for a quick connect
    #[allow(dead_code)]
    pub fn record_quick_connect_start(
        &mut self,
        host: &str,
        port: u16,
        protocol: &str,
        username: Option<&str>,
    ) -> Uuid {
        if !self.settings.history.track_quick_connect {
            return Uuid::nil();
        }
        let entry = ConnectionHistoryEntry::new_quick_connect(
            host.to_string(),
            port,
            protocol.to_string(),
            username.map(String::from),
        );
        let entry_id = entry.id;
        self.history_entries.push(entry);
        self.trim_history();
        let _ = self.save_history();
        entry_id
    }

    /// Marks a history entry as ended (successful)
    pub fn record_connection_end(&mut self, entry_id: Uuid) {
        if let Some(entry) = self.history_entries.iter_mut().find(|e| e.id == entry_id) {
            entry.end();
            let _ = self.save_history();
        }
    }

    /// Marks a history entry as failed
    pub fn record_connection_failed(&mut self, entry_id: Uuid, error: &str) {
        if let Some(entry) = self.history_entries.iter_mut().find(|e| e.id == entry_id) {
            entry.fail(error);
            let _ = self.save_history();
        }
    }

    /// Clears all history entries
    #[allow(dead_code)]
    pub fn clear_history(&mut self) {
        self.history_entries.clear();
        let _ = self.save_history();
    }

    /// Gets statistics for a specific connection
    #[must_use]
    #[allow(dead_code)]
    pub fn get_connection_statistics(&self, connection_id: Uuid) -> ConnectionStatistics {
        let mut stats = ConnectionStatistics::new(connection_id);
        for entry in &self.history_entries {
            if entry.connection_id == connection_id {
                stats.update_from_entry(entry);
            }
        }
        stats
    }

    /// Gets statistics for all connections
    #[must_use]
    pub fn get_all_statistics(&self) -> Vec<(String, ConnectionStatistics)> {
        let mut stats_map: HashMap<Uuid, (String, ConnectionStatistics)> = HashMap::new();

        for entry in &self.history_entries {
            let stat_entry = stats_map.entry(entry.connection_id).or_insert_with(|| {
                (
                    entry.connection_name.clone(),
                    ConnectionStatistics::new(entry.connection_id),
                )
            });
            stat_entry.1.update_from_entry(entry);
        }

        stats_map.into_values().collect()
    }

    /// Clears all connection statistics by clearing history
    pub fn clear_all_statistics(&mut self) {
        self.history_entries.clear();
        if let Err(e) = self.save_history() {
            tracing::error!("Failed to save cleared history: {e}");
        }
    }

    /// Trims history to max entries and retention period
    #[allow(dead_code)]
    fn trim_history(&mut self) {
        let max_entries = self.settings.history.max_entries;
        let retention_days = self.settings.history.retention_days;

        // Remove old entries
        let cutoff = chrono::Utc::now() - chrono::Duration::days(i64::from(retention_days));
        self.history_entries.retain(|e| e.started_at > cutoff);

        // Trim to max entries (keep most recent)
        if self.history_entries.len() > max_entries {
            self.history_entries
                .sort_by(|a, b| b.started_at.cmp(&a.started_at));
            self.history_entries.truncate(max_entries);
        }
    }

    /// Saves history to disk
    fn save_history(&self) -> Result<(), String> {
        self.config_manager
            .save_history(&self.history_entries)
            .map_err(|e| format!("Failed to save history: {e}"))
    }

    // ========== Clipboard Operations ==========

    /// Gets a reference to the connection clipboard
    ///
    /// Note: Part of clipboard API for connection copy/paste.
    #[allow(dead_code)]
    pub const fn clipboard(&self) -> &ConnectionClipboard {
        &self.clipboard
    }

    /// Gets a mutable reference to the connection clipboard
    ///
    /// Note: Part of clipboard API for connection copy/paste.
    #[allow(dead_code)]
    pub fn clipboard_mut(&mut self) -> &mut ConnectionClipboard {
        &mut self.clipboard
    }

    /// Copies a connection to the clipboard
    ///
    /// # Arguments
    /// * `connection_id` - The ID of the connection to copy
    ///
    /// # Returns
    /// `Ok(())` if the connection was copied, `Err` if not found
    pub fn copy_connection(&mut self, connection_id: Uuid) -> Result<(), String> {
        let connection = self
            .get_connection(connection_id)
            .ok_or_else(|| format!("Connection not found: {connection_id}"))?
            .clone();
        let group_id = connection.group_id;
        self.clipboard.copy(&connection, group_id);
        Ok(())
    }

    /// Pastes a connection from the clipboard
    ///
    /// Creates a duplicate connection with a new ID and "(Copy)" suffix.
    /// The connection is added to the same group as the original.
    ///
    /// # Returns
    /// `Ok(Uuid)` with the new connection's ID, or `Err` if clipboard is empty
    pub fn paste_connection(&mut self) -> Result<Uuid, String> {
        let new_conn = self
            .clipboard
            .paste()
            .ok_or_else(|| "Clipboard is empty".to_string())?;

        // Get the source group from clipboard
        let target_group = self.clipboard.source_group();

        // Create the connection with the target group
        let mut conn_with_group = new_conn;
        conn_with_group.group_id = target_group;

        // Generate unique name if needed using protocol-aware naming
        if self.connection_exists_by_name(&conn_with_group.name) {
            conn_with_group.name = self
                .generate_unique_connection_name(&conn_with_group.name, conn_with_group.protocol);
        }

        self.connection_manager
            .create_connection_from(conn_with_group)
            .map_err(|e| format!("Failed to paste connection: {e}"))
    }

    /// Checks if the clipboard has content
    #[must_use]
    pub const fn has_clipboard_content(&self) -> bool {
        self.clipboard.has_content()
    }

    // ========== Session Restore Operations ==========

    /// Saves active sessions for later restoration
    ///
    /// This method collects information about currently active sessions
    /// and stores them in settings for restoration on next startup.
    ///
    /// # Arguments
    /// * `sessions` - List of active terminal sessions to save
    ///
    /// Note: Part of session restore API - called on app shutdown.
    #[allow(dead_code)]
    pub fn save_active_sessions(
        &mut self,
        sessions: &[crate::terminal::TerminalSession],
    ) -> Result<(), String> {
        use rustconn_core::config::SavedSession;

        let now = Utc::now();
        let saved: Vec<SavedSession> = sessions
            .iter()
            .filter_map(|session| {
                // Get connection details
                self.get_connection(session.connection_id)
                    .map(|conn| SavedSession {
                        connection_id: conn.id,
                        connection_name: conn.name.clone(),
                        protocol: session.protocol.clone(),
                        host: conn.host.clone(),
                        port: conn.port,
                        saved_at: now,
                    })
            })
            .collect();

        self.settings.ui.session_restore.saved_sessions = saved;
        self.config_manager
            .save_settings(&self.settings)
            .map_err(|e| format!("Failed to save session restore settings: {e}"))
    }

    /// Gets sessions that should be restored based on settings
    ///
    /// Filters saved sessions by max_age_hours and returns only those
    /// whose connections still exist.
    ///
    /// # Returns
    /// List of saved sessions that are eligible for restoration
    ///
    /// Note: Part of session restore API - called on app startup.
    #[must_use]
    #[allow(dead_code)]
    pub fn get_sessions_to_restore(&self) -> Vec<rustconn_core::config::SavedSession> {
        if !self.settings.ui.session_restore.enabled {
            return Vec::new();
        }

        let max_age = self.settings.ui.session_restore.max_age_hours;
        let now = Utc::now();

        self.settings
            .ui
            .session_restore
            .saved_sessions
            .iter()
            .filter(|session| {
                // Check if connection still exists
                if self.get_connection(session.connection_id).is_none() {
                    return false;
                }

                // Check age limit (0 = no limit)
                if max_age > 0 {
                    let age_hours = (now - session.saved_at).num_hours();
                    if age_hours > i64::from(max_age) {
                        return false;
                    }
                }

                true
            })
            .cloned()
            .collect()
    }

    /// Clears saved sessions
    ///
    /// Note: Part of session restore API.
    #[allow(dead_code)]
    pub fn clear_saved_sessions(&mut self) -> Result<(), String> {
        self.settings.ui.session_restore.saved_sessions.clear();
        self.config_manager
            .save_settings(&self.settings)
            .map_err(|e| format!("Failed to clear saved sessions: {e}"))
    }

    /// Checks if session restore is enabled
    ///
    /// Note: Part of session restore API.
    #[must_use]
    #[allow(dead_code)]
    pub const fn is_session_restore_enabled(&self) -> bool {
        self.settings.ui.session_restore.enabled
    }

    /// Checks if prompt should be shown before restoring sessions
    ///
    /// Note: Part of session restore API.
    #[must_use]
    #[allow(dead_code)]
    pub const fn should_prompt_on_restore(&self) -> bool {
        self.settings.ui.session_restore.prompt_on_restore
    }
}

/// Shared application state type
pub type SharedAppState = Rc<RefCell<AppState>>;

/// Creates a new shared application state
pub fn create_shared_state() -> Result<SharedAppState, String> {
    AppState::new().map(|state| Rc::new(RefCell::new(state)))
}
