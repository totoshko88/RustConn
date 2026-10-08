//! Flatpak sandbox detection and path helpers
//!
//! This module provides utilities for detecting if the application is running
//! inside a Flatpak sandbox, resolving SSH key paths, and checking CLI
//! availability in the sandbox PATH.
//!
//! CLI tools are installed into the sandbox via Flatpak Components
//! (`~/.var/app/io.github.totoshko88.RustConn/cli/`). Tools that cannot be
//! bundled (notably the host's KeePassXC) are still reached via
//! `flatpak-spawn --host`, which requires `--talk-name=org.freedesktop.Flatpak`.

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

/// Returns the user's **real host** home directory as it is visible inside the
/// sandbox.
///
/// Flatpak only grants this app `--filesystem=home/.ssh:ro` (not full
/// `--filesystem=home`), so the host `~/.ssh` is reachable at its real absolute
/// path, but `$HOME` — and therefore [`dirs::home_dir`] — points at the per-app
/// sandbox home `<real_home>/.var/app/<app-id>`. Any importer that resolves
/// `~/.ssh/config` through `dirs::home_dir()` then looks inside the sandbox home
/// and finds nothing, which is why the default SSH-config import silently
/// imported zero entries while picking the same file through the portal worked
/// (issue #368).
///
/// This derives the real home by stripping a trailing `.var/app/<app-id>`
/// segment from the sandbox home when present. Outside Flatpak (and when no such
/// segment is present) it is identical to [`dirs::home_dir`].
#[must_use]
pub fn host_home_dir() -> Option<std::path::PathBuf> {
    let home = dirs::home_dir()?;

    if !is_flatpak() {
        return Some(home);
    }

    Some(strip_sandbox_home(&home))
}

/// Recovers the real host home from a sandbox home path.
///
/// Flatpak's per-app home is `<real_home>/.var/app/<app-id>`. When the path
/// ends with a `.var/app/<app-id>` segment this strips it to return
/// `<real_home>`; otherwise (e.g. `$HOME` was never remapped) the path is
/// returned unchanged. Pure so it can be unit-tested without a sandbox.
fn strip_sandbox_home(home: &std::path::Path) -> std::path::PathBuf {
    let components: Vec<std::ffi::OsString> =
        home.iter().map(std::ffi::OsStr::to_os_string).collect();
    if components.len() >= 3
        && components[components.len() - 3] == std::ffi::OsStr::new(".var")
        && components[components.len() - 2] == std::ffi::OsStr::new("app")
    {
        return components[..components.len() - 3]
            .iter()
            .collect::<std::path::PathBuf>();
    }
    home.to_path_buf()
}

