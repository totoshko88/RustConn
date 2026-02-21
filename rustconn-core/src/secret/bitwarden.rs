//! Bitwarden CLI backend for password management
//!
//! This module implements credential storage using the Bitwarden CLI (`bw`).
//! It supports both cloud and self-hosted Bitwarden instances.
//!
//! # Authentication Methods
//!
//! The Bitwarden CLI supports several authentication methods:
//!
//! 1. **Email and Password** (interactive) - `bw login`
//! 2. **API Key** (automated) - Using `BW_CLIENTID` and `BW_CLIENTSECRET` environment variables
//! 3. **SSO** - `bw login --sso`
//!
//! After login, the vault must be unlocked with `bw unlock` to access credentials.
//! The unlock command returns a session key that must be passed to subsequent commands.
//!
//! # Session Management
//!
//! Session keys are valid until:
//! - `bw lock` is called
//! - `bw logout` is called
//! - A new terminal session is started (keys don't persist)
//!
//! # Usage Example
//!
//! ```ignore
//! use rustconn_core::secret::{BitwardenBackend, unlock_vault};
//! use secrecy::SecretString;
//!
//! // Unlock vault with master password
//! let password = SecretString::from("master_password");
//! let session_key = unlock_vault(&password).await?;
//!
//! // Create backend with session
//! let backend = BitwardenBackend::with_session(session_key);
//!
//! // Store credentials
//! backend.store("my-server", &credentials).await?;
//! ```

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use std::process::Stdio;
use std::sync::RwLock;
use tokio::process::Command;

use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

use super::backend::SecretBackend;

/// Thread-safe in-process storage for the Bitwarden session key.
///
/// Replaces `std::env::set_var("BW_SESSION", ...)` which is unsafe in
/// multi-threaded contexts (Rust 1.66+, hard error in edition 2024).
/// The session key is passed to `bw` commands via `--session` CLI arg
/// in [`BitwardenBackend::build_command`], so child processes do not
/// need the environment variable.
static BW_SESSION_STORE: RwLock<Option<String>> = RwLock::new(None);

/// Stores the Bitwarden session key in thread-safe in-process storage.
///
/// Call this instead of `std::env::set_var("BW_SESSION", ...)`.
/// The key is used by [`get_session_key`] and passed to `bw` commands
/// via `--session` argument.
pub fn set_session_key(key: &str) {
    if let Ok(mut guard) = BW_SESSION_STORE.write() {
        *guard = Some(key.to_string());
    }
}

/// Retrieves the Bitwarden session key from thread-safe in-process storage.
///
/// Returns `None` if no session key has been stored or if the lock is poisoned.
#[must_use]
pub fn get_session_key() -> Option<String> {
    BW_SESSION_STORE.read().ok().and_then(|guard| guard.clone())
}

/// Clears the stored Bitwarden session key.
pub fn clear_session_key() {
    if let Ok(mut guard) = BW_SESSION_STORE.write() {
        *guard = None;
    }
}

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
    id: Option<String>,
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

        // Find existing folder (skip folders with null id)
        if let Some(folder) = folders
            .iter()
            .find(|f| f.name == self.folder_name && f.id.is_some())
        {
            return Ok(folder.id.clone());
        }

        // Create folder
        let folder_json = serde_json::json!({ "name": self.folder_name });
        let encoded = base64_encode(folder_json.to_string().as_bytes());

        let output = self.run_command(&["create", "folder", &encoded]).await?;
        let folder: BitwardenFolder = serde_json::from_str(&output)
            .map_err(|e| SecretError::StoreFailed(format!("Failed to create folder: {e}")))?;

        Ok(folder.id)
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

            let edit_result = self
                .run_command(&["edit", "item", &existing.id, &encoded])
                .await;

            if let Err(ref e) = edit_result {
                let err_msg = format!("{e}");
                if err_msg.contains("out of date") || err_msg.contains("out-of-date") {
                    tracing::info!("Bitwarden cipher out of date, syncing and retrying...");
                    let _ = self.run_command(&["sync"]).await;

                    // Re-fetch the item to get updated revision
                    if let Some(refreshed) = self.find_item(connection_id).await? {
                        let refreshed_encoded = base64_encode(json.as_bytes());
                        self.run_command(&["edit", "item", &refreshed.id, &refreshed_encoded])
                            .await?;
                    } else {
                        // Item was deleted remotely — create instead
                        self.run_command(&["create", "item", &encoded]).await?;
                    }
                } else {
                    edit_result?;
                }
            }
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

        // Sync vault to get latest data from server
        // This ensures we have the most recent credentials
        let _ = self.sync().await; // Ignore sync errors, proceed with local data

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
/// Uses `std::process::Command` (blocking) via `spawn_blocking` because the
/// new Rust-based Bitwarden CLI (v2026+) has compatibility issues with
/// `tokio::process::Command` for the `unlock` subcommand.
///
/// Tries three strategies in order:
/// 1. `--passwordenv BW_PASSWORD --raw` (returns session key directly)
/// 2. `--passwordenv BW_PASSWORD` without `--raw` (parses session key from verbose output)
/// 3. Stdin password pipe without `--raw` (for older CLI versions)
///
/// # Errors
/// Returns `SecretError` if the unlock command fails or password is incorrect
pub async fn unlock_vault(password: &SecretString) -> SecretResult<SecretString> {
    let pw = password.expose_secret().to_string();

    tokio::task::spawn_blocking(move || unlock_vault_sync(&pw))
        .await
        .map_err(|e| SecretError::ConnectionFailed(format!("Unlock task panicked: {e}")))?
}

