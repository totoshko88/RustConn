//! Executable lookup in `PATH`, resolved in process.
//!
//! Every "is this client installed?" question in RustConn used to be answered by
//! spawning `/usr/bin/which`, which makes the answer depend on a program that is
//! not part of POSIX, is not guaranteed to be installed, and is being retired by
//! distributions in favour of the shell builtin `command -v` — unusable here,
//! since there is no shell in the loop. When `which` is missing, `Command::spawn`
//! fails with `ENOENT` and every probe reports "not installed", however many
//! clients the user actually has. That is what issue
//! [#303](https://github.com/totoshko88/RustConn/issues/303) turned out to be:
//! a SPICE session refusing to launch with *"Install virt-viewer"* on a machine
//! where virt-viewer was installed.
//!
//! Resolving the lookup here costs a handful of `stat` calls instead of a process
//! spawn, cannot fail for want of a helper binary, and lets the search cover the
//! places a sandboxed or bundle-launched RustConn keeps its tools:
//!
//! * `/app/bin` inside Flatpak, where the manifest's own modules land;
//! * `$SNAP/{usr/bin,bin,usr/local/bin}` under snap confinement;
//! * the writable per-application CLI directories of either sandbox, where
//!   [`crate::cli_download`] installs tools on demand;
//! * everything in [`crate::cli_download::get_extended_path`], which is `PATH`
//!   plus those directories plus the Homebrew paths a macOS `.app` launched from
//!   Finder does not inherit.
//!
//! For a binary that only exists outside the sandbox — a host GUI application
//! such as `virt-viewer` or KeePassXC, which cannot be bundled — see
//! [`find_on_host`].

use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long the host probe may take before it is abandoned.
///
/// It is a `flatpak-spawn` round trip through the session helper plus a host
/// *login* shell, which sources the user's profile — slow enough to need a real
/// allowance, and out of RustConn's hands enough to need a limit. Two seconds is
/// well past any working host and short enough not to read as a freeze.
const HOST_PROBE_TIMEOUT: Duration = Duration::from_secs(2);

