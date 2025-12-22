//! `KeePassXC` browser integration protocol backend
//!
//! This module implements the `KeePassXC` browser integration protocol for
//! secure credential storage. It communicates with `KeePassXC` via a Unix socket
//! using the native messaging protocol.

use async_trait::async_trait;
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;

use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

use super::backend::SecretBackend;

/// `KeePassXC` browser integration protocol client
///
/// This backend communicates with `KeePassXC` using the browser integration
/// protocol over a Unix socket. It requires `KeePassXC` to be running with
/// browser integration enabled.
pub struct KeePassXcBackend {
    /// Path to the `KeePassXC` socket
    socket_path: PathBuf,
    /// Client ID for association
    client_id: String,
    /// Whether the backend has been associated with `KeePassXC`
    associated: bool,
}

/// Request message for `KeePassXC` protocol
#[derive(Debug, Serialize)]
struct KeePassXcRequest {
    action: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    login: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    password: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    group: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    uuid: Option<String>,
}

/// Response message from `KeePassXC` protocol
#[derive(Debug, Deserialize)]
struct KeePassXcResponse {
    #[serde(default)]
    success: Option<String>,
    #[serde(default)]
    error: Option<String>,
    #[serde(default)]
    entries: Option<Vec<KeePassXcEntry>>,
}

/// Entry returned from `KeePassXC`
#[derive(Debug, Deserialize)]
struct KeePassXcEntry {
    login: Option<String>,
    password: Option<String>,
    /// Entry name from `KeePassXC`
    ///
    /// Part of the `KeePassXC` response structure. Currently unused but
    /// preserved for potential future use in entry display or logging.
    /// Required for correct JSON deserialization of `KeePassXC` responses.
    #[serde(default)]
    #[allow(dead_code)] // Required for JSON deserialization completeness
    name: Option<String>,
    /// Entry UUID from `KeePassXC`
    ///
    /// Part of the `KeePassXC` response structure. Currently unused but
    /// preserved for potential future use in entry identification or updates.
    /// Required for correct JSON deserialization of `KeePassXC` responses.
    #[serde(default)]
    #[allow(dead_code)] // Required for JSON deserialization completeness
    uuid: Option<String>,
}

impl KeePassXcBackend {
    /// Creates a new `KeePassXC` backend
    ///
    /// # Arguments
    /// * `client_id` - A unique identifier for this client
    ///
    /// # Returns
    /// A new `KeePassXcBackend` instance
    #[must_use]
    pub fn new(client_id: impl Into<String>) -> Self {
        let socket_path = Self::default_socket_path();
        Self {
            socket_path,
            client_id: client_id.into(),
            associated: false,
        }
    }

    /// Creates a new `KeePassXC` backend with a custom socket path
    ///
    /// # Arguments
    /// * `client_id` - A unique identifier for this client
    /// * `socket_path` - Path to the `KeePassXC` socket
    ///
    /// # Returns
    /// A new `KeePassXcBackend` instance
    #[must_use]
    pub fn with_socket_path(client_id: impl Into<String>, socket_path: PathBuf) -> Self {
        Self {
            socket_path,
            client_id: client_id.into(),
            associated: false,
        }
    }

    /// Returns the default socket path for `KeePassXC`
    fn default_socket_path() -> PathBuf {
        // KeePassXC uses XDG_RUNTIME_DIR for the socket
        std::env::var("XDG_RUNTIME_DIR").map_or_else(
            |_| PathBuf::from("/tmp").join(format!("kpxc_server_{}", std::process::id())),
            |runtime_dir| PathBuf::from(runtime_dir).join("kpxc_server"),
        )
    }

    /// Connects to the `KeePassXC` socket
    async fn connect(&self) -> SecretResult<UnixStream> {
        UnixStream::connect(&self.socket_path)
            .await
            .map_err(|e| SecretError::KeePassXC(format!("Failed to connect to socket: {e}")))
    }

