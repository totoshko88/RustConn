//! Passbolt CLI backend for password management
//!
//! This module implements credential storage using the Passbolt CLI
//! (`passbolt` / `go-passbolt-cli`). Passbolt is an open-source
//! password manager for teams with a server-based architecture.
//!
//! # Prerequisites
//!
//! The Passbolt CLI must be installed and configured:
//! ```bash
//! passbolt configure --serverAddress https://passbolt.example.org \
//!     --userPassword 'passphrase' --userPrivateKeyFile 'key.asc'
//! ```
//!
//! # Resources
//!
//! Credentials are stored as Passbolt "resources" with the naming
//! convention `RustConn: {connection_id}`. Each resource stores
//! username in the name/description and password as the secret.

use async_trait::async_trait;
use secrecy::SecretString;
use serde::Deserialize;
use std::process::Stdio;
use tokio::process::Command;

use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

use super::backend::SecretBackend;

/// Passbolt CLI backend
///
/// Uses the `passbolt` command-line tool (go-passbolt-cli) to interact
/// with a Passbolt server. Requires prior configuration via
/// `passbolt configure`.
pub struct PassboltBackend {
    /// Custom server address (overrides config file)
    server_address: Option<String>,
}

/// Passbolt resource from JSON output
#[derive(Debug, Deserialize)]
struct PassboltResource {
    #[serde(alias = "ID")]
    id: String,
    #[serde(alias = "Name")]
    name: String,
    #[serde(alias = "Username", default)]
    _username: Option<String>,
    #[serde(alias = "URI", default)]
    _uri: Option<String>,
}

/// Passbolt resource detail (from `get resource`)
#[derive(Debug, Deserialize)]
struct PassboltResourceDetail {
    #[serde(alias = "ID")]
    _id: String,
    #[serde(alias = "Name")]
    _name: String,
    #[serde(alias = "Username", default)]
    username: Option<String>,
    #[serde(alias = "Password", default)]
    password: Option<String>,
    #[serde(alias = "URI", default)]
    _uri: Option<String>,
    #[serde(alias = "Description", default)]
    _description: Option<String>,
}

/// Passbolt version information
#[derive(Debug, Clone)]
pub struct PassboltVersion {
    /// CLI version string
    pub version: String,
    /// Whether CLI is installed
    pub installed: bool,
}

/// Passbolt status information
#[derive(Debug, Clone)]
pub struct PassboltStatus {
    /// Whether CLI is installed
    pub installed: bool,
    /// CLI version
    pub version: Option<String>,
    /// Whether configuration exists (can connect)
    pub configured: bool,
    /// Server address from config
    pub server_address: Option<String>,
    /// Status message for display
    pub status_message: String,
}

impl PassboltBackend {
    /// Creates a new Passbolt backend
    #[must_use]
    pub fn new() -> Self {
        Self {
            server_address: None,
        }
    }

    /// Sets a custom server address (overrides config file)
    #[must_use]
    pub fn with_server_address(mut self, address: impl Into<String>) -> Self {
        self.server_address = Some(address.into());
        self
    }

    /// Builds a passbolt command with common flags
    fn build_command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("passbolt");
        cmd.args(args);

        if let Some(ref addr) = self.server_address {
            cmd.arg("--serverAddress").arg(addr);
        }

        // Always request JSON output for parsing
        cmd.arg("--json");

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd
    }

    /// Runs a passbolt command and returns stdout
    async fn run_command(&self, args: &[&str]) -> SecretResult<String> {
        let output =
            self.build_command(args).output().await.map_err(|e| {
                SecretError::ConnectionFailed(format!("Failed to run passbolt: {e}"))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SecretError::ConnectionFailed(format!(
                "passbolt command failed: {stderr}"
            )));
        }

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    }

    /// Generates a unique resource name for a connection entry
    fn entry_name(connection_id: &str) -> String {
        format!("RustConn: {connection_id}")
    }

    /// Finds a resource by connection ID (searches by name)
    async fn find_resource(&self, connection_id: &str) -> SecretResult<Option<PassboltResource>> {
        let name = Self::entry_name(connection_id);

        let output = self.run_command(&["list", "resource"]).await;

        // If command fails, assume no resources
        let output = match output {
            Ok(o) => o,
            Err(_) => return Ok(None),
        };

        let resources: Vec<PassboltResource> = serde_json::from_str(&output).unwrap_or_default();

        Ok(resources.into_iter().find(|r| r.name == name))
    }

    /// Gets full resource details including password
    async fn get_resource_detail(&self, resource_id: &str) -> SecretResult<PassboltResourceDetail> {
        let output = self
            .run_command(&["get", "resource", "--id", resource_id])
            .await?;

        serde_json::from_str(&output)
            .map_err(|e| SecretError::RetrieveFailed(format!("Failed to parse resource: {e}")))
    }

    /// Checks if the CLI is configured and can connect
    pub async fn is_configured(&self) -> bool {
        // Try listing users as a connectivity check
        self.run_command(&["list", "user"]).await.is_ok()
    }
}

