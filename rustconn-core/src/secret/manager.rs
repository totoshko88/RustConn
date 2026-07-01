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

use super::backend::{BackendAvailability, SecretBackend};

/// Default TTL for cached credentials in seconds (5 minutes).
pub const CACHE_TTL_SECONDS: i64 = 300;

/// Reports which backend in the fallback chain accepted a store operation.
///
/// Returned by [`SecretManager::store_reported`] so callers can surface the
/// difference between a write to the user's preferred backend and a graceful
/// fallback (e.g. to the encrypted-file store when the system keyring is
/// unavailable, issue #201).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreOutcome {
    /// The credential was stored in the preferred (primary) backend.
    Primary,
    /// The credential was stored in a fallback backend after the primary failed.
    Fallback {
        /// Identifier of the backend that accepted the write (e.g. `encrypted_file`).
        backend_id: String,
    },
}

/// A cache entry with a timestamp for TTL-based expiry.
#[derive(Debug, Clone)]
struct CacheEntry {
    credentials: Credentials,
    cached_at: chrono::DateTime<chrono::Utc>,
}

impl CacheEntry {
    fn new(credentials: Credentials) -> Self {
        Self {
            credentials,
            cached_at: chrono::Utc::now(),
        }
    }

    fn is_expired(&self) -> bool {
        let age = chrono::Utc::now()
            .signed_duration_since(self.cached_at)
            .num_seconds();
        age > CACHE_TTL_SECONDS
    }
}

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
///
/// # Security
///
/// ## Credential lifecycle
///
/// 1. **Retrieval** — `resolve_credentials()` queries backends in priority
///    order. The first successful result is returned and optionally cached.
/// 2. **Caching** — Resolved credentials are held in an in-memory
///    `HashMap<String, Credentials>` behind an `Arc<RwLock<…>>`. The cache
///    lives for the duration of the `SecretManager` instance (typically the
///    application session). Passwords are stored as `SecretString` and are
///    never logged or serialized.
/// 3. **Eviction** — Call `clear_cache()` to drop all cached entries
///    immediately. The cache is also dropped when the last `SecretManager`
///    clone is dropped (normal `Arc` semantics).
/// 4. **Storage** — `store_credentials()` writes to the highest-priority
///    backend that accepts the operation. Passwords are passed as
///    `SecretString` and exposed only at the backend boundary.
/// 5. **Deletion** — `delete_credentials()` removes the entry from all
///    backends and evicts the cache entry.
pub struct SecretManager {
    /// Backends in priority order (first = highest priority)
    backends: Vec<Arc<dyn SecretBackend>>,
    /// Session cache for retrieved credentials (with TTL-based expiry)
    cache: Arc<RwLock<HashMap<String, CacheEntry>>>,
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

