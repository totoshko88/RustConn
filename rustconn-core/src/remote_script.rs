//! Runs a local script on many hosts: bash over SSH, PowerShell over WinRM.
//!
//! The script travels over stdin and nothing is written to the server. It is
//! read whole before any of it runs, and stdin stays open for the whole run so
//! that losing the connection stops the script, see [`REMOTE_COMMAND`]. Each host goes
//! through the same control channel as SSH key setup
//! ([`crate::ssh_key_setup::remote_command_args`]): strict host key checking, no
//! forwarding, no tty, and passwords through the askpass helper only.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use thiserror::Error;
use uuid::Uuid;

use secrecy::{ExposeSecret, SecretString};
use zeroize::Zeroizing;

use crate::models::{
    Connection, ConnectionGroup, ProtocolConfig, ProtocolType, collect_descendant_group_ids,
};
use crate::proc::{self, Polled};
use crate::ssh_key_setup::{self, DeployCredentials, SecretFiles, SshTarget};
use crate::ssh_tunnel::{self, SshTunnel, SshTunnelParams};

/// The remote command that receives the script on stdin, ended by a NUL byte.
///
/// Reading everything first means commands inside the script see an empty stdin
/// instead of eating the rest of the script, as they do under `bash -s`. The
/// `bash -n` pass is what makes a syntax error stop the script before its first
/// line runs; `eval` alone would execute the lines ahead of the error. `eval` is
/// a builtin, so the script size is not limited by `ARG_MAX`. The check reads the
/// script through a pipe from the `printf` builtin, never a here-string: bash
/// before 5.1 writes every here-string to a temporary file on the host, and later
/// versions do so above 64 KiB.
///
/// There is no tty, so sshd sends no hangup when the connection ends and the
/// script would carry on. The sender therefore holds stdin open until the run is
/// over: the script runs as its own process group (`set -m`) and a watcher waiting
/// on stdin signals that group when it reaches end of file, TERM first and KILL
/// two seconds later. Once past the `read` the watcher ignores TERM, or the
/// exiting shell would cancel the KILL still owed to a script that survived TERM.
/// A status of 255 also prints [`SCRIPT_EXIT_255_MARKER`], see there.
pub const REMOTE_COMMAND: &str = r#"bash -c 'exec 3<&0; IFS= read -r -d "" s || exit 125; printf %s "$s" | bash -n || exit 2; set -m; eval "$s" </dev/null 3<&- & p=$!; set +m; { read -r _ <&3; trap "" TERM; kill -TERM -- -$p; sleep 2; kill -KILL -- -$p; } >/dev/null 2>&1 & w=$!; wait $p; c=$?; kill $w 2>/dev/null; [ $c -ne 255 ] || printf "\036rustconn-script-exit-255\n" >&2; exit $c'"#;

/// The stderr line [`REMOTE_COMMAND`] prints when the script exits with 255.
///
/// `ssh` uses 255 for its own failures, so the status alone cannot tell a script
/// that chose it from a connection that failed.
const SCRIPT_EXIT_255_MARKER: &[u8] = b"\x1erustconn-script-exit-255";

/// Runs a PowerShell script over WinRM with the `pywinrm` library.
///
/// `evil-winrm` has no way to run one script and report its exit status, so
/// script runs need a client that does. The first stdin line is a JSON header
/// holding the password; then comes the script and a NUL. As with SSH, stdin stays
/// open for the whole run: at end of file the helper closes the remote shell,
/// which ends the script on the host.
///
/// The script is sent base64-encoded because a WinRM console reads stdin in the
/// OEM code page, and in chunks because a WinRM message is limited in size.
/// Every stream is merged and printed by the bootstrap, because under
/// `-EncodedCommand` PowerShell otherwise writes them to stderr as CLIXML, whatever
/// `-OutputFormat` says. It has to be a script block: `Invoke-Expression` loses the
/// error stream when redirected. `Out-String` renders objects before the final
/// `exit`, which would discard a table still waiting in the default formatter. A
/// script fails if its last output was an error, as `$?` would report.
///
/// ponytail: NTLM only; add Kerberos through `transport` if domains need it.
/// ponytail: a table still being formatted when the script calls `exit N` is lost,
/// because `exit` ends the process; `| Out-String` in the script avoids it.
const WINRM_HELPER: &str = r#"
import base64, json, os, sys, threading
def fail(message):
    sys.stderr.write(message + "\n")
    sys.stderr.flush()
    sys.exit(255)
try:
    from winrm.protocol import Protocol
    from winrm.exceptions import WinRMOperationTimeoutError as Idle
except ImportError:
    fail("pywinrm is not installed on this computer. Install python3-winrm, or: pip install pywinrm")
BOOT = r'''
$ProgressPreference='SilentlyContinue'
try { [Console]::OutputEncoding = New-Object Text.UTF8Encoding($false) } catch {}
$__rc_s = [Text.Encoding]::UTF8.GetString([Convert]::FromBase64String([Console]::In.ReadToEnd()))
$__rc_t = $null; $__rc_e = $null
[void][Management.Automation.Language.Parser]::ParseInput($__rc_s, [ref]$__rc_t, [ref]$__rc_e)
if ($__rc_e.Count) { foreach ($__rc_x in $__rc_e) { [Console]::Error.WriteLine($__rc_x.ToString()) }; exit 2 }
$global:LASTEXITCODE = 0
$__rc_f = $false
try {
    & ([ScriptBlock]::Create($__rc_s)) *>&1 | ForEach-Object {
        $__rc_f = $_ -is [Management.Automation.ErrorRecord]
        if ($__rc_f) { [Console]::Error.WriteLine($_.ToString()) }
        elseif ($_ -is [Management.Automation.WarningRecord]) { [Console]::Error.WriteLine("WARNING: " + $_.Message) }
        elseif ($_ -is [Management.Automation.InformationRecord]) { [Console]::Out.WriteLine($_.ToString()) }
        elseif ($_ -is [Management.Automation.VerboseRecord] -or $_ -is [Management.Automation.DebugRecord]) { [Console]::Error.WriteLine($_.Message) }
        else { $_ }
    } | Out-String -Stream -Width 160 | ForEach-Object { [Console]::Out.WriteLine($_) }
} catch { [Console]::Error.WriteLine($_.ToString()); exit 1 }
if ($LASTEXITCODE) { exit $LASTEXITCODE }
if ($__rc_f) { exit 1 }
exit 0
'''
stream = sys.stdin.buffer
header = json.loads(stream.readline())
script = bytearray()
while True:
    byte = stream.read(1)
    if not byte:
        sys.exit(125)
    if byte == b"\0":
        break
    script += byte
stop = threading.Event()
def watch():
    while os.read(0, 4096):
        pass
    stop.set()
