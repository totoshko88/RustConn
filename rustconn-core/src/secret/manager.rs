//! Secret manager with fallback chain support
//!
//! This module provides the `SecretManager` which manages multiple secret backends
//! and automatically falls back to alternative backends when the primary is unavailable.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use secrecy::SecretString;
use uuid::Uuid;

use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

use super::backend::SecretBackend;

/// Result of a bulk credential operation
#[derive(Debug, Clone)]
pub struct BulkOperationResult {
    /// Number of successful operations
    pub success_count: usize,
    /// Number of failed operations
    pub failure_count: usize,
    /// IDs of connections that failed
    pub failed_ids: Vec<Uuid>,
    /// Error messages for failed operations
    pub errors: Vec<String>,
}

impl BulkOperationResult {
    /// Creates a new empty result
    #[must_use]
    pub const fn new() -> Self {
        Self {
            success_count: 0,
            failure_count: 0,
            failed_ids: Vec::new(),
            errors: Vec::new(),
        }
    }

    /// Returns true if all operations succeeded
    #[must_use]
    pub const fn is_success(&self) -> bool {
        self.failure_count == 0
    }

    /// Returns true if any operations failed
    #[must_use]
    pub const fn has_failures(&self) -> bool {
        self.failure_count > 0
    }

    /// Returns the total number of operations attempted
    #[must_use]
    pub const fn total(&self) -> usize {
        self.success_count + self.failure_count
    }

    /// Records a successful operation
    fn record_success(&mut self) {
        self.success_count += 1;
    }

    /// Records a failed operation
    fn record_failure(&mut self, id: Uuid, error: String) {
        self.failure_count += 1;
        self.failed_ids.push(id);
        self.errors.push(error);
    }
}

impl Default for BulkOperationResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Specification for updating credentials in bulk
#[derive(Debug, Clone)]
pub struct CredentialUpdate {
    /// New username (None = keep existing)
    pub username: Option<String>,
    /// New password (None = keep existing)
    pub password: Option<SecretString>,
    /// New domain (None = keep existing)
    pub domain: Option<String>,
    /// Whether to clear the password
    pub clear_password: bool,
}

/// Composite secret manager with fallback support
///
/// The `SecretManager` maintains a list of secret backends in priority order.
/// When storing or retrieving credentials, it tries each backend in order
/// until one succeeds. It also provides session-level caching to avoid
/// repeated queries to the backend.
pub struct SecretManager {
    /// Backends in priority order (first = highest priority)
    backends: Vec<Arc<dyn SecretBackend>>,
    /// Session cache for retrieved credentials
    cache: Arc<RwLock<HashMap<String, Credentials>>>,
    /// Whether caching is enabled
    cache_enabled: bool,
}

impl Clone for SecretManager {
    fn clone(&self) -> Self {
        Self {
            backends: self.backends.clone(),
            cache: Arc::clone(&self.cache),
            cache_enabled: self.cache_enabled,
        }
    }
}

impl SecretManager {
    /// Creates a new `SecretManager` with the given backends
    ///
    /// # Arguments
    /// * `backends` - List of backends in priority order
    ///
    /// # Returns
    /// A new `SecretManager` instance
    #[must_use]
    pub fn new(backends: Vec<Arc<dyn SecretBackend>>) -> Self {
        Self {
            backends,
            cache: Arc::new(RwLock::new(HashMap::new())),
            cache_enabled: true,
        }
    }

    /// Creates an empty `SecretManager` with no backends
    #[must_use]
    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    /// Enables or disables credential caching
    pub const fn set_cache_enabled(&mut self, enabled: bool) {
        self.cache_enabled = enabled;
    }

    /// Adds a backend to the manager
    ///
    /// The backend is added at the end of the priority list.
    pub fn add_backend(&mut self, backend: Arc<dyn SecretBackend>) {
        self.backends.push(backend);
    }

