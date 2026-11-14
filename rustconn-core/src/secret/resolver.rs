//! Credential resolution chain for connections
//!
//! This module provides the `CredentialResolver` which resolves credentials
//! for connections based on their configuration and available backends.

use std::sync::Arc;

use tracing::{debug, warn};

use super::backend::LookupIdentity;
use super::hierarchy::KeePassHierarchy;
use super::manager::SecretManager;
use super::verification::{CredentialStatus, CredentialVerificationManager, VerifiedCredentials};
use crate::config::{SecretBackendType, SecretSettings};
use crate::error::SecretResult;
use crate::models::{Connection, ConnectionGroup, Credentials, PasswordSource};

/// Resolves the effective KeePass (`.kdbx`) database path for a connection
/// (issue [#411](https://github.com/totoshko88/RustConn/issues/411)).
///
/// Resolution order:
/// 1. the connection's own `kdbx_path_override`, if set;
/// 2. otherwise the nearest ancestor group in the connection's group chain that
///    sets `kdbx_path_override`, walking parent by parent;
/// 3. otherwise `None`, meaning "use the global database from settings".
///
/// Group-chain traversal is cycle-safe: a malformed parent loop is broken by a
/// visited-set guard, mirroring the `Inherit` credential walk.
///
/// This decides only *which file* to open. The database password, key file and
/// YubiKey slot still come from the global KeePass settings, so an override
/// database that shares the organisation's master password works; one that needs
/// a different passphrase is deliberately out of scope for the per-connection
/// override.
#[must_use]
pub fn resolve_kdbx_path_override(
    connection: &Connection,
    groups: &[ConnectionGroup],
) -> Option<std::path::PathBuf> {
    if let Some(path) = connection.kdbx_path_override.as_ref() {
        return Some(path.clone());
    }
    resolve_group_kdbx_path_override(connection.group_id, groups)
}

/// Resolves the effective KeePass (`.kdbx`) database override for a group:
/// the group's own `kdbx_path_override`, else the nearest ancestor's, else
/// `None` (use the global database).
///
/// This is where a group's own Vault credential lives, and the part of
/// [`resolve_kdbx_path_override`] a connection inherits. `None` for
/// `group_id` (an ungrouped connection) resolves to `None`.
#[must_use]
pub fn resolve_group_kdbx_path_override(
    group_id: Option<uuid::Uuid>,
    groups: &[ConnectionGroup],
) -> Option<std::path::PathBuf> {
    let mut current_group_id = group_id;
    let mut visited = std::collections::HashSet::new();
    while let Some(group_id) = current_group_id {
        if !visited.insert(group_id) {
            warn!(
                group_id = %group_id,
                "cycle detected in group hierarchy while resolving kdbx_path_override"
            );
            break;
        }
        let Some(group) = groups.iter().find(|g| g.id == group_id) else {
            break;
        };
        if let Some(path) = group.kdbx_path_override.as_ref() {
            return Some(path.clone());
        }
        current_group_id = group.parent_id;
    }
    None
}

/// Resolves credentials for a connection based on configuration
///
/// The resolver implements a fallback chain that tries multiple credential
/// sources in order based on the connection's `password_source` setting
/// and the application's secret settings.
pub struct CredentialResolver {
    /// Secret manager for backend access
    secret_manager: Arc<SecretManager>,
    /// Secret settings for configuration
    settings: SecretSettings,
}

impl CredentialResolver {
    /// Creates a new `CredentialResolver`
    ///
    /// # Arguments
    /// * `secret_manager` - The secret manager with configured backends
    /// * `settings` - Secret settings for configuration
    #[must_use]
    pub const fn new(secret_manager: Arc<SecretManager>, settings: SecretSettings) -> Self {
        Self {
            secret_manager,
            settings,
        }
    }

    /// Generates a lookup key for `KeePass` entry retrieval
    ///
    /// The key format is: `rustconn/{name}` where name is the connection name.
    /// If the connection name is empty, falls back to using the host.
    ///
    /// # Arguments
    /// * `connection` - The connection to generate a key for
    ///
    /// # Returns
    /// A string key suitable for `KeePass` entry lookup
    #[must_use]
    pub fn generate_lookup_key(connection: &Connection) -> String {
        let identifier = if connection.name.trim().is_empty() {
            &connection.host
        } else {
            &connection.name
        };
        format!("rustconn/{identifier}")
    }

    /// Alias-aware primary vault lookup for the manual-vault backends.
    ///
    /// Issue #353: a root-search backend should match a vault entry titled by
    /// the connection's human-readable **name** or **host**, not only by the
    /// `rustconn/{name}` lookup key or the UUID. This threads the connection's
    /// name and host alongside `lookup_key` into
    /// [`SecretManager::retrieve_identity`], so a root-search-enabled backend
    /// can widen its match to them. A scoped (non root-search) backend ignores
    /// the aliases and behaves exactly as a plain `retrieve(lookup_key)` call.
    async fn retrieve_vault_identity(
        &self,
        lookup_key: &str,
        connection: &Connection,
    ) -> SecretResult<Option<Credentials>> {
        let name = connection.name.trim();
        let host = connection.host.trim();
        let identity = LookupIdentity {
            key: lookup_key,
            name: (!name.is_empty()).then_some(name),
            host: (!host.is_empty()).then_some(host),
        };
        self.secret_manager.retrieve_identity(identity).await
    }

    /// Generates a lookup key for libsecret/keyring storage (legacy flat format)
    ///
    /// The key format is: `{name} ({protocol})` — the pre-0.19.18 format.
    /// Kept for backward-compatible fallback lookups.
    ///
    /// # Arguments
    /// * `connection` - The connection to generate a key for
    ///
    /// # Returns
    /// A string key suitable for keyring entry lookup (legacy format)
    #[must_use]
    pub fn generate_keyring_key(connection: &Connection) -> String {
        let protocol = connection.protocol_config.protocol_type();
        let name =
            crate::import::sanitize_imported_value(&connection.name.trim().replace('/', "-"));
        format!("{} ({})", name, protocol.as_str().to_lowercase())
    }