threading.Thread(target=watch, daemon=True).start()
try:
    client = Protocol(endpoint=header["endpoint"], transport="ntlm", username=header["username"],
                      password=header["password"], server_cert_validation=header["certificates"],
                      read_timeout_sec=15, operation_timeout_sec=10)
    del header
    shell = client.open_shell()
    command = client.run_command(shell, "powershell", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass",
        "-OutputFormat", "Text", "-EncodedCommand", base64.b64encode(BOOT.encode("utf-16-le")).decode()])
    data = base64.b64encode(bytes(script)).decode()
    size = 65536
    for start in range(0, len(data), size):
        if stop.is_set():
            break
        client.send_command_input(shell, command, data[start:start + size], end=start + size >= len(data))
    done = False
    code = 0
    while not done and not stop.is_set():
        try:
            out, err, code, done = client.get_command_output_raw(shell, command)
        except Idle:
            continue
        sys.stdout.buffer.write(out)
        sys.stdout.buffer.flush()
        sys.stderr.buffer.write(err)
        sys.stderr.buffer.flush()
    if stop.is_set():
        client.close_shell(shell)
        sys.exit(130)
    client.cleanup_command(shell, command)
    client.close_shell(shell)
except Exception as error:
    fail("WinRM failed: %s" % error)
if code == 255:
    sys.stderr.write("\x1erustconn-script-exit-255\n")
elif not 0 <= code <= 255:
    sys.stderr.write("exit code %d\n" % code)
    code = 1
sys.stderr.flush()
sys.exit(code)
"#;

/// How long the WinRM helper gets to close the remote shell when stopped: one
/// operation timeout, plus the close itself.
const WINRM_STOP_GRACE: Duration = Duration::from_secs(25);

/// Largest script accepted. A script is text; a bigger file is a wrong pick.
pub const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;
/// Output kept per host, so a chatty script cannot exhaust memory.
pub const MAX_OUTPUT_BYTES_PER_HOST: usize = 4 * 1024 * 1024;
/// Hosts running at once unless the caller chooses.
pub const DEFAULT_PARALLEL: usize = 8;
/// Upper bound on hosts running at once.
pub const MAX_PARALLEL: usize = 64;
/// Time allowed per host unless the caller chooses.
pub const DEFAULT_HOST_TIMEOUT: Duration = Duration::from_mins(10);

/// Longest output line delivered in one piece.
const MAX_LINE_BYTES: u64 = 64 * 1024;
/// How long to wait for output after `ssh` exits. A `ProxyCommand` child can
/// outlive it and hold the pipe open, so this must not wait for end of file.
const OUTPUT_DRAIN: Duration = Duration::from_secs(2);

/// Why a local script file cannot be used.
#[derive(Debug, Error)]
pub enum ScriptFileError {
    /// The file could not be read.
    #[error("cannot read the script: {0}")]
    Unreadable(#[from] std::io::Error),
    /// The path is not a regular file.
    #[error("the script is not a regular file")]
    NotAFile,
    /// The file is empty or only whitespace.
    #[error("the script is empty")]
    Empty,
    /// The file is larger than [`MAX_SCRIPT_BYTES`].
    #[error("the script is larger than {max} bytes")]
    TooLarge {
        /// The limit in bytes.
        max: u64,
    },
    /// The file holds NUL bytes, so it is not a shell script.
    #[error("the script is a binary file")]
    Binary,
}

/// Reads and checks a local script.
///
/// # Errors
///
/// Returns a [`ScriptFileError`] when the file is unreadable, not a regular
/// file, empty, too large or binary.
pub fn load_script(path: &Path) -> Result<Vec<u8>, ScriptFileError> {
    let metadata = std::fs::metadata(path)?;
    if !metadata.is_file() {
        return Err(ScriptFileError::NotAFile);
    }
    if metadata.len() > MAX_SCRIPT_BYTES {
        return Err(ScriptFileError::TooLarge {
            max: MAX_SCRIPT_BYTES,
        });
    }
    let script = std::fs::read(path)?;
    if script.contains(&0) {
        return Err(ScriptFileError::Binary);
    }
    if script.iter().all(u8::is_ascii_whitespace) {
        return Err(ScriptFileError::Empty);
    }
    Ok(script)
}

/// Which kind of script a file is, which decides the hosts it can run on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    /// A bash script, for SSH hosts.
    Bash,
    /// A PowerShell script (`.ps1`), for WinRM hosts.
    PowerShell,
}

impl ScriptKind {
    /// Tells the kind from a file name.
    #[must_use]
    pub fn of_path(path: &Path) -> Self {
        let powershell = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("ps1"));
        if powershell {
            Self::PowerShell
        } else {
            Self::Bash
        }
    }

    const fn protocol(self) -> ProtocolType {
        match self {
            Self::Bash => ProtocolType::Ssh,
            Self::PowerShell => ProtocolType::WinRm,
        }
    }
}

/// The connections a selection resolves to.
#[derive(Debug, Default)]
pub struct ScriptTargets<'a> {
    /// Connections the script can run on, sorted by name.
    pub hosts: Vec<&'a Connection>,
    /// Selected connections left out because they use another protocol.
    pub skipped: usize,
}

/// Resolves selected connection and group ids to the hosts a `kind` script runs on.
///
/// A group stands for every connection in it and in its subgroups.
#[must_use]
pub fn script_targets<'a>(
    selected: &[Uuid],
    connections: &'a [Connection],
    groups: &[ConnectionGroup],
    kind: ScriptKind,
) -> ScriptTargets<'a> {
    let group_ids: HashSet<Uuid> = selected
        .iter()
        .copied()
        .filter(|id| groups.iter().any(|g| g.id == *id))
        .flat_map(|id| collect_descendant_group_ids(id, groups))
        .collect();

    let mut targets = ScriptTargets::default();
    for connection in connections {
        let chosen = selected.contains(&connection.id)
            || connection.group_id.is_some_and(|g| group_ids.contains(&g));
        if !chosen {
            continue;
        }
        if connection.protocol == kind.protocol() {
            targets.hosts.push(connection);
        } else {
            targets.skipped += 1;
        }
    }
    targets.hosts.sort_by_key(|c| c.name.to_lowercase());
    targets
}

/// One host to run the script on.
#[derive(Debug)]
pub struct RemoteHost {
    /// The display name, for reporting.
    pub name: String,
    /// How to reach it.
    pub transport: ScriptTransport,
}

/// How a script reaches a host.
#[derive(Debug)]
pub enum ScriptTransport {
    /// Bash over SSH.
    Ssh {
        /// Where to connect, with the connection's own `ssh` arguments.
        target: SshTarget,
        /// Passwords for the host and its jump hosts. Empty for key or agent login.
        credentials: DeployCredentials,
    },
    /// PowerShell over WinRM.
    WinRm(WinRmTarget),
}

/// A WinRM host to run a script on.
#[derive(Debug)]
pub struct WinRmTarget {
    /// Host name or address.
    pub host: String,
    /// WinRM port.
    pub port: u16,
    /// The account, `DOMAIN\\user` for a domain account.
    pub account: String,
    /// The account password.
    pub password: Option<SecretString>,
    /// Use HTTPS.
    pub use_ssl: bool,
    /// Accept a self-signed or mismatched certificate.
    pub accept_invalid_certs: bool,
    /// An SSH tunnel to reach the host through.
    pub tunnel: Option<SshTunnelParams>,
}

