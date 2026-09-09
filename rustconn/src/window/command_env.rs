//! Secret-bearing environment for a Custom Command launch (issue #151).
//!
//! A Custom Command template may reference `${password}`. Substituting the
//! password into the template would put it in the argv of the `sh -c` we spawn,
//! where `/proc/<pid>/cmdline` and `ps` expose it to every process of the same
//! user. Instead the placeholder becomes a shell reference to an environment
//! variable and the value is delivered out of band:
//!
//! * Outside Flatpak, VTE sets the variable directly in the child environment —
//!   nothing touches a command line.
//! * Inside Flatpak the command runs on the host through `flatpak-spawn`, which
//!   does **not** forward the sandbox environment (verified against the GNOME 50
//!   runtime). Its `--env=VAR=VALUE` option would move the secret straight back
//!   into argv, so this module writes an `env -0` block to a mode-0600 file in
//!   `$XDG_RUNTIME_DIR` (tmpfs, user-private) that the spawned shell opens as a
//!   file descriptor and immediately unlinks, handing the descriptor to
//!   `flatpak-spawn --env-fd`. Only the path is ever visible in a command line.
//!
//! The residual exposure on Flatpak is the portal `HostCommand` D-Bus call, which
//! carries the child environment by design; that channel is the user's own
//! session bus. The launched program's own argv is the user's choice and outside
//! RustConn's control.

use std::fs::OpenOptions;
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use secrecy::{ExposeSecret, SecretString};

/// Resolves the user-private directory for an ephemeral mode-0600 secret file.
///
/// `$XDG_RUNTIME_DIR` (`/run/user/<uid>`, tmpfs and user-private) is the right
/// home on Linux. macOS has no `XDG_RUNTIME_DIR`, so there we fall back to the
/// per-user temp directory (`$TMPDIR`, a `/var/folders/…` path that is
/// `0700`-owned by the user) — without this the SPICE `.vv` password file could
/// never be written on macOS and every SPICE launch fell back to a prompt.
///
/// The fallback is deliberately macOS-only: on Linux a missing
/// `$XDG_RUNTIME_DIR` is unusual, and `std::env::temp_dir()` there is the
/// world-writable `/tmp`, a weaker location than the caller assumes. The files
/// are created `0600` regardless, so their contents stay private either way.
fn secret_file_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|path| path.is_dir())
    {
        return Some(dir);
    }
    #[cfg(target_os = "macos")]
    {
        let tmp = std::env::temp_dir();
        return tmp.is_dir().then_some(tmp);
    }
    #[cfg(not(target_os = "macos"))]
    {
        None
    }
}

/// Builds a validated `NAME=VALUE` environment entry.
///
/// The single guard both delivery paths share. `None` is returned — and the
/// caller launches without the variable — when the name is empty, the name holds
/// a NUL or `=`, or the value holds a NUL: in the `env -0` file a NUL would forge
/// a second variable, and in either encoding it would truncate the value.
///
/// The result is [`zeroize::Zeroizing`] because it is the only place the
/// plaintext is materialised outside `SecretString`.
pub(super) fn env_entry(name: &str, value: &SecretString) -> Option<zeroize::Zeroizing<String>> {
    if name.is_empty() || name.contains(['\0', '=']) || value.expose_secret().contains('\0') {
        tracing::warn!(
            variable = name,
            "Refusing to build command environment: name or value contains NUL or '='"
        );
        return None;
    }
    Some(zeroize::Zeroizing::new(format!(
        "{name}={}",
        value.expose_secret()
    )))
}

/// Appends the NUL separator the `env -0` format requires.
fn terminate(entry: &zeroize::Zeroizing<String>) -> zeroize::Zeroizing<String> {
    zeroize::Zeroizing::new(format!("{}\0", entry.as_str()))
}

/// Single-use `env -0` file consumed by `flatpak-spawn --env-fd`.
///
/// Deliberately has no `Drop` cleanup: the spawn is asynchronous, so removing the
/// file when the guard goes out of scope would race the child that still has to
/// open it. The generated shell command unlinks it as its first action, right
/// after opening the descriptor. A file left behind by a failed spawn lives in
/// tmpfs at mode 0600 and disappears at logout.
pub(super) struct EphemeralCommandEnv {
    path: PathBuf,
}