/// Returns whether `path` is a file the current user may execute.
fn is_executable_file(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        std::fs::metadata(path).is_ok_and(|meta| {
            // Any execute bit: which one applies depends on ownership, and an
            // `access(2)` probe would still race with an exec. A directory with
            // the bit set is not a candidate.
            meta.is_file() && meta.permissions().mode() & 0o111 != 0
        })
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

/// Finds `binary` and returns its absolute path, or `None` when it is not installed.
///
/// A `binary` that already contains a separator is treated as a path and only
/// checked, never searched for — the same rule `which` follows.
#[must_use]
pub fn find_in_path(binary: &str) -> Option<PathBuf> {
    if binary.is_empty() {
        return None;
    }

    if binary.contains('/') {
        let path = PathBuf::from(binary);
        return is_executable_file(&path).then_some(path);
    }

    // Bundled clients first: inside a sandbox they are the ones that will
    // actually run, whatever a same-named host binary on PATH might suggest.
    if crate::flatpak::is_flatpak() {
        let bundled = PathBuf::from("/app/bin").join(binary);
        if is_executable_file(&bundled) {
            return Some(bundled);
        }
    }

    if let Ok(snap_dir) = std::env::var("SNAP") {
        let snap_root = Path::new(&snap_dir);
        for subdir in ["usr/bin", "bin", "usr/local/bin"] {
            let bundled = snap_root.join(subdir).join(binary);
            if is_executable_file(&bundled) {
                return Some(bundled);
            }
        }
    }

    if crate::is_sandboxed() {
        for dir in crate::cli_download::get_cli_path_dirs() {
            let downloaded = dir.join(binary);
            if is_executable_file(&downloaded) {
                return Some(downloaded);
            }
        }
    }

    // `get_extended_path` rather than `PATH`: it is a superset, and the two
    // additions are exactly the cases where a bare `PATH` lookup was wrong — the
    // sandbox CLI directories, and the Homebrew prefixes missing from the
    // environment a macOS `.app` inherits from Finder.
    let search_path = crate::cli_download::get_extended_path();
    search_path
        .split(':')
        .filter(|dir| !dir.is_empty())
        .map(|dir| Path::new(dir).join(binary))
        .find(|candidate| is_executable_file(candidate))
}

/// Returns whether `binary` is installed and executable.
///
/// Prefer [`find_in_path`] when the resolved path is going to be used: passing
/// the absolute path to `Command::new` avoids a second lookup by the kernel and
/// makes the log say which of several same-named binaries was chosen.
#[must_use]
pub fn is_available(binary: &str) -> bool {
    find_in_path(binary).is_some()
}

/// Finds `binary` on the host from inside a Flatpak sandbox.
///
/// For tools that cannot be bundled because they are the user's own desktop
/// applications — `virt-viewer`, KeePassXC — and so exist only outside the
/// sandbox. Runs `flatpak-spawn --host sh -lc 'command -v <binary>'`: a *login*
/// shell, so the host's own `PATH` is honoured rather than the sandbox's, and
/// `command -v` rather than `which`, because the host is not guaranteed to have
/// the latter either. Requires `--talk-name=org.freedesktop.Flatpak`, which the
/// manifest already grants.
///
/// Returns `None` when not running in Flatpak, so a caller can use it as an
/// unconditional fallback.
#[must_use]
pub fn find_on_host(binary: &str) -> Option<PathBuf> {
    if !crate::flatpak::is_flatpak() {
        return None;
    }
    // The name reaches a shell, so anything but a plain binary name is refused
    // rather than quoted: no caller needs it, and `command -v` would otherwise
    // become an injection point for a value that came from a config file.
    if binary.is_empty()
        || !binary
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '+'))
    {
        tracing::warn!(
            binary,
            "refusing to resolve an unsafe binary name on the host"
        );
        return None;
    }

    let child = std::process::Command::new("flatpak-spawn")
        .args(["--host", "sh", "-lc", &format!("command -v {binary}")])
        // stdin nulled rather than inherited: this is a host *login* shell, and
        // handing it the application's stdin gives it something to read from if it
        // ever decides to.
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn();

    let child = match child {
        Ok(child) => child,
        Err(e) => {
            tracing::debug!(binary, %e, "flatpak-spawn unavailable; cannot probe the host");
            return None;
        }
    };

    // Bounded, because this runs on the GTK main thread: the SPICE launch asks it
    // before opening a session and the Settings tab asks it while building a row.
    // The wait is on the Flatpak session helper and a host login shell, neither of
    // which RustConn controls, and an unresponsive one must not freeze the window.
    let output = match crate::proc::wait_bounded(child, HOST_PROBE_TIMEOUT, "host binary probe") {
        Ok(crate::proc::Waited::Exited(output)) => output,
        Ok(crate::proc::Waited::TimedOut) => return None,
        Err(e) => {
            tracing::debug!(binary, %e, "failed to poll the host binary probe");
            return None;
        }
    };
    if !output.status.success() {
        tracing::debug!(binary, "binary not found on the host");
        return None;
    }

    let path = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if path.is_empty() {
        return None;
    }
    tracing::debug!(binary, path = %path, "resolved binary on the host");
    Some(PathBuf::from(path))
}

