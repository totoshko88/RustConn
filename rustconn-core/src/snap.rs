//! Snap confinement support
//!
//! This module provides utilities for detecting and working with snap confinement,
//! including path management and interface detection.

use std::env;
use std::path::{Path, PathBuf};

/// Detects if the application is running inside a snap
#[must_use]
pub fn is_snap() -> bool {
    env::var("SNAP").is_ok()
}

/// Returns `true` when running inside any confined sandbox (snap or Flatpak).
///
/// Both sandboxes share the same constraint: external CLI tools are not
/// available on the host `PATH` and must be installed into a writable,
/// per-application directory (see [`crate::cli_download`]). Use this in
/// preference to checking [`is_snap`] / [`crate::flatpak::is_flatpak`]
/// individually when the behaviour should be identical for both.
#[must_use]
pub fn is_sandboxed() -> bool {
    is_snap() || crate::flatpak::is_flatpak()
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

/// Returns the writable CLI installation directory inside the snap sandbox.
///
/// Mirrors [`crate::cli_download::get_cli_install_dir`] for Flatpak. External
/// CLI tools (cloud providers, password managers) are downloaded here at
/// runtime because strict confinement does not expose host binaries.
///
/// In a snap: `$SNAP_USER_DATA/cli/` (writable, persists across the snap's
/// revisions via the `current` symlink). Returns `None` when not in a snap.
#[must_use]
pub fn get_cli_install_dir() -> Option<PathBuf> {
    if !is_snap() {
        return None;
    }

    if let Ok(snap_user_data) = env::var("SNAP_USER_DATA") {
        return Some(PathBuf::from(snap_user_data).join("cli"));
    }

    // Fallback: derive from data dir (keeps behaviour sane if SNAP_USER_DATA
    // is unset for any reason).
    get_data_dir().map(|d| d.join("cli"))
}

/// Returns a writable CLI configuration directory inside the snap sandbox.
///
/// Mirrors [`crate::flatpak::get_flatpak_cli_config_dir`]. CLI tools such as
/// gcloud and the Azure CLI need a writable config directory; under strict
/// confinement the host's `~/.config/<tool>` is either unavailable or
/// connected read-only via a `personal-files` plug. This returns
/// `$SNAP_USER_DATA/.config/<subdir>` and creates it if needed.
///
/// When `host_source` is provided and the directory is freshly created,
/// credential files listed in `bootstrap_files` are copied from the host's
/// (read-only) mount so the user does not have to re-authenticate.
///
/// Returns `None` when not running in a snap.
#[must_use]
pub fn get_snap_cli_config_dir(
    subdir: &str,
    host_source: Option<&Path>,
    bootstrap_files: &[&str],
) -> Option<PathBuf> {
    if !is_snap() {
        return None;
    }

    let base = env::var("SNAP_USER_DATA")
        .map(PathBuf::from)
        .ok()
        .or_else(get_data_dir)?;
    let cli_dir = base.join(".config").join(subdir);

    if !cli_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&cli_dir) {
            tracing::warn!(?e, path = %cli_dir.display(), "Failed to create snap CLI config dir");
            return None;
        }
        tracing::debug!(path = %cli_dir.display(), "Created snap CLI config directory");

        if let Some(host_dir) = host_source
            && host_dir.exists()
        {
            for name in bootstrap_files {
                let src = host_dir.join(name);
                let dst = cli_dir.join(name);
                if src.exists() && !dst.exists() {
                    if let Err(e) = std::fs::copy(&src, &dst) {
                        tracing::warn!(?e, file = %name, "Failed to bootstrap CLI credential file");
                    } else {
                        tracing::info!(file = %name, "Bootstrapped CLI credential file from host");
                    }
                }
            }
        }
    }

    Some(cli_dir)
}

/// Returns a writable gcloud configuration directory inside the snap sandbox.
///
/// Bootstraps credentials from the host's `~/.config/gcloud/` mount (granted
/// via the `gcloud-credentials` personal-files plug).
#[must_use]
pub fn get_snap_gcloud_config_dir() -> Option<PathBuf> {
    let home = env::var("HOME").ok()?;
    let host_gcloud = PathBuf::from(&home).join(".config/gcloud");
    get_snap_cli_config_dir(
        "gcloud",
        Some(&host_gcloud),
        &[
            "credentials.db",
            "application_default_credentials.json",
            "properties",
            "access_tokens.db",
        ],
    )
}

/// Returns a writable Azure CLI configuration directory inside the snap sandbox.
///
/// Bootstraps credentials from the host's `~/.azure/` mount (granted via the
/// `azure-credentials` personal-files plug).
#[must_use]
pub fn get_snap_azure_config_dir() -> Option<PathBuf> {
    let home = env::var("HOME").ok()?;
    let host_azure = PathBuf::from(&home).join(".azure");
    get_snap_cli_config_dir(
        "azure",
        Some(&host_azure),
        &[
            "azureProfile.json",
            "clouds.config",
            "msal_token_cache.json",
            "msal_token_cache.bin",
        ],
    )
}

/// Returns a writable Teleport (`tsh`) config directory inside the snap sandbox.
///
/// No host mount exists for `~/.tsh`, so nothing is bootstrapped.
#[must_use]
pub fn get_snap_teleport_config_dir() -> Option<PathBuf> {
    get_snap_cli_config_dir("tsh", None, &[])
}

/// Returns a writable OCI CLI config directory inside the snap sandbox.
#[must_use]
pub fn get_snap_oci_config_dir() -> Option<PathBuf> {
    get_snap_cli_config_dir("oci", None, &[])
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
/// Checks whether key interfaces are connected and provides `snap connect`
/// commands for any that are missing.
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

    // Check personal-files plugs for cloud credentials (manual-connect).
    // These are only relevant when the user actually has the corresponding
    // config directories on the host.
    let credential_plugs: &[(&str, &str)] = &[
        ("aws-credentials", ".aws"),
        ("gcloud-credentials", ".config/gcloud"),
        ("azure-credentials", ".azure"),
        ("oci-credentials", ".oci"),
        ("kube-credentials", ".kube"),
    ];

    if let Ok(home) = env::var("HOME") {
        let home_path = Path::new(&home);
        for &(plug, subdir) in credential_plugs {
            let host_dir = home_path.join(subdir);
            // Only warn if the host directory exists (user has the tool
            // configured) but we cannot read it (plug not connected).
            if host_dir.exists() && std::fs::read_dir(&host_dir).is_err() {
                messages.push(format!(
                    "~/{subdir} not accessible. Run: sudo snap connect rustconn:{plug}"
                ));
            }
        }
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

    #[test]
    fn test_is_sandboxed_matches_components() {
        // is_sandboxed() must be true iff either snap or Flatpak is detected.
        assert_eq!(is_sandboxed(), is_snap() || crate::flatpak::is_flatpak());
    }

    #[test]
    fn test_cli_install_dir_outside_snap() {
        // Outside a snap, the snap-specific CLI install dir must be None so
        // the Flatpak/sandbox logic in cli_download can take over.
        if !is_snap() {
            assert!(get_cli_install_dir().is_none());
        }
    }

    #[test]
    fn test_cli_config_dirs_outside_snap() {
        // Snap config-dir helpers return None outside a snap; they must never
        // touch the filesystem in that case.
        if !is_snap() {
            assert!(get_snap_gcloud_config_dir().is_none());
            assert!(get_snap_azure_config_dir().is_none());
            assert!(get_snap_teleport_config_dir().is_none());
            assert!(get_snap_oci_config_dir().is_none());
            assert!(get_snap_cli_config_dir("tsh", None, &[]).is_none());
        }
    }
}
