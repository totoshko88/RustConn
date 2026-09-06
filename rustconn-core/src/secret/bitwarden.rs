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

use std::process::Stdio;
use std::sync::RwLock;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use tokio::process::Command;

use super::backend::SecretBackend;
use super::serde_helpers::serde_error_kind;
use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

/// Thread-safe in-process storage for the Bitwarden session key.
///
/// Replaces `std::env::set_var("BW_SESSION", ...)` which is unsafe in
/// multi-threaded contexts (Rust 1.66+, hard error in edition 2024).
/// The session key is passed to `bw` commands via `--session` CLI arg
/// in [`BitwardenBackend::build_command`], so child processes do not
/// need the environment variable.
static BW_SESSION_STORE: RwLock<Option<SecretString>> = RwLock::new(None);

/// Thread-safe in-process storage for the resolved `bw` CLI command path.
///
/// In Flatpak sandboxes, `bw` is not on the default PATH. The UI layer
/// detects the correct binary path (e.g.
/// `~/.var/app/io.github.totoshko88.RustConn/cli/bitwarden/bw`) and
/// stores it here so that all backend functions use the same resolved path.
///
/// Falls back to `"bw"` when no custom path has been stored.
static BW_CMD_STORE: RwLock<Option<String>> = RwLock::new(None);

/// Timestamp of the last successful vault unlock/status verification.
///
/// Used to skip redundant `bw status` calls when the session key is
/// already present and was verified recently. The threshold is
/// [`UNLOCK_VERIFY_TTL_SECS`].
static BW_LAST_VERIFIED: RwLock<Option<std::time::Instant>> = RwLock::new(None);

/// How long (in seconds) a successful `bw status` result is trusted
/// before re-checking. Keeps reconnect fast while still detecting
/// vault locks within a reasonable window.
const UNLOCK_VERIFY_TTL_SECS: u64 = 120;

/// Ceiling on a single `bw` invocation.
///
/// One number for "one child process", so the guarantee is the same wherever a
/// `bw` is spawned. [`BitwardenBackend::run_command`] had it as a literal 30 and
/// was the only place that honoured it; [`unlock_vault_sync`] spawns up to three
/// children and bounded none of them, which is what let an unlock sit
/// indefinitely on a stalled network sync while the GUI's status row waited for
/// an answer that was never coming. A `gio::spawn_blocking` worker is cheap but
/// not free, and there are only a handful of them.
///
/// Thirty seconds is generous for a call that normally costs a few — measured at
/// about 4.5 s for a real unlock in issue
/// [#312](https://github.com/totoshko88/RustConn/issues/312) — and the point is
/// to bound a hang, not to race a slow vault server.
pub(crate) const BW_INVOCATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Records that the vault was just verified as unlocked.
fn mark_verified() {
    if let Ok(mut guard) = BW_LAST_VERIFIED.write() {
        *guard = Some(std::time::Instant::now());
    }
}

/// Returns `true` if the vault was verified as unlocked within the TTL.
fn is_recently_verified() -> bool {
    BW_LAST_VERIFIED
        .read()
        .ok()
        .and_then(|guard| *guard)
        .is_some_and(|t| t.elapsed().as_secs() < UNLOCK_VERIFY_TTL_SECS)
}

/// Clears the verification timestamp (e.g. on lock/logout).
fn clear_verified() {
    if let Ok(mut guard) = BW_LAST_VERIFIED.write() {
        *guard = None;
    }
}

/// Stores the Bitwarden session key in thread-safe in-process storage.
///
/// Call this instead of `std::env::set_var("BW_SESSION", ...)`.
/// The key is used by [`get_session_key`] and passed to `bw` commands
/// via `--session` argument.
pub fn set_session_key(key: SecretString) {
    if let Ok(mut guard) = BW_SESSION_STORE.write() {
        *guard = Some(key);
    }
}

/// Retrieves the Bitwarden session key from thread-safe in-process storage.
///
/// Returns `None` if no session key has been stored or if the lock is poisoned.
#[must_use]
pub fn get_session_key() -> Option<SecretString> {
    BW_SESSION_STORE.read().ok().and_then(|guard| guard.clone())
}

/// Clears the stored Bitwarden session key.
pub fn clear_session_key() {
    if let Ok(mut guard) = BW_SESSION_STORE.write() {
        *guard = None;
    }
    clear_verified();
}

