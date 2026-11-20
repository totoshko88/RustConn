//! MobaXterm-style "edit remote file" built on the `sftp` CLI.
//!
//! Download to a temporary copy, let the GUI edit it, upload on [`RemoteEditor::save`].
//! Authentication is key/agent only: `sftp -b` is non-interactive (`BatchMode=yes`),
//! no `SecretString` is available for an askpass helper, and a secret must never
//! reach argv or a batch file.
//!
//! Everything that decides what reaches a process (host/user validation, the
//! argument vectors, path quoting, the scp allow-list) is a pure function, so it
//! is unit-tested without a network.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, Instant};

use thiserror::Error;
use tokio::process::Command;

use crate::config::NetworkSettings;
use crate::models::{Connection, ConnectionGroup, PasswordSource, ProtocolConfig, SshConfig};

/// Largest file we will download or upload.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Upper bound for one `sftp`/`scp` run (connect, auth and the transfer).
const TRANSFER_TIMEOUT: Duration = Duration::from_secs(120);

/// How often a running transfer is checked against the size cap and deadline.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// Errors from opening or saving a remote file.
#[derive(Debug, Error)]
pub enum RemoteEditorError {
    /// The supplied remote path was empty.
    #[error("remote path must not be empty")]
    EmptyRemotePath,
    /// The remote path contained a NUL byte, newline or another control character.
    #[error("remote path contains a control character")]
    ControlInRemotePath,
    /// The connection is not an SSH or SFTP connection.
    #[error("the remote editor works with SSH and SFTP connections only")]
    NotSsh,
    /// The connection needs a password that cannot be passed safely.
    #[error(
        "this connection uses password authentication, which the remote editor \
         cannot pass to sftp without exposing it; use key-based authentication \
         or type the password in a terminal session"
    )]
    PasswordAuthUnsupported,
    /// The host could be read by ssh as an option or is not a plain name.
    #[error(
        "refusing host {0:?}: it must not start with '-' or contain spaces, '@' or control characters"
    )]
    InvalidHost(String),
    /// The user name could be read by ssh as an option or is not a plain name.
    #[error(
        "refusing user name {0:?}: it must not start with '-' or contain spaces or control characters"
    )]
    InvalidUser(String),
    /// A jump host entry could be read by ssh as an option.
    #[error(
        "refusing jump host {0:?}: it must not start with '-' or contain spaces or control characters"
    )]
    InvalidJumpHost(String),
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
    #[error("sftp failed (exit status {code:?}): {detail}")]
    SftpFailed {
        /// Exit code, `None` when killed by a signal.
        code: Option<i32>,
        /// Last lines the process printed.
        detail: String,
    },
    /// scp could not be started (fallback for hosts without an sftp subsystem).
    #[error("failed to start scp: {0}")]
    ScpSpawn(std::io::Error),
    /// scp exited unsuccessfully.
    #[error("scp failed (exit status {code:?}): {detail}")]
    ScpFailed {
        /// Exit code, `None` when killed by a signal.
        code: Option<i32>,
        /// Last lines the process printed.
        detail: String,
    },
    /// The host has no sftp subsystem and the path cannot go through scp safely.
    #[error(
        "this host has no SFTP subsystem and the path has characters the scp \
         fallback cannot pass safely (allowed: letters, digits and . _ / @ : + = , % -)"
    )]
    UnsafeScpPath,
    /// The transfer did not finish in time.
    #[error("the transfer did not finish within {0} seconds; check that the host is reachable")]
    Timeout(u64),
    /// The remote file is larger than the download limit.
    #[error("the remote file is larger than the {limit} byte download limit")]
    DownloadTooLarge {
        /// Maximum allowed size in bytes.
        limit: u64,
    },
    /// The local copy could not be read.
    #[error("failed to read local file: {0}")]
    ReadLocal(std::io::Error),
    /// The local copy could not be copied to the chosen place.
    #[error("failed to save a copy: {0}")]
    SaveCopy(std::io::Error),
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

/// Where and how to reach the remote host. Built once from a [`Connection`] on
/// the caller's thread, so the editor needs no connection list.
#[derive(Debug, Clone)]
pub struct RemoteTarget {
    /// Connection name, used only for log context.
    name: String,
    /// Validated host.
    host: String,
    /// SSH port.
    port: u16,
    /// Validated user name.
    username: Option<String>,
    /// Options for `sftp`/`scp`, from the shared SSH argument builder.
    ssh_args: Vec<String>,
    /// Resolved agent socket override, if any.
    agent_socket: Option<String>,
}

