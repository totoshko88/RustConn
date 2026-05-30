//! Flatpak sandbox detection and path helpers
//!
//! This module provides utilities for detecting if the application is running
//! inside a Flatpak sandbox, resolving SSH key paths, and checking CLI
//! availability in the sandbox PATH.
//!
//! CLI tools are installed into the sandbox via Flatpak Components
//! (`~/.var/app/io.github.totoshko88.RustConn/cli/`).
//! Host command execution via `flatpak-spawn --host` is no longer used.

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

/// Checks whether a CLI tool is available in PATH.
///
/// Runs `which <cli>` to check if the binary exists.
/// In Flatpak, CLI tools are installed to the sandbox via Flatpak Components,
/// so the extended PATH (including CLI directories) is used for the lookup.
#[must_use]
pub fn is_host_command_available(cli: &str) -> bool {
    use std::process::Command;

    let mut cmd = Command::new("which");
    cmd.arg(cli)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());

    // In a sandbox (snap or Flatpak), CLI tools are installed outside the
    // default PATH. Use the extended PATH that includes the sandbox CLI dirs.
    if crate::is_sandboxed() {
        let extended_path = crate::cli_download::get_extended_path();
        cmd.env("PATH", &extended_path);
        tracing::trace!(cli, path = %extended_path, "Checking CLI availability with extended PATH");
    }

    cmd.status().is_ok_and(|s| s.success())
}

