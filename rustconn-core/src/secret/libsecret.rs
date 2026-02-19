//! libsecret backend for GNOME Keyring/KDE Wallet integration
//!
//! This module implements credential storage using the Secret Service API
//! via the libsecret library. It provides fallback storage when `KeePassXC`
//! is unavailable.

use async_trait::async_trait;
use secrecy::SecretString;
use std::collections::HashMap;
use std::process::Stdio;
use tokio::process::Command;

use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

use super::backend::SecretBackend;

/// libsecret backend for GNOME Keyring/KDE Wallet
///
/// This backend uses the `secret-tool` command-line utility to interact
/// with the Secret Service API. It works with GNOME Keyring, KDE Wallet,
/// and other Secret Service implementations.
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

    /// Builds the attribute map for a connection
    fn build_attributes(&self, connection_id: &str) -> HashMap<String, String> {
        // Validate connection_id to prevent injection via secret-tool arguments
        debug_assert!(
            connection_id
                .chars()
                .all(|c| c.is_alphanumeric() || c == '-'),
            "connection_id should be a UUID: {connection_id}"
        );
        let mut attrs = HashMap::new();
        attrs.insert("application".to_string(), self.application_id.clone());
        attrs.insert("connection_id".to_string(), connection_id.to_string());
        attrs
    }

    /// Converts attributes to secret-tool command arguments
    fn attrs_to_args(attrs: &HashMap<String, String>) -> Vec<String> {
        attrs
            .iter()
            .flat_map(|(k, v)| vec![k.clone(), v.clone()])
            .collect()
    }

    /// Stores a value using secret-tool
    async fn store_value(
        &self,
        connection_id: &str,
        key: &str,
        value: &str,
        label: &str,
    ) -> SecretResult<()> {
        let mut attrs = self.build_attributes(connection_id);
        attrs.insert("key".to_string(), key.to_string());

        let mut args = vec![
            "store".to_string(),
            "--label".to_string(),
            label.to_string(),
        ];
        args.extend(Self::attrs_to_args(&attrs));

        let mut child = Command::new("secret-tool")
            .args(&args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SecretError::LibSecret(format!("Failed to spawn secret-tool: {e}")))?;

        // Write the secret to stdin
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(value.as_bytes())
                .await
                .map_err(|e| SecretError::LibSecret(format!("Failed to write secret: {e}")))?;
        }

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| SecretError::LibSecret(format!("Failed to wait for secret-tool: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SecretError::StoreFailed(format!(
                "secret-tool store failed: {stderr}"
            )));
        }

        Ok(())
    }

    /// Retrieves a value using secret-tool
    async fn retrieve_value(&self, connection_id: &str, key: &str) -> SecretResult<Option<String>> {
        let mut attrs = self.build_attributes(connection_id);
        attrs.insert("key".to_string(), key.to_string());

        let mut args = vec!["lookup".to_string()];
        args.extend(Self::attrs_to_args(&attrs));

        let output = Command::new("secret-tool")
            .args(&args)
            .output()
            .await
            .map_err(|e| SecretError::LibSecret(format!("Failed to run secret-tool: {e}")))?;

        if !output.status.success() {
            // Not found is not an error, just return None
            return Ok(None);
        }

        let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    /// Deletes a value using secret-tool
    async fn delete_value(&self, connection_id: &str, key: &str) -> SecretResult<()> {
        let mut attrs = self.build_attributes(connection_id);
        attrs.insert("key".to_string(), key.to_string());

        let mut args = vec!["clear".to_string()];
        args.extend(Self::attrs_to_args(&attrs));

        let output = Command::new("secret-tool")
            .args(&args)
            .output()
            .await
            .map_err(|e| SecretError::LibSecret(format!("Failed to run secret-tool: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SecretError::DeleteFailed(format!(
                "secret-tool clear failed: {stderr}"
            )));
        }

        Ok(())
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
        // Check if secret-tool is available
        Command::new("secret-tool")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn backend_id(&self) -> &'static str {
        "libsecret"
    }

    fn display_name(&self) -> &'static str {
        "GNOME Keyring / KDE Wallet"
    }
}