impl RemoteTarget {
    /// Builds a target from an SSH/SFTP connection.
    ///
    /// Key file, jump host chain (own, group, global, `jump_host_id`),
    /// `ProxyCommand`, custom options and agent socket come from the same
    /// builders the terminal uses ([`SshConfig::build_command_args`] and
    /// [`crate::connection::resolve_proxy_jump_value`]).
    ///
    /// # Errors
    /// Returns an error for non-SSH connections, password authentication, or a
    /// host, user or jump host that ssh could read as an option.
    pub fn from_connection(
        connection: &Connection,
        connections: &[Connection],
        groups: &[ConnectionGroup],
        network: &NetworkSettings,
        global_agent_socket: Option<&str>,
    ) -> Result<Self, RemoteEditorError> {
        let (ProtocolConfig::Ssh(cfg) | ProtocolConfig::Sftp(cfg)) = &connection.protocol_config
        else {
            return Err(RemoteEditorError::NotSsh);
        };
        if !matches!(connection.password_source, PasswordSource::None) {
            return Err(RemoteEditorError::PasswordAuthUnsupported);
        }
        validate_host(&connection.host)?;
        if let Some(user) = connection.username.as_deref() {
            validate_user(user)?;
        }

        let jump =
            crate::connection::resolve_proxy_jump_value(connection, connections, groups, network);
        if let Some(spec) = jump.as_deref() {
            validate_jump_spec(spec)?;
        }
        let inherited_key =
            crate::connection::ssh_inheritance::resolve_ssh_key_path(connection, groups);
        // Connection, then group chain, then the Settings socket and the rest.
        let agent_socket = crate::sftp::resolve_ssh_agent_socket(
            crate::connection::ssh_inheritance::resolve_ssh_agent_socket(connection, groups)
                .as_deref(),
            global_agent_socket,
        );

        Ok(Self {
            name: connection.name.clone(),
            host: connection.host.clone(),
            port: connection.port,
            username: connection.username.clone(),
            ssh_args: transfer_ssh_args(cfg, jump.as_deref(), inherited_key.as_deref()),
            agent_socket,
        })
    }

    fn destination(&self) -> String {
        let host = bracket_host(&self.host);
        match &self.username {
            Some(user) => format!("{user}@{host}"),
            None => host,
        }
    }

    fn command(&self, program: &str, args: &[String]) -> Command {
        let mut cmd = Command::new(program);
        cmd.args(args);
        // `agent_socket` is already resolved; it wins over the Settings socket.
        crate::sftp::apply_agent_env_async(&mut cmd, self.agent_socket.as_deref());
        cmd
    }
}

/// A remote file checked out into a temporary local copy for editing.
#[derive(Debug)]
pub struct RemoteEditor {
    /// How to reach the host.
    target: RemoteTarget,
    /// Original remote path, used for the upload command.
    remote_path: String,
    /// Local temporary copy the GUI edits.
    local_path: PathBuf,
    /// Keeps the temporary directory alive until the editor is dropped. An
    /// upload error leaves the editor (and this directory) with the caller.
    temp_dir: tempfile::TempDir,
    /// Byte length of the file as first downloaded.
    original_len: u64,
    /// FNV-1a hash of the file as first downloaded.
    original_hash: u64,
}

impl RemoteEditor {
    /// Downloads `remote_path` into a temporary local copy.
    ///
    /// Runs the `sftp` CLI with a batch file, a null stdin, `BatchMode=yes` and a
    /// connect timeout, never a shell. Falls back to `scp -O` only when the host
    /// has no sftp subsystem; a missing file or a refused login shows the real
    /// error instead.
    ///
    /// # Errors
    /// Returns a path-validation, spawn, timeout, size or transfer error.
    pub async fn open(target: RemoteTarget, remote_path: &str) -> Result<Self, RemoteEditorError> {
        validate_remote_path(remote_path)?;

        let temp_dir = tempfile::Builder::new()
            .prefix("rustconn-remote-edit-")
            .tempdir()
            .map_err(RemoteEditorError::TempDir)?;

        let local_path = temp_dir.path().join(local_filename(remote_path));
        let batch_path = temp_dir.path().join("get.batch");
        write_batch(&batch_path, &build_get_batch(remote_path, &local_path))?;
        let err_path = temp_dir.path().join("transfer.log");

        transfer(
            &target,
            &batch_path,
            &err_path,
            Some((&local_path, MAX_FILE_BYTES)),
            || scp_transfer(&target, remote_path, &local_path, false, &err_path),
        )
        .await?;

        restrict_permissions(&local_path);
        let bytes = tokio::fs::read(&local_path)
            .await
            .map_err(RemoteEditorError::ReadLocal)?;
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(RemoteEditorError::DownloadTooLarge {
                limit: MAX_FILE_BYTES,
            });
        }
        let original_len = bytes.len() as u64;
        let original_hash = fnv1a(&bytes);