    /// Generates a hierarchical lookup key for libsecret/keyring storage.
    ///
    /// The key format is: `RustConn/{group_path}/{name} ({protocol})`
    /// For connections without a group: `RustConn/{name} ({protocol})`
    ///
    /// This format:
    /// - Prevents collisions between same-named connections in different groups
    /// - Uses `/` as separator for KDE Wallet folder hierarchy compatibility
    /// - Prefixes with `RustConn` for namespace clarity
    ///
    /// # Arguments
    /// * `connection` - The connection to generate a key for
    /// * `groups` - All available connection groups for hierarchy resolution
    ///
    /// # Returns
    /// A string key suitable for keyring entry lookup
    #[must_use]
    pub fn generate_keyring_key_with_hierarchy(
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> String {
        let protocol = connection.protocol_config.protocol_type();
        let name =
            crate::import::sanitize_imported_value(&connection.name.trim().replace('/', "-"));
        let suffix = format!("{name} ({})", protocol.as_str().to_lowercase());

        let mut path_parts = vec!["RustConn".to_string()];

        if let Some(group_id) = connection.group_id {
            let group_path = KeePassHierarchy::resolve_group_path(group_id, groups);
            path_parts.extend(group_path);
        }

        path_parts.push(suffix);
        path_parts.join("/")
    }

    /// Resolves credentials for a connection
    ///
    /// Resolution based on `password_source`:
    /// - `Vault` → resolve from the configured secret backend
    /// - `Variable(name)` → resolve from secret variable stored in vault
    /// - `Prompt` / `Inherit` → caller handles
    /// - `None` → try fallback chain if enabled
    ///
    /// # Arguments
    /// * `connection` - The connection to resolve credentials for
    ///
    /// # Returns
    /// `Some(Credentials)` if found from any source, `None` if not found
    ///
    /// # Errors
    /// Returns `SecretError` if backend operations fail
    #[tracing::instrument(skip(self, connection), fields(connection_id = %connection.id, password_source = ?connection.password_source, host = %connection.host))]
    pub async fn resolve(&self, connection: &Connection) -> SecretResult<Option<Credentials>> {
        debug!(
            connection_name = %connection.name,
            "Resolving credentials"
        );

        let result = match connection.password_source {
            PasswordSource::Vault => self.resolve_from_vault(connection).await,
            PasswordSource::Variable(ref name) => self.resolve_from_variable(name).await,
            PasswordSource::Script(ref command) => {
                super::script_resolver::resolve_script(command).await
            }
            PasswordSource::Prompt | PasswordSource::Inherit => {
                // Caller handles these cases.
                // Inherit requires group hierarchy — use resolve_with_hierarchy() instead.
                if matches!(connection.password_source, PasswordSource::Inherit) {
                    warn!(
                        connection_id = %connection.id,
                        connection_name = %connection.name,
                        "resolve() called with PasswordSource::Inherit — use resolve_with_hierarchy() instead"
                    );
                }
                debug!("Password source requires caller handling");
                Ok(None)
            }
            PasswordSource::None => {
                // Try fallback chain if enabled
                if self.settings.enable_fallback {
                    self.resolve_with_fallback(connection).await
                } else {
                    Ok(None)
                }
            }
        };

        match &result {
            Ok(Some(_)) => debug!("Credentials resolved successfully"),
            Ok(None) => debug!("No credentials found"),
            Err(e) => warn!(error = %e, "Credential resolution failed"),
        }

        result
    }

    /// Resolves credentials from the configured vault backend
    ///
    /// Delegates to the appropriate backend based on `preferred_backend`
    /// in settings. Tries KeePass first if enabled, then keyring, then
    /// other backends. Falls back through the chain if enabled.
    async fn resolve_from_vault(
        &self,
        connection: &Connection,
    ) -> SecretResult<Option<Credentials>> {
        let backend = self.select_storage_backend();
        match backend {
            SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
                self.resolve_from_keepass(connection).await
            }
            SecretBackendType::LibSecret | SecretBackendType::MacOsKeychain => {
                self.resolve_from_keyring(connection).await
            }
            SecretBackendType::Bitwarden => self.resolve_from_bitwarden(connection).await,
            SecretBackendType::OnePassword => self.resolve_from_onepassword(connection).await,
            SecretBackendType::Passbolt => self.resolve_from_passbolt(connection).await,
            SecretBackendType::Pass => self.resolve_from_pass(connection).await,
            SecretBackendType::EncryptedFile => {
                // Flat-key backend: retrieve via the secret manager using the
                // same flat lookup key as store_unified, keeping store/resolve
                // symmetric with the other app-managed backends.
                let lookup_key = Self::generate_lookup_key(connection);
                self.secret_manager.retrieve(&lookup_key).await
            }
            SecretBackendType::PortableEncryptedFile => {
                // Passphrase-based flat-key backend: same lookup key scheme.
                let lookup_key = Self::generate_lookup_key(connection);
                self.secret_manager.retrieve(&lookup_key).await
            }
        }
    }

    /// Resolves credentials from a secret global variable
    ///
    /// Looks up the variable value from the secret backend using the
    /// key format `rustconn/var/{name}`.
    async fn resolve_from_variable(
        &self,
        variable_name: &str,
    ) -> SecretResult<Option<Credentials>> {
        let lookup_key = crate::variables::variable_secret_key(variable_name);
        debug!(
            variable_name,
            lookup_key = %lookup_key,
            "Resolving from variable"
        );

        if let Some(creds) = self.secret_manager.retrieve(&lookup_key).await? {
            return Ok(Some(creds));
        }

        debug!(variable_name, "No secret found in vault for variable");
        Ok(None)
    }

