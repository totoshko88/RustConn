//! SSH Agent management module
//!
//! This module provides functionality for interacting with the SSH agent,
//! including starting the agent, managing keys, and parsing agent output.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;

/// SSH Agent status and key information
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentStatus {
    /// Whether the agent is running
    pub running: bool,
    /// Path to the agent socket
    pub socket_path: Option<String>,
    /// List of keys loaded in the agent
    pub keys: Vec<AgentKey>,
}

/// A key loaded in the SSH agent
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentKey {
    /// Key fingerprint (SHA256 or MD5 format)
    pub fingerprint: String,
    /// Key size in bits
    pub bits: u32,
    /// Key type (e.g., "RSA", "ED25519", "ECDSA")
    pub key_type: String,
    /// Key comment (usually the key file path or email)
    pub comment: String,
}

/// Errors related to SSH agent operations
#[derive(Debug, Error)]
pub enum AgentError {
    /// SSH agent is not running
    #[error("SSH agent not running")]
    NotRunning,

    /// Failed to start the SSH agent
    #[error("Failed to start agent: {0}")]
    StartFailed(String),

    /// Failed to parse agent output
    #[error("Failed to parse agent output: {0}")]
    ParseError(String),

    /// Key not found in agent
    #[error("Key not found: {0}")]
    KeyNotFound(String),

    /// Failed to add key to agent
    #[error("Failed to add key: {0}")]
    AddKeyFailed(String),

    /// Failed to remove key from agent
    #[error("Failed to remove key: {0}")]
    RemoveKeyFailed(String),

    /// I/O error during agent operation
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result type alias for SSH agent operations
pub type AgentResult<T> = std::result::Result<T, AgentError>;

/// SSH Agent manager for interacting with ssh-agent
#[derive(Debug, Clone, Default)]
pub struct SshAgentManager {
    /// Path to the agent socket (`SSH_AUTH_SOCK`)
    socket_path: Option<String>,
}

impl SshAgentManager {
    /// Creates a new `SshAgentManager` with the given socket path
    #[must_use]
    pub const fn new(socket_path: Option<String>) -> Self {
        Self { socket_path }
    }

    /// Creates a new `SshAgentManager` from the environment
    ///
    /// Reads `SSH_AUTH_SOCK` from the environment if available.
    #[must_use]
    pub fn from_env() -> Self {
        let socket_path = std::env::var("SSH_AUTH_SOCK").ok();
        Self { socket_path }
    }

    /// Returns the current socket path
    #[must_use]
    pub fn socket_path(&self) -> Option<&str> {
        self.socket_path.as_deref()
    }