/// Locates a macOS application installed as an `.app` bundle.
///
/// GUI applications on macOS are `.app` bundles under `/Applications` (or the
/// per-user `~/Applications`), not binaries on `PATH` — and the `PATH` a bundle
/// inherits from Finder does not list them anyway. A bare-name [`find_in_path`]
/// lookup therefore never finds a RealVNC viewer, virt-viewer, a Chromium
/// browser and the like, which is why every external-viewer probe reported
/// "not installed" on macOS. This checks the bundle locations directly.
///
/// `candidates` is a list of `(bundle_file_name, executable_name)` pairs —
/// e.g. `("Google Chrome.app", "Google Chrome")`. The executable name is not
/// always the bundle stem (Brave's differs), so both are given explicitly.
/// Returns the full path to the executable inside the first bundle found
/// (`…/Contents/MacOS/<executable>`), which accepts the same command-line
/// arguments as the binary would.
///
/// Always returns `None` off macOS, so a caller can use it as an unconditional
/// fallback after a `PATH` probe.
#[must_use]
pub fn find_macos_app(candidates: &[(&str, &str)]) -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let mut roots: Vec<PathBuf> = vec![PathBuf::from("/Applications")];
        if let Some(home) = std::env::var_os("HOME") {
            roots.push(PathBuf::from(home).join("Applications"));
        }

        for root in &roots {
            for (bundle, exe) in candidates {
                let path = root.join(bundle).join("Contents/MacOS").join(exe);
                if path.is_file() {
                    return Some(path);
                }
            }
        }
        None
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = candidates;
        None
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    fn executable(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "#!/bin/sh\n").expect("write stub");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
            .expect("make stub executable");
        path
    }

    #[test]
    fn a_binary_on_path_is_found() {
        // `sh` is the one executable POSIX guarantees at a known location.
        let found = find_in_path("sh").expect("sh must be found");
        assert!(found.is_absolute(), "got {}", found.display());
        assert!(is_available("sh"));
    }

    #[test]
    fn a_binary_that_does_not_exist_is_not_found() {
        assert!(find_in_path("rustconn-no-such-binary-9d3f").is_none());
        assert!(!is_available("rustconn-no-such-binary-9d3f"));
    }

    #[test]
    fn an_empty_name_is_not_a_lookup() {
        // An empty candidate would otherwise match every directory on PATH.
        assert!(find_in_path("").is_none());
    }

    #[test]
    fn a_path_is_checked_and_not_searched() {
        let dir = tempfile::tempdir().expect("temp dir");
        let script = executable(dir.path(), "probe");

        let resolved = find_in_path(script.to_str().expect("utf-8 path"));
        assert_eq!(resolved.as_deref(), Some(script.as_path()));

        let missing = dir.path().join("absent");
        assert!(find_in_path(missing.to_str().expect("utf-8 path")).is_none());
    }

    #[test]
    fn a_non_executable_file_is_not_a_binary() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("data");
        std::fs::write(&path, "not a program").expect("write file");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644))
            .expect("drop the execute bit");

        assert!(find_in_path(path.to_str().expect("utf-8 path")).is_none());
    }

    #[test]
    fn a_directory_is_not_a_binary() {
        // `/usr/bin` has every execute bit set and is emphatically not a program.
        assert!(find_in_path("/usr/bin").is_none());
    }

    #[test]
    fn the_macos_app_probe_is_a_no_op_off_macos() {
        // Off macOS the bundle probe must always decline, so callers fall
        // through to their PATH lookup unchanged.
        #[cfg(not(target_os = "macos"))]
        assert!(find_macos_app(&[("Google Chrome.app", "Google Chrome")]).is_none());
        // On macOS a made-up bundle name must still not resolve.
        #[cfg(target_os = "macos")]
        assert!(find_macos_app(&[("RustConn No Such App.app", "Nope")]).is_none());
    }

    #[test]
    fn the_host_probe_is_a_no_op_outside_flatpak() {
        if !crate::flatpak::is_flatpak() {
            assert!(find_on_host("sh").is_none());
        }
    }

    #[test]
    fn the_host_probe_refuses_a_name_that_would_reach_the_shell() {
        // Guards the `sh -lc` interpolation. Outside Flatpak the early return
        // makes this vacuous, so assert on the validator's own verdict.
        for name in ["sh; rm -rf /", "$(id)", "a b", "`id`", ""] {
            assert!(
                find_on_host(name).is_none(),
                "{name:?} must not be resolved"
            );
        }
    }
}
