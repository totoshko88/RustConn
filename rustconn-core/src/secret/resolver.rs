//! Credential resolution chain for connections
//!
//! This module provides the `CredentialResolver` which resolves credentials
//! for connections based on their configuration and available backends.

use std::sync::Arc;

use tracing::{debug, warn};

use crate::config::{SecretBackendType, SecretSettings};
use crate::error::SecretResult;
use crate::models::{Connection, ConnectionGroup, Credentials, PasswordSource};

use super::hierarchy::KeePassHierarchy;
use super::manager::SecretManager;
use super::verification::{CredentialStatus, CredentialVerificationManager, VerifiedCredentials};

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

    /// Generates a lookup key for libsecret/keyring storage
    ///
    /// The key format is: `{name} ({protocol})` matching the format used
    /// by `resolve_from_keyring` for consistent store/retrieve behavior.
    ///
    /// # Arguments
    /// * `connection` - The connection to generate a key for
    ///
    /// # Returns
    /// A string key suitable for keyring entry lookup
    #[must_use]
    pub fn generate_keyring_key(connection: &Connection) -> String {
        let protocol = connection.protocol_config.protocol_type();
        let name =
            crate::import::sanitize_imported_value(&connection.name.trim().replace('/', "-"));
        format!("{} ({})", name, protocol.as_str().to_lowercase())
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
            PasswordSource::Prompt | PasswordSource::Inherit => {
                // Caller handles these cases
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
            SecretBackendType::LibSecret => self.resolve_from_keyring(connection).await,
            SecretBackendType::Bitwarden => self.resolve_from_bitwarden(connection).await,
            SecretBackendType::OnePassword => self.resolve_from_onepassword(connection).await,
            SecretBackendType::Passbolt => self.resolve_from_passbolt(connection).await,
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
        let result = self.secret_manager.retrieve(&lookup_key).await?;

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

    /// Resolves credentials from Bitwarden vault
    async fn resolve_from_bitwarden(
        &self,
        connection: &Connection,
    ) -> SecretResult<Option<Credentials>> {
        // Primary: use the same key format as store_unified: "rustconn/{name}"
        let lookup_key = Self::generate_lookup_key(connection);
        if let Some(creds) = self.secret_manager.retrieve(&lookup_key).await? {
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
        // Primary: use the same key format as store_unified: "rustconn/{name}"
        let lookup_key = Self::generate_lookup_key(connection);
        if let Some(creds) = self.secret_manager.retrieve(&lookup_key).await? {
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
        // Primary: use the same key format as store_unified: "rustconn/{name}"
        let lookup_key = Self::generate_lookup_key(connection);
        if let Some(creds) = self.secret_manager.retrieve(&lookup_key).await? {
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
            if let Some(creds) = self.secret_manager.retrieve(&lookup_key).await? {
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
    /// # Requirements Coverage
    ///
    /// - Requirement 3.1: Store to preferred backend based on settings
    ///
    /// # Returns
    /// The selected backend type
    #[must_use]
    pub const fn select_storage_backend(&self) -> SecretBackendType {
        match self.settings.preferred_backend {
            SecretBackendType::Bitwarden => SecretBackendType::Bitwarden,
            SecretBackendType::OnePassword => SecretBackendType::OnePassword,
            SecretBackendType::Passbolt => SecretBackendType::Passbolt,
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
        }
    }

    /// Checks if credentials need migration from Keyring to `KeePass`
    ///
    /// Returns true if:
    /// - `KeePass` integration is enabled
    /// - Credentials exist in Keyring for the connection
    /// - Credentials do NOT exist in `KeePass` for the connection
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 3.3: Detect credentials in Keyring but not `KeePass`
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

    /// Checks if credentials need migration using hierarchical paths
    ///
    /// # Arguments
    /// * `connection` - The connection to check
    /// * `groups` - All available connection groups
    ///
    /// # Returns
    /// `true` if migration is needed, `false` otherwise
    ///
    /// # Errors
    /// Returns `SecretError` if backend operations fail
    pub async fn needs_keepass_migration_with_hierarchy(
        &self,
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> SecretResult<bool> {
        // Migration only makes sense if KeePass is enabled
        if !self.settings.kdbx_enabled || self.settings.kdbx_path.is_none() {
            return Ok(false);
        }

        // Check if credentials exist in Keyring
        let keyring_creds = self.resolve_from_keyring(connection).await?;
        if keyring_creds.is_none() {
            return Ok(false);
        }

        // Check if credentials exist in KeePass (hierarchical path)
        let lookup_key = Self::generate_hierarchical_lookup_key(connection, groups);
        let keepass_creds = self.secret_manager.retrieve(&lookup_key).await?;

        if keepass_creds.is_some() {
            return Ok(false);
        }

        // Also check legacy flat key
        let legacy_key = Self::generate_lookup_key(connection);
        let legacy_creds = self.secret_manager.retrieve(&legacy_key).await?;

        // Need migration if in Keyring but not in KeePass (either path)
        Ok(legacy_creds.is_none())
    }

    /// Migrates credentials from Keyring to `KeePass`
    ///
    /// Copies credentials from Keyring to `KeePass` and optionally deletes
    /// them from Keyring after successful copy.
    ///
    /// # Requirements Coverage
    ///
    /// - Requirement 3.4: Copy credentials from Keyring to `KeePass`
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

    /// Migrates credentials from Keyring to `KeePass` using hierarchical paths
    ///
    /// # Arguments
    /// * `connection` - The connection to migrate credentials for
    /// * `groups` - All available connection groups
    /// * `delete_from_keyring` - Whether to delete from Keyring after migration
    ///
    /// # Returns
    /// `true` if migration was successful, `false` if no credentials to migrate
    ///
    /// # Errors
    /// Returns `SecretError` if backend operations fail
    pub async fn migrate_to_keepass_with_hierarchy(
        &self,
        connection: &Connection,
        groups: &[ConnectionGroup],
        delete_from_keyring: bool,
    ) -> SecretResult<bool> {
        // Get credentials from Keyring
        let keyring_creds = self.resolve_from_keyring(connection).await?;
        let Some(creds) = keyring_creds else {
            return Ok(false);
        };

        // Store in KeePass with hierarchical path
        let lookup_key = Self::generate_hierarchical_lookup_key(connection, groups);
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
    /// # Requirements Coverage
    ///
    /// - Requirement 3.5: Verify libsecret service is accessible
    /// - Requirement 3.6: Display warning when Keyring unavailable
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
    /// # Requirements Coverage
    ///
    /// - Requirement 3.1: Store to preferred backend based on settings
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
            SecretBackendType::LibSecret => {
                let lookup_key = Self::generate_keyring_key(connection);
                self.secret_manager.store(&lookup_key, credentials).await
            }
            SecretBackendType::Bitwarden
            | SecretBackendType::OnePassword
            | SecretBackendType::Passbolt => {
                // For Bitwarden/1Password/Passbolt, use connection name as identifier
                let lookup_key = Self::generate_lookup_key(connection);
                self.secret_manager.store(&lookup_key, credentials).await
            }
        }
    }

    /// Stores credentials using the unified storage backend with hierarchical paths
    ///
    /// # Arguments
    /// * `connection` - The connection to store credentials for
    /// * `credentials` - The credentials to store
    /// * `groups` - All available connection groups
    ///
    /// # Errors
    /// Returns `SecretError` if storage fails
    pub async fn store_unified_with_hierarchy(
        &self,
        connection: &Connection,
        credentials: &Credentials,
        groups: &[ConnectionGroup],
    ) -> SecretResult<()> {
        let backend = self.select_storage_backend();

        match backend {
            SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
                let lookup_key = Self::generate_hierarchical_lookup_key(connection, groups);
                self.secret_manager.store(&lookup_key, credentials).await
            }
            SecretBackendType::LibSecret => {
                let lookup_key = Self::generate_keyring_key(connection);
                self.secret_manager.store(&lookup_key, credentials).await
            }
            SecretBackendType::Bitwarden
            | SecretBackendType::OnePassword
            | SecretBackendType::Passbolt => {
                // For Bitwarden/1Password/Passbolt, use hierarchical path as entry name
                let lookup_key = Self::generate_hierarchical_lookup_key(connection, groups);
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
    /// # Requirements Coverage
    ///
    /// - Requirement 2.1: Skip dialog for verified credentials
    /// - Requirement 2.2: Show dialog for missing credentials
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

    /// Resolves credentials with verification status using hierarchical paths
    ///
    /// This is the hierarchical version of `resolve_verified()` that uses the
    /// connection's group structure to determine the `KeePass` entry path.
    ///
    /// # Arguments
    /// * `connection` - The connection to resolve credentials for
    /// * `groups` - All available connection groups for hierarchy resolution
    /// * `verification_manager` - Manager tracking verification status
    ///
    /// # Returns
    /// `VerifiedCredentials` with status information
    ///
    /// # Errors
    /// Returns `SecretError` if backend operations fail
    pub async fn resolve_verified_with_hierarchy(
        &self,
        connection: &Connection,
        groups: &[ConnectionGroup],
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

        // Try to resolve credentials from storage using hierarchical path
        let credentials = self.resolve_with_hierarchy(connection, groups).await?;

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

    /// Stores credentials for a connection with hierarchical path support.
    ///
    /// This method stores credentials using the connection's group hierarchy
    /// to determine the `KeePass` entry path.
    ///
    /// # Arguments
    /// * `connection` - The connection to store credentials for
    /// * `credentials` - The credentials to store
    /// * `groups` - All available connection groups for hierarchy resolution
    ///
    /// # Errors
    /// Returns `SecretError` if storage fails
    pub async fn store_with_hierarchy(
        &self,
        connection: &Connection,
        credentials: &Credentials,
        groups: &[ConnectionGroup],
    ) -> SecretResult<()> {
        let lookup_key = Self::generate_hierarchical_lookup_key(connection, groups);
        self.secret_manager.store(&lookup_key, credentials).await
    }

    /// Retrieves credentials for a connection using hierarchical path.
    ///
    /// # Arguments
    /// * `connection` - The connection to retrieve credentials for
    /// * `groups` - All available connection groups for hierarchy resolution
    ///
    /// # Returns
    /// `Some(Credentials)` if found, `None` if not found
    ///
    /// # Errors
    /// Returns `SecretError` if retrieval fails
    pub async fn retrieve_with_hierarchy(
        &self,
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> SecretResult<Option<Credentials>> {
        let lookup_key = Self::generate_hierarchical_lookup_key(connection, groups);
        self.secret_manager.retrieve(&lookup_key).await
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
            SecretBackendType::LibSecret => self.resolve_from_keyring(connection).await,
            SecretBackendType::Bitwarden => self.resolve_from_bitwarden(connection).await,
            SecretBackendType::OnePassword => self.resolve_from_onepassword(connection).await,
            SecretBackendType::Passbolt => self.resolve_from_passbolt(connection).await,
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
                return self.resolve_from_keyring(connection).await;
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
            self.resolve_from_keyring(connection).await
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

        // Fall back to keyring
        self.resolve_from_keyring(connection).await
    }

    /// Deletes credentials for a connection using hierarchical path.
    ///
    /// # Arguments
    /// * `connection` - The connection to delete credentials for
    /// * `groups` - All available connection groups for hierarchy resolution
    ///
    /// # Errors
    /// Returns `SecretError` if deletion fails
    pub async fn delete_with_hierarchy(
        &self,
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> SecretResult<()> {
        let lookup_key = Self::generate_hierarchical_lookup_key(connection, groups);
        self.secret_manager.delete(&lookup_key).await
    }

    /// Moves a credential entry when a connection's group changes.
    ///
    /// This retrieves the credential from the old path, stores it at the new path,
    /// and deletes the old entry.
    ///
    /// # Arguments
    /// * `connection` - The connection with updated `group_id`
    /// * `old_group_id` - The previous group ID (None if was at root)
    /// * `groups` - All available connection groups
    ///
    /// # Errors
    /// Returns `SecretError` if the move operation fails
    pub async fn move_credential_on_group_change(
        &self,
        connection: &Connection,
        old_group_id: Option<uuid::Uuid>,
        groups: &[ConnectionGroup],
    ) -> SecretResult<()> {
        // Build old path
        let mut old_connection = connection.clone();
        old_connection.group_id = old_group_id;
        let old_key = Self::generate_hierarchical_lookup_key(&old_connection, groups);

        // Retrieve from old location
        let credentials = self.secret_manager.retrieve(&old_key).await?;

        if let Some(creds) = credentials {
            // Store at new location
            let new_key = Self::generate_hierarchical_lookup_key(connection, groups);
            self.secret_manager.store(&new_key, &creds).await?;

            // Delete from old location
            let _ = self.secret_manager.delete(&old_key).await;
        }

        Ok(())
    }

    /// Renames a credential entry when a connection is renamed.
    ///
    /// This retrieves the credential using the old name, stores it with the new name,
    /// and deletes the old entry. Handles all backend types appropriately.
    ///
    /// # Arguments
    /// * `connection` - The connection with the NEW name already set
    /// * `old_name` - The previous connection name
    /// * `groups` - All available connection groups
    ///
    /// # Errors
    /// Returns `SecretError` if the rename operation fails
    pub async fn rename_credential(
        &self,
        connection: &Connection,
        old_name: &str,
        groups: &[ConnectionGroup],
    ) -> SecretResult<()> {
        // Build old connection with old name for key generation
        let mut old_connection = connection.clone();
        old_connection.name = old_name.to_string();

        let protocol = connection.protocol_config.protocol_type();
        let protocol_str = protocol.as_str().to_lowercase();

        match connection.password_source {
            PasswordSource::Vault => {
                // Vault uses the configured backend — rename based on
                // which backend is active
                let backend = self.select_storage_backend();
                match backend {
                    SecretBackendType::KdbxFile | SecretBackendType::KeePassXc => {
                        // KeePass uses hierarchical paths
                        let old_key =
                            Self::generate_hierarchical_lookup_key(&old_connection, groups);
                        let new_key = Self::generate_hierarchical_lookup_key(connection, groups);
                        if old_key != new_key {
                            if let Some(creds) = self.secret_manager.retrieve(&old_key).await? {
                                self.secret_manager.store(&new_key, &creds).await?;
                                let _ = self.secret_manager.delete(&old_key).await;
                            }
                        }
                    }
                    SecretBackendType::LibSecret | SecretBackendType::Bitwarden => {
                        // Uses "{name} ({protocol})" format
                        let old_key = format!("{} ({})", old_name.replace('/', "-"), protocol_str);
                        let new_key =
                            format!("{} ({})", connection.name.replace('/', "-"), protocol_str);
                        if old_key != new_key {
                            if let Some(creds) = self.secret_manager.retrieve(&old_key).await? {
                                self.secret_manager.store(&new_key, &creds).await?;
                                let _ = self.secret_manager.delete(&old_key).await;
                            }
                        }
                    }
                    SecretBackendType::OnePassword | SecretBackendType::Passbolt => {
                        // Use connection ID — no rename needed
                    }
                }
            }
            PasswordSource::Variable(_)
            | PasswordSource::Prompt
            | PasswordSource::Inherit
            | PasswordSource::None => {
                // No credentials stored in these modes
            }
        }

        Ok(())
    }

    /// Resolves credentials by inheriting from parent groups
    async fn resolve_inherited_credentials(
        &self,
        connection: &Connection,
        groups: &[ConnectionGroup],
    ) -> SecretResult<Option<Credentials>> {
        let mut current_group_id = connection.group_id;

        // Traverse up the hierarchy
        while let Some(group_id) = current_group_id {
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
                                let group_id_str = group.id.to_string();
                                if let Some(creds) =
                                    self.secret_manager.retrieve(&group_id_str).await?
                                {
                                    return Ok(Some(self.merge_group_credentials(creds, group)));
                                }
                            }
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
    use super::*;
    use crate::models::{ConnectionGroup, ProtocolConfig, ProtocolType, SshConfig};
    use uuid::Uuid;

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
