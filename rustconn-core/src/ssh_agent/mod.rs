//! SSH Agent management module
//!
//! This module provides functionality for interacting with the SSH agent,
//! including starting the agent, managing keys, and parsing agent output.

mod keyfile;
mod own_keys;
mod unlock;
#[cfg(unix)]
mod wire;

#[cfg(test)]
pub(crate) use keyfile::openssh_key_fixture;
pub use keyfile::{
    KeyEncryption, KeyFileInfo, KeyFormat, PublicKeyInfo, inspect_key_file, openssh_public_key_line,
};
pub use own_keys::{OWN_KEY_LIFETIME, evict_own_keys};
pub use unlock::{
    AfterWrongPassphrase, AgentHolds, MAX_PASSPHRASE_ATTEMPTS, SkipReason, UnlockDecision,
    after_wrong_passphrase, agent_refused_add, check_key_unlock, decide,
};

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

    /// `ssh-add` could not decrypt the key with the passphrase it was given
    #[error("Incorrect passphrase")]
    WrongPassphrase,

    /// The agent refused to take the key (an agent such as 1Password's that
    /// does not accept added keys, or a locked agent)
    #[error("The agent refused to add the key: {0}")]
    AddRefused(String),

    /// Whether the agent already holds the key could not be told, so it was
    /// not added; the agent may hold it anyway
    #[error("Cannot tell whether the agent holds the key: {0}")]
    KeyPresenceUnknown(String),

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
        assert_eq!(
            env("SSH_ASKPASS_REQUIRE"),
            Some(Some("never".to_owned())),
            "no compiled-in default askpass either"
        );
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
            None,
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

    /// The stderr texts measured on OpenSSH 10.2 with the one-shot askpass.
    #[test]
    fn ssh_add_outcomes_are_told_apart() {
        assert!(
            classify_ssh_add(true, "Identity added: /k (u@h)\nLifetime set to 12:00:00\n").is_ok()
        );
        assert!(matches!(
            classify_ssh_add(false, ""),
            Err(AgentError::WrongPassphrase)
        ));
        assert!(matches!(
            classify_ssh_add(false, "Bad passphrase, try again for /k: \n"),
            Err(AgentError::WrongPassphrase)
        ));
        assert!(matches!(
            classify_ssh_add(false, "Could not add identity \"/k\": agent refused operation\n"),
            Err(AgentError::AddRefused(m)) if m == "Could not add identity \"/k\": agent refused operation"
        ));
        assert!(matches!(
            classify_ssh_add(false, "Error loading key \"/k\": invalid format\n"),
            Err(AgentError::AddKeyFailed(m)) if m == "Error loading key \"/k\": invalid format"
        ));
    }

    /// The unlock add carries the 12 h lifetime and forces the askpass, whose
    /// environment holds the two paths and not the passphrase.
    #[test]
    fn the_unlock_add_carries_the_lifetime_and_only_paths() {
        let cmd = ssh_add_askpass_command(
            std::path::Path::new("/home/u/.ssh/id_ed25519"),
            "/tmp/agent.sock",
            std::path::Path::new("/run/user/1000/askpass.sh"),
            std::path::Path::new("/run/user/1000/secret"),
            Some(OWN_KEY_LIFETIME),
        );
        let args: Vec<String> = cmd
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["-t", "43200", "/home/u/.ssh/id_ed25519"]);
        let envs: Vec<(String, String)> = cmd
            .get_envs()
            .filter_map(|(k, v)| Some((k.to_str()?.to_owned(), v?.to_str()?.to_owned())))
            .collect();
        assert!(envs.contains(&("SSH_ASKPASS_REQUIRE".to_owned(), "force".to_owned())));
        assert!(envs.contains(&(
            "SSH_ASKPASS".to_owned(),
            "/run/user/1000/askpass.sh".to_owned()
        )));
        assert!(envs.contains(&(
            crate::ssh_tunnel::TUNNEL_ASKPASS_ENV_VAR.to_owned(),
            "/run/user/1000/secret".to_owned()
        )));
        assert!(envs.contains(&("SSH_AUTH_SOCK".to_owned(), "/tmp/agent.sock".to_owned())));
    }
}

// ============================================================================
// SSH Agent Manager Implementation
// ============================================================================

