//! Headless core of the "Set Up Key Authentication…" feature (K2).
//!
//! The feature takes a connection that logs in with a password and gives it a
//! key: it generates a key pair with `ssh-keygen`, installs the public half on
//! the server, then proves the server accepts it. This module is the part with
//! no widgets in it; the dialog that drives it lives in the `rustconn` crate.
//! Design and decisions: `.agents/tasks/ssh-key-provisioning-design.md`.
//!
//! # Pieces
//!
//! * **Naming and location** — [`key_setup_dir`], [`connection_slug`],
//!   [`key_file_stem`], [`resolve_key_slot`], [`key_comment`]. Keys live in
//!   `<ssh dir>/rustconn/` (mode 0700) under `<slug>-<id8>_<type>`; a repeat run
//!   finds and reuses the pair it generated before ([`KeySlot::Reusable`]).
//! * **Generation** — [`generate_key`] runs `ssh-keygen`. Ed25519 by default,
//!   no passphrase by default.
//! * **Deploy** — [`run_deploy`] feeds the public key to [`DEPLOY_SCRIPT`] on
//!   stdin. The script is our own, idempotent, and takes nothing but the key
//!   from the outside: the key goes in through stdin and is never interpolated,
//!   so a hostile slug, host or comment has nowhere to inject into.
//! * **Verify** — [`run_verify`] logs in with the new key alone and succeeds
//!   only when the server's `-v` output names the new key's fingerprint.
//! * **Classification** — [`classify_deploy`] and [`classify_verify`] turn exit
//!   code plus captured text into a [`KeySetupError`] variant. They are pure and
//!   tested on captured samples.
//!
//! # Rules that shape the code
//!
//! * `StrictHostKeyChecking=yes` is forced on every control-channel connection
//!   and beats whatever `custom_options` say. OpenSSH keeps the **first** value
//!   of an option, so [`control_channel_args`] puts its override prefix first
//!   and also strips the conflicting options from the session arguments. An
//!   unknown or changed host key is its own error ([`KeySetupError::HostKeyUnknown`],
//!   [`KeySetupError::HostKeyChanged`]) so the caller can offer to connect
//!   normally and accept the key there.
//! * Verification disables multiplexing, the agent and every method but
//!   `publickey`, so a lucky pre-existing login cannot pass for the new key.
//! * Passwords reach `ssh` through the K1 askpass helper
//!   ([`crate::ssh_askpass`]): the environment carries the *path* of an
//!   owner-only file, never a value, and argv carries nothing. A key
//!   passphrase takes the tunnel helper path for the same reason and is never
//!   stored anywhere; the files are removed when the run ends, including on
//!   early return.
//! * Stderr of `ssh` is logged at `debug` (truncated) and is never put into an
//!   error value.
//!
//! All runners block; call them from a worker thread.

use std::collections::VecDeque;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use secrecy::SecretString;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::proc::{self, Waited};
use crate::ssh_agent::{self, KeyEncryption, KeyFormat, PublicKeyInfo};
use crate::ssh_askpass::{self, AskpassSlot};
use crate::ssh_tunnel;

/// Directory name under the SSH directory that holds generated keys.
pub const KEY_SETUP_SUBDIR: &str = "rustconn";

/// Default wall-clock budget for one `ssh`/`ssh-keygen` run.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(60);

/// Printed by [`DEPLOY_SCRIPT`] when it appended the key.
pub const SENTINEL_INSTALLED: &str = "RUSTCONN-KEY-INSTALLED";

/// Printed by [`DEPLOY_SCRIPT`] when the key was already in `authorized_keys`.
pub const SENTINEL_PRESENT: &str = "RUSTCONN-KEY-PRESENT";

/// Longest public-key line accepted, in bytes.
const MAX_PUBLIC_KEY_LEN: usize = 16 * 1024;

/// Longest connection slug, in characters.
const SLUG_MAX_LEN: usize = 32;

/// How many `-N` suffixes [`resolve_key_slot`] tries before giving up.
const MAX_SLOT_ATTEMPTS: u32 = 100;

/// How much of a captured stream is written to the debug log.
const LOG_LIMIT: usize = 4096;

/// The idempotent installer run on the server as `sh -c '<script>'`.
///
/// One line, with no `'`, newline or `!`, because it is pasted between single
/// quotes into a command string that the account's *login shell* parses first
/// (sh, bash, zsh, fish and csh all have to read it the same way). The key
/// arrives on stdin only. Exit codes: 10 `mkdir`, 11 creating
/// `authorized_keys`, 12 no key on stdin, 13 appending. `umask 077` gives a new
/// `~/.ssh` mode 0700 and a new `authorized_keys` mode 0600; the modes of files
/// that already exist are left alone, and a bad mode shows up in verification.
pub const DEPLOY_SCRIPT: &str = concat!(
    "umask 077; ",
    "d=\"$HOME/.ssh\"; ",
    "f=\"$d/authorized_keys\"; ",
    "IFS= read -r k || [ -n \"$k\" ] || exit 12; ",
    "[ -n \"$k\" ] || exit 12; ",
    "mkdir -p \"$d\" || exit 10; ",
    // A subshell, because `:` is a special builtin: in dash a failed redirection
    // on it would end the whole script with status 2 instead of reaching `exit 11`.
    "[ -f \"$f\" ] || ( : > \"$f\" ) || exit 11; ",
    "if grep -qxF -- \"$k\" \"$f\" 2>/dev/null; then echo RUSTCONN-KEY-PRESENT; exit 0; fi; ",
    "if [ -s \"$f\" ] && [ -n \"$(tail -c 1 \"$f\")\" ]; then echo >> \"$f\" || exit 13; fi; ",
    "printf \"%s\\n\" \"$k\" >> \"$f\" || exit 13; ",
    "command -v restorecon >/dev/null 2>&1 && restorecon -F \"$d\" \"$f\" >/dev/null 2>&1; ",
    "echo RUSTCONN-KEY-INSTALLED",
);

/// Options forced in front of every control-channel `ssh` command, in order.
///
/// OpenSSH uses the first value it sees for an option, so these win over any
/// later `-o` from the connection's custom options.
const OVERRIDE_OPTIONS: [&str; 10] = [
    "StrictHostKeyChecking=yes",
    "ControlMaster=no",
    "ControlPath=none",
    "ClearAllForwardings=yes",
    "ForwardAgent=no",
    "ForwardX11=no",
    "RemoteCommand=none",
    "RequestTTY=no",
    "PermitLocalCommand=no",
    "NumberOfPasswordPrompts=1",
];

/// Further `-o` keys dropped from session arguments, besides
/// [`OVERRIDE_OPTIONS`]: forwarding, multiplexing persistence, commands, and the
/// target itself (the tail sets the port, user and host from the [`SshTarget`]).
const STRIPPED_OPTION_KEYS: [&str; 14] = [
    "ControlPersist",
    "LocalForward",
    "RemoteForward",
    "DynamicForward",
    "BatchMode",
    "SessionType",
    "StdinNull",
    "ForkAfterAuthentication",
    "LocalCommand",
    "Tunnel",
    "ExitOnForwardFailure",
    "Port",
    "User",
    "HostName",
];

/// Extra `-o` keys dropped when the connection's own identity must not leak
/// into verification.
const IDENTITY_OPTION_KEYS: [&str; 8] = [
    "IdentityFile",
    "CertificateFile",
    "IdentitiesOnly",
    "IdentityAgent",
    "PreferredAuthentications",
    "PasswordAuthentication",
    "KbdInteractiveAuthentication",
    "PubkeyAuthentication",
];

/// Lone flags dropped from session arguments (forwarding, agent, tty, pty
/// allocation, background, multiplexing and the like).
const STRIPPED_FLAGS: &str = "AaXxYCTtNfMnGsKk";

/// Flags that take a value (attached or as the next token).
const VALUE_FLAGS: &str = "BbcDEeFIiJLlmOoPpQRSWw";

/// Value-taking flags dropped together with their value.
const STRIPPED_VALUE_FLAGS: &str = "DEeLOQRSWw";

/// Key algorithm for a generated key pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum KeyAlgorithm {
    /// `ssh-ed25519`; the default.
    #[default]
    Ed25519,
    /// `ssh-rsa` with a 4096-bit modulus, for servers that predate Ed25519.
    Rsa4096,
}

impl KeyAlgorithm {
    /// Suffix of the key file name (`ed25519` or `rsa`).
    #[must_use]
    pub const fn file_suffix(self) -> &'static str {
        match self {
            Self::Ed25519 => "ed25519",
            Self::Rsa4096 => "rsa",
        }
    }

    /// The type field of the public-key line.
    #[must_use]
    pub const fn ssh_type(self) -> &'static str {
        match self {
            Self::Ed25519 => "ssh-ed25519",
            Self::Rsa4096 => "ssh-rsa",
        }
    }

    /// The `ssh-keygen` arguments that select this algorithm.
    #[must_use]
    pub fn keygen_type_args(self) -> &'static [&'static str] {
        match self {
            Self::Ed25519 => &["-t", "ed25519"],
            Self::Rsa4096 => &["-t", "rsa", "-b", "4096"],
        }
    }
}

