//! Ephemeral FreeRDP args file for RemoteApp passwords.
//!
//! `xfreerdp3 /from-stdin` does not work for RemoteApp (RAIL) sessions:
//! the FreeRDP RAIL handshake bypasses the stdin password reader, so the
//! credentials never reach the server. Until [FreeRDP#12485] is fixed
//! upstream we previously fell back to `/p:<password>` on the command
//! line, which exposes the password in `/proc/<pid>/cmdline` to any
//! process running under the same uid (Known Issue from 0.14.10).
//!
//! `xfreerdp3 /args-from:file:<path>` reads CLI arguments from a file
//! instead of the command line, so writing `/p:<password>` into a
//! single-use file in `$XDG_RUNTIME_DIR` (mode 0600) keeps the secret
//! out of `cmdline` while still satisfying the RAIL handshake.
//!
//! [FreeRDP#12485]: https://github.com/FreeRDP/FreeRDP/issues/12485
//!
//! # Lifecycle
//!
//! [`EphemeralRdpArgs`] writes the args file in [`Self::write`] and
//! removes it on `Drop` (best-effort). Callers must hold the guard
//! alive until the spawned `xfreerdp3` process has actually consumed
//! the file (a fraction of a second after `spawn`). Because FreeRDP
//! keeps the file open for the duration of the unaltered argument
//! parsing, dropping the guard immediately after `child.try_wait()`
//! returns `None` is safe.

use secrecy::{ExposeSecret, SecretString};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use rustconn_core::error::SecretResult;

/// Single-use args file containing only the RemoteApp password line.
///
/// The file is created with mode `0600` so only the owning user can
/// read it. It is removed when the guard is dropped, even if the
/// launcher panics partway through `spawn`.
pub(super) struct EphemeralRdpArgs {
    path: PathBuf,
}

impl EphemeralRdpArgs {
    /// Returns the path the spawned `xfreerdp3` should read its args
    /// from via `/args-from:file:<path>`.
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Writes a `/p:<password>` line to a fresh file in
    /// `$XDG_RUNTIME_DIR` and returns a guard that removes the file
    /// on drop.
    ///
    /// # Errors
    ///
    /// Returns `SecretError::Pass` when the runtime directory cannot
    /// be located or the file cannot be created with the requested
    /// permissions.
    pub(super) fn write(password: &SecretString) -> SecretResult<Self> {
        use rustconn_core::error::SecretError;

        // $XDG_RUNTIME_DIR is the natural choice on Linux desktops:
        // tmpfs, mode 0700, owned by the user, cleared on logout.
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .ok_or_else(|| {
                SecretError::Pass(
                    "XDG_RUNTIME_DIR is not set or is not a directory; \
                     cannot create ephemeral RemoteApp args file"
                        .to_string(),
                )
            })?;

        Self::write_in_dir(&dir, password)
    }

    /// Writes the args file into a specific directory. Used by `write`
    /// (with `$XDG_RUNTIME_DIR`) and by the tests.
    fn write_in_dir(dir: &Path, password: &SecretString) -> SecretResult<Self> {
        use rustconn_core::error::SecretError;

        // Avoid name collisions across concurrent RemoteApp launches by
        // suffixing with a random UUID.
        let path = dir.join(format!("rustconn-rdp-{}.args", uuid::Uuid::new_v4()));

        let mut file: File = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| {
                SecretError::Pass(format!(
                    "failed to create ephemeral RemoteApp args file at {}: {e}",
                    path.display()
                ))
            })?;

        // FreeRDP /args-from:file: format is one argument per line.
        // We put exactly one line — the password switch — into the
        // file. Everything else is still passed on the command line so
        // it stays visible in `ps` (helpful for debugging) without
        // exposing the secret.
        let line = format!("/p:{}\n", password.expose_secret());
        let res = file.write_all(line.as_bytes());
        // Zero out the heap copy of the line as soon as the write
        // completes; the file itself still holds the password until
        // it is removed in `Drop`.
        let mut zline = zeroize::Zeroizing::new(line);
        zline.clear();

