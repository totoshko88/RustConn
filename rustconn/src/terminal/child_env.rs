//! Child process environment, exit watching and cleanup of files handed to a spawned child.
//!
//! Moved out of `terminal/mod.rs`; behaviour is unchanged.

use super::*;

/// Whether an output filter's program can actually be executed.
///
/// Both shapes a filter can take — a bare command name and an absolute path — go
/// through the one lookup, because `find_in_path` checks the **executable bit**
/// and also knows about Flatpak's `/app/bin`, Snap's bundled directories and the
/// sandboxed CLI download dirs. A leading `~/` has already been expanded by
/// `PostpendCommand::filter_argv`, so it arrives here absolute.
///
/// The absolute-path case used to be checked with `Path::is_file()` alone. That
/// passes for `/etc/passwd`, which then fails at exec time with the pipeline
/// already built — losing every byte the session wrote, the exact failure this
/// gate exists to prevent, and inconsistent with the bare-name case that was
/// checked properly.
///
/// A free function rather than a method so it is testable without a GTK notebook.
pub(super) fn filter_program_is_runnable(program: &str) -> bool {
    rustconn_core::which::find_in_path(program).is_some()
}

/// Kills a child and collects it, so a failed startup leaves no zombie.
pub(super) fn reap_child(pid: u32) {
    let pid = nix::unistd::Pid::from_raw(pid as i32);
    let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGKILL);
    let _ = nix::sys::wait::waitpid(pid, None);
}

/// What is undone when a session child exits or fails to spawn.
///
/// Runs from the child watch, so it runs after the tab is closed too, unlike a
/// `child-exited` handler, which needs the terminal widget.
#[derive(Default)]
pub(crate) struct ChildCleanup {
    /// Files to remove, such as a secret file the child reads.
    pub(crate) paths: Vec<PathBuf>,
    /// Runs once, after the files are removed.
    pub(crate) on_exit: Option<Box<dyn FnOnce()>>,
}

impl From<Vec<PathBuf>> for ChildCleanup {
    fn from(paths: Vec<PathBuf>) -> Self {
        Self {
            paths,
            on_exit: None,
        }
    }
}

impl Drop for ChildCleanup {
    fn drop(&mut self) {
        for path in &self.paths {
            if let Err(error) = std::fs::remove_file(path)
                && error.kind() != std::io::ErrorKind::NotFound
            {
                tracing::warn!(%error, path = %path.display(), "Failed to remove child secret file");
            }
        }
        if let Some(on_exit) = self.on_exit.take() {
            on_exit();
        }
    }
}

/// Raises `child-exited` on a terminal when its child process ends.
///
/// `cleanup` stays owned by the child watch for the complete process lifetime
/// and runs on exit, even when the terminal is gone. Early spawn failures drop
/// the same guard before returning. VTE does not emit this signal for a child
/// it did not spawn; the GLib watch also performs `waitpid`, which is why
/// [`pty_spawn::spawn_on_pty`] deliberately leaks its `Child` handle.
pub(super) fn watch_child_exit(terminal: &Terminal, pid: u32, cleanup: ChildCleanup) {
    let terminal = terminal.downgrade();
    let mut cleanup = Some(cleanup);
    glib::child_watch_add_local(glib::Pid(pid as i32), move |_pid, status| {
        tracing::debug!(status, pid, "Session child exited");
        if let Some(terminal) = terminal.upgrade() {
            terminal.emit_by_name::<()>("child-exited", &[&status]);
        }
        drop(cleanup.take());
    });
}