impl EphemeralCommandEnv {
    /// Returns the path the spawned shell reads the descriptor from.
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Writes `entries` as a NUL-separated `env -0` block to a fresh runtime file.
    ///
    /// Returns `None` when `$XDG_RUNTIME_DIR` is unusable, a name or value holds a
    /// NUL byte (which would forge an extra variable), or the file cannot be
    /// created — the caller then launches without the variable, leaving
    /// `${password}` unresolved rather than leaking it another way.
    pub(super) fn write(entries: &[(&str, &SecretString)]) -> Option<Self> {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|path| path.is_dir())?;
        Self::write_in_dir(&dir, entries)
    }

    fn write_in_dir(dir: &Path, entries: &[(&str, &SecretString)]) -> Option<Self> {
        // Build (and thereby validate) every entry before opening, so a rejected
        // one never creates a file.
        let blocks: Vec<zeroize::Zeroizing<String>> = entries
            .iter()
            .map(|(name, value)| env_entry(name, value).map(|entry| terminate(&entry)))
            .collect::<Option<_>>()?;

        let path = dir.join(format!("rustconn-cmdenv-{}", uuid::Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|error| {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "Failed to create command environment file"
                );
            })
            .ok()?;

        let guard = Self { path };
        for block in &blocks {
            if let Err(error) = file.write_all(block.as_bytes()) {
                tracing::warn!(
                    path = %guard.path.display(),
                    %error,
                    "Failed to write command environment file"
                );
                // Remove the partial file here: nothing will consume it, so the
                // "the shell unlinks it" contract does not apply.
                let _ = std::fs::remove_file(&guard.path);
                return None;
            }
        }
        Some(guard)
    }
}

impl std::fmt::Debug for EphemeralCommandEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EphemeralCommandEnv")
            .field("path", &self.path)
            .finish()
    }
}

/// A mode-0600 virt-viewer `.vv` connection file that removes itself on drop.
///
/// SPICE has no embedded client, so it launches `remote-viewer`, and the only
/// way to hand that a password without putting it on argv is a `.vv` file (issue
/// #308). The file's `[virt-viewer]` section carries `password=` together with
/// the connection parameters; its contents come from
/// [`rustconn_core::spice_client::build_vv_connection_file`], which also sets
/// `delete-this-file=1` so a viewer that starts removes the file after reading.
///
/// Unlike [`EphemeralCommandEnv`], this guard removes the file on drop — but
/// only for a launch that never happened. The spawn is asynchronous: `spawn()`
/// returns as soon as the child is forked, and `remote-viewer` has GTK to
/// initialise before it opens its connection file, so a guard still alive at
/// that point would delete the file out from under the viewer and the password
/// would never arrive. That is the same race [`EphemeralCommandEnv`] documents
/// as its reason for having no `Drop` at all.
///
/// So ownership is handed over explicitly: [`Self::release_to_viewer`] on a
/// successful spawn, after which `delete-this-file=1` in the file itself is what
/// removes it. Dropping the guard without releasing it — a spawn that failed, or
/// a write that failed halfway — removes the file here. Removal is best effort
/// and ignores a missing file, so the two paths never fight.
pub(super) struct EphemeralVvFile {
    path: PathBuf,
    /// Cleared by [`Self::release_to_viewer`] so `Drop` leaves the file alone.
    remove_on_drop: bool,
}

impl EphemeralVvFile {
    /// Returns the path passed to `remote-viewer` as its connection file.
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Gives up the guard's claim on the file after the viewer has been spawned.
    ///
    /// Call this once `remote-viewer` is running: from then on the file's own
    /// `delete-this-file=1` is what removes it, and deleting it here would be a
    /// race against a viewer that has not opened it yet. A guard that is dropped
    /// without this call still cleans up, which is what covers a failed spawn.
    pub(super) fn release_to_viewer(mut self) {
        self.remove_on_drop = false;
    }

    /// Writes `contents` to a fresh mode-0600 file in a user-private directory.
    ///
    /// Uses `$XDG_RUNTIME_DIR` (`/run/user/<uid>`, tmpfs and user-private) on
    /// Linux and the per-user temp directory on macOS — see [`secret_file_dir`].
    /// Returns `None` when no such directory is usable or the file cannot be
    /// created; the caller then falls back to the plain argv launch and lets the
    /// viewer prompt.
    ///
    /// It is **not** shared with the host at the same path. Inside Flatpak the
    /// sandbox gets its own runtime directory, kept on the host under
    /// `/run/user/<uid>/.flatpak/<app-id>/xdg-run/`, so a `host:` viewer run
    /// through `flatpak-spawn --host` cannot open the path written here. This doc
    /// comment claimed the opposite until 0.21.4, and the SPICE launch believed
    /// it — see [`rustconn_core::host_visible_path`], which the caller uses to
    /// translate the path, and issue
    /// [#308](https://github.com/totoshko88/RustConn/issues/308).
    pub(super) fn write(contents: &str) -> Option<Self> {
        let dir = secret_file_dir()?;
        Self::write_in_dir(&dir, contents)
    }

    fn write_in_dir(dir: &Path, contents: &str) -> Option<Self> {
        let path = dir.join(format!("rustconn-spice-{}.vv", uuid::Uuid::new_v4()));
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|error| {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "Failed to create SPICE connection file"
                );
            })
            .ok()?;

        let guard = Self {
            path,
            remove_on_drop: true,
        };
        if let Err(error) = file.write_all(contents.as_bytes()) {
            tracing::warn!(
                path = %guard.path.display(),
                %error,
                "Failed to write SPICE connection file"
            );
            return None; // guard's Drop removes the partial file
        }
        Some(guard)
    }
}

