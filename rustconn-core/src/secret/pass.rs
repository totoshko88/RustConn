//! Pass (password-store) backend for Unix password manager
//!
//! This module implements credential storage using the standard Unix password
//! manager "pass" (passwordstore.org). Pass uses GPG encryption and git-backed
//! storage, making it ideal for command-line users.

use async_trait::async_trait;
use secrecy::SecretString;
use std::process::Stdio;
use tokio::process::Command;

use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

use super::backend::SecretBackend;

/// Pass (password-store) backend for Unix password manager
///
/// This backend uses the `pass` command-line utility which stores passwords
/// in GPG-encrypted files organized in a directory hierarchy, typically
/// at ~/.password-store/. Each password is stored in a separate file.
pub struct PassBackend {
    /// Optional custom password store directory (defaults to ~/.password-store)
    store_dir: Option<String>,
}

impl PassBackend {
    /// Creates a new Pass backend
    ///
    /// # Arguments
    /// * `store_dir` - Optional custom password store directory
    ///
    /// # Returns
    /// A new `PassBackend` instance
    #[must_use]
    pub fn new(store_dir: Option<String>) -> Self {
        Self { store_dir }
    }

    /// Creates a new Pass backend with default settings
    #[must_use]
    pub fn default() -> Self {
        Self::new(None)
    }

    /// Builds the pass path for a connection's credential field
    ///
    /// Structure: rustconn/<connection_id>/<field>
    /// Where field is one of: username, password, key_passphrase, domain
    fn build_pass_path(&self, connection_id: &str, field: &str) -> String {
        format!("rustconn/{connection_id}/{field}")
    }

    /// Sets up the Command with optional PASSWORD_STORE_DIR
    fn setup_command(&self) -> Command {
        let mut cmd = Command::new("pass");
        if let Some(ref dir) = self.store_dir {
            cmd.env("PASSWORD_STORE_DIR", dir);
        }
        cmd
    }

    /// Stores a value using pass insert
    async fn store_value(&self, connection_id: &str, field: &str, value: &str) -> SecretResult<()> {
        let path = self.build_pass_path(connection_id, field);

        let mut child = self
            .setup_command()
            .arg("insert")
            .arg("--force") // Overwrite if exists
            .arg("--multiline")
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| SecretError::Pass(format!("Failed to spawn pass: {e}")))?;

        // Write the secret to stdin and close it
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin
                .write_all(value.as_bytes())
                .await
                .map_err(|e| SecretError::Pass(format!("Failed to write secret: {e}")))?;
            stdin
                .write_all(b"\n")
                .await
                .map_err(|e| SecretError::Pass(format!("Failed to write newline: {e}")))?;
            // Close stdin to signal EOF for --multiline
            drop(stdin);
        }

        let output = child
            .wait_with_output()
            .await
            .map_err(|e| SecretError::Pass(format!("Failed to wait for pass: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SecretError::StoreFailed(format!(
                "pass insert failed: {stderr}"
            )));
        }

        Ok(())
    }

    /// Retrieves a value using pass show
    async fn retrieve_value(
        &self,
        connection_id: &str,
        field: &str,
    ) -> SecretResult<Option<String>> {
        let path = self.build_pass_path(connection_id, field);

        let output = self
            .setup_command()
            .arg("show")
            .arg(&path)
            .output()
            .await
            .map_err(|e| SecretError::Pass(format!("Failed to run pass: {e}")))?;

        if !output.status.success() {
            // Not found is not an error, just return None
            return Ok(None);
        }

        let value = String::from_utf8_lossy(&output.stdout)
            .lines()
            .next() // Pass stores the password on the first line
            .unwrap_or("")
            .trim()
            .to_string();

        if value.is_empty() {
            Ok(None)
        } else {
            Ok(Some(value))
        }
    }

    /// Deletes a value using pass rm
    async fn delete_value(&self, connection_id: &str, field: &str) -> SecretResult<()> {
        let path = self.build_pass_path(connection_id, field);

        let output = self
            .setup_command()
            .arg("rm")
            .arg("--force") // Don't prompt for confirmation
            .arg(&path)
            .output()
            .await
            .map_err(|e| SecretError::Pass(format!("Failed to run pass: {e}")))?;

        if !output.status.success() {
            // It's okay if the file doesn't exist
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.contains("is not in the password store") {
                return Err(SecretError::DeleteFailed(format!(
                    "pass rm failed: {stderr}"
                )));
            }
        }

        Ok(())
    }

    /// Deletes the entire connection directory if empty
    async fn cleanup_directory(&self, connection_id: &str) -> SecretResult<()> {
        // Try to remove the connection directory (will only work if empty)
        let dir_path = format!("rustconn/{connection_id}");

        let _ = self
            .setup_command()
            .arg("rm")
            .arg("--force")
            .arg(&dir_path)
            .output()
            .await;

        // Also try to remove rustconn directory if empty
        let _ = self
            .setup_command()
            .arg("rm")
            .arg("--force")
            .arg("rustconn")
            .output()
            .await;

        Ok(())
    }
}

#[async_trait]
impl SecretBackend for PassBackend {
    async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
        // Store username if present
        if let Some(username) = &credentials.username {
            self.store_value(connection_id, "username", username)
                .await?;
        }

        // Store password if present
        if let Some(password) = credentials.expose_password() {
            self.store_value(connection_id, "password", password)
                .await?;
        }

        // Store key passphrase if present
        if let Some(passphrase) = credentials.expose_key_passphrase() {
            self.store_value(connection_id, "key_passphrase", passphrase)
                .await?;
        }

        // Store domain if present
        if let Some(domain) = &credentials.domain {
            self.store_value(connection_id, "domain", domain).await?;
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
        // Ignore errors for individual fields (they might not exist)
        let _ = self.delete_value(connection_id, "username").await;
        let _ = self.delete_value(connection_id, "password").await;
        let _ = self.delete_value(connection_id, "key_passphrase").await;
        let _ = self.delete_value(connection_id, "domain").await;

        // Try to clean up empty directories
        let _ = self.cleanup_directory(connection_id).await;

        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Check if pass is available
        let mut cmd = Command::new("pass");
        if let Some(ref dir) = self.store_dir {
            cmd.env("PASSWORD_STORE_DIR", dir);
        }

        cmd.arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    fn backend_id(&self) -> &'static str {
        "pass"
    }

    fn display_name(&self) -> &'static str {
        "Pass (Unix Password Manager)"
    }
}
