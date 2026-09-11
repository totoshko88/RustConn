//! Ephemeral FreeRDP args file for connection arguments.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

use rustconn_core::error::SecretResult;
use secrecy::{ExposeSecret, SecretString};

/// Single-use args file containing all FreeRDP connection arguments.
///
/// The mode is `0600`; dropping the guard removes complete or partial files.
pub(super) struct EphemeralRdpArgs {
    path: PathBuf,
}

impl EphemeralRdpArgs {
    /// Returns the path used by the FreeRDP `/args-from:` switch.
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Writes plain and secret arguments to a fresh runtime file.
    ///
    /// All line components are validated before the file is opened.
    ///
    /// # Errors
    /// Returns `SecretError::Pass` if validation, creation, or writing fails.
    pub(super) fn write_all(
        plain_args: &[String],
        secret_args: &[(&str, &SecretString)],
    ) -> SecretResult<Self> {
        use rustconn_core::error::SecretError;
        // `secret_file_dir()` is `$XDG_RUNTIME_DIR` on Linux and `$TMPDIR` on
        // macOS, which has no runtime dir — the shared resolver SPICE already
        // uses. Reading `XDG_RUNTIME_DIR` directly here hard-failed on macOS
        // (the runtime dir is absent by default), so external FreeRDP never
        // started with a stored password because this file could not be written.
        let dir = rustconn_core::secret_file_dir().ok_or_else(|| {
            SecretError::Pass(
                "no user-private runtime directory is available; \
                 cannot create ephemeral RDP args file"
                    .to_string(),
            )
        })?;
        Self::write_all_in_dir(&dir, plain_args, secret_args)
    }

    fn validate_line_component(kind: &str, value: &str) -> SecretResult<()> {
        use rustconn_core::error::SecretError;
        if value.contains(['\r', '\n', '\0']) {
            return Err(SecretError::Pass(format!(
                "RDP {kind} contains a forbidden CR, LF, or NUL byte"
            )));
        }
        Ok(())
    }

    pub(super) fn validate_plain_args(plain_args: &[String]) -> SecretResult<()> {
        use rustconn_core::error::SecretError;
        use rustconn_core::protocol::{
            contains_freerdp_secret_field, is_freerdp_shell_or_proxy_arg,
        };

        for arg in plain_args {
            Self::validate_line_component("argument", arg)?;
            if contains_freerdp_secret_field(arg) {
                return Err(SecretError::Pass(
                    "RDP password arguments must use the protected secret path".to_string(),
                ));
            }
            if is_freerdp_shell_or_proxy_arg(arg) {
                return Err(SecretError::Pass(
                    "RDP shell or proxy arguments are not allowed".to_string(),
                ));
            }
        }
        Ok(())
    }

    fn validate_args(
        plain_args: &[String],
        secret_args: &[(&str, &SecretString)],
    ) -> SecretResult<()> {
        Self::validate_plain_args(plain_args)?;
        for (flag, secret) in secret_args {
            Self::validate_line_component("secret flag", flag)?;
            Self::validate_line_component("secret value", secret.expose_secret())?;
        }
        Ok(())
    }

    fn write_all_in_dir(
        dir: &Path,
        plain_args: &[String],
        secret_args: &[(&str, &SecretString)],
    ) -> SecretResult<Self> {
        use rustconn_core::error::SecretError;
        Self::validate_args(plain_args, secret_args)?;
        let path = dir.join(format!("rustconn-rdp-{}.args", uuid::Uuid::new_v4()));
        let mut file: File = OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&path)
            .map_err(|error| {
                SecretError::Pass(format!(
                    "failed to create ephemeral RDP args file at {}: {error}",
                    path.display()
                ))
            })?;