    /// Builds a `SecretManager` with backends configured from settings
    ///
    /// Creates the preferred backend based on `SecretSettings.preferred_backend`
    /// and optionally adds libsecret as a fallback. This ensures the manager
    /// can resolve credentials (including variable-based passwords) without
    /// requiring callers to manually construct backends.
    #[must_use]
    pub fn build_from_settings(settings: &crate::config::SecretSettings) -> Self {
        use crate::config::SecretBackendType;

        let mut backends: Vec<Arc<dyn SecretBackend>> = Vec::new();

        match settings.preferred_backend {
            SecretBackendType::Bitwarden => {
                backends.push(Arc::new(super::BitwardenBackend::new()));
            }
            SecretBackendType::OnePassword => {
                let mut backend = super::OnePasswordBackend::new();
                if let Some(ref token) = settings.onepassword_service_account_token {
                    backend.set_service_account_token(token.clone());
                }
                backends.push(Arc::new(backend));
            }
            SecretBackendType::Passbolt => {
                let mut backend = super::PassboltBackend::new();
                if let Some(ref url) = settings.passbolt_server_url {
                    backend = backend.with_server_address(url.clone());
                }
                if let Some(ref passphrase) = settings.passbolt_passphrase {
                    backend = backend.with_user_password(passphrase.clone());
                }
                backends.push(Arc::new(backend));
            }
            SecretBackendType::LibSecret => {
                // macOS never constructs LibSecretBackend (oo7 is not compiled
                // there) — it routes to the system Keychain instead (R10.1,
                // R10.2). Non-macOS keeps the oo7-backed libsecret client.
                #[cfg(target_os = "macos")]
                backends.push(Arc::new(super::MacOsKeychainBackend::new()));
                #[cfg(not(target_os = "macos"))]
                backends.push(Arc::new(super::LibSecretBackend::default_app()));
            }
            SecretBackendType::Pass => {
                backends.push(Arc::new(super::PassBackend::from_secret_settings(settings)));
            }
            SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => {
                // KeePass is handled via direct KDBX access in
                // resolve_credentials_blocking, not through SecretManager.
                // Add the system keyring as the operational backend for
                // non-KeePass lookups (e.g. variable resolution). macOS uses
                // the Keychain (LibSecretBackend is not compiled there).
                #[cfg(target_os = "macos")]
                backends.push(Arc::new(super::MacOsKeychainBackend::new()));
                #[cfg(not(target_os = "macos"))]
                backends.push(Arc::new(super::LibSecretBackend::default_app()));
            }
            #[cfg(target_os = "macos")]
            SecretBackendType::MacOsKeychain => {
                backends.push(Arc::new(super::MacOsKeychainBackend::new()));
            }
            #[cfg(not(target_os = "macos"))]
            SecretBackendType::MacOsKeychain => {
                // Fallback to libsecret on non-macOS platforms
                backends.push(Arc::new(super::LibSecretBackend::default_app()));
            }
            SecretBackendType::EncryptedFile => {
                // Application-managed encrypted file: addressed by the flat
                // `generate_store_key` value, same key scheme as the other
                // app-managed (flat-key) backends.
                backends.push(Arc::new(super::EncryptedFileBackend::new()));
            }
        }

        // Register the application-managed encrypted file as the terminal
        // fallback when fallback is enabled and it is not already the preferred
        // backend (avoids a duplicate entry in the chain).
        //
        // This replaces the previous "append LibSecret as fallback" logic: on a
        // box without a responding Secret Service (issue #201) LibSecret is the
        // failing primary, so appending it as its own fallback is useless. The
        // encrypted-file backend works in every environment (it only needs the
        // machine key), making it the single sound terminal fallback. Because
        // `retrieve` walks the backend chain in priority order, a credential
        // stored here is found on the next resolution.
        if settings.enable_fallback
            && !matches!(settings.preferred_backend, SecretBackendType::EncryptedFile)
        {
            backends.push(Arc::new(super::EncryptedFileBackend::new()));
        }

        tracing::debug!(
            backend_count = backends.len(),
            preferred = ?settings.preferred_backend,
            "SecretManager built from settings"
        );

        Self::new(backends)
    }

    /// Replaces all backends with a fresh set built from settings
    ///
    /// Call this after settings change (e.g. user switches secret backend
    /// in Preferences) to ensure the manager uses the correct backends.
    pub fn rebuild_from_settings(&mut self, settings: &crate::config::SecretSettings) {
        let old_backend_count = self.backends.len();
        let fresh = Self::build_from_settings(settings);
        self.backends = fresh.backends;
        // Clear cache on rebuild — backend change may invalidate cached entries
        if let Ok(mut cache) = self.cache.try_write() {
            cache.clear();
        }
        tracing::info!(
            old_backends = old_backend_count,
            new_backends = self.backends.len(),
            preferred = ?settings.preferred_backend,
            "SecretManager backends rebuilt from settings"
        );
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

    /// Stores credentials in the cache when caching is enabled.
    async fn cache_stored(&self, connection_id: &str, credentials: &Credentials) {
        if self.cache_enabled {
            let mut cache = self.cache.write().await;
            cache.insert(
                connection_id.to_string(),
                CacheEntry::new(credentials.clone()),
            );
        }
    }

    /// Store credentials for a connection
    ///
    /// Delegates to [`Self::store_reported`] with fallback authorised and
    /// discards which backend accepted the write, preserving the original
    /// `Result<()>` contract. Also updates the cache when caching is enabled.
    ///
    /// # Arguments
    /// * `connection_id` - Unique identifier for the connection
    /// * `credentials` - The credentials to store
    ///
    /// # Errors
    /// Returns `SecretError` if no backend is available or storage fails on
    /// every backend in the chain.
    pub async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
        self.store_reported(connection_id, credentials, true)
            .await
            .map(|_| ())
    }

