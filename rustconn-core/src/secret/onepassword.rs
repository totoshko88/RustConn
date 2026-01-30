//! 1Password CLI backend for password management
//!
//! This module implements credential storage using the 1Password CLI (`op`).
//! It supports both personal accounts and service accounts.
//!
//! # Authentication
//!
//! 1Password CLI v2 integrates with the 1Password desktop app for authentication.
//! When the desktop app integration is enabled, `op` commands automatically prompt
//! for biometric authentication (Touch ID, Windows Hello, etc.) or the account password.
//!
//! For automation/service accounts, use `OP_SERVICE_ACCOUNT_TOKEN` environment variable.

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::process::Stdio;
use tokio::process::Command;

use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

use super::backend::SecretBackend;

/// 1Password CLI backend
///
/// This backend uses the `op` command-line utility to interact with
/// 1Password vaults. Requires either:
/// - 1Password desktop app with CLI integration enabled (recommended)
/// - Service account token for automation
pub struct OnePasswordBackend {
    /// Service account token (for automation)
    service_account_token: Option<SecretString>,
    /// Vault name for RustConn entries
    vault_name: String,
    /// Account shorthand (for multi-account setups)
    account: Option<String>,
}

/// 1Password item structure for JSON parsing
#[derive(Debug, Deserialize)]
struct OnePasswordItem {
    id: String,
    title: String,
    #[serde(default)]
    fields: Vec<OnePasswordField>,
}

/// 1Password field structure
#[derive(Debug, Deserialize)]
struct OnePasswordField {
    id: String,
    #[serde(default)]
    value: Option<String>,
    #[serde(default)]
    label: Option<String>,
}

/// 1Password vault structure
#[derive(Debug, Deserialize)]
struct OnePasswordVault {
    id: String,
    name: String,
}

/// 1Password whoami response
#[derive(Debug, Deserialize)]
pub struct OnePasswordWhoami {
    pub url: Option<String>,
    pub email: Option<String>,
    pub user_uuid: Option<String>,
    pub account_uuid: Option<String>,
}

/// 1Password status information
#[derive(Debug, Clone)]
pub struct OnePasswordStatus {
    /// Whether CLI is installed
    pub installed: bool,
    /// CLI version
    pub version: Option<String>,
    /// Whether user is signed in
    pub signed_in: bool,
    /// Account email (if signed in)
    pub email: Option<String>,
    /// Account URL
    pub url: Option<String>,
    /// Status message for display
    pub status_message: String,
}

impl OnePasswordBackend {
    /// Creates a new 1Password backend
    #[must_use]
    pub fn new() -> Self {
        Self {
            service_account_token: None,
            vault_name: "RustConn".to_string(),
            account: None,
        }
    }

    /// Creates a new 1Password backend with a service account token
    #[must_use]
    pub fn with_service_account(token: SecretString) -> Self {
        Self {
            service_account_token: Some(token),
            vault_name: "RustConn".to_string(),
            account: None,
        }
    }

    /// Sets the vault name for storing RustConn entries
    #[must_use]
    pub fn with_vault_name(mut self, name: impl Into<String>) -> Self {
        self.vault_name = name.into();
        self
    }

    /// Sets the account shorthand for multi-account setups
    #[must_use]
    pub fn with_account(mut self, account: impl Into<String>) -> Self {
        self.account = Some(account.into());
        self
    }

    /// Sets the service account token
    pub fn set_service_account_token(&mut self, token: SecretString) {
        self.service_account_token = Some(token);
    }

    /// Clears the service account token
    pub fn clear_service_account(&mut self) {
        self.service_account_token = None;
    }

    /// Builds command with appropriate authentication
    fn build_command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("op");
        cmd.args(args);

        // Add service account token if available
        if let Some(ref token) = self.service_account_token {
            cmd.env("OP_SERVICE_ACCOUNT_TOKEN", token.expose_secret());
        }

        // Add account if specified
        if let Some(ref account) = self.account {
            cmd.arg("--account").arg(account);
        }

