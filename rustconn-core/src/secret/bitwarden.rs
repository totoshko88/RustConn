//! Bitwarden CLI backend for password management
//!
//! This module implements credential storage using the Bitwarden CLI (`bw`).
//! It supports both cloud and self-hosted Bitwarden instances.

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

use super::backend::SecretBackend;

/// Bitwarden CLI backend
///
/// This backend uses the `bw` command-line utility to interact with
/// Bitwarden vaults. Requires the user to be logged in and have an
/// active session.
pub struct BitwardenBackend {
    /// Session key for authenticated operations
    session_key: Option<SecretString>,
    /// Custom server URL (for self-hosted instances)
    server_url: Option<String>,
    /// Organization ID (optional, for org vaults)
    organization_id: Option<String>,
    /// Folder name for RustConn entries
    folder_name: String,
}

/// Bitwarden item structure for JSON parsing
#[derive(Debug, Deserialize)]
struct BitwardenItem {
    id: String,
    name: String,
    login: Option<BitwardenLogin>,
    notes: Option<String>,
}

/// Bitwarden login structure
#[derive(Debug, Deserialize)]
struct BitwardenLogin {
    username: Option<String>,
    password: Option<String>,
}

/// Bitwarden item template for creation
#[derive(Debug, Serialize)]
struct BitwardenItemTemplate {
    #[serde(rename = "type")]
    item_type: u8,
    name: String,
    notes: Option<String>,
    login: BitwardenLoginTemplate,
    #[serde(rename = "folderId", skip_serializing_if = "Option::is_none")]
    folder_id: Option<String>,
}

/// Bitwarden login template for creation
#[derive(Debug, Serialize)]
struct BitwardenLoginTemplate {
    username: Option<String>,
    password: Option<String>,
    uris: Vec<BitwardenUri>,
}

/// Bitwarden URI structure
#[derive(Debug, Serialize)]
struct BitwardenUri {
    uri: String,
    #[serde(rename = "match")]
    match_type: Option<u8>,
}

/// Bitwarden folder structure
#[derive(Debug, Deserialize)]
struct BitwardenFolder {
    id: String,
    name: String,
}

/// Bitwarden status response
#[derive(Debug, Deserialize)]
pub struct BitwardenStatus {
    status: String,
    #[serde(rename = "userEmail")]
    #[allow(dead_code)]
    user_email: Option<String>,
}

impl BitwardenBackend {
    /// Creates a new Bitwarden backend
    #[must_use]
    pub fn new() -> Self {
        Self {
            session_key: None,
            server_url: None,
            organization_id: None,
            folder_name: "RustConn".to_string(),
        }
    }

    /// Creates a new Bitwarden backend with a session key
    #[must_use]
    pub fn with_session(session_key: SecretString) -> Self {
        Self {
            session_key: Some(session_key),
            server_url: None,
            organization_id: None,
            folder_name: "RustConn".to_string(),
        }
    }

    /// Sets the server URL for self-hosted instances
    #[must_use]
    pub fn with_server_url(mut self, url: impl Into<String>) -> Self {
        self.server_url = Some(url.into());
        self
    }

    /// Sets the organization ID for org vault access
    #[must_use]
    pub fn with_organization(mut self, org_id: impl Into<String>) -> Self {
        self.organization_id = Some(org_id.into());
        self
    }

    /// Sets the folder name for storing RustConn entries
    #[must_use]
    pub fn with_folder_name(mut self, name: impl Into<String>) -> Self {
        self.folder_name = name.into();
        self
    }

    /// Sets the session key
    pub fn set_session_key(&mut self, key: SecretString) {
        self.session_key = Some(key);
    }

    /// Clears the session key
    pub fn clear_session(&mut self) {
        self.session_key = None;
    }

    /// Builds command with session key if available
    fn build_command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("bw");
        cmd.args(args);

        if let Some(ref session) = self.session_key {
            cmd.arg("--session").arg(session.expose_secret());
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd
    }

