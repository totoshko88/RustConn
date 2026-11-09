//! Runs a local bash script on many SSH hosts.
//!
//! The script travels over stdin and nothing is written to the server. It is
//! read whole before any of it runs, see [`REMOTE_COMMAND`]. Each host goes
//! through the same control channel as SSH key setup
//! ([`crate::ssh_key_setup::remote_command_args`]): strict host key checking, no
//! forwarding, no tty, and passwords through the askpass helper only.

use std::collections::HashSet;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use thiserror::Error;
use uuid::Uuid;

use crate::models::{Connection, ConnectionGroup, ProtocolType};
use crate::ssh_key_setup::{self, DeployCredentials, SecretFiles, SshTarget};

/// The remote command that receives the script on stdin.
///
/// Reading everything first means commands inside the script see an empty stdin
/// instead of eating the rest of the script, as they do under `bash -s`. The
/// `bash -n` pass is what makes a syntax error stop the script before its first
/// line runs; `eval` alone would execute the lines ahead of the error. `eval` is
/// a builtin, so the script size is not limited by `ARG_MAX`.
pub const REMOTE_COMMAND: &str = r#"bash -c 's=$(cat); bash -n <<<"$s" && eval "$s"'"#;

/// Largest script accepted. A script is text; a bigger file is a wrong pick.
pub const MAX_SCRIPT_BYTES: u64 = 1024 * 1024;
/// Output kept per host, so a chatty script cannot exhaust memory.
pub const MAX_OUTPUT_BYTES_PER_HOST: usize = 4 * 1024 * 1024;
/// Hosts running at once unless the caller chooses.
pub const DEFAULT_PARALLEL: usize = 8;
/// Upper bound on hosts running at once.
pub const MAX_PARALLEL: usize = 64;
/// Time allowed per host unless the caller chooses.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_mins(10);

/// Longest output line delivered in one piece.
const MAX_LINE_BYTES: u64 = 64 * 1024;
/// How often a running child is checked for exit, timeout and cancellation.
const POLL_INTERVAL: Duration = Duration::from_millis(20);
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

/// The connections a selection resolves to.
#[derive(Debug, Default)]
pub struct ScriptTargets<'a> {
    /// SSH connections, sorted by name.
    pub hosts: Vec<&'a Connection>,
    /// Selected connections left out because they do not use SSH.
    pub skipped: usize,
}

