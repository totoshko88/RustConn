//! macOS Keychain backend for credential storage
//!
//! This module implements credential storage using the macOS Security framework
//! (Keychain Services). It provides native integration with the system keychain
//! on macOS, replacing `libsecret`/`secret-tool` which are unavailable on macOS.
//!
//! Uses the `security-framework` crate for safe Rust bindings to Security.framework.

#[cfg(target_os = "macos")]
mod inner {
    use async_trait::async_trait;
    use secrecy::SecretString;
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };
    use zeroize::Zeroizing;

    use super::super::backend::SecretBackend;
    use crate::error::{SecretError, SecretResult};
    use crate::models::Credentials;

    /// Service name used for all Keychain entries
    const SERVICE_NAME: &str = "rustconn";

    /// macOS Keychain backend
    ///
    /// Stores credentials in the user's default Keychain using the
    /// Security.framework generic password API. Each credential field
    /// (username, password, key_passphrase, domain) is stored as a
    /// separate Keychain item with a composite account name.
    pub struct MacOsKeychainBackend;

    impl std::fmt::Debug for MacOsKeychainBackend {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MacOsKeychainBackend").finish()
        }
    }

    impl Default for MacOsKeychainBackend {
        fn default() -> Self {
            Self::new()
        }
    }

    impl MacOsKeychainBackend {
        /// Creates a new macOS Keychain backend
        #[must_use]
        pub const fn new() -> Self {
            Self
        }

        /// Builds the account name for a Keychain item
        ///
        /// Format: `{connection_id}/{field}`
        fn account_name(connection_id: &str, field: &str) -> String {
            format!("{connection_id}/{field}")
        }

        /// Stores a single value in the Keychain
        fn store_field(connection_id: &str, field: &str, value: &str) -> SecretResult<()> {
            let account = Self::account_name(connection_id, field);
            set_generic_password(SERVICE_NAME, &account, value.as_bytes()).map_err(|e| {
                SecretError::StoreFailed(format!("Keychain store failed for {field}: {e}"))
            })
        }

        /// Retrieves a single value from the Keychain
        fn retrieve_field(connection_id: &str, field: &str) -> SecretResult<Option<String>> {
            let account = Self::account_name(connection_id, field);
            match get_generic_password(SERVICE_NAME, &account) {
                Ok(bytes) => {
                    // `from_utf8` reuses the buffer on success; on success the
                    // decoded value is moved by the caller into a `SecretString`
                    // (zeroized on drop). On failure the raw bytes may still hold
                    // password material, so zeroize them before returning.
                    let value = match String::from_utf8(bytes) {
                        Ok(value) => value,
                        Err(e) => {
                            // Wipe the raw bytes (may hold password material)
                            // before returning — `Zeroizing` zeroizes on drop.
                            drop(Zeroizing::new(e.into_bytes()));
                            return Err(SecretError::LibSecret(format!(
                                "Keychain value is not valid UTF-8 for {field}"
                            )));
                        }
                    };
                    if value.is_empty() {
                        Ok(None)
                    } else {
                        Ok(Some(value))
                    }
                }
                Err(e) => {
                    // errSecItemNotFound (-25300) means the item doesn't exist
                    let err_str = e.to_string();
                    if err_str.contains("-25300") || err_str.contains("not found") {
                        Ok(None)
                    } else {
                        Err(SecretError::LibSecret(format!(
                            "Keychain lookup failed for {field}: {e}"
                        )))
                    }
                }
            }
        }

        /// Deletes a single value from the Keychain
        fn delete_field(connection_id: &str, field: &str) -> SecretResult<()> {
            let account = Self::account_name(connection_id, field);
            match delete_generic_password(SERVICE_NAME, &account) {
                Ok(()) => Ok(()),
                Err(e) => {
                    // Ignore "not found" errors during deletion
                    let err_str = e.to_string();
                    if err_str.contains("-25300") || err_str.contains("not found") {
                        Ok(())
                    } else {
                        Err(SecretError::DeleteFailed(format!(
                            "Keychain delete failed for {field}: {e}"
                        )))
                    }
                }
            }
        }
    }

    #[async_trait]
    impl SecretBackend for MacOsKeychainBackend {
        async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
            let conn_id = connection_id.to_string();
            let creds = credentials.clone();

            tokio::task::spawn_blocking(move || {
                // Store username if present
                if let Some(ref username) = creds.username {
                    Self::store_field(&conn_id, "username", username)?;
                }

                // Store password if present (expose via secrecy)
                if let Some(password) = creds.expose_password() {
                    Self::store_field(&conn_id, "password", password)?;
                }

                // Store key passphrase if present
                if let Some(passphrase) = creds.expose_key_passphrase() {
                    Self::store_field(&conn_id, "key_passphrase", passphrase)?;
                }

                // Store domain if present
                if let Some(ref domain) = creds.domain {
                    Self::store_field(&conn_id, "domain", domain)?;
                }

                Ok(())
            })
            .await
            .map_err(|e| SecretError::LibSecret(format!("Keychain task panicked: {e}")))?
        }

        async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>> {
            let conn_id = connection_id.to_string();

            tokio::task::spawn_blocking(move || {
                let username = Self::retrieve_field(&conn_id, "username")?;
                let password = Self::retrieve_field(&conn_id, "password")?;
                let key_passphrase = Self::retrieve_field(&conn_id, "key_passphrase")?;
                let domain = Self::retrieve_field(&conn_id, "domain")?;

                // If nothing was found, return None
                if username.is_none()
                    && password.is_none()
                    && key_passphrase.is_none()
                    && domain.is_none()
                {
                    return Ok(None);
                }

                Ok(Some(Credentials {
                    username,
                    password: password.map(SecretString::from),
                    key_passphrase: key_passphrase.map(SecretString::from),
                    domain,
                }))
            })
            .await
            .map_err(|e| SecretError::LibSecret(format!("Keychain task panicked: {e}")))?
        }

        async fn delete(&self, connection_id: &str) -> SecretResult<()> {
            let conn_id = connection_id.to_string();

            tokio::task::spawn_blocking(move || {
                // Delete all stored fields for this connection
                // Ignore individual errors (fields might not exist)
                let _ = Self::delete_field(&conn_id, "username");
                let _ = Self::delete_field(&conn_id, "password");
                let _ = Self::delete_field(&conn_id, "key_passphrase");
                let _ = Self::delete_field(&conn_id, "domain");
                Ok(())
            })
            .await
            .map_err(|e| SecretError::LibSecret(format!("Keychain task panicked: {e}")))?
        }

        async fn is_available(&self) -> bool {
            // macOS Keychain is always available on macOS
            true
        }

        fn backend_id(&self) -> &'static str {
            "macos_keychain"
        }

        fn display_name(&self) -> &'static str {
            "macOS Keychain"
        }
    }

    #[cfg(test)]
    mod debug_tests {
        use super::*;

        #[test]
        fn debug_does_not_leak_secret() {
            // MacOsKeychainBackend is a unit struct — no in-process secrets.
            // Sentinel test guards against future field additions.
            let backend = MacOsKeychainBackend::new();
            let rendered = format!("{backend:?}");
            assert_eq!(rendered, "MacOsKeychainBackend");
        }
    }
}

