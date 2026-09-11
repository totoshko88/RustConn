//! Resolves the user-private directory for ephemeral mode-0600 secret files.
//!
//! Several launch paths write a short-lived secret to disk and hand only its
//! path to a child process: the SPICE `.vv` connection file, the FreeRDP
//! `/args-from:` file, the SSH askpass script and its secret file, and the
//! ssh-agent public-key cache. They all need the same answer to one question —
//! *which directory is user-private and safe for a 0600 file* — and they used
//! to answer it four different ways, three of which were wrong on macOS.
//!
//! This module is that single answer.

use std::path::PathBuf;

/// Returns a user-private directory suitable for an ephemeral mode-0600 file.
///
/// `$XDG_RUNTIME_DIR` (`/run/user/<uid>`, tmpfs and user-private) is the right
/// home on Linux. macOS has no `XDG_RUNTIME_DIR`, so there we fall back to the
/// per-user temp directory (`$TMPDIR`, a `/var/folders/…` path that is
/// `0700`-owned by the user). Without that fallback the RDP `/args-from:` file
/// could not be written on macOS and external FreeRDP never started with a
/// stored password — the same class of bug already fixed for the SPICE `.vv`
/// file, which is why both now share this function.
///
/// The macOS fallback is deliberate and platform-gated: on Linux a missing
/// `$XDG_RUNTIME_DIR` is unusual, and `std::env::temp_dir()` there is the
/// world-writable `/tmp`, a weaker location than a caller of this function
/// assumes. Returning `None` on Linux lets the caller degrade explicitly (a
/// prompt, a plain-argv launch) rather than silently drop a secret into `/tmp`.
/// Callers create the file `0600` regardless, so its contents stay private on
/// either platform.
///
/// This is **not** the right helper for a UNIX-domain socket path: macOS caps
/// `sun_path` at 104 bytes and `$TMPDIR` alone eats about half of that, so the
/// socket paths in `monitoring::ssh_exec` deliberately fall back to `/tmp`
/// instead. Use this only for regular files.
#[must_use]
pub fn secret_file_dir() -> Option<PathBuf> {
    resolve(std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from))
}

/// Testable core of [`secret_file_dir`]: the runtime-dir value is an argument
/// rather than read from the environment, so the per-platform fallback can be
/// exercised without mutating process-global state (`set_var` is `unsafe` in
/// Rust 2024 and forbidden outside the `-sys` crates).
fn resolve(runtime_dir: Option<PathBuf>) -> Option<PathBuf> {
    if let Some(dir) = runtime_dir.filter(|path: &PathBuf| path.is_dir()) {
        return Some(dir);
    }
    macos_temp_fallback()
}

/// The macOS-only temp fallback, factored out so [`resolve`] reads the same on
/// both platforms. Always `None` off macOS.
fn macos_temp_fallback() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let tmp = std::env::temp_dir();
        tmp.is_dir().then_some(tmp)
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// A present, real runtime directory is always preferred, on every platform.
    #[test]
    fn prefers_the_runtime_dir_when_it_is_a_real_directory() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let resolved = resolve(Some(tmp.path().to_path_buf()));
        assert_eq!(resolved.as_deref(), Some(tmp.path()));
    }

    /// A runtime dir that is not a real directory is rejected, then the
    /// per-platform fallback decides.
    #[test]
    fn rejects_a_runtime_dir_that_is_not_a_directory() {
        let resolved = resolve(Some(PathBuf::from("/rustconn-no-such-dir-9d3f")));
        assert_eq!(resolved, macos_temp_fallback());
    }

    /// Absent runtime dir: macOS falls back to a usable temp directory; Linux
    /// declines so the caller can degrade explicitly rather than use `/tmp`.
    #[test]
    fn falls_back_per_platform_when_runtime_dir_is_absent() {
        let resolved = resolve(None);
        if cfg!(target_os = "macos") {
            assert!(
                resolved.as_deref().is_some_and(Path::is_dir),
                "macOS must fall back to a usable temp directory"
            );
        } else {
            assert!(
                resolved.is_none(),
                "Linux must decline rather than use world-writable /tmp"
            );
        }
    }
}