/// Assembles the complete environment for a session's child process.
///
/// The parent environment is inherited so the child sees `HOME`, `DISPLAY` and
/// friends, then four things are layered on top; `envv` overrides all of them.
///
/// Both spawn paths used to carry their own copy of this, which is how the
/// macOS `SSH_ASKPASS_REQUIRE` guard (#161) ended up applied on one path only.
///
/// Entries are zeroizing because callers may pass short-lived sensitive
/// values. SSH askpass uses only owner-only secret-file paths here; password
/// bytes never enter the child environment.
pub(super) fn build_child_env(
    envv: Option<&[&str]>,
    ssh_agent_socket: Option<&str>,
) -> Vec<Zeroizing<String>> {
    /// Replaces any existing definitions of `KEY=` before appending `entry`.
    fn set(env_vec: &mut Vec<Zeroizing<String>>, entry: String) {
        if let Some(eq_pos) = entry.find('=') {
            let key_prefix = &entry[..=eq_pos];
            env_vec.retain(|existing| !existing.starts_with(key_prefix));
        }
        env_vec.push(Zeroizing::new(entry));
    }

    // PATH is replaced with the extended version so CLI tools RustConn
    // downloaded itself (Flatpak, snap) are found by name.
    let extended_path = rustconn_core::cli_download::get_extended_path();
    let mut env_vec: Vec<Zeroizing<String>> = std::env::vars()
        .map(|(key, value)| {
            Zeroizing::new(if key == "PATH" {
                format!("PATH={extended_path}")
            } else {
                format!("{key}={value}")
            })
        })
        .collect();
    if std::env::var_os("PATH").is_none() {
        env_vec.push(Zeroizing::new(format!("PATH={extended_path}")));
    }

    // SSH agent: the connection's socket wins over the Settings socket, which
    // wins over the agent RustConn started, then whatever the desktop session
    // exported (`rustconn_core::sftp::resolve_ssh_agent_socket`).
    for (key, value) in rustconn_core::sftp::agent_env_vars(ssh_agent_socket) {
        set(&mut env_vec, format!("{key}={value}"));
    }

    // Strip host SSH_ASKPASS — RustConn types passwords into the session
    // itself, so the host askpass program (e.g. ksshaskpass) is never needed
    // and may not exist inside a sandbox (#48).
    env_vec.retain(|e| !e.starts_with("SSH_ASKPASS="));

    // On macOS, ssh may still try its compiled-in askpass path (XQuartz) with
    // SSH_ASKPASS unset; OpenSSH >= 8.4 honours this instead (#161).
    #[cfg(target_os = "macos")]
    set(&mut env_vec, "SSH_ASKPASS_REQUIRE=never".to_owned());

    push_sandbox_cli_config(&mut env_vec);

    // ncurses programs (mc, htop) need TERM to detect colour and mouse
    // support; a GUI process usually has none, and a Flatpak sandbox may
    // inherit TERM=dumb. xterm-256color is universally available.
    if !env_vec.iter().any(|e| e.starts_with("TERM="))
        || rustconn_core::flatpak::is_flatpak()
        || env_vec.iter().any(|e| e.as_str() == "TERM=dumb")
    {
        set(&mut env_vec, "TERM=xterm-256color".to_owned());
    }

    if let Some(user_env) = envv {
        for entry in user_env {
            set(&mut env_vec, (*entry).to_owned());
        }
    }

    env_vec
}

/// Redirects CLI config directories to writable locations inside a sandbox.
///
/// Host directories are either mounted read-only (gcloud, Azure, kubectl) or
/// not mounted at all (Teleport, OCI). Boundary needs nothing: it reaches the
/// system keyring over D-Bus, which works in a sandbox. Cloudflare Tunnel needs
/// nothing either: `cloudflared access ssh` authenticates in a browser with
/// short-lived tokens and keeps no config.
fn push_sandbox_cli_config(env_vec: &mut Vec<Zeroizing<String>>) {
    let dirs: [(&str, Option<PathBuf>); 4] = if rustconn_core::flatpak::is_flatpak() {
        [
            (
                "CLOUDSDK_CONFIG",
                rustconn_core::flatpak::get_flatpak_gcloud_config_dir(),
            ),
            (
                "AZURE_CONFIG_DIR",
                rustconn_core::flatpak::get_flatpak_azure_config_dir(),
            ),
            (
                "TELEPORT_HOME",
                rustconn_core::flatpak::get_flatpak_teleport_config_dir(),
            ),
            (
                "OCI_CLI_CONFIG_FILE",
                rustconn_core::flatpak::get_flatpak_oci_config_dir().map(|dir| dir.join("config")),
            ),
        ]
    } else if rustconn_core::is_snap() {
        // The personal-files plugs expose host credentials read-only, so
        // writable config lives under $SNAP_USER_DATA.
        [
            (
                "CLOUDSDK_CONFIG",
                rustconn_core::snap::get_snap_gcloud_config_dir(),
            ),
            (
                "AZURE_CONFIG_DIR",
                rustconn_core::snap::get_snap_azure_config_dir(),
            ),
            (
                "TELEPORT_HOME",
                rustconn_core::snap::get_snap_teleport_config_dir(),
            ),
            (
                "OCI_CLI_CONFIG_FILE",
                rustconn_core::snap::get_snap_oci_config_dir().map(|dir| dir.join("config")),
            ),
        ]
    } else {
        return;
    };

    for (key, dir) in dirs {
        let prefix = format!("{key}=");
        // An inherited or caller-provided value is left alone.
        if let Some(dir) = dir
            && !env_vec.iter().any(|e| e.starts_with(&prefix))
        {
            env_vec.push(Zeroizing::new(format!("{prefix}{}", dir.display())));
        }
    }
}

