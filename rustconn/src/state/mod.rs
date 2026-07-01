//! Application state management
//!
//! This module provides the central application state that holds all managers
//! and provides thread-safe access to core functionality.

mod connections;
mod sessions;
mod sync;

use crate::async_utils::with_runtime;
use chrono::Utc;
use rustconn_core::automation::FolderConnectionTracker;
use rustconn_core::cluster::ClusterManager;
use rustconn_core::config::{AppSettings, ConfigManager};
use rustconn_core::connection::ConnectionManager;
use rustconn_core::document::{Document, DocumentManager, EncryptionStrength};
use rustconn_core::models::{
    Connection, ConnectionGroup, ConnectionHistoryEntry, Credentials, PasswordSource,
};
use rustconn_core::secret::{CredentialResolver, SecretManager};
use rustconn_core::session::SessionManager;
use rustconn_core::snippet::SnippetManager;
use rustconn_core::sync::SyncManager;
use rustconn_core::template::TemplateManager;
use rustconn_core::workspace::WorkspaceProfileManager;
use secrecy::SecretString;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
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

    /// Gets a reference to the original copied connection (before paste transforms it).
    #[must_use]
    pub fn original_connection(&self) -> Option<&Connection> {
        self.connection.as_ref()
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
    /// Template manager for connection templates
    template_manager: TemplateManager,
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
    /// Whether `history_entries` has unsaved changes (see `mark_history_dirty`)
    history_dirty: std::cell::Cell<bool>,
    /// Wakes the debounced history flusher in `app.rs`; `None` until the
    /// flusher is installed (or in tests), in which case saves are immediate
    history_dirty_tx: Option<async_channel::Sender<()>>,
    /// Cached secret backend availability (updated on init and settings change)
    secret_backend_available: Option<bool>,
    /// Cached fine-grained availability of the *preferred* secret backend.
    /// Distinguishes a missing client from an unresponsive Secret Service so
    /// the startup check can surface an accurate, actionable warning (#201).
    secret_backend_availability: Option<rustconn_core::secret::BackendAvailability>,
    /// Cloud Sync manager for export/import operations
    sync_manager: SyncManager,
    /// Simple Sync deletion tombstones (only populated when Simple Sync is on)
    tombstones: Vec<rustconn_core::sync::Tombstone>,
    /// Set when local data changed and Simple Sync should re-export (debounced
    /// by the poll loop in `app.rs`). Ignored when Simple Sync is disabled.
    simple_sync_dirty: std::cell::Cell<bool>,
    /// Workspace profile manager for named session sets
    workspace_manager: WorkspaceProfileManager,
    /// Shared folder connection tracker for conditional task execution
    folder_tracker: Arc<std::sync::Mutex<FolderConnectionTracker>>,
    /// Whether the KeePass keyring load at startup failed or timed out.
    /// Checked once after the main window is shown to display a toast.
    kdbx_keyring_failed: bool,
}

/// Bundles the parameters needed for blocking credential resolution.
///
/// This avoids `clippy::too_many_arguments` on `resolve_credentials_blocking`.
struct CredentialResolutionContext {
    connection: Connection,
    groups: Vec<ConnectionGroup>,
    kdbx_enabled: bool,
    kdbx_path: Option<std::path::PathBuf>,
    kdbx_password: Option<SecretString>,
    kdbx_key_file: Option<std::path::PathBuf>,
    secret_settings: rustconn_core::config::SecretSettings,
    secret_manager: SecretManager,
    global_variables: Vec<rustconn_core::Variable>,
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
        let mut kdbx_keyring_failed = false;
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

