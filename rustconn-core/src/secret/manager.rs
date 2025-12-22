//! Secret manager with fallback chain support
//!
//! This module provides the `SecretManager` which manages multiple secret backends
//! and automatically falls back to alternative backends when the primary is unavailable.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

use super::backend::SecretBackend;

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
    pub fn set_cache_enabled(&mut self, enabled: bool) {
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
