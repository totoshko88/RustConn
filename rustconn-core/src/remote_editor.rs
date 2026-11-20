//! MobaXterm-style "edit remote file" built on the `sftp` CLI.
//!
//! Download to a temporary copy, let the GUI edit it, upload on [`RemoteEditor::save`].
//! Authentication is key-only — `sftp -b` is non-interactive, no `SecretString`
//! is available for an askpass helper, and a secret must never reach argv or a
//! batch file.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use thiserror::Error;
use tokio::process::Command;

use crate::models::{Connection, PasswordSource};

/// Largest file we will upload. Downloaded files bigger than this are still
/// opened, but saving refuses to push them back.
const MAX_UPLOAD_BYTES: u64 = 16 * 1024 * 1024;

/// Errors from opening or saving a remote file.
#[derive(Debug, Error)]
pub enum RemoteEditorError {
    /// The supplied remote path was empty.
    #[error("remote path must not be empty")]
    EmptyRemotePath,
    /// The remote path contained a NUL byte.
    #[error("remote path contains a NUL byte")]
    NulInRemotePath,
    /// The remote path contained a newline, which would inject sftp commands.
    #[error("remote path contains a newline")]
    NewlineInRemotePath,
    /// The connection needs a password that cannot be passed safely.
    #[error(
        "this connection uses password authentication, which the remote editor \
         cannot pass to sftp without exposing it; use key-based authentication \
         or type the password in a terminal session"
    )]
    PasswordAuthUnsupported,
    /// A temporary directory could not be created.
    #[error("failed to create temporary directory: {0}")]
    TempDir(std::io::Error),
    /// The sftp batch file could not be written.
    #[error("failed to write sftp batch file: {0}")]
    WriteBatch(std::io::Error),
    /// sftp could not be started.
    #[error("failed to start sftp: {0}")]
    SftpSpawn(std::io::Error),
    /// sftp exited unsuccessfully.
    #[error("sftp exited with status {0:?}")]
    SftpFailed(Option<i32>),
    /// scp could not be started (fallback for hosts without an sftp subsystem).
    #[error("failed to start scp: {0}")]
    ScpSpawn(std::io::Error),
    /// scp exited unsuccessfully.
    #[error("scp exited with status {0:?}")]
    ScpFailed(Option<i32>),
    /// The remote path is not safe to pass through a remote shell in scp mode.
    #[error("remote path contains shell metacharacters")]
    UnsafeScpPath,
    /// The local copy could not be read.
    #[error("failed to read local file: {0}")]
    ReadLocal(std::io::Error),
    /// The local copy grew past the upload limit.
    #[error("file is too large to upload ({size} bytes, limit {limit} bytes)")]
    FileTooLarge {
        /// Actual size in bytes.
        size: u64,
        /// Maximum allowed size in bytes.
        limit: u64,
    },
}

/// Result of a [`RemoteEditor::save`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The local copy was byte-for-byte identical to the original; nothing was
    /// uploaded.
    Unchanged,
    /// The edited copy was uploaded through sftp.
    Uploaded,
}

/// A remote file checked out into a temporary local copy for editing.
#[derive(Debug)]
pub struct RemoteEditor {
    /// Connection name, used only for log context.
    connection_name: String,
    /// Remote host passed to sftp.
    host: String,
    /// Remote SSH port.
    port: u16,
    /// Remote username, if the connection defines one.
    username: Option<String>,
    /// Original remote path, used for the upload command.
    remote_path: String,
    /// Local temporary copy the GUI edits.
    local_path: PathBuf,
    /// Keeps the temporary directory alive until editing is finished.
    temp_dir: tempfile::TempDir,
    /// Byte length of the file as first downloaded.
    original_len: u64,
    /// FNV-1a hash of the file as first downloaded.
    original_hash: u64,
}