impl WinRmTarget {
    /// Builds the target of a WinRM `connection`, `None` for any other protocol.
    ///
    /// The tunnel's jump host must already be in `known_hosts`: its `ssh` is
    /// given `StrictHostKeyChecking=yes` ahead of the caller's arguments, so a
    /// run never accepts a new key or sends a password to an unknown one. Jump
    /// hosts beyond it, reached through `-J`, follow the user's ssh
    /// configuration, as in [`crate::sftp`].
    #[must_use]
    pub fn for_connection(
        connection: &Connection,
        password: Option<SecretString>,
        tunnel: Option<SshTunnelParams>,
    ) -> Option<Self> {
        let ProtocolConfig::WinRm(config) = &connection.protocol_config else {
            return None;
        };
        Some(Self {
            host: connection.host.clone(),
            port: connection.port,
            account: crate::protocol::winrm_account_name(connection).unwrap_or_default(),
            password,
            use_ssl: config.use_ssl,
            accept_invalid_certs: config.accept_invalid_certs,
            tunnel: tunnel.map(require_known_host_key),
        })
    }
}

/// Puts `StrictHostKeyChecking=yes` first in a tunnel's arguments. OpenSSH
/// keeps the first value of an option, so the caller's arguments cannot relax it.
fn require_known_host_key(mut tunnel: SshTunnelParams) -> SshTunnelParams {
    tunnel.extra_args.splice(
        0..0,
        ["-o".to_string(), "StrictHostKeyChecking=yes".to_string()],
    );
    tunnel
}

/// How a run behaves.
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// Hosts running at once, from 1 to [`MAX_PARALLEL`].
    pub parallel: usize,
    /// Time allowed per host before it is stopped.
    pub timeout: Duration,
    /// Set to `true` to stop running hosts and skip the rest.
    pub cancel: Arc<AtomicBool>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            parallel: DEFAULT_PARALLEL,
            timeout: DEFAULT_HOST_TIMEOUT,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// How the script ended on one host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptOutcome {
    /// The script exited with status 0.
    Succeeded,
    /// The script exited with a failure status (`None` if killed by a signal).
    ScriptFailed {
        /// The exit status.
        code: Option<i32>,
    },
    /// `ssh` exited with 255 on its own account: it could not connect or log in,
    /// or the connection dropped.
    ConnectionFailed,
    /// The host exceeded its time limit.
    TimedOut,
    /// The run was cancelled.
    Cancelled,
    /// The client could not be started for this host. A
    /// [`ScriptEvent::NotStarted`] says why, unless waiting for it failed.
    NotStarted,
}

/// Why a host's client could not be started.
///
/// Typed so the GUI can translate the reason; the `detail` fields carry the
/// untranslated text of the underlying error, which never holds a secret.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LaunchError {
    /// `ssh` is not on `PATH`.
    #[error("ssh was not found")]
    SshNotFound,
    /// `python3` is not on `PATH`.
    #[error("python3 was not found")]
    PythonNotFound,
    /// The Python found cannot import `winrm`: outside a sandbox `python3`
    /// lacks the library, inside one the `pywinrm` component is not installed.
    #[error("the pywinrm library is not installed")]
    PyWinRmNotInstalled,
    /// A WinRM host has no password; the helper cannot log in without one.
    #[error("WinRM needs a password; none is saved for this host")]
    NoWinRmPassword,
    /// The host's `ssh` arguments were refused.
    #[error("the host cannot be reached with ssh: {detail}")]
    InvalidTarget {
        /// Why, untranslated.
        detail: String,
    },
    /// The askpass files for the passwords could not be written.
    #[error("the password could not be handed to ssh: {detail}")]
    PasswordNotStaged {
        /// Why, untranslated.
        detail: String,
    },
    /// The SSH tunnel to a WinRM host's jump host did not come up.
    #[error("the SSH tunnel could not be opened: {detail}")]
    TunnelFailed {
        /// Why, untranslated.
        detail: String,
    },
    /// The WinRM helper's request could not be encoded.
    #[error("the WinRM request could not be prepared: {detail}")]
    RequestNotEncoded {
        /// Why, untranslated.
        detail: String,
    },
    /// The client process could not be spawned.
    #[error("{program} could not be started: {detail}")]
    SpawnFailed {
        /// The program's path.
        program: String,
        /// Why, untranslated.
        detail: String,
    },
}

/// Which pipe an output line came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutputStream {
    /// Standard output.
    Stdout,
    /// Standard error, including messages from `ssh` itself.
    Stderr,
}

/// Progress of a run. `host` is an index into the list passed to [`run_script`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptEvent {
    /// The host has started.
    Started {
        /// The host.
        host: usize,
    },
    /// The host printed a line.
    Output {
        /// The host.
        host: usize,
        /// The pipe it came from.
        stream: OutputStream,
        /// The line, without control characters.
        line: String,
    },
    /// The host passed [`MAX_OUTPUT_BYTES_PER_HOST`]; later output is dropped.
    OutputTruncated {
        /// The host.
        host: usize,
    },
    /// The host's client could not be started. A
    /// [`ScriptEvent::Finished`] with [`ScriptOutcome::NotStarted`] follows.
    NotStarted {
        /// The host.
        host: usize,
        /// Why.
        reason: LaunchError,
    },
    /// The host has finished.
    Finished {
        /// The host.
        host: usize,
        /// How it ended.
        outcome: ScriptOutcome,
    },
}

/// Receives events, from several threads at once.
pub type ScriptEventSink = Arc<dyn Fn(ScriptEvent) + Send + Sync>;

/// Runs `script` on every host and returns when all have finished.
///
/// Blocks, so keep it off the GTK main thread.
pub fn run_script(
    script: &Arc<Vec<u8>>,
    hosts: &[RemoteHost],
    options: &RunOptions,
    on_event: &ScriptEventSink,
) {
    let has_winrm = hosts
        .iter()
        .any(|h| matches!(h.transport, ScriptTransport::WinRm(_)));
    let programs = Programs {
        ssh: crate::which::find_in_path("ssh"),
        // Never read without a WinRM host, so an SSH-only run starts no probe.
        python: if has_winrm {
            winrm_python()
        } else {
            Err(LaunchError::PythonNotFound)
        },
    };
    run_script_with(&programs, script, hosts, options, on_event);
}

/// The local programs a run can use.
struct Programs {
    ssh: Option<PathBuf>,
    /// A Python that can import `winrm`, or why there is none.
    python: Result<PathBuf, LaunchError>,
}

/// The Python that runs [`WINRM_HELPER`].
///
/// Inside a sandbox it is the `pywinrm` component's wrapper, a `python3` that
/// sees the component's site-packages; outside one it is `python3` from `PATH`.
fn winrm_python() -> Result<PathBuf, LaunchError> {
    let python = if crate::is_sandboxed() {
        crate::cli_download::get_component(crate::cli_download::PYWINRM_COMPONENT)
            .and_then(crate::cli_download::DownloadableComponent::find_installed_binary)
            .ok_or(LaunchError::PyWinRmNotInstalled)?
    } else {
        crate::which::find_in_path("python3").ok_or(LaunchError::PythonNotFound)?
    };
    probe_winrm(python)
}