    /// Runs a bw command and returns stdout
    async fn run_command(&self, args: &[&str]) -> SecretResult<String> {
        let output = self
            .build_command(args)
            .output()
            .await
            .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SecretError::ConnectionFailed(format!(
                "bw command failed: {stderr}"
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Gets the vault status
    ///
    /// # Errors
    /// Returns `SecretError` if the command fails or output cannot be parsed
    pub async fn get_status(&self) -> SecretResult<BitwardenStatus> {
        let output = self.run_command(&["status"]).await?;
        serde_json::from_str(&output)
            .map_err(|e| SecretError::ConnectionFailed(format!("Failed to parse status: {e}")))
    }

    /// Checks if the vault is unlocked
    pub async fn is_unlocked(&self) -> bool {
        self.get_status()
            .await
            .map(|s| s.status == "unlocked")
            .unwrap_or(false)
    }

    /// Syncs the vault with the server
    ///
    /// # Errors
    /// Returns `SecretError` if the sync command fails
    pub async fn sync(&self) -> SecretResult<()> {
        self.run_command(&["sync"]).await?;
        Ok(())
    }

    /// Gets or creates the RustConn folder
    async fn get_or_create_folder(&self) -> SecretResult<Option<String>> {
        // List folders
        let output = self.run_command(&["list", "folders"]).await?;
        let folders: Vec<BitwardenFolder> = serde_json::from_str(&output)
            .map_err(|e| SecretError::ConnectionFailed(format!("Failed to parse folders: {e}")))?;

        // Find existing folder
        if let Some(folder) = folders.iter().find(|f| f.name == self.folder_name) {
            return Ok(Some(folder.id.clone()));
        }

        // Create folder
        let folder_json = serde_json::json!({ "name": self.folder_name });
        let encoded = base64_encode(folder_json.to_string().as_bytes());

        let output = self.run_command(&["create", "folder", &encoded]).await?;
        let folder: BitwardenFolder = serde_json::from_str(&output)
            .map_err(|e| SecretError::StoreFailed(format!("Failed to create folder: {e}")))?;

        Ok(Some(folder.id))
    }

    /// Generates a unique name for a connection entry
    fn entry_name(connection_id: &str) -> String {
        format!("RustConn: {connection_id}")
    }

    /// Generates a URI for a connection (used for searching)
    fn connection_uri(connection_id: &str) -> String {
        format!("rustconn://{connection_id}")
    }

    /// Finds an item by connection ID
    async fn find_item(&self, connection_id: &str) -> SecretResult<Option<BitwardenItem>> {
        let search_term = Self::entry_name(connection_id);
        let output = self
            .run_command(&["list", "items", "--search", &search_term])
            .await?;

        let items: Vec<BitwardenItem> = serde_json::from_str(&output)
            .map_err(|e| SecretError::RetrieveFailed(format!("Failed to parse items: {e}")))?;

        // Find exact match by name
        Ok(items.into_iter().find(|item| item.name == search_term))
    }
}

impl Default for BitwardenBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SecretBackend for BitwardenBackend {
    async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
        // Check if vault is unlocked
        if !self.is_unlocked().await {
            return Err(SecretError::BackendUnavailable(
                "Bitwarden vault is locked. Please unlock with 'bw unlock'".to_string(),
            ));
        }

        // Get or create folder
        let folder_id = self.get_or_create_folder().await?;

        // Check if item already exists
        if let Some(existing) = self.find_item(connection_id).await? {
            // Update existing item
            let item_template = BitwardenItemTemplate {
                item_type: 1, // Login
                name: Self::entry_name(connection_id),
                notes: credentials.domain.clone(),
                login: BitwardenLoginTemplate {
                    username: credentials.username.clone(),
                    password: credentials.expose_password().map(String::from),
                    uris: vec![BitwardenUri {
                        uri: Self::connection_uri(connection_id),
                        match_type: Some(3), // Exact match
                    }],
                },
                folder_id,
            };

            let json = serde_json::to_string(&item_template)
                .map_err(|e| SecretError::StoreFailed(format!("Failed to serialize: {e}")))?;
            let encoded = base64_encode(json.as_bytes());

            self.run_command(&["edit", "item", &existing.id, &encoded])
                .await?;
        } else {
            // Create new item
            let item_template = BitwardenItemTemplate {
                item_type: 1, // Login
                name: Self::entry_name(connection_id),
                notes: credentials.domain.clone(),
                login: BitwardenLoginTemplate {
                    username: credentials.username.clone(),
                    password: credentials.expose_password().map(String::from),
                    uris: vec![BitwardenUri {
                        uri: Self::connection_uri(connection_id),
                        match_type: Some(3), // Exact match
                    }],
                },
                folder_id,
            };

            let json = serde_json::to_string(&item_template)
                .map_err(|e| SecretError::StoreFailed(format!("Failed to serialize: {e}")))?;
            let encoded = base64_encode(json.as_bytes());

            self.run_command(&["create", "item", &encoded]).await?;
        }

        Ok(())
    }

    async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>> {
        // Check if vault is unlocked
        if !self.is_unlocked().await {
            return Err(SecretError::BackendUnavailable(
                "Bitwarden vault is locked. Please unlock with 'bw unlock'".to_string(),
            ));
        }

        let item = match self.find_item(connection_id).await? {
            Some(item) => item,
            None => return Ok(None),
        };

        let login = match item.login {
            Some(login) => login,
            None => return Ok(None),
        };

        Ok(Some(Credentials {
            username: login.username,
            password: login.password.map(SecretString::from),
            key_passphrase: None,
            domain: item.notes,
        }))
    }

    async fn delete(&self, connection_id: &str) -> SecretResult<()> {
        // Check if vault is unlocked
        if !self.is_unlocked().await {
            return Err(SecretError::BackendUnavailable(
                "Bitwarden vault is locked. Please unlock with 'bw unlock'".to_string(),
            ));
        }

        let item = match self.find_item(connection_id).await? {
            Some(item) => item,
            None => return Ok(()), // Already deleted
        };

        self.run_command(&["delete", "item", &item.id]).await?;
        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Check if bw CLI is installed
        let installed = Command::new("bw")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !installed {
            return false;
        }

        // Check if logged in (status != "unauthenticated")
        self.get_status()
            .await
            .map(|s| s.status != "unauthenticated")
            .unwrap_or(false)
    }

    fn backend_id(&self) -> &'static str {
        "bitwarden"
    }

    fn display_name(&self) -> &'static str {
        "Bitwarden"
    }
}