        // Always request JSON output for parsing
        cmd.arg("--format").arg("json");

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd
    }

    /// Runs an op command and returns stdout
    async fn run_command(&self, args: &[&str]) -> SecretResult<String> {
        let output = self
            .build_command(args)
            .output()
            .await
            .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run op: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SecretError::ConnectionFailed(format!(
                "op command failed: {stderr}"
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Gets the current account status
    ///
    /// # Errors
    /// Returns `SecretError` if the command fails
    pub async fn whoami(&self) -> SecretResult<OnePasswordWhoami> {
        let output = self.run_command(&["whoami"]).await?;
        serde_json::from_str(&output)
            .map_err(|e| SecretError::ConnectionFailed(format!("Failed to parse whoami: {e}")))
    }

    /// Checks if the user is signed in
    pub async fn is_signed_in(&self) -> bool {
        self.whoami().await.is_ok()
    }

    /// Gets or creates the RustConn vault
    async fn get_or_create_vault(&self) -> SecretResult<String> {
        // List vaults
        let output = self.run_command(&["vault", "list"]).await?;
        let vaults: Vec<OnePasswordVault> = serde_json::from_str(&output)
            .map_err(|e| SecretError::ConnectionFailed(format!("Failed to parse vaults: {e}")))?;

        // Find existing vault
        if let Some(vault) = vaults.iter().find(|v| v.name == self.vault_name) {
            return Ok(vault.id.clone());
        }

        // Create vault
        let output = self
            .run_command(&["vault", "create", &self.vault_name])
            .await?;
        let vault: OnePasswordVault = serde_json::from_str(&output)
            .map_err(|e| SecretError::StoreFailed(format!("Failed to create vault: {e}")))?;

        Ok(vault.id)
    }

    /// Generates a unique title for a connection entry
    fn entry_title(connection_id: &str) -> String {
        format!("RustConn: {connection_id}")
    }

    /// Finds an item by connection ID using tags
    async fn find_item(&self, connection_id: &str) -> SecretResult<Option<OnePasswordItem>> {
        let title = Self::entry_title(connection_id);

        // List items in vault with rustconn tag
        let output = self
            .run_command(&[
                "item",
                "list",
                "--vault",
                &self.vault_name,
                "--tags",
                "rustconn",
            ])
            .await;

        // If vault doesn't exist or is empty, return None
        let output = match output {
            Ok(o) => o,
            Err(_) => return Ok(None),
        };

        let items: Vec<OnePasswordItem> = serde_json::from_str(&output).unwrap_or_default();

        // Find exact match by title
        for item in items {
            if item.title == title {
                // Get full item details with fields
                let details = self
                    .run_command(&["item", "get", &item.id, "--vault", &self.vault_name])
                    .await?;
                let full_item: OnePasswordItem = serde_json::from_str(&details).map_err(|e| {
                    SecretError::RetrieveFailed(format!("Failed to parse item: {e}"))
                })?;
                return Ok(Some(full_item));
            }
        }

        Ok(None)
    }
}

impl Default for OnePasswordBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SecretBackend for OnePasswordBackend {
    async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
        // Check if signed in
        if !self.is_signed_in().await {
            return Err(SecretError::BackendUnavailable(
                "Not signed in to 1Password. Run 'op signin' or enable desktop app integration"
                    .to_string(),
            ));
        }

        // Get or create vault
        let vault_id = self.get_or_create_vault().await?;

        let title = Self::entry_title(connection_id);
        let username = credentials.username.clone().unwrap_or_default();
        let password = credentials
            .expose_password()
            .unwrap_or_default()
            .to_string();

        // Check if item already exists
        if let Some(existing) = self.find_item(connection_id).await? {
            // Update existing item using op item edit with assignment statements
            // Format: op item edit <id> field=value
            let username_assignment = format!("username={username}");
            let password_assignment = format!("password={password}");

            self.run_command(&[
                "item",
                "edit",
                &existing.id,
                "--vault",
                &self.vault_name,
                &username_assignment,
                &password_assignment,
            ])
            .await?;
        } else {
            // Create new item using op item create with assignment statements
            // Format: op item create --category login --title "..." --vault "..." --tags "..."
            let username_assignment = format!("username={username}");
            let password_assignment = format!("password={password}");

            self.run_command(&[
                "item",
                "create",
                "--category",
                "login",
                "--title",
                &title,
                "--vault",
                &vault_id,
                "--tags",
                "rustconn",
                &username_assignment,
                &password_assignment,
            ])
            .await?;
        }

        Ok(())
    }

    async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>> {
        // Check if signed in
        if !self.is_signed_in().await {
            return Err(SecretError::BackendUnavailable(
                "Not signed in to 1Password. Run 'op signin' or enable desktop app integration"
                    .to_string(),
            ));
        }

        let item = match self.find_item(connection_id).await? {
            Some(item) => item,
            None => return Ok(None),
        };

        let mut username = None;
        let mut password = None;

        for field in &item.fields {
            match field.id.as_str() {
                "username" => username = field.value.clone(),
                "password" => password = field.value.clone(),
                _ => {
                    // Also check by label for custom fields
                    if let Some(ref label) = field.label {
                        match label.to_lowercase().as_str() {
                            "username" => username = field.value.clone(),
                            "password" => password = field.value.clone(),
                            _ => {}
                        }
                    }
                }
            }
        }

        Ok(Some(Credentials {
            username,
            password: password.map(SecretString::from),
            key_passphrase: None,
            domain: None,
        }))
    }

    async fn delete(&self, connection_id: &str) -> SecretResult<()> {
        // Check if signed in
        if !self.is_signed_in().await {
            return Err(SecretError::BackendUnavailable(
                "Not signed in to 1Password. Run 'op signin' or enable desktop app integration"
                    .to_string(),
            ));
        }

        let item = match self.find_item(connection_id).await? {
            Some(item) => item,
            None => return Ok(()), // Already deleted
        };

        // Delete the item (moves to Recently Deleted in 1Password)
        self.run_command(&["item", "delete", &item.id, "--vault", &self.vault_name])
            .await?;

        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Check if op CLI is installed
        let installed = Command::new("op")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !installed {
            return false;
        }

        // Check if signed in (either via desktop app integration or service account)
        self.is_signed_in().await
    }

    fn backend_id(&self) -> &'static str {
        "onepassword"
    }

    fn display_name(&self) -> &'static str {
        "1Password"
    }
}