impl Drop for EphemeralVvFile {
    fn drop(&mut self) {
        if self.remove_on_drop {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

impl std::fmt::Debug for EphemeralVvFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Hand-written rather than derived so the file's *contents* — which hold
        // the password — can never be pulled in by a future field. The path and
        // the ownership flag are both safe to show, and showing whether cleanup
        // is still ours is exactly what a log about a leftover file needs.
        f.debug_struct("EphemeralVvFile")
            .field("path", &self.path)
            .field("remove_on_drop", &self.remove_on_drop)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    struct TempRuntimeDir(PathBuf);
    impl TempRuntimeDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("rustconn-test-env-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).expect("create test runtime dir");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .expect("set test runtime permissions");
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
        fn assert_empty(&self) {
            assert_eq!(
                std::fs::read_dir(&self.0)
                    .expect("read test runtime dir")
                    .count(),
                0,
                "a rejected entry must not create a file"
            );
        }
    }
    impl Drop for TempRuntimeDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn writes_nul_separated_env_block() {
        let dir = TempRuntimeDir::new();
        let password = SecretString::from("s3cret with spaces".to_string());
        let guard = EphemeralCommandEnv::write_in_dir(dir.path(), &[("RC_PW", &password)])
            .expect("write env file");
        let written = std::fs::read(guard.path()).expect("read env file");
        assert_eq!(written, b"RC_PW=s3cret with spaces\0");
    }

    #[test]
    fn file_mode_is_0600() {
        let dir = TempRuntimeDir::new();
        let password = SecretString::from("any".to_string());
        let guard = EphemeralCommandEnv::write_in_dir(dir.path(), &[("RC_PW", &password)])
            .expect("write env file");
        let mode = std::fs::metadata(guard.path())
            .expect("read env file metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn rejects_nul_in_value_before_open() {
        let dir = TempRuntimeDir::new();
        let password = SecretString::from("secret\0INJECTED=1".to_string());
        assert!(EphemeralCommandEnv::write_in_dir(dir.path(), &[("RC_PW", &password)]).is_none());
        dir.assert_empty();
    }

    #[test]
    fn rejects_malformed_names_before_open() {
        let password = SecretString::from("safe".to_string());
        for name in ["", "RC\0PW", "RC=PW"] {
            let dir = TempRuntimeDir::new();
            assert!(EphemeralCommandEnv::write_in_dir(dir.path(), &[(name, &password)]).is_none());
            dir.assert_empty();
        }
    }

    #[test]
    fn debug_does_not_leak_password() {
        let dir = TempRuntimeDir::new();
        let password = SecretString::from("hunter2-secret".to_string());
        let guard = EphemeralCommandEnv::write_in_dir(dir.path(), &[("RC_PW", &password)])
            .expect("write env file");
        assert!(!format!("{guard:?}").contains("hunter2-secret"));
    }

    #[test]
    fn vv_file_is_written_mode_0600() {
        let dir = TempRuntimeDir::new();
        let guard = EphemeralVvFile::write_in_dir(dir.path(), "[virt-viewer]\ntype=spice\n")
            .expect("write vv file");
        let mode = std::fs::metadata(guard.path())
            .expect("read vv file metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        assert_eq!(
            std::fs::read_to_string(guard.path()).expect("read vv file"),
            "[virt-viewer]\ntype=spice\n"
        );
    }

    #[test]
    fn vv_file_removed_on_drop_without_release() {
        // A spawn that never happened: the guard is the only thing that can
        // clean up, because no viewer will act on `delete-this-file=1`.
        let dir = TempRuntimeDir::new();
        let path = {
            let guard = EphemeralVvFile::write_in_dir(dir.path(), "x").expect("write vv file");
            guard.path().to_path_buf()
        };
        assert!(!path.exists(), "the .vv file must be removed on drop");
        dir.assert_empty();
    }

    #[test]
    fn vv_file_survives_release_to_viewer() {
        // A spawned viewer opens its connection file some milliseconds later, so
        // releasing must leave the file in place for it to read (issue #308).
        let dir = TempRuntimeDir::new();
        let guard = EphemeralVvFile::write_in_dir(dir.path(), "x").expect("write vv file");
        let path = guard.path().to_path_buf();
        guard.release_to_viewer();
        assert!(
            path.exists(),
            "a released .vv file must outlive the guard for the viewer to read it"
        );
        std::fs::remove_file(&path).expect("test cleanup");
    }

    #[test]
    fn vv_file_debug_does_not_leak_contents() {
        let dir = TempRuntimeDir::new();
        let guard = EphemeralVvFile::write_in_dir(dir.path(), "password=hunter2-secret\n")
            .expect("write vv file");
        assert!(!format!("{guard:?}").contains("hunter2-secret"));
    }
}