/// Bitwarden version information
#[derive(Debug, Clone)]
pub struct BitwardenVersion {
    /// CLI version string
    pub version: String,
    /// Whether CLI is installed
    pub installed: bool,
}

/// Gets Bitwarden CLI version
pub async fn get_bitwarden_version() -> Option<BitwardenVersion> {
    let output = Command::new("bw").arg("--version").output().await.ok()?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(BitwardenVersion {
            version,
            installed: true,
        })
    } else {
        None
    }
}

/// Unlocks Bitwarden vault with master password
///
/// # Errors
/// Returns `SecretError` if the unlock command fails or password is incorrect
pub async fn unlock_vault(password: &SecretString) -> SecretResult<SecretString> {
    let mut child = Command::new("bw")
        .args(["unlock", "--raw"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to spawn bw: {e}")))?;

    // Write password to stdin
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(password.expose_secret().as_bytes())
            .await
            .map_err(|e| SecretError::ConnectionFailed(format!("Failed to write password: {e}")))?;
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to wait for bw: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SecretError::ConnectionFailed(format!(
            "Failed to unlock vault: {stderr}"
        )));
    }

    let session_key = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Ok(SecretString::from(session_key))
}

/// Locks the Bitwarden vault
///
/// # Errors
/// Returns `SecretError` if the lock command fails
pub async fn lock_vault() -> SecretResult<()> {
    let output = Command::new("bw")
        .arg("lock")
        .output()
        .await
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw lock: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SecretError::ConnectionFailed(format!(
            "Failed to lock vault: {stderr}"
        )));
    }

    Ok(())
}

/// Base64 encode helper (standard base64 alphabet)
fn base64_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    let mut result = String::new();
    let mut i = 0;

    while i < data.len() {
        let b0 = data[i];
        let b1 = data.get(i + 1).copied().unwrap_or(0);
        let b2 = data.get(i + 2).copied().unwrap_or(0);

        result.push(ALPHABET[(b0 >> 2) as usize] as char);
        result.push(ALPHABET[(((b0 & 0x03) << 4) | (b1 >> 4)) as usize] as char);

        if i + 1 < data.len() {
            result.push(ALPHABET[(((b1 & 0x0f) << 2) | (b2 >> 6)) as usize] as char);
        } else {
            result.push('=');
        }

        if i + 2 < data.len() {
            result.push(ALPHABET[(b2 & 0x3f) as usize] as char);
        } else {
            result.push('=');
        }

        i += 3;
    }

    result
}