/// How long `python -c "import winrm"` may take.
const WINRM_PROBE_TIMEOUT: Duration = Duration::from_secs(10);

/// Refuses `python` when it cleanly fails to import `winrm`, so a missing
/// library is named before the run instead of as each host's ImportError.
///
/// A probe that cannot be spawned, polled or finished in time passes: the
/// real launch then reports what is wrong.
fn probe_winrm(python: PathBuf) -> Result<PathBuf, LaunchError> {
    let child = Command::new(&python)
        .args(["-c", "import winrm"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    let child = match child {
        Ok(child) => child,
        Err(error) => {
            tracing::debug!(%error, "the pywinrm probe could not start");
            return Ok(python);
        }
    };
    match crate::proc::wait_bounded(child, WINRM_PROBE_TIMEOUT, "pywinrm probe") {
        Ok(crate::proc::Waited::Exited(output)) if !output.status.success() => {
            Err(LaunchError::PyWinRmNotInstalled)
        }
        Ok(_) => Ok(python),
        Err(error) => {
            tracing::debug!(%error, "the pywinrm probe could not be polled");
            Ok(python)
        }
    }
}

fn run_script_with(
    programs: &Programs,
    script: &Arc<Vec<u8>>,
    hosts: &[RemoteHost],
    options: &RunOptions,
    on_event: &ScriptEventSink,
) {
    let workers = options.parallel.clamp(1, MAX_PARALLEL).min(hosts.len());
    let next = AtomicUsize::new(0);
    std::thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| {
                loop {
                    let index = next.fetch_add(1, Ordering::Relaxed);
                    let Some(host) = hosts.get(index) else {
                        break;
                    };
                    let outcome = if options.cancel.load(Ordering::Relaxed) {
                        ScriptOutcome::Cancelled
                    } else {
                        on_event(ScriptEvent::Started { host: index });
                        run_host(programs, script, index, host, options, on_event)
                    };
                    on_event(ScriptEvent::Finished {
                        host: index,
                        outcome,
                    });
                }
            });
        }
    });
}

fn not_started(host: usize, reason: LaunchError, on_event: &ScriptEventSink) -> ScriptOutcome {
    on_event(ScriptEvent::NotStarted { host, reason });
    ScriptOutcome::NotStarted
}

/// One host's client process, ready to start.
struct Launch {
    program: PathBuf,
    args: Vec<String>,
    env: Vec<(String, String)>,
    /// Written to stdin ahead of the script.
    preamble: Zeroizing<Vec<u8>>,
    /// How long the client gets to clean up after stdin closes.
    stop_grace: Duration,
    /// Held to the end of the run: dropping it removes the secret files.
    _files: SecretFiles,
    /// Held to the end of the run: dropping it closes the tunnel.
    _tunnel: Option<SshTunnel>,
}

fn ssh_launch(
    programs: &Programs,
    target: &SshTarget,
    credentials: &DeployCredentials,
) -> Result<Launch, LaunchError> {
    let program = programs.ssh.clone().ok_or(LaunchError::SshNotFound)?;
    let args =
        ssh_key_setup::remote_command_args(target, credentials.password.is_some(), REMOTE_COMMAND)
            .map_err(|error| LaunchError::InvalidTarget {
                detail: error.to_string(),
            })?;
    let mut files = SecretFiles::default();
    let mut env = ssh_key_setup::stage_password_env(
        &credentials.jump_host_passwords,
        credentials.password.as_ref(),
        &mut files,
    )
    .map_err(|error| LaunchError::PasswordNotStaged {
        detail: error.to_string(),
    })?;
    ssh_key_setup::ensure_askpass_display(&mut env);
    Ok(Launch {
        program,
        args,
        env,
        preamble: Zeroizing::default(),
        stop_grace: Duration::ZERO,
        _files: files,
        _tunnel: None,
    })
}

/// The first stdin line of [`WINRM_HELPER`].
#[derive(serde::Serialize)]
struct WinRmHeader<'a> {
    endpoint: &'a str,
    username: &'a str,
    password: &'a str,
    certificates: &'a str,
}

/// The most bytes [`winrm_preamble`] can write for `header`.
///
/// JSON escapes a byte into at most six (`\u001f`), and the keys, quotes and
/// punctuation fit in the fixed part.
fn winrm_preamble_capacity(header: &WinRmHeader<'_>) -> usize {
    let fields = header.endpoint.len()
        + header.username.len()
        + header.password.len()
        + header.certificates.len();
    fields * 6 + 128
}

/// The JSON header line holding the password, in one buffer that never grows.
///
/// A `Vec` that reallocates frees its old storage without zeroing it, and
/// `serde_json::to_vec` grows from a small start, so each step would leave a
/// copy of the password behind. Reserving the worst case first keeps the only
/// copy in the `Zeroizing` buffer.
fn winrm_preamble(header: &WinRmHeader<'_>) -> Result<Zeroizing<Vec<u8>>, LaunchError> {
    let mut preamble = Zeroizing::new(Vec::with_capacity(winrm_preamble_capacity(header)));
    serde_json::to_writer(&mut *preamble, header).map_err(|e| LaunchError::RequestNotEncoded {
        detail: e.to_string(),
    })?;
    preamble.push(b'\n');
    Ok(preamble)
}

fn winrm_launch(programs: &Programs, target: &WinRmTarget) -> Result<Launch, LaunchError> {
    let program = programs.python.clone()?;
    let password = target
        .password
        .as_ref()
        .ok_or(LaunchError::NoWinRmPassword)?;

    let tunnel_failed = |e: ssh_tunnel::SshTunnelError| LaunchError::TunnelFailed {
        detail: e.to_string(),
    };
    let tunnel = target
        .tunnel
        .as_ref()
        .map(|params| {
            let mut tunnel = ssh_tunnel::create_tunnel(params).map_err(tunnel_failed)?;
            ssh_tunnel::wait_for_tunnel_ready(&mut tunnel, 50, Duration::from_millis(200))
                .map_err(tunnel_failed)?;
            Ok::<_, LaunchError>(tunnel)
        })
        .transpose()?;
    let (host, port) = tunnel.as_ref().map_or_else(
        || (target.host.clone(), target.port),
        |tunnel| ("127.0.0.1".to_string(), tunnel.local_port()),
    );
    let scheme = if target.use_ssl { "https" } else { "http" };
    let host = if host.contains(':') {
        format!("[{host}]")
    } else {
        host
    };
    let header = WinRmHeader {
        endpoint: &format!("{scheme}://{host}:{port}/wsman"),
        username: &target.account,
        password: password.expose_secret(),
        certificates: if target.accept_invalid_certs {
            "ignore"
        } else {
            "validate"
        },
    };
    let preamble = winrm_preamble(&header)?;
    Ok(Launch {
        program,
        args: vec!["-c".to_string(), WINRM_HELPER.to_string()],
        env: Vec::new(),
        preamble,
        stop_grace: WINRM_STOP_GRACE,
        _files: SecretFiles::default(),
        _tunnel: tunnel,
    })
}