    /// Resolves credentials from `KeePass`
    async fn resolve_from_keepass(
        &self,
        connection: &Connection,
    ) -> SecretResult<Option<Credentials>> {
        if !self.settings.kdbx_enabled {
            // `KeePass` not enabled, try fallback if allowed
            if self.settings.enable_fallback {
                return self.resolve_from_keyring(connection).await;
            }
            return Ok(None);
        }

        let lookup_key = Self::generate_lookup_key(connection);
        let result = self
            .retrieve_vault_identity(&lookup_key, connection)
            .await?;

        if result.is_some() {
            return Ok(result);
        }

        // `KeePass` lookup failed, try fallback if enabled
        if self.settings.enable_fallback {
            self.resolve_from_keyring(connection).await
        } else {
            Ok(None)
        }
    }

    /// Resolves credentials from system keyring (libsecret)
    async fn resolve_from_keyring(
        &self,
        connection: &Connection,
    ) -> SecretResult<Option<Credentials>> {
        // Use the same key format as used when saving: "{name} ({protocol})"
        // Sanitize the name to strip any trailing escape sequences (e.g. \n
        // from Remmina INI files) so the lookup key matches what was stored.
        let protocol = connection.protocol_config.protocol_type();
        let name =
            crate::import::sanitize_imported_value(&connection.name.trim().replace('/', "-"));
        let lookup_key = format!("{} ({})", name, protocol.as_str().to_lowercase());

        // Try the new format first
        if let Some(creds) = self.secret_manager.retrieve(&lookup_key).await? {
            return Ok(Some(creds));
        }

        // Fall back to legacy UUID-based key for backward compatibility
        let connection_id = connection.id.to_string();
        self.secret_manager.retrieve(&connection_id).await
    }