impl SshAgentManager {
    /// Starts a new SSH agent and returns the socket path.
    ///
    /// Executes `ssh-agent -s` and parses the output to extract the socket path.
    /// The run is bounded by [`AGENT_COMMAND_TIMEOUT`].
    ///
    /// # Errors
    ///
    /// Returns `AgentError::StartFailed` if the agent cannot be started or
    /// `ssh-agent` does not finish in time (it has then been killed and reaped).
    pub fn start_agent() -> AgentResult<String> {
        let mut cmd = std::process::Command::new("ssh-agent");
        cmd.arg("-s");
        let output = run_agent_command(cmd, AGENT_COMMAND_TIMEOUT, "ssh-agent -s")
            .map_err(|e| AgentError::StartFailed(e.to_string()))?
            .ok_or_else(|| {
                AgentError::StartFailed(format!(
                    "ssh-agent did not finish within {} s",
                    AGENT_COMMAND_TIMEOUT.as_secs()
                ))
            })?;

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
    /// [`Self::get_status_timeout`] with [`AGENT_COMMAND_TIMEOUT`]: the native
    /// query is bounded by the wire timeouts, and its `ssh-add -l` fallback is
    /// killed after that budget, so a broken or launchd-throttled agent cannot
    /// park the caller.
    ///
    /// # Errors
    ///
    /// Returns `AgentError::NotRunning` if no socket path is configured.
    pub fn get_status(&self) -> AgentResult<AgentStatus> {
        self.get_status_timeout(AGENT_COMMAND_TIMEOUT)
    }

    /// The agent's identities over the native wire protocol, or `None` to make
    /// the caller fall back to `ssh-add -l`.
    ///
    /// `Some(status)` is a complete answer: `running: true` with the keys, or
    /// `running: false` when the agent did not answer in time, which is what
    /// an `ssh-add -l` against the same agent would end in too, only after
    /// another wait. `None` means the native client could not talk to this
    /// agent for another reason (an unexpected socket, a non-standard
    /// implementation) and the shell-out path should be tried instead, so
    /// behaviour is never worse than before (A1).
    #[cfg(unix)]
    fn native_status(socket_path: &str) -> Option<AgentStatus> {
        match wire::request_identities(socket_path) {
            Ok(keys) => Some(AgentStatus {
                running: true,
                socket_path: Some(socket_path.to_string()),
                keys,
            }),
            Err(e) if is_timeout(&e) => {
                tracing::debug!(
                    error = %e,
                    "ssh-agent did not answer in time; not falling back to ssh-add -l"
                );
                Some(AgentStatus {
                    running: false,
                    socket_path: Some(socket_path.to_string()),
                    keys: Vec::new(),
                })
            }
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
    /// Asks the agent over its socket first. When the agent does not answer
    /// that in time, it is reported as not running without a second wait.
    /// When the native query fails otherwise, spawns `ssh-add -l` and waits up
    /// to `timeout` for it to complete. If the process does
    /// not finish in time it is killed and reaped, and the agent is reported as
    /// not running. This prevents the caller from hanging
    /// when the system ssh-agent is broken or launchd-throttled (common on
    /// macOS).
    ///
    /// # Errors
    ///
    /// Returns `AgentError::NotRunning` if no socket path is configured.
    pub fn get_status_timeout(&self, timeout: std::time::Duration) -> AgentResult<AgentStatus> {
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

        // Fallback: `ssh-add -l`, killed and reaped once `timeout` runs out.
        // A spawn failure, a wait failure and a timeout all report the agent
        // as not running.
        let mut cmd = std::process::Command::new("ssh-add");
        cmd.arg("-l").env("SSH_AUTH_SOCK", &socket_path);
        let output = run_agent_command(cmd, timeout, "ssh-add -l").ok().flatten();

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
    /// key comes off RustConn's own-key list, so a later exit, lock or suspend
    /// does not remove it even if RustConn had added the same key itself
    /// earlier. It is taken off before `ssh-add` runs and again after, so an
    /// eviction that starts in between does not pick it up; a failed add
    /// leaves it off the list too, and RustConn's earlier copy then expires
    /// with its lifetime. A `SSH_AGENTC_REMOVE_IDENTITY` an eviction had
    /// already sent can still reach the agent after the add and remove the
    /// key: closing that would need the add and the eviction to share a lock,
    /// which they do not. The passphrase is used for this one add and not
    /// kept.
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
        own_keys::forget(socket_path, key_path);
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
    /// be told (the agent does not answer, or the key file does not carry its
    /// public key and the agent holds any key; a `.pub` beside it may be
    /// stale and is not trusted) nothing is added. Otherwise the key goes in
    /// with `ssh-add -t` [`OWN_KEY_LIFETIME`]. It is recorded for
    /// [`evict_own_keys`] only when its own private key file carries its
    /// public blob (an OpenSSH key) and the agent shows that blob after the
    /// add, together with its `<key>-cert.pub` certificate when the add put
    /// that in the agent too; any other key is left to its lifetime.
    ///
    /// No passphrase is taken and no askpass is offered
    /// (`SSH_ASKPASS_REQUIRE=never`): an encrypted key loads only if
    /// `ssh-add` can decrypt it without asking. The `ssh-add` run is bounded,
    /// so a caller on the GTK thread never waits on it for more than 30 s.
    ///
    /// # Errors
    ///
    /// Returns `AgentError::NotRunning` if no socket is configured.
    /// Returns `AgentError::KeyPresenceUnknown` if the agent's answer cannot
    /// settle whether it already holds this key (the key file does not carry
    /// its public key and the agent holds any key).
    /// Returns `AgentError::AddKeyFailed` if the agent cannot be asked which
    /// keys it holds, or if `ssh-add` fails or times out.
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

    /// Adds an encrypted key RustConn needs in the agent on its own, with the
    /// passphrase the user just gave in RustConn's unlock dialog.
    ///
    /// The same terms as [`Self::add_own_key`]: the agent at this manager's
    /// socket is asked which keys it holds before and after, a key it already
    /// holds is left alone, and the key goes in with `ssh-add -t`
    /// [`OWN_KEY_LIFETIME`] and is recorded for [`evict_own_keys`].
    ///
    /// The passphrase is used for this one `ssh-add` and not kept. It reaches
    /// `ssh-add` only through an owner-only (0600) one-shot secret file whose
    /// path, never the passphrase, is in the child's environment; the file is
    /// zeroized and unlinked before this returns. An agent that refuses the
    /// add is remembered for the process ([`agent_refused_add`]). The run is
    /// bounded to 30 s; call it off the GTK thread.
    ///
    /// # Errors
    ///
    /// Returns `AgentError::NotRunning` if no socket is configured.
    /// Returns `AgentError::WrongPassphrase` if `ssh-add` could not decrypt
    /// the key with `passphrase`.
    /// Returns `AgentError::AddRefused` if the agent refused the key.
    /// Returns `AgentError::KeyPresenceUnknown` on the same terms as
    /// [`Self::add_own_key`].
    /// Returns `AgentError::AddKeyFailed` if the agent cannot be asked which
    /// keys it holds, the secret file cannot be written, or `ssh-add` fails
    /// otherwise or times out.
    pub fn add_own_key_with_passphrase(
        &self,
        key_path: &std::path::Path,
        passphrase: &secrecy::SecretString,
    ) -> AgentResult<()> {
        let socket_path = self
            .socket_path
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or(AgentError::NotRunning)?;
        add_own_key_with_passphrase_in(
            &own_keys::OWN_KEYS,
            socket_path,
            key_path,
            passphrase,
            |cmd| wait_ssh_add(cmd, "ssh-add with askpass"),
        )
    }
}

/// [`SshAgentManager::add_own_key_with_passphrase`] with the record and the
/// `ssh-add` run passed in, so tests can stand in for both.
fn add_own_key_with_passphrase_in(
    own: &own_keys::OwnKeys,
    socket_path: &str,
    key_path: &std::path::Path,
    passphrase: &secrecy::SecretString,
    run: impl FnOnce(std::process::Command) -> AgentResult<std::process::Output>,
) -> AgentResult<()> {
    let result = own.add_and_record(
        socket_path,
        key_path,
        &keyfile::key_file_blobs(key_path),
        || {
            let output = ssh_add_with_one_shot_askpass(
                key_path,
                socket_path,
                passphrase,
                Some(OWN_KEY_LIFETIME),
                run,
            )?;
            classify_ssh_add(
                output.status.success(),
                &String::from_utf8_lossy(&output.stderr),
            )?;
            tracing::info!(?key_path, "unlocked SSH key added to agent");
            Ok(())
        },
    );
    if let Err(AgentError::AddRefused(_)) = &result {
        unlock::record_add_refused(socket_path);
    }
    result
}

/// What an `ssh-add` run through the one-shot askpass means, from its exit
/// status and stderr.
///
/// Measured on OpenSSH 10.2: a wrong passphrase exits 1 with empty stderr
/// (the helper's empty second answer ends `ssh-add`'s retry loop silently);
/// an agent that refuses the key exits 1 with `Could not add identity
/// "<path>": agent refused operation`.
fn classify_ssh_add(success: bool, stderr: &str) -> AgentResult<()> {
    let stderr = stderr.trim();
    if success {
        Ok(())
    } else if stderr.contains("Could not add identity") {
        Err(AgentError::AddRefused(stderr.to_owned()))
    } else if stderr.is_empty() || stderr.contains("Bad passphrase") {
        Err(AgentError::WrongPassphrase)
    } else {
        Err(AgentError::AddKeyFailed(stderr.to_owned()))
    }
}

/// Runs `ssh-add` for `key_path` against `socket_path` through `run`, with the
/// passphrase supplied by the one-shot askpass helper, and removes the helper
/// and the secret file before returning, whatever the outcome.
fn ssh_add_with_one_shot_askpass(
    key_path: &std::path::Path,
    socket_path: &str,
    passphrase: &secrecy::SecretString,
    lifetime: Option<std::time::Duration>,
    run: impl FnOnce(std::process::Command) -> AgentResult<std::process::Output>,
) -> AgentResult<std::process::Output> {
    use crate::ssh_tunnel::{
        cleanup_askpass_script, create_one_shot_askpass_script, create_tunnel_secret_file,
    };

    // The passphrase reaches ssh-add's askpass helper through an owner-only
    // (0600) file in `$XDG_RUNTIME_DIR`; the environment carries only that
    // file's path. It used to travel as the value of an env var, which the
    // same user can read in `/proc/<pid>/environ` for as long as ssh-add
    // runs. The secret file is the SSH tunnel's; the helper is its one-shot
    // variant, because ssh-add asks again after a wrong passphrase and only
    // an empty answer stops it.
    let script_path = create_one_shot_askpass_script().map_err(AgentError::AddKeyFailed)?;
    // ponytail: temp_dir fallback kept by decision (K1, 0.24): one
    // resolver for every askpass secret file, `ssh_askpass::secret_dir`;
    // refusing would break su/sudo/container sessions, and a private
    // /tmp subdir adds a DoS vector and no secrecy over 0600 O_EXCL
    // random names. Follow-ups: tunnel scripts to the runtime dir,
    // atomic GUI script.
    let secret_path = match create_tunnel_secret_file(passphrase) {
        Ok(path) => path,
        Err(e) => {
            cleanup_askpass_script(&script_path);
            return Err(AgentError::AddKeyFailed(e));
        }
    };

    // Bounded, so the files below are always cleaned up and the caller never
    // waits on an ssh-add that does not finish.
    let output = run(ssh_add_askpass_command(
        key_path,
        socket_path,
        &script_path,
        &secret_path,
        lifetime,
    ));

    // The child has exited or been killed and reaped, so neither file is
    // needed any more. The secret file is zeroized before it is unlinked.
    cleanup_askpass_script(&secret_path);
    cleanup_askpass_script(&script_path);
    output
}

/// Runs the user's `ssh-add` for `key_path` against `socket_path`, with the
/// passphrase through the one-shot askpass when one is given. No lifetime.
fn run_ssh_add(
    key_path: &std::path::Path,
    socket_path: &str,
    passphrase: Option<&secrecy::SecretString>,
) -> AgentResult<()> {
    if let Some(pass) = passphrase {
        let output = ssh_add_with_one_shot_askpass(key_path, socket_path, pass, None, |cmd| {
            wait_ssh_add(cmd, "ssh-add with askpass")
        })?;
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
    /// When the public half can be read without a passphrase (an OpenSSH
    /// private key, or a `.pub` file passed as `key_path`) the key is removed
    /// with one native `SSH_AGENTC_REMOVE_IDENTITY`, followed by its
    /// `<key>-cert.pub` certificate when one sits beside it and certifies
    /// this key. As with `ssh-add -d`, the certificate is tried even when the
    /// agent no longer holds the key, and the removal succeeds when either
    /// went; a certificate for another key is left in the agent (where
    /// `ssh-add -d` stops with an error). Otherwise `ssh-add -d` runs,
    /// bounded by [`AGENT_COMMAND_TIMEOUT`].
    ///
    /// # Arguments
    ///
    /// * `key_path` - Path to the private key file (or public key)
    ///
    /// # Errors
    ///
    /// Returns `AgentError::NotRunning` if no socket is configured.
    /// Returns `AgentError::RemoveKeyFailed` if the key cannot be removed, the
    /// agent does not hold it, or `ssh-add -d` does not finish in time.
    pub fn remove_key(&self, key_path: &std::path::Path) -> AgentResult<()> {
        let socket_path = self.socket_path.as_ref().ok_or(AgentError::NotRunning)?;

        #[cfg(unix)]
        {
            let blobs = keyfile::key_file_blobs(key_path);
            // A public-key line is read only from a `.pub` path: a legacy PEM
            // or PKCS#8 private key goes to `ssh-add -d` instead.
            let is_pub = key_path.extension().is_some_and(|ext| ext == "pub");
            if let Some(blob) = blobs
                .key_file
                .clone()
                .or_else(|| is_pub.then(|| keyfile::pub_file_blob(key_path)).flatten())
            {
                let cert = blobs.cert_file.as_deref().filter(|cert| {
                    let matches = keyfile::certifies(cert, &blob);
                    if !matches {
                        tracing::warn!(
                            ?key_path,
                            "the -cert.pub beside the key does not certify it; leaving it in the agent"
                        );
                    }
                    matches
                });
                return remove_natively(socket_path, &blob, cert);
            }
        }

        let mut cmd = std::process::Command::new("ssh-add");
        cmd.arg("-d")
            .arg(key_path)
            .env("SSH_AUTH_SOCK", socket_path);
        let output = run_agent_command(cmd, AGENT_COMMAND_TIMEOUT, "ssh-add -d")
            .map_err(|e| AgentError::RemoveKeyFailed(e.to_string()))?
            .ok_or_else(|| {
                AgentError::RemoveKeyFailed(format!(
                    "ssh-add -d did not finish within {} s",
                    AGENT_COMMAND_TIMEOUT.as_secs()
                ))
            })?;

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
    /// Asks the agent for its identities over the socket and fingerprints each
    /// blob in-process; no process runs. Only when the agent cannot be asked
    /// that way, for a reason other than a timeout, does `ssh-add -L` run,
    /// bounded by [`AGENT_COMMAND_TIMEOUT`], and its lines are fingerprinted
    /// in-process too.
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
    /// Returns `AgentError::Io` (`TimedOut` or `WouldBlock`) if the agent did
    /// not answer the socket in time, or if `ssh-add -L` did not finish.
    pub fn get_public_key_by_fingerprint(&self, fingerprint: &str) -> AgentResult<String> {
        let socket_path = self.socket_path.as_ref().ok_or(AgentError::NotRunning)?;

        #[cfg(unix)]
        match wire::request_identity_blobs(socket_path) {
            Ok(blobs) => {
                return match blobs
                    .iter()
                    .find(|blob| keyfile::sha256_fingerprint(blob) == fingerprint)
                {
                    Some(blob) => keyfile::public_key_line(blob),
                    None => Err(AgentError::KeyNotFound(fingerprint.to_string())),
                };
            }
            // An agent that did not answer the socket in time would not
            // answer `ssh-add -L` either: fail now rather than wait again.
            Err(e) if is_timeout(&e) => return Err(e),
            Err(e) => tracing::debug!(
                error = %e,
                "native ssh-agent query failed; falling back to ssh-add -L"
            ),
        }

        let mut cmd = std::process::Command::new("ssh-add");
        cmd.arg("-L").env("SSH_AUTH_SOCK", socket_path);
        let output =
            run_agent_command(cmd, AGENT_COMMAND_TIMEOUT, "ssh-add -L")?.ok_or_else(|| {
                AgentError::Io(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    format!(
                        "ssh-add -L did not finish within {} s",
                        AGENT_COMMAND_TIMEOUT.as_secs()
                    ),
                ))
            })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            if stderr.contains("no identities") || stderr.contains("The agent has no identities") {
                return Err(AgentError::KeyNotFound(fingerprint.to_string()));
            }
            return Err(AgentError::ParseError(stderr.to_string()));
        }

        // Each line is `algo base64 [comment]`; fingerprinted in-process.
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .map(str::trim)
            .find(|line| {
                PublicKeyInfo::from_pub_line(line).is_ok_and(|info| info.fingerprint == fingerprint)
            })
            .map(str::to_string)
            .ok_or_else(|| AgentError::KeyNotFound(fingerprint.to_string()))
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

/// Ceiling on every agent child process that does not decrypt a key:
/// `ssh-agent -s`, and the `ssh-add -l`, `-L` and `-d` fallbacks.
///
/// Each of them answers in milliseconds from a healthy agent; this only bounds
/// a wedged or launchd-throttled one so the caller (often the GTK thread, or
/// startup) is never parked on it.
pub(crate) const AGENT_COMMAND_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Spawns `cmd` with no stdin and piped output and waits for it at most
/// `budget`. `Ok(None)` means it ran over and has been killed and reaped.
///
/// The output is read only after the child exits ([`crate::proc::wait_bounded`]),
/// so a child that writes more than a pipe buffer (about 64 KiB on Linux)
/// blocks and comes back as `Ok(None)`, a timeout. For the `ssh-add -l` / `-L`
/// fallbacks that takes an agent holding several hundred keys, and only after
/// the native query, which has no such limit, has failed.
///
/// `cmd` must not carry a secret in its arguments or environment: none of the
/// agent commands routed here takes one.
///
/// # Errors
///
/// Returns the `io::Error` from spawning or waiting.
// ponytail: pipes are read after exit, fine for agent-sized output (< 64 KiB);
// drain them on a thread in `proc` if a fallback ever has to list more.
pub(crate) fn run_agent_command(
    mut cmd: std::process::Command,
    budget: std::time::Duration,
    what: &'static str,
) -> std::io::Result<Option<std::process::Output>> {
    use std::process::Stdio;

    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let child = cmd.spawn()?;
    Ok(crate::proc::wait_bounded(child, budget, what)?.output())
}

/// Whether a native agent request failed because the agent did not answer in
/// time: the connect's `TimedOut`, or a read or write timeout, which a Unix
/// socket reports as `WouldBlock`.
///
/// An agent in that state would not answer an `ssh-add` fallback either, so
/// callers skip the fallback and do not make the caller (often the GTK
/// thread) wait a second time.
#[cfg(unix)]
fn is_timeout(error: &AgentError) -> bool {
    matches!(
        error,
        AgentError::Io(e)
            if matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)
    )
}

/// Removes the key with public blob `blob` from the agent at `socket_path`
/// over the native protocol, then its certificate `cert` if given, the way
/// `ssh-add -d` does: the certificate is tried whether or not the agent held
/// the key, and either one going counts as success.
///
/// `cert` must already be checked to certify `blob` ([`keyfile::certifies`]).
///
/// # Errors
///
/// Returns `AgentError::RemoveKeyFailed` if the agent cannot be reached, or
/// holds neither the key nor the certificate.
#[cfg(unix)]
fn remove_natively(socket_path: &str, blob: &[u8], cert: Option<&[u8]>) -> AgentResult<()> {
    let key_removed = wire::remove_identity(socket_path, blob)
        .map_err(|e| AgentError::RemoveKeyFailed(e.to_string()))?;
    let cert_removed = cert.is_some_and(|cert| {
        wire::remove_identity(socket_path, cert).unwrap_or_else(|e| {
            tracing::debug!(error = %e, "could not remove the key's certificate from the agent");
            false
        })
    });
    if key_removed || cert_removed {
        Ok(())
    } else {
        Err(AgentError::RemoveKeyFailed(
            "the agent does not hold this key".to_string(),
        ))
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
/// the host `SSH_ASKPASS` stripped (it may name a helper missing inside the
/// Flatpak sandbox). `SSH_ASKPASS_REQUIRE=never` as well, because without a
/// tty and with `DISPLAY` or `WAYLAND_DISPLAY` set, `ssh-add` would otherwise
/// fall back to its compiled-in default askpass and open a passphrase prompt.
fn own_ssh_add_command(key_path: &std::path::Path, socket_path: &str) -> std::process::Command {
    let mut cmd = ssh_add_command(key_path, socket_path, Some(OWN_KEY_LIFETIME));
    cmd.env_remove("SSH_ASKPASS")
        .env("SSH_ASKPASS_REQUIRE", "never");
    cmd
}

/// The `ssh-add` command that loads `key_path` with its passphrase supplied by
/// the askpass helper at `script_path`, which reads it from `secret_path`.
///
/// Only the two paths enter the child's environment, never the passphrase.
/// `-t <seconds>` when a `lifetime` is given.
fn ssh_add_askpass_command(
    key_path: &std::path::Path,
    socket_path: &str,
    script_path: &std::path::Path,
    secret_path: &std::path::Path,
    lifetime: Option<std::time::Duration>,
) -> std::process::Command {
    use std::process::Stdio;

    let mut cmd = ssh_add_command(key_path, socket_path, lifetime);
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

/// `remove_key` and `get_public_key_by_fingerprint` against an agent that
/// speaks the protocol: answered natively, so no `ssh-add` or `ssh-keygen`
/// runs (a fallback would show up as no request reaching this agent).
#[cfg(test)]
#[cfg(unix)]
mod native_path_tests {
    use std::io::{Read as _, Write as _};
    use std::os::unix::net::UnixListener;
    use std::sync::{Arc, Mutex};

    use super::*;

    /// What the fake agent holds and every message type it was sent.
    type State = Arc<Mutex<(Vec<Vec<u8>>, Vec<u8>)>>;

    fn ssh_string(out: &mut Vec<u8>, bytes: &[u8]) {
        out.extend_from_slice(&u32::try_from(bytes.len()).unwrap().to_be_bytes());
        out.extend_from_slice(bytes);
    }

    fn blob(tag: u8) -> Vec<u8> {
        let mut blob = Vec::new();
        ssh_string(&mut blob, b"ssh-ed25519");
        ssh_string(&mut blob, &[tag; 32]);
        blob
    }

    /// A minimal agent answering `REQUEST_IDENTITIES` (11) and
    /// `REMOVE_IDENTITY` (18). The accept thread ends with the test process.
    fn fake_agent(dir: &Path, keys: &[&[u8]]) -> (String, State) {
        let path = dir.join("agent.sock");
        let listener = UnixListener::bind(&path).unwrap();
        let state: State = Arc::new(Mutex::new((
            keys.iter().map(|k| k.to_vec()).collect(),
            Vec::new(),
        )));
        let served = Arc::clone(&state);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { return };
                let mut len = [0u8; 4];
                while stream.read_exact(&mut len).is_ok() {
                    let mut body = vec![0u8; u32::from_be_bytes(len) as usize];
                    stream.read_exact(&mut body).unwrap();
                    let mut state = served.lock().unwrap();
                    state.1.push(body[0]);
                    let reply = match body[0] {
                        11 => {
                            let mut answer = vec![12];
                            let count = u32::try_from(state.0.len()).unwrap();
                            answer.extend_from_slice(&count.to_be_bytes());
                            for key in &state.0 {
                                ssh_string(&mut answer, key);
                                ssh_string(&mut answer, b"comment");
                            }
                            answer
                        }
                        18 => {
                            let held = state.0.iter().position(|k| k[..] == body[5..]);
                            vec![if held.map(|i| state.0.remove(i)).is_some() {
                                6
                            } else {
                                5
                            }]
                        }
                        _ => vec![5],
                    };
                    drop(state);
                    let mut framed = u32::try_from(reply.len()).unwrap().to_be_bytes().to_vec();
                    framed.extend_from_slice(&reply);
                    stream.write_all(&framed).unwrap();
                }
            }
        });
        (path.to_string_lossy().into_owned(), state)
    }

    #[test]
    fn a_public_key_is_found_by_fingerprint_without_a_process() {
        let dir = tempfile::tempdir().unwrap();
        let (wanted, other) = (blob(1), blob(2));
        let (socket, state) = fake_agent(dir.path(), &[&other, &wanted]);
        let manager = SshAgentManager::new(Some(socket));

        let line = manager
            .get_public_key_by_fingerprint(&keyfile::sha256_fingerprint(&wanted))
            .unwrap();
        assert_eq!(
            line,
            format!("ssh-ed25519 {}", data_encoding::BASE64.encode(&wanted))
        );
        assert!(matches!(
            manager.get_public_key_by_fingerprint("SHA256:not-held"),
            Err(AgentError::KeyNotFound(_))
        ));
        assert_eq!(
            state.lock().unwrap().1,
            vec![11, 11],
            "both answered natively"
        );
    }

    /// Writes an OpenSSH private key carrying `key` and a `-cert.pub` beside
    /// it holding `cert`; returns the key path.
    fn key_with_cert(dir: &Path, key: &[u8], cert: &[u8]) -> PathBuf {
        let key_path = dir.join("id_ed25519");
        std::fs::write(&key_path, keyfile::openssh_key_fixture("none", key)).unwrap();
        std::fs::write(
            dir.join("id_ed25519-cert.pub"),
            format!(
                "ssh-ed25519-cert-v01@openssh.com {}\n",
                data_encoding::BASE64.encode(cert)
            ),
        )
        .unwrap();
        key_path
    }

    /// An OpenSSH private key names its public half, so the key and the
    /// certificate beside it are removed natively and nothing else is.
    #[test]
    fn an_openssh_key_and_its_certificate_are_removed_natively() {
        let dir = tempfile::tempdir().unwrap();
        let (key, users) = (blob(3), blob(5));
        let cert = keyfile::openssh_cert_fixture(&key);
        let key_path = key_with_cert(dir.path(), &key, &cert);
        let (socket, state) = fake_agent(dir.path(), &[&key, &cert, &users]);

        SshAgentManager::new(Some(socket))
            .remove_key(&key_path)
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.0, vec![users], "the user's other key stays");
        assert_eq!(state.1, vec![18, 18]);
    }

    /// A `-cert.pub` that certifies some other key is not sent to the agent:
    /// the key goes, the certificate stays.
    #[test]
    fn a_certificate_for_another_key_is_left_in_the_agent() {
        let dir = tempfile::tempdir().unwrap();
        let (key, other) = (blob(8), blob(9));
        let foreign_cert = keyfile::openssh_cert_fixture(&other);
        let key_path = key_with_cert(dir.path(), &key, &foreign_cert);
        let (socket, state) = fake_agent(dir.path(), &[&key, &foreign_cert]);

        SshAgentManager::new(Some(socket))
            .remove_key(&key_path)
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.0, vec![foreign_cert], "the foreign certificate stays");
        assert_eq!(state.1, vec![18], "only the key was asked for");
    }

    /// As with `ssh-add -d`, the certificate is removed even when the agent
    /// no longer holds the key itself, and that counts as success.
    #[test]
    fn the_certificate_goes_even_when_the_key_is_already_gone() {
        let dir = tempfile::tempdir().unwrap();
        let (key, users) = (blob(10), blob(11));
        let cert = keyfile::openssh_cert_fixture(&key);
        let key_path = key_with_cert(dir.path(), &key, &cert);
        let (socket, state) = fake_agent(dir.path(), &[&cert, &users]);

        SshAgentManager::new(Some(socket))
            .remove_key(&key_path)
            .unwrap();
        let state = state.lock().unwrap();
        assert_eq!(state.0, vec![users]);
        assert_eq!(state.1, vec![18, 18]);
    }

    #[test]
    fn a_pub_file_is_removed_natively_and_a_key_not_held_is_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let (held, not_held) = (blob(6), blob(7));
        let line = |b: &[u8]| format!("ssh-ed25519 {} c\n", data_encoding::BASE64.encode(b));
        let held_pub = dir.path().join("held.pub");
        let not_held_pub = dir.path().join("not_held.pub");
        std::fs::write(&held_pub, line(&held)).unwrap();
        std::fs::write(&not_held_pub, line(&not_held)).unwrap();
        let (socket, state) = fake_agent(dir.path(), &[&held]);
        let manager = SshAgentManager::new(Some(socket));

        manager.remove_key(&held_pub).unwrap();
        assert!(matches!(
            manager.remove_key(&not_held_pub),
            Err(AgentError::RemoveKeyFailed(_))
        ));
        let state = state.lock().unwrap();
        assert!(state.0.is_empty());
        assert_eq!(state.1, vec![18, 18]);
    }

    /// An agent that takes the connection but never answers is a timeout, and
    /// neither query then falls back to `ssh-add`, which would wait on the
    /// same agent for another [`AGENT_COMMAND_TIMEOUT`].
    #[test]
    fn a_silent_agent_is_not_asked_again_through_ssh_add() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.sock");
        // Never accepts: the connect lands in the listen queue, the request
        // fits in the socket buffer, and no reply ever comes.
        let _listener = UnixListener::bind(&path).unwrap();
        let socket = path.to_string_lossy().into_owned();

        let started = std::time::Instant::now();
        let status = {
            let socket = socket.clone();
            std::thread::spawn(move || SshAgentManager::new(Some(socket)).get_status())
        };
        let lookup = SshAgentManager::new(Some(socket)).get_public_key_by_fingerprint("SHA256:x");
        let status = status.join().unwrap().unwrap();
        let elapsed = started.elapsed();

        assert!(!status.running);
        assert!(
            matches!(&lookup, Err(e) if is_timeout(e)),
            "expected a timeout, got {lookup:?}"
        );
        // One 3 s wire timeout, not that plus an `ssh-add` budget.
        assert!(
            elapsed < AGENT_COMMAND_TIMEOUT,
            "took {elapsed:?}, so an ssh-add fallback ran"
        );
    }

    /// An agent command that outlives its budget is killed and reaped:
    /// `Ok(None)`, back in about the budget, and the child's pid is gone.
    /// `kill(pid, 0)` answers `ESRCH` only once the process is both dead and
    /// reaped, since a zombie child of this process still answers it.
    #[test]
    fn an_agent_command_past_its_budget_is_killed_and_reaped() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("pid");
        // A stand-in for a wedged `ssh-add`: records its pid, then hangs.
        let mut cmd = std::process::Command::new("sh");
        cmd.arg("-c")
            .arg("echo $$ > \"$1\"; exec sleep 30")
            .arg("sh")
            .arg(&pid_file);

        let started = std::time::Instant::now();
        let output = run_agent_command(
            cmd,
            std::time::Duration::from_millis(500),
            "test-agent-command",
        )
        .unwrap();
        let elapsed = started.elapsed();

        assert!(output.is_none(), "a run past its budget has no output");
        assert!(
            elapsed < std::time::Duration::from_secs(5),
            "returned after {elapsed:?}, so the budget was not enforced"
        );
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(
            nix::sys::signal::kill(nix::unistd::Pid::from_raw(pid), None),
            Err(nix::errno::Errno::ESRCH),
            "the timed-out child is still alive or was left a zombie"
        );
    }

    /// The fallback `ssh-add` paths run bounded: a missing binary or agent
    /// still returns promptly with an error, never a hang.
    #[test]
    fn get_status_without_a_reachable_agent_reports_not_running() {
        let manager = SshAgentManager::new(Some("/nonexistent/rustconn-test/agent.sock".into()));
        let started = std::time::Instant::now();
        let status = manager.get_status().unwrap();
        assert!(!status.running);
        assert!(started.elapsed() < AGENT_COMMAND_TIMEOUT + std::time::Duration::from_secs(2));
    }
}