fn run_host(
    programs: &Programs,
    script: &Arc<Vec<u8>>,
    index: usize,
    host: &RemoteHost,
    options: &RunOptions,
    on_event: &ScriptEventSink,
) -> ScriptOutcome {
    let launched = match &host.transport {
        ScriptTransport::Ssh {
            target,
            credentials,
        } => ssh_launch(programs, target, credentials),
        ScriptTransport::WinRm(target) => winrm_launch(programs, target),
    };
    let mut launch = match launched {
        Ok(launch) => launch,
        Err(error) => return not_started(index, error, on_event),
    };

    let mut child = match Command::new(&launch.program)
        .args(&launch.args)
        .envs(std::mem::take(&mut launch.env))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => {
            let reason = LaunchError::SpawnFailed {
                program: launch.program.display().to_string(),
                detail: error.to_string(),
            };
            return not_started(index, reason, on_event);
        }
    };

    let (release_stdin, hold_stdin) = mpsc::channel::<()>();
    let preamble = std::mem::take(&mut launch.preamble);
    if let Some(mut pipe) = child.stdin.take() {
        let script = Arc::clone(script);
        // A thread of its own: `ssh` may fill its output pipes before it has
        // read the whole script. A write error means `ssh` exited, which the
        // exit status reports. The pipe stays open until this function returns,
        // because its end of file is what stops the script (see REMOTE_COMMAND).
        std::thread::spawn(move || {
            let sent = pipe.write_all(&preamble).is_ok()
                && pipe.write_all(&script).is_ok()
                && pipe.write_all(&[0]).is_ok();
            drop(preamble);
            if sent {
                let _ = pipe.flush();
                let _ = hold_stdin.recv();
            }
        });
    }

    let budget = Arc::new(AtomicUsize::new(0));
    let exited_255 = Arc::new(AtomicBool::new(false));
    let (done_tx, done_rx) = mpsc::channel();
    let mut pumps = 0;
    if let Some(pipe) = child.stdout.take() {
        pumps += 1;
        spawn_pump(
            pipe,
            OutputStream::Stdout,
            index,
            &budget,
            &exited_255,
            on_event,
            &done_tx,
        );
    }
    if let Some(pipe) = child.stderr.take() {
        pumps += 1;
        spawn_pump(
            pipe,
            OutputStream::Stderr,
            index,
            &budget,
            &exited_255,
            on_event,
            &done_tx,
        );
    }
    drop(done_tx);

    let outcome = wait_for(&mut child, options, release_stdin, launch.stop_grace);

    let deadline = Instant::now() + OUTPUT_DRAIN;
    for _ in 0..pumps {
        let left = deadline.saturating_duration_since(Instant::now());
        if done_rx.recv_timeout(left).is_err() {
            break;
        }
    }
    if outcome == ScriptOutcome::ConnectionFailed && exited_255.load(Ordering::Relaxed) {
        return ScriptOutcome::ScriptFailed { code: Some(255) };
    }
    outcome
}

/// Waits for `child`, stopping it on timeout or cancellation.
fn wait_for(
    child: &mut Child,
    options: &RunOptions,
    release_stdin: mpsc::Sender<()>,
    stop_grace: Duration,
) -> ScriptOutcome {
    let outcome = match proc::poll_until(child, options.timeout, &options.cancel) {
        Ok(Polled::Exited(status)) => {
            return match status.code() {
                Some(0) => ScriptOutcome::Succeeded,
                Some(255) => ScriptOutcome::ConnectionFailed,
                code => ScriptOutcome::ScriptFailed { code },
            };
        }
        Ok(Polled::Cancelled) => ScriptOutcome::Cancelled,
        Ok(Polled::OutOfTime) => ScriptOutcome::TimedOut,
        Err(error) => {
            tracing::warn!(%error, "could not wait for the client");
            ScriptOutcome::NotStarted
        }
    };
    stop(child, release_stdin, stop_grace);
    outcome
}

/// Closes the client's stdin, gives it `grace` to clean up, then kills it.
fn stop(child: &mut Child, release_stdin: mpsc::Sender<()>, grace: Duration) {
    drop(release_stdin);
    if !matches!(
        proc::poll_until(child, grace, &AtomicBool::new(false)),
        Ok(Polled::Exited(_))
    ) {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn spawn_pump(
    pipe: impl Read + Send + 'static,
    stream: OutputStream,
    host: usize,
    budget: &Arc<AtomicUsize>,
    exited_255: &Arc<AtomicBool>,
    on_event: &ScriptEventSink,
    done: &mpsc::Sender<()>,
) {
    let budget = Arc::clone(budget);
    let exited_255 = Arc::clone(exited_255);
    let on_event = Arc::clone(on_event);
    let done = done.clone();
    std::thread::spawn(move || {
        let mut reader = BufReader::new(pipe);
        let mut line = Vec::new();
        loop {
            line.clear();
            let read = reader
                .by_ref()
                .take(MAX_LINE_BYTES)
                .read_until(b'\n', &mut line);
            let count = match read {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            if stream == OutputStream::Stderr
                && line.strip_suffix(b"\n").unwrap_or(&line) == SCRIPT_EXIT_255_MARKER
            {
                exited_255.store(true, Ordering::Relaxed);
                continue;
            }
            let used = budget.fetch_add(count, Ordering::Relaxed) + count;
            if used <= MAX_OUTPUT_BYTES_PER_HOST {
                on_event(ScriptEvent::Output {
                    host,
                    stream,
                    line: clean_line(&line),
                });
            } else if used - count <= MAX_OUTPUT_BYTES_PER_HOST {
                on_event(ScriptEvent::OutputTruncated { host });
            }
        }
        let _ = done.send(());
    });
}

/// Makes remote output safe to display: a hostile server must not be able to
/// send terminal escape sequences to the CLI's terminal.
///
/// Whole escape sequences go first, then every control character that is left
/// except tab, so a lone ESC never survives either.
fn clean_line(raw: &[u8]) -> String {
    crate::session::strip_ansi_escapes(&String::from_utf8_lossy(raw))
        .chars()
        .filter(|c| *c == '\t' || !c.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::{Mutex, OnceLock};

    use super::*;

    /// A stand-in for `ssh` whose behaviour is chosen by the destination, the
    /// second-to-last argument. Written once and probed before use, because
    /// another test forking while the file is open for writing makes exec fail
    /// with ETXTBSY.
    fn stub() -> &'static Path {
        static STUB: OnceLock<(tempfile::TempDir, PathBuf)> = OnceLock::new();
        let (_, path) = STUB.get_or_init(|| {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("ssh");
            let body = r#"#!/bin/bash
for last in "$@"; do dest="$prev"; prev="$last"; done
case "$dest" in
  down) echo "ssh: connect failed" >&2; exit 255 ;;
esac
IFS= read -r -d "" script
case "$dest" in
  echo-script) printf %s "$script" ;;
  both) echo out; echo err >&2 ;;
  fail) exit 3 ;;
  script255) printf '\036rustconn-script-exit-255\n' >&2; exit 255 ;;
  slow) sleep 30 ;;
  escape) printf 'a\033]0;x\007b\n' ;;
  color) printf 'a\033[31mb\n' ;;
  flood) yes aaaaaaaaaaaaaaaa | head -c 5000000 ;;