/// Synchronous implementation of vault unlock.
///
/// Uses `std::process::Command` which is compatible with all Bitwarden CLI versions
/// including the new Rust-based CLI (v2026+).
fn unlock_vault_sync(password: &str) -> SecretResult<SecretString> {
    tracing::debug!(
        password_len = password.len(),
        "Bitwarden: unlock_vault_sync called"
    );

    // Strategy 1: --passwordenv with --raw (returns session key directly)
    let output = std::process::Command::new("bw")
        .args(["unlock", "--passwordenv", "BW_PASSWORD", "--raw"])
        .env("BW_PASSWORD", password)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw unlock: {e}")))?;

    if output.status.success() {
        let session_key = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !session_key.is_empty() {
            tracing::debug!("Bitwarden: unlocked with --passwordenv --raw");
            return Ok(SecretString::from(session_key));
        }
    }

    let stderr_raw = String::from_utf8_lossy(&output.stderr).to_string();
    tracing::debug!("Bitwarden: --raw unlock failed, trying verbose: {stderr_raw}");

    // Strategy 2: --passwordenv without --raw (parse session key from verbose output)
    let output = std::process::Command::new("bw")
        .args(["unlock", "--passwordenv", "BW_PASSWORD"])
        .env("BW_PASSWORD", password)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw unlock: {e}")))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(session_key) = extract_session_key(&stdout) {
            tracing::debug!("Bitwarden: unlocked with --passwordenv (verbose)");
            return Ok(SecretString::from(session_key));
        }
        tracing::debug!("Bitwarden: verbose unlock succeeded but no session key in output");
    }

    let stderr_verbose = String::from_utf8_lossy(&output.stderr);
    tracing::debug!("Bitwarden: verbose unlock failed: {stderr_verbose}");

    // Strategy 3: stdin pipe without --raw (for maximum compatibility)
    let mut child = std::process::Command::new("bw")
        .arg("unlock")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw unlock: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        let _ = stdin.write_all(password.as_bytes());
        let _ = stdin.write_all(b"\n");
        drop(stdin);
    }

    let output = child
        .wait_with_output()
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to wait for bw: {e}")))?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(session_key) = extract_session_key(&stdout) {
            tracing::debug!("Bitwarden: unlocked with stdin pipe (verbose)");
            return Ok(SecretString::from(session_key));
        }
    }

    let stderr = String::from_utf8_lossy(&output.stderr);

    // Check if this is the "provided key" error — indicates corrupted vault data
    if stderr_raw.contains("provided key is not the expected type")
        || stderr.contains("provided key is not the expected type")
    {
        return Err(SecretError::ConnectionFailed(
            "Vault data may be corrupted (key type mismatch). \
             Try: bw logout → bw login → bw unlock"
                .to_string(),
        ));
    }

    Err(SecretError::ConnectionFailed(format!(
        "Failed to unlock vault: {stderr}"
    )))
}

