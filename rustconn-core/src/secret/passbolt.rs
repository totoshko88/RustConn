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

use std::process::Stdio;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tokio::process::Command;

use super::backend::SecretBackend;
use super::serde_helpers::serde_error_kind;
use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

/// Ceiling on a single `passbolt` CLI invocation that reaches the server.
///
/// `go-passbolt-cli` talks to a remote Passbolt server for every real
/// operation, so a slow or unreachable host would otherwise block the caller
/// forever. Nothing bounded these calls before. Thirty seconds matches the
/// Bitwarden and 1Password backends: long enough for a genuine round-trip,
/// short enough to fail while the user is still watching. Local `--version`
/// availability probes are left unbounded — they never touch the network.
const PASSBOLT_INVOCATION_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// Passbolt CLI backend
///
/// Uses the `passbolt` command-line tool (go-passbolt-cli) to interact
/// with a Passbolt server. Requires prior configuration via
/// `passbolt configure`.
pub struct PassboltBackend {
    /// Custom server address (overrides config file)
    server_address: Option<String>,
    /// GPG private key passphrase (overrides config file)
    user_password: Option<SecretString>,
}

/// Passbolt resource from JSON output
#[derive(Debug, Deserialize)]
struct PassboltResource {
    #[serde(alias = "ID")]
    id: String,
    #[serde(alias = "Name")]
    name: String,
    #[serde(rename = "username", alias = "Username", default)]
    _username: Option<String>,
    #[serde(rename = "uri", alias = "URI", default)]
    _uri: Option<String>,
}