    /// Returns the list of available backends
    ///
    /// # Returns
    /// A vector of backend IDs that are currently available
    pub async fn available_backends(&self) -> Vec<&'static str> {
        let mut available = Vec::new();
        for backend in &self.backends {
            if backend.is_available().await {
                available.push(backend.backend_id());
            }
        }
        available
    }

    /// Returns the first available backend
    async fn get_available_backend(&self) -> SecretResult<&Arc<dyn SecretBackend>> {
        for backend in &self.backends {
            if backend.is_available().await {
                return Ok(backend);
            }
        }
        Err(SecretError::BackendUnavailable(
            "No secret backend available".to_string(),
        ))
    }

    /// Store credentials for a connection
    ///
    /// Stores credentials using the first available backend.
    /// Also updates the cache if caching is enabled.
    ///
    /// # Arguments
    /// * `connection_id` - Unique identifier for the connection
    /// * `credentials` - The credentials to store
    ///
    /// # Errors
    /// Returns `SecretError` if no backend is available or storage fails
    pub async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
        let backend = self.get_available_backend().await?;
        backend.store(connection_id, credentials).await?;

        // Update cache
        if self.cache_enabled {
            let mut cache = self.cache.write().await;
            cache.insert(connection_id.to_string(), credentials.clone());
        }

        Ok(())
    }

    /// Retrieve credentials for a connection
    ///
    /// First checks the cache (if enabled), then queries backends in order.
    /// Caches the result for the session duration.
    ///
    /// # Arguments
    /// * `connection_id` - Unique identifier for the connection
    ///
    /// # Returns
    /// `Some(Credentials)` if found, `None` if not found
    ///
    /// # Errors
    /// Returns `SecretError` if no backend is available or retrieval fails
    pub async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>> {
        // Check cache first
        if self.cache_enabled {
            let cache = self.cache.read().await;
            if let Some(creds) = cache.get(connection_id) {
                return Ok(Some(creds.clone()));
            }
        }

        // Try each backend in order
        for backend in &self.backends {
            if !backend.is_available().await {
                continue;
            }

            if let Ok(Some(creds)) = backend.retrieve(connection_id).await {
                // Cache the result
                if self.cache_enabled {
                    let mut cache = self.cache.write().await;
                    cache.insert(connection_id.to_string(), creds.clone());
                }
                return Ok(Some(creds));
            }
        }

        Ok(None)
    }

    /// Delete credentials for a connection
    ///
    /// Deletes credentials from all backends that have them.
    /// Also removes from cache.
    ///
    /// # Arguments
    /// * `connection_id` - Unique identifier for the connection
    ///
    /// # Errors
    /// Returns `SecretError` if deletion fails on all backends
    pub async fn delete(&self, connection_id: &str) -> SecretResult<()> {
        // Remove from cache
        if self.cache_enabled {
            let mut cache = self.cache.write().await;
            cache.remove(connection_id);
        }

        // Try to delete from all available backends
        let mut deleted = false;
        let mut last_error = None;

        for backend in &self.backends {
            if !backend.is_available().await {
                continue;
            }

            match backend.delete(connection_id).await {
                Ok(()) => deleted = true,
                Err(e) => last_error = Some(e),
            }
        }

        if deleted {
            Ok(())
        } else if let Some(err) = last_error {
            Err(err)
        } else {
            // No backends available
            Err(SecretError::BackendUnavailable(
                "No secret backend available".to_string(),
            ))
        }
    }

    /// Clear the credential cache
    ///
    /// This should be called when the session ends or when
    /// credentials may have changed externally.
    pub async fn clear_cache(&self) {
        let mut cache = self.cache.write().await;
        cache.clear();
    }

    /// Check if any backend is available
    pub async fn is_available(&self) -> bool {
        for backend in &self.backends {
            if backend.is_available().await {
                return true;
            }
        }
        false
    }
}

impl Default for SecretManager {
    fn default() -> Self {
        Self::empty()
    }
}

impl CredentialUpdate {
    /// Creates a new credential update with no changes
    #[must_use]
    pub const fn new() -> Self {
        Self {
            username: None,
            password: None,
            domain: None,
            clear_password: false,
        }
    }

    /// Sets the new username
    #[must_use]
    pub fn with_username(mut self, username: impl Into<String>) -> Self {
        self.username = Some(username.into());
        self
    }

    /// Sets the new password
    #[must_use]
    pub fn with_password(mut self, password: impl Into<String>) -> Self {
        self.password = Some(SecretString::from(password.into()));
        self
    }

    /// Sets the new domain
    #[must_use]
    pub fn with_domain(mut self, domain: impl Into<String>) -> Self {
        self.domain = Some(domain.into());
        self
    }

    /// Marks the password to be cleared
    #[must_use]
    pub const fn with_clear_password(mut self) -> Self {
        self.clear_password = true;
        self
    }

    /// Applies this update to existing credentials
    #[must_use]
    pub fn apply(&self, existing: &Credentials) -> Credentials {
        Credentials {
            username: self.username.clone().or_else(|| existing.username.clone()),
            password: if self.clear_password {
                None
            } else {
                self.password.clone().or_else(|| existing.password.clone())
            },
            key_passphrase: existing.key_passphrase.clone(),
            domain: self.domain.clone().or_else(|| existing.domain.clone()),
        }
    }
}

impl Default for CredentialUpdate {
    fn default() -> Self {
        Self::new()
    }
}