/// Why a public-key line was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicKeyError {
    /// Longer than the 16 KiB limit.
    TooLong,
    /// More than one line.
    MultipleLines,
    /// A control character (NUL, tab, carriage return, DEL, …).
    ControlCharacter,
    /// The type is not `ssh-ed25519` or `ssh-rsa`, so an `authorized_keys`
    /// option prefix such as `command="…"` is refused too.
    UnsupportedType,
    /// No key body after the type.
    MissingBody,
    /// The body is not strict base64.
    BadBase64,
    /// The type inside the key blob differs from the type field.
    TypeMismatch,
    /// The comment is not printable ASCII.
    BadComment,
}

/// Why a downstream `ssh` connection could not be made.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkFailure {
    /// `Connection refused`.
    Refused,
    /// `Connection timed out`.
    TimedOut,
    /// `No route to host` or `Network is unreachable`.
    Unreachable,
    /// `Could not resolve hostname`.
    UnknownHost,
    /// The peer closed or reset the connection during setup.
    ClosedByRemote,
}

/// The step of [`DEPLOY_SCRIPT`] that failed, from its exit code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployStep {
    /// Exit 10: `mkdir -p ~/.ssh`.
    CreateSshDir,
    /// Exit 11: creating `authorized_keys`.
    CreateAuthorizedKeys,
    /// Exit 12: no key arrived on stdin.
    ReadKey,
    /// Exit 13: appending the key.
    AppendKey,
}

/// Everything that can go wrong while setting up a key.
///
/// Carries no secret and no remote text: callers match on the variant and write
/// their own user-facing message. The `Display` text is English and meant for
/// logs.
#[derive(Debug, thiserror::Error)]
pub enum KeySetupError {
    /// `ssh-keygen` is not installed.
    #[error("ssh-keygen is not installed")]
    KeygenMissing,
    /// `ssh` is not installed.
    #[error("ssh is not installed")]
    SshMissing,
    /// The home directory could not be determined.
    #[error("no home directory")]
    NoHomeDir,
    /// The key directory is missing, unusable or not private.
    #[error("key directory {path} is not usable: {source}")]
    KeyDir {
        /// The directory.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },
    /// Refusing to overwrite an existing key file.
    #[error("key file {path} already exists")]
    KeyFileExists {
        /// The file that exists.
        path: PathBuf,
    },
    /// Every candidate file name in the directory is taken.
    #[error("no free key file name in {dir}")]
    NoFreeKeyName {
        /// The directory searched.
        dir: PathBuf,
    },
    /// A key file could not be read or removed.
    #[error("cannot read or remove key file {path}: {source}")]
    KeyFile {
        /// The file.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },
    /// A file chosen as an existing private key is too large, or not a
    /// recognised private-key format (a `.pub` file, for instance).
    #[error("{path} is not a private key")]
    NotAPrivateKey {
        /// The refused path.
        path: PathBuf,
    },
    /// A legacy PEM/PKCS#8 private key (a format the key-file inspector does
    /// not parse) has no `.pub` file next to it, so its public key cannot be
    /// derived without a passphrase.
    #[error("no public key next to {path}")]
    NoPublicKey {
        /// The private-key path with no matching `.pub`.
        path: PathBuf,
    },
    /// [`delete_generated_key`] was pointed at a file RustConn did not generate.
    #[error("refusing to delete {path}: not a RustConn-generated key")]
    RefusedToDelete {
        /// The refused path.
        path: PathBuf,
    },
    /// `ssh-keygen` exited unsuccessfully.
    #[error("ssh-keygen failed (exit code {code:?})")]
    KeygenFailed {
        /// Exit code, `None` when killed by a signal.
        code: Option<i32>,
    },
    /// A public-key line failed validation.
    #[error("invalid public key: {0:?}")]
    InvalidPublicKey(PublicKeyError),
    /// Host, user or port cannot be passed to `ssh` safely.
    #[error("invalid SSH target")]
    InvalidTarget,
    /// The askpass helper or its secret file could not be prepared.
    #[error("askpass helper could not be prepared")]
    AskpassSetup,
    /// A child process could not be started or waited on.
    #[error("failed to run {program}: {source}")]
    Spawn {
        /// `ssh` or `ssh-keygen`.
        program: &'static str,
        /// The underlying failure.
        #[source]
        source: io::Error,
    },
    /// The host key is not in `known_hosts`. The caller can offer to connect
    /// normally and accept it there.
    #[error("host key is not known")]
    HostKeyUnknown,
    /// The host key differs from the one in `known_hosts`.
    #[error("host key has changed")]
    HostKeyChanged,
    /// The server could not be reached.
    #[error("network failure: {0:?}")]
    Network(NetworkFailure),
    /// The server refused the credentials.
    #[error("authentication failed")]
    AuthFailed {
        /// Methods from `Permission denied (…)`, e.g. `publickey`, `password`.
        /// Empty when the server gave none (`Too many authentication failures`).
        server_methods: Vec<String>,
    },
    /// The account password has expired and must be changed first.
    #[error("password has expired")]
    PasswordExpired,
    /// The key passphrase was wrong.
    #[error("wrong key passphrase")]
    WrongPassphrase,
    /// `ssh` failed in a way none of the other variants describe.
    #[error("ssh failed (exit code {code:?})")]
    SshFailed {
        /// Exit code, `None` when killed by a signal.
        code: Option<i32>,
    },
    /// The server's shell does not understand the installer.
    #[error("server has no POSIX shell")]
    NoPosixShell,
    /// [`DEPLOY_SCRIPT`] failed at a known step.
    #[error("deploy script failed at {0:?}")]
    DeployScriptFailed(DeployStep),
    /// The login worked but the installer never ran (a `ForceCommand`, a
    /// restricted shell, a network appliance).
    #[error("server did not run the install command (exit code {code:?})")]
    NoSentinel {
        /// Exit code of `ssh`.
        code: Option<i32>,
    },
    /// The key is installed but the server did not accept it (`StrictModes`,
    /// `AuthorizedKeysFile`, SELinux, `PubkeyAuthentication no`).
    #[error("server did not accept the new key")]
    KeyNotAccepted,
    /// The run exceeded its budget and the child was killed.
    #[error("timed out")]
    Timeout,
    /// The run was cancelled and the child was killed.
    #[error("cancelled")]
    Cancelled,
}

// ---------------------------------------------------------------------------
// Location and naming
// ---------------------------------------------------------------------------

/// Where generated keys live, given the SSH directory.
///
/// Pure. Inside a snap (`snap_user_data` is `$SNAP_USER_DATA`) the revisioned
/// directory is replaced by `current`, so the path survives a refresh:
/// `~/snap/rustconn/current/.ssh/rustconn`. Otherwise `<ssh_dir>/rustconn`.
#[must_use]
pub fn key_setup_dir(ssh_dir: &Path, snap_user_data: Option<&Path>) -> PathBuf {
    match snap_user_data {
        Some(data) => match data.parent() {
            Some(snap_root) => snap_root
                .join("current")
                .join(".ssh")
                .join(KEY_SETUP_SUBDIR),
            None => data.join(".ssh").join(KEY_SETUP_SUBDIR),
        },
        None => ssh_dir.join(KEY_SETUP_SUBDIR),
    }
}

/// [`key_setup_dir`] for the running environment: the Flatpak SSH directory,
/// the snap's `$SNAP_USER_DATA`, or `~/.ssh`.
///
/// # Errors
///
/// Returns [`KeySetupError::NoHomeDir`] when no home directory can be found.
pub fn default_key_setup_dir() -> Result<PathBuf, KeySetupError> {
    if crate::snap::is_snap()
        && let Some(data) = std::env::var_os("SNAP_USER_DATA")
    {
        return Ok(key_setup_dir(Path::new(""), Some(Path::new(&data))));
    }
    let ssh_dir = match crate::flatpak::get_flatpak_ssh_dir() {
        Some(dir) => dir,
        None => dirs::home_dir()
            .ok_or(KeySetupError::NoHomeDir)?
            .join(".ssh"),
    };
    Ok(key_setup_dir(&ssh_dir, None))
}

/// Creates the key directory with mode 0700 and checks it is usable.
///
/// A symlink or a non-directory at `dir` is refused, an existing directory is
/// tightened to 0700, and a write probe (create, then remove, a scratch file)
/// proves the directory is writable before `ssh-keygen` is asked to use it.
///
/// # Errors
///
/// Returns [`KeySetupError::KeyDir`] describing the first thing that failed.
pub fn ensure_key_setup_dir(dir: &Path) -> Result<(), KeySetupError> {
    let fail = |source: io::Error| KeySetupError::KeyDir {
        path: dir.to_path_buf(),
        source,
    };
    match fs::symlink_metadata(dir) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(fail(io::Error::new(
                io::ErrorKind::InvalidInput,
                "is a symlink",
            )));
        }
        Ok(meta) if !meta.is_dir() => {
            return Err(fail(io::Error::new(
                io::ErrorKind::InvalidInput,
                "is not a directory",
            )));
        }
        Ok(_) => {}
        Err(e) if e.kind() == io::ErrorKind::NotFound => create_private_dir(dir).map_err(fail)?,
        Err(e) => return Err(fail(e)),
    }
    set_private_mode(dir).map_err(fail)?;