                // If password still not available and user chose system keyring storage,
                // load it from keyring now. This is typically a fast local D-Bus
                // call (~10ms), but on cold boot (daemon not started) or with KWallet
                // it may block longer. Use a 5-second timeout to avoid delaying startup.
                // Without this, connections using KeePass vault cannot resolve credentials
                // until the user opens Settings (where keyring loading previously lived).
                // Guard: skip if kdbx_path doesn't exist (file deleted, USB detached) —
                // no point holding a password in memory for an unreachable database.
                if settings.secrets.kdbx_password.is_none()
                    && settings.secrets.kdbx_save_to_keyring
                    && settings
                        .secrets
                        .kdbx_path
                        .as_ref()
                        .is_some_and(|p| p.exists())
                {
                    match with_runtime(|rt| {
                        rt.block_on(async {
                            tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                rustconn_core::secret::get_kdbx_password_from_keyring(),
                            )
                            .await
                        })
                    }) {
                        Ok(Ok(Ok(Some(password)))) => {
                            settings.secrets.kdbx_password = Some(password);
                            tracing::info!("KeePass password restored from system keyring");
                        }
                        Ok(Ok(Ok(None))) => {
                            tracing::warn!(
                                "KeePass password not found in system keyring — \
                                 user may need to re-enter it in Settings"
                            );
                            kdbx_keyring_failed = true;
                        }
                        Ok(Ok(Err(e))) => {
                            tracing::warn!(
                                error = %e,
                                "Failed to load KeePass password from system keyring"
                            );
                            kdbx_keyring_failed = true;
                        }
                        Ok(Err(_elapsed)) => {
                            tracing::warn!(
                                "Keyring query timed out after 5s — \
                                 KeePass credentials will be loaded when Settings is opened"
                            );
                            kdbx_keyring_failed = true;
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "Runtime error loading KeePass password from keyring"
                            );
                            kdbx_keyring_failed = true;
                        }
                    }
                }
            }
        }

        // Note: Bitwarden password decryption and vault auto-unlock are deferred
        // to startup which runs asynchronously after the
        // main window is presented. This avoids blocking the UI on startup.

        // Decrypt 1Password / Passbolt credentials at startup (decrypt is ~instant).
        // Only for the preferred backend — lazy init principle.
        match settings.secrets.preferred_backend {
            rustconn_core::config::SecretBackendType::OnePassword => {
                if settings
                    .secrets
                    .onepassword_service_account_token_encrypted
                    .is_some()
                    && settings.secrets.decrypt_onepassword_token()
                {
                    tracing::info!(
                        "1Password service account token restored from encrypted storage"
                    );
                }
                // If token still not available and keyring storage is configured,
                // load from keyring with a 5s timeout (same pattern as KeePass above).
                if settings.secrets.onepassword_service_account_token.is_none()
                    && settings.secrets.onepassword_save_to_keyring
                {
                    match with_runtime(|rt| {
                        rt.block_on(async {
                            tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                rustconn_core::secret::get_token_from_keyring(),
                            )
                            .await
                        })
                    }) {
                        Ok(Ok(Ok(Some(token)))) => {
                            settings.secrets.onepassword_service_account_token = Some(token);
                            tracing::info!(
                                "1Password service account token restored from system keyring"
                            );
                        }
                        Ok(Ok(Ok(None))) => {
                            tracing::debug!("No 1Password token found in system keyring");
                        }
                        Ok(Ok(Err(e))) => {
                            tracing::warn!(
                                error = %e,
                                "Failed to load 1Password token from system keyring"
                            );
                        }
                        Ok(Err(_elapsed)) => {
                            tracing::warn!(
                                "Keyring query timed out after 5s — \
                                 1Password token will be loaded when Settings is opened"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "Runtime error loading 1Password token from keyring"
                            );
                        }
                    }
                }
            }
            rustconn_core::config::SecretBackendType::Passbolt => {
                if settings.secrets.passbolt_passphrase_encrypted.is_some()
                    && settings.secrets.decrypt_passbolt_passphrase()
                {
                    tracing::info!("Passbolt passphrase restored from encrypted storage");
                }
                // If passphrase still not available and keyring storage is configured,
                // load from keyring with a 5s timeout.
                if settings.secrets.passbolt_passphrase.is_none()
                    && settings.secrets.passbolt_save_to_keyring
                {
                    match with_runtime(|rt| {
                        rt.block_on(async {
                            tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                rustconn_core::secret::get_passphrase_from_keyring(),
                            )
                            .await
                        })
                    }) {
                        Ok(Ok(Ok(Some(passphrase)))) => {
                            settings.secrets.passbolt_passphrase = Some(passphrase);
                            tracing::info!("Passbolt passphrase restored from system keyring");
                        }
                        Ok(Ok(Ok(None))) => {
                            tracing::debug!("No Passbolt passphrase found in system keyring");
                        }
                        Ok(Ok(Err(e))) => {
                            tracing::warn!(
                                error = %e,
                                "Failed to load Passbolt passphrase from system keyring"
                            );
                        }
                        Ok(Err(_elapsed)) => {
                            tracing::warn!(
                                "Keyring query timed out after 5s — \
                                 Passbolt passphrase will be loaded when Settings is opened"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "Runtime error loading Passbolt passphrase from keyring"
                            );
                        }
                    }
                }
            }
            _ => {
                // Bitwarden: handled in app.rs idle_add_local_once
                // KeePass: handled above
                // LibSecret, Pass, macOS Keychain: stateless
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

        // Initialize template manager
        let template_manager = TemplateManager::new(config_manager.clone())
            .map_err(|e| format!("Failed to initialize template manager: {e}"))?;

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

        // Initialize Cloud Sync manager
        let sync_manager = SyncManager::new(settings.sync.clone());

        // Load Simple Sync tombstones
        let tombstones = config_manager.load_tombstones().unwrap_or_default();

        // Initialize Workspace Profile manager
        let workspace_manager = WorkspaceProfileManager::new(config_manager.clone())
            .unwrap_or_else(|e| {
                tracing::warn!("Failed to load workspace profiles: {e}");
                WorkspaceProfileManager::new_empty(config_manager.clone())
            });

        Ok(Self {
            connection_manager,
            session_manager,
            snippet_manager,
            template_manager,
            secret_manager,
            config_manager,
            document_manager,
            cluster_manager,
            active_document_id: None,
            settings,
            password_cache: HashMap::new(),
            clipboard: ConnectionClipboard::new(),
            history_entries,
            history_dirty: std::cell::Cell::new(false),
            history_dirty_tx: None,
            secret_backend_available: None,
            secret_backend_availability: None,
            sync_manager,
            tombstones,
            simple_sync_dirty: std::cell::Cell::new(false),
            workspace_manager,
            folder_tracker: Arc::new(std::sync::Mutex::new(FolderConnectionTracker::new())),
            kdbx_keyring_failed,
        })
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

    /// Checks if any secret backend is available (uses cache if available)
    ///
    /// Used internally by `resolve_credentials_blocking` and `resolve_credentials_gtk`.
    /// Includes a 5-second timeout to prevent blocking the GTK main thread
    /// if the backend is unresponsive.
    pub fn has_secret_backend(&self) -> bool {
        if let Some(cached) = self.secret_backend_available {
            return cached;
        }
        let secret_manager = self.secret_manager.clone();

        with_runtime(|rt| {
            rt.block_on(async {
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    secret_manager.is_available(),
                )
                .await
                .unwrap_or(false)
            })
        })
        .unwrap_or(false)
    }

    /// Refreshes the cached secret backend availability
    ///
    /// Call this after settings changes
    /// that affect the secret backend configuration.
    /// Includes a 5-second timeout to prevent blocking the GTK main thread
    /// if the backend is unresponsive.
    pub fn refresh_secret_backend_cache(&mut self) {
        let secret_manager = self.secret_manager.clone();
        let (available, availability) = with_runtime(|rt| {
            rt.block_on(async {
                // Fine-grained availability of the preferred backend, used by
                // the startup warning to distinguish a missing client from an
                // unresponsive Secret Service (#201).
                let availability = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    secret_manager.primary_availability(),
                )
                .await
                .unwrap_or(rustconn_core::secret::BackendAvailability::ServiceUnavailable);
                // Whether any backend can store secrets, used to gate
                // credential resolution.
                let available = tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    secret_manager.is_available(),
                )
                .await
                .unwrap_or(false);
                (available, availability)
            })
        })
        .unwrap_or((
            false,
            rustconn_core::secret::BackendAvailability::ServiceUnavailable,
        ));
        self.secret_backend_available = Some(available);
        self.secret_backend_availability = Some(availability);
    }

    /// Returns the cached fine-grained availability of the preferred secret
    /// backend, or `None` if it has not been probed yet.
    ///
    /// Populated by [`Self::refresh_secret_backend_cache`]; used by the startup
    /// check to surface an accurate warning when the keyring cannot work.
    #[must_use]
    pub fn secret_backend_availability(
        &self,
    ) -> Option<rustconn_core::secret::BackendAvailability> {
        self.secret_backend_availability.clone()
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
        F: FnOnce(Result<rustconn_core::sync::CredentialResolutionResult, String>) + 'static,
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
        let global_variables = self.settings.global_variables.clone();

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
                Self::resolve_credentials_blocking(CredentialResolutionContext {
                    connection,
                    groups,
                    kdbx_enabled,
                    kdbx_path,
                    kdbx_password,
                    kdbx_key_file,
                    secret_settings,
                    secret_manager,
                    global_variables,
                })
            },
            callback,
        );
    }

    /// Resolves a connection's password synchronously, returning only the
    /// secret (or `None` when unset/empty).
    ///
    /// Shares the exact resolution logic of
    /// [`Self::resolve_credentials_blocking`] — every `PasswordSource` (Vault,
    /// Variable, Inherit, Script) and every backend (KeePassXC/KDBX included) —
    /// so out-of-band consumers such as the SSH jump-host bastion resolver stay
    /// in lockstep with normal connection login and never diverge on lookup key
    /// or backend (issue #191).
    ///
    /// Performs a blocking vault call; GTK-thread callers MUST NOT hold any
    /// other `AppState` borrow across it. An empty password resolves to `None`.
    pub(crate) fn resolve_connection_password_blocking(
        &self,
        connection: &Connection,
    ) -> Option<SecretString> {
        use rustconn_core::sync::CredentialResolutionResult;
        use secrecy::ExposeSecret;

        let groups: Vec<ConnectionGroup> = self
            .connection_manager
            .list_groups()
            .iter()
            .cloned()
            .cloned()
            .collect();

        let ctx = CredentialResolutionContext {
            connection: connection.clone(),
            groups,
            kdbx_enabled: self.settings.secrets.kdbx_enabled,
            kdbx_path: self.settings.secrets.kdbx_path.clone(),
            kdbx_password: self.settings.secrets.kdbx_password.clone(),
            kdbx_key_file: self.settings.secrets.kdbx_key_file.clone(),
            secret_settings: self.settings.secrets.clone(),
            secret_manager: self.secret_manager.clone(),
            global_variables: self.settings.global_variables.clone(),
        };

        match Self::resolve_credentials_blocking(ctx) {
            Ok(CredentialResolutionResult::Resolved(creds)) => {
                creds.password.filter(|p| !p.expose_secret().is_empty())
            }
            _ => None,
        }
    }

    /// Internal blocking credential resolution (runs in background thread)
    ///
    /// This is extracted from `resolve_credentials` to be callable from a background
    /// thread without needing `&self`.
    ///
    /// Returns a [`CredentialResolutionResult`] that the UI layer uses to show
    /// the appropriate dialog (variable setup, backend missing, etc.) instead
    /// of silently returning `None`.
    fn resolve_credentials_blocking(
        ctx: CredentialResolutionContext,
    ) -> Result<rustconn_core::sync::CredentialResolutionResult, String> {
        use rustconn_core::secret::{KeePassHierarchy, KeePassStatus};
        use rustconn_core::sync::CredentialResolutionResult;
        use secrecy::ExposeSecret;

        let connection = &ctx.connection;
        let groups = &ctx.groups;
        let kdbx_enabled = ctx.kdbx_enabled;
        let kdbx_path = ctx.kdbx_path;
        let kdbx_password = ctx.kdbx_password;
        let kdbx_key_file = ctx.kdbx_key_file;
        let secret_settings = ctx.secret_settings;
        let secret_manager = ctx.secret_manager;

        // For Variable password source — resolve directly via vault backend
        if let PasswordSource::Variable(ref var_name) = connection.password_source {
            tracing::debug!(
                var_name,
                "[resolve_credentials_blocking] Resolving variable password"
            );
            // Look up the variable's custom kdbx_entry_path if configured
            let kdbx_entry_path = ctx
                .global_variables
                .iter()
                .find(|v| v.name == *var_name)
                .and_then(|v| v.kdbx_entry_path.as_deref());
            let vault_entry_name = ctx
                .global_variables
                .iter()
                .find(|v| v.name == *var_name)
                .and_then(|v| v.vault_entry_name.as_deref());
            match load_variable_from_vault_with_path(
                &secret_settings,
                var_name,
                kdbx_entry_path,
                vault_entry_name,
            ) {
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
                    return Ok(CredentialResolutionResult::Resolved(creds));
                }
                Ok(None) => {
                    tracing::warn!(
                        var_name,
                        "[resolve_credentials_blocking] No secret found for variable"
                    );
                    // Variable exists but has no value on this device
                    return Ok(CredentialResolutionResult::VariableMissing {
                        variable_name: var_name.clone(),
                        description: None,
                        is_secret: true,
                    });
                }
                Err(e) => {
                    tracing::error!(
                        var_name,
                        error = %e,
                        "[resolve_credentials_blocking] Failed to load variable from vault"
                    );
                    // Backend may not be configured
                    return Ok(CredentialResolutionResult::VariableMissing {
                        variable_name: var_name.clone(),
                        description: None,
                        is_secret: true,
                    });
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
                    return Ok(CredentialResolutionResult::Resolved(creds));
                }
                Ok(None) => {
                    tracing::debug!("[resolve_credentials_blocking] No password found in KeePass");
                }
                Err(e) => {
                    tracing::error!("[resolve_credentials_blocking] KeePass error: {}", e);
                }
            }
        }

        // For Vault password source with non-KeePass backends (Bitwarden, 1Password, etc.)
        // Use dispatch_vault_op which calls auto_unlock to ensure the vault is accessible.
        if connection.password_source == PasswordSource::Vault
            && !matches!(
                secret_settings.preferred_backend,
                rustconn_core::config::SecretBackendType::KeePassXc
                    | rustconn_core::config::SecretBackendType::KdbxFile
            )
        {
            let backend_type = select_backend_for_load(&secret_settings);
            let lookup_key = generate_store_key(
                &connection.name,
                &connection.host,
                &connection
                    .protocol_config
                    .protocol_type()
                    .as_str()
                    .to_lowercase(),
                backend_type,
            );

            tracing::debug!(
                lookup_key = %lookup_key,
                ?backend_type,
                "[resolve_credentials_blocking] Vault (non-KeePass): resolving"
            );

            match dispatch_vault_op(&secret_settings, &lookup_key, VaultOp::Retrieve) {
                Ok(Some(creds)) => {
                    tracing::debug!("[resolve_credentials_blocking] Found password in vault");
                    return Ok(CredentialResolutionResult::Resolved(creds));
                }
                Ok(None) => {
                    tracing::debug!("[resolve_credentials_blocking] No password found in vault");
                    // Vault entry not found — return specific result so UI can prompt
                    let protocol_str = connection
                        .protocol_config
                        .protocol_type()
                        .as_str()
                        .to_lowercase();
                    return Ok(CredentialResolutionResult::VaultEntryMissing {
                        connection_name: connection.name.clone(),
                        lookup_key: generate_store_key(
                            &connection.name,
                            &connection.host,
                            &protocol_str,
                            backend_type,
                        ),
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "[resolve_credentials_blocking] Vault lookup failed"
                    );
                    // Backend may not be properly configured
                    return Ok(CredentialResolutionResult::BackendNotConfigured {
                        required_backend: secret_settings.preferred_backend,
                    });
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
            let mut visited = std::collections::HashSet::new();
            while let Some(group_id) = current_group_id {
                // Cycle detection
                if !visited.insert(group_id) {
                    tracing::warn!(
                        %group_id,
                        "Cycle detected in KeePass group hierarchy during Inherit resolution"
                    );
                    break;
                }

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
                            return Ok(CredentialResolutionResult::Resolved(creds));
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
        // See also: CredentialResolver::resolve_inherited_credentials() in resolver.rs
        if connection.password_source == PasswordSource::Inherit
            && !matches!(
                secret_settings.preferred_backend,
                rustconn_core::config::SecretBackendType::KeePassXc
                    | rustconn_core::config::SecretBackendType::KdbxFile
            )
        {
            let mut current_group_id = connection.group_id;
            let mut visited = std::collections::HashSet::new();

            while let Some(group_id) = current_group_id {
                if !visited.insert(group_id) {
                    tracing::warn!(
                        %group_id,
                        "Cycle detected in group hierarchy during Inherit resolution"
                    );
                    break;
                }

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

                    match dispatch_vault_op(&secret_settings, &group_key, VaultOp::Retrieve) {
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
                            return Ok(CredentialResolutionResult::Resolved(creds));
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
        let groups = groups.clone();

        // Use thread-local runtime (created lazily per thread)
        // 30-second timeout prevents indefinite hangs if the backend is unresponsive
        let fallback_result = crate::async_utils::with_runtime(|rt| {
            rt.block_on(async {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    resolver.resolve_with_hierarchy(&connection, &groups),
                )
                .await
                {
                    Ok(result) => result.map_err(|e| format!("Failed to resolve credentials: {e}")),
                    Err(_) => Err("Credential resolution timed out after 30s".to_string()),
                }
            })
        })?;

        // Convert Option<Credentials> to CredentialResolutionResult
        Ok(match fallback_result {
            Ok(Some(creds)) => CredentialResolutionResult::Resolved(creds),
            Ok(None) => CredentialResolutionResult::NotNeeded,
            Err(e) => return Err(e),
        })
    }

    // ========== Settings Operations ==========

    /// Gets the current settings
    pub const fn settings(&self) -> &AppSettings {
        &self.settings
    }

    /// Returns the shared folder connection tracker for task conditional execution
    pub fn folder_tracker(&self) -> &Arc<std::sync::Mutex<FolderConnectionTracker>> {
        &self.folder_tracker
    }

    /// Returns `true` (once) if the KeePass keyring load at startup failed.
    ///
    /// After the first call the flag is cleared, so the toast is shown only once.
    pub fn take_kdbx_keyring_failed(&mut self) -> bool {
        std::mem::take(&mut self.kdbx_keyring_failed)
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
            // Invalidate cache so next check re-evaluates availability
            self.secret_backend_available = None;
            self.secret_backend_availability = None;
        }

        // Keep the sync manager's settings copy in step (sync_dir, device
        // identity, retention, simple_sync toggle) so Simple/Group Sync use
        // the current configuration without an app restart.
        if self.settings.sync != settings.sync {
            self.sync_manager.set_settings(settings.sync.clone());
        }

        // Simple Sync: a change to the (non-secret) global variables must be
        // republished. Compared against the still-current `self.settings`.
        if self.settings.global_variables != settings.global_variables {
            self.mark_simple_sync_dirty();
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

    /// Returns true if the document has unsaved changes
    pub fn is_document_dirty(&self, id: Uuid) -> bool {
        self.document_manager.is_dirty(id)
    }

    /// Gets the file path for a document if it has been saved
    pub fn get_document_path(&self, id: Uuid) -> Option<&Path> {
        self.document_manager.get_path(id)
    }

    /// Gets the currently active document ID
    pub const fn active_document_id(&self) -> Option<Uuid> {
        self.active_document_id
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

    // ========== Cluster Operations ==========
}

/// Shared application state type
pub type SharedAppState = Rc<RefCell<AppState>>;

/// Safe read access to `SharedAppState`, preventing borrow panics from
/// leaking across callback boundaries.
pub fn with_state<R>(state: &SharedAppState, f: impl FnOnce(&AppState) -> R) -> R {
    f(&state.borrow())
}

/// Safe read access that returns `None` if the state is already mutably borrowed.
pub fn try_with_state<R>(state: &SharedAppState, f: impl FnOnce(&AppState) -> R) -> Option<R> {
    state.try_borrow().ok().map(|s| f(&s))
}

/// Safe write access to `SharedAppState`.
pub fn with_state_mut<R>(state: &SharedAppState, f: impl FnOnce(&mut AppState) -> R) -> R {
    f(&mut state.borrow_mut())
}

/// Safe write access that returns `None` if the state is already borrowed.
pub fn try_with_state_mut<R>(
    state: &SharedAppState,
    f: impl FnOnce(&mut AppState) -> R,
) -> Option<R> {
    state.try_borrow_mut().ok().map(|mut s| f(&mut s))
}

/// Creates a new shared application state
pub fn create_shared_state() -> Result<SharedAppState, String> {
    AppState::new().map(|state| Rc::new(RefCell::new(state)))
}

// Vault credential operations — extracted to reduce module complexity.
// Re-exported here so all `crate::state::` paths continue to work.
pub use crate::vault_ops::*;
