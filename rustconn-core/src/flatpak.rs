//! Flatpak sandbox detection and host command execution
//!
//! This module provides utilities for detecting if the application is running
//! inside a Flatpak sandbox and executing host commands via `flatpak-spawn`.
//!
//! When running in Flatpak, external CLI tools (xfreerdp, vncviewer, aws, etc.)
//! are not available inside the sandbox. To use them, we need to spawn them
//! on the host system using `flatpak-spawn --host`.

use std::process::{Command, Output, Stdio};
use std::sync::OnceLock;

/// Cached result of Flatpak detection
static IS_FLATPAK: OnceLock<bool> = OnceLock::new();

/// Checks if the application is running inside a Flatpak sandbox.
///
/// This function caches the result for performance.
///
/// Detection is based on:
/// 1. Presence of `/.flatpak-info` file (most reliable)
/// 2. `FLATPAK_ID` environment variable
#[must_use]
pub fn is_flatpak() -> bool {
    *IS_FLATPAK.get_or_init(|| {
        // Primary check: /.flatpak-info exists in Flatpak sandbox
        if std::path::Path::new("/.flatpak-info").exists() {
            tracing::debug!("Detected Flatpak sandbox via /.flatpak-info");
            return true;
        }

        // Secondary check: FLATPAK_ID environment variable
        if std::env::var("FLATPAK_ID").is_ok() {
            tracing::debug!("Detected Flatpak sandbox via FLATPAK_ID env var");
            return true;
        }

        false
    })
}

/// Creates a command that will run on the host system if in Flatpak,
/// or directly if not in Flatpak.
///
/// When in Flatpak, this wraps the command with `flatpak-spawn --host`.
///
/// # Arguments
///
/// * `program` - The program to execute
///
/// # Example
///
/// ```ignore
/// use rustconn_core::flatpak::host_command;
///
/// let mut cmd = host_command("xfreerdp");
/// cmd.arg("/v:server.example.com");
/// let output = cmd.output()?;
/// ```
#[must_use]
pub fn host_command(program: &str) -> Command {
    if is_flatpak() {
        let mut cmd = Command::new("flatpak-spawn");
        cmd.arg("--host").arg(program);
        cmd
    } else {
        Command::new(program)
    }
}

/// Checks if a program exists on the host system.
///
/// Uses `which` command, wrapped with `flatpak-spawn --host` if in Flatpak.
///
/// # Arguments
///
/// * `program` - The program name to check
///
/// # Returns
///
/// `true` if the program is found, `false` otherwise
#[must_use]
pub fn host_has_command(program: &str) -> bool {
    let output = host_command("which")
        .arg(program)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();

    output.is_ok_and(|s| s.success())
}

/// Gets the path of a program on the host system.
///
/// Uses `which` command, wrapped with `flatpak-spawn --host` if in Flatpak.
///
/// # Arguments
///
/// * `program` - The program name to find
///
/// # Returns
///
/// The full path to the program if found, `None` otherwise
#[must_use]
pub fn host_which(program: &str) -> Option<String> {
    let output = host_command("which").arg(program).output().ok()?;

    if output.status.success() {
        let path = String::from_utf8_lossy(&output.stdout);
        let path = path.trim();
        if !path.is_empty() {
            return Some(path.to_string());
        }
    }

    None
}

/// Executes a command on the host and returns its output.
///
/// This is a convenience function for simple command execution.
///
/// # Arguments
///
/// * `program` - The program to execute
/// * `args` - Arguments to pass to the program
///
/// # Returns
///
/// The command output if successful
///
/// # Errors
///
/// Returns an error if the command fails to execute.
pub fn host_exec(program: &str, args: &[&str]) -> std::io::Result<Output> {
    host_command(program).args(args).output()
}

/// Spawns a command on the host system without waiting for it.
///
/// Useful for launching GUI applications like xfreerdp or vncviewer.
///
/// # Arguments
///
/// * `program` - The program to execute
/// * `args` - Arguments to pass to the program
///
/// # Returns
///
/// The child process handle if successful
///
/// # Errors
///
/// Returns an error if the command fails to spawn.
pub fn host_spawn(program: &str, args: &[&str]) -> std::io::Result<std::process::Child> {
    host_command(program).args(args).spawn()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_flatpak_detection() {
        // This test will return false in normal test environment
        // and true only when actually running in Flatpak
        let result = is_flatpak();
        // Just verify it doesn't panic and returns a boolean
        // The result depends on the environment
        let _ = result;
    }

    #[test]
    fn test_host_command_creates_command() {
        let cmd = host_command("echo");
        // Verify command was created (we can't easily test the actual command)
        assert!(
            format!("{cmd:?}").contains("echo") || format!("{cmd:?}").contains("flatpak-spawn")
        );
    }

    #[test]
    fn test_host_has_command_for_common_tools() {
        // These should exist on most Linux systems
        // Note: In Flatpak, this will check the host system
        let has_sh = host_has_command("sh");
        // sh should always exist
        assert!(has_sh || is_flatpak()); // May fail in Flatpak if host doesn't have sh in PATH
    }
}