    let probe = dir.join(format!(".write-probe-{}", Uuid::new_v4().simple()));
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&probe)
        .map_err(fail)?;
    fs::remove_file(&probe).map_err(fail)
}

fn create_private_dir(dir: &Path) -> io::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

fn set_private_mode(dir: &Path) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        Ok(())
    }
}

/// A file-name-safe form of a connection name.
///
/// ASCII letters and digits are lowercased and kept; every other run of
/// characters (spaces, punctuation, all non-ASCII) becomes one `-`. The result
/// is at most 32 characters with no leading or trailing `-`, and falls back to
/// `connection` when nothing is left. Names are not transliterated.
#[must_use]
pub fn connection_slug(name: &str) -> String {
    let mut slug = String::new();
    let mut pending_separator = false;
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            if pending_separator && !slug.is_empty() {
                slug.push('-');
            }
            pending_separator = false;
            slug.push(c.to_ascii_lowercase());
        } else {
            pending_separator = true;
        }
    }
    slug.truncate(SLUG_MAX_LEN);
    let slug = slug.trim_end_matches('-');
    if slug.is_empty() {
        "connection".to_owned()
    } else {
        slug.to_owned()
    }
}

/// The private-key file name: `<slug>-<id8>_<type>`, e.g.
/// `prod-db-1-5fb5186b_ed25519`.
#[must_use]
pub fn key_file_stem(connection_name: &str, connection_id: Uuid, algo: KeyAlgorithm) -> String {
    let simple = connection_id.simple().to_string();
    format!(
        "{}-{}_{}",
        connection_slug(connection_name),
        &simple[..8],
        algo.file_suffix()
    )
}

/// What [`resolve_key_slot`] found for a connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeySlot {
    /// A free name: neither the private file nor the `.pub` exists.
    Fresh(PathBuf),
    /// A complete, consistent pair from an earlier run. A retry reuses it
    /// instead of generating another key.
    Reusable {
        /// The private key file.
        private_key_path: PathBuf,
        /// Its public half, as read from the `.pub` file.
        public_key: PublicKeyLine,
    },
}

/// Decides where this connection's key goes.
///
/// Tries `<dir>/<stem>`, then `<stem>-2`, `-3`, … A name where both files exist
/// and the `.pub` fingerprint equals the private key's is [`KeySlot::Reusable`].
/// A name that is taken in any other way (one file only, mismatched pair, a
/// symlink) is skipped. Nothing is ever overwritten.
///
/// # Errors
///
/// Returns [`KeySetupError::KeyDir`] when the directory cannot be inspected and
/// [`KeySetupError::NoFreeKeyName`] after 100 taken names.
pub fn resolve_key_slot(
    dir: &Path,
    connection_name: &str,
    connection_id: Uuid,
    algo: KeyAlgorithm,
) -> Result<KeySlot, KeySetupError> {
    let base = key_file_stem(connection_name, connection_id, algo);
    for attempt in 1..=MAX_SLOT_ATTEMPTS {
        let stem = if attempt == 1 {
            base.clone()
        } else {
            format!("{base}-{attempt}")
        };
        let private = dir.join(&stem);
        let public = dir.join(format!("{stem}.pub"));
        let private_taken = exists_at(&private, dir)?;
        let public_taken = exists_at(&public, dir)?;
        if !private_taken && !public_taken {
            return Ok(KeySlot::Fresh(private));
        }
        if private_taken
            && public_taken
            && let Some(public_key) = consistent_pair(&private, &public, algo)
        {
            return Ok(KeySlot::Reusable {
                private_key_path: private,
                public_key,
            });
        }
    }
    Err(KeySetupError::NoFreeKeyName {
        dir: dir.to_path_buf(),
    })
}

/// Whether anything (a dangling symlink included) is at `path`.
fn exists_at(path: &Path, dir: &Path) -> Result<bool, KeySetupError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(KeySetupError::KeyDir {
            path: dir.to_path_buf(),
            source,
        }),
    }
}

/// The public key of a private/public pair whose fingerprints agree.
fn consistent_pair(private: &Path, public: &Path, algo: KeyAlgorithm) -> Option<PublicKeyLine> {
    let public_key = PublicKeyLine::read(public).ok()?;
    if public_key.algorithm() != algo {
        return None;
    }
    let contents = Zeroizing::new(fs::read_to_string(private).ok()?);
    let from_private = PublicKeyInfo::from_private_key_file(&contents).ok()?;
    (from_private.fingerprint == public_key.fingerprint()).then_some(public_key)
}

/// The key comment: `rustconn@<local host> <YYYY-MM-DD>`.
///
/// The connection name is deliberately absent: the comment is copied to the
/// server's `authorized_keys`, which is not the place for a label the user
/// typed. The host name is reduced to `[A-Za-z0-9.-]`, cut to 64 characters,
/// and becomes `localhost` when nothing is left.
#[must_use]
pub fn key_comment(local_hostname: &str, date: chrono::NaiveDate) -> String {
    let mut host: String = local_hostname
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(64)
        .collect();
    if host.trim_matches('-').is_empty() {
        host = "localhost".to_owned();
    }
    format!("rustconn@{host} {}", date.format("%Y-%m-%d"))
}

/// [`key_comment`] for this machine and today.
#[must_use]
pub fn default_key_comment() -> String {
    let host = hostname::get()
        .ok()
        .and_then(|name| name.into_string().ok())
        .unwrap_or_default();
    key_comment(&host, chrono::Local::now().date_naive())
}

// ---------------------------------------------------------------------------
// Public key line
// ---------------------------------------------------------------------------

/// A validated one-line OpenSSH public key.
///
/// Only `ssh-ed25519` and `ssh-rsa` lines with a strict base64 body, a blob
/// whose inner type matches, and a printable-ASCII comment are accepted. That
/// makes the line safe to send to the server as-is: no newline, no option
/// prefix, nothing that `authorized_keys` would read as anything but a key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicKeyLine {
    line: String,
    algorithm: KeyAlgorithm,
    fingerprint: String,
}

impl PublicKeyLine {
    /// Validates `text`: one line, optionally ended by a single `\n`.
    ///
    /// # Errors
    ///
    /// Returns [`KeySetupError::InvalidPublicKey`] with the reason.
    pub fn parse(text: &str) -> Result<Self, KeySetupError> {
        parse_public_key(text).map_err(KeySetupError::InvalidPublicKey)
    }

    /// Reads and validates a `.pub` file.
    ///
    /// # Errors
    ///
    /// Returns [`KeySetupError::KeyFile`] when the file cannot be read and
    /// [`KeySetupError::InvalidPublicKey`] when its content is not a key.
    pub fn read(path: &Path) -> Result<Self, KeySetupError> {
        let key_file = |source| KeySetupError::KeyFile {
            path: path.to_path_buf(),
            source,
        };
        let size = fs::metadata(path).map_err(key_file)?.len();
        if size > MAX_PUBLIC_KEY_LEN as u64 {
            return Err(KeySetupError::InvalidPublicKey(PublicKeyError::TooLong));
        }
        let text = fs::read_to_string(path).map_err(key_file)?;
        Self::parse(&text)
    }

    /// The line, without a trailing newline.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.line
    }

    /// The key type.
    #[must_use]
    pub fn algorithm(&self) -> KeyAlgorithm {
        self.algorithm
    }

    /// The `SHA256:…` fingerprint, computed by the same code as the agent.
    #[must_use]
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
}

fn parse_public_key(text: &str) -> Result<PublicKeyLine, PublicKeyError> {
    if text.len() > MAX_PUBLIC_KEY_LEN {
        return Err(PublicKeyError::TooLong);
    }
    let line = text.strip_suffix('\n').unwrap_or(text);
    if line.contains('\n') {
        return Err(PublicKeyError::MultipleLines);
    }
    if line.chars().any(|c| c.is_ascii_control()) {
        return Err(PublicKeyError::ControlCharacter);
    }
    if line.is_empty() {
        return Err(PublicKeyError::MissingBody);
    }
    let mut fields = line.splitn(3, ' ');
    let key_type = fields.next().unwrap_or_default();
    let algorithm = match key_type {
        "ssh-ed25519" => KeyAlgorithm::Ed25519,
        "ssh-rsa" => KeyAlgorithm::Rsa4096,
        _ => return Err(PublicKeyError::UnsupportedType),
    };
    let body = fields
        .next()
        .filter(|body| !body.is_empty())
        .ok_or(PublicKeyError::MissingBody)?;
    if fields.next().is_some_and(|comment| !comment.is_ascii()) {
        return Err(PublicKeyError::BadComment);
    }
    let blob = data_encoding::BASE64
        .decode(body.as_bytes())
        .map_err(|_| PublicKeyError::BadBase64)?;
    if blob_type(&blob) != Some(key_type.as_bytes()) {
        return Err(PublicKeyError::TypeMismatch);
    }
    let info = PublicKeyInfo::from_pub_line(line).map_err(|_| PublicKeyError::BadBase64)?;
    Ok(PublicKeyLine {
        line: line.to_owned(),
        algorithm,
        fingerprint: info.fingerprint,
    })
}