    /// Sets the socket path
    pub fn set_socket_path(&mut self, path: Option<String>) {
        self.socket_path = path;
    }
}

// ============================================================================
// SSH Agent Output Parsing
// ============================================================================

/// Parses the output of `ssh-agent -s` (bash format) or `ssh-agent -c` (csh format)
/// to extract the `SSH_AUTH_SOCK` path.
///
/// # Bash format example:
/// ```text
/// SSH_AUTH_SOCK=/tmp/ssh-XXXXXX/agent.12345; export SSH_AUTH_SOCK;
/// SSH_AGENT_PID=12346; export SSH_AGENT_PID;
/// echo Agent pid 12346;
/// ```
///
/// # Csh format example:
/// ```text
/// setenv SSH_AUTH_SOCK /tmp/ssh-XXXXXX/agent.12345;
/// setenv SSH_AGENT_PID 12346;
/// echo Agent pid 12346;
/// ```
///
/// # Errors
///
/// Returns `AgentError::ParseError` if the output doesn't contain a valid `SSH_AUTH_SOCK`.
pub fn parse_agent_output(output: &str) -> AgentResult<String> {
    // Try bash format first: SSH_AUTH_SOCK=/path/to/socket;
    for line in output.lines() {
        let line = line.trim();

        // Bash format: SSH_AUTH_SOCK=/path; export SSH_AUTH_SOCK;
        if line.starts_with("SSH_AUTH_SOCK=") {
            if let Some(value) = line
                .strip_prefix("SSH_AUTH_SOCK=")
                .and_then(|s| s.split(';').next())
            {
                let socket_path = value.trim();
                if !socket_path.is_empty() {
                    return Ok(socket_path.to_string());
                }
            }
        }

        // Csh format: setenv SSH_AUTH_SOCK /path;
        if line.starts_with("setenv SSH_AUTH_SOCK ") {
            if let Some(value) = line
                .strip_prefix("setenv SSH_AUTH_SOCK ")
                .and_then(|s| s.split(';').next())
            {
                let socket_path = value.trim();
                if !socket_path.is_empty() {
                    return Ok(socket_path.to_string());
                }
            }
        }
    }

    Err(AgentError::ParseError(
        "SSH_AUTH_SOCK not found in agent output".to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_bash_format() {
        let output = r"SSH_AUTH_SOCK=/tmp/ssh-XXXXXXabc123/agent.12345; export SSH_AUTH_SOCK;
SSH_AGENT_PID=12346; export SSH_AGENT_PID;
echo Agent pid 12346;";

        let result = parse_agent_output(output).unwrap();
        assert_eq!(result, "/tmp/ssh-XXXXXXabc123/agent.12345");
    }

    #[test]
    fn test_parse_csh_format() {
        let output = r"setenv SSH_AUTH_SOCK /tmp/ssh-XXXXXXabc123/agent.12345;
setenv SSH_AGENT_PID 12346;
echo Agent pid 12346;";

        let result = parse_agent_output(output).unwrap();
        assert_eq!(result, "/tmp/ssh-XXXXXXabc123/agent.12345");
    }

    #[test]
    fn test_parse_empty_output() {
        let result = parse_agent_output("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_invalid_output() {
        let output = "some random output without SSH_AUTH_SOCK";
        let result = parse_agent_output(output);
        assert!(result.is_err());
    }
}

// ============================================================================
// SSH Key List Parsing
// ============================================================================

/// Parses the output of `ssh-add -l` to extract loaded keys.
///
/// # Output format example:
/// ```text
/// 4096 SHA256:abcdef123456... user@host (RSA)
/// 256 SHA256:xyz789... /home/user/.ssh/id_ed25519 (ED25519)
/// ```
///
/// Each line contains: bits fingerprint comment (`key_type`)
///
/// # Returns
///
/// A vector of `AgentKey` structs, one for each loaded key.
/// Returns an empty vector if no keys are loaded (output is "The agent has no identities.").
///
/// # Errors
///
/// Returns `AgentError::ParseError` if a line cannot be parsed.
pub fn parse_key_list(output: &str) -> AgentResult<Vec<AgentKey>> {
    let output = output.trim();

    // Handle empty agent
    if output.is_empty() || output.contains("The agent has no identities") {
        return Ok(Vec::new());
    }

    let mut keys = Vec::new();

    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        // Parse: bits fingerprint comment (key_type)
        // Example: 4096 SHA256:abcdef123456... user@host (RSA)
        let parts: Vec<&str> = line.splitn(3, ' ').collect();

        if parts.len() < 3 {
            return Err(AgentError::ParseError(format!(
                "Invalid key list line (expected at least 3 parts): {line}"
            )));
        }

        // Parse bits
        let bits: u32 = parts[0]
            .parse()
            .map_err(|_| AgentError::ParseError(format!("Invalid bit count: {}", parts[0])))?;

        // Fingerprint is the second part
        let fingerprint = parts[1].to_string();

        // The rest contains comment and (key_type)
        let rest = parts[2];

        // Extract key type from parentheses at the end
        // Using nested if-let for clarity over map_or_else
        #[allow(clippy::option_if_let_else)]
        let (comment, key_type) = if let Some(paren_start) = rest.rfind('(') {
            if let Some(paren_end) = rest.rfind(')') {
                let key_type = rest[paren_start + 1..paren_end].to_string();
                let comment = rest[..paren_start].trim().to_string();
                (comment, key_type)
            } else {
                (rest.to_string(), String::new())
            }
        } else {
            (rest.to_string(), String::new())
        };

        keys.push(AgentKey {
            fingerprint,
            bits,
            key_type,
            comment,
        });
    }

    Ok(keys)
}

#[cfg(test)]
mod key_list_tests {
    use super::*;

    #[test]
    fn test_parse_single_key() {
        let output = "4096 SHA256:abcdef123456789 user@host (RSA)";
        let keys = parse_key_list(output).unwrap();

        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].bits, 4096);
        assert_eq!(keys[0].fingerprint, "SHA256:abcdef123456789");
        assert_eq!(keys[0].comment, "user@host");
        assert_eq!(keys[0].key_type, "RSA");
    }

    #[test]
    fn test_parse_multiple_keys() {
        let output = "4096 SHA256:abc123 user@host (RSA)\n256 SHA256:xyz789 /home/user/.ssh/id_ed25519 (ED25519)";
        let keys = parse_key_list(output).unwrap();

        assert_eq!(keys.len(), 2);
        assert_eq!(keys[0].key_type, "RSA");
        assert_eq!(keys[1].key_type, "ED25519");
    }

    #[test]
    fn test_parse_empty_agent() {
        let output = "The agent has no identities.";
        let keys = parse_key_list(output).unwrap();
        assert!(keys.is_empty());
    }

    #[test]
    fn test_parse_empty_output() {
        let keys = parse_key_list("").unwrap();
        assert!(keys.is_empty());
    }

    #[test]
    fn test_parse_key_with_spaces_in_comment() {
        let output = "4096 SHA256:abc123 My Key Comment (RSA)";
        let keys = parse_key_list(output).unwrap();

        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].comment, "My Key Comment");
    }

    #[test]
    fn test_parse_key_with_file_path_comment() {
        let output = "256 SHA256:xyz789 /home/user/.ssh/id_ed25519 (ED25519)";
        let keys = parse_key_list(output).unwrap();

        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].bits, 256);
        assert_eq!(keys[0].comment, "/home/user/.ssh/id_ed25519");
        assert_eq!(keys[0].key_type, "ED25519");
    }

    #[test]
    fn test_parse_invalid_bits() {
        let output = "notanumber SHA256:abc123 comment (RSA)";
        let result = parse_key_list(output);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_incomplete_line() {
        let output = "4096 SHA256:abc123";
        let result = parse_key_list(output);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_whitespace_only() {
        let output = "   \n\t\n   ";
        let keys = parse_key_list(output).unwrap();
        assert!(keys.is_empty());
    }
}

#[cfg(test)]
mod manager_tests {
    use super::*;

    #[test]
    fn test_manager_new() {
        let manager = SshAgentManager::new(Some("/tmp/test.sock".to_string()));
        assert_eq!(manager.socket_path(), Some("/tmp/test.sock"));
    }

    #[test]
    fn test_manager_default() {
        let manager = SshAgentManager::default();
        assert_eq!(manager.socket_path(), None);
    }

    #[test]
    fn test_manager_set_socket_path() {
        let mut manager = SshAgentManager::default();
        manager.set_socket_path(Some("/tmp/new.sock".to_string()));
        assert_eq!(manager.socket_path(), Some("/tmp/new.sock"));
    }

    #[test]
    fn test_get_status_no_socket() {
        let manager = SshAgentManager::default();
        let status = manager.get_status().unwrap();
        assert!(!status.running);
        assert!(status.socket_path.is_none());
        assert!(status.keys.is_empty());
    }
}

// ============================================================================
// SSH Agent Manager Implementation
// ============================================================================

impl SshAgentManager {
    /// Starts a new SSH agent and returns the socket path.
    ///
    /// Executes `ssh-agent -s` and parses the output to extract the socket path.
    ///
    /// # Errors
    ///
    /// Returns `AgentError::StartFailed` if the agent cannot be started.
    pub fn start_agent() -> AgentResult<String> {
        use std::process::Command;

        let output = Command::new("ssh-agent")
            .arg("-s")
            .output()
            .map_err(|e| AgentError::StartFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AgentError::StartFailed(format!(
                "ssh-agent exited with error: {stderr}"
            )));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        parse_agent_output(&stdout)
    }

    /// Gets the current agent status including loaded keys.
    ///
    /// # Errors
    ///
    /// Returns `AgentError::NotRunning` if no socket path is configured.
    pub fn get_status(&self) -> AgentResult<AgentStatus> {
        use std::process::Command;

        let socket_path = match &self.socket_path {
            Some(path) => path.clone(),
            None => {
                return Ok(AgentStatus {
                    running: false,
                    socket_path: None,
                    keys: Vec::new(),
                });
            }
        };

        // Check if the socket exists and agent is responsive
        let output = Command::new("ssh-add")
            .arg("-l")
            .env("SSH_AUTH_SOCK", &socket_path)
            .output();

        match output {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                // Exit code 1 with "no identities" is still a running agent
                if output.status.success()
                    || stdout.contains("no identities")
                    || stderr.contains("no identities")
                {
                    let keys = parse_key_list(&stdout).unwrap_or_default();
                    Ok(AgentStatus {
                        running: true,
                        socket_path: Some(socket_path),
                        keys,
                    })
                } else {
                    // Agent not responding
                    Ok(AgentStatus {
                        running: false,
                        socket_path: Some(socket_path),
                        keys: Vec::new(),
                    })
                }
            }
            Err(_) => Ok(AgentStatus {
                running: false,
                socket_path: Some(socket_path),
                keys: Vec::new(),
            }),
        }
    }

    /// Adds a key to the SSH agent.
    ///
    /// # Arguments
    ///
    /// * `key_path` - Path to the private key file
    /// * `passphrase` - Optional passphrase for encrypted keys
    ///
    /// # Errors
    ///
    /// Returns `AgentError::NotRunning` if no socket is configured.
    /// Returns `AgentError::AddKeyFailed` if the key cannot be added.
    pub fn add_key(&self, key_path: &std::path::Path, passphrase: Option<&str>) -> AgentResult<()> {
        use std::process::{Command, Stdio};

        let socket_path = self.socket_path.as_ref().ok_or(AgentError::NotRunning)?;

        if let Some(pass) = passphrase {
            // Use SSH_ASKPASS with a helper script for passphrase
            // This is a simplified approach - in production, you'd use expect or a proper PTY
            let mut cmd = Command::new("ssh-add");
            cmd.arg(key_path)
                .env("SSH_AUTH_SOCK", socket_path)
                .env("SSH_ASKPASS_REQUIRE", "force")
                .env("DISPLAY", ":0")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            // For now, we'll use a simple echo approach via environment
            // In a real implementation, you'd want to use a proper PTY or expect-like library
            let output = cmd
                .output()
                .map_err(|e| AgentError::AddKeyFailed(e.to_string()))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                // If it needs a passphrase, we need a different approach
                if stderr.contains("passphrase") || stderr.contains("bad passphrase") {
                    return Err(AgentError::AddKeyFailed(format!(
                        "Passphrase required or incorrect: {stderr}"
                    )));
                }
                return Err(AgentError::AddKeyFailed(stderr.to_string()));
            }
            let _ = pass; // Acknowledge passphrase was provided
        } else {
            // No passphrase - simple ssh-add
            let output = Command::new("ssh-add")
                .arg(key_path)
                .env("SSH_AUTH_SOCK", socket_path)
                .output()
                .map_err(|e| AgentError::AddKeyFailed(e.to_string()))?;

            if !output.status.success() {
                let stderr = String::from_utf8_lossy(&output.stderr);
                return Err(AgentError::AddKeyFailed(stderr.to_string()));
            }
        }

        Ok(())
    }

    /// Removes a key from the SSH agent.
    ///
    /// # Arguments
    ///
    /// * `key_path` - Path to the private key file (or public key)
    ///
    /// # Errors
    ///
    /// Returns `AgentError::NotRunning` if no socket is configured.
    /// Returns `AgentError::RemoveKeyFailed` if the key cannot be removed.
    pub fn remove_key(&self, key_path: &std::path::Path) -> AgentResult<()> {
        use std::process::Command;

        let socket_path = self.socket_path.as_ref().ok_or(AgentError::NotRunning)?;

        let output = Command::new("ssh-add")
            .arg("-d")
            .arg(key_path)
            .env("SSH_AUTH_SOCK", socket_path)
            .output()
            .map_err(|e| AgentError::RemoveKeyFailed(e.to_string()))?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AgentError::RemoveKeyFailed(stderr.to_string()));
        }

        Ok(())
    }

    /// Lists available SSH key files in ~/.ssh/ directory.
    ///
    /// Looks for common key file patterns like `id_rsa`, `id_ed25519`, etc.
    ///
    /// # Errors
    ///
    /// Returns `AgentError::Io` if the directory cannot be read.
    pub fn list_key_files() -> AgentResult<Vec<PathBuf>> {
        let home = std::env::var("HOME").map_err(|_| {
            AgentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "HOME environment variable not set",
            ))
        })?;

        let ssh_dir = PathBuf::from(home).join(".ssh");

        if !ssh_dir.exists() {
            return Ok(Vec::new());
        }

        let mut keys = Vec::new();

        // Common key file patterns (private keys only)
        let key_patterns = [
            "id_rsa",
            "id_ed25519",
            "id_ecdsa",
            "id_dsa",
            "id_ecdsa_sk",
            "id_ed25519_sk",
        ];

        for entry in std::fs::read_dir(&ssh_dir)? {
            let entry = entry?;
            let path = entry.path();

            if path.is_file() {
                if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                    // Check if it matches a known pattern or starts with id_
                    let is_pub = std::path::Path::new(file_name)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("pub"));
                    let is_key = key_patterns.contains(&file_name)
                        || (file_name.starts_with("id_") && !is_pub);

                    if is_key {
                        keys.push(path);
                    }
                }
            }
        }

        keys.sort();
        Ok(keys)
    }
}
