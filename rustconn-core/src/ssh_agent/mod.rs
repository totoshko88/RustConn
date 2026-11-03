//! SSH Agent management module
//!
//! This module provides functionality for interacting with the SSH agent,
//! including starting the agent, managing keys, and parsing agent output.

mod keyfile;
mod own_keys;
#[cfg(unix)]
mod wire;

#[cfg(test)]
pub(crate) use keyfile::openssh_key_fixture;
pub use keyfile::{
    KeyEncryption, KeyFileInfo, KeyFormat, PublicKeyInfo, inspect_key_file, openssh_public_key_line,
};
pub use own_keys::{OWN_KEY_LIFETIME, evict_own_keys};

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
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
        if line.starts_with("SSH_AUTH_SOCK=")
            && let Some(value) = line
                .strip_prefix("SSH_AUTH_SOCK=")
                .and_then(|s| s.split(';').next())
        {
            let socket_path = value.trim();
            if !socket_path.is_empty() {
                return Ok(socket_path.to_string());
            }
        }

        // Csh format: setenv SSH_AUTH_SOCK /path;
        if line.starts_with("setenv SSH_AUTH_SOCK ")
            && let Some(value) = line
                .strip_prefix("setenv SSH_AUTH_SOCK ")
                .and_then(|s| s.split(';').next())
        {
            let socket_path = value.trim();
            if !socket_path.is_empty() {
                return Ok(socket_path.to_string());
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
// Agent Identity Files
// ============================================================================

/// Turns a key fingerprint into a filename-safe stem.
///
/// SHA256 fingerprints are base64 and contain `:`, `/` and `+`, none of which
/// belong in a path component. Every character outside `[A-Za-z0-9._-]` is
/// replaced by `_`.
#[must_use]
pub fn sanitize_fingerprint(fingerprint: &str) -> String {
    fingerprint
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
mod identity_file_tests {
    use super::*;

    #[test]
    fn test_sanitize_sha256_fingerprint() {
        let sanitized = sanitize_fingerprint("SHA256:ab/cd+ef=gh");
        assert_eq!(sanitized, "SHA256_ab_cd_ef_gh");
    }

    #[test]
    fn test_sanitize_keeps_safe_characters() {
        assert_eq!(sanitize_fingerprint("a.b-c_1"), "a.b-c_1");
    }

    #[test]
    fn test_sanitize_never_yields_path_separators() {
        let sanitized = sanitize_fingerprint("../../etc/passwd");
        assert!(!sanitized.contains('/'));
        assert!(!sanitized.contains(std::path::MAIN_SEPARATOR));
    }

    #[test]
    fn test_materialize_requires_socket() {
        let manager = SshAgentManager::new(None);
        let result = manager.materialize_agent_identity("SHA256:whatever");
        assert!(matches!(result, Err(AgentError::NotRunning)));
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
        #[expect(
            clippy::option_if_let_else,
            reason = "if-let form reads more naturally than map_or_else for the side-effecting else branch"
        )]
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

    /// A key RustConn adds on its own carries the 12 h lifetime ahead of the
    /// key path; the user's own add carries none.
    #[test]
    fn own_key_add_carries_the_twelve_hour_lifetime() {
        let key = std::path::Path::new("/home/u/.ssh/id_ed25519");
        let args = |cmd: std::process::Command| {
            cmd.get_args()
                .map(|a| a.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            args(own_ssh_add_command(key, "/tmp/agent.sock")),
            vec!["-t", "43200", "/home/u/.ssh/id_ed25519"]
        );
        assert_eq!(
            args(ssh_add_command(key, "/tmp/agent.sock", None)),
            vec!["/home/u/.ssh/id_ed25519"]
        );
    }

    /// The own-key `ssh-add` writes to the socket the agent was asked about,
    /// never the inherited one, and cannot open the host askpass.
    #[test]
    fn own_key_add_targets_the_given_socket_without_askpass() {
        let cmd = own_ssh_add_command(
            std::path::Path::new("/home/u/.ssh/id_ed25519"),
            "/run/user/1000/rustconn-test/agent.sock",
        );
        let env = |name: &str| {
            cmd.get_envs()
                .find(|(k, _)| *k == name)
                .map(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
        };
        assert_eq!(
            env("SSH_AUTH_SOCK"),
            Some(Some("/run/user/1000/rustconn-test/agent.sock".to_owned()))
        );
        assert_eq!(env("SSH_ASKPASS"), Some(None), "SSH_ASKPASS is removed");
    }

    #[test]
    fn add_own_key_requires_socket() {
        let manager = SshAgentManager::new(None);
        let result = manager.add_own_key(std::path::Path::new("/nonexistent/id_ed25519"));
        assert!(matches!(result, Err(AgentError::NotRunning)));
    }

    /// `add_key` used to put the passphrase itself in the `ssh-add`
    /// environment, readable in `/proc/<pid>/environ`. Now only the paths of
    /// the askpass helper and of the owner-only secret file are there, and the
    /// helper still prints the passphrase exactly.
    #[cfg(unix)]
    #[test]
    fn add_key_askpass_env_carries_paths_not_the_passphrase() {
        use crate::ssh_tunnel::{
            TUNNEL_ASKPASS_ENV_VAR, cleanup_askpass_script, create_one_shot_askpass_script,
            create_tunnel_secret_file,
        };

        let passphrase = secrecy::SecretString::from(" hunter2 agent ");
        let script = create_one_shot_askpass_script().unwrap();
        let secret = create_tunnel_secret_file(&passphrase).unwrap();

        let cmd = ssh_add_askpass_command(
            std::path::Path::new("/home/u/.ssh/id_ed25519"),
            "/tmp/agent.sock",
            &script,
            &secret,
        );
        let envs: Vec<(String, String)> = cmd
            .get_envs()
            .filter_map(|(k, v)| Some((k.to_str()?.to_owned(), v?.to_str()?.to_owned())))
            .collect();
        // Through `sh`, so a `noexec` temp dir does not fail the test.
        let ask = || {
            std::process::Command::new("sh")
                .arg(&script)
                .env(TUNNEL_ASKPASS_ENV_VAR, &secret)
                .output()
                .unwrap()
        };
        let printed = ask();
        // ssh-add re-asks after a wrong passphrase until the answer is empty;
        // the helper answers once, so that loop ends.
        let asked_again = ask();
        let left_in_file = std::fs::metadata(&secret).unwrap().len();
        cleanup_askpass_script(&secret);
        cleanup_askpass_script(&script);

        assert!(
            envs.iter().all(|(_, v)| !v.contains("hunter2")),
            "passphrase reached the ssh-add environment"
        );
        assert!(envs.iter().all(|(k, _)| k != "RUSTCONN_ASKPASS_SECRET"));
        assert!(envs.contains(&(
            TUNNEL_ASKPASS_ENV_VAR.to_owned(),
            secret.to_string_lossy().into_owned()
        )));
        assert!(envs.contains(&("SSH_ASKPASS_REQUIRE".to_owned(), "force".to_owned())));
        assert!(printed.status.success());
        assert_eq!(printed.stdout, b" hunter2 agent ");
        assert!(!asked_again.status.success(), "a second ask must fail");
        assert!(
            asked_again.stdout.is_empty(),
            "a second ask must print nothing"
        );
        assert_eq!(
            left_in_file, 0,
            "the passphrase must leave the file once read"
        );
        assert!(!secret.exists(), "secret file must be removed");
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
    /// This calls `ssh-add -l` synchronously. On macOS, if the system ssh-agent
    /// is broken or launchd-throttled, this can block indefinitely. Prefer
    /// [`Self::get_status_timeout`] when calling from a UI thread.
    ///
    /// # Errors
    ///
    /// Returns `AgentError::NotRunning` if no socket path is configured.
    pub fn get_status(&self) -> AgentResult<AgentStatus> {
        self.get_status_inner(None)
    }

    /// The agent's identities over the native wire protocol, or `None` to make
    /// the caller fall back to `ssh-add -l`.
    ///
    /// `Some(status)` is a complete answer with `running: true`; `None` means
    /// the native client could not talk to this agent (an unexpected socket, a
    /// non-standard implementation, an I/O timeout) and the shell-out path
    /// should be tried instead, so behaviour is never worse than before (A1).
    #[cfg(unix)]
    fn native_status(socket_path: &str) -> Option<AgentStatus> {
        match wire::request_identities(socket_path) {
            Ok(keys) => Some(AgentStatus {
                running: true,
                socket_path: Some(socket_path.to_string()),
                keys,
            }),
            Err(e) => {
                tracing::debug!(
                    error = %e,
                    "native ssh-agent query failed; falling back to ssh-add -l"
                );
                None
            }
        }
    }

    /// Gets the current agent status with a timeout.
    ///
    /// Spawns `ssh-add -l` as a child process and waits up to `timeout` for it
    /// to complete. If the process does not finish in time it is killed and the
    /// agent is reported as not running. This prevents the caller from hanging
    /// when the system ssh-agent is broken or launchd-throttled (common on
    /// macOS).
    ///
    /// # Errors
    ///
    /// Returns `AgentError::NotRunning` if no socket path is configured.
    pub fn get_status_timeout(&self, timeout: std::time::Duration) -> AgentResult<AgentStatus> {
        self.get_status_inner(Some(timeout))
    }

    /// Internal implementation shared by `get_status` and `get_status_timeout`.
    fn get_status_inner(&self, timeout: Option<std::time::Duration>) -> AgentResult<AgentStatus> {
        use std::process::{Command, Stdio};

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

        // Native fast path (A1): one Unix-socket round trip, no process and no
        // per-key `ssh-keygen`, computing each fingerprint with `ring`. On any
        // failure it falls through to the `ssh-add -l` spawn below, so an agent
        // the native client cannot talk to behaves exactly as before.
        #[cfg(unix)]
        if let Some(status) = Self::native_status(&socket_path) {
            return Ok(status);
        }

        // Spawn the child process so we can apply a timeout.
        let child_result = Command::new("ssh-add")
            .arg("-l")
            .env("SSH_AUTH_SOCK", &socket_path)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn();

        let child = match child_result {
            Ok(c) => c,
            Err(_) => {
                return Ok(AgentStatus {
                    running: false,
                    socket_path: Some(socket_path),
                    keys: Vec::new(),
                });
            }
        };

        let output = if let Some(dur) = timeout {
            // Wait with timeout using a helper thread + channel.
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            let child_id = child.id();
            std::thread::spawn(move || {
                let result = child.wait_with_output();
                // Send result; ignore error if receiver dropped (timeout).
                let _ = tx.send(result);
            });

            match rx.recv_timeout(dur) {
                Ok(Ok(out)) => Some(out),
                Ok(Err(_)) => None,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                    // Timeout expired — kill the child process.
                    #[expect(
                        clippy::cast_possible_truncation,
                        reason = "timeout duration is always small enough to fit u64 milliseconds"
                    )]
                    let timeout_ms = dur.as_millis() as u64;
                    tracing::warn!(
                        pid = child_id,
                        timeout_ms,
                        "ssh-add timed out, killing child process"
                    );
                    // Best-effort kill on Unix via SIGKILL.
                    #[cfg(unix)]
                    {
                        #[expect(
                            clippy::cast_possible_wrap,
                            reason = "PID values never exceed i32::MAX on any supported OS"
                        )]
                        let _ = nix::sys::signal::kill(
                            nix::unistd::Pid::from_raw(child_id as i32),
                            nix::sys::signal::Signal::SIGKILL,
                        );
                    }
                    None
                }
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => None,
            }
        } else {
            // No timeout — blocking wait (original behaviour).
            child.wait_with_output().ok()
        };

        match output {
            Some(output) => {
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
            None => Ok(AgentStatus {
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
    /// * `passphrase` - Optional passphrase for encrypted keys (as `SecretString`)
    ///
    /// # Where the passphrase is written
    ///
    /// A passphrase goes to `ssh-add`'s askpass helper through a mode-0600,
    /// `create_new`, randomly named file from `create_tunnel_secret_file`, the
    /// same rule as every askpass secret file (`crate::ssh_askpass`):
    /// `$XDG_RUNTIME_DIR` (tmpfs) on Linux, the per-user `$TMPDIR` on macOS,
    /// and `std::env::temp_dir()` (`/tmp`) only on a Linux session without a
    /// runtime dir. In the last two cases the file is on a real disk, where the
    /// zero-overwrite before unlinking does not reliably erase it (APFS, CoW
    /// filesystems). Accepted for now: refusing would break adding a key in
    /// su/sudo/container shells, the file is readable only by the same uid,
    /// and the one-shot helper empties it right after its single read.
    ///
    /// This is the user's own add (Settings → SSH Agent): no lifetime, and the
    /// key counts as the user's from now on, so RustConn will not remove it on
    /// exit, lock or suspend even if it had added the same key itself earlier.
    /// The passphrase is used for this one add and not kept.
    ///
    /// # Errors
    ///
    /// Returns `AgentError::NotRunning` if no socket is configured.
    /// Returns `AgentError::AddKeyFailed` if the key cannot be added.
    pub fn add_key(
        &self,
        key_path: &std::path::Path,
        passphrase: Option<&secrecy::SecretString>,
    ) -> AgentResult<()> {
        let socket_path = self.socket_path.as_ref().ok_or(AgentError::NotRunning)?;
        run_ssh_add(key_path, socket_path, passphrase)?;
        own_keys::forget(socket_path, key_path);
        Ok(())
    }

    /// Adds a key RustConn needs in the agent on its own (a multi-hop `-J`
    /// connect, an SFTP or mc launch), to be removed again on exit, screen
    /// lock and suspend.
    ///
    /// The agent at this manager's socket is asked which keys it holds before
    /// and after the add, and that one socket is also the one `ssh-add` writes
    /// to. A key it already holds is left exactly as it is: it may be the
    /// user's, and re-adding would replace its constraints. When that cannot
    /// be told (the agent does not answer, or the key's public half is
    /// unreadable and the agent holds other keys) nothing is added. Otherwise
    /// the key goes in with `ssh-add -t` [`OWN_KEY_LIFETIME`], and the
    /// identities that appeared in the agent are recorded for
    /// [`evict_own_keys`].
    ///
    /// No passphrase is taken and no askpass is offered: an encrypted key
    /// loads only if `ssh-add` can decrypt it without asking. The `ssh-add`
    /// run is bounded, so a caller on the GTK thread never waits on it for
    /// more than 30 s.
    ///
    /// # Errors
    ///
    /// Returns `AgentError::NotRunning` if no socket is configured.
    /// Returns `AgentError::AddKeyFailed` if the agent cannot be asked which
    /// keys it holds, if that question cannot settle whether it already holds
    /// this key, or if `ssh-add` fails or times out.
    pub fn add_own_key(&self, key_path: &std::path::Path) -> AgentResult<()> {
        let socket_path = self
            .socket_path
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or(AgentError::NotRunning)?;
        own_keys::add_and_record(socket_path, key_path, || {
            run_own_ssh_add(key_path, socket_path)
        })
    }
}

/// Runs the user's `ssh-add` for `key_path` against `socket_path`, with the
/// passphrase through the one-shot askpass when one is given. No lifetime.
fn run_ssh_add(
    key_path: &std::path::Path,
    socket_path: &str,
    passphrase: Option<&secrecy::SecretString>,
) -> AgentResult<()> {
    if let Some(pass) = passphrase {
        use crate::ssh_tunnel::{
            cleanup_askpass_script, create_one_shot_askpass_script, create_tunnel_secret_file,
        };

        // The passphrase reaches ssh-add's askpass helper through an
        // owner-only (0600) file in `$XDG_RUNTIME_DIR`; the environment
        // carries only that file's path. It used to travel as the value of
        // an env var, which the same user can read in `/proc/<pid>/environ`
        // for as long as ssh-add runs. The secret file is the SSH tunnel's;
        // the helper is its one-shot variant, because ssh-add asks again
        // after a wrong passphrase and only an empty answer stops it.
        let script_path = create_one_shot_askpass_script().map_err(AgentError::AddKeyFailed)?;
        // ponytail: temp_dir fallback kept by decision (K1, 0.24): one
        // resolver for every askpass secret file, `ssh_askpass::secret_dir`;
        // refusing would break su/sudo/container sessions, and a private
        // /tmp subdir adds a DoS vector and no secrecy over 0600 O_EXCL
        // random names. Follow-ups: tunnel scripts to the runtime dir,
        // atomic GUI script.
        let secret_path = match create_tunnel_secret_file(pass) {
            Ok(path) => path,
            Err(e) => {
                cleanup_askpass_script(&script_path);
                return Err(AgentError::AddKeyFailed(e));
            }
        };

        // Bounded, so the files below are always cleaned up and the caller
        // (the GTK thread) never waits on an ssh-add that does not finish.
        let output = wait_ssh_add(
            ssh_add_askpass_command(key_path, socket_path, &script_path, &secret_path),
            "ssh-add with askpass",
        );

        // The child has exited or been killed and reaped, so neither file
        // is needed any more. The secret file is zeroized before it is
        // unlinked.
        cleanup_askpass_script(&secret_path);
        cleanup_askpass_script(&script_path);

        let output = output?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("passphrase") || stderr.contains("bad passphrase") {
                return Err(AgentError::AddKeyFailed("Incorrect passphrase".to_string()));
            }
            return Err(AgentError::AddKeyFailed(stderr.to_string()));
        }
    } else {
        // No passphrase: the host askpass, if any, may still ask for one.
        let output = wait_ssh_add(ssh_add_command(key_path, socket_path, None), "ssh-add")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(AgentError::AddKeyFailed(stderr.to_string()));
        }
    }

    Ok(())
}

/// Runs the `ssh-add` of a key RustConn adds on its own
/// ([`own_ssh_add_command`]), bounded.
fn run_own_ssh_add(key_path: &std::path::Path, socket_path: &str) -> AgentResult<()> {
    let output = wait_ssh_add(own_ssh_add_command(key_path, socket_path), "ssh-add")?;
    if output.status.success() {
        tracing::info!(?key_path, "SSH key added to agent");
        Ok(())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(AgentError::AddKeyFailed(stderr))
    }
}

/// Spawns an `ssh-add` with no stdin and piped output, and waits for it at
/// most [`SSH_ADD_TIMEOUT`].
///
/// # Errors
///
/// Returns `AgentError::AddKeyFailed` if it cannot be spawned or waited for,
/// or did not finish in time (it has then been killed and reaped).
fn wait_ssh_add(
    mut cmd: std::process::Command,
    what: &'static str,
) -> AgentResult<std::process::Output> {
    use std::process::Stdio;

    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd.spawn()
        .and_then(|child| crate::proc::wait_bounded(child, SSH_ADD_TIMEOUT, what))
        .map_err(|e| AgentError::AddKeyFailed(e.to_string()))?
        .output()
        .ok_or_else(|| {
            AgentError::AddKeyFailed(format!(
                "ssh-add did not finish within {} s",
                SSH_ADD_TIMEOUT.as_secs()
            ))
        })
}

impl SshAgentManager {
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

    /// Lists available SSH key files in `~/.ssh/`.
    ///
    /// [`list_key_files_in`] against `$HOME/.ssh`.
    ///
    /// # Errors
    ///
    /// Returns `AgentError::Io` if `HOME` is not set or the directory cannot
    /// be read.
    pub fn list_key_files() -> AgentResult<Vec<PathBuf>> {
        let home = std::env::var("HOME").map_err(|_| {
            AgentError::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "HOME environment variable not set",
            ))
        })?;
        list_key_files_in(&PathBuf::from(home).join(".ssh"))
    }

    /// Gets the public key for a specific fingerprint from the SSH agent.
    ///
    /// Uses `ssh-add -L` to list all public keys and matches by fingerprint.
    /// This is useful when you need to specify a particular agent key for SSH connection.
    ///
    /// # Arguments
    ///
    /// * `fingerprint` - The key fingerprint to search for (e.g., "SHA256:abc123...")
    ///
    /// # Returns
    ///
    /// The public key in OpenSSH format (e.g., "ssh-ed25519 AAAA... comment")
    ///
    /// # Errors
    ///
    /// Returns `AgentError::NotRunning` if no socket is configured.
    /// Returns `AgentError::KeyNotFound` if no key matches the fingerprint.
    pub fn get_public_key_by_fingerprint(&self, fingerprint: &str) -> AgentResult<String> {
        use std::process::Command;

        let socket_path = self.socket_path.as_ref().ok_or(AgentError::NotRunning)?;

        // Get all public keys from agent
        let output = Command::new("ssh-add")
            .arg("-L")
            .env("SSH_AUTH_SOCK", socket_path)
            .output()
            .map_err(AgentError::Io)?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("no identities") || stderr.contains("The agent has no identities") {
                return Err(AgentError::KeyNotFound(fingerprint.to_string()));
            }
            return Err(AgentError::ParseError(stderr.to_string()));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);

        // For each public key line, compute its fingerprint and compare
        for line in stdout.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            // Compute fingerprint of this public key using ssh-keygen
            let keygen_output = Command::new("ssh-keygen")
                .arg("-lf")
                .arg("-")
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn();

            if let Ok(mut child) = keygen_output {
                use std::io::Write;
                if let Some(ref mut stdin) = child.stdin {
                    let _ = stdin.write_all(line.as_bytes());
                }

                if let Ok(output) = child.wait_with_output()
                    && output.status.success()
                {
                    let fp_line = String::from_utf8_lossy(&output.stdout);
                    // Output format: "256 SHA256:xxx comment (ED25519)"
                    // Extract fingerprint (second field)
                    let parts: Vec<&str> = fp_line.split_whitespace().collect();
                    if parts.len() >= 2 && parts[1] == fingerprint {
                        return Ok(line.to_string());
                    }
                }
            }
        }

        Err(AgentError::KeyNotFound(fingerprint.to_string()))
    }

    /// Writes the agent key with `fingerprint` to a public-key file for `ssh -i`.
    ///
    /// `ssh -i` accepts a `.pub` file: OpenSSH then asks the agent to sign with
    /// the matching private key instead of reading key material from disk.
    /// Combined with `-o IdentitiesOnly=yes` this restricts the connection to
    /// exactly the selected agent key. Unlike pointing `-i` at the *private*
    /// key, it produces a single agent confirmation prompt (issue #125).
    ///
    /// The file holds only public key material. It is written to
    /// `$XDG_RUNTIME_DIR/rustconn/agent-keys/` (mode 0600 inside a 0700
    /// directory), falling back to the system temp directory when the runtime
    /// directory is unavailable. Rewriting an existing file is intentional: the
    /// agent is the source of truth and the content is derived from it.
    ///
    /// # Errors
    /// Returns `AgentError::NotRunning` if no socket is configured,
    /// `AgentError::KeyNotFound` if the agent holds no key with that
    /// fingerprint, and `AgentError::Io` if the file cannot be written.
    pub fn materialize_agent_identity(&self, fingerprint: &str) -> AgentResult<PathBuf> {
        let public_key = self.get_public_key_by_fingerprint(fingerprint)?;

        // Shared resolver: `$XDG_RUNTIME_DIR` on Linux, `$TMPDIR` on macOS.
        // `prune_stale_agent_keys` resolves the same base, so both must stay in
        // step. The `temp_dir` fallback only applies on a Linux box with no
        // runtime dir; the file holds public-key material, so a weaker location
        // is acceptable and a directory must always be produced.
        let base = crate::secret_file_dir()
            .unwrap_or_else(std::env::temp_dir)
            .join("rustconn")
            .join("agent-keys");
        std::fs::create_dir_all(&base)?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&base, std::fs::Permissions::from_mode(0o700))?;
        }

        let path = base.join(format!("{}.pub", sanitize_fingerprint(fingerprint)));
        std::fs::write(&path, format!("{}\n", public_key.trim()))?;

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;
        }

        Ok(path)
    }
}