/// The leading SSH string (the key type) of a public-key blob.
fn blob_type(blob: &[u8]) -> Option<&[u8]> {
    let length = u32::from_be_bytes(blob.get(..4)?.try_into().ok()?);
    let end = 4usize.checked_add(usize::try_from(length).ok()?)?;
    blob.get(4..end)
}

// ---------------------------------------------------------------------------
// Key generation
// ---------------------------------------------------------------------------

/// What to generate.
pub struct KeygenRequest {
    /// Where the private key goes; the public key is `<path>.pub`.
    pub private_key_path: PathBuf,
    /// Key type.
    pub algorithm: KeyAlgorithm,
    /// Key comment; see [`key_comment`].
    pub comment: String,
    /// Passphrase for the private key. `None` (the default) means none. It is
    /// handed to `ssh-keygen` through the askpass path and goes nowhere else.
    pub passphrase: Option<SecretString>,
}

impl std::fmt::Debug for KeygenRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KeygenRequest")
            .field("private_key_path", &self.private_key_path)
            .field("algorithm", &self.algorithm)
            .field("comment", &self.comment)
            .field(
                "passphrase",
                &self.passphrase.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// A key pair [`generate_key`] produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedKey {
    /// The private key file.
    pub private_key_path: PathBuf,
    /// The public key file.
    pub public_key_path: PathBuf,
    /// The public key as read back from disk.
    pub public_key: PublicKeyLine,
}

/// The `ssh-keygen` argument vector.
///
/// `-N ""` is added only when there is no passphrase; with one, the passphrase
/// is asked for through askpass and **never** appears in argv.
#[must_use]
pub fn keygen_args(
    path: &Path,
    algo: KeyAlgorithm,
    comment: &str,
    with_passphrase: bool,
) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec!["-q".into()];
    args.extend(
        algo.keygen_type_args()
            .iter()
            .map(|arg| OsString::from(*arg)),
    );
    args.extend(["-C".into(), comment.into(), "-f".into(), path.into()]);
    if !with_passphrase {
        args.extend(["-N".into(), OsString::new()]);
    }
    args
}

/// Generates a key pair with `ssh-keygen`.
///
/// The caller prepares the directory ([`ensure_key_setup_dir`]) and picks the
/// path ([`resolve_key_slot`]). Neither `<path>` nor `<path>.pub` may exist. If
/// the run fails, times out or is cancelled, files it may have created are
/// removed. A passphrase goes through a helper that repeats its answer, because
/// `ssh-keygen` asks twice (the passphrase, then the confirmation); the helper
/// and its secret file are deleted on return.
///
/// # Errors
///
/// [`KeySetupError::KeygenMissing`], [`KeySetupError::KeyFileExists`],
/// [`KeySetupError::AskpassSetup`], [`KeySetupError::Spawn`],
/// [`KeySetupError::Timeout`], [`KeySetupError::Cancelled`],
/// [`KeySetupError::KeygenFailed`] and [`KeySetupError::InvalidPublicKey`].
pub fn generate_key(
    request: &KeygenRequest,
    control: &RunControl,
) -> Result<GeneratedKey, KeySetupError> {
    let program = crate::which::find_in_path("ssh-keygen").ok_or(KeySetupError::KeygenMissing)?;
    generate_key_with(&program, request, control)
}

fn generate_key_with(
    program: &Path,
    request: &KeygenRequest,
    control: &RunControl,
) -> Result<GeneratedKey, KeySetupError> {
    let private = request.private_key_path.clone();
    let public = public_path(&private);
    for path in [&private, &public] {
        if fs::symlink_metadata(path).is_ok() {
            return Err(KeySetupError::KeyFileExists { path: path.clone() });
        }
    }
    if !program.exists() {
        return Err(KeySetupError::KeygenMissing);
    }

    let mut files = SecretFiles::default();
    let mut env = Vec::new();
    if let Some(passphrase) = &request.passphrase {
        let script = ssh_tunnel::create_tunnel_askpass_script().map_err(|e| askpass_failed(&e))?;
        files.track(script.clone());
        let secret =
            ssh_tunnel::create_tunnel_secret_file(passphrase).map_err(|e| askpass_failed(&e))?;
        files.track(secret.clone());
        push_passphrase_env(&mut env, &script, &secret);
    }

    let args = keygen_args(
        &private,
        request.algorithm,
        &request.comment,
        request.passphrase.is_some(),
    );
    let finished = run_child(program, "ssh-keygen", &args, None, &env, control);
    let generated = match finished {
        Ok(done) if done.code == Some(0) => PublicKeyLine::read(&public),
        Ok(done) => Err(KeySetupError::KeygenFailed { code: done.code }),
        Err(error) => Err(error),
    };
    match generated {
        Ok(public_key) => Ok(GeneratedKey {
            private_key_path: private,
            public_key_path: public,
            public_key,
        }),
        Err(error) => {
            // Neither file existed before this run, so whatever is there is ours.
            for path in [&private, &public] {
                let _ = fs::remove_file(path);
            }
            Err(error)
        }
    }
}

fn public_path(private: &Path) -> PathBuf {
    let mut name = private.as_os_str().to_owned();
    name.push(".pub");
    PathBuf::from(name)
}

// ---------------------------------------------------------------------------
// Existing key (K5: "Use an Existing Key")
// ---------------------------------------------------------------------------

/// Longest private-key file [`existing_key`] will read, in bytes. A real key
/// file is a few KiB; this is generous headroom while still refusing to read
/// an arbitrary large file that was not meant to be a key.
const MAX_PRIVATE_KEY_LEN: u64 = 64 * 1024;

/// A private key chosen for "Use an Existing Key", with its public half
/// resolved and its encryption known — everything [`run_deploy`] and
/// [`run_verify`] need, without ever touching a passphrase.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExistingKey {
    /// The chosen private-key file, unchanged from what was passed in.
    pub private_key_path: PathBuf,
    /// The public key: from a matching `.pub` file when one exists and agrees
    /// with the private key's fingerprint, otherwise derived from the private
    /// key itself (OpenSSH only).
    pub public_key: PublicKeyLine,
    /// Whether the private key needs a passphrase to use.
    pub encryption: KeyEncryption,
}

/// Reads a private key the user picked and resolves its public half, without
/// ever asking for or using a passphrase.
///
/// An OpenSSH private key (even an encrypted one) carries its public key in
/// the clear, so the public line can always be rebuilt from it
/// ([`ssh_agent::openssh_public_key_line`]). A `.pub` file next to the private
/// key is preferred when it parses and its fingerprint agrees with the
/// private key — it keeps the comment the user's tooling wrote; an
/// unparseable or mismatched `.pub` is silently ignored in favour of the
/// derived line. A legacy PEM or PKCS#8 key is not parsed by this module (see
/// the `ssh_agent` key-file inspector), so it needs a valid `.pub` file; its
/// fingerprint is not cross-checked against the private key here, so a
/// `.pub` for a different key is only caught later, by [`run_verify`].
///
// ponytail: a legacy PEM/PKCS#8 key with no `.pub` is not supported (needs
// `ssh-keygen -y`, which needs the passphrase through askpass); upgrade by
// running it through the askpass path the way `generate_key` already does
// for `ssh-keygen`. ECDSA and `…-sk` keys are rejected by `PublicKeyLine`
// (`InvalidPublicKey(UnsupportedType)`) for the same reason `KeyAlgorithm`
// does not offer them for generation.
///
/// # Errors
///
/// - [`KeySetupError::KeyFile`] if the file cannot be read.
/// - [`KeySetupError::NotAPrivateKey`] if the file is larger than 64 KiB, is a
///   `.pub` file, or is not a recognised private-key format.
/// - [`KeySetupError::NoPublicKey`] if the format is legacy PEM/PKCS#8 and no
///   valid `.pub` file sits next to it.
/// - [`KeySetupError::InvalidPublicKey`] if the resolved public key is not
///   `ssh-ed25519` or `ssh-rsa`.
pub fn existing_key(private_key_path: &Path) -> Result<ExistingKey, KeySetupError> {
    let not_a_key = || KeySetupError::NotAPrivateKey {
        path: private_key_path.to_path_buf(),
    };
    let key_file = |source| KeySetupError::KeyFile {
        path: private_key_path.to_path_buf(),
        source,
    };
    let size = fs::metadata(private_key_path).map_err(key_file)?.len();
    if size > MAX_PRIVATE_KEY_LEN {
        return Err(not_a_key());
    }
    let contents = Zeroizing::new(fs::read_to_string(private_key_path).map_err(key_file)?);
    let info = ssh_agent::inspect_key_file(&contents).map_err(|_| not_a_key())?;

    let matching_pub = PublicKeyLine::read(&public_path(private_key_path)).ok();
    let public_key = match info.format {
        KeyFormat::OpenSsh => {
            let derived_fingerprint = info.public_key.as_ref().map(|pk| pk.fingerprint.as_str());
            match matching_pub {
                Some(pub_key) if Some(pub_key.fingerprint()) == derived_fingerprint => pub_key,
                _ => {
                    let line =
                        ssh_agent::openssh_public_key_line(&contents).map_err(|_| not_a_key())?;
                    PublicKeyLine::parse(&line)?
                }
            }
        }
        KeyFormat::LegacyPem | KeyFormat::Pkcs8 => {
            matching_pub.ok_or_else(|| KeySetupError::NoPublicKey {
                path: private_key_path.to_path_buf(),
            })?
        }
        KeyFormat::Unknown => return Err(not_a_key()),
    };

    Ok(ExistingKey {
        private_key_path: private_key_path.to_path_buf(),
        public_key,
        encryption: info.encryption,
    })
}