        tracing::info!(
            connection = %target.name,
            host = %target.host,
            bytes = original_len,
            "remote file opened for editing"
        );

        Ok(Self {
            target,
            remote_path: remote_path.to_string(),
            local_path,
            temp_dir,
            original_len,
            original_hash,
        })
    }

    /// Uploads the local copy if it changed since [`Self::open`].
    ///
    /// Takes `&self`: on error the editor and its temporary directory stay with
    /// the caller, so the edit is never lost and the call can be repeated.
    ///
    /// # Errors
    /// Returns a read, size, spawn, timeout or transfer error.
    pub async fn save(&self) -> Result<SaveOutcome, RemoteEditorError> {
        let bytes = tokio::fs::read(&self.local_path)
            .await
            .map_err(RemoteEditorError::ReadLocal)?;
        if is_unchanged(self.original_len, self.original_hash, &bytes) {
            return Ok(SaveOutcome::Unchanged);
        }

        let size = bytes.len() as u64;
        if size > MAX_FILE_BYTES {
            return Err(RemoteEditorError::FileTooLarge {
                size,
                limit: MAX_FILE_BYTES,
            });
        }

        let batch_path = self.temp_dir.path().join("put.batch");
        write_batch(
            &batch_path,
            &build_put_batch(&self.local_path, &self.remote_path),
        )?;
        let err_path = self.temp_dir.path().join("transfer.log");

        transfer(&self.target, &batch_path, &err_path, None, || {
            scp_transfer(
                &self.target,
                &self.remote_path,
                &self.local_path,
                true,
                &err_path,
            )
        })
        .await?;

        tracing::info!(
            connection = %self.target.name,
            host = %self.target.host,
            bytes = size,
            "remote file uploaded"
        );
        Ok(SaveOutcome::Uploaded)
    }

    /// Copies the edited file to `dest`, for "Save a copy…" after a failed upload.
    ///
    /// # Errors
    /// Returns [`RemoteEditorError::SaveCopy`] when the copy fails.
    pub fn save_copy(&self, dest: &Path) -> Result<(), RemoteEditorError> {
        std::fs::copy(&self.local_path, dest)
            .map(|_| ())
            .map_err(RemoteEditorError::SaveCopy)
    }

    /// Path of the local temporary copy, for the GUI to open in a text editor.
    #[must_use]
    pub fn local_path(&self) -> &Path {
        &self.local_path
    }
}

/// Runs the sftp batch; falls back to `scp_fallback` only when the host has no
/// sftp subsystem (see [`should_fall_back_to_scp`]).
async fn transfer<F, Fut>(
    target: &RemoteTarget,
    batch: &Path,
    err_path: &Path,
    watch: Option<(&Path, u64)>,
    scp_fallback: F,
) -> Result<(), RemoteEditorError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<(), RemoteEditorError>>,
{
    match run(
        target.command("sftp", &sftp_argv(target, batch)),
        err_path,
        watch,
    )
    .await
    {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => {
            let detail = read_detail(err_path);
            if should_fall_back_to_scp(&detail) {
                tracing::info!(code = ?status.code(), "no sftp subsystem; falling back to scp -O");
                scp_fallback().await
            } else {
                Err(RemoteEditorError::SftpFailed {
                    code: status.code(),
                    detail,
                })
            }
        }
        Err(RunError::Spawn(e)) => Err(RemoteEditorError::SftpSpawn(e)),
        Err(e) => Err(e.into_editor_error()),
    }
}

