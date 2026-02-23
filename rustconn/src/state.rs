//! Application state management
//!
//! This module provides the central application state that holds all managers
//! and provides thread-safe access to core functionality.

use crate::async_utils::with_runtime;
use chrono::Utc;
use rustconn_core::cluster::{Cluster, ClusterManager};
use rustconn_core::config::{AppSettings, ConfigManager};
use rustconn_core::connection::ConnectionManager;
use rustconn_core::document::{Document, DocumentManager, EncryptionStrength};
use rustconn_core::error::ConfigResult;
use rustconn_core::import::ImportResult;
use rustconn_core::models::{
    Connection, ConnectionGroup, ConnectionHistoryEntry, ConnectionStatistics, Credentials,
    PasswordSource, Snippet,
};
use rustconn_core::secret::{
    AsyncCredentialResolver, AsyncCredentialResult, CancellationToken, CredentialResolver,
    SecretManager,
};
use rustconn_core::session::{Session, SessionManager};
use rustconn_core::snippet::SnippetManager;
use secrecy::SecretString;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

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

/// Default TTL for cached credentials in seconds (5 minutes)
pub const DEFAULT_CREDENTIAL_TTL_SECONDS: u64 = 300;

/// Cached credentials for a connection (session-only, not persisted)
///
/// Credentials are automatically expired after `ttl_seconds` to minimize
/// the window of exposure for sensitive data in memory.
#[derive(Clone)]
pub struct CachedCredentials {
    /// Username
    pub username: String,
    /// Password (stored securely in memory)
    pub password: SecretString,
    /// Domain for Windows authentication
    pub domain: String,
    /// Timestamp when credentials were cached
    cached_at: chrono::DateTime<chrono::Utc>,
    /// Time-to-live in seconds (credentials expire after this duration)
    ttl_seconds: u64,
}

impl CachedCredentials {
    /// Creates new cached credentials with default TTL
    #[must_use]
    pub fn new(username: String, password: SecretString, domain: String) -> Self {
        Self {
            username,
            password,
            domain,
            cached_at: chrono::Utc::now(),
            ttl_seconds: DEFAULT_CREDENTIAL_TTL_SECONDS,
        }
    }

    /// Checks if the cached credentials have expired
    #[must_use]
    pub fn is_expired(&self) -> bool {
        let elapsed = chrono::Utc::now() - self.cached_at;
        // Handle negative durations gracefully (clock skew)
        elapsed.num_seconds().max(0) as u64 > self.ttl_seconds
    }