// Bulk operations implementation
impl SecretManager {
    /// Store credentials for multiple connections
    ///
    /// # Arguments
    /// * `credentials_map` - Map of connection IDs to credentials
    ///
    /// # Returns
    /// Result with success/failure counts
    pub async fn store_bulk(
        &self,
        credentials_map: &HashMap<Uuid, Credentials>,
    ) -> BulkOperationResult {
        let mut result = BulkOperationResult::new();

        for (id, creds) in credentials_map {
            match self.store(&id.to_string(), creds).await {
                Ok(()) => result.record_success(),
                Err(e) => result.record_failure(*id, e.to_string()),
            }
        }

        result
    }

    /// Delete credentials for multiple connections
    ///
    /// # Arguments
    /// * `connection_ids` - List of connection IDs to delete credentials for
    ///
    /// # Returns
    /// Result with success/failure counts
    pub async fn delete_bulk(&self, connection_ids: &[Uuid]) -> BulkOperationResult {
        let mut result = BulkOperationResult::new();

        for id in connection_ids {
            match self.delete(&id.to_string()).await {
                Ok(()) => result.record_success(),
                Err(e) => result.record_failure(*id, e.to_string()),
            }
        }

        result
    }

    /// Update credentials for multiple connections with the same update
    ///
    /// This is useful for updating username/password across a group of connections.
    ///
    /// # Arguments
    /// * `connection_ids` - List of connection IDs to update
    /// * `update` - The credential update to apply
    ///
    /// # Returns
    /// Result with success/failure counts
    pub async fn update_bulk(
        &self,
        connection_ids: &[Uuid],
        update: &CredentialUpdate,
    ) -> BulkOperationResult {
        let mut result = BulkOperationResult::new();

        for id in connection_ids {
            let id_str = id.to_string();

            // Retrieve existing credentials
            let existing = match self.retrieve(&id_str).await {
                Ok(Some(creds)) => creds,
                Ok(None) => Credentials::empty(),
                Err(e) => {
                    result.record_failure(*id, format!("Failed to retrieve: {e}"));
                    continue;
                }
            };

            // Apply update
            let updated = update.apply(&existing);

            // Store updated credentials
            match self.store(&id_str, &updated).await {
                Ok(()) => result.record_success(),
                Err(e) => result.record_failure(*id, format!("Failed to store: {e}")),
            }
        }

        result
    }

    /// Update credentials for all connections in a group
    ///
    /// # Arguments
    /// * `group_connection_ids` - List of connection IDs in the group
    /// * `update` - The credential update to apply
    ///
    /// # Returns
    /// Result with success/failure counts
    pub async fn update_credentials_for_group(
        &self,
        group_connection_ids: &[Uuid],
        update: &CredentialUpdate,
    ) -> BulkOperationResult {
        self.update_bulk(group_connection_ids, update).await
    }

    /// Retrieve credentials for multiple connections
    ///
    /// # Arguments
    /// * `connection_ids` - List of connection IDs to retrieve
    ///
    /// # Returns
    /// Map of connection IDs to credentials (only includes found credentials)
    pub async fn retrieve_bulk(&self, connection_ids: &[Uuid]) -> HashMap<Uuid, Credentials> {
        let mut result = HashMap::new();

        for id in connection_ids {
            if let Ok(Some(creds)) = self.retrieve(&id.to_string()).await {
                result.insert(*id, creds);
            }
        }

        result
    }

    /// Copy credentials from one connection to others
    ///
    /// # Arguments
    /// * `source_id` - Connection ID to copy credentials from
    /// * `target_ids` - Connection IDs to copy credentials to
    ///
    /// # Returns
    /// Result with success/failure counts
    ///
    /// # Errors
    /// Returns error if source credentials cannot be retrieved
    pub async fn copy_credentials(
        &self,
        source_id: Uuid,
        target_ids: &[Uuid],
    ) -> SecretResult<BulkOperationResult> {
        // Retrieve source credentials
        let source_creds = self
            .retrieve(&source_id.to_string())
            .await?
            .ok_or_else(|| {
                SecretError::RetrieveFailed(format!("Source credentials not found: {source_id}"))
            })?;

        let mut result = BulkOperationResult::new();

        for target_id in target_ids {
            match self.store(&target_id.to_string(), &source_creds).await {
                Ok(()) => result.record_success(),
                Err(e) => result.record_failure(*target_id, e.to_string()),
            }
        }

        Ok(result)
    }

    /// Check which connections have stored credentials
    ///
    /// # Arguments
    /// * `connection_ids` - List of connection IDs to check
    ///
    /// # Returns
    /// List of connection IDs that have stored credentials
    pub async fn connections_with_credentials(&self, connection_ids: &[Uuid]) -> Vec<Uuid> {
        let mut result = Vec::new();

        for id in connection_ids {
            if let Ok(Some(_)) = self.retrieve(&id.to_string()).await {
                result.push(*id);
            }
        }

        result
    }
}
