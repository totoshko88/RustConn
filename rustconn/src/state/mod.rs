//! Application state management
//!
//! This module provides the central application state that holds all managers
//! and provides thread-safe access to core functionality.

mod connections;
mod sessions;
mod sync;

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use chrono::Utc;
use rustconn_core::automation::FolderConnectionTracker;
use rustconn_core::cluster::ClusterManager;
use rustconn_core::config::{AppSettings, ConfigManager};
use rustconn_core::connection::ConnectionManager;
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
use uuid::Uuid;

use crate::async_utils::with_runtime;

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
    /// Cluster manager for connection clusters
    cluster_manager: ClusterManager,
    /// Application settings
    settings: AppSettings,
    /// Session-level password cache (cleared on app exit)
    ///
    /// The negative counterpart — which connections the vault had *nothing* for —
    /// deliberately does not live here; see [`crate::vault_miss_cache`] for why.
    password_cache: HashMap<Uuid, CachedCredentials>,
    /// Transient answers to interactive `@ask:` variables, per connection.
    ///
    /// Populated at connect time from the ASK prompt dialog and consumed by the
    /// variable-substitution builders for that connect. Session-only and never
    /// serialized — a secret answer must not outlive the process, and every
    /// connect re-prompts because the stored variable keeps its `@ask:`
    /// directive. Overwritten on each connect and cleared when the session ends.
    ask_answers: HashMap<Uuid, Vec<rustconn_core::Variable>>,
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

    /// Backends whose "remember this on the machine" request the last settings
    /// save could not honour.
    ///
    /// Set by [`Self::update_settings`] and read once by the Settings save path.
    /// It is state rather than a return value because `update_settings` has eight
    /// callers and only one of them can put a dialog on screen; the rest would
    /// have had to thread a value they cannot act on.
    persistence_failures: Vec<&'static str>,
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
            rustconn_core::config::SecretBackendType::PortableEncryptedFile => {
                // Restore the portable store passphrase so the first connection
                // of the session does not have to prompt. Without this the
                // "Save passphrase" choice in Settings had no effect at all:
                // the blob was written and never read back.
                if settings.secrets.portable_passphrase_encrypted.is_some()
                    && settings.secrets.decrypt_portable_passphrase()
                {
                    tracing::info!("Portable file passphrase restored from encrypted storage");
                }
                if settings.secrets.portable_passphrase.is_none()
                    && settings.secrets.portable_save_to_keyring
                {
                    match with_runtime(|rt| {
                        rt.block_on(async {
                            tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                rustconn_core::secret::get_portable_passphrase_from_keyring(),
                            )
                            .await
                        })
                    }) {
                        Ok(Ok(Ok(Some(passphrase)))) => {
                            settings.secrets.portable_passphrase = Some(passphrase);
                            tracing::info!("Portable file passphrase restored from system keyring");
                        }
                        Ok(Ok(Ok(None))) => {
                            tracing::debug!("No portable file passphrase found in system keyring");
                        }
                        Ok(Ok(Err(e))) => {
                            tracing::warn!(
                                error = %e,
                                "Failed to load the portable file passphrase from system keyring"
                            );
                        }
                        Ok(Err(_elapsed)) => {
                            tracing::warn!(
                                "Keyring query timed out after 5s — the portable file \
                                 passphrase will be asked for on first connect"
                            );
                        }
                        Err(e) => {
                            tracing::warn!(
                                error = %e,
                                "Runtime error loading the portable file passphrase from keyring"
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
            cluster_manager,
            settings,
            password_cache: HashMap::new(),
            ask_answers: HashMap::new(),
            clipboard: ConnectionClipboard::new(),
            history_entries,
            history_dirty: std::cell::Cell::new(false),
            history_dirty_tx: None,
            secret_backend_available: None,
            secret_backend_availability: None,
            persistence_failures: Vec::new(),
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

    // ========== Interactive (ASK) answers ==========

    /// Stores the answers to a connection's interactive `@ask:` variables for
    /// the current connect, replacing any previous set.
    ///
    /// Each answer is an ordinary (non-directive) [`rustconn_core::Variable`]
    /// named after the ASK variable it answers, so it shadows the stored
    /// directive during substitution. Session-only.
    pub fn set_ask_answers(&mut self, connection_id: Uuid, answers: Vec<rustconn_core::Variable>) {
        self.ask_answers.insert(connection_id, answers);
    }

    /// Returns the stored ASK answers for a connection, or an empty slice.
    ///
    /// Answers are session-only and reused across a reconnect of the same
    /// connection; they are overwritten on the next fresh connect and scrubbed
    /// (secret values via `Variable`'s `Drop`) when the process exits.
    #[must_use]
    pub fn ask_answers(&self, connection_id: Uuid) -> &[rustconn_core::Variable] {
        self.ask_answers
            .get(&connection_id)
            .map_or(&[], Vec::as_slice)
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

    /// Reports that the portable store must be unlocked before resolving.
    ///
    /// Returns `None` when the preferred backend is not the portable file, or
    /// when the session passphrase is already in hand. Both credential paths
    /// (variable-sourced and vault-sourced) ask before doing any lookup, because
    /// a locked backend can only answer `PassphraseRequired` and that would
    /// surface as a bare failure rather than a prompt.
    fn portable_unlock_request(
        secret_settings: &rustconn_core::config::SecretSettings,
        connection_id: uuid::Uuid,
    ) -> Option<rustconn_core::sync::CredentialResolutionResult> {
        if !matches!(
            secret_settings.preferred_backend,
            rustconn_core::config::SecretBackendType::PortableEncryptedFile
        ) || secret_settings.portable_passphrase.is_some()
        {
            return None;
        }
        Some(
            rustconn_core::sync::CredentialResolutionResult::PortableFileLocked {
                file_path: rustconn_core::secret::resolve_portable_store_path(
                    secret_settings.portable_file_path.as_deref(),
                ),
                connection_id,
            },
        )
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

        // PasswordSource::None means the connection does not use a vault/variable
        // password (e.g. key-based SSH, agent auth). Skip the expensive vault
        // fallback lookup entirely — it wastes 3-6s per Bitwarden CLI call for
        // connections that will never have a vault entry.
        // ponytail: legacy migration (enable_fallback) preserved only in
        // resolver.resolve_with_hierarchy; if a migrated connection still has
        // PasswordSource::None with a vault entry, user should set source to Vault.
        if connection.password_source == PasswordSource::None {
            return Ok(CredentialResolutionResult::NotNeeded);
        }

        // For Variable password source — resolve directly via vault backend
        if let PasswordSource::Variable(ref var_name) = connection.password_source {
            tracing::debug!(
                var_name,
                "[resolve_credentials_blocking] Resolving variable password"
            );

            // Check if the KDBX database needs to be unlocked before we can
            // resolve the variable. This covers the "Don't save" mode where
            // kdbx_password is None at runtime (#273).
            let needs_kdbx_unlock = kdbx_enabled
                && matches!(
                    secret_settings.preferred_backend,
                    rustconn_core::config::SecretBackendType::KeePassXc
                        | rustconn_core::config::SecretBackendType::KdbxFile
                )
                && secret_settings.kdbx_use_password
                && kdbx_password.is_none()
                && kdbx_path.is_some();

            if needs_kdbx_unlock {
                tracing::debug!(
                    var_name,
                    "[resolve_credentials_blocking] KDBX password unavailable, requesting unlock"
                );
                return Ok(CredentialResolutionResult::KdbxLocked {
                    kdbx_path: kdbx_path.clone().expect("checked is_some above"),
                    connection_id: connection.id,
                });
            }

            // Check if the portable encrypted file needs to be unlocked.
            if let Some(locked) = Self::portable_unlock_request(&secret_settings, connection.id) {
                tracing::debug!(
                    var_name,
                    "[resolve_credentials_blocking] Portable passphrase unavailable, requesting unlock"
                );
                return Ok(locked);
            }

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
                        Credentials::with_password(username, password.as_str())
                    } else {
                        Credentials {
                            username: None,
                            password: Some(secrecy::SecretString::from(
                                password.as_str().to_owned(),
                            )),
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
            // Check if the KDBX database needs to be unlocked (#273).
            // When "Save password = Don't save", kdbx_password is None and
            // keepassxc-cli cannot unlock the database without it.
            if secret_settings.kdbx_use_password && kdbx_password.is_none() {
                tracing::debug!(
                    "[resolve_credentials_blocking] KDBX password unavailable for Vault source, requesting unlock"
                );
                return Ok(CredentialResolutionResult::KdbxLocked {
                    kdbx_path: kdbx_path.clone(),
                    connection_id: connection.id,
                });
            }

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
                // Both arms below return. Until now they logged and fell
                // through, and the fall-through was the bug: the non-KeePass
                // Vault block is skipped (wrong backend), the Inherit block is
                // skipped (wrong source), so execution reached the generic
                // resolver at the end of this function — which holds the
                // `SecretManager` *chain*, i.e. the system keyring plus the
                // encrypted file.
                //
                // So with KeePassXC selected and the database locked, the CLI
                // missing or the path wrong, a password could be read out of
                // libsecret or out of `credentials.enc` and used to log in, with
                // nothing anywhere saying the chosen database had not been
                // consulted. And a genuine miss came back as `NotNeeded`, so the
                // KeePass user did not even get the "Vault entry not found"
                // notice every other backend produces.
                //
                // Now KeePass answers the way the others do, and the two arms
                // differ the way they differ everywhere else in this function: a
                // database that could not be *read* is reported and nothing else
                // is consulted, while a database that opened and does not hold the
                // entry falls back to the encrypted file if the user left that
                // switch on — the same one line the block below ends with.
                //
                // The distinction is the whole fix. What was wrong was a *locked*
                // database serving a password out of libsecret or `credentials.enc`
                // with nothing saying the chosen database was never opened. A
                // genuine miss on an open database saying "then look where the
                // user told me to also look" is not that, and refusing it would
                // make **Also read from the encrypted file** mean one thing for
                // KeePassXC and another for every other backend.
                Ok(None) => {
                    tracing::debug!(
                        lookup_key = %lookup_key,
                        "[resolve_credentials_blocking] No password under this key in KeePass"
                    );
                    // The flat `rustconn/{name}` key, which is what
                    // `generate_store_key` yields for KeePassXC and for every
                    // other non-keyring backend — so a password saved before the
                    // switch to KeePassXC is under the key this asks for.
                    let fallback_key = generate_store_key(
                        &connection.name,
                        &connection.host,
                        &protocol_str.to_lowercase(),
                        secret_settings.preferred_backend,
                    );
                    if let Some(creds) = retrieve_from_encrypted_file_fallback(
                        &secret_settings,
                        std::slice::from_ref(&fallback_key),
                    ) {
                        tracing::debug!(
                            "[resolve_credentials_blocking] Found password in the encrypted-file fallback"
                        );
                        return Ok(CredentialResolutionResult::Resolved(creds));
                    }
                    return Ok(CredentialResolutionResult::VaultEntryMissing {
                        connection_name: connection.name.clone(),
                        lookup_key,
                    });
                }
                Err(e) => {
                    tracing::warn!(
                        error = %e,
                        "[resolve_credentials_blocking] KeePass database could not be read"
                    );
                    return Ok(CredentialResolutionResult::BackendNotConfigured {
                        required_backend: secret_settings.preferred_backend,
                    });
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
            // Check if the portable encrypted file needs to be unlocked first.
            if let Some(locked) = Self::portable_unlock_request(&secret_settings, connection.id) {
                tracing::debug!(
                    "[resolve_credentials_blocking] Portable passphrase unavailable for Vault source, requesting unlock"
                );
                return Ok(locked);
            }

            let backend_type = select_backend_for_load(&secret_settings);
            let protocol_str = connection
                .protocol_config
                .protocol_type()
                .as_str()
                .to_lowercase();

            // Look the credential up where it is written. Saving goes through
            // `generate_store_key_with_group`, which for keyring backends
            // produces `RustConn/{group_path}/{name} ({protocol})`; resolving
            // used `generate_store_key`, which passes no group and produces the
            // flat `{name} ({protocol})`. So a password saved normally was
            // searched for at a key nothing is written to any more: the lookup
            // reported "no password found", and the connection fell through to
            // the credential prompt with the secret sitting in the keyring the
            // whole time. Same defect the doc comment on
            // `generate_store_key_with_group` records for the macOS Keychain in
            // 0.19.19 — the load side of it was left behind.
            //
            // Fixing it in 0.21.0 left half of it in place, and that half is issue
            // #316: this built the group path with `connection.group_id.map(…)`,
            // which is `None` for an ungrouped connection, and `None` means the
            // *flat* key rather than the prefixed one an ungrouped connection is
            // saved under. So a grouped connection resolved and an ungrouped one
            // never did. The ungrouped decision now lives in one function.
            let hierarchical_key = generate_store_key_for_connection(
                &connection.name,
                &connection.host,
                &protocol_str,
                backend_type,
                connection.group_id,
                groups,
            );
            // Tried second so credentials written by a release that stored flat
            // keep resolving, exactly as `resolve_from_keyring_hierarchical`
            // falls back for the keyring backends it handles.
            let legacy_key = generate_store_key(
                &connection.name,
                &connection.host,
                &protocol_str,
                backend_type,
            );

            let mut lookup_keys = vec![hierarchical_key.clone()];
            if legacy_key != hierarchical_key {
                lookup_keys.push(legacy_key);
            }

            for lookup_key in &lookup_keys {
                tracing::debug!(
                    lookup_key = %lookup_key,
                    ?backend_type,
                    "[resolve_credentials_blocking] Vault (non-KeePass): resolving"
                );

                match dispatch_vault_op(&secret_settings, lookup_key, VaultOp::Retrieve) {
                    // An entry holding an empty password is not a hit. Older
                    // releases left such entries behind under the flat key, and
                    // accepting one would end the search before the key that
                    // actually holds the secret is tried.
                    Ok(Some(creds))
                        if creds
                            .password
                            .as_ref()
                            .is_some_and(|password| !password.expose_secret().is_empty()) =>
                    {
                        tracing::debug!(
                            lookup_key = %lookup_key,
                            "[resolve_credentials_blocking] Found password in vault"
                        );
                        return Ok(CredentialResolutionResult::Resolved(creds));
                    }
                    Ok(_) => {
                        tracing::debug!(
                            lookup_key = %lookup_key,
                            "[resolve_credentials_blocking] No password under this key"
                        );
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

            // The selected backend answered "not here" under either key. Before
            // giving up, look where the **Also read from the encrypted file**
            // switch says to look — with the same keys, because that is where the
            // "Save to This Computer" response of the store-failure dialog wrote.
            //
            // This block is what makes that response mean anything. Without it
            // the dialog offered a destination, the toast confirmed the save, and
            // then this function queried the selected backend alone and reported
            // the password missing — the same "saved and missing at the same time"
            // the silent redirect produced, only with the user's consent on it.
            //
            // Deliberately on the miss path only. The `Err` arm above reports
            // `BackendNotConfigured` and does not come here: a store that could
            // not be read has not said the password is absent, and serving one
            // from elsewhere is what was wrong with the old KeePass fall-through.
            if let Some(creds) =
                retrieve_from_encrypted_file_fallback(&secret_settings, &lookup_keys)
            {
                tracing::debug!(
                    "[resolve_credentials_blocking] Found password in the encrypted-file fallback"
                );
                return Ok(CredentialResolutionResult::Resolved(creds));
            }

            tracing::debug!("[resolve_credentials_blocking] No password found in vault");
            // Vault entry not found — return specific result so UI can prompt.
            // The key reported is the hierarchical one, since that is where a
            // credential saved from here on will land.
            return Ok(CredentialResolutionResult::VaultEntryMissing {
                connection_name: connection.name.clone(),
                lookup_key: hierarchical_key,
            });
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
            // Check if the KDBX database needs to be unlocked (#273).
            if secret_settings.kdbx_use_password && kdbx_password.is_none() {
                tracing::debug!(
                    "[resolve_credentials_blocking] KDBX password unavailable for Inherit source, requesting unlock"
                );
                return Ok(CredentialResolutionResult::KdbxLocked {
                    kdbx_path: kdbx_path.clone(),
                    connection_id: connection.id,
                });
            }

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

    /// Records a verified portable-store passphrase for the rest of the session.
    ///
    /// Both destinations are required. The settings field is what the GUI's own
    /// credential paths read and what the "Save passphrase" choice persists from;
    /// the secret manager holds the live backend, and nothing would otherwise
    /// hand it the passphrase — `rebuild_from_settings` only runs when
    /// `SecretSettings` compares unequal, and the runtime passphrase is
    /// deliberately excluded from that comparison. Updating one and not the other
    /// is why unlocking appeared to succeed and the next lookup still failed.
    pub fn unlock_portable_store(&mut self, passphrase: secrecy::SecretString) {
        self.settings.secrets.portable_passphrase = Some(passphrase.clone());
        if !self.secret_manager.set_portable_passphrase(passphrase) {
            tracing::debug!(
                "Portable passphrase stored in settings; no portable backend is configured"
            );
        }
    }

    /// Reports whether the portable credential store is unlocked this session.
    ///
    /// `false` also when no portable backend is configured, which reads as "no
    /// unlock needed" — callers should only consult this when the portable
    /// backend is the preferred one.
    #[must_use]
    pub fn portable_store_unlocked(&self) -> bool {
        self.secret_manager.portable_unlocked()
    }

    /// Takes the local-encryption failures recorded by the last settings save.
    ///
    /// Draining is deliberate: the failure is reported once, on the save that
    /// produced it, and a later unrelated save must not repeat a stale warning.
    pub fn take_persistence_failures(&mut self) -> Vec<&'static str> {
        std::mem::take(&mut self.persistence_failures)
    }

    /// Locks the portable credential store, dropping the session passphrase.
    ///
    /// Clears both copies the unlock set — the settings field the GUI reads and
    /// the live backend's own passphrase and cached data key — so the next
    /// credential lookup prompts again. Used by the explicit lock action and on
    /// shutdown; without a caller the passphrase simply lived for the whole
    /// process, which is the one thing a "remember for this session only" choice
    /// is supposed to bound.
    pub fn lock_portable_store(&mut self) {
        self.settings.secrets.portable_passphrase = None;
        // Synchronously, so this is complete even when called from the window's
        // close handler: once `lock_portable` returns, neither the passphrase nor
        // the derived key is left in memory.
        self.secret_manager.lock_portable();

        // Clearing the credential cache needs the cache's async lock, so it is
        // spawned. On the shutdown path the main loop can stop before this future
        // runs — which is exactly why the part that matters is the synchronous
        // call above. The cache is behind an `Arc`, so the clone clears the same
        // map.
        let manager = self.secret_manager.clone();
        gtk4::glib::spawn_future_local(async move {
            manager.clear_cache().await;
        });
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
        // Turn each backend's storage choice into what actually lands on disk.
        // "System keyring" makes the keyring the persistence layer, so an
        // encrypted blob would duplicate the secret on disk against the user's
        // explicit choice — the runtime password is kept in memory only and
        // written to the keyring by the Settings dialog (issue #272). Until
        // 0.19.19 only KDBX honoured that; the rule now lives in one pure,
        // test-covered method so it stays symmetric across all five backends.
        // A backend whose local encryption failed is recorded rather than
        // ignored: the user asked for the secret to be remembered, and for the
        // portable store that secret is the key to every credential in the file.
        self.persistence_failures = settings.secrets.apply_storage_persistence();

        self.config_manager
            .save_settings(&settings)
            .map_err(|e| format!("Failed to save settings: {e}"))?;

        // Update session manager logging
        if settings.logging.enabled != self.settings.logging.enabled {
            self.session_manager
                .set_logging_enabled(settings.logging.enabled);
        }

        // Preserve runtime-only secret fields (#[serde(skip)]) that the
        // Settings dialog does not collect (password entries are intentionally
        // left blank for security). Without this, closing the dialog would
        // wipe the in-memory passwords loaded at startup, breaking vault
        // access until restart (issue #259).
        if settings.secrets.kdbx_password.is_none() {
            settings.secrets.kdbx_password = self.settings.secrets.kdbx_password.clone();
        }
        if settings.secrets.bitwarden_password.is_none() {
            settings.secrets.bitwarden_password = self.settings.secrets.bitwarden_password.clone();
        }
        if settings.secrets.bitwarden_client_id.is_none() {
            settings.secrets.bitwarden_client_id =
                self.settings.secrets.bitwarden_client_id.clone();
        }
        if settings.secrets.bitwarden_client_secret.is_none() {
            settings.secrets.bitwarden_client_secret =
                self.settings.secrets.bitwarden_client_secret.clone();
        }
        if settings.secrets.onepassword_service_account_token.is_none() {
            settings.secrets.onepassword_service_account_token = self
                .settings
                .secrets
                .onepassword_service_account_token
                .clone();
        }
        if settings.secrets.passbolt_passphrase.is_none() {
            settings.secrets.passbolt_passphrase =
                self.settings.secrets.passbolt_passphrase.clone();
        }
        // The portable passphrase is the one runtime secret that can also arrive
        // from outside the Settings dialog: `unlock_portable_store` records it
        // when a connection prompts for it mid-session. Under the default
        // "Don't save" choice the dialog collects `None` for it, so without this
        // line pressing Save re-locked the session — the GUI prompted again on
        // the next connection while the manager's backend was still unlocked and
        // would have answered.
        if settings.secrets.portable_passphrase.is_none() {
            settings.secrets.portable_passphrase =
                self.settings.secrets.portable_passphrase.clone();
        }

        // Runtime-only secret restoration happens *before* the rebuild below,
        // not after. `rebuild_from_settings` hands the portable passphrase to the
        // new backend, so restoring it afterwards left the rebuilt backend locked
        // while `settings.secrets.portable_passphrase` said otherwise: the next
        // connection prompted for a passphrase the session already had.
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