impl Default for PassboltBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl SecretBackend for PassboltBackend {
    async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
        if !self.is_configured().await {
            return Err(SecretError::BackendUnavailable(
                "Passbolt CLI not configured. Run \
                 'passbolt configure' first"
                    .to_string(),
            ));
        }

        let name = Self::entry_name(connection_id);
        let username = credentials.username.clone().unwrap_or_default();
        let password = credentials
            .expose_password()
            .unwrap_or_default()
            .to_string();

        // Check if resource already exists
        if let Some(existing) = self.find_resource(connection_id).await? {
            // Update existing resource
            let mut args = vec!["update", "resource", "--id", &existing.id];

            // Only update fields that have values
            if !username.is_empty() {
                args.extend(["--username", &username]);
            }
            if !password.is_empty() {
                args.extend(["--password", &password]);
            }

            self.run_command(&args).await?;
        } else {
            // Create new resource
            let mut args = vec!["create", "resource", "--name", &name];

            if !username.is_empty() {
                args.extend(["--username", &username]);
            }
            if !password.is_empty() {
                args.extend(["--password", &password]);
            }

            self.run_command(&args).await?;
        }

        Ok(())
    }

    async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>> {
        if !self.is_configured().await {
            return Err(SecretError::BackendUnavailable(
                "Passbolt CLI not configured. Run \
                 'passbolt configure' first"
                    .to_string(),
            ));
        }

        let resource = match self.find_resource(connection_id).await? {
            Some(r) => r,
            None => return Ok(None),
        };

        // Get full details including password
        let detail = self.get_resource_detail(&resource.id).await?;

        Ok(Some(Credentials {
            username: detail.username.filter(|u| !u.is_empty()),
            password: detail
                .password
                .filter(|p| !p.is_empty())
                .map(SecretString::from),
            key_passphrase: None,
            domain: None,
        }))
    }

    async fn delete(&self, connection_id: &str) -> SecretResult<()> {
        if !self.is_configured().await {
            return Err(SecretError::BackendUnavailable(
                "Passbolt CLI not configured. Run \
                 'passbolt configure' first"
                    .to_string(),
            ));
        }

        let resource = match self.find_resource(connection_id).await? {
            Some(r) => r,
            None => return Ok(()),
        };

        self.run_command(&["delete", "resource", "--id", &resource.id])
            .await?;

        Ok(())
    }

    async fn is_available(&self) -> bool {
        // Check if passbolt CLI is installed
        let installed = Command::new("passbolt")
            .arg("--version")
            .output()
            .await
            .map(|o| o.status.success())
            .unwrap_or(false);

        if !installed {
            return false;
        }

        // Check if configured and can connect
        self.is_configured().await
    }

    fn backend_id(&self) -> &'static str {
        "passbolt"
    }

    fn display_name(&self) -> &'static str {
        "Passbolt"
    }
}

/// Gets Passbolt CLI version
pub async fn get_passbolt_version() -> Option<PassboltVersion> {
    let output = Command::new("passbolt")
        .arg("--version")
        .output()
        .await
        .ok()?;

    if output.status.success() {
        let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Some(PassboltVersion {
            version,
            installed: true,
        })
    } else {
        None
    }
}

/// Gets comprehensive Passbolt status
pub async fn get_passbolt_status() -> PassboltStatus {
    // Check if installed
    let version_output = Command::new("passbolt").arg("--version").output().await;

    let (installed, version) = match version_output {
        Ok(output) if output.status.success() => {
            let ver = String::from_utf8_lossy(&output.stdout).trim().to_string();
            (true, Some(ver))
        }
        _ => (false, None),
    };

    if !installed {
        return PassboltStatus {
            installed: false,
            version: None,
            configured: false,
            server_address: None,
            status_message: "Not installed".to_string(),
        };
    }

    // Check if configured by trying to list users
    let list_output = Command::new("passbolt")
        .args(["list", "user", "--json"])
        .output()
        .await;

    match list_output {
        Ok(output) if output.status.success() => PassboltStatus {
            installed: true,
            version,
            configured: true,
            server_address: None,
            status_message: "Configured".to_string(),
        },
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let message = if stderr.contains("no configuration") {
                "Not configured"
            } else if stderr.contains("authentication") || stderr.contains("passphrase") {
                "Authentication failed"
            } else {
                "Not configured"
            };
            PassboltStatus {
                installed: true,
                version,
                configured: false,
                server_address: None,
                status_message: message.to_string(),
            }
        }
        Err(_) => PassboltStatus {
            installed: true,
            version,
            configured: false,
            server_address: None,
            status_message: "Error checking status".to_string(),
        },
    }
}