/// Extracts session key from verbose `bw unlock` output.
///
/// Parses output lines looking for `BW_SESSION="<key>"` or `BW_SESSION=<key>`.
fn extract_session_key(output: &str) -> Option<String> {
    for line in output.lines() {
        if line.contains("BW_SESSION=") {
            // Try quoted format: export BW_SESSION="<key>"
            if let Some(start) = line.find('"') {
                if let Some(end) = line.rfind('"') {
                    if end > start {
                        return Some(line[start + 1..end].to_string());
                    }
                }
            }
            // Try unquoted format: BW_SESSION=<key>
            if let Some(pos) = line.find("BW_SESSION=") {
                let value_start = pos + "BW_SESSION=".len();
                let value = line[value_start..].trim().trim_matches('"');
                if !value.is_empty() {
                    return Some(value.to_string());
                }
            }
        }
    }
    None
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

/// Logs in to Bitwarden using API key credentials
///
/// This is the recommended method for automated workflows and CI/CD pipelines.
/// Uses `BW_CLIENTID` and `BW_CLIENTSECRET` environment variables as documented.
///
/// After login, you must still call `unlock_vault()` to access vault data.
///
/// # Arguments
/// * `client_id` - Personal API key client_id
/// * `client_secret` - Personal API key client_secret
///
/// # Errors
/// Returns `SecretError` if login fails
pub async fn login_with_api_key(
    client_id: &SecretString,
    client_secret: &SecretString,
) -> SecretResult<()> {
    let output = Command::new("bw")
        .args(["login", "--apikey"])
        .env("BW_CLIENTID", client_id.expose_secret())
        .env("BW_CLIENTSECRET", client_secret.expose_secret())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw login: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SecretError::ConnectionFailed(format!(
            "Failed to login with API key: {stderr}"
        )));
    }

    Ok(())
}

/// Logs out from Bitwarden
///
/// # Errors
/// Returns `SecretError` if logout fails
pub async fn logout() -> SecretResult<()> {
    let output = Command::new("bw")
        .arg("logout")
        .output()
        .await
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw logout: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        // Ignore "not logged in" error
        if !stderr.contains("not logged in") {
            return Err(SecretError::ConnectionFailed(format!(
                "Failed to logout: {stderr}"
            )));
        }
    }

    Ok(())
}

/// Configures Bitwarden CLI to use a self-hosted server
///
/// # Arguments
/// * `server_url` - URL of the self-hosted Bitwarden server
///
/// # Errors
/// Returns `SecretError` if configuration fails
pub async fn configure_server(server_url: &str) -> SecretResult<()> {
    let output = Command::new("bw")
        .args(["config", "server", server_url])
        .output()
        .await
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw config: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SecretError::ConnectionFailed(format!(
            "Failed to configure server: {stderr}"
        )));
    }

    Ok(())
}

// ============================================================================
// Keyring storage for Bitwarden credentials
// ============================================================================

const KEY_BW_MASTER: &str = "bitwarden-master";
const KEY_BW_CLIENT_ID: &str = "bitwarden-client-id";
const KEY_BW_CLIENT_SECRET: &str = "bitwarden-client-secret";

/// Stores Bitwarden master password in system keyring (libsecret)
///
/// # Errors
/// Returns `SecretError` if storage fails
pub async fn store_master_password_in_keyring(password: &SecretString) -> SecretResult<()> {
    super::keyring::store(
        KEY_BW_MASTER,
        password.expose_secret(),
        "Bitwarden Master Password",
    )
    .await
}

/// Retrieves Bitwarden master password from system keyring
///
/// # Errors
/// Returns `SecretError` if retrieval fails
pub async fn get_master_password_from_keyring() -> SecretResult<Option<SecretString>> {
    super::keyring::lookup(KEY_BW_MASTER)
        .await
        .map(|opt| opt.map(SecretString::from))
}

/// Deletes Bitwarden master password from system keyring
///
/// # Errors
/// Returns `SecretError` if deletion fails
pub async fn delete_master_password_from_keyring() -> SecretResult<()> {
    super::keyring::clear(KEY_BW_MASTER).await
}

/// Stores Bitwarden API credentials in system keyring
///
/// # Errors
/// Returns `SecretError` if storage fails
pub async fn store_api_credentials_in_keyring(
    client_id: &SecretString,
    client_secret: &SecretString,
) -> SecretResult<()> {
    super::keyring::store(
        KEY_BW_CLIENT_ID,
        client_id.expose_secret(),
        "Bitwarden API Client ID",
    )
    .await?;
    super::keyring::store(
        KEY_BW_CLIENT_SECRET,
        client_secret.expose_secret(),
        "Bitwarden API Client Secret",
    )
    .await?;
    Ok(())
}

/// Retrieves Bitwarden API credentials from system keyring
///
/// # Returns
/// Tuple of (`client_id`, `client_secret`) if both exist
///
/// # Errors
/// Returns `SecretError` if retrieval fails
pub async fn get_api_credentials_from_keyring() -> SecretResult<Option<(SecretString, SecretString)>>
{
    let client_id = super::keyring::lookup(KEY_BW_CLIENT_ID).await?;
    let client_secret = super::keyring::lookup(KEY_BW_CLIENT_SECRET).await?;

    match (client_id, client_secret) {
        (Some(id), Some(secret)) => Ok(Some((SecretString::from(id), SecretString::from(secret)))),
        _ => Ok(None),
    }
}