    /// Stores credentials and reports whether the primary or a fallback backend stored them.
    ///
    /// Attempts the preferred (primary) backend — the highest-priority backend
    /// in the chain — first. When its store fails, either because the backend is
    /// unavailable or because the write itself errors, and `allow_fallback` is
    /// `true`, the remaining backends are tried in priority order; the first that
    /// accepts the write is reported as [`StoreOutcome::Fallback`]. On success
    /// (primary or fallback) the cache is updated exactly like [`Self::store`].
    ///
    /// # Arguments
    /// * `connection_id` - Unique identifier for the connection
    /// * `credentials` - The credentials to store
    /// * `allow_fallback` - Whether to try subsequent backends when the primary fails
    ///
    /// # Errors
    /// Returns `SecretError::BackendUnavailable` when no backends are registered.
    /// When `allow_fallback` is `false` and the primary backend fails, the
    /// primary's original error is returned unchanged — it is neither wrapped nor
    /// replaced (Requirement 14.2). When `allow_fallback` is `true` and every
    /// backend in the chain fails, the primary backend's error is returned so the
    /// most relevant cause is surfaced and no write is silently lost.
    pub async fn store_reported(
        &self,
        connection_id: &str,
        credentials: &Credentials,
        allow_fallback: bool,
    ) -> SecretResult<StoreOutcome> {
        let Some(primary) = self.backends.first() else {
            return Err(SecretError::BackendUnavailable(
                "No secret backend available".to_string(),
            ));
        };

        // Try the preferred backend first. A store error (including an
        // unavailable backend) is the trigger for falling back.
        let primary_error = match primary.store(connection_id, credentials).await {
            Ok(()) => {
                self.cache_stored(connection_id, credentials).await;
                return Ok(StoreOutcome::Primary);
            }
            Err(e) => e,
        };

        // Requirement 14.2: without fallback authorisation, surface the
        // primary error unchanged.
        if !allow_fallback {
            return Err(primary_error);
        }

        tracing::warn!(
            backend = primary.backend_id(),
            error = %primary_error,
            "primary secret backend store failed; attempting fallback chain"
        );

        // Walk the remaining backends; the first success wins.
        for backend in self.backends.iter().skip(1) {
            match backend.store(connection_id, credentials).await {
                Ok(()) => {
                    self.cache_stored(connection_id, credentials).await;
                    let backend_id = backend.backend_id().to_string();
                    tracing::info!(backend = %backend_id, "credential stored via fallback backend");
                    return Ok(StoreOutcome::Fallback { backend_id });
                }
                Err(e) => {
                    tracing::warn!(
                        backend = backend.backend_id(),
                        error = %e,
                        "fallback secret backend store failed"
                    );
                }
            }
        }

        // Every backend failed: surface the primary error as the most relevant
        // cause so no write is silently lost.
        Err(primary_error)
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
        // Check cache first (with TTL)
        if self.cache_enabled {
            let cache = self.cache.read().await;
            if let Some(entry) = cache.get(connection_id)
                && !entry.is_expired()
            {
                return Ok(Some(entry.credentials.clone()));
            }
            // Expired entries fall through to backend lookup
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
                    cache.insert(connection_id.to_string(), CacheEntry::new(creds.clone()));
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

    /// Reports the fine-grained availability of the primary (preferred) backend.
    ///
    /// Unlike [`Self::is_available`], which reports whether *any* backend can
    /// store secrets, this inspects only the highest-priority backend so the
    /// UI can surface why the user's chosen backend cannot work — for example
    /// distinguishing a missing client from an unresponsive Secret Service
    /// (issue #201).
    ///
    /// Returns [`BackendAvailability::ClientMissing`] when no backends are
    /// registered.
    pub async fn primary_availability(&self) -> BackendAvailability {
        match self.backends.first() {
            Some(backend) => backend.availability().await,
            None => BackendAvailability::ClientMissing,
        }
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

impl std::fmt::Debug for SecretManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Cache size is read with try_read to avoid blocking in Debug.
        // If the lock is contended we report `?` rather than waiting.
        let cache_size = self
            .cache
            .try_read()
            .map_or_else(|_| None, |c| Some(c.len()));

        let backend_ids: Vec<&'static str> = self.backends.iter().map(|b| b.backend_id()).collect();

        f.debug_struct("SecretManager")
            .field("backend_count", &self.backends.len())
            .field("backend_ids", &backend_ids)
            .field("cache_enabled", &self.cache_enabled)
            .field("cache_size", &cache_size)
            .field("cache_ttl_secs", &CACHE_TTL_SECONDS)
            .finish()
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_secret() {
        // SecretManager keeps cached credentials in-process. Even though
        // cache values aren't rendered (only the count), this sentinel
        // guards against future Debug expansions that could expose
        // Credentials directly.
        let manager = SecretManager::empty();
        let rendered = format!("{manager:?}");
        assert!(rendered.contains("SecretManager"));
        assert!(rendered.contains("backend_count"));
        assert!(rendered.contains("cache_enabled"));
        // Make sure no `Credentials { ... }` ends up in Debug accidentally.
        assert!(
            !rendered.contains("password"),
            "Debug should not render password field: {rendered}"
        );
    }
}