        // Install cleanup immediately after open so every partial write path
        // removes the file before propagating its error.
        let guard = Self { path };
        for arg in plain_args {
            file.write_all(arg.as_bytes()).map_err(|error| {
                SecretError::Pass(format!(
                    "failed to write ephemeral RDP args file at {}: {error}",
                    guard.path.display()
                ))
            })?;
            file.write_all(b"\n").map_err(|error| {
                SecretError::Pass(format!(
                    "failed to write ephemeral RDP args file at {}: {error}",
                    guard.path.display()
                ))
            })?;
        }
        for (flag, secret) in secret_args {
            let line = zeroize::Zeroizing::new(format!("/{flag}:{}\n", secret.expose_secret()));
            file.write_all(line.as_bytes()).map_err(|error| {
                SecretError::Pass(format!(
                    "failed to write ephemeral RDP args file at {}: {error}",
                    guard.path.display()
                ))
            })?;
        }
        Ok(guard)
    }

    #[cfg(test)]
    fn write_in_dir(dir: &Path, args: &[(&str, &SecretString)]) -> SecretResult<Self> {
        Self::write_all_in_dir(dir, &[], args)
    }
}

impl Drop for EphemeralRdpArgs {
    fn drop(&mut self) {
        if self.path.exists()
            && let Err(error) = std::fs::remove_file(&self.path)
        {
            tracing::warn!(
                path = %self.path.display(),
                %error,
                "failed to remove ephemeral RDP args file; it will be cleaned up at logout"
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
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    struct TempRuntimeDir(PathBuf);
    impl TempRuntimeDir {
        fn new() -> Self {
            let path =
                std::env::temp_dir().join(format!("rustconn-test-rt-{}", uuid::Uuid::new_v4()));
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
                "rejected arguments must not create a file"
            );
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
            let password = SecretString::from("hunter2".to_string());
            let guard = EphemeralRdpArgs::write_in_dir(dir.path(), &[("p", &password)])
                .expect("write args file");
            path_after_drop = guard.path().to_path_buf();
            assert!(path_after_drop.exists());
        }
        assert!(!path_after_drop.exists());
    }

    #[test]
    fn file_mode_is_0600() {
        let dir = TempRuntimeDir::new();
        let password = SecretString::from("any".to_string());
        let guard = EphemeralRdpArgs::write_in_dir(dir.path(), &[("p", &password)])
            .expect("write args file");
        let mode = std::fs::metadata(guard.path())
            .expect("read args file metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn safe_special_characters_are_written_and_cleaned_up() {
        let dir = TempRuntimeDir::new();
        let path_after_drop;
        {
            let password =
                SecretString::from("p@ss\twith spaces and 'quotes' and \"escapes\\\"".to_string());
            let guard = EphemeralRdpArgs::write_in_dir(dir.path(), &[("p", &password)])
                .expect("write args file");
            path_after_drop = guard.path().to_path_buf();
        }
        assert!(!path_after_drop.exists());
    }

    #[test]
    fn rejects_cr_lf_and_nul_in_plain_arguments_before_open() {
        for suffix in ["\r/injected", "\n/injected", "\0/injected"] {
            let dir = TempRuntimeDir::new();
            let plain = vec![format!("/v:host{suffix}")];
            assert!(EphemeralRdpArgs::write_all_in_dir(dir.path(), &plain, &[]).is_err());
            dir.assert_empty();
        }
    }

    #[test]
    fn rejects_passwords_in_plain_arguments_before_open() {
        for argument in [
            "/p:secret",
            "/PASSWORD:secret",
            "  /gp:secret",
            "/gateway-password:secret",
            "/pth:secret-hash",
        ] {
            let dir = TempRuntimeDir::new();
            let plain = vec![argument.to_string()];
            assert!(EphemeralRdpArgs::write_all_in_dir(dir.path(), &plain, &[]).is_err());
            dir.assert_empty();
        }
    }

    #[test]
    fn rejects_composite_password_fields_before_open() {
        for argument in [
            "/gateway:g:host,p:secret",
            "/gateway:g:host,PASSWORD:secret",
            "/gateway:g:host, gp:secret",
            "/gateway:g:host,GATEWAY-PASSWORD:secret",
            "/gateway:g:host,  PTH:secret-hash",
        ] {
            let dir = TempRuntimeDir::new();
            let plain = vec![argument.to_string()];
            assert!(EphemeralRdpArgs::write_all_in_dir(dir.path(), &plain, &[]).is_err());
            dir.assert_empty();
        }
    }

    #[test]
    fn accepts_generated_gateway_host_and_user_fields() {
        let dir = TempRuntimeDir::new();
        let plain = vec!["/gateway:g:host,u:user".to_string()];
        let guard = EphemeralRdpArgs::write_all_in_dir(dir.path(), &plain, &[])
            .expect("generated gateway argument should be accepted");
        assert_eq!(
            std::fs::read_to_string(guard.path()).expect("read args file"),
            "/gateway:g:host,u:user\n"
        );
    }

    #[test]
    fn rejects_cr_lf_and_nul_in_secret_flags_before_open() {
        let password = SecretString::from("safe".to_string());
        for flag in ["p\r/injected", "p\n/injected", "p\0/injected"] {
            let dir = TempRuntimeDir::new();
            assert!(
                EphemeralRdpArgs::write_all_in_dir(dir.path(), &[], &[(flag, &password)]).is_err()
            );
            dir.assert_empty();
        }
    }

    #[test]
    fn rejects_cr_lf_and_nul_in_secret_values_before_open() {
        for value in [
            "secret\r/injected",
            "secret\n/injected",
            "secret\0/injected",
        ] {
            let dir = TempRuntimeDir::new();
            let password = SecretString::from(value.to_string());
            assert!(
                EphemeralRdpArgs::write_all_in_dir(dir.path(), &[], &[("p", &password)]).is_err()
            );
            dir.assert_empty();
        }
    }

    #[test]
    fn debug_does_not_leak_password() {
        let dir = TempRuntimeDir::new();
        let password = SecretString::from("hunter2-secret".to_string());
        let guard = EphemeralRdpArgs::write_in_dir(dir.path(), &[("p", &password)])
            .expect("write args file");
        assert!(!format!("{guard:?}").contains("hunter2-secret"));
    }

    #[test]
    fn writes_multiple_secret_args_one_per_line() {
        let dir = TempRuntimeDir::new();
        let session = SecretString::from("session-pw".to_string());
        let gateway = SecretString::from("gateway-pw".to_string());
        let guard =
            EphemeralRdpArgs::write_in_dir(dir.path(), &[("p", &session), ("gp", &gateway)])
                .expect("write args file");
        assert_eq!(
            std::fs::read_to_string(guard.path()).expect("read args file"),
            "/p:session-pw\n/gp:gateway-pw\n"
        );
    }

    #[test]
    fn blocks_shell_proxy_and_split_passwords_before_open() {
        for plain in [
            vec!["  /SHELL:command".to_string()],
            vec!["\t//PrOxY:command".to_string()],
            vec!["--password".to_string(), "split-secret".to_string()],
        ] {
            let dir = TempRuntimeDir::new();
            assert!(EphemeralRdpArgs::write_all_in_dir(dir.path(), &plain, &[]).is_err());
            dir.assert_empty();
        }
    }

    #[test]
    fn write_all_combines_plain_and_secret_args() {
        let dir = TempRuntimeDir::new();
        let plain = vec![
            "/v:myhost".to_string(),
            "/u:admin".to_string(),
            "+clipboard".to_string(),
        ];
        let password = SecretString::from("s3cret".to_string());
        let guard = EphemeralRdpArgs::write_all_in_dir(dir.path(), &plain, &[("p", &password)])
            .expect("write args file");
        assert_eq!(
            std::fs::read_to_string(guard.path()).expect("read args file"),
            "/v:myhost\n/u:admin\n+clipboard\n/p:s3cret\n"
        );
    }
}