/// Checks if a file contains a private key by reading its header.
fn is_private_key_file(path: &PathBuf) -> bool {
    use std::io::{BufRead, BufReader};

    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };

    let reader = BufReader::new(file);
    let Some(Ok(first_line)) = reader.lines().next() else {
        return false;
    };

    // Check for common private key headers
    first_line.contains("PRIVATE KEY")
        || first_line.contains("OPENSSH PRIVATE KEY")
        || first_line.contains("RSA PRIVATE KEY")
        || first_line.contains("EC PRIVATE KEY")
        || first_line.contains("DSA PRIVATE KEY")
}

/// Lists private-key files in `ssh_dir`.
///
/// Looks for common key file patterns like `id_rsa`, `id_ed25519`, etc.
/// Also detects other private key files by checking file content headers.
/// A directory that does not exist yields an empty list, not an error: a
/// caller scanning several candidate directories (`~/.ssh`,
/// `~/.ssh/rustconn`, …) should not have to check existence first.
///
/// # Errors
///
/// Returns `AgentError::Io` if the directory exists but cannot be read.
pub fn list_key_files_in(ssh_dir: &Path) -> AgentResult<Vec<PathBuf>> {
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

    // File extensions that indicate private keys
    let key_extensions = ["pem", "key"];

    // Files to skip (not private keys)
    let skip_files = [
        "known_hosts",
        "known_hosts.old",
        "config",
        "authorized_keys",
    ];

    for entry in std::fs::read_dir(ssh_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file()
            && let Some(file_name) = path.file_name().and_then(|n| n.to_str())
        {
            // Skip public keys
            let is_pub = Path::new(file_name)
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("pub"));
            if is_pub {
                continue;
            }

            // Skip known non-key files
            if skip_files.contains(&file_name) {
                continue;
            }

            // Check if it matches a known pattern
            let is_standard_key = key_patterns.contains(&file_name) || file_name.starts_with("id_");

            // Check if it has a key extension (.pem, .key)
            let has_key_extension = Path::new(file_name)
                .extension()
                .is_some_and(|ext| key_extensions.iter().any(|e| ext.eq_ignore_ascii_case(e)));

            // Check file content for private key header
            let is_private_key_content = if !is_standard_key && !has_key_extension {
                is_private_key_file(&path)
            } else {
                false
            };

            if is_standard_key || has_key_extension || is_private_key_content {
                keys.push(path);
            }
        }
    }

    keys.sort();
    Ok(keys)
}