    /// Refreshes the cache timestamp (extends TTL)
    pub fn refresh(&mut self) {
        self.cached_at = chrono::Utc::now();
    }
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
                    tracing::warn!(
                        path = %kdbx_path.display(),
                        "KeePass database file not found. Disabling integration."
                    );
                    disable_integration = true;
                }
            } else {
                tracing::warn!(
                    "KeePass integration enabled but no database path configured. Disabling."
                );
                disable_integration = true;
            }

            if disable_integration {
                settings.secrets.kdbx_enabled = false;
                settings.secrets.clear_password();
                // Save updated settings
                if let Err(e) = config_manager.save_settings(&settings) {
                    tracing::error!(%e, "Failed to save settings after disabling KDBX");
                }
            } else {
                // Try to decrypt stored password
                if settings.secrets.decrypt_password() {
                    tracing::info!("KeePass password restored from encrypted storage");
                }
            }
        }

        // Note: Bitwarden password decryption and vault auto-unlock are deferred
        // to `initialize_secret_backends()` which runs asynchronously after the
        // main window is presented. This avoids blocking the UI on startup.

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

        // Initialize secret manager with backends from settings
        let secret_manager = SecretManager::build_from_settings(&settings.secrets);

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
            active_document_id: None,
            settings,
            password_cache: HashMap::new(),
            clipboard: ConnectionClipboard::new(),
            history_entries,
        })
    }

    /// Initializes secret backends asynchronously after the main window is shown.
    ///
    /// This decrypts Bitwarden/KDBX passwords and auto-unlocks vaults without
    /// blocking the GTK main thread. Call this via `spawn_async` after
    /// `window.present()` to keep startup fast.
    ///
    /// Returns `true` if a backend was successfully initialized.
    pub fn initialize_secret_backends(&mut self) -> bool {
        let mut backend_ready = false;

        // Decrypt Bitwarden password from encrypted storage
        if self.settings.secrets.bitwarden_password_encrypted.is_some() {
            if self.settings.secrets.decrypt_bitwarden_password() {
                tracing::info!("Bitwarden password restored from encrypted storage");
            } else {
                tracing::warn!("Failed to decrypt Bitwarden password");
            }
        }

        // Decrypt Bitwarden API credentials
        if self.settings.secrets.bitwarden_use_api_key
            && (self
                .settings
                .secrets
                .bitwarden_client_id_encrypted
                .is_some()
                || self
                    .settings
                    .secrets
                    .bitwarden_client_secret_encrypted
                    .is_some())
        {
            if self.settings.secrets.decrypt_bitwarden_api_credentials() {
                tracing::info!("Bitwarden API credentials restored from encrypted storage");
            } else {
                tracing::warn!("Failed to decrypt Bitwarden API credentials");
            }
        }

        // Auto-unlock Bitwarden vault
        if matches!(
            self.settings.secrets.preferred_backend,
            rustconn_core::config::SecretBackendType::Bitwarden
        ) {
            match crate::async_utils::with_runtime(|rt| {
                rt.block_on(rustconn_core::secret::auto_unlock(&self.settings.secrets))
            }) {
                Ok(Ok(_backend)) => {
                    tracing::info!("Bitwarden vault unlocked at startup");
                    backend_ready = true;
                }
                Ok(Err(e)) => {
                    tracing::warn!("Bitwarden auto-unlock at startup failed: {e}");
                }
                Err(e) => {
                    tracing::warn!("Bitwarden auto-unlock at startup failed (runtime): {e}");
                }
            }
        }

        backend_ready
    }

    // ========== Password Cache Operations ==========

    /// Caches credentials for a connection (session-only)
    ///
    /// Credentials are cached with a default TTL and will automatically expire.
    /// Use `cache_credentials_with_ttl` for custom expiration times.
    pub fn cache_credentials(
        &mut self,
        connection_id: Uuid,
        username: &str,
        password: &str,
        domain: &str,
    ) {
        self.password_cache.insert(
            connection_id,
            CachedCredentials::new(
                username.to_string(),
                SecretString::from(password.to_string()),
                domain.to_string(),
            ),
        );
    }

    /// Gets cached credentials for a connection if not expired
    ///
    /// Returns `None` if credentials are not cached or have expired.
    /// Note: This method does not remove expired credentials. Use
    /// `get_cached_credentials_mut` or `cleanup_expired_credentials` for cleanup.
    #[must_use]
    pub fn get_cached_credentials(&self, connection_id: Uuid) -> Option<&CachedCredentials> {
        self.password_cache
            .get(&connection_id)
            .filter(|creds| !creds.is_expired())
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

    /// Restores a deleted connection
    pub fn restore_connection(&mut self, id: Uuid) -> ConfigResult<()> {
        self.connection_manager.restore_connection(id)
    }

    /// Restores a deleted group
    pub fn restore_group(&mut self, id: Uuid) -> ConfigResult<()> {
        self.connection_manager.restore_group(id)
    }

    /// Permanently empties the trash
    #[allow(dead_code)]
    pub fn empty_trash(&mut self) -> ConfigResult<()> {
        self.connection_manager.empty_trash()
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

    /// Finds a connection by name (case-insensitive)
    ///
    /// Returns the first match. Used by CLI `--connect <name>` resolution.
    pub fn find_connection_by_name(&self, name: &str) -> Option<&Connection> {
        let lower = name.to_lowercase();
        self.connection_manager
            .list_connections()
            .into_iter()
            .find(|c| c.name.to_lowercase() == lower)
    }

    /// Lists all connections
    pub fn list_connections(&self) -> Vec<&Connection> {
        self.connection_manager.list_connections()
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

    /// Flushes any pending persistence operations immediately
    ///
    /// This ensures that debounced saves are written to disk before application exit.
    pub fn flush_persistence(&self) -> Result<(), String> {
        with_runtime(|rt| {
            rt.block_on(self.connection_manager.flush_persistence())
                .map_err(|e| format!("Failed to flush persistence: {e}"))
        })?
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

    /// Checks if any secret backend is available (blocking wrapper)
    ///
    /// Note: Used internally by resolve_credentials_blocking and should_prompt_for_credentials.
    #[allow(dead_code)]
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
    /// - `PasswordSource::Bitwarden` - Try Bitwarden vault
    /// - `PasswordSource::Prompt` - Return None (caller prompts user)
    /// - `PasswordSource::None` - Try fallback chain if enabled
    ///
    /// # Returns
    /// - `Ok(Some(Credentials))` - Credentials found from a backend
    /// - `Ok(None)` - No credentials found, caller should prompt user
    /// - `Err(String)` - Error during resolution
    ///
    /// Note: This is a blocking method. Prefer `resolve_credentials_gtk` for GUI code.
    #[allow(dead_code)]
    pub fn resolve_credentials(
        &self,
        connection: &Connection,
    ) -> Result<Option<Credentials>, String> {
        use rustconn_core::secret::{KeePassHierarchy, KeePassStatus};
        use secrecy::ExposeSecret;

        let groups: Vec<ConnectionGroup> = self
            .connection_manager
            .list_groups()
            .iter()
            .cloned()
            .cloned()
            .collect();

        // For Variable password source — resolve directly via vault backend
        if let PasswordSource::Variable(ref var_name) = connection.password_source {
            tracing::debug!(
                var_name,
                "[resolve_credentials] Resolving variable password"
            );
            match load_variable_from_vault(&self.settings.secrets, var_name) {
                Ok(Some(password)) => {
                    tracing::debug!(var_name, "[resolve_credentials] Variable resolved");
                    let creds = if let Some(ref username) = connection.username {
                        Credentials::with_password(username, &password)
                    } else {
                        Credentials {
                            username: None,
                            password: Some(secrecy::SecretString::from(password)),
                            key_passphrase: None,
                            domain: None,
                        }
                    };
                    return Ok(Some(creds));
                }
                Ok(None) => {
                    tracing::warn!(
                        var_name,
                        "[resolve_credentials] No secret found for variable"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        var_name,
                        error = %e,
                        "[resolve_credentials] Failed to load variable from vault"
                    );
                }
            }
        }

        // For Vault password source with KeePass backend, directly use
        // KeePassStatus to retrieve password. This bypasses the
        // SecretManager which requires registered backends.
        if connection.password_source == PasswordSource::Vault
            && self.settings.secrets.kdbx_enabled
            && matches!(
                self.settings.secrets.preferred_backend,
                rustconn_core::config::SecretBackendType::KeePassXc
                    | rustconn_core::config::SecretBackendType::KdbxFile
            )
            && let Some(ref kdbx_path) = self.settings.secrets.kdbx_path
        {
            // Build hierarchical entry path using KeePassHierarchy
            // This matches how passwords are saved with group structure
            let entry_path = KeePassHierarchy::build_entry_path(connection, &groups);

            // Add protocol suffix for uniqueness
            let protocol = connection.protocol_config.protocol_type();
            let protocol_str = protocol.as_str();

            // Strip RustConn/ prefix since get_password_from_kdbx_with_key adds it back
            let entry_name = entry_path.strip_prefix("RustConn/").unwrap_or(&entry_path);
            let lookup_key = format!("{entry_name} ({protocol_str})");

            // Get credentials - password and key file can be used together
            let db_password = self.settings.secrets.kdbx_password.as_ref();

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
                        Credentials::with_password(username, password.expose_secret())
                    } else {
                        Credentials {
                            username: None,
                            password: Some(password),
                            key_passphrase: None,
                            domain: None,
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

        // For Inherit password source, traverse parent groups to find credentials
        if connection.password_source == PasswordSource::Inherit
            && self.settings.secrets.kdbx_enabled
            && matches!(
                self.settings.secrets.preferred_backend,
                rustconn_core::config::SecretBackendType::KeePassXc
                    | rustconn_core::config::SecretBackendType::KdbxFile
            )
            && let Some(ref kdbx_path) = self.settings.secrets.kdbx_path
        {
            let db_password = self.settings.secrets.kdbx_password.as_ref();
            let key_file = self.settings.secrets.kdbx_key_file.as_deref();

            // Traverse up the group hierarchy
            let mut current_group_id = connection.group_id;
            while let Some(group_id) = current_group_id {
                let Some(group) = groups.iter().find(|g| g.id == group_id) else {
                    break;
                };

                // Check if this group has Vault credentials configured
                if group.password_source == Some(PasswordSource::Vault) {
                    let group_path = KeePassHierarchy::build_group_entry_path(group, &groups);

                    tracing::debug!(
                        "[resolve_credentials] Inherit: checking group '{}' at path '{}'",
                        group.name,
                        group_path
                    );

                    match KeePassStatus::get_password_from_kdbx_with_key(
                        kdbx_path,
                        db_password,
                        key_file,
                        &group_path,
                        None,
                    ) {
                        Ok(Some(password)) => {
                            tracing::debug!(
                                "[resolve_credentials] Found inherited password from group '{}'",
                                group.name
                            );
                            let username = connection
                                .username
                                .clone()
                                .or_else(|| group.username.clone());
                            let creds = if let Some(ref uname) = username {
                                Credentials::with_password(uname, password.expose_secret())
                            } else {
                                Credentials {
                                    username: None,
                                    password: Some(password),
                                    key_passphrase: None,
                                    domain: None,
                                }
                            };
                            return Ok(Some(creds));
                        }
                        Ok(None) => {
                            tracing::debug!(
                                "[resolve_credentials] No password in group '{}'",
                                group.name
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[resolve_credentials] KeePass error for group '{}': {}",
                                group.name,
                                e
                            );
                        }
                    }
                } else if group.password_source == Some(PasswordSource::Inherit) {
                }

                current_group_id = group.parent_id;
            }

            tracing::debug!(
                "[resolve_credentials] No inherited credentials found in group hierarchy"
            );
        }

        // Fall back to the standard resolver for other password sources
        let secret_manager = self.secret_manager.clone();
        let resolver =
            CredentialResolver::new(Arc::new(secret_manager), self.settings.secrets.clone());
        let connection = connection.clone();

        with_runtime(|rt| {
            rt.block_on(async {
                resolver
                    .resolve_with_hierarchy(&connection, &groups)
                    .await
                    .map_err(|e| format!("Failed to resolve credentials: {e}"))
            })
        })?
    }

    /// Resolves credentials for a connection by ID
    ///
    /// Convenience method that looks up the connection and resolves credentials.
    ///
    /// Note: This is a blocking method. Prefer `resolve_credentials_gtk` for GUI code.
    #[allow(dead_code)]
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
    ///
    /// Note: Part of credential resolution API - used internally.
    #[allow(dead_code)]
    pub fn should_prompt_for_credentials(&self, connection: &Connection) -> bool {
        match connection.password_source {
            PasswordSource::Prompt => true,
            PasswordSource::None => {
                // Check if fallback is enabled and backends are available
                !self.settings.secrets.enable_fallback || !self.has_secret_backend()
            }
            PasswordSource::Vault => {
                // Check if the configured backend is available
                match self.settings.secrets.preferred_backend {
                    rustconn_core::config::SecretBackendType::KeePassXc
                    | rustconn_core::config::SecretBackendType::KdbxFile => {
                        !self.settings.secrets.kdbx_enabled
                    }
                    rustconn_core::config::SecretBackendType::LibSecret => {
                        !self.has_secret_backend()
                    }
                    _ => false, // Bitwarden/1Password/Passbolt handle auth
                }
            }
            PasswordSource::Variable(_) => false, // Resolved from vault
            PasswordSource::Inherit => false,
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

    // ========== GTK-Friendly Async Credential Operations ==========

    /// Resolves credentials for a connection without blocking the GTK main thread
    ///
    /// This method spawns the credential resolution in a background thread and
    /// delivers the result via callback in the GTK main thread. This is the
    /// preferred method for credential resolution in GUI code.
    ///
    /// # Arguments
    /// * `connection_id` - The ID of the connection to resolve credentials for
    /// * `callback` - Function called with the result when resolution completes
    ///
    /// # Requirements Coverage
    /// - Requirement 9.1: Async operations instead of blocking calls
    /// - Requirement 9.2: Avoid `block_on()` in GUI code
    ///
    /// # Example
    /// ```ignore
    /// state.resolve_credentials_gtk(connection_id, move |result| {
    ///     match result {
    ///         Ok(Some(creds)) => { /* use credentials */ }
    ///         Ok(None) => { /* prompt user */ }
    ///         Err(e) => { /* show error */ }
    ///     }
    /// });
    /// ```
    pub fn resolve_credentials_gtk<F>(&self, connection_id: Uuid, callback: F)
    where
        F: FnOnce(Result<Option<Credentials>, String>) + 'static,
    {
        // Get connection and settings needed for resolution
        let connection = if let Some(conn) = self.get_connection(connection_id) {
            conn.clone()
        } else {
            callback(Err(format!("Connection not found: {connection_id}")));
            return;
        };

        // Capture settings needed for KeePass resolution
        let kdbx_enabled = self.settings.secrets.kdbx_enabled;
        let kdbx_path = self.settings.secrets.kdbx_path.clone();
        let kdbx_password = self.settings.secrets.kdbx_password.clone();
        let kdbx_key_file = self.settings.secrets.kdbx_key_file.clone();
        let secret_settings = self.settings.secrets.clone();
        let secret_manager = self.secret_manager.clone();

        // Get groups for hierarchical path building
        let groups: Vec<ConnectionGroup> = self
            .connection_manager
            .list_groups()
            .iter()
            .cloned()
            .cloned()
            .collect();

        // Spawn blocking operation in background thread
        crate::utils::spawn_blocking_with_callback(
            move || {
                Self::resolve_credentials_blocking(
                    &connection,
                    &groups,
                    kdbx_enabled,
                    kdbx_path,
                    kdbx_password,
                    kdbx_key_file,
                    secret_settings,
                    secret_manager,
                )
            },
            callback,
        );
    }

    /// Internal blocking credential resolution (runs in background thread)
    ///
    /// This is extracted from `resolve_credentials` to be callable from a background
    /// thread without needing `&self`.
    #[allow(clippy::too_many_arguments)]
    fn resolve_credentials_blocking(
        connection: &Connection,
        groups: &[ConnectionGroup],
        kdbx_enabled: bool,
        kdbx_path: Option<std::path::PathBuf>,
        kdbx_password: Option<SecretString>,
        kdbx_key_file: Option<std::path::PathBuf>,
        secret_settings: rustconn_core::config::SecretSettings,
        secret_manager: SecretManager,
    ) -> Result<Option<Credentials>, String> {
        use rustconn_core::secret::{KeePassHierarchy, KeePassStatus};
        use secrecy::ExposeSecret;

        // For Variable password source — resolve directly via vault backend
        // This bypasses SecretManager's backend list and uses the same
        // backend selection logic as save_variable_to_vault, ensuring
        // the variable is read from the same backend it was written to.
        if let PasswordSource::Variable(ref var_name) = connection.password_source {
            tracing::debug!(
                var_name,
                "[resolve_credentials_blocking] Resolving variable password"
            );
            match load_variable_from_vault(&secret_settings, var_name) {
                Ok(Some(password)) => {
                    tracing::debug!(var_name, "[resolve_credentials_blocking] Variable resolved");
                    let creds = if let Some(ref username) = connection.username {
                        Credentials::with_password(username, &password)
                    } else {
                        Credentials {
                            username: None,
                            password: Some(secrecy::SecretString::from(password)),
                            key_passphrase: None,
                            domain: None,
                        }
                    };
                    return Ok(Some(creds));
                }
                Ok(None) => {
                    tracing::warn!(
                        var_name,
                        "[resolve_credentials_blocking] No secret found for variable"
                    );
                }
                Err(e) => {
                    tracing::error!(
                        var_name,
                        error = %e,
                        "[resolve_credentials_blocking] Failed to load variable from vault"
                    );
                }
            }
        }

        // For Vault password source with KeePass backend
        if connection.password_source == PasswordSource::Vault
            && kdbx_enabled
            && matches!(
                secret_settings.preferred_backend,
                rustconn_core::config::SecretBackendType::KeePassXc
                    | rustconn_core::config::SecretBackendType::KdbxFile
            )
            && let Some(ref kdbx_path) = kdbx_path
        {
            // Build hierarchical entry path using KeePassHierarchy
            // This matches how passwords are saved with group structure
            let entry_path = KeePassHierarchy::build_entry_path(connection, groups);

            // Add protocol suffix for uniqueness
            let protocol = connection.protocol_config.protocol_type();
            let protocol_str = protocol.as_str();

            // Strip RustConn/ prefix since get_password_from_kdbx_with_key adds it back
            let entry_name = entry_path.strip_prefix("RustConn/").unwrap_or(&entry_path);
            let lookup_key = format!("{entry_name} ({protocol_str})");

            // Get credentials - password and key file can be used together
            let db_password = kdbx_password.as_ref();
            let key_file = kdbx_key_file.as_deref();

            tracing::debug!(
                "[resolve_credentials_blocking] KeePass lookup: key='{}', has_password={}, has_key_file={}",
                lookup_key,
                db_password.is_some(),
                key_file.is_some()
            );

            match KeePassStatus::get_password_from_kdbx_with_key(
                kdbx_path,
                db_password,
                key_file,
                &lookup_key,
                None,
            ) {
                Ok(Some(password)) => {
                    tracing::debug!("[resolve_credentials_blocking] Found password in KeePass");
                    let creds = if let Some(ref username) = connection.username {
                        Credentials::with_password(username, password.expose_secret())
                    } else {
                        Credentials {
                            username: None,
                            password: Some(password),
                            key_passphrase: None,
                            domain: None,
                        }
                    };
                    return Ok(Some(creds));
                }
                Ok(None) => {
                    tracing::debug!("[resolve_credentials_blocking] No password found in KeePass");
                }
                Err(e) => {
                    tracing::error!("[resolve_credentials_blocking] KeePass error: {}", e);
                }
            }
        }

        // For Inherit password source, traverse parent groups to find credentials
        if connection.password_source == PasswordSource::Inherit
            && kdbx_enabled
            && matches!(
                secret_settings.preferred_backend,
                rustconn_core::config::SecretBackendType::KeePassXc
                    | rustconn_core::config::SecretBackendType::KdbxFile
            )
            && let Some(ref kdbx_path) = kdbx_path
        {
            let db_password = kdbx_password.as_ref();
            let key_file = kdbx_key_file.as_deref();

            // Traverse up the group hierarchy
            let mut current_group_id = connection.group_id;
            while let Some(group_id) = current_group_id {
                let Some(group) = groups.iter().find(|g| g.id == group_id) else {
                    break;
                };

                // Check if this group has Vault credentials configured
                if group.password_source == Some(PasswordSource::Vault) {
                    let group_path = KeePassHierarchy::build_group_entry_path(group, groups);

                    tracing::debug!(
                        "[resolve_credentials_blocking] Inherit: checking group '{}' at path '{}'",
                        group.name,
                        group_path
                    );

                    match KeePassStatus::get_password_from_kdbx_with_key(
                        kdbx_path,
                        db_password,
                        key_file,
                        &group_path,
                        None,
                    ) {
                        Ok(Some(password)) => {
                            tracing::debug!(
                                "[resolve_credentials_blocking] Found inherited password from group '{}'",
                                group.name
                            );
                            // Use group's username if connection doesn't have one
                            let username = connection
                                .username
                                .clone()
                                .or_else(|| group.username.clone());
                            let creds = if let Some(ref uname) = username {
                                Credentials::with_password(uname, password.expose_secret())
                            } else {
                                Credentials {
                                    username: None,
                                    password: Some(password),
                                    key_passphrase: None,
                                    domain: None,
                                }
                            };
                            return Ok(Some(creds));
                        }
                        Ok(None) => {
                            tracing::debug!(
                                "[resolve_credentials_blocking] No password in group '{}'",
                                group.name
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[resolve_credentials_blocking] KeePass error for group '{}': {}",
                                group.name,
                                e
                            );
                        }
                    }
                } else if group.password_source == Some(PasswordSource::Inherit) {
                    // Continue to parent
                    tracing::debug!(
                        "[resolve_credentials_blocking] Group '{}' also inherits, continuing to parent",
                        group.name
                    );
                }

                // Move to parent group
                current_group_id = group.parent_id;
            }

            tracing::debug!(
                "[resolve_credentials_blocking] No inherited credentials found in group hierarchy"
            );
        }

        // For Inherit password source with non-KeePass backends
        if connection.password_source == PasswordSource::Inherit && !kdbx_enabled {
            let backend_type = select_backend_for_load(&secret_settings);
            let mut current_group_id = connection.group_id;

            while let Some(group_id) = current_group_id {
                let Some(group) = groups.iter().find(|g| g.id == group_id) else {
                    break;
                };

                if group.password_source == Some(PasswordSource::Vault) {
                    let group_key = group.id.to_string();

                    tracing::debug!(
                        "[resolve_credentials_blocking] Inherit (non-KeePass): checking group '{}' with key '{}'",
                        group.name,
                        group_key
                    );

                    let result: Result<Option<Credentials>, String> = {
                        use rustconn_core::config::SecretBackendType;
                        use rustconn_core::secret::SecretBackend;

                        match backend_type {
                            SecretBackendType::Bitwarden => {
                                crate::async_utils::with_runtime(|rt| {
                                    let backend = rt
                                        .block_on(rustconn_core::secret::auto_unlock(
                                            &secret_settings,
                                        ))
                                        .map_err(|e| format!("{e}"))?;
                                    rt.block_on(backend.retrieve(&group_key))
                                        .map_err(|e| format!("{e}"))
                                })
                                .and_then(|r| r)
                            }
                            SecretBackendType::OnePassword => {
                                let backend = rustconn_core::secret::OnePasswordBackend::new();
                                crate::async_utils::with_runtime(|rt| {
                                    rt.block_on(backend.retrieve(&group_key))
                                        .map_err(|e| format!("{e}"))
                                })
                                .and_then(|r| r)
                            }
                            SecretBackendType::Passbolt => {
                                let backend = rustconn_core::secret::PassboltBackend::new();
                                crate::async_utils::with_runtime(|rt| {
                                    rt.block_on(backend.retrieve(&group_key))
                                        .map_err(|e| format!("{e}"))
                                })
                                .and_then(|r| r)
                            }
                            SecretBackendType::Pass => {
                                let backend = create_pass_backend_from_secret_settings(&secret_settings);
                                crate::async_utils::with_runtime(|rt| {
                                    rt.block_on(backend.retrieve(&group_key))
                                        .map_err(|e| format!("{e}"))
                                })
                                .and_then(|r| r)
                            }
                            SecretBackendType::LibSecret | SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => {
                                let backend =
                                    rustconn_core::secret::LibSecretBackend::new("rustconn");
                                crate::async_utils::with_runtime(|rt| {
                                    rt.block_on(backend.retrieve(&group_key))
                                        .map_err(|e| format!("{e}"))
                                })
                                .and_then(|r| r)
                            }
                        }
                    };

                    match result {
                        Ok(Some(mut creds)) => {
                            tracing::debug!(
                                "[resolve_credentials_blocking] Found inherited password from group '{}'",
                                group.name
                            );
                            // Merge group overrides
                            if let Some(ref uname) = group.username {
                                creds.username = Some(uname.clone());
                            }
                            if let Some(ref dom) = group.domain {
                                creds.domain = Some(dom.clone());
                            }
                            return Ok(Some(creds));
                        }
                        Ok(None) => {
                            tracing::debug!(
                                "[resolve_credentials_blocking] No password in group '{}'",
                                group.name
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                "[resolve_credentials_blocking] Backend error for group '{}': {}",
                                group.name,
                                e
                            );
                        }
                    }
                } else if group.password_source == Some(PasswordSource::Inherit) {
                    tracing::debug!(
                        "[resolve_credentials_blocking] Group '{}' also inherits, continuing to parent",
                        group.name
                    );
                }

                current_group_id = group.parent_id;
            }

            tracing::debug!(
                "[resolve_credentials_blocking] No inherited credentials found in non-KeePass hierarchy"
            );
        }

        // Fall back to the standard resolver for other password sources
        let resolver = CredentialResolver::new(Arc::new(secret_manager), secret_settings);
        let connection = connection.clone();
        let groups = groups.to_vec();

        // Use thread-local runtime (created lazily per thread)
        crate::async_utils::with_runtime(|rt| {
            rt.block_on(async {
                resolver
                    .resolve_with_hierarchy(&connection, &groups)
                    .await
                    .map_err(|e| format!("Failed to resolve credentials: {e}"))
            })
        })?
    }

    // ========== Settings Operations ==========

    /// Gets the current settings
    pub const fn settings(&self) -> &AppSettings {
        &self.settings
    }

    /// Gets mutable reference to settings for in-place modifications
    ///
    /// Note: After modifying, call `save_settings()` to persist changes.
    pub fn settings_mut(&mut self) -> &mut AppSettings {
        &mut self.settings
    }

    /// Saves current settings to disk
    ///
    /// # Errors
    ///
    /// Returns an error if settings cannot be saved.
    pub fn save_settings(&self) -> Result<(), String> {
        self.config_manager
            .save_settings(&self.settings)
            .map_err(|e| format!("Failed to save settings: {e}"))
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

        // Encrypt Bitwarden password before saving if present
        if settings.secrets.bitwarden_password.is_some() {
            settings.secrets.encrypt_bitwarden_password();
        }

        // Encrypt Bitwarden API credentials before saving if present
        if settings.secrets.bitwarden_client_id.is_some()
            || settings.secrets.bitwarden_client_secret.is_some()
        {
            settings.secrets.encrypt_bitwarden_api_credentials();
        }

        self.config_manager
            .save_settings(&settings)
            .map_err(|e| format!("Failed to save settings: {e}"))?;

        // Update session manager logging
        if settings.logging.enabled != self.settings.logging.enabled {
            self.session_manager
                .set_logging_enabled(settings.logging.enabled);
        }

        // Rebuild secret manager backends if secret settings changed
        if self.settings.secrets != settings.secrets {
            self.secret_manager.rebuild_from_settings(&settings.secrets);
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

            // Sanitize imported values — strip trailing escape sequences
            // (e.g. literal \n from Remmina INI files)
            connection.name = rustconn_core::import::sanitize_imported_value(&connection.name);
            connection.host = rustconn_core::import::sanitize_imported_value(&connection.host);
            if let Some(ref username) = connection.username {
                let clean = rustconn_core::import::sanitize_imported_value(username);
                connection.username = if clean.is_empty() { None } else { Some(clean) };
            }

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
                if !subgroup_map.contains_key(&group_name)
                    && let Some(parent_id) = parent_group_id
                    && let Ok(id) = self
                        .connection_manager
                        .create_group_with_parent(group_name.clone(), parent_id)
                {
                    subgroup_map.insert(group_name.clone(), id);
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
                Err(e) => tracing::warn!(name = %conn.name, %e, "Failed to import connection"),
            }
        }

        // Store imported credentials using synchronous secret-tool calls.
        // We avoid the async LibSecretBackend here because block_on inside
        // the GTK main thread can deadlock with the D-Bus/GLib main loop
        // that secret-tool relies on.
        if result.has_credentials() {
            let mut stored = 0usize;
            let total = result.credentials.len();

            for (conn_id, creds) in &result.credentials {
                // Build the lookup key in the same "{name} ({protocol})" format
                // that resolve_from_keyring uses for retrieval
                let Some(conn) = self.connection_manager.get_connection(*conn_id) else {
                    tracing::warn!(
                        connection_id = %conn_id,
                        "Skipping credential store: connection not found"
                    );
                    continue;
                };
                let protocol = conn.protocol_config.protocol_type();
                let name = rustconn_core::import::sanitize_imported_value(
                    &conn.name.trim().replace('/', "-"),
                );
                let lookup_key = format!("{} ({})", name, protocol.as_str().to_lowercase());

                match Self::store_credential_sync(&lookup_key, &creds) {
                    Ok(()) => {
                        stored += 1;
                        tracing::debug!(lookup_key, "Stored imported credential");
                    }
                    Err(e) => {
                        tracing::warn!(
                            lookup_key,
                            error = %e,
                            "Failed to store imported credential"
                        );
                    }
                }
            }

            if stored == total {
                tracing::info!("Stored {stored} imported credential(s)");
            } else {
                tracing::warn!("Stored {stored}/{total} imported credential(s)");
            }
        }

        Ok(imported)
    }

    /// Stores a single credential field via synchronous `secret-tool store`.
    ///
    /// Uses `std::process::Command` instead of the async `LibSecretBackend`
    /// to avoid deadlocks when `block_on` is called on the GTK main thread
    /// (the D-Bus calls that `secret-tool` makes can re-enter the GLib main
    /// loop, which is blocked by the tokio runtime).
    fn store_secret_tool_sync(
        lookup_key: &str,
        key: &str,
        value: &str,
        label: &str,
    ) -> Result<(), String> {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let mut child = Command::new("secret-tool")
            .args([
                "store",
                "--label",
                label,
                "application",
                "rustconn",
                "connection_id",
                lookup_key,
                "key",
                key,
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("Failed to spawn secret-tool: {e}"))?;

        if let Some(mut stdin) = child.stdin.take() {
            stdin
                .write_all(value.as_bytes())
                .map_err(|e| format!("Failed to write secret: {e}"))?;
        }
        // stdin is closed here (dropped), signalling EOF to secret-tool

        let output = child
            .wait_with_output()
            .map_err(|e| format!("Failed to wait for secret-tool: {e}"))?;

        if output.status.success() {
            Ok(())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(format!("secret-tool store failed: {stderr}"))
        }
    }

    /// Stores credentials for an imported connection using synchronous I/O.
    fn store_credential_sync(
        lookup_key: &str,
        creds: &rustconn_core::models::Credentials,
    ) -> Result<(), String> {
        let label = format!("RustConn: {lookup_key}");

        if let Some(username) = &creds.username {
            Self::store_secret_tool_sync(lookup_key, "username", username, &label)?;
        }

        if let Some(password) = creds.expose_password() {
            Self::store_secret_tool_sync(lookup_key, "password", password, &label)?;
        }

        if let Some(passphrase) = creds.expose_key_passphrase() {
            Self::store_secret_tool_sync(lookup_key, "key_passphrase", passphrase, &label)?;
        }

        if let Some(domain) = &creds.domain {
            Self::store_secret_tool_sync(lookup_key, "domain", domain, &label)?;
        }

        Ok(())
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
        strength: EncryptionStrength,
    ) -> Result<(), String> {
        self.document_manager
            .save(id, path, password, strength)
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
        if let Some(doc_id) = self.active_document_id
            && let Some(doc) = self.document_manager.get_mut(doc_id)
        {
            doc.add_template(template.clone());
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
        if let Some(doc_id) = self.active_document_id
            && let Some(doc) = self.document_manager.get_mut(doc_id)
        {
            doc.remove_template(id);
            doc.add_template(template.clone());
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
        if let Some(doc_id) = self.active_document_id
            && let Some(doc) = self.document_manager.get_mut(doc_id)
        {
            doc.remove_template(template_id);
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

// ========== Safe State Access Helpers ==========

/// Error type for state access failures
#[derive(Debug, Clone, thiserror::Error)]
#[allow(dead_code)] // Part of safe state access API
pub enum StateAccessError {
    /// State is already borrowed mutably
    #[error("State is already borrowed")]
    AlreadyBorrowed,
    /// State is already borrowed immutably (for mutable access)
    #[error("State is already borrowed immutably")]
    AlreadyBorrowedImmutably,
}

/// Safely accesses the state for reading
///
/// Returns `None` if the state is already borrowed mutably.
/// Use this instead of `state.borrow()` to avoid panics.
///
/// # Example
/// ```ignore
/// if let Some(state_ref) = with_state(&state, |s| s.list_connections().len()) {
///     println!("Connection count: {}", state_ref);
/// }
/// ```
#[must_use]
#[allow(dead_code)] // Part of safe state access API
pub fn with_state<F, R>(state: &SharedAppState, f: F) -> Option<R>
where
    F: FnOnce(&AppState) -> R,
{
    state.try_borrow().ok().map(|s| f(&s))
}

/// Safely accesses the state for reading, with error handling
///
/// Returns `Err(StateAccessError)` if the state is already borrowed mutably.
///
/// # Example
/// ```ignore
/// match try_with_state(&state, |s| s.get_connection(id).cloned()) {
///     Ok(Some(conn)) => { /* use connection */ }
///     Ok(None) => { /* connection not found */ }
///     Err(e) => tracing::warn!("State access failed: {}", e),
/// }
/// ```
#[allow(dead_code)] // Part of safe state access API
pub fn try_with_state<F, R>(state: &SharedAppState, f: F) -> Result<R, StateAccessError>
where
    F: FnOnce(&AppState) -> R,
{
    state
        .try_borrow()
        .map(|s| f(&s))
        .map_err(|_| StateAccessError::AlreadyBorrowed)
}

/// Safely accesses the state for mutation
///
/// Returns `None` if the state is already borrowed.
/// Use this instead of `state.borrow_mut()` to avoid panics.
///
/// # Example
/// ```ignore
/// if with_state_mut(&state, |s| s.update_last_connected(conn_id)).is_none() {
///     tracing::warn!("Could not update last connected - state busy");
/// }
/// ```
#[must_use]
#[allow(dead_code)] // Part of safe state access API
pub fn with_state_mut<F, R>(state: &SharedAppState, f: F) -> Option<R>
where
    F: FnOnce(&mut AppState) -> R,
{
    state.try_borrow_mut().ok().map(|mut s| f(&mut s))
}

/// Safely accesses the state for mutation, with error handling
///
/// Returns `Err(StateAccessError)` if the state is already borrowed.
///
/// # Example
/// ```ignore
/// if let Err(e) = try_with_state_mut(&state, |s| s.cache_credentials(id, user, pass, domain)) {
///     tracing::warn!("Could not cache credentials: {}", e);
/// }
/// ```
#[allow(dead_code)] // Part of safe state access API
pub fn try_with_state_mut<F, R>(state: &SharedAppState, f: F) -> Result<R, StateAccessError>
where
    F: FnOnce(&mut AppState) -> R,
{
    state
        .try_borrow_mut()
        .map(|mut s| f(&mut s))
        .map_err(|_| StateAccessError::AlreadyBorrowedImmutably)
}

/// Shows a toast notification for vault save errors on the active window.
/// Creates a PassBackend from an optional store directory path
///
/// Helper to avoid code duplication when creating PassBackend instances.
/// Converts PathBuf to String if present.
fn create_pass_backend_from_path(pass_store_dir: Option<std::path::PathBuf>) -> rustconn_core::secret::PassBackend {
    rustconn_core::secret::PassBackend::new(
        pass_store_dir.as_ref().map(|p| p.to_string_lossy().to_string())
    )
}

/// Creates a PassBackend from secret settings
///
/// Helper to avoid code duplication when creating PassBackend instances.
/// Extracts and clones pass_store_dir from settings.
pub fn create_pass_backend_from_secret_settings(settings: &rustconn_core::config::SecretSettings) -> rustconn_core::secret::PassBackend {
    create_pass_backend_from_path(settings.pass_store_dir.clone())
}

/// Creates a PassBackend from app settings
///
/// Helper to avoid code duplication when creating PassBackend instances.
/// Extracts and clones pass_store_dir from app settings.
pub fn create_pass_backend(settings: &rustconn_core::config::AppSettings) -> rustconn_core::secret::PassBackend {
    create_pass_backend_from_path(settings.secrets.pass_store_dir.clone())
}

/// Shows an error toast when saving to vault fails.
///
/// Uses `glib::idle_add_local_once` to ensure the toast is shown on the GTK
/// main thread. Falls back to stderr if no active window is found.
fn show_vault_save_error_toast() {
    use gtk4::prelude::*;
    gtk4::glib::idle_add_local_once(|| {
        if let Some(app) = gtk4::gio::Application::default()
            && let Some(gtk_app) = app.downcast_ref::<gtk4::Application>()
            && let Some(window) = gtk_app.active_window()
        {
            crate::toast::show_toast_on_window(
                &window,
                "Failed to save password to vault",
                crate::toast::ToastType::Error,
            );
            return;
        }
        tracing::warn!("Could not show vault save error toast: no active window");
    });
}

/// Saves a connection password to the configured vault backend.
///
/// Dispatches to KeePass (hierarchical) or generic backend (flat key)
/// based on the current settings.
#[allow(clippy::too_many_arguments)]
pub fn save_password_to_vault(
    settings: &rustconn_core::config::AppSettings,
    groups: &[rustconn_core::models::ConnectionGroup],
    conn: Option<&rustconn_core::models::Connection>,
    conn_name: &str,
    conn_host: &str,
    protocol: rustconn_core::models::ProtocolType,
    username: &str,
    password: &str,
    conn_id: uuid::Uuid,
) {
    let protocol_str = protocol.as_str().to_lowercase();

    if settings.secrets.kdbx_enabled
        && matches!(
            settings.secrets.preferred_backend,
            rustconn_core::config::SecretBackendType::KeePassXc
                | rustconn_core::config::SecretBackendType::KdbxFile
        )
    {
        // KeePass backend — use hierarchical path
        if let Some(kdbx_path) = settings.secrets.kdbx_path.clone() {
            let key_file = settings.secrets.kdbx_key_file.clone();
            let entry_name = if let Some(c) = conn {
                let entry_path =
                    rustconn_core::secret::KeePassHierarchy::build_entry_path(c, groups);
                let base_path = entry_path.strip_prefix("RustConn/").unwrap_or(&entry_path);
                format!("{base_path} ({protocol_str})")
            } else {
                format!("{conn_name} ({protocol_str})")
            };
            let username = username.to_string();
            let url = format!("{}://{}", protocol_str, conn_host);
            let pwd = password.to_string();

            crate::utils::spawn_blocking_with_callback(
                move || {
                    let kdbx = std::path::Path::new(&kdbx_path);
                    let key = key_file.as_ref().map(|p| std::path::Path::new(p));
                    rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                        kdbx,
                        None,
                        key,
                        &entry_name,
                        &username,
                        &pwd,
                        Some(&url),
                    )
                },
                move |result| {
                    if let Err(e) = result {
                        tracing::error!("Failed to save password to vault: {e}");
                        show_vault_save_error_toast();
                    } else {
                        tracing::info!("Password saved to vault for connection {conn_id}");
                    }
                },
            );
        }
    } else {
        // Generic backend — dispatch based on preferred_backend
        let lookup_key = format!("{} ({protocol_str})", conn_name.replace('/', "-"),);
        let username = username.to_string();
        let pwd = password.to_string();
        let backend_type = select_backend_for_load(&settings.secrets);
        let secret_settings = settings.secrets.clone();

        crate::utils::spawn_blocking_with_callback(
            move || {
                use rustconn_core::config::SecretBackendType;
                use rustconn_core::secret::SecretBackend;

                let creds = rustconn_core::models::Credentials {
                    username: Some(username),
                    password: Some(secrecy::SecretString::from(pwd)),
                    key_passphrase: None,
                    domain: None,
                };

                match backend_type {
                    SecretBackendType::Bitwarden => crate::async_utils::with_runtime(|rt| {
                        let backend = rt
                            .block_on(rustconn_core::secret::auto_unlock(&secret_settings))
                            .map_err(|e| format!("{e}"))?;
                        rt.block_on(backend.store(&lookup_key, &creds))
                            .map_err(|e| format!("{e}"))
                    })?,
                    SecretBackendType::OnePassword => {
                        let backend = rustconn_core::secret::OnePasswordBackend::new();
                        crate::async_utils::with_runtime(|rt| {
                            rt.block_on(backend.store(&lookup_key, &creds))
                                .map_err(|e| format!("{e}"))
                        })?
                    }
                    SecretBackendType::Passbolt => {
                        let backend = rustconn_core::secret::PassboltBackend::new();
                        crate::async_utils::with_runtime(|rt| {
                            rt.block_on(backend.store(&lookup_key, &creds))
                                .map_err(|e| format!("{e}"))
                        })?
                    }
                    SecretBackendType::Pass => {
                        let backend = create_pass_backend_from_secret_settings(&secret_settings);
                        crate::async_utils::with_runtime(|rt| {
                            rt.block_on(backend.store(&lookup_key, &creds))
                                .map_err(|e| format!("{e}"))
                        })?
                    }
                    SecretBackendType::LibSecret | SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => {
                        let backend = rustconn_core::secret::LibSecretBackend::new("rustconn");
                        crate::async_utils::with_runtime(|rt| {
                            rt.block_on(backend.store(&lookup_key, &creds))
                                .map_err(|e| format!("{e}"))
                        })?
                    }
                }
            },
            move |result: Result<(), String>| {
                if let Err(e) = result {
                    tracing::error!("Failed to save password to vault: {e}");
                    show_vault_save_error_toast();
                } else {
                    tracing::info!("Password saved to vault for connection {conn_id}");
                }
            },
        );
    }
}

/// Saves a group password to the configured vault backend.
pub fn save_group_password_to_vault(
    settings: &rustconn_core::config::AppSettings,
    group_path: &str,
    lookup_key: &str,
    username: &str,
    password: &str,
) {
    if settings.secrets.kdbx_enabled
        && matches!(
            settings.secrets.preferred_backend,
            rustconn_core::config::SecretBackendType::KeePassXc
                | rustconn_core::config::SecretBackendType::KdbxFile
        )
    {
        if let Some(kdbx_path) = settings.secrets.kdbx_path.clone() {
            let key_file = settings.secrets.kdbx_key_file.clone();
            let entry_name = group_path
                .strip_prefix("RustConn/")
                .unwrap_or(group_path)
                .to_string();
            let username_val = username.to_string();
            let password_val = password.to_string();

            crate::utils::spawn_blocking_with_callback(
                move || {
                    let kdbx = std::path::Path::new(&kdbx_path);
                    let key = key_file.as_ref().map(|p| std::path::Path::new(p));
                    rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                        kdbx,
                        None,
                        key,
                        &entry_name,
                        &username_val,
                        &password_val,
                        None,
                    )
                },
                move |result| {
                    if let Err(e) = result {
                        tracing::error!("Failed to save group password to vault: {e}");
                        show_vault_save_error_toast();
                    } else {
                        tracing::info!("Group password saved to vault");
                    }
                },
            );
        }
    } else {
        let lookup_key = lookup_key.to_string();
        let username_val = username.to_string();
        let password_val = password.to_string();
        let backend_type = select_backend_for_load(&settings.secrets);
        let secret_settings = settings.secrets.clone();

        crate::utils::spawn_blocking_with_callback(
            move || {
                use rustconn_core::config::SecretBackendType;
                use rustconn_core::secret::SecretBackend;

                let creds = rustconn_core::models::Credentials {
                    username: Some(username_val),
                    password: Some(secrecy::SecretString::from(password_val)),
                    key_passphrase: None,
                    domain: None,
                };

                match backend_type {
                    SecretBackendType::Bitwarden => crate::async_utils::with_runtime(|rt| {
                        let backend = rt
                            .block_on(rustconn_core::secret::auto_unlock(&secret_settings))
                            .map_err(|e| format!("{e}"))?;
                        rt.block_on(backend.store(&lookup_key, &creds))
                            .map_err(|e| format!("{e}"))
                    })?,
                    SecretBackendType::OnePassword => {
                        let backend = rustconn_core::secret::OnePasswordBackend::new();
                        crate::async_utils::with_runtime(|rt| {
                            rt.block_on(backend.store(&lookup_key, &creds))
                                .map_err(|e| format!("{e}"))
                        })?
                    }
                    SecretBackendType::Passbolt => {
                        let backend = rustconn_core::secret::PassboltBackend::new();
                        crate::async_utils::with_runtime(|rt| {
                            rt.block_on(backend.store(&lookup_key, &creds))
                                .map_err(|e| format!("{e}"))
                        })?
                    }
                    SecretBackendType::Pass => {
                        let backend = create_pass_backend_from_secret_settings(&secret_settings);
                        crate::async_utils::with_runtime(|rt| {
                            rt.block_on(backend.store(&lookup_key, &creds))
                                .map_err(|e| format!("{e}"))
                        })?
                    }
                    SecretBackendType::LibSecret | SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => {
                        let backend = rustconn_core::secret::LibSecretBackend::new("rustconn");
                        crate::async_utils::with_runtime(|rt| {
                            rt.block_on(backend.store(&lookup_key, &creds))
                                .map_err(|e| format!("{e}"))
                        })?
                    }
                }
            },
            move |result: Result<(), String>| {
                if let Err(e) = result {
                    tracing::error!("Failed to save group password to vault: {e}");
                    show_vault_save_error_toast();
                } else {
                    tracing::info!("Group password saved to vault");
                }
            },
        );
    }
}

/// Renames a credential in the configured vault backend when a connection
/// is renamed.
pub fn rename_vault_credential(
    settings: &rustconn_core::config::AppSettings,
    groups: &[rustconn_core::models::ConnectionGroup],
    updated_conn: &rustconn_core::models::Connection,
    old_name: &str,
    protocol_str: &str,
) -> Result<(), String> {
    if settings.secrets.kdbx_enabled
        && matches!(
            settings.secrets.preferred_backend,
            rustconn_core::config::SecretBackendType::KeePassXc
                | rustconn_core::config::SecretBackendType::KdbxFile
        )
    {
        // KeePass — rename hierarchical entry
        let mut old_conn = updated_conn.clone();
        old_conn.name = old_name.to_string();
        let old_base = rustconn_core::secret::KeePassHierarchy::build_entry_path(&old_conn, groups);
        let new_base =
            rustconn_core::secret::KeePassHierarchy::build_entry_path(updated_conn, groups);
        let old_key = format!("{old_base} ({protocol_str})");
        let new_key = format!("{new_base} ({protocol_str})");

        if old_key == new_key {
            return Ok(());
        }

        if let Some(kdbx_path) = settings.secrets.kdbx_path.as_ref() {
            let key_file = settings.secrets.kdbx_key_file.clone();
            rustconn_core::secret::KeePassStatus::rename_entry_in_kdbx(
                std::path::Path::new(kdbx_path),
                None,
                key_file.as_ref().map(|p| std::path::Path::new(p)),
                &old_key,
                &new_key,
            )
            .map_err(|e| format!("{e}"))
        } else {
            Ok(())
        }
    } else {
        // Generic backend — rename flat key based on preferred_backend
        use rustconn_core::config::SecretBackendType;
        use rustconn_core::secret::SecretBackend;

        let old_key = format!("{} ({protocol_str})", old_name.replace('/', "-"),);
        let new_key = format!("{} ({protocol_str})", updated_conn.name.replace('/', "-"),);

        if old_key == new_key {
            return Ok(());
        }

        let backend_type = select_backend_for_load(&settings.secrets);
        let secret_settings = settings.secrets.clone();

        match backend_type {
            SecretBackendType::Bitwarden => crate::async_utils::with_runtime(|rt| {
                let backend = rt
                    .block_on(rustconn_core::secret::auto_unlock(&secret_settings))
                    .map_err(|e| format!("{e}"))?;
                let creds = rt
                    .block_on(backend.retrieve(&old_key))
                    .map_err(|e| format!("{e}"))?;
                if let Some(creds) = creds {
                    rt.block_on(backend.store(&new_key, &creds))
                        .map_err(|e| format!("{e}"))?;
                    let _ = rt.block_on(backend.delete(&old_key));
                }
                Ok(())
            })?,
            SecretBackendType::OnePassword => {
                let backend = rustconn_core::secret::OnePasswordBackend::new();
                crate::async_utils::with_runtime(|rt| {
                    let creds = rt
                        .block_on(backend.retrieve(&old_key))
                        .map_err(|e| format!("{e}"))?;
                    if let Some(creds) = creds {
                        rt.block_on(backend.store(&new_key, &creds))
                            .map_err(|e| format!("{e}"))?;
                        let _ = rt.block_on(backend.delete(&old_key));
                    }
                    Ok(())
                })?
            }
            SecretBackendType::Passbolt => {
                let backend = rustconn_core::secret::PassboltBackend::new();
                crate::async_utils::with_runtime(|rt| {
                    let creds = rt
                        .block_on(backend.retrieve(&old_key))
                        .map_err(|e| format!("{e}"))?;
                    if let Some(creds) = creds {
                        rt.block_on(backend.store(&new_key, &creds))
                            .map_err(|e| format!("{e}"))?;
                        let _ = rt.block_on(backend.delete(&old_key));
                    }
                    Ok(())
                })?
            }
            _ => {
                let backend = rustconn_core::secret::LibSecretBackend::new("rustconn");
                crate::async_utils::with_runtime(|rt| {
                    let creds = rt
                        .block_on(backend.retrieve(&old_key))
                        .map_err(|e| format!("{e}"))?;
                    if let Some(creds) = creds {
                        rt.block_on(backend.store(&new_key, &creds))
                            .map_err(|e| format!("{e}"))?;
                        let _ = rt.block_on(backend.delete(&old_key));
                    }
                    Ok(())
                })?
            }
        }
    }
}

/// Saves a secret variable value to the configured vault backend.
///
/// Respects `preferred_backend` from secret settings, using the same
/// backend selection logic as connection passwords.
pub fn save_variable_to_vault(
    settings: &rustconn_core::config::SecretSettings,
    var_name: &str,
    password: &str,
) -> Result<(), String> {
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::secret::SecretBackend;

    let lookup_key = rustconn_core::variable_secret_key(var_name);
    let backend_type = select_backend_for_load(settings);

    tracing::debug!(?backend_type, var_name, "Saving secret variable to vault");

    let creds = rustconn_core::models::Credentials {
        username: None,
        password: Some(secrecy::SecretString::from(password.to_string())),
        key_passphrase: None,
        domain: None,
    };

    match backend_type {
        SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
            if let Some(kdbx_path) = settings.kdbx_path.as_ref() {
                let key_file = settings.kdbx_key_file.clone();
                let kdbx = std::path::Path::new(kdbx_path);
                let key = key_file.as_ref().map(|p| std::path::Path::new(p));
                rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                    kdbx,
                    None,
                    key,
                    &lookup_key,
                    "",
                    password,
                    None,
                )
                .map_err(|e| format!("{e}"))
            } else {
                Err("KeePass enabled but no database file configured".to_string())
            }
        }
        SecretBackendType::Bitwarden => {
            let secret_settings = settings.clone();
            crate::async_utils::with_runtime(|rt| {
                let backend = rt
                    .block_on(rustconn_core::secret::auto_unlock(&secret_settings))
                    .map_err(|e| format!("{e}"))?;
                rt.block_on(backend.store(&lookup_key, &creds))
                    .map_err(|e| format!("{e}"))
            })?
        }
        SecretBackendType::OnePassword => {
            let backend = rustconn_core::secret::OnePasswordBackend::new();
            crate::async_utils::with_runtime(|rt| {
                rt.block_on(backend.store(&lookup_key, &creds))
                    .map_err(|e| format!("{e}"))
            })?
        }
        SecretBackendType::Passbolt => {
            let backend = rustconn_core::secret::PassboltBackend::new();
            crate::async_utils::with_runtime(|rt| {
                rt.block_on(backend.store(&lookup_key, &creds))
                    .map_err(|e| format!("{e}"))
            })?
        }
        SecretBackendType::Pass => {
            let backend = create_pass_backend_from_secret_settings(&settings);
            crate::async_utils::with_runtime(|rt| {
                rt.block_on(backend.store(&lookup_key, &creds))
                    .map_err(|e| format!("{e}"))
            })?
        }
        SecretBackendType::LibSecret => {
            let backend = rustconn_core::secret::LibSecretBackend::new("rustconn");
            crate::async_utils::with_runtime(|rt| {
                rt.block_on(backend.store(&lookup_key, &creds))
                    .map_err(|e| format!("{e}"))
            })?
        }
    }
}

/// Loads a secret variable value from the configured vault backend.
///
/// Respects `preferred_backend` from secret settings, using the same
/// backend selection logic as connection passwords.
pub fn load_variable_from_vault(
    settings: &rustconn_core::config::SecretSettings,
    var_name: &str,
) -> Result<Option<String>, String> {
    use rustconn_core::config::SecretBackendType;
    use rustconn_core::secret::SecretBackend;
    use secrecy::ExposeSecret;

    let lookup_key = rustconn_core::variable_secret_key(var_name);
    let backend_type = select_backend_for_load(settings);

    tracing::debug!(
        ?backend_type,
        var_name,
        "Loading secret variable from vault"
    );

    match backend_type {
        SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
            if let Some(kdbx_path) = settings.kdbx_path.as_ref() {
                let key_file = settings.kdbx_key_file.clone();
                let kdbx = std::path::Path::new(kdbx_path);
                let key = key_file.as_ref().map(|p| std::path::Path::new(p));
                rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                    kdbx,
                    None,
                    key,
                    &lookup_key,
                    None,
                )
                .map(|opt| opt.map(|s| s.expose_secret().to_string()))
                .map_err(|e| format!("{e}"))
            } else {
                Err("KeePass enabled but no database file configured".to_string())
            }
        }
        SecretBackendType::Bitwarden => {
            let secret_settings = settings.clone();
            crate::async_utils::with_runtime(|rt| {
                let backend = rt
                    .block_on(rustconn_core::secret::auto_unlock(&secret_settings))
                    .map_err(|e| format!("{e}"))?;
                let creds = rt
                    .block_on(backend.retrieve(&lookup_key))
                    .map_err(|e| format!("{e}"))?;
                Ok(creds.and_then(|c| c.expose_password().map(String::from)))
            })?
        }
        SecretBackendType::OnePassword => {
            let backend = rustconn_core::secret::OnePasswordBackend::new();
            crate::async_utils::with_runtime(|rt| {
                let creds = rt
                    .block_on(backend.retrieve(&lookup_key))
                    .map_err(|e| format!("{e}"))?;
                Ok(creds.and_then(|c| c.expose_password().map(String::from)))
            })?
        }
        SecretBackendType::Passbolt => {
            let backend = rustconn_core::secret::PassboltBackend::new();
            crate::async_utils::with_runtime(|rt| {
                let creds = rt
                    .block_on(backend.retrieve(&lookup_key))
                    .map_err(|e| format!("{e}"))?;
                Ok(creds.and_then(|c| c.expose_password().map(String::from)))
            })?
        }
        SecretBackendType::Pass => {
            let backend = create_pass_backend_from_secret_settings(&settings);
            crate::async_utils::with_runtime(|rt| {
                let creds = rt
                    .block_on(backend.retrieve(&lookup_key))
                    .map_err(|e| format!("{e}"))?;
                Ok(creds.and_then(|c| c.expose_password().map(String::from)))
            })?
        }
        SecretBackendType::LibSecret => {
            let backend = rustconn_core::secret::LibSecretBackend::new("rustconn");
            crate::async_utils::with_runtime(|rt| {
                let creds = rt
                    .block_on(backend.retrieve(&lookup_key))
                    .map_err(|e| format!("{e}"))?;
                Ok(creds.and_then(|c| c.expose_password().map(String::from)))
            })?
        }
    }
}

/// Selects the appropriate storage backend for variable secrets.
///
/// Mirrors `CredentialResolver::select_storage_backend` logic.
/// Also used by connection password load/save and variable vault operations.
pub fn select_backend_for_load(
    secrets: &rustconn_core::config::SecretSettings,
) -> rustconn_core::config::SecretBackendType {
    use rustconn_core::config::SecretBackendType;

    match secrets.preferred_backend {
        SecretBackendType::Bitwarden => SecretBackendType::Bitwarden,
        SecretBackendType::OnePassword => SecretBackendType::OnePassword,
        SecretBackendType::Passbolt => SecretBackendType::Passbolt,
        SecretBackendType::Pass => SecretBackendType::Pass,
        SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => {
            if secrets.kdbx_enabled && secrets.kdbx_path.is_some() {
                SecretBackendType::KdbxFile
            } else if secrets.enable_fallback {
                SecretBackendType::LibSecret
            } else {
                secrets.preferred_backend
            }
        }
        SecretBackendType::LibSecret => SecretBackendType::LibSecret,
    }
}