/// Text ssh could read as an option, or that a command line cannot carry.
fn is_plain_token(s: &str) -> bool {
    !s.is_empty() && !s.starts_with('-') && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Host must be a plain token without `@` (the user separator).
fn validate_host(host: &str) -> Result<(), RemoteEditorError> {
    if is_plain_token(host) && !host.contains('@') {
        Ok(())
    } else {
        Err(RemoteEditorError::InvalidHost(host.to_string()))
    }
}

/// User must be a plain token.
fn validate_user(user: &str) -> Result<(), RemoteEditorError> {
    if is_plain_token(user) {
        Ok(())
    } else {
        Err(RemoteEditorError::InvalidUser(user.to_string()))
    }
}

/// Every comma-separated hop of a `-J` value must be a plain token.
fn validate_jump_spec(spec: &str) -> Result<(), RemoteEditorError> {
    match spec.split(',').find(|hop| !is_plain_token(hop)) {
        Some(hop) => Err(RemoteEditorError::InvalidJumpHost(hop.to_string())),
        None => Ok(()),
    }
}

/// IPv6 literals need brackets in `user@[::1]` and `user@[::1]:path`.
fn bracket_host(host: &str) -> String {
    if host.contains(':') && !host.starts_with('[') {
        format!("[{host}]")
    } else {
        host.to_string()
    }
}

/// Turns the terminal's ssh options into options `sftp`/`scp` accept.
///
/// Starts from [`SshConfig::build_command_args`] (key, `ProxyCommand`, custom
/// options, keep-alive, `ConnectTimeout`). Drops what only `ssh` understands
/// (`-A`, `-X`, port forwards) and the config's own `-J`, which is replaced by
/// the fully resolved chain. `BatchMode=yes` goes first because ssh keeps the
/// first value given for an option.
fn transfer_ssh_args(
    cfg: &SshConfig,
    jump: Option<&str>,
    inherited_key: Option<&Path>,
) -> Vec<String> {
    let mut out = vec!["-o".to_string(), "BatchMode=yes".to_string()];
    let mut has_key = false;
    let mut it = cfg.build_command_args().into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "-A" | "-X" => {}
            "-J" | "-L" | "-R" | "-D" => {
                it.next();
            }
            "-o" | "-i" => {
                has_key |= arg == "-i";
                out.push(arg);
                out.extend(it.next());
            }
            _ => out.push(arg),
        }
    }
    if let Some(jump) = jump
        && cfg.proxy_command.is_none()
    {
        out.push("-J".to_string());
        out.push(jump.to_string());
    }
    if !has_key && let Some(key) = inherited_key {
        out.push("-i".to_string());
        out.push(key.to_string_lossy().into_owned());
    }
    out
}

/// `sftp` argument vector. `--` ends options, so the destination can never be
/// taken for one.
fn sftp_argv(target: &RemoteTarget, batch: &Path) -> Vec<String> {
    let mut v = vec![
        "-b".to_string(),
        batch.to_string_lossy().into_owned(),
        "-P".to_string(),
        target.port.to_string(),
    ];
    v.extend(target.ssh_args.iter().cloned());
    v.push("--".to_string());
    v.push(target.destination());
    v
}

/// `scp` argument vector. `-O` forces the original SCP protocol: OpenSSH 9+
/// otherwise tries SFTP first, which is what Dropbear/OpenWrt hosts lack.
fn scp_argv(target: &RemoteTarget, remote_path: &str, local: &Path, upload: bool) -> Vec<String> {
    let mut v = vec!["-O".to_string(), "-P".to_string(), target.port.to_string()];
    v.extend(target.ssh_args.iter().cloned());
    v.push("--".to_string());
    let remote = format!("{}:{remote_path}", target.destination());
    let local = local.to_string_lossy().into_owned();
    if upload {
        v.push(local);
        v.push(remote);
    } else {
        v.push(remote);
        v.push(local);
    }
    v
}

/// Whether an sftp failure means "this host has no sftp subsystem".
///
/// Allow-list: only the messages Dropbear and minimal servers produce lead to
/// scp. A missing file, a refused login, a bad host key or a timeout show their
/// own error, because scp would only repeat them less clearly.
fn should_fall_back_to_scp(stderr: &str) -> bool {
    const NO_SUBSYSTEM: &[&str] = &[
        "subsystem request failed",
        "received message too long",
        "sftp-server: not found",
        "sftp-server: command not found",
    ];
    let s = stderr.to_lowercase();
    NO_SUBSYSTEM.iter().any(|m| s.contains(m))
}

/// Rejects paths that would break or inject into a line-based sftp batch file.
fn validate_remote_path(path: &str) -> Result<(), RemoteEditorError> {
    if path.is_empty() {
        return Err(RemoteEditorError::EmptyRemotePath);
    }
    if path.chars().any(char::is_control) {
        return Err(RemoteEditorError::ControlInRemotePath);
    }
    Ok(())
}