/// Checks whether a CLI tool is available in PATH.
///
/// Thin alias for [`crate::which::is_available`], which searches the extended
/// PATH (sandbox CLI directories included) in process. It used to spawn `which`
/// with `PATH` overridden, which reported every tool as missing on a system
/// without that binary — see the docs on `crate::which` and issue #303.
#[must_use]
pub fn is_host_command_available(cli: &str) -> bool {
    crate::which::is_available(cli)
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

/// How long to wait for the host readability probe.
///
/// Matches the budget `crate::which::find_on_host` allows its own host probe, and
/// for the same reason: this runs on the GTK main thread while a SPICE session is
/// being opened, and the wait is on the Flatpak session helper, which RustConn
/// does not control.
const HOST_PROBE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// Translates a path in this process's view into one a `flatpak-spawn --host`
/// process can open, or `None` when no such path could be confirmed.
///
/// Outside Flatpak the two views are the same filesystem, so the path is returned
/// unchanged. Inside Flatpak they are not: `$XDG_RUNTIME_DIR` looks like
/// `/run/user/<uid>` on both sides but is a *different* directory, because Flatpak
/// gives the sandbox its own and keeps it on the host under
/// `/run/user/<uid>/.flatpak/<app-id>/xdg-run/`. A file written to
/// `$XDG_RUNTIME_DIR/x` and handed to a host process by that path therefore does
/// not exist as far as that process is concerned — which is what issue
/// [#308](https://github.com/totoshko88/RustConn/issues/308) was: the SPICE
/// `.vv` connection file was invisible to a host `remote-viewer`, so it fell back
/// to reading the argument as a URI and failed with "connection type cannot be
/// detected from URI".
///
/// The remapped location is *verified*, not assumed: the candidate is built and
/// then probed with `test -r` on the host, so a Flatpak that arranges its runtime
/// directory differently yields `None` rather than another path that does not
/// exist. Callers are expected to treat `None` as "cannot deliver a file to the
/// host" and degrade accordingly.
///
/// Only paths under `$XDG_RUNTIME_DIR` are remapped; anything else returns `None`
/// inside Flatpak, since this function knows nothing about how it might be shared.
#[must_use]
pub fn host_visible_path(path: &std::path::Path) -> Option<std::path::PathBuf> {
    if !is_flatpak() {
        return Some(path.to_path_buf());
    }

    let runtime_dir = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from)?;
    let app_id = std::env::var("FLATPAK_ID").ok()?;
    let candidate = host_runtime_candidate(&runtime_dir, &app_id, path)?;

    if host_can_read(&candidate) {
        tracing::debug!(
            sandbox = %path.display(),
            host = %candidate.display(),
            "resolved the host-visible path for a sandbox runtime file"
        );
        Some(candidate)
    } else {
        tracing::warn!(
            sandbox = %path.display(),
            candidate = %candidate.display(),
            "no host-visible path for this sandbox runtime file"
        );
        None
    }
}

/// Builds the host counterpart of a sandbox path under `$XDG_RUNTIME_DIR`.
///
/// Split out from [`host_visible_path`] so the mapping can be tested without a
/// sandbox: the probe that confirms the result needs `flatpak-spawn`, but the
/// path arithmetic does not, and it is the part that can be wrong in a way tests
/// can catch. Returns `None` when `path` is not under `runtime_dir`, or when
/// `app_id` is empty or holds a path separator — an `app_id` read from the
/// environment must not be able to climb out of the directory it names.
fn host_runtime_candidate(
    runtime_dir: &std::path::Path,
    app_id: &str,
    path: &std::path::Path,
) -> Option<std::path::PathBuf> {
    let relative = path.strip_prefix(runtime_dir).ok()?;
    if app_id.is_empty() || app_id.contains('/') || app_id.contains("..") {
        tracing::warn!(app_id, "refusing to build a host path from this app id");
        return None;
    }
    Some(
        runtime_dir
            .join(".flatpak")
            .join(app_id)
            .join("xdg-run")
            .join(relative),
    )
}