/// Stores the resolved `bw` CLI command path.
///
/// Call this from the UI layer after detecting the correct binary path.
/// All backend functions will use this path instead of bare `"bw"`.
pub fn set_bw_cmd(cmd: &str) {
    if let Ok(mut guard) = BW_CMD_STORE.write() {
        *guard = Some(cmd.to_string());
        tracing::debug!(bw_cmd = %cmd, "Bitwarden: CLI command path stored");
    }
}

/// Returns the stored `bw` CLI command path, or `"bw"` as default.
#[must_use]
pub fn get_bw_cmd() -> String {
    BW_CMD_STORE
        .read()
        .ok()
        .and_then(|guard| guard.clone())
        .unwrap_or_else(|| "bw".to_string())
}

/// Creates a [`Command`] for the resolved `bw` binary with extended PATH.
///
/// On macOS, GUI apps launched via `.app` bundle have minimal PATH.
/// This helper ensures `bw` (and any tools it invokes) are discoverable.
fn bw_command(bw_cmd: &str) -> Command {
    let mut cmd = Command::new(bw_cmd);
    cmd.env("PATH", crate::cli_download::get_extended_path());
    cmd
}

/// Resolves the `bw` CLI command path by probing known locations.
///
/// Checks (in order):
/// 1. Previously stored path via [`set_bw_cmd`]
/// 2. Flatpak CLI install directory (`~/.var/app/<app-id>/cli/bitwarden/bw`)
/// 3. Bare `"bw"` on PATH
///
/// Stores and returns the first working path.
#[must_use]
pub fn resolve_bw_cmd() -> String {
    // If already resolved, return cached value
    if let Ok(guard) = BW_CMD_STORE.read()
        && let Some(ref cmd) = *guard
    {
        return cmd.clone();
    }

    let mut candidates: Vec<String> = vec![];

    // Flatpak CLI directory
    if let Some(cli_dir) = crate::cli_download::get_cli_install_dir() {
        let flatpak_bw = cli_dir.join("bitwarden").join("bw");
        if flatpak_bw.exists() {
            candidates.push(flatpak_bw.to_string_lossy().to_string());
        }
    }

    // Non-Flatpak system paths
    if !crate::flatpak::is_flatpak() {
        candidates.extend(["/snap/bin/bw".to_string(), "/usr/local/bin/bw".to_string()]);
    }

    // Default bare command (relies on PATH)
    candidates.push("bw".to_string());

    for candidate in &candidates {
        if std::process::Command::new(candidate)
            .env("PATH", crate::cli_download::get_extended_path())
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
        {
            set_bw_cmd(candidate);
            return candidate.clone();
        }
    }

    // Nothing found — return "bw" as fallback (will produce clear error on use)
    "bw".to_string()
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
    /// Resolved path to the `bw` CLI binary
    bw_cmd: String,
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
    #[serde(
        default,
        deserialize_with = "super::serde_helpers::deserialize_optional_secret"
    )]
    password: Option<SecretString>,
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
    // Plain String: serde `Serialize` cannot derive over `Zeroizing`/`SecretString`.
    // The plaintext lifetime is bounded — the serialized JSON is wrapped in
    // `Zeroizing` at the call site and the template is dropped right after.
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
    #[expect(
        dead_code,
        reason = "Deserialized from `bw status` JSON but not used directly"
    )]
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
            bw_cmd: get_bw_cmd(),
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
            bw_cmd: get_bw_cmd(),
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

    /// Builds command with session key if available.
    ///
    /// Checks the instance-level session key first, then falls back to the
    /// global in-process session store set by [`set_session_key`] (module-level).
    /// This ensures that backends created via `BitwardenBackend::new()` (which
    /// have no instance session key) still pick up a session established by
    /// [`auto_unlock`].
    ///
    /// The session key is passed via `BW_SESSION` environment variable instead
    /// of `--session` CLI argument to avoid exposure in `/proc/PID/cmdline`.
    fn build_command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.bw_cmd);
        cmd.env("PATH", crate::cli_download::get_extended_path());
        // --nointeraction prevents CLI from prompting for input or performing
        // implicit network operations that can hang in sandboxed environments.
        cmd.arg("--nointeraction");
        cmd.args(args);

        if let Some(ref session) = self.session_key {
            cmd.env("BW_SESSION", session.expose_secret());
        } else if let Some(ref global_session) = get_session_key() {
            cmd.env("BW_SESSION", global_session.expose_secret());
        }

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd
    }

    /// Runs a bw command and returns stdout
    async fn run_command(&self, args: &[&str]) -> SecretResult<String> {
        // Only the verb reaches the log, never the whole argv.
        //
        // `bw create item` and `bw edit item` take the item as a base64 argument,
        // and that item contains the plaintext password (see `store`). Logging
        // `args` therefore wrote every saved password to the log at debug level,
        // wearing base64 as a disguise. The bulk credential transfer turned that
        // from an occasional line into one per stored password.
        let verb = args.iter().take(2).copied().collect::<Vec<_>>().join(" ");
        tracing::debug!(
            %verb,
            has_session = self.session_key.is_some(),
            has_global_session = get_session_key().is_some(),
            "Bitwarden run_command"
        );

        // Timeout prevents hanging when Bitwarden CLI stalls on network
        // requests (e.g. expired session triggers a sync attempt that blocks).
        let output = tokio::time::timeout(BW_INVOCATION_TIMEOUT, self.build_command(args).output())
            .await
            .map_err(|_| {
                tracing::warn!(%verb, "Bitwarden run_command: timed out after 30s");
                // The argv is not in this message for the same reason it is not in the
                // log line above: it carries the credential, and this error is shown to
                // the user and logged by `vault_ops`.
                SecretError::ConnectionFailed("bw command timed out after 30s".to_string())
            })?
            .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::debug!(
                %verb,
                stderr = %stderr,
                "Bitwarden run_command: failed"
            );
            return Err(SecretError::ConnectionFailed(format!(
                "bw command failed: {stderr}"
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        tracing::debug!(
            %verb,
            output_len = stdout.len(),
            "Bitwarden run_command: success"
        );
        Ok(stdout)
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
        let unlocked = self
            .get_status()
            .await
            .map(|s| s.status == "unlocked")
            .unwrap_or(false);
        if unlocked {
            mark_verified();
        }
        unlocked
    }

    /// Fast check: returns `true` if the vault was recently verified as
    /// unlocked and a session key is available, without spawning `bw status`.
    /// Falls back to the full [`is_unlocked`] check when the cached result
    /// has expired.
    ///
    /// # Bitwarden CLI v2026.4+ compatibility
    ///
    /// `bw status` ignores the `BW_SESSION` environment variable and always
    /// reports "locked" unless the vault was unlocked in the same process.
    /// When a session key is present, we trust it and skip the unreliable
    /// `bw status` check. If the session is actually expired, the next
    /// data command (`bw list items`) will fail and trigger a re-unlock.
    pub async fn is_unlocked_fast(&self) -> bool {
        let has_session = self.session_key.is_some() || get_session_key().is_some();
        if has_session {
            // Trust the session key — bw status is unreliable with BW_SESSION
            if is_recently_verified() {
                return true;
            }
            // Session key exists but not recently verified — re-mark and trust.
            // The alternative (calling bw status) always returns "locked" on
            // Bitwarden CLI v2026.4+ even with a valid session key.
            mark_verified();
            return true;
        }
        self.is_unlocked().await
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
        tracing::debug!(
            search_term = %search_term,
            connection_id = %connection_id,
            "Bitwarden find_item: searching vault"
        );

        let output = self
            .run_command(&["list", "items", "--search", &search_term])
            .await?;

        // Position, not the serde `Display`. `bw list items` includes
        // `login.password`, and serde reports a type mismatch by quoting the value
        // it choked on — a numeric password comes back as `invalid type: integer
        // \`1234\`, expected a string`, which `vault_ops` then logs and shows.
        let items: Vec<BitwardenItem> = serde_json::from_str(&output).map_err(|e| {
            SecretError::RetrieveFailed(format!(
                "Failed to parse items: {} error at line {}, column {}",
                serde_error_kind(&e),
                e.line(),
                e.column()
            ))
        })?;

        tracing::debug!(
            items_count = items.len(),
            search_term = %search_term,
            "Bitwarden find_item: parsed results"
        );

        // Find exact match by name
        let result = items.into_iter().find(|item| item.name == search_term);

        if result.is_none() {
            tracing::debug!(
                search_term = %search_term,
                "Bitwarden find_item: no exact match found"
            );
        }

        Ok(result)
    }

    /// Finds an item by exact vault entry name (without `RustConn:` prefix)
    /// and returns its password.
    ///
    /// This allows reusing existing Bitwarden entries that were not created
    /// by RustConn. The search matches the item name exactly as stored.
    ///
    /// # Errors
    /// Returns `SecretError` if the vault is locked or CLI fails.
    pub async fn find_password_by_exact_name(
        &self,
        entry_name: &str,
    ) -> SecretResult<Option<SecretString>> {
        tracing::debug!(
            entry_name = %entry_name,
            "Bitwarden find_password_by_exact_name: searching vault"
        );

        let output = self
            .run_command(&["list", "items", "--search", entry_name])
            .await?;

        // Position, not the serde `Display`. `bw list items` includes
        // `login.password`, and serde reports a type mismatch by quoting the value
        // it choked on — a numeric password comes back as `invalid type: integer
        // \`1234\`, expected a string`, which `vault_ops` then logs and shows.
        let items: Vec<BitwardenItem> = serde_json::from_str(&output).map_err(|e| {
            SecretError::RetrieveFailed(format!(
                "Failed to parse items: {} error at line {}, column {}",
                serde_error_kind(&e),
                e.line(),
                e.column()
            ))
        })?;

        tracing::debug!(
            items_count = items.len(),
            entry_name = %entry_name,
            "Bitwarden find_password_by_exact_name: parsed results"
        );

        // Find exact match by name (case-sensitive)
        let result = items.into_iter().find(|item| item.name == entry_name);

        if result.is_none() {
            tracing::debug!(
                entry_name = %entry_name,
                "Bitwarden find_password_by_exact_name: no exact match found"
            );
        }

        Ok(result.and_then(|item| item.login).and_then(|l| l.password))
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
        let entry_name = Self::entry_name(connection_id);
        tracing::debug!(
            connection_id = %connection_id,
            entry_name = %entry_name,
            has_session = self.session_key.is_some(),
            has_global_session = get_session_key().is_some(),
            "Bitwarden store: starting"
        );

        // Check if vault is unlocked
        if !self.is_unlocked_fast().await {
            tracing::error!("Bitwarden store: vault is locked");
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

            // Wrap the serialized item in Zeroizing: the JSON buffer holds the
            // plaintext password and is wiped on drop instead of lingering.
            let json = zeroize::Zeroizing::new(
                serde_json::to_string(&item_template)
                    .map_err(|e| SecretError::StoreFailed(format!("Failed to serialize: {e}")))?,
            );
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

            // Wrap the serialized item in Zeroizing: the JSON buffer holds the
            // plaintext password and is wiped on drop instead of lingering.
            let json = zeroize::Zeroizing::new(
                serde_json::to_string(&item_template)
                    .map_err(|e| SecretError::StoreFailed(format!("Failed to serialize: {e}")))?,
            );
            let encoded = base64_encode(json.as_bytes());

            self.run_command(&["create", "item", &encoded]).await?;
        }

        Ok(())
    }

    async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>> {
        tracing::debug!(
            connection_id = %connection_id,
            "Bitwarden retrieve: starting"
        );

        // Fast unlock check — skips `bw status` if recently verified
        if !self.is_unlocked_fast().await {
            return Err(SecretError::BackendUnavailable(
                "Bitwarden vault is locked. Please unlock with 'bw unlock'".to_string(),
            ));
        }

        // Note: `bw sync` is intentionally NOT called here. The vault is
        // synced once during `auto_unlock` and on explicit user request.
        // Skipping the per-retrieve sync eliminates a ~0.5-2s network
        // round-trip on every credential lookup, which is critical for
        // fast reconnect and batch operations.

        let item = if let Some(item) = self.find_item(connection_id).await? {
            tracing::debug!(
                item_id = %item.id,
                item_name = %item.name,
                "Bitwarden retrieve: item found"
            );
            item
        } else {
            tracing::debug!(
                connection_id = %connection_id,
                "Bitwarden retrieve: no item found"
            );
            return Ok(None);
        };

        let login = match item.login {
            Some(login) => login,
            None => return Ok(None),
        };

        Ok(Some(Credentials {
            username: login.username,
            password: login.password,
            key_passphrase: None,
            domain: item.notes,
        }))
    }

    async fn delete(&self, connection_id: &str) -> SecretResult<()> {
        // Check if vault is unlocked
        if !self.is_unlocked_fast().await {
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
        let installed = bw_command(&self.bw_cmd)
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
    let bw_cmd = get_bw_cmd();
    let output = bw_command(&bw_cmd).arg("--version").output().await.ok()?;

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
    let pw = zeroize::Zeroizing::new(password.expose_secret().to_string());

    tokio::task::spawn_blocking(move || unlock_vault_sync(&pw))
        .await
        .map_err(|e| SecretError::ConnectionFailed(format!("Unlock task panicked: {e}")))?
}

/// Unlocks the Bitwarden vault from a thread that is already blocking.
///
/// Same three strategies and the same [`BW_INVOCATION_TIMEOUT`] as
/// [`unlock_vault`], without the `spawn_blocking` hop — for callers that are
/// themselves inside a blocking worker and would otherwise have to nest a
/// runtime. The GUI's Secrets page has four such call sites, each of which used
/// to build its own `bw unlock`: none passed the extended `PATH` a sandboxed `bw`
/// needs, none passed `--nointeraction`, none had a deadline, and only two of the
/// four implemented the verbose fallback (issue
/// [#312](https://github.com/totoshko88/RustConn/issues/312)).
///
/// # Errors
/// Returns `SecretError::ConnectionFailed` if every strategy fails, the password
/// is wrong, or an invocation exceeded its budget.
pub fn unlock_vault_blocking(password: &SecretString) -> SecretResult<SecretString> {
    // Zeroizing, not a bare String: this is the master password in the clear and
    // it must not be left in freed heap when the call returns.
    let pw = zeroize::Zeroizing::new(password.expose_secret().to_string());
    unlock_vault_sync(&pw)
}

/// Strips anything that looks like a session key out of `bw`'s stderr.
///
/// Unlock stderr is logged at debug level and is also formatted into the
/// `SecretError` that the Secrets page renders, so it reaches two places a
/// credential must not. The session key is the vault's bearer token — it is what
/// [`get_session_key`] hands to every later `bw` call — and nothing here can
/// promise a given `bw` build keeps its `export BW_SESSION="…"` banner on stdout,
/// where [`extract_session_key`] expects it. A build that writes it to stderr
/// instead would leak the key through the diagnostic path on the one code path
/// that never parses it.
///
/// Redacts by line rather than dropping the whole message, because the useful
/// part of unlock stderr is the reason — "Invalid master password", "not logged
/// in" — and both callers match on it.
fn redact_session_keys(stderr: &str) -> String {
    stderr
        .lines()
        .map(|line| {
            if line.contains("BW_SESSION=") {
                "[redacted: line contained a session key]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Runs one bounded `bw unlock` attempt, with the master password in the child's
/// environment.
///
/// The password travels as `BW_PASSWORD` and is named to `bw` with
/// `--passwordenv`, never as an argument: argv is world-readable through
/// `/proc/PID/cmdline`.
///
/// `--nointeraction` is passed here but deliberately *not* by the stdin strategy
/// in [`unlock_vault_sync`], which needs `bw` to read the interactive password
/// prompt it would otherwise suppress.
///
/// # Errors
/// Returns `SecretError::ConnectionFailed` if the child cannot be spawned or
/// waited on, or if it exceeded [`BW_INVOCATION_TIMEOUT`].
fn unlock_attempt(bw_cmd: &str, password: &str, raw: bool) -> SecretResult<std::process::Output> {
    let mut cmd = std::process::Command::new(bw_cmd);
    cmd.env("PATH", crate::cli_download::get_extended_path());
    cmd.arg("--nointeraction");
    cmd.args(["unlock", "--passwordenv", "BW_PASSWORD"]);
    if raw {
        cmd.arg("--raw");
    }
    cmd.env("BW_PASSWORD", password)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

    let child = cmd
        .spawn()
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run bw unlock: {e}")))?;

    // "bw unlock" and not the argv: `what` is logged on timeout, and while this
    // particular argv holds only a variable *name*, keeping the habit is what
    // stops the next caller interpolating a value.
    crate::proc::wait_bounded(child, BW_INVOCATION_TIMEOUT, "bw unlock")
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to wait for bw unlock: {e}")))?
        .output()
        .ok_or_else(|| {
            SecretError::ConnectionFailed(format!(
                "bw unlock timed out after {}s",
                BW_INVOCATION_TIMEOUT.as_secs()
            ))
        })
}

/// Synchronous implementation of vault unlock.
///
/// Uses `std::process::Command` which is compatible with all Bitwarden CLI versions
/// including the new Rust-based CLI (v2026+).
///
/// Every child is bounded by [`BW_INVOCATION_TIMEOUT`]. None of them were: an
/// unlock that stalled on a network sync blocked its caller forever, and both
/// callers are worker threads with a status row waiting on them
/// (issue [#312](https://github.com/totoshko88/RustConn/issues/312)).
fn unlock_vault_sync(password: &str) -> SecretResult<SecretString> {
    // Presence, not length. A master password's length is bruteforce metadata,
    // and the GUI's unlock handler already declined to log it for that reason —
    // then sent all four of its call sites through here, where it was being
    // logged anyway.
    tracing::debug!(
        has_password = !password.is_empty(),
        "Bitwarden: unlock_vault_sync called"
    );

    let bw_cmd = get_bw_cmd();

    // Strategy 1: --passwordenv with --raw (returns session key directly)
    let output = unlock_attempt(&bw_cmd, password, true)?;

    let raw_succeeded = output.status.success();
    if raw_succeeded {
        let session_key = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !session_key.is_empty() {
            tracing::debug!("Bitwarden: unlocked with --passwordenv --raw");
            return Ok(SecretString::from(session_key));
        }
    }

    // `--raw` prints the key bare, with no `BW_SESSION=` in front of it, so
    // `redact_session_keys` has no marker to find here. That is why the content is
    // logged only when the command actually *failed*: exit 0 with empty stdout is
    // the one shape where the key could plausibly have gone to stderr instead, and
    // it is precisely the shape where redaction cannot recognise it. Kept as a
    // value regardless, because the `provided key` check below matches on it — a
    // `contains` yields a bool and cannot leak.
    let stderr_raw = redact_session_keys(&String::from_utf8_lossy(&output.stderr));
    if raw_succeeded {
        tracing::debug!(
            "Bitwarden: --raw unlock reported success but printed no key; trying verbose"
        );
    } else {
        tracing::debug!("Bitwarden: --raw unlock failed, trying verbose: {stderr_raw}");
    }

    // Strategy 2: --passwordenv without --raw (parse session key from verbose output)
    let output = unlock_attempt(&bw_cmd, password, false)?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(session_key) = extract_session_key(&stdout) {
            tracing::debug!("Bitwarden: unlocked with --passwordenv (verbose)");
            return Ok(SecretString::from(session_key));
        }
        tracing::debug!("Bitwarden: verbose unlock succeeded but no session key in output");
    }

    let stderr_verbose = redact_session_keys(&String::from_utf8_lossy(&output.stderr));
    tracing::debug!("Bitwarden: verbose unlock failed: {stderr_verbose}");

    // Strategy 3: stdin pipe without --raw (for maximum compatibility).
    //
    // No `--nointeraction` here, unlike the two attempts above: this strategy
    // exists for older CLI builds that have no `--passwordenv`, and it works by
    // answering the interactive prompt that `--nointeraction` suppresses.
    let mut child = std::process::Command::new(&bw_cmd)
        .env("PATH", crate::cli_download::get_extended_path())
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

    // Bounded like the other two. The stdin write above is what unblocks a
    // healthy child; a child that never reads it used to park here forever.
    let output = crate::proc::wait_bounded(child, BW_INVOCATION_TIMEOUT, "bw unlock")
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to wait for bw: {e}")))?
        .output()
        .ok_or_else(|| {
            SecretError::ConnectionFailed(format!(
                "bw unlock timed out after {}s",
                BW_INVOCATION_TIMEOUT.as_secs()
            ))
        })?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        if let Some(session_key) = extract_session_key(&stdout) {
            tracing::debug!("Bitwarden: unlocked with stdin pipe (verbose)");
            return Ok(SecretString::from(session_key));
        }
    }

    // Redacted before it is formatted into the error below, which the Secrets
    // page renders and `vault_ops` logs.
    let stderr = redact_session_keys(&String::from_utf8_lossy(&output.stderr));

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
            if let Some(start) = line.find('"')
                && let Some(end) = line.rfind('"')
                && end > start
            {
                return Some(line[start + 1..end].to_string());
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
    clear_verified();
    clear_session_key();
    let bw_cmd = get_bw_cmd();
    // Bounded like every other bw invocation: `lock` can trigger a network sync
    // that stalls, and this is awaited from the GUI's teardown path.
    let output = tokio::time::timeout(
        BW_INVOCATION_TIMEOUT,
        bw_command(&bw_cmd).arg("lock").output(),
    )
    .await
    .map_err(|_| {
        SecretError::ConnectionFailed(format!(
            "bw lock timed out after {}s",
            BW_INVOCATION_TIMEOUT.as_secs()
        ))
    })?
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
    let bw_cmd = get_bw_cmd();
    // `login --apikey` reaches the Bitwarden server, so it is the most likely of
    // all bw invocations to hang on a slow or unreachable network. Bound it.
    let output = tokio::time::timeout(
        BW_INVOCATION_TIMEOUT,
        bw_command(&bw_cmd)
            .args(["login", "--apikey"])
            .env("BW_CLIENTID", client_id.expose_secret())
            .env("BW_CLIENTSECRET", client_secret.expose_secret())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output(),
    )
    .await
    .map_err(|_| {
        SecretError::ConnectionFailed(format!(
            "bw login timed out after {}s",
            BW_INVOCATION_TIMEOUT.as_secs()
        ))
    })?
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
    clear_verified();
    let bw_cmd = get_bw_cmd();
    // Bounded like the others: `logout` can attempt a final server round-trip.
    let output = tokio::time::timeout(
        BW_INVOCATION_TIMEOUT,
        bw_command(&bw_cmd).arg("logout").output(),
    )
    .await
    .map_err(|_| {
        SecretError::ConnectionFailed(format!(
            "bw logout timed out after {}s",
            BW_INVOCATION_TIMEOUT.as_secs()
        ))
    })?
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
    let bw_cmd = get_bw_cmd();
    let output = bw_command(&bw_cmd)
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
    // Wrapped on the line it is read, so the plain `String` does not stay alive
    // across the second fallible lookup — where a failure would drop it un-wiped.
    let client_id = super::keyring::lookup(KEY_BW_CLIENT_ID)
        .await?
        .map(SecretString::from);
    let client_secret = super::keyring::lookup(KEY_BW_CLIENT_SECRET)
        .await?
        .map(SecretString::from);

    match (client_id, client_secret) {
        (Some(id), Some(secret)) => Ok(Some((id, secret))),
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

    set_session_key(session_key);
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
#[expect(
    clippy::too_many_lines,
    reason = "long match/dispatch over many enum variants; splitting per variant only relocates the boilerplate"
)] // multi-strategy unlock with ordered fallbacks
pub async fn auto_unlock(
    settings: &crate::config::SecretSettings,
) -> SecretResult<BitwardenBackend> {
    // 0. Fast path: if session key exists and was recently verified, skip
    //    all CLI calls entirely. This makes reconnect near-instant.
    let stored_session = get_session_key().or_else(|| {
        std::env::var("BW_SESSION")
            .ok()
            .filter(|s| !s.is_empty())
            .map(SecretString::from)
    });
    if let Some(ref session) = stored_session
        && is_recently_verified()
    {
        tracing::debug!("Bitwarden: using cached session key (recently verified)");
        // Third site of the same round trip as the two below: going through
        // `expose_secret().to_owned()` puts the key in a bare `String` that drops
        // unwiped. `clone` and not a move because `stored_session` is borrowed and
        // is read again further down.
        return Ok(BitwardenBackend::with_session(session.clone()));
    }

    // 1. Check in-process session store, then fall back to BW_SESSION env var
    //    (supports externally unlocked vaults, e.g. `export BW_SESSION=...` in shell)
    //
    // Trust the session key without calling `bw status` — on Bitwarden CLI
    // v2026.4+ `bw status` ignores BW_SESSION and always reports "locked".
    // If the session is expired, the subsequent data command will fail and
    // the caller can retry.
    if let Some(session) = stored_session {
        tracing::debug!("Bitwarden: using existing session key (trusting without status check)");
        mark_verified();
        let backend = BitwardenBackend::with_session(session);
        let _ = backend.sync().await;
        return Ok(backend);
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
        mark_verified();
        let _ = bare.sync().await;
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
    if settings.bitwarden_save_to_keyring
        && let Ok(Some(password)) = get_master_password_from_keyring().await
    {
        tracing::debug!("Bitwarden: attempting unlock with keyring password");
        match unlock_vault(&password).await {
            Ok(session_key) => {
                // `clone`, not `SecretString::from(expose_secret().to_owned())`:
                // the round trip put the key in a bare `String` that dropped
                // unwiped. Same defect the Secrets page had at four call sites.
                set_session_key(session_key.clone());
                mark_verified();
                let backend = BitwardenBackend::with_session(session_key);
                let _ = backend.sync().await;
                return Ok(backend);
            }
            Err(e) => {
                let is_key_type_error = e.to_string().contains("key type mismatch");
                tracing::warn!("Bitwarden: keyring password unlock failed: {e}");
                // If key type mismatch, try re-login before giving up
                if is_key_type_error
                    && try_relogin_and_unlock(settings, &password).await.is_ok()
                    && let Some(key) = get_session_key()
                {
                    mark_verified();
                    return Ok(BitwardenBackend::with_session(key));
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
                    // See the sibling arm above: `clone` keeps the key inside
                    // `SecretString` instead of round-tripping it through a bare
                    // `String` that drops unwiped.
                    set_session_key(session_key.clone());
                    mark_verified();
                    let backend = BitwardenBackend::with_session(session_key);
                    let _ = backend.sync().await;
                    return Ok(backend);
                }
                Err(e) => {
                    let is_key_type_error = e.to_string().contains("key type mismatch");
                    tracing::warn!("Bitwarden: saved password unlock failed: {e}");
                    // If key type mismatch, try re-login before giving up
                    if is_key_type_error
                        && try_relogin_and_unlock(settings, password).await.is_ok()
                        && let Some(key) = get_session_key()
                    {
                        mark_verified();
                        return Ok(BitwardenBackend::with_session(key));
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

impl std::fmt::Debug for BitwardenBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BitwardenBackend")
            // Never log the actual session key — only its presence.
            .field("session_present", &self.session_key.is_some())
            .field("server_url", &self.server_url)
            .field("organization_id", &self.organization_id)
            .field("folder_name", &self.folder_name)
            .field("bw_cmd", &self.bw_cmd)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_secret() {
        let session = SecretString::from("hunter2-bw-session".to_string());
        let backend = BitwardenBackend::with_session(session)
            .with_server_url("https://vault.example.org")
            .with_organization("org-hunter2");
        let rendered = format!("{backend:?}");
        assert!(
            !rendered.contains("hunter2-bw-session"),
            "Debug leaked the session key: {rendered}"
        );
        assert!(rendered.contains("BitwardenBackend"));
        assert!(rendered.contains("session_present"));
    }
}

#[cfg(test)]
mod unlock_tests {
    use super::{BW_INVOCATION_TIMEOUT, redact_session_keys};

    /// Unlock stderr reaches a debug log *and* the error string the Secrets page
    /// renders. `extract_session_key` looks for the export banner on stdout, so a
    /// `bw` build that writes it to stderr instead would leak the vault's bearer
    /// token down the diagnostic path — the one path that never parses it.
    #[test]
    fn a_session_key_on_stderr_does_not_survive_into_a_message() {
        let stderr = "\
Something went wrong
export BW_SESSION=\"hunter2-session-key\"
Invalid master password.";

        let redacted = redact_session_keys(stderr);

        assert!(
            !redacted.contains("hunter2-session-key"),
            "session key survived redaction: {redacted}"
        );
        // The reason is what both callers match on, so it has to stay.
        assert!(redacted.contains("Invalid master password."));
        assert!(redacted.contains("Something went wrong"));
        // Redacted by line, so the shape of the output is preserved.
        assert_eq!(redacted.lines().count(), 3);
    }

    #[test]
    fn the_unquoted_form_is_redacted_too() {
        let redacted = redact_session_keys("BW_SESSION=bare-key-no-quotes");
        assert!(!redacted.contains("bare-key-no-quotes"));
    }

    /// Ordinary stderr must pass through untouched, or the callers that match on
    /// "not logged in" and "provided key is not the expected type" would stop
    /// recognising it.
    #[test]
    fn stderr_without_a_session_key_is_left_alone() {
        for message in [
            "You are not logged in.",
            "Invalid master password.",
            "provided key is not the expected type",
            "",
        ] {
            assert_eq!(redact_session_keys(message), message);
        }
    }

    /// The budget has to clear a real unlock, measured at about 4.5 s in issue
    /// #312, by enough margin that a slow vault server is not mistaken for a hang.
    #[test]
    fn the_invocation_budget_clears_a_real_unlock() {
        assert!(BW_INVOCATION_TIMEOUT >= std::time::Duration::from_secs(15));
    }
}