/// The scp fallback sends the path through the remote shell: allow only
/// characters no shell treats specially.
fn is_scp_safe_path(path: &str) -> bool {
    !path.is_empty()
        && path
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._/@:+=,%-".contains(c))
}

/// Local file name for the copy: keeps letters, digits and `. _ -`, so it has
/// no glob characters or spaces and does not start with a dot.
fn local_filename(remote_path: &str) -> String {
    let name: String = remote_path
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("")
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || "._-".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect();
    let name = name.trim_start_matches('.');
    if name.is_empty() {
        "remote-file".to_string()
    } else {
        name.to_string()
    }
}

/// Escapes the characters sftp's glob treats as patterns, so `get` of a file
/// named `a[1]*.conf` addresses exactly that file.
fn escape_glob(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        if matches!(c, '\\' | '*' | '?' | '[') {
            out.push('\\');
        }
        out.push(c);
    }
    out
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
        quote(&escape_glob(remote)),
        quote(&local.to_string_lossy())
    )
}

/// Contents of the upload batch file.
fn build_put_batch(local: &Path, remote: &str) -> String {
    format!(
        "put {} {}\n",
        quote(&escape_glob(&local.to_string_lossy())),
        quote(remote)
    )
}

/// Writes a batch file; these files never contain credentials.
fn write_batch(path: &Path, content: &str) -> Result<(), RemoteEditorError> {
    std::fs::write(path, content).map_err(RemoteEditorError::WriteBatch)
}

/// The downloaded copy may carry the remote execute bit; make it private and
/// not executable.
fn restrict_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Last lines the process printed.
fn read_detail(path: &Path) -> String {
    summarize_output(&std::fs::read_to_string(path).unwrap_or_default())
}

/// Last three non-empty lines, joined, without control characters, max 400 chars.
fn summarize_output(text: &str) -> String {
    let lines: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect();
    let tail = lines[lines.len().saturating_sub(3)..].join(" / ");
    let clean: String = tail.chars().filter(|c| !c.is_control()).take(400).collect();
    if clean.is_empty() {
        "no output".to_string()
    } else {
        clean
    }
}

/// Why a child process run ended without an exit status.
enum RunError {
    Spawn(std::io::Error),
    Wait(std::io::Error),
    Timeout,
    TooLarge,
}

impl RunError {
    fn into_editor_error(self) -> RemoteEditorError {
        match self {
            Self::Spawn(e) | Self::Wait(e) => RemoteEditorError::SftpSpawn(e),
            Self::Timeout => RemoteEditorError::Timeout(TRANSFER_TIMEOUT.as_secs()),
            Self::TooLarge => RemoteEditorError::DownloadTooLarge {
                limit: MAX_FILE_BYTES,
            },
        }
    }
}

/// Runs a command with null stdin/stdout and stderr written to `err_path`.
/// Kills it after [`TRANSFER_TIMEOUT`], or when `watch` (a growing download)
/// passes its size cap.
async fn run(
    mut cmd: Command,
    err_path: &Path,
    watch: Option<(&Path, u64)>,
) -> Result<std::process::ExitStatus, RunError> {
    let err_file = std::fs::File::create(err_path).map_err(RunError::Spawn)?;
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::from(err_file))
        .kill_on_drop(true);
    let mut child = cmd.spawn().map_err(RunError::Spawn)?;
    let deadline = Instant::now() + TRANSFER_TIMEOUT;
    loop {
        tokio::select! {
            status = child.wait() => return status.map_err(RunError::Wait),
            () = tokio::time::sleep(POLL_INTERVAL) => {
                let too_big = watch.is_some_and(|(path, cap)| {
                    std::fs::metadata(path).is_ok_and(|m| m.len() > cap)
                });
                if too_big || Instant::now() >= deadline {
                    let _ = child.kill().await;
                    return Err(if too_big { RunError::TooLarge } else { RunError::Timeout });
                }
            }
        }
    }
}