    /// Resolves credentials from system keyring using hierarchical key format.
    ///
    /// Tries lookup keys in order:
    /// 1. New hierarchical: `RustConn/{group_path}/{name} ({protocol})`
    /// 2. Legacy flat: `{name} ({protocol})`
    /// 3. Legacy UUID-based key
    ///
    /// When found under a legacy key, the credential is transparently migrated
    /// to the new hierarchical key for future lookups.
    async fn resolve_from_keyring_hierarchical(
        &self,
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> SecretResult<Option<Credentials>> {
        // 1. Try new hierarchical format first
        let hierarchical_key = Self::generate_keyring_key_with_hierarchy(connection, groups);
        if let Some(creds) = self.secret_manager.retrieve(&hierarchical_key).await? {
            return Ok(Some(creds));
        }

        // 2. Try legacy flat format: "{name} ({protocol})"
        let legacy_key = Self::generate_keyring_key(connection);
        if let Some(creds) = self.secret_manager.retrieve(&legacy_key).await? {
            // Migrate to new format transparently
            let _ = self.secret_manager.store(&hierarchical_key, &creds).await;
            let _ = self.secret_manager.delete(&legacy_key).await;
            return Ok(Some(creds));
        }

        // 3. Fall back to legacy UUID-based key
        let connection_id = connection.id.to_string();
        if let Some(creds) = self.secret_manager.retrieve(&connection_id).await? {
            // Migrate to new format transparently
            let _ = self.secret_manager.store(&hierarchical_key, &creds).await;
            let _ = self.secret_manager.delete(&connection_id).await;
            return Ok(Some(creds));
        }

        Ok(None)
    }

    /// Resolves credentials from Bitwarden vault
    async fn resolve_from_bitwarden(
        &self,
        connection: &Connection,
    ) -> SecretResult<Option<Credentials>> {
        // Primary: use the same key format as store_unified: "rustconn/{name}".
        // Alias-aware (issue #353): a root-search backend also matches a vault
        // entry titled by the connection name or host.
        let lookup_key = Self::generate_lookup_key(connection);
        if let Some(creds) = self
            .retrieve_vault_identity(&lookup_key, connection)
            .await?
        {
            return Ok(Some(creds));
        }

        // Fallback: "{name} ({protocol})" format for backward compatibility
        let protocol = connection.protocol_config.protocol_type();
        let name =
            crate::import::sanitize_imported_value(&connection.name.trim().replace('/', "-"));
        let alt_key = format!("{} ({})", name, protocol.as_str().to_lowercase());
        if let Some(creds) = self.secret_manager.retrieve(&alt_key).await? {
            return Ok(Some(creds));
        }

        // Legacy: UUID-based key
        let connection_id = connection.id.to_string();
        self.secret_manager.retrieve(&connection_id).await
    }

    /// Resolves credentials from 1Password vault
    async fn resolve_from_onepassword(
        &self,
        connection: &Connection,
    ) -> SecretResult<Option<Credentials>> {
        // Primary: use the same key format as store_unified: "rustconn/{name}".
        // Alias-aware (issue #353): a root-search backend also matches a vault
        // entry titled by the connection name or host.
        let lookup_key = Self::generate_lookup_key(connection);
        if let Some(creds) = self
            .retrieve_vault_identity(&lookup_key, connection)
            .await?
        {
            return Ok(Some(creds));
        }

        // Fallback: UUID-based key (matches OnePasswordBackend::entry_title)
        let connection_id = connection.id.to_string();
        if let Some(creds) = self.secret_manager.retrieve(&connection_id).await? {
            return Ok(Some(creds));
        }

        // Legacy: "{name} ({protocol})" format
        let protocol = connection.protocol_config.protocol_type();
        let name = connection.name.replace('/', "-");
        let alt_key = format!("{} ({})", name, protocol.as_str().to_lowercase());
        self.secret_manager.retrieve(&alt_key).await
    }

    /// Resolves credentials from Passbolt server
    async fn resolve_from_passbolt(
        &self,
        connection: &Connection,
    ) -> SecretResult<Option<Credentials>> {
        // Primary: use the same key format as store_unified: "rustconn/{name}".
        // Alias-aware (issue #353): a root-search backend also matches a vault
        // entry titled by the connection name or host.
        let lookup_key = Self::generate_lookup_key(connection);
        if let Some(creds) = self
            .retrieve_vault_identity(&lookup_key, connection)
            .await?
        {
            return Ok(Some(creds));
        }

        // Fallback: UUID-based key (matches PassboltBackend::entry_name)
        let connection_id = connection.id.to_string();
        if let Some(creds) = self.secret_manager.retrieve(&connection_id).await? {
            return Ok(Some(creds));
        }

        // Legacy: "{name} ({protocol})" format
        let protocol = connection.protocol_config.protocol_type();
        let name = connection.name.replace('/', "-");
        let alt_key = format!("{} ({})", name, protocol.as_str().to_lowercase());
        self.secret_manager.retrieve(&alt_key).await
    }

    /// Resolves credentials from Pass (Unix password manager)
    async fn resolve_from_pass(
        &self,
        connection: &Connection,
    ) -> SecretResult<Option<Credentials>> {
        // Primary: use the same key format as store_unified: "rustconn/{name}".
        // Alias-aware (issue #353): a root-search backend also matches a store
        // entry at the root named by the connection name or host.
        let lookup_key = Self::generate_lookup_key(connection);
        if let Some(creds) = self
            .retrieve_vault_identity(&lookup_key, connection)
            .await?
        {
            return Ok(Some(creds));
        }

        // Fallback: "{name} ({protocol})" format
        let protocol = connection.protocol_config.protocol_type();
        let name = connection.name.replace('/', "-");
        let alt_key = format!("{} ({})", name, protocol.as_str().to_lowercase());
        self.secret_manager.retrieve(&alt_key).await
    }

    /// Resolves credentials using the fallback chain
    ///
    /// Tries sources in order: `KeePass` (if enabled) -> Keyring
    async fn resolve_with_fallback(
        &self,
        connection: &Connection,
    ) -> SecretResult<Option<Credentials>> {
        // Try `KeePass` first if enabled
        if self.settings.kdbx_enabled {
            let lookup_key = Self::generate_lookup_key(connection);
            if let Some(creds) = self
                .retrieve_vault_identity(&lookup_key, connection)
                .await?
            {
                return Ok(Some(creds));
            }
        }

        // Fall back to keyring
        self.resolve_from_keyring(connection).await
    }

    /// Checks if `KeePass` integration is currently active
    #[must_use]
    pub const fn is_keepass_active(&self) -> bool {
        self.settings.kdbx_enabled && self.settings.kdbx_path.is_some()
    }

    /// Selects the appropriate storage backend based on settings
    ///
    /// Returns the preferred backend type based on configuration.
    ///
    /// # Returns
    /// The selected backend type
    #[must_use]
    pub const fn select_storage_backend(&self) -> SecretBackendType {
        match self.settings.preferred_backend {
            SecretBackendType::Bitwarden => SecretBackendType::Bitwarden,
            SecretBackendType::OnePassword => SecretBackendType::OnePassword,
            SecretBackendType::Passbolt => SecretBackendType::Passbolt,
            SecretBackendType::Pass => SecretBackendType::Pass,
            SecretBackendType::MacOsKeychain => SecretBackendType::MacOsKeychain,
            SecretBackendType::KeePassXc | SecretBackendType::KdbxFile => {
                if self.settings.kdbx_enabled && self.settings.kdbx_path.is_some() {
                    SecretBackendType::KdbxFile
                } else if self.settings.enable_fallback {
                    SecretBackendType::LibSecret
                } else {
                    self.settings.preferred_backend
                }
            }
            SecretBackendType::LibSecret => SecretBackendType::LibSecret,
            // EncryptedFile is a flat-key backend (no hierarchy); identity
            // mapping mirrors the other app-managed flat-key backends above.
            SecretBackendType::EncryptedFile => SecretBackendType::EncryptedFile,
            // PortableEncryptedFile is also a flat-key backend (passphrase-based).
            SecretBackendType::PortableEncryptedFile => SecretBackendType::PortableEncryptedFile,
        }
    }

    /// Checks if credentials need migration from Keyring to `KeePass`
    ///
    /// Returns true if:
    /// - `KeePass` integration is enabled
    /// - Credentials exist in Keyring for the connection
    /// - Credentials do NOT exist in `KeePass` for the connection
    ///
    /// # Arguments
    /// * `connection` - The connection to check
    ///
    /// # Returns
    /// `true` if migration is needed, `false` otherwise
    ///
    /// # Errors
    /// Returns `SecretError` if backend operations fail
    pub async fn needs_keepass_migration(&self, connection: &Connection) -> SecretResult<bool> {
        // Migration only makes sense if KeePass is enabled
        if !self.settings.kdbx_enabled || self.settings.kdbx_path.is_none() {
            return Ok(false);
        }

        // Check if credentials exist in Keyring
        let keyring_creds = self.resolve_from_keyring(connection).await?;
        if keyring_creds.is_none() {
            return Ok(false);
        }

        // Check if credentials exist in KeePass
        let lookup_key = Self::generate_lookup_key(connection);
        let keepass_creds = self.secret_manager.retrieve(&lookup_key).await?;

        // Need migration if in Keyring but not in KeePass
        Ok(keepass_creds.is_none())
    }

    /// Migrates credentials from Keyring to `KeePass`
    ///
    /// Copies credentials from Keyring to `KeePass` and optionally deletes
    /// them from Keyring after successful copy.
    ///
    /// # Arguments
    /// * `connection` - The connection to migrate credentials for
    /// * `delete_from_keyring` - Whether to delete from Keyring after migration
    ///
    /// # Returns
    /// `true` if migration was successful, `false` if no credentials to migrate
    ///
    /// # Errors
    /// Returns `SecretError` if backend operations fail
    pub async fn migrate_to_keepass(
        &self,
        connection: &Connection,
        delete_from_keyring: bool,
    ) -> SecretResult<bool> {
        // Get credentials from Keyring
        let keyring_creds = self.resolve_from_keyring(connection).await?;
        let Some(creds) = keyring_creds else {
            return Ok(false);
        };

        // Store in KeePass
        let lookup_key = Self::generate_lookup_key(connection);
        self.secret_manager.store(&lookup_key, &creds).await?;

        // Optionally delete from Keyring
        if delete_from_keyring {
            let connection_id = connection.id.to_string();
            let _ = self.secret_manager.delete(&connection_id).await;
        }

        Ok(true)
    }

    /// Checks if the system Keyring is available
    ///
    /// # Returns
    /// `true` if Keyring is available, `false` otherwise
    pub async fn is_keyring_available(&self) -> bool {
        // Check if libsecret backend is available
        let available = self.secret_manager.available_backends().await;
        available.contains(&"libsecret")
    }

    /// Stores credentials using the unified storage backend
    ///
    /// Automatically selects backend based on settings.
    ///
    /// # Arguments
    /// * `connection` - The connection to store credentials for
    /// * `credentials` - The credentials to store
    ///
    /// # Errors
    /// Returns `SecretError` if storage fails
    pub async fn store_unified(
        &self,
        connection: &Connection,
        credentials: &Credentials,
    ) -> SecretResult<()> {
        let backend = self.select_storage_backend();

        match backend {
            SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
                let lookup_key = Self::generate_lookup_key(connection);
                self.secret_manager.store(&lookup_key, credentials).await
            }
            SecretBackendType::LibSecret | SecretBackendType::MacOsKeychain => {
                let lookup_key = Self::generate_keyring_key(connection);
                self.secret_manager.store(&lookup_key, credentials).await
            }
            SecretBackendType::Bitwarden
            | SecretBackendType::OnePassword
            | SecretBackendType::Passbolt
            | SecretBackendType::Pass => {
                // For Bitwarden/1Password/Passbolt/Pass, use connection name as identifier
                let lookup_key = Self::generate_lookup_key(connection);
                self.secret_manager.store(&lookup_key, credentials).await
            }
            SecretBackendType::EncryptedFile => {
                // Flat-key backend: store via the secret manager using the same
                // flat lookup key as the other app-managed backends.
                let lookup_key = Self::generate_lookup_key(connection);
                self.secret_manager.store(&lookup_key, credentials).await
            }
            SecretBackendType::PortableEncryptedFile => {
                // Passphrase-based flat-key backend: same store scheme.
                let lookup_key = Self::generate_lookup_key(connection);
                self.secret_manager.store(&lookup_key, credentials).await
            }
        }
    }