/// Deletes Bitwarden API credentials from system keyring
///
/// # Errors
/// Returns `SecretError` if deletion fails
pub async fn delete_api_credentials_from_keyring() -> SecretResult<()> {
    let _ = super::keyring::clear(KEY_BW_CLIENT_ID).await;
    let _ = super::keyring::clear(KEY_BW_CLIENT_SECRET).await;
    Ok(())
}

/// Attempts to automatically unlock the Bitwarden vault using saved credentials.
///
/// Tries the following sources in order:
/// 1. `BW_SESSION` environment variable (already unlocked)
/// 2. Master password from system keyring (libsecret)
/// 3. Master password from encrypted settings
///
/// Returns a `BitwardenBackend` with session key set, or an error
/// with a user-friendly message.
///
/// # Errors
/// Returns `SecretError` if vault cannot be unlocked from any source
/// Attempts API key login when vault is unauthenticated.
///
/// Decrypts stored API credentials if needed and calls `bw login --apikey`.
///
/// # Errors
/// Returns `SecretError::BackendUnavailable` if login fails or credentials are missing.
async fn try_api_key_login(settings: &crate::config::SecretSettings) -> SecretResult<()> {
    if !settings.bitwarden_use_api_key {
        tracing::warn!("Bitwarden: vault unauthenticated, API key login not enabled");
        return Err(SecretError::BackendUnavailable(
            "Bitwarden vault is not logged in. \
             Run 'bw login' in terminal or enable API key in Settings → Secrets."
                .to_string(),
        ));
    }

    let mut settings_clone = settings.clone();
    if (settings_clone.bitwarden_client_id.is_none()
        || settings_clone.bitwarden_client_secret.is_none())
        && (settings_clone.bitwarden_client_id_encrypted.is_some()
            || settings_clone.bitwarden_client_secret_encrypted.is_some())
    {
        settings_clone.decrypt_bitwarden_api_credentials();
    }

    if let (Some(ref client_id), Some(ref client_secret)) = (
        settings_clone.bitwarden_client_id,
        settings_clone.bitwarden_client_secret,
    ) {
        tracing::debug!("Bitwarden: attempting API key login");
        login_with_api_key(client_id, client_secret)
            .await
            .map_err(|e| {
                tracing::warn!("Bitwarden: API key login failed: {e}");
                SecretError::BackendUnavailable(
                    "Bitwarden vault is not logged in. \
                 API key login failed. Check credentials in Settings → Secrets."
                        .to_string(),
                )
            })?;
        tracing::info!("Bitwarden: API key login successful");
        Ok(())
    } else {
        tracing::warn!("Bitwarden: vault unauthenticated but no API credentials configured");
        Err(SecretError::BackendUnavailable(
            "Bitwarden vault is not logged in. \
             Run 'bw login' in terminal or configure API key in Settings → Secrets."
                .to_string(),
        ))
    }
}

/// Attempts to fix "key type mismatch" by re-logging in and retrying unlock.
///
/// This handles corrupted vault data (e.g. after CLI upgrade from Node.js to Rust).
/// Performs: `bw logout` → `bw login --apikey` → `bw unlock`.
///
/// # Errors
/// Returns `SecretError` if re-login or unlock fails, or if API key is not configured.
async fn try_relogin_and_unlock(
    settings: &crate::config::SecretSettings,
    password: &SecretString,
) -> SecretResult<()> {
    if !settings.bitwarden_use_api_key {
        tracing::debug!("Bitwarden: key type mismatch but no API key configured, cannot re-login");
        return Err(SecretError::BackendUnavailable(
            "Vault data corrupted. Run 'bw logout && bw login' manually.".to_string(),
        ));
    }

    tracing::info!("Bitwarden: key type mismatch detected, attempting re-login");

    // Logout (ignore errors — might already be logged out)
    let _ = logout().await;

    // Re-login with API key
    try_api_key_login(settings).await.map_err(|e| {
        tracing::warn!("Bitwarden: re-login failed: {e}");
        SecretError::BackendUnavailable(
            "Vault data corrupted and re-login failed. \
             Run 'bw logout && bw login' manually."
                .to_string(),
        )
    })?;

    // Retry unlock
    let session_key = unlock_vault(password).await.map_err(|e| {
        tracing::warn!("Bitwarden: unlock after re-login failed: {e}");
        SecretError::BackendUnavailable(
            "Re-login succeeded but unlock still failed. Check master password.".to_string(),
        )
    })?;

    set_session_key(session_key.expose_secret());
    tracing::info!("Bitwarden: re-login + unlock successful");
    Ok(())
}