/// 1Password version information
#[derive(Debug, Clone)]
pub struct OnePasswordVersion {
    /// CLI version string
    pub version: String,
    /// Whether CLI is installed
    pub installed: bool,
}

/// Gets 1Password CLI version
pub async fn get_onepassword_version() -> Option<OnePasswordVersion> {
    let output = Command::new("op").arg("--version").output().await.ok()?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(OnePasswordVersion {
            version,
            installed: true,
        })
    } else {
        None
    }
}

/// Gets comprehensive 1Password status
pub async fn get_onepassword_status() -> OnePasswordStatus {
    // Check if installed
    let version_output = Command::new("op").arg("--version").output().await;

    let (installed, version) = match version_output {
        Ok(output) if output.status.success() => {
            let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (true, Some(ver))
        }
        _ => (false, None),
    };

    if !installed {
        return OnePasswordStatus {
            installed: false,
            version: None,
            signed_in: false,
            email: None,
            url: None,
            status_message: "Not installed".to_string(),
        };
    }

    // Check if signed in using whoami
    // This will trigger desktop app authentication if integration is enabled
    let whoami_output = Command::new("op")
        .args(["whoami", "--format", "json"])
        .output()
        .await;

    match whoami_output {
        Ok(output) if output.status.success() => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Ok(whoami) = serde_json::from_str::<OnePasswordWhoami>(&stdout) {
                OnePasswordStatus {
                    installed: true,
                    version,
                    signed_in: true,
                    email: whoami.email,
                    url: whoami.url,
                    status_message: "Signed in".to_string(),
                }
            } else {
                OnePasswordStatus {
                    installed: true,
                    version,
                    signed_in: true,
                    email: None,
                    url: None,
                    status_message: "Signed in".to_string(),
                }
            }
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let message = if stderr.contains("not signed in") || stderr.contains("sign in") {
                "Not signed in"
            } else if stderr.contains("session expired") {
                "Session expired"
            } else {
                "Not signed in"
            };
            OnePasswordStatus {
                installed: true,
                version,
                signed_in: false,
                email: None,
                url: None,
                status_message: message.to_string(),
            }
        }
        Err(_) => OnePasswordStatus {
            installed: true,
            version,
            signed_in: false,
            email: None,
            url: None,
            status_message: "Error checking status".to_string(),
        },
    }
}

/// Signs out from 1Password
///
/// # Errors
/// Returns `SecretError` if sign-out fails
pub async fn signout() -> SecretResult<()> {
    let output = Command::new("op")
        .arg("signout")
        .output()
        .await
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run op signout: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SecretError::ConnectionFailed(format!(
            "Sign-out failed: {stderr}"
        )));
    }

    Ok(())
}