/// The output-filter availability gate.
///
/// Extracted into a free function precisely so this can be checked without a GTK
/// notebook — the absolute-path branch used to accept any existing file.
#[cfg(test)]
mod output_filter_gate_tests {
    use super::filter_program_is_runnable;

    #[test]
    fn a_bare_command_on_path_is_runnable() {
        assert!(filter_program_is_runnable("sh"));
    }

    #[test]
    fn a_command_that_does_not_exist_is_not_runnable() {
        assert!(!filter_program_is_runnable(
            "rustconn-no-such-filter-binary"
        ));
        assert!(!filter_program_is_runnable(""));
    }

    /// The regression this gate was rewritten for: an absolute path that exists
    /// but is not executable must be refused *before* the pipeline is built.
    /// Checking `Path::is_file()` alone passed it and then lost every byte of the
    /// session at exec time.
    #[test]
    fn an_existing_but_non_executable_absolute_path_is_refused() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("not-executable");
        std::fs::write(&path, b"#!/bin/sh\necho hi\n").expect("write");

        assert!(
            !filter_program_is_runnable(path.to_str().expect("utf-8 path")),
            "a file without the executable bit is not a runnable filter"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
            assert!(
                filter_program_is_runnable(path.to_str().expect("utf-8 path")),
                "the same path becomes runnable once it is executable"
            );
        }
    }

    #[test]
    fn a_directory_is_not_a_runnable_filter() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(!filter_program_is_runnable(
            dir.path().to_str().expect("utf-8 path")
        ));
    }
}

/// The mc tab passes its wrapper `PATH` as an `envv` entry; this pins that it
/// replaces the extended `PATH` rather than sitting next to it (issue #422).
#[cfg(test)]
mod child_env_tests {
    use super::build_child_env;

    #[test]
    fn a_path_entry_from_the_caller_replaces_the_extended_path() {
        // An explicit agent socket keeps the test away from the agent lookup.
        let env = build_child_env(
            Some(&["PATH=/run/rc-mc-wrapper:/usr/bin"]),
            Some("/run/rc-test-agent.sock"),
        );
        let paths: Vec<&str> = env
            .iter()
            .map(|entry| entry.as_str())
            .filter(|entry| entry.starts_with("PATH="))
            .collect();
        assert_eq!(paths, ["PATH=/run/rc-mc-wrapper:/usr/bin"]);
    }

    /// F-01: the VTE child gets the connection's agent socket, and without one
    /// exactly what core resolves for every other launch.
    #[test]
    fn the_child_env_carries_the_resolved_agent_socket() {
        let sock = |env: &[zeroize::Zeroizing<String>]| -> Vec<String> {
            env.iter()
                .filter(|e| e.starts_with("SSH_AUTH_SOCK="))
                .map(|e| e.as_str().to_owned())
                .collect()
        };
        let own = build_child_env(None, Some("/run/rc-conn.sock"));
        assert_eq!(sock(&own), ["SSH_AUTH_SOCK=/run/rc-conn.sock"]);

        let resolved = build_child_env(None, None);
        let expected: Vec<String> = rustconn_core::sftp::agent_env_vars(None)
            .into_iter()
            .filter(|(k, _)| *k == "SSH_AUTH_SOCK")
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        // Without an agent anywhere the variable is simply absent (or the
        // inherited one), never a second entry.
        if !expected.is_empty() {
            assert_eq!(sock(&resolved), expected);
        }
        assert!(sock(&resolved).len() <= 1);
    }

    #[test]
    fn without_a_caller_path_the_child_gets_the_extended_one() {
        let env = build_child_env(None, Some("/run/rc-test-agent.sock"));
        let paths: Vec<String> = env
            .iter()
            .filter(|entry| entry.starts_with("PATH="))
            .map(|entry| entry.as_str().to_owned())
            .collect();
        assert_eq!(
            paths,
            [format!(
                "PATH={}",
                rustconn_core::cli_download::get_extended_path()
            )]
        );
    }
}
