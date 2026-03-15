//! Flatpak sandbox detection and host command helpers
//!
//! This module provides utilities for detecting if the application is running
//! inside a Flatpak sandbox and for executing host commands via
//! `flatpak-spawn --host`.
//!
//! Host command execution requires `--talk-name=org.freedesktop.Flatpak`
//! in the Flatpak manifest. This permission is included in the local build
//! manifest but may not be present in Flathub builds. Users can add it via:
//! ```text
//! flatpak override --user --talk-name=org.freedesktop.Flatpak io.github.totoshko88.RustConn
//! ```

use std::sync::OnceLock;

/// Cached result of Flatpak detection
static IS_FLATPAK: OnceLock<bool> = OnceLock::new();

/// Returns a writable SSH directory inside the Flatpak sandbox.
///
/// In Flatpak, `~/.ssh` is mounted read-only. SSH needs a writable location
/// for `known_hosts`. This returns `$XDG_DATA_HOME/../.ssh/` which resolves
/// to `~/.var/app/io.github.totoshko88.RustConn/.ssh/`.
///
/// Returns `None` if not running in Flatpak or if the path cannot be determined.
#[must_use]
pub fn get_flatpak_ssh_dir() -> Option<std::path::PathBuf> {
    if !is_flatpak() {
        return None;
    }

    // XDG_DATA_HOME in Flatpak is ~/.var/app/<app-id>/data
    // We want ~/.var/app/<app-id>/.ssh
    std::env::var("XDG_DATA_HOME").ok().map(|data_home| {
        std::path::PathBuf::from(data_home)
            .parent()
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(".ssh")
    })
}

/// Returns a writable `known_hosts` path for SSH inside the Flatpak sandbox.
///
/// Creates the parent `.ssh` directory if it does not exist.
/// Returns `None` if not running in Flatpak or if the directory cannot be created.
#[must_use]
pub fn get_flatpak_known_hosts_path() -> Option<std::path::PathBuf> {
    let ssh_dir = get_flatpak_ssh_dir()?;

    if !ssh_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&ssh_dir) {
            tracing::warn!(?e, path = %ssh_dir.display(), "Failed to create Flatpak SSH dir");
            return None;
        }
        tracing::debug!(path = %ssh_dir.display(), "Created Flatpak SSH directory");
    }

    Some(ssh_dir.join("known_hosts"))
}

/// Checks if the application is running inside a Flatpak sandbox.
///
/// This function caches the result for performance.
///
/// Detection is based on:
/// 1. Presence of `/.flatpak-info` file (most reliable — only exists inside sandbox)
/// 2. `FLATPAK_ID` environment variable matching our app ID (guards against
///    stray `FLATPAK_ID` from other Flatpak apps or user environment)
#[must_use]
pub fn is_flatpak() -> bool {
    *IS_FLATPAK.get_or_init(|| {
        // Primary check: /.flatpak-info exists only inside a Flatpak sandbox
        if std::path::Path::new("/.flatpak-info").exists() {
            tracing::debug!("Detected Flatpak sandbox via /.flatpak-info");
            return true;
        }

        // Secondary check: FLATPAK_ID must match our app ID to avoid false
        // positives when the env var leaks from another Flatpak process (#59)
        if let Ok(id) = std::env::var("FLATPAK_ID") {
            if id == "io.github.totoshko88.RustConn" {
                tracing::debug!("Detected Flatpak sandbox via FLATPAK_ID");
                return true;
            }
            tracing::debug!(
                flatpak_id = %id,
                "FLATPAK_ID set but does not match our app ID, ignoring"
            );
        }

        false
    })
}

/// Checks whether a CLI tool is available, accounting for Flatpak sandbox.
///
/// - Outside Flatpak: runs `which <cli>` directly.
/// - Inside Flatpak: runs `flatpak-spawn --host which <cli>` to check the host.
///
/// Returns `false` if the tool is not found or if `flatpak-spawn --host` is
/// not permitted (missing `--talk-name=org.freedesktop.Flatpak`).
#[must_use]
pub fn is_host_command_available(cli: &str) -> bool {
    use std::process::Command;

    if !is_flatpak() {
        return Command::new("which")
            .arg(cli)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
    }

    let result = Command::new("flatpak-spawn")
        .arg("--host")
        .arg("which")
        .arg(cli)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();

    match result {
        Ok(status) if status.success() => true,
        Ok(_) => {
            tracing::debug!(cli, "Host command not found via flatpak-spawn");
            false
        }
        Err(e) => {
            tracing::warn!(
                cli,
                ?e,
                "flatpak-spawn --host failed; grant permission with: \
                 flatpak override --user --talk-name=org.freedesktop.Flatpak \
                 io.github.totoshko88.RustConn"
            );
            false
        }
    }
}

/// Wraps a shell command string for host execution when inside Flatpak.
///
/// - Outside Flatpak: returns the command unchanged.
/// - Inside Flatpak: prepends `flatpak-spawn --host` so the shell runs on the host.
///
/// The returned string is suitable for `bash -c "<command>"` or direct VTE spawn.
#[must_use]
pub fn wrap_host_command(command: &str) -> String {
    if is_flatpak() {
        format!("flatpak-spawn --host bash -c {}", shell_escape(command))
    } else {
        command.to_string()
    }
}

/// Shell-escapes a string by wrapping it in single quotes.
fn shell_escape(s: &str) -> String {
    // Replace single quotes with '\'' (end quote, escaped quote, start quote)
    format!("'{}'", s.replace('\'', "'\\''"))
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
    fn test_shell_escape_simple() {
        assert_eq!(shell_escape("hello world"), "'hello world'");
    }

    #[test]
    fn test_shell_escape_with_quotes() {
        assert_eq!(shell_escape("it's"), "'it'\\''s'");
    }

    #[test]
    fn test_wrap_host_command_outside_flatpak() {
        // Outside Flatpak, command is returned unchanged
        if !is_flatpak() {
            let cmd = "aws ssm start-session --target i-123";
            assert_eq!(wrap_host_command(cmd), cmd);
        }
    }
}