/// [`crate::ssh_agent::list_key_files_in`] over each of `dirs`, deduplicated
/// while keeping the order they were first seen in.
///
/// A directory that cannot be scanned is logged and skipped, not
/// propagated, so one bad directory does not empty the whole list.
#[must_use]
pub fn existing_key_candidates_in(dirs: &[PathBuf]) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    let mut candidates = Vec::new();
    for dir in dirs {
        match crate::ssh_agent::list_key_files_in(dir) {
            Ok(files) => {
                for file in files {
                    if seen.insert(file.clone()) {
                        candidates.push(file);
                    }
                }
            }
            Err(error) => {
                tracing::debug!(
                    dir = %dir.display(),
                    %error,
                    "key setup: could not scan directory for existing keys",
                );
            }
        }
    }
    candidates
}

/// [`existing_key_candidates_in`] over likely key directories: `~/.ssh`, the
/// Flatpak SSH directory when sandboxed, and [`default_key_setup_dir`].
///
/// The last of those is RustConn's own `rustconn/` subdirectory, which also
/// holds keys generated by an earlier run that a later run may want to reuse
/// as "existing".
#[must_use]
pub fn existing_key_candidates() -> Vec<PathBuf> {
    let mut scan_dirs = Vec::new();
    if let Some(home) = dirs::home_dir() {
        scan_dirs.push(home.join(".ssh"));
    }
    if let Some(flatpak_ssh_dir) = crate::flatpak::get_flatpak_ssh_dir() {
        scan_dirs.push(flatpak_ssh_dir);
    }
    if let Ok(dir) = default_key_setup_dir() {
        scan_dirs.push(dir);
    }
    existing_key_candidates_in(&scan_dirs)
}

/// Deletes a key pair this module generated ("Delete Generated Key").
///
/// Removes `<path>` and `<path>.pub`, but only when the parent directory is
/// named `rustconn` and the file name has the shape [`key_file_stem`] produces
/// (`…-<8 hex>_ed25519` or `…_rsa`, optionally followed by `-<n>`). Anything
/// else is refused, so a key the user brought is never removed, even through a
/// caller's mistake. A file that is already gone is not an error.
///
/// # Errors
///
/// [`KeySetupError::RefusedToDelete`] for a foreign path;
/// [`KeySetupError::KeyFile`] when a removal fails.
pub fn delete_generated_key(private_key_path: &Path) -> Result<(), KeySetupError> {
    let in_key_dir = private_key_path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == KEY_SETUP_SUBDIR);
    let generated_name = private_key_path
        .file_name()
        .and_then(OsStr::to_str)
        .is_some_and(is_generated_key_name);
    if !in_key_dir || !generated_name {
        return Err(KeySetupError::RefusedToDelete {
            path: private_key_path.to_path_buf(),
        });
    }
    for path in [
        private_key_path.to_path_buf(),
        public_path(private_key_path),
    ] {
        match fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(KeySetupError::KeyFile { path, source }),
        }
    }
    Ok(())
}