/// Resolves selected connection and group ids to the SSH hosts to run on.
///
/// A group stands for every connection in it and in its subgroups.
#[must_use]
pub fn script_targets<'a>(
    selected: &[Uuid],
    connections: &'a [Connection],
    groups: &[ConnectionGroup],
) -> ScriptTargets<'a> {
    let mut group_ids: HashSet<Uuid> = selected
        .iter()
        .copied()
        .filter(|id| groups.iter().any(|g| g.id == *id))
        .collect();
    let mut pending: Vec<Uuid> = group_ids.iter().copied().collect();
    while let Some(current) = pending.pop() {
        for group in groups.iter().filter(|g| g.parent_id == Some(current)) {
            if group_ids.insert(group.id) {
                pending.push(group.id);
            }
        }
    }

    let mut targets = ScriptTargets::default();
    for connection in connections {
        let chosen = selected.contains(&connection.id)
            || connection.group_id.is_some_and(|g| group_ids.contains(&g));
        if !chosen {
            continue;
        }
        if connection.protocol == ProtocolType::Ssh {
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
pub struct ScriptHost {
    /// The display name, for reporting.
    pub name: String,
    /// Where to connect, with the connection's own `ssh` arguments.
    pub target: SshTarget,
    /// Passwords for the host and its jump hosts. Empty for key or agent login.
    pub credentials: DeployCredentials,
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
            timeout: DEFAULT_TIMEOUT,
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
    /// `ssh` exited with 255: it could not connect or log in, the connection
    /// dropped, or the script itself exited with 255.
    ConnectionFailed,
    /// The host exceeded its time limit.
    TimedOut,
    /// The run was cancelled.
    Cancelled,
    /// `ssh` could not be started for this host. An output line says why.
    NotStarted,
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
    hosts: &[ScriptHost],
    options: &RunOptions,
    on_event: &ScriptEventSink,
) {
    let Some(program) = crate::which::find_in_path("ssh") else {
        for host in 0..hosts.len() {
            on_event(ScriptEvent::Output {
                host,
                stream: OutputStream::Stderr,
                line: "ssh was not found".to_string(),
            });
            on_event(ScriptEvent::Finished {
                host,
                outcome: ScriptOutcome::NotStarted,
            });
        }
        return;
    };
    run_script_with(&program, script, hosts, options, on_event);
}

fn run_script_with(
    program: &Path,
    script: &Arc<Vec<u8>>,
    hosts: &[ScriptHost],
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
                        run_host(program, script, index, host, options, on_event)
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

fn not_started(
    host: usize,
    reason: impl std::fmt::Display,
    on_event: &ScriptEventSink,
) -> ScriptOutcome {
    on_event(ScriptEvent::Output {
        host,
        stream: OutputStream::Stderr,
        line: reason.to_string(),
    });
    ScriptOutcome::NotStarted
}

fn run_host(
    program: &Path,
    script: &Arc<Vec<u8>>,
    index: usize,
    host: &ScriptHost,
    options: &RunOptions,
    on_event: &ScriptEventSink,
) -> ScriptOutcome {
    let args = match ssh_key_setup::remote_command_args(
        &host.target,
        host.credentials.password.is_some(),
        REMOTE_COMMAND,
    ) {
        Ok(args) => args,
        Err(error) => return not_started(index, error, on_event),
    };
    // Held to the end of this function: dropping it removes the secret files.
    let mut files = SecretFiles::default();
    let mut env = match ssh_key_setup::stage_password_env(
        &host.credentials.jump_host_passwords,
        host.credentials.password.as_ref(),
        &mut files,
    ) {
        Ok(env) => env,
        Err(error) => return not_started(index, error, on_event),
    };
    ssh_key_setup::ensure_askpass_display(&mut env);

    let mut child = match Command::new(program)
        .args(&args)
        .envs(env)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) => return not_started(index, error, on_event),
    };

    if let Some(mut pipe) = child.stdin.take() {
        let script = Arc::clone(script);
        // A thread of its own: `ssh` may fill its output pipes before it has
        // read the whole script. A write error means `ssh` exited, which the
        // exit status reports.
        std::thread::spawn(move || {
            let _ = pipe.write_all(&script);
        });
    }

    let budget = Arc::new(AtomicUsize::new(0));
    let (done_tx, done_rx) = mpsc::channel();
    let mut pumps = 0;
    if let Some(pipe) = child.stdout.take() {
        pumps += 1;
        spawn_pump(
            pipe,
            OutputStream::Stdout,
            index,
            &budget,
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
            on_event,
            &done_tx,
        );
    }
    drop(done_tx);

    let outcome = wait_for(&mut child, options);

    let deadline = Instant::now() + OUTPUT_DRAIN;
    for _ in 0..pumps {
        let left = deadline.saturating_duration_since(Instant::now());
        if done_rx.recv_timeout(left).is_err() {
            break;
        }
    }
    outcome
}

/// Waits for `child`, stopping it on timeout or cancellation.
fn wait_for(child: &mut Child, options: &RunOptions) -> ScriptOutcome {
    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                return match status.code() {
                    Some(0) => ScriptOutcome::Succeeded,
                    Some(255) => ScriptOutcome::ConnectionFailed,
                    code => ScriptOutcome::ScriptFailed { code },
                };
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!(%error, "could not wait for ssh");
                stop(child);
                return ScriptOutcome::NotStarted;
            }
        }
        if options.cancel.load(Ordering::Relaxed) {
            stop(child);
            return ScriptOutcome::Cancelled;
        }
        if started.elapsed() >= options.timeout {
            stop(child);
            return ScriptOutcome::TimedOut;
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

fn stop(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn spawn_pump(
    pipe: impl Read + Send + 'static,
    stream: OutputStream,
    host: usize,
    budget: &Arc<AtomicUsize>,
    on_event: &ScriptEventSink,
    done: &mpsc::Sender<()>,
) {
    let budget = Arc::clone(budget);
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
/// ponytail: drops the ESC byte but leaves the text of a sequence, so colored
/// output shows stray `[31m`; parse CSI sequences if that proves annoying.
fn clean_line(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw)
        .chars()
        .filter(|c| *c == '\t' || !c.is_control())
        .collect()
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;
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
            let body = r#"#!/bin/sh
for last in "$@"; do dest="$prev"; prev="$last"; done
case "$dest" in
  echo-script) cat ;;
  both) cat >/dev/null; echo out; echo err >&2 ;;
  fail) cat >/dev/null; exit 3 ;;
  down) echo "ssh: connect failed" >&2; exit 255 ;;
  slow) cat >/dev/null; sleep 30 ;;
  escape) cat >/dev/null; printf 'a\033]0;x\007b\n' ;;
  flood) cat >/dev/null; yes aaaaaaaaaaaaaaaa | head -c 5000000 ;;
  *) cat >/dev/null ;;
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

    fn host(dest: &str) -> ScriptHost {
        ScriptHost {
            name: dest.to_string(),
            target: SshTarget {
                host: dest.to_string(),
                port: 22,
                username: None,
                session_args: Vec::new(),
                known_hosts_file: None,
            },
            credentials: DeployCredentials::default(),
        }
    }

    fn run(hosts: &[ScriptHost], options: &RunOptions, script: &str) -> Vec<ScriptEvent> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let collected = Arc::clone(&events);
        let sink: ScriptEventSink = Arc::new(move |e| collected.lock().unwrap().push(e));
        run_script_with(
            stub(),
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
    }

    #[test]
    fn terminal_escapes_are_removed_from_output() {
        let events = run(&[host("escape")], &RunOptions::default(), "x");
        assert_eq!(lines(&events, 0), ["a]0;xb"]);
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

    /// Runs [`REMOTE_COMMAND`] the way a remote login shell would.
    fn remote(script: &str) -> Option<std::process::Output> {
        crate::which::find_in_path("bash")?;
        let mut child = Command::new("sh")
            .arg("-c")
            .arg(REMOTE_COMMAND)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .ok()?;
        child.stdin.take()?.write_all(script.as_bytes()).ok()?;
        child.wait_with_output().ok()
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
    fn a_syntax_error_runs_nothing() {
        let Some(output) = remote("echo before\nif then\n") else {
            return;
        };
        assert_eq!(output.stdout, b"");
        assert_ne!(output.status.code(), Some(0));
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
        let targets = script_targets(&[parent.id], &connections, &groups);
        let names: Vec<&str> = targets.hosts.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["a", "b"]);
        assert_eq!(targets.skipped, 1);
    }

    #[test]
    fn a_connection_can_be_selected_directly() {
        let conn = Connection::new_ssh("a".to_string(), "a".to_string(), 22);
        let id = conn.id;
        let connections = [conn];
        assert_eq!(script_targets(&[id], &connections, &[]).hosts.len(), 1);
    }
}
