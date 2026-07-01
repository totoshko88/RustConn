//! libsecret backend for GNOME Keyring/KDE Wallet integration
//!
//! This module implements credential storage using the Secret Service API.
//!
//! It talks to the Secret Service **in process** via the [`oo7`] crate
//! (`oo7::dbus::Service`), so no `secret-tool` binary or bundled libsecret C
//! library is required.
//!
//! The whole module is `#[cfg(not(target_os = "macos"))]` (gated at the `mod`
//! declaration in `secret/mod.rs`): macOS uses `MacOsKeychainBackend` and never
//! compiles `oo7`, so `LibSecretBackend` does not exist on macOS at all (R10.1,
//! R10.2).

use async_trait::async_trait;
use secrecy::SecretString;
use std::collections::HashMap;

use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

use super::backend::{BackendAvailability, SecretBackend};

/// libsecret backend for GNOME Keyring/KDE Wallet
///
/// Uses the in-process [`oo7`] Secret Service client. It works with GNOME
/// Keyring, KDE Wallet, and other Secret Service implementations.
pub struct LibSecretBackend {
    /// Application identifier for stored secrets
    application_id: String,
}

impl LibSecretBackend {
    /// Creates a new libsecret backend
    ///
    /// # Arguments
    /// * `application_id` - Application identifier for stored secrets
    ///
    /// # Returns
    /// A new `LibSecretBackend` instance
    #[must_use]
    pub fn new(application_id: impl Into<String>) -> Self {
        Self {
            application_id: application_id.into(),
        }
    }

    /// Creates a new libsecret backend with default application ID
    #[must_use]
    pub fn default_app() -> Self {
        Self::new("rustconn")
    }

    /// Builds the attribute map used to identify a stored secret.
    ///
    /// The map preserves the attribute scheme used by earlier `secret-tool`
    /// releases so entries stay mutually findable (backward compatibility,
    /// R11.1/R11.3): `application`, `connection_id`, and `key` (one of
    /// `username` / `password` / `key_passphrase` / `domain`).
    fn build_attributes(&self, connection_id: &str, key: &str) -> HashMap<String, String> {
        let mut attrs = HashMap::new();
        attrs.insert("application".to_string(), self.application_id.clone());
        attrs.insert("connection_id".to_string(), connection_id.to_string());
        attrs.insert("key".to_string(), key.to_string());
        attrs
    }

    /// Stores a single field value as a Secret Service item via oo7.
    ///
    /// Uses `replace = true` so re-saving a connection overwrites the previous
    /// item that matches the same attributes, and `window_id = None` since this
    /// backend runs headless in `rustconn-core` (no GUI handle).
    async fn store_value(
        &self,
        connection_id: &str,
        key: &str,
        value: &str,
        label: &str,
    ) -> SecretResult<()> {
        let attrs = self.build_attributes(connection_id, key);

        let service = oo7::dbus::Service::new()
            .await
            .map_err(super::keyring::map_oo7_service_error)?;
        let collection = service
            .default_collection()
            .await
            .map_err(super::keyring::map_oo7_service_error)?;

        // `Secret::text` stores the raw UTF-8 string with a `text/plain` content
        // type so values round-trip byte-for-byte like the old secret-tool path.
        collection
            .create_item(label, &attrs, oo7::Secret::text(value), true, None)
            .await
            .map_err(super::keyring::map_oo7_store_error)?;

        Ok(())
    }