impl RemoteEditor {
    /// Downloads `remote_path` from `connection` into a temporary local copy.
    ///
    /// Uses the existing `sftp` CLI with a batch file and a null stdin, never a
    /// shell. Connections configured for password authentication are refused:
    /// there is no `SecretString` available here to hand to an askpass helper,
    /// and a secret must never reach argv or the batch file.
    ///
    /// # Errors
    /// Returns a path-validation, password-auth, or I/O/spawn variant on failure.
    pub async fn open(
        connection: &Connection,
        remote_path: &str,
    ) -> Result<Self, RemoteEditorError> {
        validate_remote_path(remote_path)?;
        if !matches!(connection.password_source, PasswordSource::None) {
            return Err(RemoteEditorError::PasswordAuthUnsupported);
        }

        let temp_dir = tempfile::Builder::new()
            .prefix("rustconn-remote-edit-")
            .tempdir()
            .map_err(RemoteEditorError::TempDir)?;

        let local_path = temp_dir.path().join(local_filename(remote_path));
        let batch_path = temp_dir.path().join("get.batch");
        write_batch(&batch_path, &build_get_batch(remote_path, &local_path))?;

        let status = sftp_command(
            connection.host.as_str(),
            connection.port,
            connection.username.as_deref(),
            &batch_path,
        )
        .status()
        .await
        .map_err(RemoteEditorError::SftpSpawn)?;
        if !status.success() {
            // Dropbear and other minimal SSH servers have no sftp subsystem.
            // Legacy SCP (-O) is available on those hosts; it needs no server-side
            // helper beyond the SSH shell.
            tracing::info!(
                code = ?status.code(),
                "sftp unavailable; falling back to scp -O"
            );
            scp_transfer(
                &connection.host,
                connection.port,
                connection.username.as_deref(),
                remote_path,
                &local_path,
                false,
            )
            .await?;
        }

        let bytes = tokio::fs::read(&local_path)
            .await
            .map_err(RemoteEditorError::ReadLocal)?;
        let original_len = bytes.len() as u64;
        let original_hash = fnv1a(&bytes);

        tracing::info!(
            connection = %connection.name,
            host = %connection.host,
            bytes = original_len,
            "remote file opened for editing"
        );

        Ok(Self {
            connection_name: connection.name.clone(),
            host: connection.host.clone(),
            port: connection.port,
            username: connection.username.clone(),
            remote_path: remote_path.to_string(),
            local_path,
            temp_dir,
            original_len,
            original_hash,
        })
    }

    /// Uploads the local copy if it changed since [`Self::open`].
    ///
    /// # Errors
    /// Returns [`RemoteEditorError::ReadLocal`],
    /// [`RemoteEditorError::FileTooLarge`], or a batch/spawn variant on failure.
    pub async fn save(&self) -> Result<SaveOutcome, RemoteEditorError> {
        let bytes = tokio::fs::read(&self.local_path)
            .await
            .map_err(RemoteEditorError::ReadLocal)?;
        if is_unchanged(self.original_len, self.original_hash, &bytes) {
            return Ok(SaveOutcome::Unchanged);
        }

        let size = bytes.len() as u64;
        if size > MAX_UPLOAD_BYTES {
            return Err(RemoteEditorError::FileTooLarge {
                size,
                limit: MAX_UPLOAD_BYTES,
            });
        }

        let batch_path = self.temp_dir.path().join("put.batch");
        write_batch(
            &batch_path,
            &build_put_batch(&self.local_path, &self.remote_path),
        )?;
        let status = sftp_command(&self.host, self.port, self.username.as_deref(), &batch_path)
            .status()
            .await
            .map_err(RemoteEditorError::SftpSpawn)?;
        if !status.success() {
            scp_transfer(
                &self.host,
                self.port,
                self.username.as_deref(),
                &self.remote_path,
                &self.local_path,
                true,
            )
            .await?;
        }

        tracing::info!(
            connection = %self.connection_name,
            host = %self.host,
            bytes = size,
            "remote file uploaded"
        );
        Ok(SaveOutcome::Uploaded)
    }

    /// Path of the local temporary copy, for the GUI to open with the system
    /// editor (`gio open` / `xdg-open` / macOS `open`).
    #[must_use]
    pub fn local_path(&self) -> &Path {
        &self.local_path
    }
}

/// Rejects paths that would break or inject into a line-based sftp batch file.
fn validate_remote_path(path: &str) -> Result<(), RemoteEditorError> {
    if path.is_empty() {
        return Err(RemoteEditorError::EmptyRemotePath);
    }
    if path.contains('\0') {
        return Err(RemoteEditorError::NulInRemotePath);
    }
    if path.contains('\n') || path.contains('\r') {
        return Err(RemoteEditorError::NewlineInRemotePath);
    }
    Ok(())
}

/// Picks a readable filename for the local copy so the system editor can offer
/// syntax highlighting. Falls back to a fixed name for odd remote paths.
fn local_filename(remote_path: &str) -> String {
    match remote_path.rsplit(['/', '\\']).next() {
        Some(name) if !name.is_empty() && name != "." && name != ".." => name.to_string(),
        _ => "remote-file".to_string(),
    }
}

/// Quotes one sftp batch argument, escaping the characters sftp's argument
/// parser treats specially.
fn quote(arg: &str) -> String {
    let mut out = String::with_capacity(arg.len() + 2);
    out.push('"');
    for c in arg.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    out
}

/// Contents of the download batch file.
fn build_get_batch(remote: &str, local: &Path) -> String {
    format!(
        "get {} {}\n",
        quote(remote),
        quote(&local.to_string_lossy())
    )
}

/// Contents of the upload batch file.
fn build_put_batch(local: &Path, remote: &str) -> String {
    format!(
        "put {} {}\n",
        quote(&local.to_string_lossy()),
        quote(remote)
    )
}

/// Writes a batch file; these files never contain credentials.
fn write_batch(path: &Path, content: &str) -> Result<(), RemoteEditorError> {
    std::fs::write(path, content).map_err(RemoteEditorError::WriteBatch)
}

