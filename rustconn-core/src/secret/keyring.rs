//! Shared keyring storage via `secret-tool` (libsecret Secret Service API)
//!
//! Provides generic store/retrieve/delete operations for all secret backends
//! that need system keyring integration (GNOME Keyring, KDE Wallet, etc.).

use std::process::Stdio;
use tokio::process::Command;

use crate::error::{SecretError, SecretResult};

/// Application identifier used as the `application` attribute in keyring entries
const APP_ID: &str = "rustconn";

/// Checks whether `secret-tool` binary is available on the system.
///
/// All keyring operations depend on this tool. If it is missing,
/// callers should fall back to encrypted-settings storage and
/// inform the user to install `libsecret-tools`.
pub async fn is_secret_tool_available() -> bool {
    Command::new("secret-tool")
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Stores a value in the system keyring.
///
/// # Errors
/// Returns `SecretError::BackendUnavailable` if `secret-tool` is not installed.
/// Returns `SecretError::LibSecret` if `secret-tool` cannot be spawned
/// or the store command fails.
pub async fn store(key: &str, value: &str, label: &str) -> SecretResult<()> {
    use tokio::io::AsyncWriteExt;

    if !is_secret_tool_available().await {
        return Err(SecretError::BackendUnavailable(
            "secret-tool not found. Install libsecret-tools or use \
             encrypted settings storage."
                .into(),
        ));
    }

    let mut child = Command::new("secret-tool")
        .args(["store", "--label", label, "application", APP_ID, "key", key])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| SecretError::LibSecret(format!("Failed to spawn secret-tool: {e}")))?;

    if let Some(mut stdin) = child.stdin.take() {
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

/// Retrieves a value from the system keyring.
///
/// Returns `Ok(None)` when the key does not exist.
///
/// # Errors
/// Returns `SecretError::LibSecret` if `secret-tool` cannot be spawned.
pub async fn lookup(key: &str) -> SecretResult<Option<String>> {
    let output = Command::new("secret-tool")
        .args(["lookup", "application", APP_ID, "key", key])
        .output()
        .await
        .map_err(|e| SecretError::LibSecret(format!("Failed to run secret-tool: {e}")))?;

    if !output.status.success() {
        return Ok(None);
    }

    let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if value.is_empty() {
        Ok(None)
    } else {
        Ok(Some(value))
    }
}

/// Deletes a value from the system keyring.
///
/// # Errors
/// Returns `SecretError::DeleteFailed` if the clear command fails.
pub async fn clear(key: &str) -> SecretResult<()> {
    let output = Command::new("secret-tool")
        .args(["clear", "application", APP_ID, "key", key])
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