    /// Sends a request and receives a response
    async fn send_request(&self, request: &KeePassXcRequest) -> SecretResult<KeePassXcResponse> {
        let mut stream = self.connect().await?;

        // Serialize request
        let request_json = serde_json::to_string(request)
            .map_err(|e| SecretError::KeePassXC(format!("Failed to serialize request: {e}")))?;

        // Send length-prefixed message (native messaging format)
        #[allow(clippy::cast_possible_truncation)]
        let len = request_json.len() as u32;
        stream
            .write_all(&len.to_ne_bytes())
            .await
            .map_err(|e| SecretError::KeePassXC(format!("Failed to write length: {e}")))?;
        stream
            .write_all(request_json.as_bytes())
            .await
            .map_err(|e| SecretError::KeePassXC(format!("Failed to write request: {e}")))?;

        // Read response length
        let mut len_buf = [0u8; 4];
        stream
            .read_exact(&mut len_buf)
            .await
            .map_err(|e| SecretError::KeePassXC(format!("Failed to read response length: {e}")))?;
        let response_len = u32::from_ne_bytes(len_buf) as usize;

        // Read response
        let mut response_buf = vec![0u8; response_len];
        stream
            .read_exact(&mut response_buf)
            .await
            .map_err(|e| SecretError::KeePassXC(format!("Failed to read response: {e}")))?;

        // Parse response
        let response: KeePassXcResponse = serde_json::from_slice(&response_buf)
            .map_err(|e| SecretError::KeePassXC(format!("Failed to parse response: {e}")))?;

        // Check for errors
        if let Some(error) = &response.error {
            return Err(SecretError::KeePassXC(error.clone()));
        }

        Ok(response)
    }

    /// Generates a URL for a connection ID (used as lookup key)
    fn connection_url(connection_id: &str) -> String {
        format!("rustconn://{connection_id}")
    }

    /// Associates with `KeePassXC` if not already associated
    async fn ensure_associated(&self) -> SecretResult<()> {
        if self.associated {
            return Ok(());
        }

        let request = KeePassXcRequest {
            action: "test-associate".to_string(),
            id: Some(self.client_id.clone()),
            url: None,
            login: None,
            password: None,
            group: None,
            uuid: None,
        };

        let response = self.send_request(&request).await?;

        if response.success.as_deref() != Some("true") {
            // Need to associate
            let assoc_request = KeePassXcRequest {
                action: "associate".to_string(),
                id: Some(self.client_id.clone()),
                url: None,
                login: None,
                password: None,
                group: None,
                uuid: None,
            };

            let assoc_response = self.send_request(&assoc_request).await?;
            if assoc_response.success.as_deref() != Some("true") {
                return Err(SecretError::KeePassXC(
                    "Failed to associate with KeePassXC".to_string(),
                ));
            }
        }

        Ok(())
    }
}

#[async_trait]
impl SecretBackend for KeePassXcBackend {
    async fn store(&self, connection_id: &str, credentials: &Credentials) -> SecretResult<()> {
        self.ensure_associated().await?;

        let url = Self::connection_url(connection_id);
        let login = credentials.username.clone().unwrap_or_default();
        let password = credentials
            .expose_password()
            .unwrap_or_default()
            .to_string();

        let request = KeePassXcRequest {
            action: "set-login".to_string(),
            id: Some(self.client_id.clone()),
            url: Some(url),
            login: Some(login),
            password: Some(password),
            group: Some("RustConn".to_string()),
            uuid: None,
        };

        let response = self.send_request(&request).await?;

        if response.success.as_deref() != Some("true") {
            return Err(SecretError::StoreFailed(
                "KeePassXC did not confirm storage".to_string(),
            ));
        }

        Ok(())
    }

    async fn retrieve(&self, connection_id: &str) -> SecretResult<Option<Credentials>> {
        self.ensure_associated().await?;

        let url = Self::connection_url(connection_id);

        let request = KeePassXcRequest {
            action: "get-logins".to_string(),
            id: Some(self.client_id.clone()),
            url: Some(url),
            login: None,
            password: None,
            group: None,
            uuid: None,
        };

        let response = self.send_request(&request).await?;

        if let Some(entries) = response.entries {
            if let Some(entry) = entries.into_iter().next() {
                let credentials = Credentials {
                    username: entry.login,
                    password: entry.password.map(SecretString::from),
                    key_passphrase: None,
                };
                return Ok(Some(credentials));
            }
        }

        Ok(None)
    }

    async fn delete(&self, connection_id: &str) -> SecretResult<()> {
        // KeePassXC browser protocol doesn't support deletion directly
        // We would need to use a different approach or mark as deleted
        // For now, we'll return an error indicating this limitation
        Err(SecretError::KeePassXC(format!(
            "KeePassXC browser protocol does not support credential deletion for {connection_id}. \
             Please delete manually in KeePassXC."
        )))
    }

    async fn is_available(&self) -> bool {
        // Check if socket exists and we can connect
        if !self.socket_path.exists() {
            return false;
        }

        // Try to connect
        self.connect().await.is_ok()
    }

    fn backend_id(&self) -> &'static str {
        "keepassxc"
    }

    fn display_name(&self) -> &'static str {
        "KeePassXC"
    }
}