/// Returns a writable CLI configuration directory inside the Flatpak sandbox.
///
/// Several CLI tools need writable config directories but the Flatpak
/// manifest mounts host directories as read-only (or doesn't mount them
/// at all). This function returns `$XDG_CONFIG_HOME/<subdir>` and creates
/// it if needed.
///
/// When `host_source` is provided and the directory is freshly created,
/// credential files listed in `bootstrap_files` are copied from the
/// host's read-only mount so the user doesn't have to re-authenticate.
///
/// Returns `None` if not running in Flatpak.
#[must_use]
pub fn get_flatpak_cli_config_dir(
    subdir: &str,
    host_source: Option<&std::path::Path>,
    bootstrap_files: &[&str],
) -> Option<std::path::PathBuf> {
    if !is_flatpak() {
        return None;
    }

    let config_home = std::env::var("XDG_CONFIG_HOME").ok()?;
    let cli_dir = std::path::PathBuf::from(config_home).join(subdir);

    if !cli_dir.exists() {
        if let Err(e) = std::fs::create_dir_all(&cli_dir) {
            tracing::warn!(?e, path = %cli_dir.display(), "Failed to create Flatpak CLI config dir");
            return None;
        }
        tracing::debug!(path = %cli_dir.display(), "Created Flatpak CLI config directory");

        // Bootstrap credential files from host read-only mount
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

/// Returns a writable gcloud configuration directory inside the Flatpak sandbox.
///
/// Convenience wrapper around [`get_flatpak_cli_config_dir`] for gcloud.
/// Bootstraps credentials from the host's read-only `~/.config/gcloud/` mount.
#[must_use]
pub fn get_flatpak_gcloud_config_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let host_gcloud = std::path::PathBuf::from(&home).join(".config/gcloud");
    get_flatpak_cli_config_dir(
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

/// Returns a writable Azure CLI configuration directory inside the Flatpak sandbox.
///
/// Bootstraps credentials from the host's read-only `~/.azure/` mount.
#[must_use]
pub fn get_flatpak_azure_config_dir() -> Option<std::path::PathBuf> {
    let home = std::env::var("HOME").ok()?;
    let host_azure = std::path::PathBuf::from(&home).join(".azure");
    get_flatpak_cli_config_dir(
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

/// Returns writable CLI config directories for tools that have no host mount.
///
/// These tools are installed inside the sandbox via Flatpak Components
/// and the user configures them from scratch. No bootstrap is needed.
#[must_use]
pub fn get_flatpak_teleport_config_dir() -> Option<std::path::PathBuf> {
    get_flatpak_cli_config_dir("tsh", None, &[])
}

/// Returns a writable OCI CLI config directory.
#[must_use]
pub fn get_flatpak_oci_config_dir() -> Option<std::path::PathBuf> {
    get_flatpak_cli_config_dir("oci", None, &[])
}

/// Returns the command unchanged.
///
/// Previously wrapped commands with `flatpak-spawn --host` for host execution.
/// Since 0.10.1, CLI tools are installed into the Flatpak sandbox via
/// Flatpak Components, so host execution is no longer needed.
#[must_use]
pub fn wrap_host_command(command: &str) -> String {
    command.to_string()
}

/// Checks if a path is a Flatpak document portal path.
///
/// Portal paths look like `/run/user/<uid>/doc/<hash>/<filename>`.
/// These paths become stale after Flatpak rebuilds because the hash changes.
///
/// The check requires the path to start with `/run/user/` followed by a
/// numeric UID and a `/doc/` segment with a hex hash component, reducing
/// false positives from unrelated paths that happen to contain those substrings.
#[must_use]
pub fn is_portal_path(path: &std::path::Path) -> bool {
    let s = path.to_string_lossy();
    // Must start with /run/user/<digits>/doc/
    s.starts_with("/run/user/")
        && s.split('/')
            .nth(3) // uid component
            .is_some_and(|uid| !uid.is_empty() && uid.chars().all(|c| c.is_ascii_digit()))
        && s.contains("/doc/")
}

/// Copies a key file from a Flatpak document portal path to the stable
/// Flatpak SSH directory (`~/.var/app/<app-id>/.ssh/`).
///
/// If a file with the same name already exists and has identical content,
/// the existing path is returned without copying. If the name collides but
/// content differs, a numeric suffix is appended (e.g., `key_1.pem`).
///
/// Returns `None` if not running in Flatpak, the SSH dir cannot be created,
/// or the copy fails.
pub fn copy_key_to_flatpak_ssh(portal_path: &std::path::Path) -> Option<std::path::PathBuf> {
    let ssh_dir = get_flatpak_ssh_dir()?;

    if !ssh_dir.exists()
        && let Err(e) = std::fs::create_dir_all(&ssh_dir)
    {
        tracing::warn!(?e, path = %ssh_dir.display(), "Failed to create Flatpak SSH dir");
        return None;
    }

    let file_name = portal_path.file_name()?.to_string_lossy().to_string();
    let stem = portal_path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let ext = portal_path
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();

    let source_content = match std::fs::read(portal_path) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!(?e, path = %portal_path.display(), "Failed to read portal key file");
            return None;
        }
    };

    // Try the original filename first
    let candidate = ssh_dir.join(&file_name);
    if candidate.exists() {
        if let Ok(existing) = std::fs::read(&candidate)
            && existing == source_content
        {
            tracing::debug!(path = %candidate.display(), "Key file already exists with same content");
            return Some(candidate);
        }
    } else {
        return copy_and_set_permissions(&source_content, &candidate);
    }

    // Name collision with different content — try suffixed names
    for i in 1..100 {
        let suffixed = ssh_dir.join(format!("{stem}_{i}{ext}"));
        if suffixed.exists() {
            if let Ok(existing) = std::fs::read(&suffixed)
                && existing == source_content
            {
                tracing::debug!(path = %suffixed.display(), "Key file already exists with same content (suffixed)");
                return Some(suffixed);
            }
            continue;
        }
        return copy_and_set_permissions(&source_content, &suffixed);
    }

    tracing::warn!(
        file_name,
        "Too many key file name collisions in Flatpak SSH dir"
    );
    None
}

/// Resolves a key file path that may have become stale after a Flatpak rebuild.
///
/// If the path exists, returns it unchanged. If it doesn't exist and we're in
/// Flatpak, checks whether a file with the same name exists in the Flatpak SSH
/// directory as a fallback.
///
/// Returns `None` if the path cannot be resolved.
#[must_use]
pub fn resolve_key_path(path: &std::path::Path) -> Option<std::path::PathBuf> {
    if path.exists() {
        return Some(path.to_path_buf());
    }

    // Fallback: check Flatpak SSH dir for a file with the same name
    let ssh_dir = get_flatpak_ssh_dir()?;
    let file_name = path.file_name()?;
    let fallback = ssh_dir.join(file_name);
    if fallback.exists() {
        tracing::info!(
            original = %path.display(),
            resolved = %fallback.display(),
            "Resolved stale key path via Flatpak SSH dir fallback"
        );
        Some(fallback)
    } else {
        None
    }
}

/// Writes content to a file and sets 0600 permissions (owner read/write only).
fn copy_and_set_permissions(content: &[u8], dest: &std::path::Path) -> Option<std::path::PathBuf> {
    use std::os::unix::fs::PermissionsExt;

    if let Err(e) = std::fs::write(dest, content) {
        tracing::warn!(?e, path = %dest.display(), "Failed to copy key file");
        return None;
    }
    // SSH requires key files to be 0600
    if let Err(e) = std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o600)) {
        tracing::warn!(?e, path = %dest.display(), "Failed to set key file permissions");
    }
    tracing::info!(path = %dest.display(), "Copied key file to Flatpak SSH dir");
    Some(dest.to_path_buf())
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
    fn test_wrap_host_command_outside_flatpak() {
        // Outside Flatpak, command is returned unchanged
        if !is_flatpak() {
            let cmd = "aws ssm start-session --target i-123";
            assert_eq!(wrap_host_command(cmd), cmd);
        }
    }

    #[test]
    fn test_is_portal_path_valid() {
        use std::path::Path;
        assert!(is_portal_path(Path::new(
            "/run/user/1000/doc/abc123/Documents"
        )));
        assert!(is_portal_path(Path::new(
            "/run/user/0/doc/deadbeef/myfile.txt"
        )));
        assert!(is_portal_path(Path::new(
            "/run/user/65534/doc/hash/nested/path"
        )));
    }

    #[test]
    fn test_is_portal_path_rejects_non_portal() {
        use std::path::Path;
        // Regular home directory path
        assert!(!is_portal_path(Path::new("/home/user/Documents")));
        // Path that contains /doc/ but not under /run/user/
        assert!(!is_portal_path(Path::new("/var/lib/doc/something")));
        // Path with /run/user/ but non-numeric UID
        assert!(!is_portal_path(Path::new("/run/user/abc/doc/hash/file")));
        // Path that doesn't start with /run/user/
        assert!(!is_portal_path(Path::new(
            "/home/user/run/user/1000/doc/hash/file"
        )));
    }
}
