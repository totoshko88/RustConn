//! Secret manager with fallback chain support
//!
//! This module provides the `SecretManager` which manages multiple secret backends
//! and automatically falls back to alternative backends when the primary is unavailable.

use std::collections::HashMap;
use std::sync::Arc;

use secrecy::SecretString;
use tokio::sync::RwLock;
use uuid::Uuid;

use super::backend::{BackendAvailability, SecretBackend};
use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

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
    /// The portable backend in `backends`, if one was configured.
    ///
    /// Held typed as well as erased because that backend is the only one that
    /// can be *unlocked after construction*: its passphrase arrives from a
    /// dialog, minutes after the manager was built. Reaching it through
    /// `backends` would mean downcasting a `dyn SecretBackend`, and rebuilding
    /// the manager instead does not work — `rebuild_from_settings` only fires
    /// when `SecretSettings` compares unequal, and the runtime passphrase is
    /// deliberately excluded from that comparison.
    portable: Option<Arc<super::PortableEncryptedFileBackend>>,
}

impl Clone for SecretManager {
    fn clone(&self) -> Self {
        Self {
            backends: self.backends.clone(),
            cache: Arc::clone(&self.cache),
            cache_enabled: self.cache_enabled,
            portable: self.portable.clone(),
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
            portable: None,
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
        let mut portable: Option<Arc<super::PortableEncryptedFileBackend>> = None;

        match settings.preferred_backend {
            SecretBackendType::Bitwarden => {
                backends.push(Arc::new(
                    super::BitwardenBackend::new().with_settings_toggles(settings),
                ));
            }
            SecretBackendType::OnePassword => {
                backends.push(Arc::new(super::OnePasswordBackend::from_secret_settings(
                    settings,
                )));
            }
            SecretBackendType::Passbolt => {
                backends.push(Arc::new(super::PassboltBackend::from_secret_settings(
                    settings,
                )));
            }
            SecretBackendType::LibSecret => {
                // macOS never constructs LibSecretBackend (oo7 is not compiled
                // there) — it routes to the system Keychain instead (R10.1,
                // R10.2). Non-macOS keeps the oo7-backed libsecret client.
                #[cfg(all(feature = "system-keyring", target_os = "macos"))]
                backends.push(Arc::new(super::MacOsKeychainBackend::new()));
                #[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
                backends.push(Arc::new(super::LibSecretBackend::default_app()));
                #[cfg(not(feature = "system-keyring"))]
                tracing::warn!(
                    "System keyring backend selected but rustconn-core/system-keyring is disabled; \
                     credentials requiring keyring will not resolve"
                );
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
                #[cfg(all(feature = "system-keyring", target_os = "macos"))]
                backends.push(Arc::new(super::MacOsKeychainBackend::new()));
                #[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
                backends.push(Arc::new(super::LibSecretBackend::default_app()));
                #[cfg(not(feature = "system-keyring"))]
                tracing::warn!(
                    "KDBX backend selected without system-keyring; only direct KDBX \
                     and app-managed fallback backends are available"
                );
            }
            #[cfg(all(feature = "system-keyring", target_os = "macos"))]
            SecretBackendType::MacOsKeychain => {
                backends.push(Arc::new(super::MacOsKeychainBackend::new()));
            }
            #[cfg(all(feature = "system-keyring", not(target_os = "macos")))]
            SecretBackendType::MacOsKeychain => {
                // Fallback to libsecret on non-macOS platforms
                backends.push(Arc::new(super::LibSecretBackend::default_app()));
            }
            #[cfg(not(feature = "system-keyring"))]
            SecretBackendType::MacOsKeychain => {
                tracing::warn!(
                    "macOS Keychain backend selected but rustconn-core/system-keyring is disabled; \
                     credentials requiring keyring will not resolve"
                );
            }
            SecretBackendType::EncryptedFile => {
                // Application-managed encrypted file: addressed by the flat
                // `generate_store_key` value, same key scheme as the other
                // app-managed (flat-key) backends.
                backends.push(Arc::new(super::EncryptedFileBackend::new()));
            }
            SecretBackendType::PortableEncryptedFile => {
                // Passphrase-based portable encrypted file: same flat-key scheme
                // as EncryptedFile, but keyed from a user passphrase.
                let path =
                    super::resolve_portable_store_path(settings.portable_file_path.as_deref());
                let backend = Arc::new(super::PortableEncryptedFileBackend::with_path(path));
                // Seed from settings when the passphrase is already known —
                // restored from the keyring or the local encrypted copy at
                // startup, or carried across a settings save. When it is not,
                // the backend stays locked and reports `PassphraseRequired`
                // until `set_portable_passphrase` supplies it.
                if let Some(ref passphrase) = settings.portable_passphrase {
                    backend.set_passphrase(passphrase.clone());
                }
                portable = Some(Arc::clone(&backend));
                backends.push(backend);
            }
        }

        // Register the application-managed encrypted file as the terminal
        // fallback when fallback is enabled and it is not already the preferred
        // backend (which would be a duplicate entry in the chain).
        //
        // This replaces the previous "append LibSecret as fallback" logic: on a
        // box without a responding Secret Service (issue #201) LibSecret is the
        // failing primary, so appending it as its own fallback is useless. The
        // encrypted-file backend works in every environment (it only needs the
        // machine key), making it the single sound terminal fallback. Because
        // `retrieve` walks the backend chain in priority order, a credential
        // stored here is found on the next resolution.
        //
        // `PortableEncryptedFile` is deliberately *not* excluded here, even
        // though both are file backends. The two hold different files under
        // different keys: excluding the machine-bound store when portable is
        // preferred would make every credential already in `credentials.enc`
        // unreachable the moment the user switched — before they had a chance to
        // run "Copy Credentials", and permanently if they never did. The user
        // guide and the migration wizard both promise the originals stay
        // readable as this machine's fallback, and this is the line that has to
        // hold for that to be true.
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

        Self {
            portable,
            ..Self::new(backends)
        }
    }

    /// Hands a passphrase to the portable backend, if one is configured.
    ///
    /// Returns `true` when a portable backend was present and is now unlocked.
    /// `false` means the preferred backend is something else, so there was
    /// nothing to unlock — not a failure.
    ///
    /// This exists because the portable backend is unlocked *after* the manager
    /// is built: the passphrase comes from a dialog. Rebuilding the manager
    /// would not work, since `rebuild_from_settings` is only called when
    /// `SecretSettings` compares unequal and the runtime passphrase is excluded
    /// from that comparison by design.
    pub fn set_portable_passphrase(&self, passphrase: secrecy::SecretString) -> bool {
        match self.portable {
            Some(ref backend) => {
                backend.set_passphrase(passphrase);
                true
            }
            None => false,
        }
    }

    /// Drops the portable backend's passphrase and cached data key.
    ///
    /// Synchronous, and deliberately separate from the credential cache: this is
    /// the part that must happen on a shutdown path, where a spawned future is
    /// not guaranteed to run before the main loop stops.
    ///
    /// The counterpart to [`Self::set_portable_passphrase`].
    pub fn lock_portable(&self) {
        if let Some(ref backend) = self.portable {
            backend.clear_passphrase();
        }
    }

    /// Re-locks the portable backend and clears the credential cache.
    ///
    /// The cache has to go too: entries decrypted from the portable store while
    /// it was unlocked would otherwise keep answering `retrieve` after the lock,
    /// which would make locking look like it had not worked. Taking the cache's
    /// `RwLock` is the only asynchronous part, which is why [`Self::lock_portable`]
    /// exists for callers that cannot await.
    pub async fn clear_portable_passphrase(&self) {
        self.lock_portable();
        self.clear_cache().await;
    }

    /// Reports whether the portable backend is configured and unlocked.
    ///
    /// `false` when no portable backend is configured at all, which callers read
    /// as "no unlock needed".
    #[must_use]
    pub fn portable_unlocked(&self) -> bool {
        self.portable
            .as_ref()
            .is_some_and(|backend| backend.is_unlocked())
    }

    /// Replaces all backends with a fresh set built from settings
    ///
    /// Call this after settings change (e.g. user switches secret backend
    /// in Preferences) to ensure the manager uses the correct backends.
    pub fn rebuild_from_settings(&mut self, settings: &crate::config::SecretSettings) {
        let old_backend_count = self.backends.len();
        let fresh = Self::build_from_settings(settings);
        self.backends = fresh.backends;
        // Carry the typed handle across too, or an unlock after a settings save
        // would reach a backend that is no longer in the chain.
        self.portable = fresh.portable;
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
    /// replaced (Requirement 14.2). A primary [`SecretError::ReadOnly`] is
    /// returned unchanged even when `allow_fallback` is `true`: read-only is a
    /// refusal, not a failure to route around. When `allow_fallback` is `true` and every
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
        // primary error unchanged. A read-only primary is a refusal the user
        // configured, not an availability failure, so it is never routed
        // around either: writing the fallback would violate read-only and leave
        // a stale copy shadowing the vault's own value on the next read.
        if !allow_fallback || matches!(primary_error, SecretError::ReadOnly(_)) {
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

        // The chain is ordered, so "not the first entry" is exactly "not the
        // backend the user chose". Worth a `warn` when it happens: "my backend
        // works" and "my backend is broken and everything is quietly coming from
        // a local file" are otherwise indistinguishable in a log. Tracked by
        // position rather than by comparing ids, because two chain entries can
        // share a backend id.
        let primary_id = self.backends.first().map(|b| b.backend_id());

        // Try each backend in order
        for (index, backend) in self.backends.iter().enumerate() {
            if !backend.is_available().await {
                continue;
            }

            match backend.retrieve(connection_id).await {
                Ok(Some(creds)) => {
                    // Cache the result
                    if self.cache_enabled {
                        let mut cache = self.cache.write().await;
                        cache.insert(connection_id.to_string(), CacheEntry::new(creds.clone()));
                    }
                    if index != 0 {
                        tracing::warn!(
                            answered_by = %backend.backend_id(),
                            preferred = ?primary_id,
                            "credential read from a fallback backend, not the selected one"
                        );
                    }
                    return Ok(Some(creds));
                }
                Ok(None) => {}
                // A passphrase problem is not a miss and must not be walked
                // past. Absorbing it made a mistyped portable passphrase — or a
                // store the sync client replaced with one under a different
                // passphrase — resolve as `Ok(None)`, i.e. "no stored password",
                // and the user got a password prompt instead of being told the
                // passphrase was wrong. Every other error stays absorbed: a
                // KeePassXC CLI hiccup or one corrupt entry should still fall
                // through to the next backend, which is what the chain is for.
                Err(e @ (SecretError::IncorrectPassphrase | SecretError::PassphraseRequired)) => {
                    tracing::warn!(
                        backend = backend.backend_id(),
                        "credential store could not be opened; reporting rather than \
                         falling through"
                    );
                    return Err(e);
                }
                Err(e) => {
                    tracing::debug!(
                        backend = backend.backend_id(),
                        error = %e,
                        "backend could not retrieve credentials; trying the next one"
                    );
                }
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
    /// Returns `SecretError` if deletion fails on all backends, and
    /// [`SecretError::ReadOnly`] if any backend in the chain is read-only — the
    /// writable backends are still cleaned, but the read-only vault keeps its
    /// entry, so reporting success would be false.
    pub async fn delete(&self, connection_id: &str) -> SecretResult<()> {
        // Remove from cache
        if self.cache_enabled {
            let mut cache = self.cache.write().await;
            cache.remove(connection_id);
        }

        // Try to delete from all available backends
        let mut deleted = false;
        let mut last_error = None;
        let mut read_only = None;

        for backend in &self.backends {
            if !backend.is_available().await {
                continue;
            }

            match backend.delete(connection_id).await {
                Ok(()) => deleted = true,
                // A read-only refusal must not be masked by another backend's
                // idempotent `Ok` (deleting what it never held): the vault the
                // user protected still holds the entry and it will still resolve.
                Err(e @ SecretError::ReadOnly(_)) => {
                    if read_only.is_none() {
                        read_only = Some(e);
                    }
                }
                Err(e) => last_error = Some(e),
            }
        }

        if let Some(err) = read_only {
            Err(err)
        } else if deleted {
            Ok(())
        } else if let Some(err) = last_error {
            Err(err)
        } else if self.portable.as_ref().is_some_and(|p| !p.is_unlocked()) {
            // The portable backend reports itself unavailable while locked, so
            // it was skipped above and left no `last_error`. "No secret backend
            // available" would be a misleading way to say "the store this
            // credential lives in needs its passphrase".
            Err(SecretError::PassphraseRequired)
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
            // `portable` is a second handle to a backend already named in
            // `backend_ids`, and it holds the store passphrase — its unlock
            // state is reported by `portable_unlocked()`, not here.
            .finish_non_exhaustive()
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

#[cfg(test)]
mod portable_tests {
    use super::*;
    use crate::config::{SecretBackendType, SecretSettings};

    /// Settings that select the portable store at `path`, with fallback on.
    fn portable_settings(path: &std::path::Path) -> SecretSettings {
        SecretSettings {
            preferred_backend: SecretBackendType::PortableEncryptedFile,
            portable_file_path: Some(path.to_path_buf()),
            enable_fallback: true,
            ..Default::default()
        }
    }

    /// The machine-bound store must stay in the chain when portable is preferred.
    ///
    /// Excluding it — as the first version of this feature did, by lumping both
    /// file backends into one `matches!` — made every credential already in
    /// `credentials.enc` unreachable the moment the user switched backends, and
    /// permanently so if they never ran "Copy Credentials". The user guide and the
    /// migration wizard both promise the opposite.
    #[test]
    fn the_machine_bound_store_stays_in_the_chain_behind_the_portable_one() {
        let dir = tempfile::TempDir::new().unwrap();
        let manager = SecretManager::build_from_settings(&portable_settings(
            &dir.path().join("portable.enc"),
        ));

        let ids: Vec<&str> = manager.backends.iter().map(|b| b.backend_id()).collect();
        assert!(
            ids.contains(&"portable_encrypted_file"),
            "the preferred backend must be first in the chain: {ids:?}"
        );
        assert!(
            ids.contains(&"encrypted_file"),
            "the machine-bound store must remain reachable as the fallback: {ids:?}"
        );
    }

    /// The machine-bound store is its own preferred backend in that case, so
    /// appending it again would walk the same file twice on every lookup.
    #[test]
    fn the_machine_bound_store_is_not_appended_to_itself() {
        let settings = SecretSettings {
            preferred_backend: SecretBackendType::EncryptedFile,
            enable_fallback: true,
            ..Default::default()
        };

        let manager = SecretManager::build_from_settings(&settings);
        let file_backends = manager
            .backends
            .iter()
            .filter(|b| b.backend_id() == "encrypted_file")
            .count();
        assert_eq!(
            file_backends, 1,
            "the fallback must not duplicate the primary"
        );
    }

    /// `build_from_settings` is synchronous, so it is the only place that can seed
    /// a passphrase the caller already holds — `rebuild_from_settings` runs only
    /// when `SecretSettings` compares unequal, and the runtime passphrase is
    /// excluded from that comparison by design.
    #[test]
    fn a_known_passphrase_is_seeded_into_the_backend() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut settings = portable_settings(&dir.path().join("portable.enc"));

        let locked = SecretManager::build_from_settings(&settings);
        assert!(
            !locked.portable_unlocked(),
            "with no passphrase in settings the backend must start locked"
        );

        settings.portable_passphrase = Some(secrecy::SecretString::from("pass".to_owned()));
        let unlocked = SecretManager::build_from_settings(&settings);
        assert!(
            unlocked.portable_unlocked(),
            "a passphrase already in settings must reach the backend"
        );
    }

    #[test]
    fn set_and_clear_passphrase_reach_the_backend_in_the_chain() {
        let dir = tempfile::TempDir::new().unwrap();
        let manager = SecretManager::build_from_settings(&portable_settings(
            &dir.path().join("portable.enc"),
        ));
        assert!(!manager.portable_unlocked());

        assert!(
            manager.set_portable_passphrase(secrecy::SecretString::from("pass".to_owned())),
            "a configured portable backend must accept the passphrase"
        );
        assert!(manager.portable_unlocked());

        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(manager.clear_portable_passphrase());
        assert!(
            !manager.portable_unlocked(),
            "clearing must re-lock, or a session lock does nothing"
        );
    }

    #[test]
    fn setting_a_passphrase_without_a_portable_backend_reports_false() {
        let manager = SecretManager::build_from_settings(&SecretSettings::default());
        assert!(
            !manager.set_portable_passphrase(secrecy::SecretString::from("pass".to_owned())),
            "no portable backend configured means the passphrase has nowhere to go"
        );
        assert!(!manager.portable_unlocked());
    }

    /// A wrong passphrase must not be walked past as though the entry were
    /// missing. Absorbing it turned a typo into "no stored password", and the
    /// user got a password prompt instead of being told the passphrase was wrong.
    #[test]
    fn retrieve_reports_a_wrong_passphrase_instead_of_falling_through() {
        use crate::secret::portable_encrypted_file as portable;

        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("portable.enc");

        // Seed a real store under a known passphrase, using the cheap KDF
        // parameters the file format carries per-file.
        let rt = tokio::runtime::Runtime::new().unwrap();
        {
            let seeder = super::super::PortableEncryptedFileBackend::with_path(path.clone());
            seeder.set_passphrase(secrecy::SecretString::from("right".to_owned()));
            rt.block_on(async {
                seeder
                    .store(
                        "rustconn/host",
                        &crate::models::Credentials {
                            username: Some("alice".to_owned()),
                            password: Some(secrecy::SecretString::from("s3cr3t".to_owned())),
                            key_passphrase: None,
                            domain: None,
                        },
                    )
                    .await
                    .expect("seed store");
            });
        }
        assert_eq!(portable::entry_count(&path).expect("count"), 1);

        let mut settings = portable_settings(&path);
        settings.portable_passphrase = Some(secrecy::SecretString::from("wrong".to_owned()));
        // Fallback off so the assertion is about the portable backend's error and
        // not about what the machine-bound store happens to hold.
        settings.enable_fallback = false;
        let manager = SecretManager::build_from_settings(&settings);

        let result = rt.block_on(manager.retrieve("rustconn/host"));
        assert!(
            matches!(result, Err(SecretError::IncorrectPassphrase)),
            "a wrong passphrase must surface, got {result:?}"
        );
    }

    /// Deleting from a locked store must say what is wrong. Before, the backend
    /// reported itself unavailable, was skipped, and the manager returned
    /// "No secret backend available" — true in a narrow sense and useless.
    #[test]
    fn delete_on_a_locked_store_reports_the_passphrase_requirement() {
        let dir = tempfile::TempDir::new().unwrap();
        let mut settings = portable_settings(&dir.path().join("portable.enc"));
        settings.enable_fallback = false;
        let manager = SecretManager::build_from_settings(&settings);

        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt.block_on(manager.delete("rustconn/host"));
        assert!(
            matches!(result, Err(SecretError::PassphraseRequired)),
            "expected a passphrase requirement, got {result:?}"
        );
    }
}

/// `build_from_settings` must thread the persisted per-backend read-only /
/// root-search toggles into the CLI backends it constructs. If it did not, a
/// user's choice would round-trip in config and then be dropped on the floor at
/// construction — the backend would run with its defaults regardless.
#[cfg(test)]
mod cli_backend_toggle_tests {
    use super::*;
    use crate::config::{SecretBackendType, SecretSettings};

    /// The preferred backend is first in the chain; read its reported
    /// capabilities straight off the trait object.
    fn preferred(settings: &SecretSettings) -> std::sync::Arc<dyn SecretBackend> {
        let manager = SecretManager::build_from_settings(settings);
        std::sync::Arc::clone(
            manager
                .backends
                .first()
                .expect("a preferred backend must be constructed"),
        )
    }

    #[test]
    fn bitwarden_toggles_reach_the_backend() {
        let defaults = preferred(&SecretSettings {
            preferred_backend: SecretBackendType::Bitwarden,
            ..Default::default()
        });
        assert!(!defaults.is_read_only());
        assert!(!defaults.searches_from_root());

        let toggled = preferred(&SecretSettings {
            preferred_backend: SecretBackendType::Bitwarden,
            bitwarden_read_only: true,
            bitwarden_root_search: true,
            ..Default::default()
        });
        assert!(
            toggled.is_read_only(),
            "bitwarden_read_only=true must make the backend read-only"
        );
        assert!(
            toggled.searches_from_root(),
            "bitwarden_root_search=true must widen the backend's reads"
        );
    }

    #[test]
    fn onepassword_toggles_reach_the_backend() {
        let toggled = preferred(&SecretSettings {
            preferred_backend: SecretBackendType::OnePassword,
            onepassword_read_only: true,
            onepassword_root_search: true,
            ..Default::default()
        });
        assert!(toggled.is_read_only());
        assert!(toggled.searches_from_root());
    }

    #[test]
    fn passbolt_toggles_reach_the_backend() {
        let toggled = preferred(&SecretSettings {
            preferred_backend: SecretBackendType::Passbolt,
            passbolt_read_only: true,
            passbolt_root_search: true,
            ..Default::default()
        });
        assert!(toggled.is_read_only());
        assert!(toggled.searches_from_root());
    }

    #[test]
    fn pass_toggles_reach_the_backend() {
        let toggled = preferred(&SecretSettings {
            preferred_backend: SecretBackendType::Pass,
            pass_read_only: true,
            pass_root_search: true,
            ..Default::default()
        });
        assert!(toggled.is_read_only());
        assert!(toggled.searches_from_root());
    }
}

/// A read-only primary is a refusal, not an outage: the manager must neither
/// route a write around it nor let a fallback's `Ok` hide it.
#[cfg(test)]
mod read_only_chain_tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Counts writes so a test can prove a backend was never touched.
    struct CountingBackend {
        read_only: bool,
        stores: AtomicUsize,
        deletes: AtomicUsize,
    }

    impl CountingBackend {
        fn new(read_only: bool) -> Arc<Self> {
            Arc::new(Self {
                read_only,
                stores: AtomicUsize::new(0),
                deletes: AtomicUsize::new(0),
            })
        }
    }

    #[async_trait::async_trait]
    impl SecretBackend for CountingBackend {
        async fn store(&self, _id: &str, _creds: &Credentials) -> SecretResult<()> {
            self.ensure_writable()?;
            self.stores.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn retrieve(&self, _id: &str) -> SecretResult<Option<Credentials>> {
            Ok(None)
        }
        async fn delete(&self, _id: &str) -> SecretResult<()> {
            self.ensure_writable()?;
            self.deletes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        async fn is_available(&self) -> bool {
            true
        }
        fn backend_id(&self) -> &'static str {
            "counting"
        }
        fn display_name(&self) -> &'static str {
            "Counting Backend"
        }
        fn is_read_only(&self) -> bool {
            self.read_only
        }
    }

    fn chain(primary: &Arc<CountingBackend>, fallback: &Arc<CountingBackend>) -> SecretManager {
        SecretManager::new(vec![
            Arc::clone(primary) as Arc<dyn SecretBackend>,
            Arc::clone(fallback) as Arc<dyn SecretBackend>,
        ])
    }

    #[tokio::test]
    async fn store_with_fallback_does_not_route_around_a_read_only_primary() {
        let primary = CountingBackend::new(true);
        let fallback = CountingBackend::new(false);
        let manager = chain(&primary, &fallback);

        let result = manager
            .store_reported("conn", &Credentials::default(), true)
            .await;

        assert!(
            matches!(result, Err(SecretError::ReadOnly(ref name)) if name == "Counting Backend"),
            "a read-only primary must be reported, got {result:?}"
        );
        assert_eq!(
            fallback.stores.load(Ordering::SeqCst),
            0,
            "the fallback must not receive a write the user's vault refused"
        );
    }

    #[tokio::test]
    async fn delete_reports_read_only_even_when_a_fallback_succeeds() {
        let primary = CountingBackend::new(true);
        let fallback = CountingBackend::new(false);
        let manager = chain(&primary, &fallback);

        let result = manager.delete("conn").await;

        assert!(
            matches!(result, Err(SecretError::ReadOnly(_))),
            "the read-only vault still holds the entry, got {result:?}"
        );
        // The writable backend is still cleaned up.
        assert_eq!(fallback.deletes.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn store_still_falls_back_past_a_writable_primary_failure() {
        // Control: a fallback-able failure keeps the historical behaviour.
        struct Down;
        #[async_trait::async_trait]
        impl SecretBackend for Down {
            async fn store(&self, _id: &str, _c: &Credentials) -> SecretResult<()> {
                Err(SecretError::BackendUnavailable("down".to_string()))
            }
            async fn retrieve(&self, _id: &str) -> SecretResult<Option<Credentials>> {
                Ok(None)
            }
            async fn delete(&self, _id: &str) -> SecretResult<()> {
                Ok(())
            }
            async fn is_available(&self) -> bool {
                false
            }
            fn backend_id(&self) -> &'static str {
                "down"
            }
            fn display_name(&self) -> &'static str {
                "Down"
            }
        }

        let fallback = CountingBackend::new(false);
        let manager = SecretManager::new(vec![
            Arc::new(Down) as Arc<dyn SecretBackend>,
            Arc::clone(&fallback) as Arc<dyn SecretBackend>,
        ]);

        let result = manager
            .store_reported("conn", &Credentials::default(), true)
            .await;
        assert!(
            matches!(result, Ok(StoreOutcome::Fallback { .. })),
            "got {result:?}"
        );
        assert_eq!(fallback.stores.load(Ordering::SeqCst), 1);
    }
}
