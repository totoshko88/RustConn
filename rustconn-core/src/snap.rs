//! Snap confinement support
//!
//! This module provides utilities for detecting and working with snap confinement,
//! including path management and interface detection.

use std::env;
use std::path::PathBuf;

/// Detects if the application is running inside a snap
#[must_use]
pub fn is_snap() -> bool {
    env::var("SNAP").is_ok()
}

/// Returns the snap user data directory
///
/// This is the directory where the snap can store user-specific data.
/// In snap environment: `~/snap/rustconn/current/`
/// Outside snap: `~/.local/share/rustconn/`
#[must_use]
pub fn get_data_dir() -> Option<PathBuf> {
    if let Ok(snap_user_data) = env::var("SNAP_USER_DATA") {
        return Some(PathBuf::from(snap_user_data));
    }

    // Fallback to XDG_DATA_HOME or ~/.local/share
    if let Ok(xdg_data) = env::var("XDG_DATA_HOME") {
        let mut path = PathBuf::from(xdg_data);
        path.push("rustconn");
        return Some(path);
    }

    if let Ok(home) = env::var("HOME") {
        let mut path = PathBuf::from(home);
        path.push(".local");
        path.push("share");
        path.push("rustconn");
        return Some(path);
    }

    None
}

/// Returns the snap config directory
///
/// This is the directory where the snap can store configuration files.
/// In snap environment: `~/snap/rustconn/current/.config/rustconn/`
/// Outside snap: `~/.config/rustconn/`
#[must_use]
pub fn get_config_dir() -> Option<PathBuf> {
    if let Ok(snap_user_data) = env::var("SNAP_USER_DATA") {
        let mut path = PathBuf::from(snap_user_data);
        path.push(".config");
        path.push("rustconn");
        return Some(path);
    }

    // Fallback to XDG_CONFIG_HOME or ~/.config
    if let Ok(xdg_config) = env::var("XDG_CONFIG_HOME") {
        let mut path = PathBuf::from(xdg_config);
        path.push("rustconn");
        return Some(path);
    }

    if let Ok(home) = env::var("HOME") {
        let mut path = PathBuf::from(home);
        path.push(".config");
        path.push("rustconn");
        return Some(path);
    }

    None
}

/// Returns the SSH config directory
///
/// In snap environment with ssh-keys interface: `~/.ssh/`
/// In snap environment without interface: `~/snap/rustconn/current/.ssh/`
/// Outside snap: `~/.ssh/`
#[must_use]
pub fn get_ssh_dir() -> Option<PathBuf> {
    // Check if SSH_CONFIG_DIR is set (snap environment)
    if let Ok(ssh_dir) = env::var("SSH_CONFIG_DIR") {
        return Some(PathBuf::from(ssh_dir));
    }

    // Check for ssh-keys interface access
    if is_snap() {
        // If ssh-keys interface is connected, we can access ~/.ssh
        if let Ok(home) = env::var("HOME") {
            let ssh_path = PathBuf::from(home).join(".ssh");
            // Test if we can access it
            if ssh_path.exists() {
                return Some(ssh_path);
            }
        }

        // Fallback to snap user data
        if let Ok(snap_user_data) = env::var("SNAP_USER_DATA") {
            let mut path = PathBuf::from(snap_user_data);
            path.push(".ssh");
            return Some(path);
        }
    }

    // Standard location
    if let Ok(home) = env::var("HOME") {
        let mut path = PathBuf::from(home);
        path.push(".ssh");
        return Some(path);
    }

    None
}

/// Returns the path to known_hosts file
///
/// In snap environment: managed by snap in `~/snap/rustconn/current/.ssh/known_hosts`
/// Outside snap: `~/.ssh/known_hosts`
#[must_use]
pub fn get_known_hosts_path() -> Option<PathBuf> {
    get_ssh_dir().map(|mut path| {
        path.push("known_hosts");
        path
    })
}

/// Checks if a snap interface is connected
///
/// This is a best-effort check based on file system access.
/// Returns `true` if the interface appears to be connected.
#[must_use]
pub fn is_interface_connected(interface: &str) -> bool {
    if !is_snap() {
        return true; // Not in snap, all interfaces "connected"
    }

    match interface {
        "ssh-keys" => {
            // Check if we can access ~/.ssh
            if let Ok(home) = env::var("HOME") {
                let ssh_path = PathBuf::from(home).join(".ssh");
                ssh_path.exists() && ssh_path.is_dir()
            } else {
                false
            }
        }
        "home" => {
            // Check if we can access home directory
            if let Ok(home) = env::var("HOME") {
                let home_path = PathBuf::from(home);
                home_path.exists() && home_path.is_dir()
            } else {
                false
            }
        }
        _ => {
            // For other interfaces, assume connected
            // (we can't reliably detect all interfaces)
            true
        }
    }
}

/// Returns a user-friendly message about snap confinement
///
/// This can be shown in the UI to help users understand snap limitations.
#[must_use]
pub fn get_confinement_message() -> Option<String> {
    if !is_snap() {
        return None;
    }

    let mut messages = Vec::new();

    // Check ssh-keys interface
    if !is_interface_connected("ssh-keys") {
        messages.push(
            "SSH keys interface not connected. Run: sudo snap connect rustconn:ssh-keys"
                .to_string(),
        );
    }

    if messages.is_empty() {
        None
    } else {
        Some(messages.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_snap() {
        // This will be false in normal test environment
        // In snap environment, SNAP env var would be set
        let result = is_snap();
        assert!(!result || env::var("SNAP").is_ok());
    }

    #[test]
    fn test_get_data_dir() {
        let dir = get_data_dir();
        assert!(dir.is_some());
        // Should contain "rustconn" in the path
        if let Some(path) = dir {
            let path_str = path.to_string_lossy();
            assert!(
                path_str.contains("rustconn") || path_str.contains("snap"),
                "Path should contain 'rustconn' or 'snap': {path_str}"
            );
        }
    }

    #[test]
    fn test_get_config_dir() {
        let dir = get_config_dir();
        assert!(dir.is_some());
        // Should contain "rustconn" in the path
        if let Some(path) = dir {
            let path_str = path.to_string_lossy();
            assert!(
                path_str.contains("rustconn") || path_str.contains("snap"),
                "Path should contain 'rustconn' or 'snap': {path_str}"
            );
        }
    }

    #[test]
    fn test_get_ssh_dir() {
        let dir = get_ssh_dir();
        assert!(dir.is_some());
        // Should contain ".ssh" in the path
        if let Some(path) = dir {
            let path_str = path.to_string_lossy();
            assert!(
                path_str.contains(".ssh"),
                "Path should contain '.ssh': {path_str}"
            );
        }
    }

    #[test]
    fn test_get_known_hosts_path() {
        let path = get_known_hosts_path();
        assert!(path.is_some());
        // Should end with "known_hosts"
        if let Some(p) = path {
            assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("known_hosts"));
        }
    }

    #[test]
    fn test_is_interface_connected() {
        // In non-snap environment, should always return true
        if !is_snap() {
            assert!(is_interface_connected("ssh-keys"));
            assert!(is_interface_connected("home"));
            assert!(is_interface_connected("network"));
        }
    }

    #[test]
    fn test_get_confinement_message() {
        let message = get_confinement_message();
        // In non-snap environment, should return None
        if !is_snap() {
            assert!(message.is_none());
        }
    }
}