/// Whether `name` looks like `<slug>-<8 hex>_<type>` with an optional `-<n>`.
fn is_generated_key_name(name: &str) -> bool {
    let name = match name.rsplit_once('-') {
        Some((head, tail)) if !tail.is_empty() && tail.bytes().all(|b| b.is_ascii_digit()) => head,
        _ => name,
    };
    let Some(head) = [KeyAlgorithm::Ed25519, KeyAlgorithm::Rsa4096]
        .iter()
        .find_map(|algo| name.strip_suffix(&format!("_{}", algo.file_suffix())))
    else {
        return false;
    };
    match head.rsplit_once('-') {
        Some((slug, id8)) => {
            !slug.is_empty() && id8.len() == 8 && id8.bytes().all(|b| b.is_ascii_hexdigit())
        }
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Target and argument vectors
// ---------------------------------------------------------------------------

/// The server a key is set up on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SshTarget {
    /// Host name or address.
    pub host: String,
    /// TCP port.
    pub port: u16,
    /// Login name; `None` lets `ssh` decide.
    pub username: Option<String>,
    /// The connection's own `ssh` arguments (`-J`, `ProxyCommand`, custom
    /// options, …), as the GUI builds them for a session: without `-p`, the
    /// identity file and the destination. They are filtered before use; see
    /// [`control_channel_args`].
    pub session_args: Vec<String>,
    /// Flatpak `known_hosts` file, passed as `UserKnownHostsFile` unless the
    /// session arguments already name one.
    pub known_hosts_file: Option<PathBuf>,
}

impl SshTarget {
    /// A target whose `known_hosts_file` is the Flatpak one when sandboxed.
    #[must_use]
    pub fn new(
        host: String,
        port: u16,
        username: Option<String>,
        session_args: Vec<String>,
    ) -> Self {
        Self {
            host,
            port,
            username,
            session_args,
            known_hosts_file: crate::flatpak::get_flatpak_known_hosts_path(),
        }
    }
}

/// The remote command that runs [`DEPLOY_SCRIPT`]: `exec sh -c '<script>'`.
#[must_use]
pub fn deploy_remote_command() -> String {
    format!("exec sh -c '{DEPLOY_SCRIPT}'")
}

/// The option prefix followed by the cleaned session arguments.
///
/// The prefix ([`OVERRIDE_OPTIONS`]: `StrictHostKeyChecking=yes`, no
/// multiplexing, no forwarding, no tty, no remote command, a single password
/// prompt) comes **first**, and OpenSSH keeps the first value of every option,
/// so the connection's `custom_options` cannot override it. As a second line of
/// defence the same options are removed from the session arguments, along with
/// forwarding and tty flags, `-p` and `-l` (the tail sets those from the
/// [`SshTarget`]) and options that would change the target. `-J`, `ProxyCommand`,
/// `-F` and every other option pass through unchanged and in order.
#[must_use]
pub fn control_channel_args(session_args: &[String]) -> Vec<String> {
    channel_args(session_args, false)
}

fn channel_args(session_args: &[String], strip_identity: bool) -> Vec<String> {
    let mut args = Vec::with_capacity(OVERRIDE_OPTIONS.len() * 2 + session_args.len());
    for option in OVERRIDE_OPTIONS {
        args.push("-o".to_owned());
        args.push(option.to_owned());
    }
    args.extend(strip_session_args(session_args, strip_identity));
    args
}

/// Removes everything from `args` that must not reach a control-channel `ssh`.
fn strip_session_args(args: &[String], strip_identity: bool) -> Vec<String> {
    let mut kept = Vec::with_capacity(args.len());
    let mut pending: VecDeque<String> = args.iter().cloned().collect();
    while let Some(token) = pending.pop_front() {
        let Some(flag) = short_flag(&token) else {
            kept.push(token);
            continue;
        };
        // `-vA` is `-v -A` to OpenSSH, and `-A` would beat the `-o` prefix, so
        // a cluster is judged flag by flag, never by its first letter.
        if let Some(split) = split_cluster(&token) {
            for part in split.into_iter().rev() {
                pending.push_front(part);
            }
            continue;
        }
        if VALUE_FLAGS.contains(flag) {
            let attached = &token[2..];
            let (value, used) = if attached.is_empty() {
                (pending.pop_front(), 2)
            } else {
                (Some(attached.to_owned()), 1)
            };
            let Some(value) = value else {
                // A value flag with nothing after it would swallow whatever
                // follows it in the final argv; drop it.
                continue;
            };
            let dropped = match flag {
                'o' => is_stripped_option(&value, strip_identity),
                'i' => strip_identity,
                'p' | 'l' => true,
                other => STRIPPED_VALUE_FLAGS.contains(other),
            };
            if !dropped {
                kept.push(token);
                if used == 2 {
                    kept.push(value);
                }
            }
        } else if !token[1..].chars().all(|c| STRIPPED_FLAGS.contains(c)) {
            kept.push(token);
        }
    }
    kept
}

/// Splits a cluster of short flags (`-vA`, `-vo`) into one token per flag:
/// `["-v", "-A"]`, `["-v", "-o"]`. A value flag ends the cluster and keeps the
/// rest of the token as its attached value (`-vL80:h:1` → `["-v", "-L80:h:1"]`).
/// `None` for a token that is not a cluster (a single flag, with or without an
/// attached value).
fn split_cluster(token: &str) -> Option<Vec<String>> {
    let flag = short_flag(token)?;
    if VALUE_FLAGS.contains(flag) || token.len() <= 2 {
        return None;
    }
    let mut parts = Vec::new();
    for (index, c) in token.char_indices().skip(1) {
        if VALUE_FLAGS.contains(c) {
            parts.push(format!("-{}", &token[index..]));
            return Some(parts);
        }
        parts.push(format!("-{c}"));
    }
    Some(parts)
}

/// The flag letter of a short option token (`-o`, `-L8080:h:80`), if it is one.
fn short_flag(token: &str) -> Option<char> {
    let rest = token.strip_prefix('-')?;
    rest.chars().next().filter(char::is_ascii_alphabetic)
}

/// The key of an `-o` value (`K=V`, `K V`, `K = V`).
fn option_key(value: &str) -> &str {
    value
        .trim_start()
        .split(|c: char| c == '=' || c.is_whitespace())
        .next()
        .unwrap_or_default()
}

fn is_stripped_option(value: &str, strip_identity: bool) -> bool {
    let key = option_key(value);
    let named = |set: &[&str]| set.iter().any(|k| k.eq_ignore_ascii_case(key));
    OVERRIDE_OPTIONS.iter().any(|o| {
        o.split('=')
            .next()
            .is_some_and(|k| k.eq_ignore_ascii_case(key))
    }) || named(&STRIPPED_OPTION_KEYS)
        || (strip_identity && named(&IDENTITY_OPTION_KEYS))
}

/// Whether `args` already carries `-o <key>…`.
fn declares_option(args: &[String], key: &str) -> bool {
    args.iter().enumerate().any(|(i, token)| {
        let value = if token == "-o" {
            args.get(i + 1).map(String::as_str)
        } else {
            token.strip_prefix("-o").filter(|v| !v.is_empty())
        };
        value.is_some_and(|v| option_key(v).eq_ignore_ascii_case(key))
    })
}

fn validate_target(target: &SshTarget) -> Result<(), KeySetupError> {
    let bad_word = |s: &str| {
        s.is_empty()
            || s.starts_with('-')
            || s.contains('@')
            || s.chars().any(|c| c.is_whitespace() || c.is_control())
    };
    // A host can reach a `ProxyCommand` through `%h`, where older OpenSSH
    // hands it to a shell unquoted, so shell metacharacters never belong in it.
    let shell_meta = |c: char| "\"'`$\\;&|<>(){}*?!#".contains(c);
    if bad_word(&target.host) || target.host.chars().any(shell_meta) || target.port == 0 {
        return Err(KeySetupError::InvalidTarget);
    }
    if target.username.as_deref().is_some_and(bad_word) {
        return Err(KeySetupError::InvalidTarget);
    }
    Ok(())
}

/// `-p`, `UserKnownHostsFile` and the destination.
fn push_tail(args: &mut Vec<String>, target: &SshTarget) -> Result<(), KeySetupError> {
    args.push("-p".to_owned());
    args.push(target.port.to_string());
    if let Some(path) = &target.known_hosts_file
        && !declares_option(&target.session_args, "UserKnownHostsFile")
    {
        let value =
            crate::ssh_tunnel::user_known_hosts_option(path).ok_or(KeySetupError::InvalidTarget)?;
        args.push("-o".to_owned());
        args.push(value);
    }
    args.push(match &target.username {
        Some(user) => format!("{user}@{}", target.host),
        None => target.host.clone(),
    });
    Ok(())
}

/// The full `ssh` argument vector (without the program name) for the deploy.
///
/// [`control_channel_args`], then `BatchMode=yes` when there is no password to
/// deliver (so `ssh` can never open `/dev/tty`), then the tail and
/// [`deploy_remote_command`].
///
/// # Errors
///
/// Returns [`KeySetupError::InvalidTarget`] for an empty host, a host or user
/// that starts with `-` or contains whitespace, control characters or `@`, a
/// host with a shell metacharacter (`` ` `` `$` `;` `&` `|` quotes, brackets
/// and the like), or port 0.
pub fn deploy_args(target: &SshTarget, has_password: bool) -> Result<Vec<String>, KeySetupError> {
    remote_command_args(target, has_password, &deploy_remote_command())
}

/// The `ssh` arguments that run `remote_command` over the control channel.
///
/// `BatchMode=yes` is added unless a password has to be answered.
///
/// # Errors
///
/// Returns [`KeySetupError::InvalidTarget`] as [`deploy_args`] does.
pub fn remote_command_args(
    target: &SshTarget,
    has_password: bool,
    remote_command: &str,
) -> Result<Vec<String>, KeySetupError> {
    validate_target(target)?;
    let mut args = control_channel_args(&target.session_args);
    if !has_password {
        args.push("-o".to_owned());
        args.push("BatchMode=yes".to_owned());
    }
    push_tail(&mut args, target)?;
    args.push(remote_command.to_owned());
    Ok(args)
}

/// The full `ssh` argument vector (without the program name) for verification.
///
/// Same prefix as the deploy, then public-key authentication with the new key
/// alone: `PreferredAuthentications=publickey`, password and
/// keyboard-interactive off, `IdentitiesOnly=yes`, `IdentityAgent=none`, `-v`
/// (so the server's acceptance is printed) and exactly one `-i`. The
/// connection's own identity options are removed from the session arguments.
/// `BatchMode=yes` is added unless a passphrase has to be answered. The remote
/// command is `exit 0`.
///
/// # Errors
///
/// Returns [`KeySetupError::InvalidTarget`] as [`deploy_args`] does, and also
/// for a key path that is not valid UTF-8.
pub fn verify_args(
    target: &SshTarget,
    private_key_path: &Path,
    has_passphrase: bool,
) -> Result<Vec<String>, KeySetupError> {
    validate_target(target)?;
    let key_path = private_key_path
        .to_str()
        .ok_or(KeySetupError::InvalidTarget)?;
    let mut args = channel_args(&target.session_args, true);
    for option in [
        "PreferredAuthentications=publickey",
        "PubkeyAuthentication=yes",
        "PasswordAuthentication=no",
        "KbdInteractiveAuthentication=no",
        "IdentitiesOnly=yes",
        "IdentityAgent=none",
    ] {
        args.push("-o".to_owned());
        args.push(option.to_owned());
    }
    if !has_passphrase {
        args.push("-o".to_owned());
        args.push("BatchMode=yes".to_owned());
    }
    args.push("-v".to_owned());
    args.push("-i".to_owned());
    args.push(key_path.to_owned());
    push_tail(&mut args, target)?;
    args.push("exit 0".to_owned());
    Ok(args)
}

// ---------------------------------------------------------------------------
// Classification
// ---------------------------------------------------------------------------

/// What a deploy achieved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeployOutcome {
    /// The key was appended.
    Installed,
    /// The key was already there; nothing changed.
    AlreadyPresent,
}

/// What verification found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerifyOutcome {
    /// The server named the new key's fingerprint in `Server accepts key:` and
    /// the login completed.
    Accepted,
    /// The server accepted the key but wants one more factor
    /// (`partial success`). The caller decides what to do.
    AcceptedSecondFactorRequired,
}

/// Interprets a finished deploy `ssh`.
///
/// Success needs exit 0 and a stdout line that is exactly one of the sentinels,
/// so a banner or an echo that merely contains the word does not count. Exit 0
/// without a sentinel means the server swallowed the command.
///
/// # Errors
///
/// The [`KeySetupError`] variant that matches the failure; see the module docs.
pub fn classify_deploy(
    exit_code: Option<i32>,
    stdout: &str,
    stderr: &str,
) -> Result<DeployOutcome, KeySetupError> {
    let Some(code) = exit_code else {
        return Err(KeySetupError::SshFailed { code: None });
    };
    if code == 0 {
        for line in stdout.lines().map(str::trim_end) {
            if line == SENTINEL_INSTALLED {
                return Ok(DeployOutcome::Installed);
            }
            if line == SENTINEL_PRESENT {
                return Ok(DeployOutcome::AlreadyPresent);
            }
        }
    }
    let visible = without_debug(stderr);
    if code == 255 {
        return Err(
            classify_ssh_failure(&visible).unwrap_or(KeySetupError::SshFailed { code: Some(255) })
        );
    }
    if visible.contains("is not recognized as an internal or external command")
        || visible.contains("The term 'exec' is not recognized")
    {
        return Err(KeySetupError::NoPosixShell);
    }
    let step = match code {
        10 => Some(DeployStep::CreateSshDir),
        11 => Some(DeployStep::CreateAuthorizedKeys),
        12 => Some(DeployStep::ReadKey),
        13 => Some(DeployStep::AppendKey),
        _ => None,
    };
    Err(match step {
        Some(step) => KeySetupError::DeployScriptFailed(step),
        None => KeySetupError::NoSentinel { code: Some(code) },
    })
}