    /// Resolves credentials with verification status
    ///
    /// This method combines credential resolution with verification tracking
    /// to determine whether credentials can be used automatically or if the
    /// password dialog should be shown.
    ///
    /// # Arguments
    /// * `connection` - The connection to resolve credentials for
    /// * `verification_manager` - Manager tracking verification status
    ///
    /// # Returns
    /// `VerifiedCredentials` with status information
    ///
    /// # Errors
    /// Returns `SecretError` if backend operations fail
    pub async fn resolve_verified(
        &self,
        connection: &Connection,
        verification_manager: &CredentialVerificationManager,
    ) -> SecretResult<VerifiedCredentials> {
        // Get verification status for this connection
        let status = verification_manager.get_status(connection.id);

        // If credentials are not verified, always show dialog
        if !status.is_verified() {
            return Ok(VerifiedCredentials::new(
                connection.username.clone(),
                None,
                connection.domain.clone(),
                status,
            ));
        }

        // Try to resolve credentials from storage
        let credentials = self.resolve(connection).await?;

        match credentials {
            Some(creds) => {
                // Found credentials and they're verified - can use automatically
                Ok(VerifiedCredentials::new(
                    creds.username,
                    creds.password,
                    connection.domain.clone(),
                    status,
                ))
            }
            None => {
                // No credentials found - need to prompt
                Ok(VerifiedCredentials::new(
                    connection.username.clone(),
                    None,
                    connection.domain.clone(),
                    CredentialStatus::new(),
                ))
            }
        }
    }