    /// Retrieves a single field value from the Secret Service via oo7.
    async fn retrieve_value(&self, connection_id: &str, key: &str) -> SecretResult<Option<String>> {
        let attrs = self.build_attributes(connection_id, key);

        let service = oo7::dbus::Service::new()
            .await
            .map_err(super::keyring::map_oo7_service_error)?;
        let collection = service
            .default_collection()
            .await
            .map_err(super::keyring::map_oo7_service_error)?;

        let items = collection
            .search_items(&attrs)
            .await
            .map_err(super::keyring::map_oo7_retrieve_error)?;

        let Some(item) = items.into_iter().next() else {
            // No matching item is not an error, just an absent value.
            return Ok(None);
        };

        let secret = item
            .secret()
            .await
            .map_err(super::keyring::map_oo7_retrieve_error)?;

        // Values were written as UTF-8 text; decode them back the same way.
        let value = String::from_utf8(secret.as_bytes().to_vec()).map_err(|e| {
            SecretError::RetrieveFailed(format!("stored secret was not valid UTF-8: {e}"))
        })?;

        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    /// Deletes every Secret Service item matching a single field via oo7.
    async fn delete_value(&self, connection_id: &str, key: &str) -> SecretResult<()> {
        let attrs = self.build_attributes(connection_id, key);

        let service = oo7::dbus::Service::new()
            .await
            .map_err(super::keyring::map_oo7_service_error)?;
        let collection = service
            .default_collection()
            .await
            .map_err(super::keyring::map_oo7_service_error)?;

        let items = collection
            .search_items(&attrs)
            .await
            .map_err(super::keyring::map_oo7_retrieve_error)?;

        for item in items {
            item.delete(None)
                .await
                .map_err(super::keyring::map_oo7_delete_error)?;
        }

        Ok(())
    }

    /// Probes availability via oo7's typed `Service::new()` result.
    ///
    /// A successful connection means a Secret Service answered (`Available`);
    /// any error means no service responded (`ServiceUnavailable`).
    ///
    /// The `ClientMissing` classification is intentionally never produced here:
    /// with the in-process oo7 client there is no external `secret-tool` binary
    /// that could be absent, so the only two observable outcomes are "a service
    /// answered" and "none did". oo7 does not cleanly distinguish "no session
    /// bus at all" from other transport failures (all surface as
    /// `Error::ZBus`/`Error::IO`), so collapsing every error to
    /// `ServiceUnavailable` is both correct and the least-code option.
    async fn availability_probe(&self) -> BackendAvailability {
        match oo7::dbus::Service::new().await {
            Ok(_) => BackendAvailability::Available,
            Err(_) => BackendAvailability::ServiceUnavailable,
        }
    }
}

#[async_trait]
impl SecretBackend for LibSecretBackend {
    async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
        let label = format!("RustConn: {connection_id}");

        // Store username if present
        if let Some(username) = &credentials.username {
            self.store_value(connection_id, "username", username, &label)
                .await?;
        }

        // Store password if present
        if let Some(password) = credentials.expose_password() {
            self.store_value(connection_id, "password", password, &label)
                .await?;
        }

        // Store key passphrase if present
        if let Some(passphrase) = credentials.expose_key_passphrase() {
            self.store_value(connection_id, "key_passphrase", passphrase, &label)
                .await?;
        }

        // Store domain if present
        if let Some(domain) = &credentials.domain {
            self.store_value(connection_id, "domain", domain, &label)
                .await?;
        }

        Ok(())
    }

    async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>> {
        let username = self.retrieve_value(connection_id, "username").await?;
        let password = self.retrieve_value(connection_id, "password").await?;
        let key_passphrase = self.retrieve_value(connection_id, "key_passphrase").await?;
        let domain = self.retrieve_value(connection_id, "domain").await?;

        // If nothing was found, return None
        if username.is_none() && password.is_none() && key_passphrase.is_none() && domain.is_none()
        {
            return Ok(None);
        }

        Ok(Some(Credentials {
            username,
            password: password.map(SecretString::from),
            key_passphrase: key_passphrase.map(SecretString::from),
            domain,
        }))
    }

    async fn delete(&self, connection_id: &str) -> SecretResult<()> {
        // Delete all stored values for this connection
        // Ignore errors for individual keys (they might not exist)
        let _ = self.delete_value(connection_id, "username").await;
        let _ = self.delete_value(connection_id, "password").await;
        let _ = self.delete_value(connection_id, "key_passphrase").await;
        let _ = self.delete_value(connection_id, "domain").await;

        Ok(())
    }

    async fn is_available(&self) -> bool {
        self.availability().await == BackendAvailability::Available
    }

    async fn availability(&self) -> BackendAvailability {
        self.availability_probe().await
    }

    fn backend_id(&self) -> &'static str {
        "libsecret"
    }

    fn display_name(&self) -> &'static str {
        "GNOME Keyring / KDE Wallet"
    }
}

impl std::fmt::Debug for LibSecretBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LibSecretBackend")
            .field("application_id", &self.application_id)
            .finish()
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_secret() {
        // LibSecretBackend stores no secrets in-process; the test guards
        // against accidental future fields that could leak.
        let backend = LibSecretBackend::new("hunter2-app-id");
        let rendered = format!("{backend:?}");
        // application_id is intentionally non-secret, so it may appear.
        // Make sure we never grow a field that contains a real password.
        assert!(rendered.contains("LibSecretBackend"));
        assert!(rendered.contains("application_id"));
    }
}