        res.map_err(|e| {
            SecretError::Pass(format!(
                "failed to write ephemeral RemoteApp args file at {}: {e}",
                path.display()
            ))
        })?;

        Ok(Self { path })
    }
}

impl Drop for EphemeralRdpArgs {
    fn drop(&mut self) {
        // Best-effort: if the file was already moved or the runtime
        // directory was wiped under our feet, there is nothing
        // sensible we can do here. We deliberately ignore the result.
        if self.path.exists()
            && let Err(e) = std::fs::remove_file(&self.path)
        {
            tracing::warn!(
                path = %self.path.display(),
                error = %e,
                "failed to remove ephemeral RemoteApp args file; \
                 it will be cleaned up at logout via XDG_RUNTIME_DIR"
            );
        }
    }
}

impl std::fmt::Debug for EphemeralRdpArgs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EphemeralRdpArgs")
            .field("path", &self.path)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Creates a temporary directory mode 0700 to mimic `$XDG_RUNTIME_DIR`.
    /// The directory and its contents are removed when the returned guard
    /// drops.
    struct TempRuntimeDir(PathBuf);

    impl TempRuntimeDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("rustconn-test-rt-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&path).expect("create test runtime dir");
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))
                .expect("set 0700 on test runtime dir");
            Self(path)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRuntimeDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn drop_removes_file_with_normal_password() {
        let dir = TempRuntimeDir::new();
        let path_after_drop;
        {
            let pwd = SecretString::from("hunter2".to_string());
            let guard = EphemeralRdpArgs::write_in_dir(dir.path(), &pwd).expect("write args file");
            let p = guard.path().to_path_buf();
            assert!(p.starts_with(dir.path()));
            assert!(p.exists(), "args file should exist while guard is alive");
            path_after_drop = p;
        }
        assert!(
            !path_after_drop.exists(),
            "args file should be removed when guard drops"
        );
    }

    #[test]
    fn file_mode_is_0600() {
        let dir = TempRuntimeDir::new();
        let pwd = SecretString::from("any".to_string());
        let guard = EphemeralRdpArgs::write_in_dir(dir.path(), &pwd).expect("write args file");
        let mode = std::fs::metadata(guard.path())
            .expect("stat")
            .permissions()
            .mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "args file must be readable only by the owner"
        );
    }

    #[test]
    fn drop_removes_file_for_password_with_special_characters() {
        // Tests a payload that includes characters which would historically
        // have been awkward on the command line (`'`, `"`, `\n`, `\t`, etc.).
        // The file format is line-based but xfreerdp consumes the whole line
        // verbatim, so we only need to ensure the cleanup path runs.
        let dir = TempRuntimeDir::new();
        let path_after_drop;
        {
            let pwd = SecretString::from(
                "p@ss\twith\nnew\rlines and 'quotes' and \"escapes\\\"".to_string(),
            );
            let guard = EphemeralRdpArgs::write_in_dir(dir.path(), &pwd).expect("write args file");
            let p = guard.path().to_path_buf();
            assert!(p.exists());
            path_after_drop = p;
        }
        assert!(!path_after_drop.exists());
    }

    #[test]
    fn write_fails_for_nonexistent_dir() {
        let pwd = SecretString::from("any".to_string());
        let nope = std::path::Path::new("/this/path/does/not/exist/and/should/not");
        let res = EphemeralRdpArgs::write_in_dir(nope, &pwd);
        assert!(res.is_err());
    }

    #[test]
    fn debug_does_not_leak_password() {
        let dir = TempRuntimeDir::new();
        let pwd = SecretString::from("hunter2-secret".to_string());
        let guard = EphemeralRdpArgs::write_in_dir(dir.path(), &pwd).expect("write args file");
        let rendered = format!("{guard:?}");
        // Path is non-secret (it's in $XDG_RUNTIME_DIR with a UUID), so it
        // may appear; the password must not.
        assert!(
            !rendered.contains("hunter2-secret"),
            "Debug output leaked the password: {rendered}"
        );
    }
}