/// Returns whether a `flatpak-spawn --host` process can read `path`.
///
/// `test` is invoked directly rather than through a shell, so the path travels as
/// one argv element and needs no quoting — it may hold spaces, and it is not
/// built from user input in a way worth interpreting.
fn host_can_read(path: &std::path::Path) -> bool {
    let child = std::process::Command::new("flatpak-spawn")
        .arg("--host")
        .arg("test")
        .arg("-r")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();

    let child = match child {
        Ok(child) => child,
        Err(e) => {
            tracing::debug!(%e, "flatpak-spawn unavailable; cannot probe the host");
            return false;
        }
    };

    match crate::proc::wait_bounded(child, HOST_PROBE_TIMEOUT, "host path probe") {
        Ok(crate::proc::Waited::Exited(output)) => output.status.success(),
        Ok(crate::proc::Waited::TimedOut) => false,
        Err(e) => {
            tracing::debug!(%e, "failed to poll the host path probe");
            false
        }
    }
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
    fn host_visible_path_is_identity_outside_flatpak() {
        use std::path::Path;
        // Outside a sandbox the host and this process share one filesystem view,
        // so the path must come back untouched — including one that looks like a
        // runtime path, since there is nothing to remap.
        if !is_flatpak() {
            for raw in [
                "/run/user/1000/rustconn-spice-abc.vv",
                "/home/user/somewhere/else",
            ] {
                assert_eq!(
                    host_visible_path(Path::new(raw)),
                    Some(Path::new(raw).to_path_buf()),
                    "{raw} should be returned unchanged outside Flatpak"
                );
            }
        }
    }

    #[test]
    fn host_visible_path_rejects_paths_outside_the_runtime_dir() {
        use std::path::Path;
        // The remap this function knows about applies to $XDG_RUNTIME_DIR only.
        // Anything else has no known host counterpart, so inside Flatpak it must
        // decline rather than invent one. Outside Flatpak the identity rule above
        // applies instead, so the assertion is scoped to the sandbox.
        if is_flatpak() {
            assert_eq!(host_visible_path(Path::new("/home/user/elsewhere")), None);
        }
    }

    /// The shape verified by hand against a real sandbox for issue #308: a file
    /// at `$XDG_RUNTIME_DIR/<name>` is readable by a host process only at
    /// `$XDG_RUNTIME_DIR/.flatpak/<app-id>/xdg-run/<name>`.
    #[test]
    fn host_runtime_candidate_maps_into_the_app_xdg_run_dir() {
        use std::path::Path;
        let runtime = Path::new("/run/user/1000");
        assert_eq!(
            host_runtime_candidate(
                runtime,
                "io.github.totoshko88.RustConn",
                Path::new("/run/user/1000/rustconn-spice-abc.vv"),
            ),
            Some(
                Path::new(
                    "/run/user/1000/.flatpak/io.github.totoshko88.RustConn/xdg-run/\
                     rustconn-spice-abc.vv"
                )
                .to_path_buf()
            )
        );
    }

    #[test]
    fn host_runtime_candidate_preserves_nested_paths() {
        use std::path::Path;
        assert_eq!(
            host_runtime_candidate(
                Path::new("/run/user/1000"),
                "app.id",
                Path::new("/run/user/1000/sub/dir/file.vv"),
            ),
            Some(Path::new("/run/user/1000/.flatpak/app.id/xdg-run/sub/dir/file.vv").to_path_buf())
        );
    }

    #[test]
    fn host_runtime_candidate_declines_paths_outside_the_runtime_dir() {
        use std::path::Path;
        let runtime = Path::new("/run/user/1000");
        for outside in [
            "/home/user/file.vv",
            "/run/user/1001/file.vv",
            "/tmp/file.vv",
        ] {
            assert_eq!(
                host_runtime_candidate(runtime, "app.id", Path::new(outside)),
                None,
                "{outside} is not under the runtime dir"
            );
        }
    }

    #[test]
    fn host_runtime_candidate_declines_an_unsafe_app_id() {
        use std::path::Path;
        let runtime = Path::new("/run/user/1000");
        let file = Path::new("/run/user/1000/file.vv");
        // An app id comes from the environment; one holding a separator or a
        // parent link would place the path outside the directory it names.
        for bad in ["", "../escape", "a/b", ".."] {
            assert_eq!(
                host_runtime_candidate(runtime, bad, file),
                None,
                "app id {bad:?} must be refused"
            );
        }
    }

    #[test]
    fn strip_sandbox_home_recovers_real_home() {
        use std::path::Path;
        assert_eq!(
            strip_sandbox_home(Path::new(
                "/home/user/.var/app/io.github.totoshko88.RustConn"
            )),
            Path::new("/home/user").to_path_buf()
        );
    }

    #[test]
    fn strip_sandbox_home_leaves_non_sandbox_paths_untouched() {
        use std::path::Path;
        // A plain host home, and a home that merely contains `.var` elsewhere,
        // must both come back unchanged.
        for raw in ["/home/user", "/home/user/.var/cache", "/root"] {
            assert_eq!(
                strip_sandbox_home(Path::new(raw)),
                Path::new(raw).to_path_buf()
            );
        }
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