/// Builds the non-interactive sftp invocation. Arguments are passed directly —
/// never through a shell — and stdin is null.
fn sftp_command(host: &str, port: u16, username: Option<&str>, batch: &Path) -> Command {
    let mut cmd = Command::new("sftp");
    cmd.arg("-b").arg(batch).arg("-P").arg(port.to_string());
    let target = match username {
        Some(user) => format!("{user}@{host}"),
        None => host.to_string(),
    };
    cmd.arg(target);
    cmd.stdin(Stdio::null());
    cmd
}

/// Download (`upload == false`) or upload (`upload == true`) via legacy SCP.
///
/// `-O` forces the original SCP protocol: OpenSSH 9+ otherwise tries SFTP first,
/// which is exactly what is missing on Dropbear/OpenWrt hosts. The remote path
/// travels through the remote shell, so shell metacharacters are refused here.
async fn scp_transfer(
    host: &str,
    port: u16,
    username: Option<&str>,
    remote_path: &str,
    local: &Path,
    upload: bool,
) -> Result<(), RemoteEditorError> {
    const UNSAFE: &[char] = &[
        ';', '|', '&', '$', '(', ')', '`', '\'', '"', '*', '?', '[', ']', '>', '<',
    ];
    if remote_path.chars().any(|c| UNSAFE.contains(&c)) {
        return Err(RemoteEditorError::UnsafeScpPath);
    }

    let target = match username {
        Some(user) => format!("{user}@{host}:{remote_path}"),
        None => format!("{host}:{remote_path}"),
    };
    let local_arg = local.to_string_lossy();
    let mut cmd = Command::new("scp");
    cmd.arg("-O")
        .arg("-q")
        .arg("-P")
        .arg(port.to_string())
        .stdin(Stdio::null());
    if upload {
        cmd.arg(local_arg.as_ref()).arg(&target);
    } else {
        cmd.arg(&target).arg(local_arg.as_ref());
    }

    let status = cmd.status().await.map_err(RemoteEditorError::ScpSpawn)?;
    if !status.success() {
        return Err(RemoteEditorError::ScpFailed(status.code()));
    }
    Ok(())
}

/// FNV-1a, used only to detect local edits (not for security).
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    hash
}

/// Whether the current bytes still match what was downloaded.
fn is_unchanged(original_len: u64, original_hash: u64, bytes: &[u8]) -> bool {
    bytes.len() as u64 == original_len && fnv1a(bytes) == original_hash
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_remote_path() {
        assert!(matches!(
            validate_remote_path(""),
            Err(RemoteEditorError::EmptyRemotePath)
        ));
        assert!(matches!(
            validate_remote_path("/etc/hosts\0.bak"),
            Err(RemoteEditorError::NulInRemotePath)
        ));
        assert!(matches!(
            validate_remote_path("/etc/hosts\nrm -rf /"),
            Err(RemoteEditorError::NewlineInRemotePath)
        ));
        assert!(matches!(
            validate_remote_path("/etc/hosts\rput x y"),
            Err(RemoteEditorError::NewlineInRemotePath)
        ));
        assert!(validate_remote_path("/var/log/app.log").is_ok());
    }

    #[tokio::test]
    async fn scp_fallback_refuses_shell_metacharacters() {
        let err = scp_transfer(
            "host",
            22,
            Some("root"),
            "/etc/hosts; rm -rf /",
            Path::new("/tmp/local"),
            false,
        )
        .await
        .expect_err("shell metacharacters must be refused");
        assert!(matches!(err, RemoteEditorError::UnsafeScpPath));
    }

    #[test]
    fn batch_file_content() {
        let local = Path::new("/tmp/rustconn-remote-edit-1/hosts");
        assert_eq!(
            build_get_batch("/etc/hosts", local),
            "get \"/etc/hosts\" \"/tmp/rustconn-remote-edit-1/hosts\"\n"
        );
        assert_eq!(
            build_put_batch(local, "/etc/hosts"),
            "put \"/tmp/rustconn-remote-edit-1/hosts\" \"/etc/hosts\"\n"
        );
        // Quotes and backslashes in a remote path cannot escape the batch line.
        assert_eq!(
            build_get_batch("a\"b\\c", local),
            "get \"a\\\"b\\\\c\" \"/tmp/rustconn-remote-edit-1/hosts\"\n"
        );
    }

    #[test]
    fn unchanged_detection() {
        let original = b"line one\nline two\n";
        let hash = fnv1a(original);
        assert!(is_unchanged(original.len() as u64, hash, original));
        let edited = b"line one\nline THREE\n";
        assert!(!is_unchanged(original.len() as u64, hash, edited));
        // Same length, different bytes: the hash must still catch the edit.
        let same_len = b"line one\nline three";
        assert!(!is_unchanged(original.len() as u64, hash, same_len));
    }
}