/// Attempts to auto-unlock the Bitwarden vault using saved credentials.
///
/// Tries the following strategies in order:
/// 1. Existing session key from in-process storage
/// 2. Vault already unlocked (no session needed)
/// 3. API key login if vault is unauthenticated
/// 4. Master password from system keyring
/// 5. Master password from encrypted settings
///
/// # Errors
/// Returns `SecretError::BackendUnavailable` if all strategies fail.
pub async fn auto_unlock(
    settings: &crate::config::SecretSettings,
) -> SecretResult<BitwardenBackend> {
    // 1. Check in-process session store, then fall back to BW_SESSION env var
    //    (supports externally unlocked vaults, e.g. `export BW_SESSION=...` in shell)
    let stored_session =
        get_session_key().or_else(|| std::env::var("BW_SESSION").ok().filter(|s| !s.is_empty()));
    if let Some(session) = stored_session {
        let backend = BitwardenBackend::with_session(SecretString::from(session));
        if backend.is_unlocked().await {
            tracing::debug!("Bitwarden: using existing session key");
            return Ok(backend);
        }
        tracing::debug!("Bitwarden: stored session key present but vault not unlocked");
    }

    // 2. Check if vault is already unlocked (no session needed)
    let bare = BitwardenBackend::new();
    let status = bare.get_status().await;
    tracing::debug!(
        vault_status = ?status.as_ref().map(|s| &s.status),
        "Bitwarden: vault status before unlock"
    );

    if status
        .as_ref()
        .map(|s| s.status == "unlocked")
        .unwrap_or(false)
    {
        tracing::debug!("Bitwarden: vault already unlocked");
        return Ok(bare);
    }

    // 2b. Check if vault is unauthenticated — need login before unlock
    let needs_login = status
        .as_ref()
        .map(|s| s.status == "unauthenticated")
        .unwrap_or(false);

    if needs_login {
        try_api_key_login(settings).await?;
    }

    // 3. Try master password from system keyring
    if settings.bitwarden_save_to_keyring {
        if let Ok(Some(password)) = get_master_password_from_keyring().await {
            tracing::debug!("Bitwarden: attempting unlock with keyring password");
            match unlock_vault(&password).await {
                Ok(session_key) => {
                    set_session_key(session_key.expose_secret());
                    return Ok(BitwardenBackend::with_session(session_key));
                }
                Err(e) => {
                    let is_key_type_error = e.to_string().contains("key type mismatch");
                    tracing::warn!("Bitwarden: keyring password unlock failed: {e}");
                    // If key type mismatch, try re-login before giving up
                    if is_key_type_error
                        && try_relogin_and_unlock(settings, &password).await.is_ok()
                    {
                        return Ok(BitwardenBackend::with_session(SecretString::from(
                            get_session_key().unwrap_or_default(),
                        )));
                    }
                }
            }
        }
    }

    // 4. Try master password from encrypted settings
    if settings.bitwarden_password.is_some() || settings.bitwarden_password_encrypted.is_some() {
        let mut settings_clone = settings.clone();
        if settings_clone.bitwarden_password.is_none()
            && settings_clone.bitwarden_password_encrypted.is_some()
        {
            settings_clone.decrypt_bitwarden_password();
        }
        if let Some(ref password) = settings_clone.bitwarden_password {
            tracing::debug!("Bitwarden: attempting unlock with saved password");
            match unlock_vault(password).await {
                Ok(session_key) => {
                    set_session_key(session_key.expose_secret());
                    return Ok(BitwardenBackend::with_session(session_key));
                }
                Err(e) => {
                    let is_key_type_error = e.to_string().contains("key type mismatch");
                    tracing::warn!("Bitwarden: saved password unlock failed: {e}");
                    // If key type mismatch, try re-login before giving up
                    if is_key_type_error && try_relogin_and_unlock(settings, password).await.is_ok()
                    {
                        return Ok(BitwardenBackend::with_session(SecretString::from(
                            get_session_key().unwrap_or_default(),
                        )));
                    }
                }
            }
        }
    }

    Err(SecretError::BackendUnavailable(
        "Bitwarden vault is locked. Unlock it in Settings → Secrets \
         or run 'bw unlock' in terminal."
            .to_string(),
    ))
}

/// Base64 encode helper (standard base64 with padding)
fn base64_encode(data: &[u8]) -> String {
    data_encoding::BASE64.encode(data)
}