/// Interprets a finished verification `ssh -v`.
///
/// Success needs a `Server accepts key:` line carrying `expected_fingerprint` as
/// a whole word. A login by some other key (an `IdentityFile` from
/// `~/.ssh/config`, say) is a failure however cleanly it ended.
///
/// The `partial success`, `Authenticated to` and askpass markers are read only
/// from that accept line onward: behind `-J` the bastion's own `-v` output sits
/// in the same stream, ahead of it.
///
/// # Errors
///
/// The [`KeySetupError`] variant that matches the failure;
/// [`KeySetupError::KeyNotAccepted`] covers every "logged in, but not with this
/// key" case.
pub fn classify_verify(
    exit_code: Option<i32>,
    stderr: &str,
    expected_fingerprint: &str,
) -> Result<VerifyOutcome, KeySetupError> {
    // ponytail: OpenSSH before 8.x prints `Server accepts key:` without the
    // fingerprint, so verification reports KeyNotAccepted there. A false
    // negative, hence safe; upgrade by matching the `-i` path on the same line.
    if let Some(tail) = after_accept_line(stderr, expected_fingerprint) {
        if tail.contains("partial success") {
            return Ok(VerifyOutcome::AcceptedSecondFactorRequired);
        }
        if exit_code.is_some_and(|code| code != 255) {
            return Ok(VerifyOutcome::Accepted);
        }
        // The server wants this key and `ssh` then asked for its passphrase
        // (one prompt allowed) but never got to sign: the answer was wrong.
        // OpenSSH 10 prints no "incorrect passphrase" line at `-v` for this,
        // only the askpass request, which a live test against a real server
        // showed; older releases do print it and are caught further down. A
        // login that completed (`Authenticated to`) and died later is not this.
        if exit_code == Some(255)
            && tail.contains("read_passphrase: requested to askpass")
            && !tail.contains("Authenticated to")
        {
            return Err(KeySetupError::WrongPassphrase);
        }
    }
    match exit_code {
        None => Err(KeySetupError::SshFailed { code: None }),
        Some(255) => Err(match classify_ssh_failure(&without_debug(stderr)) {
            Some(KeySetupError::AuthFailed { .. }) | None => KeySetupError::KeyNotAccepted,
            Some(other) => other,
        }),
        Some(_) => Err(KeySetupError::KeyNotAccepted),
    }
}

/// The part of `stderr` that starts at the first `Server accepts key:` line
/// naming `expected_fingerprint` as a whole word, or `None` if there is none.
///
/// Behind a `-J` jump host the bastion's `ssh` inherits `-v` and writes into the
/// same stream, so its `Authenticated to` and `partial success` lines are in
/// `stderr` too. The bastion signs in before the transport to the target starts,
/// so those lines always come before the target's accept line. Judging markers
/// only from here on keeps them out of the verdict.
fn after_accept_line<'a>(stderr: &'a str, expected_fingerprint: &str) -> Option<&'a str> {
    let mut offset = 0;
    for line in stderr.split_inclusive('\n') {
        let text = line.trim_end_matches(['\n', '\r']);
        if text.starts_with("debug1: Server accepts key:")
            && text
                .split_whitespace()
                .any(|word| word == expected_fingerprint)
        {
            return Some(&stderr[offset..]);
        }
        offset += line.len();
    }
    None
}