/// Passbolt resource detail (from `get resource`)
#[derive(Debug, Deserialize)]
struct PassboltResourceDetail {
    #[serde(rename = "id", alias = "ID", default)]
    _id: Option<String>,
    #[serde(rename = "name", alias = "Name", default)]
    _name: Option<String>,
    #[serde(rename = "username", alias = "Username", default)]
    username: Option<String>,
    #[serde(
        rename = "password",
        alias = "Password",
        default,
        deserialize_with = "super::serde_helpers::deserialize_optional_secret"
    )]
    password: Option<SecretString>,
    #[serde(rename = "uri", alias = "URI", default)]
    _uri: Option<String>,
    #[serde(rename = "description", alias = "Description", default)]
    _description: Option<String>,
    #[serde(rename = "folder_parent_id", default)]
    _folder_parent_id: Option<String>,
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
            user_password: None,
        }
    }

    /// Sets a custom server address (overrides config file)
    #[must_use]
    pub fn with_server_address(mut self, address: impl Into<String>) -> Self {
        self.server_address = Some(address.into());
        self
    }

    /// Sets the GPG private key passphrase (overrides config file)
    #[must_use]
    pub fn with_user_password(mut self, passphrase: SecretString) -> Self {
        self.user_password = Some(passphrase);
        self
    }

    /// Builds a passbolt command with common flags
    fn build_command(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new("passbolt");
        cmd.env("PATH", crate::cli_download::get_extended_path());
        // `PASSBOLT_INVOCATION_TIMEOUT` drops the future when it fires, and
        // `tokio::process` does not kill the child on drop by default. This
        // matters more here than elsewhere: the passphrase is on this command's
        // argv, so an orphan leaves it visible in `/proc/PID/cmdline`.
        cmd.kill_on_drop(true);
        cmd.args(args);

        if let Some(ref addr) = self.server_address {
            cmd.arg("--serverAddress").arg(addr);
        }

        if let Some(ref passphrase) = self.user_password {
            // NOTE: go-passbolt-cli does not support stdin-based passphrase input.
            // The passphrase is visible in /proc/PID/cmdline for the command duration.
            // This is a limitation of the upstream CLI tool.
            cmd.arg("--userPassword").arg(passphrase.expose_secret());
        }

        // Always request JSON output for parsing
        cmd.arg("--json");

        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        cmd
    }

    /// Removes the secret values this client passes on the command line from
    /// `text`.
    ///
    /// `go-passbolt-cli` takes both the account passphrase and the resource
    /// password as flags (see [`Self::build_command`]), and it answers a rejected
    /// invocation by quoting the invocation. That put the credential into
    /// [`SecretError::ConnectionFailed`], which callers log: `vault_ops` logs it on
    /// every failed connection and group password save, and the bulk credential
    /// transfer would have multiplied it by the number of stored passwords.
    ///
    /// Redacting at the point the error is built covers every caller, rather than
    /// asking each one to remember. Short values are left alone: a two-character
    /// password would otherwise turn every occurrence of those characters in a
    /// legitimate diagnostic into `***`, destroying the message to protect
    /// something already trivially guessable.
    fn redact_secrets(&self, text: &str, args: &[&str]) -> String {
        /// Below this length, redaction destroys more than it protects.
        const MIN_REDACTED_LEN: usize = 4;

        let mut redacted = text.to_owned();
        // Wiped on drop: this is a function whose whole purpose is handling secret
        // values, so the copies it makes of them are the last place to be casual
        // about it.
        let mut secrets: Vec<zeroize::Zeroizing<String>> = Vec::new();

        if let Some(ref passphrase) = self.user_password {
            secrets.push(zeroize::Zeroizing::new(
                passphrase.expose_secret().to_owned(),
            ));
        }
        // Values that follow a password-bearing flag. Taken from the argv this
        // call actually built, so a new flag added to `store` is covered as soon
        // as it is named here rather than silently leaking.
        for pair in args.windows(2) {
            if matches!(pair[0], "--password" | "--userPassword") {
                secrets.push(zeroize::Zeroizing::new(pair[1].to_owned()));
            }
        }

        for secret in &secrets {
            if secret.len() >= MIN_REDACTED_LEN {
                redacted = redacted.replace(secret.as_str(), "***");
            }
        }
        redacted
    }

    /// Runs a passbolt command and returns stdout
    async fn run_command(&self, args: &[&str]) -> SecretResult<String> {
        let output = tokio::time::timeout(
            PASSBOLT_INVOCATION_TIMEOUT,
            self.build_command(args).output(),
        )
        .await
        .map_err(|_| {
            SecretError::ConnectionFailed(format!(
                "passbolt command timed out after {}s",
                PASSBOLT_INVOCATION_TIMEOUT.as_secs()
            ))
        })?
        .map_err(|e| SecretError::ConnectionFailed(format!("Failed to run passbolt: {e}")))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(SecretError::ConnectionFailed(format!(
                "passbolt command failed: {}",
                self.redact_secrets(&stderr, args)
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

        // The serde error's `Display` is deliberately not used. It quotes the
        // value it choked on, and the value being parsed here is the resource's
        // password: a numeric password arrives as a JSON number, the deserializer
        // reports `invalid type: integer \`1234\`, expected a string`, and that
        // string then reaches `tracing::error!` through `vault_ops`. The position
        // is what a report needs; the token is what leaks.
        serde_json::from_str(&output).map_err(|e| {
            SecretError::RetrieveFailed(format!(
                "Failed to parse resource: {} error at line {}, column {}",
                serde_error_kind(&e),
                e.line(),
                e.column()
            ))
        })
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
        // Wiped on drop. The CLI needs the password as a plain argument (it has no
        // stdin path — see `build_command`), but that is no reason to also leave
        // the copy behind in freed memory.
        let password = zeroize::Zeroizing::new(
            credentials
                .expose_password()
                .unwrap_or_default()
                .to_string(),
        );

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
                .filter(|p| !ExposeSecret::expose_secret(p).is_empty()),
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
            .env("PATH", crate::cli_download::get_extended_path())
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
        .env("PATH", crate::cli_download::get_extended_path())
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

/// Reads the Passbolt server address from the CLI configuration file.
///
/// The `go-passbolt-cli` stores configuration in
/// `~/.config/go-passbolt-cli/config.json` with a `serverAddress` field.
fn read_passbolt_server_address() -> Option<String> {
    let home = std::env::var("HOME").ok()?;
    let config_path = std::path::PathBuf::from(home)
        .join(".config")
        .join("go-passbolt-cli")
        .join("config.json");

    let content = std::fs::read_to_string(config_path).ok()?;
    let config: serde_json::Value = serde_json::from_str(&content).ok()?;
    config
        .get("serverAddress")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Gets comprehensive Passbolt status
pub async fn get_passbolt_status() -> PassboltStatus {
    // Check if installed
    let version_output = Command::new("passbolt")
        .env("PATH", crate::cli_download::get_extended_path())
        .arg("--version")
        .output()
        .await;

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

    // Try to read server address from CLI config
    let server_address = read_passbolt_server_address();

    // Check if configured by trying to list users. This reaches the server, so
    // bound it: a timeout is treated the same as any other failure to list —
    // "not configured" — which is what the `Err` arm below already does.
    let list_output = match tokio::time::timeout(
        PASSBOLT_INVOCATION_TIMEOUT,
        Command::new("passbolt")
            .env("PATH", crate::cli_download::get_extended_path())
            // Bounded below, so the child must not outlive the timeout.
            .kill_on_drop(true)
            .args(["list", "user", "--json"])
            .output(),
    )
    .await
    {
        Ok(result) => result,
        Err(_elapsed) => Err(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            format!(
                "passbolt list user timed out after {}s",
                PASSBOLT_INVOCATION_TIMEOUT.as_secs()
            ),
        )),
    };

    match list_output {
        Ok(output) if output.status.success() => PassboltStatus {
            installed: true,
            version,
            configured: true,
            server_address,
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
                server_address,
                status_message: message.to_string(),
            }
        }
        Err(_) => PassboltStatus {
            installed: true,
            version,
            configured: false,
            server_address,
            status_message: "Error checking status".to_string(),
        },
    }
}

// ============================================================================
// Keyring storage for Passbolt credentials
// ============================================================================

const KEY_PB_PASSPHRASE: &str = "passbolt-passphrase";

/// Stores Passbolt GPG passphrase in system keyring
///
/// # Errors
/// Returns `SecretError` if storage fails
pub async fn store_passphrase_in_keyring(passphrase: &SecretString) -> SecretResult<()> {
    super::keyring::store(
        KEY_PB_PASSPHRASE,
        passphrase.expose_secret(),
        "Passbolt GPG Passphrase",
    )
    .await
}

/// Retrieves Passbolt GPG passphrase from system keyring
///
/// # Errors
/// Returns `SecretError` if retrieval fails
pub async fn get_passphrase_from_keyring() -> SecretResult<Option<SecretString>> {
    super::keyring::lookup(KEY_PB_PASSPHRASE)
        .await
        .map(|opt| opt.map(SecretString::from))
}

/// Deletes Passbolt GPG passphrase from system keyring
///
/// # Errors
/// Returns `SecretError` if deletion fails
pub async fn delete_passphrase_from_keyring() -> SecretResult<()> {
    super::keyring::clear(KEY_PB_PASSPHRASE).await
}

impl std::fmt::Debug for PassboltBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassboltBackend")
            .field("server_address", &self.server_address)
            .field(
                "user_password",
                &self.user_password.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn debug_does_not_leak_secret() {
        let backend = PassboltBackend::new()
            .with_server_address("https://passbolt.example.org")
            .with_user_password(SecretString::from("hunter2"));
        let rendered = format!("{backend:?}");
        assert!(rendered.contains("PassboltBackend"));
        assert!(
            rendered.contains("server_address"),
            "unexpected Debug shape: {rendered}"
        );
        assert!(
            !rendered.contains("hunter2"),
            "Debug leaks passphrase: {rendered}"
        );
        assert!(
            rendered.contains("[REDACTED]"),
            "Debug must show redacted marker: {rendered}"
        );
    }

    /// The CLI answers a rejected invocation by quoting it, and the invocation
    /// carries the password as a flag, so its stderr is a credential source.
    #[test]
    fn redaction_removes_the_resource_password_from_backend_output() {
        let backend = PassboltBackend::new();
        let args = ["create", "resource", "--password", "correct-horse"];
        let stderr =
            "Error: unknown flag\nUsage: passbolt create resource --password correct-horse";

        let redacted = backend.redact_secrets(stderr, &args);

        assert!(
            !redacted.contains("correct-horse"),
            "password survived redaction: {redacted}"
        );
        assert!(
            redacted.contains("unknown flag"),
            "the diagnostic itself must survive: {redacted}"
        );
    }

    #[test]
    fn redaction_removes_the_account_passphrase_from_backend_output() {
        let backend = PassboltBackend::new()
            .with_user_password(SecretString::from("s3cret-phrase".to_owned()));

        let redacted = backend.redact_secrets("failed with --userPassword s3cret-phrase", &[]);

        assert!(
            !redacted.contains("s3cret-phrase"),
            "passphrase survived redaction: {redacted}"
        );
    }

    /// A very short secret appears inside ordinary words, so replacing it would
    /// shred the diagnostic to protect something already trivially guessable.
    #[test]
    fn redaction_leaves_a_too_short_secret_alone() {
        let backend = PassboltBackend::new();
        let args = ["create", "resource", "--password", "ab"];

        let redacted = backend.redact_secrets("unable to reach the server", &args);

        assert_eq!(redacted, "unable to reach the server");
    }
}
