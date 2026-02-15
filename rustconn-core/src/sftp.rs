//! SFTP URI and command builder
//!
//! Provides utilities for building SFTP URIs and CLI commands
//! for SSH connections with SFTP enabled.

use crate::models::Connection;
use crate::models::SshKeySource;
use std::path::PathBuf;

/// Builds an SFTP URI for the given connection.
///
/// Format: `sftp://[user@]host[:port]`
///
/// Used by GUI (`nautilus`) and CLI (`xdg-open`) to open
/// the host file manager's SFTP browser.
#[must_use]
pub fn build_sftp_uri(username: Option<&str>, host: &str, port: u16) -> String {
    let user_part = username.map_or_else(String::new, |u| format!("{u}@"));
    if port == 22 {
        format!("sftp://{user_part}{host}")
    } else {
        format!("sftp://{user_part}{host}:{port}")
    }
}

/// Builds an SFTP URI from a `Connection`.
///
/// Returns `None` if the connection is not SSH.
#[must_use]
pub fn build_sftp_uri_from_connection(connection: &Connection) -> Option<String> {
    if !matches!(
        connection.protocol_config,
        crate::models::ProtocolConfig::Ssh(_) | crate::models::ProtocolConfig::Sftp(_)
    ) {
        return None;
    }

    Some(build_sftp_uri(
        connection.username.as_deref(),
        &connection.host,
        connection.port,
    ))
}

/// Builds an `sftp` CLI command for the given connection.
///
/// Returns `None` if the connection is not SSH.
///
/// The returned `Vec` has the program name as the first element,
/// followed by arguments: `["sftp", "-P", "port", "user@host"]`.
#[must_use]
pub fn build_sftp_command(connection: &Connection) -> Option<Vec<String>> {
    if !matches!(
        connection.protocol_config,
        crate::models::ProtocolConfig::Ssh(_) | crate::models::ProtocolConfig::Sftp(_)
    ) {
        return None;
    }

    let mut cmd = vec!["sftp".to_string()];

    if connection.port != 22 {
        cmd.push("-P".to_string());
        cmd.push(connection.port.to_string());
    }

    let target = if let Some(ref user) = connection.username {
        format!("{user}@{}", connection.host)
    } else {
        connection.host.clone()
    };
    cmd.push(target);

    Some(cmd)
}

/// Extracts the SSH key file path from a connection's config.
///
/// Checks `key_source` (preferred) then falls back to legacy `key_path`.
/// Returns `None` if no key is configured or the connection is not SSH.
#[must_use]
pub fn get_ssh_key_path(connection: &Connection) -> Option<PathBuf> {
    let ssh = match &connection.protocol_config {
        crate::models::ProtocolConfig::Ssh(cfg) | crate::models::ProtocolConfig::Sftp(cfg) => cfg,
        _ => return None,
    };

    match &ssh.key_source {
        SshKeySource::File { path } if !path.as_os_str().is_empty() => Some(path.clone()),
        SshKeySource::Agent { comment, .. } => {
            // Agent key identified by comment — if comment looks like
            // a file path, return it so we can ssh-add it.
            let p = std::path::Path::new(comment);
            if comment.starts_with('/') || comment.starts_with('~') {
                if comment.starts_with('~') {
                    dirs::home_dir()
                        .map(|home| home.join(comment.strip_prefix("~/").unwrap_or(comment)))
                } else {
                    Some(p.to_path_buf())
                }
            } else {
                None
            }
        }
        _ => {
            // Legacy key_path fallback
            ssh.key_path
                .as_ref()
                .filter(|p| !p.as_os_str().is_empty())
                .cloned()
        }
    }
}

/// Checks whether ssh-agent is reachable via `SSH_AUTH_SOCK`.
#[must_use]
pub fn is_ssh_agent_available() -> bool {
    std::env::var("SSH_AUTH_SOCK")
        .ok()
        .filter(|s| !s.is_empty())
        .is_some_and(|sock| std::path::Path::new(&sock).exists())
}