#[cfg(test)]
mod list_key_files_tests {
    use super::*;

    #[test]
    fn lists_only_private_keys_sorted() {
        let dir = tempfile::tempdir().unwrap();
        let ssh_dir = dir.path();
        std::fs::write(ssh_dir.join("id_ed25519"), "fake").unwrap();
        std::fs::write(ssh_dir.join("id_ed25519.pub"), "ssh-ed25519 AAAA").unwrap();
        std::fs::write(ssh_dir.join("config"), "Host *").unwrap();
        std::fs::write(ssh_dir.join("known_hosts"), "example.com AAAA").unwrap();
        std::fs::write(ssh_dir.join("x.pem"), "-----BEGIN RSA PRIVATE KEY-----").unwrap();
        std::fs::write(
            ssh_dir.join("oddname"),
            "-----BEGIN OPENSSH PRIVATE KEY-----\n",
        )
        .unwrap();

        let keys = list_key_files_in(ssh_dir).unwrap();
        let mut names: Vec<&str> = keys
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        names.sort_unstable();
        assert_eq!(names, vec!["id_ed25519", "oddname", "x.pem"]);
        // Already sorted by the function itself.
        assert!(keys.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn missing_directory_is_empty_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        assert_eq!(list_key_files_in(&missing).unwrap(), Vec::<PathBuf>::new());
    }
}

/// Ceiling on every `ssh-add` that adds a key.
///
/// A backstop, not the loop guard — the one-shot askpass already ends the
/// wrong-passphrase retry loop, and a host askpass left waiting for an answer
/// is killed at this point. Generous because a key encrypted with many bcrypt
/// KDF rounds (`ssh-keygen -a`) takes seconds to decrypt.
const SSH_ADD_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// The `ssh-add` command that loads `key_path` into the agent at
/// `socket_path`, with `-t <seconds>` when a `lifetime` is given.
fn ssh_add_command(
    key_path: &std::path::Path,
    socket_path: &str,
    lifetime: Option<std::time::Duration>,
) -> std::process::Command {
    let mut cmd = std::process::Command::new("ssh-add");
    if let Some(lifetime) = lifetime {
        cmd.arg("-t").arg(lifetime.as_secs().to_string());
    }
    cmd.arg(key_path).env("SSH_AUTH_SOCK", socket_path);
    cmd
}

/// The `ssh-add` command for a key RustConn adds on its own: the
/// [`OWN_KEY_LIFETIME`], into the agent at `socket_path` and no other, and
/// the host `SSH_ASKPASS` stripped, so no passphrase prompt can open (a host
/// helper may also be missing inside the Flatpak sandbox).
fn own_ssh_add_command(key_path: &std::path::Path, socket_path: &str) -> std::process::Command {
    let mut cmd = ssh_add_command(key_path, socket_path, Some(OWN_KEY_LIFETIME));
    cmd.env_remove("SSH_ASKPASS");
    cmd
}

/// The `ssh-add` command that loads `key_path` with its passphrase supplied by
/// the askpass helper at `script_path`, which reads it from `secret_path`.
///
/// Only the two paths enter the child's environment, never the passphrase.
fn ssh_add_askpass_command(
    key_path: &std::path::Path,
    socket_path: &str,
    script_path: &std::path::Path,
    secret_path: &std::path::Path,
) -> std::process::Command {
    use std::process::Stdio;

    let mut cmd = ssh_add_command(key_path, socket_path, None);
    cmd.env("SSH_ASKPASS", script_path)
        .env("SSH_ASKPASS_REQUIRE", "force")
        .env("DISPLAY", ":0")
        .env(crate::ssh_tunnel::TUNNEL_ASKPASS_ENV_VAR, secret_path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    cmd
}

/// Removes stale `.pub` files from the agent-keys directory.
///
/// Called at application startup to clean up files left behind if the app
/// crashed or was killed before `remove_agent_identity` ran. Files older than
/// 12 hours are considered stale — a normal session writes the file at connect
/// time and deletes it at close time, so any file surviving that long is orphaned.
///
/// Best effort: errors are logged and swallowed. The directory itself is never
/// removed because a concurrent session may be writing into it.
pub fn prune_stale_agent_keys() {
    use std::time::{Duration, SystemTime};

    // Must resolve the same base as `materialize_agent_identity` — see the note
    // there. Shared resolver, with the `temp_dir` fallback for a Linux box
    // lacking a runtime dir.
    let base = crate::secret_file_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("rustconn")
        .join("agent-keys");

    let Ok(entries) = std::fs::read_dir(&base) else {
        return; // Directory does not exist or is unreadable — nothing to prune.
    };

    // 12 hours: long enough for any legitimate session, short enough to not
    // accumulate junk across work days.
    let stale_threshold = Duration::from_hours(12);
    let now = SystemTime::now();

    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "pub") {
            continue;
        }
        let is_stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .and_then(|mtime| now.duration_since(mtime).map_err(std::io::Error::other))
            .is_ok_and(|age| age > stale_threshold);
        if is_stale {
            tracing::debug!(path = %path.display(), "Removing stale agent key file");
            let _ = std::fs::remove_file(&path);
        }
    }
}