esac
"#;
            std::fs::write(&path, body).unwrap();
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
            for _ in 0..200 {
                if Command::new(&path)
                    .arg("x")
                    .arg("y")
                    .stdin(Stdio::null())
                    .status()
                    .is_ok()
                {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            (dir, path)
        });
        path
    }

    /// `python3` running against a stand-in `winrm` module. The script's text
    /// picks the behaviour; `None` when this machine has no Python.
    fn python_stub() -> Option<&'static Path> {
        static STUB: OnceLock<Option<(tempfile::TempDir, PathBuf)>> = OnceLock::new();
        let made = STUB.get_or_init(|| {
            let python = crate::which::find_in_path("python3")?;
            let dir = tempfile::tempdir().ok()?;
            let module = dir.path().join("winrm");
            std::fs::create_dir(&module).ok()?;
            std::fs::write(module.join("__init__.py"), "").ok()?;
            std::fs::write(
                module.join("exceptions.py"),
                "class WinRMOperationTimeoutError(Exception):\n    pass\n",
            )
            .ok()?;
            std::fs::write(
                module.join("protocol.py"),
                r#"
import base64, time
from winrm.exceptions import WinRMOperationTimeoutError
class Protocol:
    def __init__(self, endpoint, transport, username, password, server_cert_validation, read_timeout_sec, operation_timeout_sec):
        if password != "s3cret":
            raise Exception("401 unauthorized")
        self.info = "%s %s %s" % (endpoint, username, server_cert_validation)
        self.data = ""
        self.closed_file = None
    def open_shell(self):
        return "shell"
    def run_command(self, shell, command, args):
        self.command = command
        return "cmd"
    def send_command_input(self, shell, cmd, text, end=False):
        self.data += text
    def get_command_output_raw(self, shell, cmd):
        script = base64.b64decode(self.data).decode()
        if script.startswith("slow:"):
            self.closed_file = script[5:].strip()
            time.sleep(0.05)
            raise WinRMOperationTimeoutError()
        code = int(script[5:]) if script.startswith("exit:") else 0
        return (("%s|%s|%s\n" % (self.command, self.info, script)).encode(), b"", code, True)
    def cleanup_command(self, shell, cmd):
        pass
    def close_shell(self, shell):
        if self.closed_file:
            open(self.closed_file, "w").close()
"#,
            )
            .ok()?;
            let path = dir.path().join("python3");
            std::fs::write(
                &path,
                format!(
                    "#!/bin/sh\nPYTHONPATH='{}' exec '{}' \"$@\"\n",
                    dir.path().display(),
                    python.display()
                ),
            )
            .ok()?;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).ok()?;
            for _ in 0..200 {
                if Command::new(&path).arg("-c").arg("pass").status().is_ok() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Some((dir, path))
        });
        made.as_ref().map(|(_, path)| path.as_path())
    }

    fn windows(password: &str) -> RemoteHost {
        RemoteHost {
            name: "win".to_string(),
            transport: ScriptTransport::WinRm(WinRmTarget {
                host: "win.lan".to_string(),
                port: 5985,
                account: "CORP\\alice".to_string(),
                password: Some(SecretString::from(password.to_string())),
                use_ssl: false,
                accept_invalid_certs: false,
                tunnel: None,
            }),
        }
    }

    #[test]
    fn a_powershell_script_runs_over_winrm() {
        if python_stub().is_none() {
            return;
        }
        let events = run(&[windows("s3cret")], &RunOptions::default(), "Get-Date");
        assert_eq!(
            lines(&events, 0),
            ["powershell|http://win.lan:5985/wsman CORP\\alice validate|Get-Date"]
        );
        assert_eq!(outcome(&events, 0), ScriptOutcome::Succeeded);
    }

    #[test]
    fn a_powershell_exit_code_is_reported() {
        if python_stub().is_none() {
            return;
        }
        let events = run(&[windows("s3cret")], &RunOptions::default(), "exit:7");
        assert_eq!(
            outcome(&events, 0),
            ScriptOutcome::ScriptFailed { code: Some(7) }
        );
        let events = run(&[windows("s3cret")], &RunOptions::default(), "exit:255");
        assert_eq!(
            outcome(&events, 0),
            ScriptOutcome::ScriptFailed { code: Some(255) }
        );
    }

    #[test]
    fn a_rejected_winrm_login_is_a_connection_failure() {
        if python_stub().is_none() {
            return;
        }
        let events = run(&[windows("wrong")], &RunOptions::default(), "x");
        assert_eq!(outcome(&events, 0), ScriptOutcome::ConnectionFailed);
        let text = lines(&events, 0).join(" ");
        assert!(text.contains("401"), "{text}");
        assert!(
            !text.contains("wrong"),
            "the password stays out of the output"
        );
    }

    #[test]
    fn a_winrm_host_with_no_password_is_not_started() {
        let mut host = windows("x");
        if let ScriptTransport::WinRm(target) = &mut host.transport {
            target.password = None;
        }
        let events = run(&[host], &RunOptions::default(), "x");
        assert_eq!(outcome(&events, 0), ScriptOutcome::NotStarted);
        // Without python3 that is found first.
        let expected = if python_stub().is_some() {
            LaunchError::NoWinRmPassword
        } else {
            LaunchError::PythonNotFound
        };
        assert_eq!(not_started_reason(&events, 0), Some(expected));
    }

    #[test]
    fn the_winrm_probe_accepts_a_python_with_winrm() {
        let Some(python) = python_stub() else {
            return;
        };
        assert_eq!(probe_winrm(python.to_path_buf()), Ok(python.to_path_buf()));
    }

    #[test]
    fn the_winrm_probe_refuses_a_python_without_winrm() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("python3");
        std::fs::write(&path, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Another test forking while the file was open for writing makes the
        // first exec fail with ETXTBSY, which the probe would let pass.
        for _ in 0..200 {
            if Command::new(&path).stdin(Stdio::null()).status().is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(probe_winrm(path), Err(LaunchError::PyWinRmNotInstalled));
    }

    #[test]
    fn stopping_a_winrm_run_closes_the_remote_shell() {
        if python_stub().is_none() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("closed");
        let options = RunOptions {
            timeout: Duration::from_millis(400),
            ..RunOptions::default()
        };
        let script = format!("slow:{}", marker.display());
        let events = run(&[windows("s3cret")], &options, &script);
        assert_eq!(outcome(&events, 0), ScriptOutcome::TimedOut);
        assert!(marker.exists(), "the shell must be closed before giving up");
    }

    #[test]
    fn debug_output_of_a_winrm_target_holds_no_password() {
        let host = windows("hunter2");
        assert!(!format!("{host:?}").contains("hunter2"));
    }

    #[test]
    fn the_file_extension_picks_the_script_kind() {
        assert_eq!(
            ScriptKind::of_path(Path::new("a.PS1")),
            ScriptKind::PowerShell
        );
        assert_eq!(ScriptKind::of_path(Path::new("a.sh")), ScriptKind::Bash);
        assert_eq!(ScriptKind::of_path(Path::new("a")), ScriptKind::Bash);
    }

    fn host(dest: &str) -> RemoteHost {
        RemoteHost {
            name: dest.to_string(),
            transport: ScriptTransport::Ssh {
                target: SshTarget {
                    host: dest.to_string(),
                    port: 22,
                    username: None,
                    session_args: Vec::new(),
                    known_hosts_file: None,
                },
                credentials: DeployCredentials::default(),
            },
        }
    }

    fn run(hosts: &[RemoteHost], options: &RunOptions, script: &str) -> Vec<ScriptEvent> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let collected = Arc::clone(&events);
        let sink: ScriptEventSink = Arc::new(move |e| collected.lock().unwrap().push(e));
        let programs = Programs {
            ssh: Some(stub().to_path_buf()),
            python: python_stub()
                .map(Path::to_path_buf)
                .ok_or(LaunchError::PythonNotFound),
        };
        run_script_with(
            &programs,
            &Arc::new(script.as_bytes().to_vec()),
            hosts,
            options,
            &sink,
        );
        events.lock().unwrap().clone()
    }

    fn outcome(events: &[ScriptEvent], host: usize) -> ScriptOutcome {
        events
            .iter()
            .find_map(|e| match e {
                ScriptEvent::Finished { host: h, outcome } if *h == host => Some(*outcome),
                _ => None,
            })
            .expect("every host finishes")
    }

    fn not_started_reason(events: &[ScriptEvent], host: usize) -> Option<LaunchError> {
        events.iter().find_map(|e| match e {
            ScriptEvent::NotStarted { host: h, reason } if *h == host => Some(reason.clone()),
            _ => None,
        })
    }

    fn lines(events: &[ScriptEvent], host: usize) -> Vec<String> {
        let mut lines: Vec<String> = events
            .iter()
            .filter_map(|e| match e {
                ScriptEvent::Output { host: h, line, .. } if *h == host => Some(line.clone()),
                _ => None,
            })
            .collect();
        lines.sort();
        lines
    }

    #[test]
    fn the_script_reaches_the_host_on_stdin() {
        let events = run(
            &[host("echo-script")],
            &RunOptions::default(),
            "line1\nline2\n",
        );
        assert_eq!(lines(&events, 0), ["line1", "line2"]);
        assert_eq!(outcome(&events, 0), ScriptOutcome::Succeeded);
    }

    #[test]
    fn both_streams_are_reported() {
        let events = run(&[host("both")], &RunOptions::default(), "x");
        assert_eq!(lines(&events, 0), ["err", "out"]);
    }

    #[test]
    fn each_host_gets_its_own_outcome() {
        let hosts = [host("ok"), host("fail"), host("down")];
        let events = run(&hosts, &RunOptions::default(), "x");
        assert_eq!(outcome(&events, 0), ScriptOutcome::Succeeded);
        assert_eq!(
            outcome(&events, 1),
            ScriptOutcome::ScriptFailed { code: Some(3) }
        );
        assert_eq!(outcome(&events, 2), ScriptOutcome::ConnectionFailed);
    }

    #[test]
    fn a_script_exiting_with_255_is_not_a_connection_failure() {
        let events = run(&[host("script255")], &RunOptions::default(), "x");
        assert_eq!(
            outcome(&events, 0),
            ScriptOutcome::ScriptFailed { code: Some(255) }
        );
        assert!(lines(&events, 0).is_empty(), "the marker stays hidden");
    }

    #[test]
    fn a_host_over_its_time_limit_is_stopped() {
        let options = RunOptions {
            timeout: Duration::from_millis(100),
            ..RunOptions::default()
        };
        let started = Instant::now();
        let events = run(&[host("slow")], &options, "x");
        assert_eq!(outcome(&events, 0), ScriptOutcome::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(10));
    }

    #[test]
    fn a_cancelled_run_starts_nothing_more() {
        let options = RunOptions::default();
        options.cancel.store(true, Ordering::Relaxed);
        let events = run(&[host("ok"), host("ok")], &options, "x");
        assert_eq!(outcome(&events, 0), ScriptOutcome::Cancelled);
        assert_eq!(outcome(&events, 1), ScriptOutcome::Cancelled);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, ScriptEvent::Started { .. }))
        );
    }

    #[test]
    fn an_invalid_target_is_not_started() {
        let events = run(&[host("-oProxyCommand=evil")], &RunOptions::default(), "x");
        assert_eq!(outcome(&events, 0), ScriptOutcome::NotStarted);
        assert!(
            matches!(
                not_started_reason(&events, 0),
                Some(LaunchError::InvalidTarget { .. })
            ),
            "{events:?}"
        );
    }

    /// The marker is written out in shell and in Python; all three spellings
    /// must give the bytes the reader looks for.
    #[test]
    fn the_exit_255_marker_is_spelt_the_same_everywhere() {
        assert_eq!(SCRIPT_EXIT_255_MARKER, b"\x1erustconn-script-exit-255");
        assert!(REMOTE_COMMAND.contains(r"\036rustconn-script-exit-255\n"));
        assert!(WINRM_HELPER.contains(r#""\x1erustconn-script-exit-255\n""#));
    }

    #[test]
    fn terminal_escapes_are_removed_from_output() {
        let events = run(&[host("escape")], &RunOptions::default(), "x");
        assert_eq!(lines(&events, 0), ["ab"]);
        let events = run(&[host("color")], &RunOptions::default(), "x");
        assert_eq!(lines(&events, 0), ["ab"]);
    }

    #[test]
    fn output_past_the_limit_is_dropped_once() {
        let events = run(&[host("flood")], &RunOptions::default(), "x");
        let truncated = events
            .iter()
            .filter(|e| matches!(e, ScriptEvent::OutputTruncated { .. }))
            .count();
        assert_eq!(truncated, 1);
        assert_eq!(outcome(&events, 0), ScriptOutcome::Succeeded);
    }

    /// Starts [`REMOTE_COMMAND`] the way a remote login shell would, with the
    /// script sent and stdin still open, as during a run.
    fn start_remote(script: &str) -> Option<(Child, std::process::ChildStdin)> {
        crate::which::find_in_path("bash")?;
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(REMOTE_COMMAND)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        let mut stdin = child.stdin.take()?;
        stdin.write_all(script.as_bytes()).ok()?;
        stdin.write_all(&[0]).ok()?;
        Some((child, stdin))
    }

    fn remote(script: &str) -> Option<std::process::Output> {
        let (child, stdin) = start_remote(script)?;
        let output = child.wait_with_output().ok();
        drop(stdin);
        output
    }

    #[test]
    fn the_remote_command_keeps_the_exit_status() {
        let Some(output) = remote("echo hi\nexit 7\n") else {
            return;
        };
        assert_eq!(output.status.code(), Some(7));
        assert_eq!(output.stdout, b"hi\n");
    }

    #[test]
    fn a_command_reading_stdin_cannot_eat_the_script() {
        let Some(output) = remote("cat\necho after\n") else {
            return;
        };
        assert_eq!(output.stdout, b"after\n");
    }

    #[test]
    fn the_winrm_preamble_never_reallocates() {
        // Control characters are the worst case for JSON escaping.
        let worst = "\u{1}".repeat(300);
        for password in ["hunter2", "quo\"te\\back", "пароль", worst.as_str()] {
            let header = WinRmHeader {
                endpoint: "https://[fe80::1]:5986/wsman",
                username: "CORP\\alice",
                password,
                certificates: "validate",
            };
            let preamble = winrm_preamble(&header).unwrap();
            assert_eq!(preamble.capacity(), winrm_preamble_capacity(&header));
            assert_eq!(preamble.last(), Some(&b'\n'));
            let parsed: serde_json::Value = serde_json::from_slice(&preamble).unwrap();
            assert_eq!(parsed["password"], password);
        }
    }

    #[test]
    fn the_remote_command_needs_no_temporary_file() {
        // A here-document or here-string can land in a file on the host.
        assert!(!REMOTE_COMMAND.contains("<<"));
    }

    #[test]
    fn a_syntax_error_runs_nothing() {
        let Some(output) = remote("echo before\nif then\n") else {
            return;
        };
        assert_eq!(output.stdout, b"");
        assert_ne!(output.status.code(), Some(0));
    }

    #[test]
    fn the_remote_command_marks_a_status_of_255() {
        let Some(output) = remote("exit 255\n") else {
            return;
        };
        assert_eq!(output.status.code(), Some(255));
        assert_eq!(
            output.stderr.strip_suffix(b"\n"),
            Some(SCRIPT_EXIT_255_MARKER)
        );
    }

    #[test]
    fn a_script_of_the_largest_size_runs() {
        let filler = "#".repeat(usize::try_from(MAX_SCRIPT_BYTES).unwrap() - 32);
        let Some(output) = remote(&format!("{filler}\necho big\n")) else {
            return;
        };
        assert_eq!(output.stdout, b"big\n");
    }

    /// Closing stdin is what a dropped connection does to the remote command.
    fn assert_stopped_on_end_of_input(preamble: &str) {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("still-ran");
        let script = format!(
            "{preamble}\necho started\nsleep 3\ntouch '{}'\n",
            marker.display()
        );
        let Some((mut child, stdin)) = start_remote(&script) else {
            return;
        };
        let mut started = String::new();
        BufReader::new(child.stdout.take().unwrap())
            .read_line(&mut started)
            .unwrap();
        assert_eq!(started, "started\n");
        drop(stdin);
        let _ = child.wait();
        std::thread::sleep(Duration::from_secs(4));
        assert!(!marker.exists(), "the script kept running");
    }

    #[test]
    fn losing_the_connection_stops_the_script() {
        assert_stopped_on_end_of_input("");
    }

    #[test]
    fn a_script_ignoring_term_is_still_stopped() {
        assert_stopped_on_end_of_input("trap '' TERM");
    }

    #[test]
    fn scripts_are_validated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.sh");
        std::fs::write(&path, "echo hi\n").unwrap();
        assert_eq!(load_script(&path).unwrap(), b"echo hi\n");

        std::fs::write(&path, "  \n\t").unwrap();
        assert!(matches!(load_script(&path), Err(ScriptFileError::Empty)));
        std::fs::write(&path, b"a\0b").unwrap();
        assert!(matches!(load_script(&path), Err(ScriptFileError::Binary)));
        assert!(matches!(
            load_script(dir.path()),
            Err(ScriptFileError::NotAFile)
        ));
        assert!(matches!(
            load_script(&dir.path().join("missing")),
            Err(ScriptFileError::Unreadable(_))
        ));
        std::fs::write(&path, vec![b'a'; 1024 * 1024 + 1]).unwrap();
        assert!(matches!(
            load_script(&path),
            Err(ScriptFileError::TooLarge { .. })
        ));
    }

    #[test]
    fn a_group_selects_its_subgroups_and_only_ssh_hosts() {
        let parent = ConnectionGroup::new("parent".to_string());
        let mut child = ConnectionGroup::new("child".to_string());
        child.parent_id = Some(parent.id);
        let mut b = Connection::new_ssh("b".to_string(), "b".to_string(), 22);
        b.group_id = Some(parent.id);
        let mut a = Connection::new_ssh("a".to_string(), "a".to_string(), 22);
        a.group_id = Some(child.id);
        let other = Connection::new_ssh("other".to_string(), "o".to_string(), 22);
        let mut telnet = Connection::new_ssh("t".to_string(), "t".to_string(), 23);
        telnet.protocol = ProtocolType::Telnet;
        telnet.group_id = Some(parent.id);

        let connections = [b, a, other, telnet];
        let groups = [parent.clone(), child];
        let targets = script_targets(&[parent.id], &connections, &groups, ScriptKind::Bash);
        let names: Vec<&str> = targets.hosts.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(targets.skipped, 1);
    }

    #[test]
    fn a_connection_can_be_selected_directly() {
        let conn = Connection::new_ssh("a".to_string(), "a".to_string(), 22);
        let id = conn.id;
        let connections = [conn];
        let targets = script_targets(&[id], &connections, &[], ScriptKind::Bash);
        assert_eq!(targets.hosts.len(), 1);
        let targets = script_targets(&[id], &connections, &[], ScriptKind::PowerShell);
        assert_eq!((targets.hosts.len(), targets.skipped), (0, 1));
    }

    #[test]
    fn a_winrm_tunnel_requires_a_known_jump_host_key() {
        let connection = Connection::new(
            "win".to_string(),
            "win.lan".to_string(),
            5985,
            ProtocolConfig::WinRm(crate::models::WinRmConfig::default()),
        );
        let tunnel = SshTunnelParams {
            jump_host: "jump".to_string(),
            jump_port: 22,
            remote_host: "win.lan".to_string(),
            remote_port: 5985,
            identity_file: None,
            password: None,
            extra_args: vec!["-o".to_string(), "StrictHostKeyChecking=no".to_string()],
            launcher: ssh_tunnel::TunnelLauncher::OpenSsh,
        };
        let target = WinRmTarget::for_connection(&connection, None, Some(tunnel)).unwrap();
        assert_eq!(
            target.tunnel.unwrap().extra_args,
            [
                "-o",
                "StrictHostKeyChecking=yes",
                "-o",
                "StrictHostKeyChecking=no"
            ]
        );
    }
}