/// Ensures an ssh-agent is running and `SSH_AUTH_SOCK` is set.
///
/// On some desktop environments (notably KDE on openSUSE Tumbleweed)
/// ssh-agent is not started by default. This function:
///
/// 1. Checks if `SSH_AUTH_SOCK` is already set and the socket exists
/// 2. If not, starts `ssh-agent` and parses its output
/// 3. Sets `SSH_AUTH_SOCK` and `SSH_AGENT_PID` in the process
///    environment so all child processes (Dolphin, mc, ssh-add)
///    inherit them
///
/// # Thread Safety
///
/// This function calls `std::env::set_var()` which is not thread-safe.
/// It must be called from `main()` before spawning any threads or
/// starting the async runtime.
///
/// # Returns
///
/// `true` if an agent is available after this call, `false` if
/// we failed to start one.
pub fn ensure_ssh_agent() -> bool {
    if is_ssh_agent_available() {
        tracing::debug!(
            sock = %std::env::var("SSH_AUTH_SOCK").unwrap_or_default(),
            "ssh-agent already available"
        );
        return true;
    }

    tracing::info!("SSH_AUTH_SOCK not set or socket missing; starting ssh-agent");

    let output = match std::process::Command::new("ssh-agent")
        .arg("-s")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(o) => o,
        Err(e) => {
            tracing::warn!(?e, "Failed to run ssh-agent");
            return false;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::warn!(
            %stderr,
            "ssh-agent exited with non-zero status"
        );
        return false;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // ssh-agent -s output looks like:
    //   SSH_AUTH_SOCK=/tmp/ssh-XXXX/agent.1234; export SSH_AUTH_SOCK;
    //   SSH_AGENT_PID=1234; export SSH_AGENT_PID;
    let mut sock = None;
    let mut pid = None;

    for line in stdout.lines() {
        if let Some(val) = line
            .strip_prefix("SSH_AUTH_SOCK=")
            .and_then(|s| s.split(';').next())
        {
            sock = Some(val.to_string());
        }
        if let Some(val) = line
            .strip_prefix("SSH_AGENT_PID=")
            .and_then(|s| s.split(';').next())
        {
            pid = Some(val.to_string());
        }
    }

    let Some(sock_val) = sock else {
        tracing::warn!(
            %stdout,
            "Could not parse SSH_AUTH_SOCK from ssh-agent output"
        );
        return false;
    };

    // set_var is safe in Rust 2021 edition. Called once at startup
    // before any threads are spawned (from main() before tokio).
    std::env::set_var("SSH_AUTH_SOCK", &sock_val);
    if let Some(ref pid_val) = pid {
        std::env::set_var("SSH_AGENT_PID", pid_val);
    }

    tracing::info!(
        %sock_val,
        pid = pid.as_deref().unwrap_or("unknown"),
        "Started ssh-agent and set environment"
    );

    is_ssh_agent_available()
}

/// Ensures the connection's SSH key is loaded in ssh-agent.
///
/// Runs `ssh-add <key_path>` if a key file is configured.
/// This is needed before opening SFTP via mc or file managers,
/// because neither can pass an identity file directly.
///
/// Returns `true` if the key was added (or no key is needed),
/// `false` if `ssh-add` failed.
pub fn ensure_key_in_agent(connection: &Connection) -> bool {
    let Some(key_path) = get_ssh_key_path(connection) else {
        // No key configured — ssh-agent may already have the
        // right key, or password auth is used. Proceed anyway.
        return true;
    };

    if !key_path.exists() {
        tracing::warn!(?key_path, "SSH key file not found, skipping ssh-add");
        return true; // Don't block SFTP — agent may have it
    }

    if !is_ssh_agent_available() {
        tracing::warn!(
            "SSH_AUTH_SOCK not set or agent not running; \
             ssh-add will likely fail"
        );
        // Continue anyway — ssh-add may still work if the
        // agent socket is at a non-standard path.
    }

    tracing::info!(?key_path, "Adding SSH key to agent for SFTP");
    match std::process::Command::new("ssh-add")
        .arg(&key_path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output()
    {
        Ok(output) if output.status.success() => {
            tracing::info!(?key_path, "SSH key added to agent");
            true
        }
        Ok(output) => {
            let stderr = String::from_utf8_lossy(&output.stderr);
            tracing::warn!(
                ?key_path,
                status = ?output.status,
                %stderr,
                "ssh-add failed"
            );
            false
        }
        Err(e) => {
            tracing::error!(?e, "Failed to run ssh-add");
            false
        }
    }
}

/// Builds a Midnight Commander command to open an SFTP panel.
///
/// Returns `None` if the connection is not SSH.
///
/// Uses mc's FISH VFS: `["mc", ".", "sh://user@host:port"]`.
/// Left panel shows local directory, right panel shows remote via FISH.
/// Requires the SSH key to be loaded in ssh-agent beforehand.
#[must_use]
pub fn build_mc_sftp_command(connection: &Connection) -> Option<Vec<String>> {
    if !matches!(
        connection.protocol_config,
        crate::models::ProtocolConfig::Ssh(_) | crate::models::ProtocolConfig::Sftp(_)
    ) {
        return None;
    }

    let target = if let Some(ref user) = connection.username {
        if connection.port == 22 {
            format!("sh://{user}@{}", connection.host)
        } else {
            format!("sh://{user}@{}:{}", connection.host, connection.port)
        }
    } else if connection.port == 22 {
        format!("sh://{}", connection.host)
    } else {
        format!("sh://{}:{}", connection.host, connection.port)
    };

    Some(vec!["mc".to_string(), ".".to_string(), target])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_sftp_uri_with_user_default_port() {
        let uri = build_sftp_uri(Some("admin"), "server.example.com", 22);
        assert_eq!(uri, "sftp://admin@server.example.com");
    }

    #[test]
    fn test_build_sftp_uri_without_user() {
        let uri = build_sftp_uri(None, "10.0.0.1", 22);
        assert_eq!(uri, "sftp://10.0.0.1");
    }

    #[test]
    fn test_build_sftp_uri_custom_port() {
        let uri = build_sftp_uri(Some("root"), "host.local", 2222);
        assert_eq!(uri, "sftp://root@host.local:2222");
    }

    #[test]
    fn test_build_sftp_command_default_port() {
        let mut conn =
            Connection::new_ssh("Test".to_string(), "server.example.com".to_string(), 22);
        conn.username = Some("admin".to_string());

        let cmd = build_sftp_command(&conn).unwrap();
        assert_eq!(cmd, vec!["sftp", "admin@server.example.com"]);
    }

    #[test]
    fn test_build_sftp_command_custom_port() {
        let mut conn = Connection::new_ssh("Test".to_string(), "host.local".to_string(), 2222);
        conn.username = Some("root".to_string());

        let cmd = build_sftp_command(&conn).unwrap();
        assert_eq!(cmd, vec!["sftp", "-P", "2222", "root@host.local"]);
    }

    #[test]
    fn test_build_sftp_command_non_ssh() {
        let conn = Connection::new_rdp("Test".to_string(), "server.example.com".to_string(), 3389);
        assert!(build_sftp_command(&conn).is_none());
    }

    #[test]
    fn test_build_sftp_uri_from_ssh_connection() {
        let mut conn =
            Connection::new_ssh("Test".to_string(), "server.example.com".to_string(), 22);
        conn.username = Some("admin".to_string());

        let uri = build_sftp_uri_from_connection(&conn).unwrap();
        assert_eq!(uri, "sftp://admin@server.example.com");
    }

    #[test]
    fn test_build_sftp_uri_from_non_ssh() {
        let conn = Connection::new_rdp("Test".to_string(), "server.example.com".to_string(), 3389);
        assert!(build_sftp_uri_from_connection(&conn).is_none());
    }

    #[test]
    fn test_build_mc_sftp_command_default_port() {
        let mut conn =
            Connection::new_ssh("Test".to_string(), "server.example.com".to_string(), 22);
        conn.username = Some("admin".to_string());

        let cmd = build_mc_sftp_command(&conn).unwrap();
        assert_eq!(cmd, vec!["mc", ".", "sh://admin@server.example.com"]);
    }

    #[test]
    fn test_build_mc_sftp_command_custom_port() {
        let mut conn = Connection::new_ssh("Test".to_string(), "host.local".to_string(), 2222);
        conn.username = Some("root".to_string());

        let cmd = build_mc_sftp_command(&conn).unwrap();
        assert_eq!(cmd, vec!["mc", ".", "sh://root@host.local:2222"]);
    }

    #[test]
    fn test_build_mc_sftp_command_non_ssh() {
        let conn = Connection::new_rdp("Test".to_string(), "server.example.com".to_string(), 3389);
        assert!(build_mc_sftp_command(&conn).is_none());
    }
}
