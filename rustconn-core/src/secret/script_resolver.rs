//! Script-based credential resolver
//!
//! Executes an external command to retrieve a password.
//! The command string is split into program + arguments via `shell_words::split()`
//! and executed directly (no shell) with a 30-second timeout.

use std::time::Duration;

use secrecy::SecretString;
use tracing::{debug, warn};
use zeroize::Zeroize;

use crate::error::{SecretError, SecretResult};
use crate::models::Credentials;

/// Timeout for script execution (30 seconds).
const SCRIPT_TIMEOUT: Duration = Duration::from_secs(30);

/// Resolve credentials by executing an external command.
///
/// The command is split into program + args via `shell_words::split()`,
/// then executed via `tokio::process::Command` (no shell).
///
/// - stdout is trimmed and wrapped in `SecretString`
/// - Non-zero exit → `SecretError::RetrieveFailed` with stderr
/// - Timeout → `SecretError::RetrieveFailed` with message
/// - The raw stdout buffer is zeroed after wrapping in `SecretString`
///
/// # Errors
///
/// Returns `SecretError::RetrieveFailed` when:
/// - The command string cannot be parsed
/// - The command is empty
/// - The child process cannot be spawned
/// - The script exits with a non-zero code
/// - The script exceeds the 30-second timeout
pub async fn resolve_script(command: &str) -> SecretResult<Option<Credentials>> {
    let parts = shell_words::split(command)
        .map_err(|e| SecretError::RetrieveFailed(format!("Failed to parse script command: {e}")))?;

    if parts.is_empty() {
        return Err(SecretError::RetrieveFailed(
            "Script command is empty".to_string(),
        ));
    }

    let program = &parts[0];
    let args = &parts[1..];

    // Log only the program, never the full command line: a credential script's
    // arguments can themselves carry a secret (e.g. `vault … -token=<secret>`),
    // and the whole point of this resolver is that the *output* is a secret. The
    // program name alone is enough to diagnose a misconfiguration.
    debug!(program = %program, "Resolving credentials via script");

    let child = tokio::process::Command::new(program)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .map_err(|e| {
            SecretError::RetrieveFailed(format!("Failed to spawn script '{program}': {e}"))
        })?;

    let output = match tokio::time::timeout(SCRIPT_TIMEOUT, child.wait_with_output()).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Err(SecretError::RetrieveFailed(format!(
                "Script execution failed: {e}"
            )));
        }
        Err(_) => {
            warn!(program = %program, "Script timed out after 30 seconds");
            return Err(SecretError::RetrieveFailed(
                "Script timed out after 30 seconds".to_string(),
            ));
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(SecretError::RetrieveFailed(format!(
            "Script exited with {}: {}",
            output.status,
            stderr.trim()
        )));
    }

    // Read stdout, trim, wrap in SecretString, then clear buffers
    let mut raw = output.stdout;
    let mut trimmed = String::from_utf8_lossy(&raw).trim().to_string();

    // Zero out the raw buffer for security
    raw.fill(0);

    if trimmed.is_empty() {
        debug!("Script returned empty output");
        return Ok(None);
    }

    let password = SecretString::from(trimmed.as_str());
    trimmed.zeroize();

    Ok(Some(Credentials {
        username: None,
        password: Some(password),
        key_passphrase: None,
        domain: None,
    }))
}