/// Download (`upload == false`) or upload (`upload == true`) via legacy SCP.
async fn scp_transfer(
    target: &RemoteTarget,
    remote_path: &str,
    local: &Path,
    upload: bool,
    err_path: &Path,
) -> Result<(), RemoteEditorError> {
    if !is_scp_safe_path(remote_path) {
        return Err(RemoteEditorError::UnsafeScpPath);
    }
    let argv = scp_argv(target, remote_path, local, upload);
    let watch = (!upload).then_some((local, MAX_FILE_BYTES));
    match run(target.command("scp", &argv), err_path, watch).await {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(RemoteEditorError::ScpFailed {
            code: status.code(),
            detail: read_detail(err_path),
        }),
        Err(RunError::Spawn(e)) => Err(RemoteEditorError::ScpSpawn(e)),
        Err(e) => Err(e.into_editor_error()),
    }
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
    use crate::models::{SshAuthMethod, SshKeySource};

    fn ssh_conn(host: &str, user: Option<&str>) -> Connection {
        let mut c = Connection::new(
            "t".to_string(),
            host.to_string(),
            22,
            ProtocolConfig::Ssh(SshConfig::default()),
        );
        c.username = user.map(str::to_string);
        c
    }

    fn target_of(conn: &Connection) -> Result<RemoteTarget, RemoteEditorError> {
        RemoteTarget::from_connection(conn, &[], &[], &NetworkSettings::default(), None)
    }

    #[test]
    fn hostile_host_is_refused() {
        for host in [
            "-oProxyCommand=x",
            "-F/tmp/x",
            "a b",
            "host\n",
            "",
            "u@host",
        ] {
            let err = target_of(&ssh_conn(host, None)).expect_err(host);
            assert!(matches!(err, RemoteEditorError::InvalidHost(_)), "{host}");
        }
        assert!(target_of(&ssh_conn("example.com", None)).is_ok());
        assert!(target_of(&ssh_conn("::1", None)).is_ok());
        assert!(target_of(&ssh_conn("münchen.example", Some("o'brien"))).is_ok());
    }

    fn env_of(cmd: &Command, name: &str) -> Option<String> {
        cmd.as_std()
            .get_envs()
            .find(|(k, _)| *k == name)
            .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    /// F-01: the sftp/scp of a remote edit reaches the agent the connection,
    /// its group or Settings names, in that order.
    #[test]
    fn agent_socket_reaches_the_child_env_in_precedence_order() {
        let mut group = ConnectionGroup::new("g".to_string());
        group.ssh_agent_socket = Some("/run/rc-group.sock".to_string());
        let mut conn = ssh_conn("example.com", None);
        conn.group_id = Some(group.id);
        let groups = [group];
        let target = |conn: &Connection, global: Option<&str>| {
            RemoteTarget::from_connection(conn, &[], &groups, &NetworkSettings::default(), global)
                .expect("target")
        };

        let t = target(&conn, Some("/run/rc-settings.sock"));
        assert_eq!(
            env_of(&t.command("sftp", &[]), "SSH_AUTH_SOCK").as_deref(),
            Some("/run/rc-group.sock"),
            "an inherited group socket beats Settings"
        );

        if let ProtocolConfig::Ssh(cfg) = &mut conn.protocol_config {
            cfg.ssh_agent_socket = Some("/run/rc-conn.sock".to_string());
        }
        let t = target(&conn, Some("/run/rc-settings.sock"));
        assert_eq!(
            env_of(&t.command("scp", &[]), "SSH_AUTH_SOCK").as_deref(),
            Some("/run/rc-conn.sock")
        );

        conn.group_id = None;
        if let ProtocolConfig::Ssh(cfg) = &mut conn.protocol_config {
            cfg.ssh_agent_socket = None;
        }
        let t = target(&conn, Some("/run/rc-settings.sock"));
        assert_eq!(
            env_of(&t.command("sftp", &[]), "SSH_AUTH_SOCK").as_deref(),
            Some("/run/rc-settings.sock")
        );
    }

    #[test]
    fn hostile_user_is_refused() {
        for user in ["-oProxyCommand=x", "a b", "u\tx", ""] {
            let err = target_of(&ssh_conn("example.com", Some(user))).expect_err(user);
            assert!(matches!(err, RemoteEditorError::InvalidUser(_)), "{user}");
        }
        assert!(target_of(&ssh_conn("example.com", Some("user@domain"))).is_ok());
    }

    #[test]
    fn hostile_jump_hop_is_refused() {
        assert!(validate_jump_spec("bastion,user@jump:2222").is_ok());
        assert!(matches!(
            validate_jump_spec("bastion,-oProxyCommand=x"),
            Err(RemoteEditorError::InvalidJumpHost(_))
        ));
        assert!(validate_jump_spec("a b").is_err());
    }

    #[test]
    fn non_ssh_and_password_connections_are_refused() {
        let mut c = ssh_conn("example.com", None);
        c.password_source = PasswordSource::Prompt;
        assert!(matches!(
            target_of(&c),
            Err(RemoteEditorError::PasswordAuthUnsupported)
        ));
        let rdp = Connection::new_rdp("r".to_string(), "example.com".to_string(), 3389);
        assert!(matches!(target_of(&rdp), Err(RemoteEditorError::NotSsh)));
    }

    #[test]
    fn argv_has_double_dash_before_destination() {
        let t = target_of(&ssh_conn("example.com", Some("root"))).expect("target");
        let argv = sftp_argv(&t, Path::new("/tmp/b"));
        let dd = argv.iter().position(|a| a == "--").expect("--");
        assert_eq!(argv[dd + 1], "root@example.com");
        assert_eq!(dd + 2, argv.len());
        assert!(argv.windows(2).any(|w| w == ["-o", "BatchMode=yes"]));
        assert!(
            argv.iter().any(|a| a.starts_with("ConnectTimeout=")),
            "connect timeout from the shared builder: {argv:?}"
        );

        let scp = scp_argv(&t, "/etc/hosts", Path::new("/tmp/l"), false);
        let dd = scp.iter().position(|a| a == "--").expect("--");
        assert_eq!(scp[dd + 1], "root@example.com:/etc/hosts");
        assert_eq!(scp[dd + 2], "/tmp/l");
        let up = scp_argv(&t, "/etc/hosts", Path::new("/tmp/l"), true);
        assert_eq!(up[up.len() - 2], "/tmp/l");
    }

    #[test]
    fn ipv6_host_is_bracketed() {
        let t = target_of(&ssh_conn("fe80::1", Some("u"))).expect("target");
        assert_eq!(t.destination(), "u@[fe80::1]");
    }

    #[test]
    fn ssh_settings_reach_the_transfer_args() {
        let mut cfg = SshConfig {
            key_source: SshKeySource::File {
                path: PathBuf::from("/keys/id"),
            },
            auth_method: SshAuthMethod::PublicKey,
            agent_forwarding: true,
            x11_forwarding: true,
            proxy_jump: Some("own-jump".to_string()),
            ..SshConfig::default()
        };
        cfg.custom_options
            .insert("StrictHostKeyChecking".to_string(), "no".to_string());
        let args = transfer_ssh_args(&cfg, Some("g-bastion,own-jump"), None);
        assert_eq!(&args[..2], ["-o", "BatchMode=yes"]);
        assert!(args.windows(2).any(|w| w == ["-i", "/keys/id"]));
        assert!(
            args.windows(2)
                .any(|w| w == ["-o", "StrictHostKeyChecking=no"])
        );
        assert!(args.windows(2).any(|w| w == ["-J", "g-bastion,own-jump"]));
        assert_eq!(args.iter().filter(|a| *a == "-J").count(), 1);
        assert!(!args.iter().any(|a| a == "-A" || a == "-X"));
    }

    #[test]
    fn proxy_command_wins_over_jump_and_forwards_are_dropped() {
        let cfg = SshConfig {
            proxy_command: Some("nc -X 5 %h %p".to_string()),
            ..SshConfig::default()
        };
        let args = transfer_ssh_args(&cfg, Some("bastion"), Some(Path::new("/k/inherited")));
        assert!(!args.iter().any(|a| a == "-J"));
        assert!(
            args.windows(2)
                .any(|w| w == ["-o", "ProxyCommand=nc -X 5 %h %p"])
        );
        assert!(args.windows(2).any(|w| w == ["-i", "/k/inherited"]));
    }

    #[test]
    fn validates_remote_path() {
        assert!(matches!(
            validate_remote_path(""),
            Err(RemoteEditorError::EmptyRemotePath)
        ));
        for bad in [
            "/etc/hosts\0.bak",
            "/etc/hosts\nrm -rf /",
            "/etc/hosts\rput x y",
        ] {
            assert!(matches!(
                validate_remote_path(bad),
                Err(RemoteEditorError::ControlInRemotePath)
            ));
        }
        assert!(validate_remote_path("/var/log/app.log").is_ok());
        assert!(validate_remote_path("/srv/my dir/файл.conf").is_ok());
    }

    #[test]
    fn scp_allow_list() {
        assert!(is_scp_safe_path("/etc/config/network"));
        assert!(is_scp_safe_path("/home/u/a-b_c.d+e=f,g%h@i:j"));
        for bad in [
            "/etc/a b",
            "/etc/a;b",
            "/etc/a\\b",
            "/etc/{a,b}",
            "~/x",
            "/etc/a!b",
            "/etc/$x",
            "/etc/a*",
            "/etc/файл",
            "",
        ] {
            assert!(!is_scp_safe_path(bad), "{bad}");
        }
    }

    #[test]
    fn scp_fallback_only_without_sftp_subsystem() {
        assert!(should_fall_back_to_scp(
            "subsystem request failed on channel 0\nConnection closed"
        ));
        assert!(should_fall_back_to_scp(
            "Received message too long 1349281121"
        ));
        for real in [
            "root@h: Permission denied (publickey).\nConnection closed",
            "File \"/nope\" not found.",
            "stat remote: No such file or directory",
            "ssh: Could not resolve hostname x",
            "Connection timed out during banner exchange",
            "Host key verification failed.",
            "",
        ] {
            assert!(!should_fall_back_to_scp(real), "{real}");
        }
    }

    #[tokio::test]
    async fn scp_fallback_refuses_unsafe_path() {
        let t = target_of(&ssh_conn("example.com", Some("root"))).expect("target");
        let err = scp_transfer(
            &t,
            "/etc/hosts; rm -rf /",
            Path::new("/tmp/local"),
            false,
            Path::new("/tmp/unused.log"),
        )
        .await
        .expect_err("must be refused");
        assert!(matches!(err, RemoteEditorError::UnsafeScpPath));
    }

    #[test]
    fn local_name_is_sanitized() {
        assert_eq!(local_filename("/etc/hosts"), "hosts");
        assert_eq!(local_filename("/a/b[1]*?.conf"), "b_1___.conf");
        assert_eq!(local_filename("/a/my file.txt"), "my_file.txt");
        assert_eq!(local_filename("/a/файл.conf"), "файл.conf");
        assert_eq!(local_filename("/a/.bashrc"), "bashrc");
        assert_eq!(local_filename("/a/.."), "remote-file");
        assert_eq!(local_filename("/"), "remote-file");
        assert_eq!(local_filename("C:\\x\\y.ini"), "y.ini");
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
        // Quotes cannot escape the batch line; a backslash is glob-escaped, then quoted.
        assert_eq!(
            build_get_batch("a\"b\\c", local),
            "get \"a\\\"b\\\\\\\\c\" \"/tmp/rustconn-remote-edit-1/hosts\"\n"
        );
        // Glob characters in a remote name address that exact file.
        assert_eq!(
            build_get_batch("/x/a[1]*?.conf", local),
            "get \"/x/a\\\\[1]\\\\*\\\\?.conf\" \"/tmp/rustconn-remote-edit-1/hosts\"\n"
        );
        // Spaces and unicode stay inside the quotes.
        assert_eq!(
            build_get_batch("/x/мій файл", local),
            "get \"/x/мій файл\" \"/tmp/rustconn-remote-edit-1/hosts\"\n"
        );
    }

    #[test]
    fn output_summary_is_short_and_clean() {
        assert_eq!(summarize_output(""), "no output");
        assert_eq!(summarize_output("a\n\n b \nc\nd\x1b[0m\n"), "b / c / d[0m");
        assert!(summarize_output(&"x".repeat(5000)).len() <= 400);
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

    #[cfg(unix)]
    #[tokio::test]
    async fn run_kills_a_download_over_the_cap_and_keeps_stderr() {
        let dir = tempfile::tempdir().expect("tempdir");
        let data = dir.path().join("data");
        let log = dir.path().join("log");
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(format!(
            "head -c 2000000 /dev/zero > {}; sleep 30",
            data.display()
        ));
        let started = Instant::now();
        let res = run(cmd, &log, Some((&data, 1_000_000))).await;
        assert!(matches!(res, Err(RunError::TooLarge)));
        assert!(started.elapsed() < Duration::from_secs(10));

        let mut cmd = Command::new("sh");
        cmd.arg("-c")
            .arg("echo 'Permission denied (publickey).' >&2; exit 1");
        let status = run(cmd, &log, None).await.ok().expect("status");
        assert!(!status.success());
        assert_eq!(read_detail(&log), "Permission denied (publickey).");
    }
}