#[cfg(target_os = "macos")]
pub use inner::MacOsKeychainBackend;

/// Shared keyring operations for macOS Keychain
///
/// These functions mirror the `keyring.rs` API but use macOS Keychain
/// instead of `secret-tool`. Used by backends that need simple key-value
/// storage in the system keyring (e.g., storing Bitwarden session keys,
/// 1Password tokens, KeePassXC passwords).
#[cfg(target_os = "macos")]
pub mod keychain_ops {
    use crate::error::{SecretError, SecretResult};

    /// Application identifier used as the service name in Keychain entries
    const APP_SERVICE: &str = "rustconn";

    /// Checks whether macOS Keychain is available (always true on macOS)
    pub fn is_keychain_available() -> bool {
        true
    }

    /// Stores a value in the macOS Keychain
    ///
    /// # Errors
    /// Returns `SecretError::StoreFailed` if the Keychain operation fails.
    pub fn store(key: &str, value: &str) -> SecretResult<()> {
        use security_framework::passwords::set_generic_password;

        set_generic_password(APP_SERVICE, key, value.as_bytes())
            .map_err(|e| SecretError::StoreFailed(format!("Keychain store failed: {e}")))
    }

    /// Retrieves a value from the macOS Keychain
    ///
    /// Returns `Ok(None)` when the key does not exist.
    ///
    /// # Errors
    /// Returns `SecretError::LibSecret` if the Keychain operation fails.
    pub fn lookup(key: &str) -> SecretResult<Option<String>> {
        use security_framework::passwords::get_generic_password;

        match get_generic_password(APP_SERVICE, key) {
            Ok(bytes) => {
                let value = String::from_utf8(bytes).map_err(|e| {
                    SecretError::LibSecret(format!("Keychain value is not valid UTF-8: {e}"))
                })?;
                if value.is_empty() {
                    Ok(None)
                } else {
                    Ok(Some(value))
                }
            }
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("-25300") || err_str.contains("not found") {
                    Ok(None)
                } else {
                    Err(SecretError::LibSecret(format!(
                        "Keychain lookup failed: {e}"
                    )))
                }
            }
        }
    }

    /// Deletes a value from the macOS Keychain
    ///
    /// # Errors
    /// Returns `SecretError::DeleteFailed` if the Keychain operation fails.
    pub fn clear(key: &str) -> SecretResult<()> {
        use security_framework::passwords::delete_generic_password;

        match delete_generic_password(APP_SERVICE, key) {
            Ok(()) => Ok(()),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("-25300") || err_str.contains("not found") {
                    Ok(())
                } else {
                    Err(SecretError::DeleteFailed(format!(
                        "Keychain clear failed: {e}"
                    )))
                }
            }
        }
    }
}