    /// Generates a hierarchical lookup key for `KeePass` entry retrieval.
    ///
    /// The key format is: `RustConn/GroupA/SubGroup/ConnectionName`
    /// This mirrors the connection's group hierarchy in the `KeePass` database.
    ///
    /// # Arguments
    /// * `connection` - The connection to generate a key for
    /// * `groups` - All available connection groups for hierarchy resolution
    ///
    /// # Returns
    /// A string key suitable for hierarchical `KeePass` entry lookup
    #[must_use]
    pub fn generate_hierarchical_lookup_key(
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> String {
        KeePassHierarchy::build_entry_path(connection, groups)
    }

    /// Resolves credentials for a connection using hierarchical paths.
    ///
    /// This is the hierarchical version of `resolve()` that uses the connection's
    /// group structure to determine the `KeePass` entry path.
    ///
    /// # Arguments
    /// * `connection` - The connection to resolve credentials for
    /// * `groups` - All available connection groups for hierarchy resolution
    ///
    /// # Returns
    /// `Some(Credentials)` if found from any source, `None` if not found
    ///
    /// # Errors
    /// Returns `SecretError` if backend operations fail
    pub async fn resolve_with_hierarchy(
        &self,
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> SecretResult<Option<Credentials>> {
        match connection.password_source {
            PasswordSource::Vault => {
                self.resolve_from_vault_hierarchical(connection, groups)
                    .await
            }
            PasswordSource::Variable(ref name) => self.resolve_from_variable(name).await,
            PasswordSource::Script(ref command) => {
                super::script_resolver::resolve_script(command).await
            }
            PasswordSource::Inherit => self.resolve_inherited_credentials(connection, groups).await,
            PasswordSource::Prompt => {
                // Caller handles these cases
                Ok(None)
            }
            PasswordSource::None => {
                // Try fallback chain if enabled
                if self.settings.enable_fallback {
                    self.resolve_with_fallback_hierarchical(connection, groups)
                        .await
                } else {
                    Ok(None)
                }
            }
        }
    }

    /// Resolves credentials from vault using hierarchical paths
    ///
    /// Delegates to the appropriate backend based on settings,
    /// using hierarchical KeePass paths when applicable.
    async fn resolve_from_vault_hierarchical(
        &self,
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> SecretResult<Option<Credentials>> {
        let backend = self.select_storage_backend();
        match backend {
            SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
                self.resolve_from_keepass_hierarchical(connection, groups)
                    .await
            }
            SecretBackendType::LibSecret | SecretBackendType::MacOsKeychain => {
                self.resolve_from_keyring_hierarchical(connection, groups)
                    .await
            }
            SecretBackendType::Bitwarden => self.resolve_from_bitwarden(connection).await,
            SecretBackendType::OnePassword => self.resolve_from_onepassword(connection).await,
            SecretBackendType::Passbolt => self.resolve_from_passbolt(connection).await,
            SecretBackendType::Pass => self.resolve_from_pass(connection).await,
            SecretBackendType::EncryptedFile => {
                // Flat-key backend with no hierarchy: use the flat lookup key
                // (the stores use the same flat key).
                let lookup_key = Self::generate_lookup_key(connection);
                self.secret_manager.retrieve(&lookup_key).await
            }
            SecretBackendType::PortableEncryptedFile => {
                // Passphrase-based flat-key backend (no hierarchy).
                let lookup_key = Self::generate_lookup_key(connection);
                self.secret_manager.retrieve(&lookup_key).await
            }
        }
    }

    /// Resolves credentials from `KeePass` using hierarchical path
    async fn resolve_from_keepass_hierarchical(
        &self,
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> SecretResult<Option<Credentials>> {
        if !self.settings.kdbx_enabled {
            // `KeePass` not enabled, try fallback if allowed
            if self.settings.enable_fallback {
                return self
                    .resolve_from_keyring_hierarchical(connection, groups)
                    .await;
            }
            return Ok(None);
        }

        let lookup_key = Self::generate_hierarchical_lookup_key(connection, groups);
        let result = self.secret_manager.retrieve(&lookup_key).await?;

        if result.is_some() {
            return Ok(result);
        }

        // Try legacy flat key as fallback for migration
        let legacy_key = Self::generate_lookup_key(connection);
        let legacy_result = self.secret_manager.retrieve(&legacy_key).await?;

        if legacy_result.is_some() {
            return Ok(legacy_result);
        }

        // `KeePass` lookup failed, try fallback if enabled
        if self.settings.enable_fallback {
            self.resolve_from_keyring_hierarchical(connection, groups)
                .await
        } else {
            Ok(None)
        }
    }

    /// Resolves credentials using the fallback chain with hierarchical paths
    async fn resolve_with_fallback_hierarchical(
        &self,
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> SecretResult<Option<Credentials>> {
        // Try `KeePass` first if enabled (with hierarchical path)
        if self.settings.kdbx_enabled {
            let lookup_key = Self::generate_hierarchical_lookup_key(connection, groups);
            if let Some(creds) = self.secret_manager.retrieve(&lookup_key).await? {
                return Ok(Some(creds));
            }

            // Try legacy flat key as fallback
            let legacy_key = Self::generate_lookup_key(connection);
            if let Some(creds) = self.secret_manager.retrieve(&legacy_key).await? {
                return Ok(Some(creds));
            }
        }

        // Fall back to keyring (with hierarchical key + legacy fallback)
        self.resolve_from_keyring_hierarchical(connection, groups)
            .await
    }

    /// Resolves credentials by inheriting from parent groups.
    ///
    /// See also: `resolve_credentials_blocking()` in `rustconn/src/state.rs`
    /// for the synchronous (blocking) equivalent used in the GUI crate.
    async fn resolve_inherited_credentials(
        &self,
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> SecretResult<Option<Credentials>> {
        let mut current_group_id = connection.group_id;
        let mut visited = std::collections::HashSet::new();

        // Traverse up the hierarchy
        while let Some(group_id) = current_group_id {
            // Cycle detection
            if !visited.insert(group_id) {
                warn!(
                    group_id = %group_id,
                    connection_id = %connection.id,
                    "Cycle detected in group hierarchy during Inherit resolution"
                );
                break;
            }

            // Find the group
            let Some(group) = groups.iter().find(|g| g.id == group_id) else {
                break;
            };

            // Check if this group has credentials configured
            if let Some(source) = group.password_source.as_ref() {
                match source {
                    PasswordSource::Vault => {
                        // Use the configured backend to look up group creds
                        let backend = self.select_storage_backend();
                        match backend {
                            SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
                                if self.settings.kdbx_enabled {
                                    let group_path =
                                        KeePassHierarchy::build_group_entry_path(group, groups);
                                    if let Some(creds) =
                                        self.secret_manager.retrieve(&group_path).await?
                                    {
                                        return Ok(Some(
                                            self.merge_group_credentials(creds, group),
                                        ));
                                    }
                                }
                            }
                            _ => {
                                // The key is the UUID; the name rides along so a
                                // backend searching from root can match an entry
                                // titled by it, as the GUI's Inherit path does
                                // (issue #353).
                                let group_id_str = group.id.to_string();
                                let name = group.name.trim();
                                let identity = LookupIdentity {
                                    key: &group_id_str,
                                    name: (!name.is_empty()).then_some(name),
                                    host: None,
                                };
                                if let Some(creds) =
                                    self.secret_manager.retrieve_identity(identity).await?
                                {
                                    return Ok(Some(self.merge_group_credentials(creds, group)));
                                }
                            }
                        }
                    }
                    PasswordSource::Variable(name) => {
                        if let Some(creds) = self.resolve_from_variable(name).await? {
                            return Ok(Some(self.merge_group_credentials(creds, group)));
                        }
                    }
                    PasswordSource::Inherit => {
                        // explicitly continue to parent
                    }
                    _ => {}
                }
            }

            // Move to parent
            current_group_id = group.parent_id;
        }

        Ok(None)
    }

    /// Merges resolved credentials with group overrides (username, domain)
    #[expect(
        clippy::unused_self,
        reason = "method is part of a uniform helper API where most operations need &self; keeping &self preserves the consistent signature"
    )]
    fn merge_group_credentials(
        &self,
        mut creds: Credentials,
        group: &ConnectionGroup,
    ) -> Credentials {
        if let Some(ref username) = group.username {
            creds.username = Some(username.clone());
        }
        if let Some(ref domain) = group.domain {
            creds.domain = Some(domain.clone());
        }
        creds
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::models::{ConnectionGroup, ProtocolConfig, ProtocolType, SshConfig};

    fn create_test_connection(name: &str, host: &str) -> Connection {
        Connection {
            id: Uuid::new_v4(),
            name: name.to_string(),
            description: None,
            host: host.to_string(),
            port: 22,
            protocol: ProtocolType::Ssh,
            username: None,
            group_id: None,
            tags: Vec::new(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            protocol_config: ProtocolConfig::Ssh(SshConfig::default()),
            sort_order: 0,
            last_connected: None,
            password_source: PasswordSource::None,
            kdbx_path_override: None,
            network_mode: crate::models::NetworkMode::default(),
            fallback_jump_host_ids: Vec::new(),
            domain: None,
            custom_properties: Vec::new(),
            pre_connect_task: None,
            post_disconnect_task: None,
            wol_config: None,
            local_variables: std::collections::HashMap::new(),
            log_config: None,
            key_sequence: None,
            automation: crate::models::AutomationConfig::default(),
            window_mode: crate::models::WindowMode::default(),
            remember_window_position: false,
            window_geometry: None,
            skip_port_check: false,
            is_pinned: false,
            pin_order: 0,
            icon: None,
            monitoring_config: None,
            activity_monitor_config: None,
            theme_override: None,
            session_recording_enabled: false,
            highlight_rules: Vec::new(),
            is_dynamic: false,
            retry_config: None,
            knock_sequence: None,
            spa_config: None,
            postpend: None,
            sync_origin_id: None,
        }
    }

    #[test]
    fn test_generate_lookup_key_with_name() {
        let conn = create_test_connection("My Server", "192.168.1.1");
        let key = CredentialResolver::generate_lookup_key(&conn);
        assert_eq!(key, "rustconn/My Server");
    }

    #[test]
    fn test_generate_lookup_key_with_empty_name() {
        let conn = create_test_connection("", "192.168.1.1");
        let key = CredentialResolver::generate_lookup_key(&conn);
        assert_eq!(key, "rustconn/192.168.1.1");
    }

    #[test]
    fn test_generate_lookup_key_with_whitespace_name() {
        let conn = create_test_connection("   ", "example.com");
        let key = CredentialResolver::generate_lookup_key(&conn);
        assert_eq!(key, "rustconn/example.com");
    }

    #[test]
    fn test_generate_lookup_key_contains_identifier() {
        let conn = create_test_connection("Production DB", "db.example.com");
        let key = CredentialResolver::generate_lookup_key(&conn);
        // Key should contain either name or host
        assert!(key.contains("Production DB") || key.contains("db.example.com"));
    }

    #[test]
    fn test_generate_hierarchical_lookup_key_no_group() {
        let conn = create_test_connection("My Server", "192.168.1.1");
        let key = CredentialResolver::generate_hierarchical_lookup_key(&conn, &[]);
        assert_eq!(key, "RustConn/My Server");
    }

    #[test]
    fn test_generate_hierarchical_lookup_key_with_group() {
        let group = ConnectionGroup::new("Production".to_string());
        let mut conn = create_test_connection("My Server", "192.168.1.1");
        conn.group_id = Some(group.id);

        let key = CredentialResolver::generate_hierarchical_lookup_key(&conn, &[group]);
        assert_eq!(key, "RustConn/Production/My Server");
    }

    #[test]
    fn test_generate_hierarchical_lookup_key_nested_groups() {
        let root = ConnectionGroup::new("Production".to_string());
        let child = ConnectionGroup::with_parent("Web".to_string(), root.id);
        let grandchild = ConnectionGroup::with_parent("Frontend".to_string(), child.id);

        let groups = vec![root, child, grandchild.clone()];

        let mut conn = create_test_connection("nginx-01", "192.168.1.10");
        conn.group_id = Some(grandchild.id);

        let key = CredentialResolver::generate_hierarchical_lookup_key(&conn, &groups);
        assert_eq!(key, "RustConn/Production/Web/Frontend/nginx-01");
    }

    #[test]
    fn test_select_storage_backend_keepass_enabled() {
        let settings = SecretSettings {
            preferred_backend: SecretBackendType::KeePassXc,
            kdbx_enabled: true,
            kdbx_path: Some(std::path::PathBuf::from("/path/to/db.kdbx")),
            ..Default::default()
        };
        let manager = Arc::new(SecretManager::empty());
        let resolver = CredentialResolver::new(manager, settings);

        assert_eq!(
            resolver.select_storage_backend(),
            SecretBackendType::KdbxFile
        );
    }

    #[test]
    fn test_select_storage_backend_keepass_disabled() {
        let settings = SecretSettings {
            kdbx_enabled: false,
            // Pin the system-keyring backend so the test is platform-independent
            // (the default is platform-aware: Keychain on macOS, libsecret elsewhere).
            preferred_backend: SecretBackendType::LibSecret,
            ..Default::default()
        };
        let manager = Arc::new(SecretManager::empty());
        let resolver = CredentialResolver::new(manager, settings);

        assert_eq!(
            resolver.select_storage_backend(),
            SecretBackendType::LibSecret
        );
    }

    #[test]
    fn test_select_storage_backend_keepass_no_path() {
        let settings = SecretSettings {
            kdbx_enabled: true,
            kdbx_path: None,
            // Pin the system-keyring backend so the test is platform-independent
            // (the default is platform-aware: Keychain on macOS, libsecret elsewhere).
            preferred_backend: SecretBackendType::LibSecret,
            ..Default::default()
        };
        let manager = Arc::new(SecretManager::empty());
        let resolver = CredentialResolver::new(manager, settings);

        assert_eq!(
            resolver.select_storage_backend(),
            SecretBackendType::LibSecret
        );
    }
}

impl std::fmt::Debug for CredentialResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialResolver")
            .field("preferred_backend", &self.settings.preferred_backend)
            .field("kdbx_enabled", &self.settings.kdbx_enabled)
            .field("kdbx_path_present", &self.settings.kdbx_path.is_some())
            .field("enable_fallback", &self.settings.enable_fallback)
            .field("secret_manager", &self.secret_manager)
            .finish()
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_secret() {
        // CredentialResolver wraps SecretManager + non-secret settings.
        // Sentinel that future fields don't accidentally render secrets.
        let manager = Arc::new(SecretManager::empty());
        let resolver = CredentialResolver::new(manager, SecretSettings::default());
        let rendered = format!("{resolver:?}");
        assert!(rendered.contains("CredentialResolver"));
        assert!(rendered.contains("preferred_backend"));
        assert!(
            !rendered.contains("password"),
            "Debug must not render password field: {rendered}"
        );
    }

    // --- issue #411: per-connection / per-group KDBX path override ---

    fn kdbx_test_connection() -> Connection {
        Connection::new_ssh("srv".to_string(), "10.0.0.1".to_string(), 22)
    }

    #[test]
    fn kdbx_override_none_when_nothing_set() {
        let conn = kdbx_test_connection();
        assert_eq!(resolve_kdbx_path_override(&conn, &[]), None);
    }

    #[test]
    fn kdbx_override_on_connection_wins() {
        let group = ConnectionGroup::new("Corp".to_string());
        let mut conn = kdbx_test_connection();
        conn.group_id = Some(group.id);
        conn.kdbx_path_override = Some(std::path::PathBuf::from("/vaults/personal.kdbx"));
        let mut group = group;
        group.kdbx_path_override = Some(std::path::PathBuf::from("/vaults/corp.kdbx"));

        assert_eq!(
            resolve_kdbx_path_override(&conn, &[group]),
            Some(std::path::PathBuf::from("/vaults/personal.kdbx")),
            "the connection's own override beats its group's"
        );
    }

    #[test]
    fn kdbx_override_inherited_from_group() {
        let mut group = ConnectionGroup::new("Corp".to_string());
        group.kdbx_path_override = Some(std::path::PathBuf::from("/vaults/corp.kdbx"));
        let mut conn = kdbx_test_connection();
        conn.group_id = Some(group.id);

        assert_eq!(
            resolve_kdbx_path_override(&conn, &[group]),
            Some(std::path::PathBuf::from("/vaults/corp.kdbx"))
        );
    }

    #[test]
    fn kdbx_override_walks_to_nearest_ancestor() {
        // root sets corp.kdbx; child sets nothing; grandchild-less connection
        // under `child` inherits root's override through the chain.
        let mut root = ConnectionGroup::new("Corp".to_string());
        root.kdbx_path_override = Some(std::path::PathBuf::from("/vaults/corp.kdbx"));
        let child = ConnectionGroup::with_parent("Web".to_string(), root.id);
        let mut conn = kdbx_test_connection();
        conn.group_id = Some(child.id);

        assert_eq!(
            resolve_kdbx_path_override(&conn, &[root, child]),
            Some(std::path::PathBuf::from("/vaults/corp.kdbx"))
        );
    }

    #[test]
    fn kdbx_override_nearest_group_wins_over_farther() {
        // child overrides root — the nearest ancestor with an override wins.
        let mut root = ConnectionGroup::new("Corp".to_string());
        root.kdbx_path_override = Some(std::path::PathBuf::from("/vaults/corp.kdbx"));
        let mut child = ConnectionGroup::with_parent("Team".to_string(), root.id);
        child.kdbx_path_override = Some(std::path::PathBuf::from("/vaults/team.kdbx"));
        let mut conn = kdbx_test_connection();
        conn.group_id = Some(child.id);

        assert_eq!(
            resolve_kdbx_path_override(&conn, &[root, child]),
            Some(std::path::PathBuf::from("/vaults/team.kdbx"))
        );
    }

    #[test]
    fn kdbx_override_survives_a_group_cycle() {
        // A malformed parent cycle must not loop forever; it just stops.
        let mut a = ConnectionGroup::new("A".to_string());
        let mut b = ConnectionGroup::with_parent("B".to_string(), a.id);
        // Make a point back at b → a cycle a→b→a with no override anywhere.
        a.parent_id = Some(b.id);
        let a_id = a.id;
        b.parent_id = Some(a_id);
        let mut conn = kdbx_test_connection();
        conn.group_id = Some(a_id);

        assert_eq!(resolve_kdbx_path_override(&conn, &[a, b]), None);
    }

    #[test]
    fn group_kdbx_override_uses_own_then_ancestor() {
        let mut root = ConnectionGroup::new("Corp".to_string());
        root.kdbx_path_override = Some(std::path::PathBuf::from("/vaults/corp.kdbx"));
        let child = ConnectionGroup::with_parent("Web".to_string(), root.id);
        let mut team = ConnectionGroup::with_parent("Team".to_string(), root.id);
        team.kdbx_path_override = Some(std::path::PathBuf::from("/vaults/team.kdbx"));
        let (child_id, team_id) = (child.id, team.id);
        let groups = [root, child, team];

        assert_eq!(
            resolve_group_kdbx_path_override(Some(child_id), &groups),
            Some(std::path::PathBuf::from("/vaults/corp.kdbx")),
            "a group without its own override inherits its parent's"
        );
        assert_eq!(
            resolve_group_kdbx_path_override(Some(team_id), &groups),
            Some(std::path::PathBuf::from("/vaults/team.kdbx")),
            "a group's own override wins"
        );
        assert_eq!(resolve_group_kdbx_path_override(None, &groups), None);
    }

    #[test]
    fn connection_and_its_group_resolve_to_the_same_database() {
        // The connection resolver and the group resolver must agree for a
        // connection with no override of its own: a group credential and an
        // Inherit connection under it read the same file.
        let mut root = ConnectionGroup::new("Corp".to_string());
        root.kdbx_path_override = Some(std::path::PathBuf::from("/vaults/corp.kdbx"));
        let child = ConnectionGroup::with_parent("Web".to_string(), root.id);
        let mut conn = kdbx_test_connection();
        conn.group_id = Some(child.id);
        let child_id = child.id;
        let groups = [root, child];

        assert_eq!(
            resolve_kdbx_path_override(&conn, &groups),
            resolve_group_kdbx_path_override(Some(child_id), &groups)
        );
    }
}