/// `stderr` without `debug1:`-style lines.
///
/// With `-v`, those lines narrate things that are not failures: a refused
/// connect to the first of several addresses, or a skipped identity.
fn without_debug(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|line| !line.starts_with("debug"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The failures every `ssh` run can share, recognised in a fixed order: a
/// changed host key before an unknown one (a changed key also prints "Host key
/// verification failed"), then the network, an expired password, a bad
/// passphrase, and last a plain "Permission denied".
fn classify_ssh_failure(stderr: &str) -> Option<KeySetupError> {
    let any_line = |test: &dyn Fn(&str) -> bool| stderr.lines().any(test);
    if stderr.contains("REMOTE HOST IDENTIFICATION HAS CHANGED")
        || any_line(&|l| l.contains("Host key for ") && l.contains(" has changed"))
    {
        return Some(KeySetupError::HostKeyChanged);
    }
    if stderr.contains("Host key verification failed")
        || any_line(&|l| l.contains("host key is known for"))
    {
        return Some(KeySetupError::HostKeyUnknown);
    }
    let network = [
        ("Could not resolve hostname", NetworkFailure::UnknownHost),
        ("Connection refused", NetworkFailure::Refused),
        ("Connection timed out", NetworkFailure::TimedOut),
        ("Operation timed out", NetworkFailure::TimedOut),
        ("No route to host", NetworkFailure::Unreachable),
        ("Network is unreachable", NetworkFailure::Unreachable),
        ("Connection closed by", NetworkFailure::ClosedByRemote),
        (
            "kex_exchange_identification",
            NetworkFailure::ClosedByRemote,
        ),
        ("Connection reset by", NetworkFailure::ClosedByRemote),
    ];
    if let Some((_, kind)) = network.iter().find(|(text, _)| stderr.contains(text)) {
        return Some(KeySetupError::Network(*kind));
    }
    let lower = stderr.to_ascii_lowercase();
    if [
        "you are required to change your password",
        "password has expired",
        "password change required",
    ]
    .iter()
    .any(|text| lower.contains(text))
    {
        return Some(KeySetupError::PasswordExpired);
    }
    if lower.contains("incorrect passphrase supplied") || lower.contains("bad passphrase") {
        return Some(KeySetupError::WrongPassphrase);
    }
    let denied = stderr.lines().find_map(|l| {
        let (_, rest) = l.split_once("Permission denied (")?;
        rest.split_once(')').map(|(methods, _)| methods.to_owned())
    });
    let Some(denied) = denied else {
        // The server cut the login off after too many offered keys (agent and
        // default identities come before the one password prompt): no
        // `Permission denied` line, no method list, but an auth failure all the
        // same.
        return stderr
            .contains("Too many authentication failures")
            .then(|| KeySetupError::AuthFailed {
                server_methods: Vec::new(),
            });
    };
    Some(KeySetupError::AuthFailed {
        server_methods: denied
            .split(',')
            .map(|method| method.trim().to_owned())
            .filter(|method| !method.is_empty())
            .collect(),
    })
}

// ---------------------------------------------------------------------------
// Running
// ---------------------------------------------------------------------------

/// Budget and cancellation for one run.
#[derive(Debug, Clone)]
pub struct RunControl {
    /// How long the child may run before it is killed.
    pub timeout: Duration,
    /// Set to `true` from another thread to kill the child and return
    /// [`KeySetupError::Cancelled`].
    pub cancel: Arc<AtomicBool>,
}

impl Default for RunControl {
    fn default() -> Self {
        Self {
            timeout: DEFAULT_TIMEOUT,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Credentials for the deploy connection.
#[derive(Default)]
pub struct DeployCredentials {
    /// The account password. `None` runs the deploy with `BatchMode=yes`.
    pub password: Option<SecretString>,
    /// Passwords for bastions in the jump chain, by hop.
    pub jump_host_passwords: Vec<(AskpassSlot, SecretString)>,
}

impl std::fmt::Debug for DeployCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DeployCredentials")
            .field("password", &self.password.as_ref().map(|_| "[REDACTED]"))
            .field("jump_host_passwords", &self.jump_host_passwords.len())
            .finish()
    }
}

/// Installs `key` on the server and reports whether it was new.
///
/// Spawns `ssh` with [`deploy_args`], writes the key and a newline to its stdin,
/// and classifies the result with [`classify_deploy`]. A password is delivered
/// through the K1 askpass helper (an owner-only file named by the environment);
/// the files are removed before this returns. If the password cannot be staged
/// the connection is not attempted without it.
///
/// # Errors
///
/// [`KeySetupError::SshMissing`], [`KeySetupError::InvalidTarget`],
/// [`KeySetupError::AskpassSetup`], [`KeySetupError::Spawn`],
/// [`KeySetupError::Timeout`], [`KeySetupError::Cancelled`] and every variant
/// [`classify_deploy`] can return.
pub fn run_deploy(
    target: &SshTarget,
    key: &PublicKeyLine,
    credentials: &DeployCredentials,
    control: &RunControl,
) -> Result<DeployOutcome, KeySetupError> {
    let program = crate::which::find_in_path("ssh").ok_or(KeySetupError::SshMissing)?;
    run_deploy_with(&program, target, key, credentials, control)
}

fn run_deploy_with(
    program: &Path,
    target: &SshTarget,
    key: &PublicKeyLine,
    credentials: &DeployCredentials,
    control: &RunControl,
) -> Result<DeployOutcome, KeySetupError> {
    let args = deploy_args(target, credentials.password.is_some())?;
    let mut files = SecretFiles::default();
    let mut env = stage_password_env(
        &credentials.jump_host_passwords,
        credentials.password.as_ref(),
        &mut files,
    )?;
    ensure_askpass_display(&mut env);
    let done = run_child(program, "ssh", &args, Some(key.as_str()), &env, control)?;
    log_stream("ssh deploy stderr", &done.stderr);
    classify_deploy(done.code, &done.stdout, &done.stderr)
}

/// Proves the server accepts the key at `private_key_path`.
///
/// Spawns `ssh` with [`verify_args`] and classifies the output with
/// [`classify_verify`] against `key`'s fingerprint. A passphrase is answered by a
/// one-shot helper that releases it only for the passphrase prompt of
/// `private_key_path` (a bastion's password prompt gets nothing; a wrong
/// passphrase is not retried in a loop) and is never stored.
/// Bastion passwords travel as in [`run_deploy`].
///
/// # Errors
///
/// [`KeySetupError::SshMissing`], [`KeySetupError::InvalidTarget`],
/// [`KeySetupError::AskpassSetup`], [`KeySetupError::Spawn`],
/// [`KeySetupError::Timeout`], [`KeySetupError::Cancelled`] and every variant
/// [`classify_verify`] can return.
pub fn run_verify(
    target: &SshTarget,
    private_key_path: &Path,
    key: &PublicKeyLine,
    passphrase: Option<&SecretString>,
    jump_host_passwords: &[(AskpassSlot, SecretString)],
    control: &RunControl,
) -> Result<VerifyOutcome, KeySetupError> {
    let program = crate::which::find_in_path("ssh").ok_or(KeySetupError::SshMissing)?;
    run_verify_with(
        &program,
        target,
        private_key_path,
        key,
        passphrase,
        jump_host_passwords,
        control,
    )
}

fn run_verify_with(
    program: &Path,
    target: &SshTarget,
    private_key_path: &Path,
    key: &PublicKeyLine,
    passphrase: Option<&SecretString>,
    jump_host_passwords: &[(AskpassSlot, SecretString)],
    control: &RunControl,
) -> Result<VerifyOutcome, KeySetupError> {
    let args = verify_args(target, private_key_path, passphrase.is_some())?;
    let mut files = SecretFiles::default();
    let mut env = stage_password_env(jump_host_passwords, None, &mut files)?;
    if let Some(passphrase) = passphrase {
        let script = create_key_passphrase_askpass_script().map_err(|e| askpass_failed(&e))?;
        files.track(script.clone());
        let secret =
            ssh_tunnel::create_tunnel_secret_file(passphrase).map_err(|e| askpass_failed(&e))?;
        files.track(secret.clone());
        push_passphrase_env(&mut env, &script, &secret);
        // `verify_args` already refused a path that is not UTF-8.
        env.push((
            KEY_PATH_ENV_VAR.to_owned(),
            private_key_path.to_string_lossy().into_owned(),
        ));
    }
    ensure_askpass_display(&mut env);
    let done = run_child(program, "ssh", &args, None, &env, control)?;
    log_stream("ssh verify stderr", &done.stderr);
    classify_verify(done.code, &done.stderr, key.fingerprint())
}

/// What a finished child printed.
struct Finished {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

/// Spawns `program`, optionally feeds `stdin_line` plus a newline to it, and
/// waits within `control`'s budget.
fn run_child<S: AsRef<OsStr>>(
    program: &Path,
    name: &'static str,
    args: &[S],
    stdin_line: Option<&str>,
    env: &[(String, String)],
    control: &RunControl,
) -> Result<Finished, KeySetupError> {
    let spawn_failed = |source| KeySetupError::Spawn {
        program: name,
        source,
    };
    if control.cancel.load(Ordering::Relaxed) {
        return Err(KeySetupError::Cancelled);
    }
    let mut command = Command::new(program);
    command
        .args(args)
        .stdin(if stdin_line.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (name, value) in env {
        command.env(name, value);
    }
    let mut child = command.spawn().map_err(spawn_failed)?;
    if let Some(line) = stdin_line
        && let Some(mut pipe) = child.stdin.take()
        && let Err(error) = pipe
            .write_all(line.as_bytes())
            .and_then(|()| pipe.write_all(b"\n"))
    {
        // The child may already be gone; its exit status tells the story.
        tracing::debug!(%error, "could not write the public key to {name} stdin");
    }
    match proc::wait_bounded_cancellable(child, control.timeout, &control.cancel, "ssh-key-setup")
        .map_err(spawn_failed)?
    {
        Some(Waited::Exited(output)) => Ok(Finished {
            code: output.status.code(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        }),
        Some(Waited::TimedOut) => Err(KeySetupError::Timeout),
        None => Err(KeySetupError::Cancelled),
    }
}

fn log_stream(what: &str, text: &str) {
    let shown: String = text.chars().take(LOG_LIMIT).collect();
    tracing::debug!(stream = what, text = %shown, "captured child output");
}

/// Temporary helper scripts and secret files, removed (zeroed first) on drop so
/// that an early `return` cannot leave one behind.
#[derive(Default)]
pub(crate) struct SecretFiles(Vec<PathBuf>);

impl SecretFiles {
    fn track(&mut self, path: PathBuf) {
        self.0.push(path);
    }
}

impl Drop for SecretFiles {
    fn drop(&mut self) {
        for path in &self.0 {
            ssh_tunnel::cleanup_askpass_script(path);
        }
    }
}

fn askpass_failed(error: &str) -> KeySetupError {
    // The text names a path and an OS error, never a secret.
    tracing::warn!(%error, "could not prepare the askpass helper");
    KeySetupError::AskpassSetup
}

/// Stages the target password and the bastion passwords as K1 secret files and
/// returns the matching environment entries (paths only).
///
/// Fails closed: if a password was given and an entry for it is missing, `ssh`
/// is not started without it.
pub(crate) fn stage_password_env(
    jump_host_passwords: &[(AskpassSlot, SecretString)],
    password: Option<&SecretString>,
    files: &mut SecretFiles,
) -> Result<Vec<(String, String)>, KeySetupError> {
    let script = match password {
        Some(_) => Some(
            ssh_askpass::helper_script(AskpassSlot::Target).map_err(|error| {
                tracing::warn!(%error, "could not write the askpass helper");
                KeySetupError::AskpassSetup
            })?,
        ),
        None => None,
    };
    let (entries, cleanup) =
        ssh_askpass::child_env(jump_host_passwords, password.zip(script.as_deref()));
    for path in cleanup {
        files.track(path);
    }
    // One entry per bastion, plus the file path, the helper and the "force"
    // switch for the target password.
    let expected = jump_host_passwords.len() + if password.is_some() { 3 } else { 0 };
    if entries.len() != expected {
        return Err(KeySetupError::AskpassSetup);
    }
    Ok(entries
        .iter()
        .filter_map(|entry| entry.split_once('='))
        .map(|(name, value)| (name.to_owned(), value.to_owned()))
        .collect())
}

/// Environment variable naming the key whose passphrase prompt the verify
/// helper may answer. A path, not a secret.
const KEY_PATH_ENV_VAR: &str = "_RC_KEY_PASSPHRASE_FOR";

/// A one-shot `SSH_ASKPASS` helper that answers **only** the passphrase prompt
/// for the key named by [`KEY_PATH_ENV_VAR`], and exits 1 for anything else.
///
/// `SSH_ASKPASS` is inherited by every child of the verification `ssh`, a
/// `ProxyJump` or `ProxyCommand` hop included. An always-answering helper would
/// hand the key passphrase to a bastion's `user@bastion's password:` prompt, over
/// the network. OpenSSH prompts `Enter passphrase for key '<path>': ` (the path
/// cut at 100 bytes) or, in older releases, `Enter passphrase for <path>: `; the
/// comparison is a quoted, hence literal, `case` pattern. Like the one-shot
/// tunnel helper it empties the secret file after the first answer, so a wrong
/// passphrase is not retried.
fn create_key_passphrase_askpass_script() -> Result<PathBuf, String> {
    ssh_tunnel::write_askpass_script(&format!(
        "#!/bin/sh\n\
         key=\"${{{KEY_PATH_ENV_VAR}-}}\"\n\
         [ -n \"$key\" ] || exit 1\n\
         short=$(printf '%.100s' \"$key\")\n\
         case \"${{1-}}\" in\n\
         \x20\x20\"Enter passphrase for key '$key':\"*|\"Enter passphrase for key '$short':\"*|\\\n\
         \x20\x20\"Enter passphrase for $key:\"*) ;;\n\
         \x20\x20*) exit 1 ;;\n\
         esac\n\
         secret_file=\"${{{TUN_SECRET_ENV_VAR}-}}\"\n\
         [ -n \"$secret_file\" ] || exit 1\n\
         [ -s \"$secret_file\" ] || exit 1\n\
         exec 3<\"$secret_file\" || exit 1\n\
         cat <&3\n\
         : > \"$secret_file\"\n",
        TUN_SECRET_ENV_VAR = ssh_tunnel::TUNNEL_ASKPASS_ENV_VAR
    ))
}

fn push_passphrase_env(env: &mut Vec<(String, String)>, script: &Path, secret: &Path) {
    env.push((
        "SSH_ASKPASS".to_owned(),
        script.to_string_lossy().into_owned(),
    ));
    env.push(("SSH_ASKPASS_REQUIRE".to_owned(), "force".to_owned()));
    env.push((
        ssh_tunnel::TUNNEL_ASKPASS_ENV_VAR.to_owned(),
        secret.to_string_lossy().into_owned(),
    ));
    ensure_askpass_display(env);
}

/// Older OpenSSH only consults `SSH_ASKPASS` when `DISPLAY` is set; the other
/// askpass users in this crate do the same.
pub(crate) fn ensure_askpass_display(env: &mut Vec<(String, String)>) {
    let has_askpass = env.iter().any(|(name, _)| name == "SSH_ASKPASS");
    let has_display = env.iter().any(|(name, _)| name == "DISPLAY");
    if has_askpass && !has_display && std::env::var_os("DISPLAY").is_none() {
        env.push(("DISPLAY".to_owned(), String::new()));
    }
}

#[cfg(test)]
mod tests;
