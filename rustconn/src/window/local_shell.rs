//! Local shell tab creation

use super::*;

impl MainWindow {
    /// Opens a local shell terminal with split view integration
    ///
    /// Returns the id of the new session, so the caller that has to reapply
    /// snapshot state to the tab — the session restore path — can do so without
    /// a parallel parameter every other caller would have to answer for.
    pub(super) fn open_local_shell_with_split(
        notebook: &SharedNotebook,
        split_view: &SharedSplitView,
        state: Option<&SharedAppState>,
        label: Option<&str>,
        group: Option<&str>,
    ) -> Uuid {
        let session_id = Self::spawn_local_shell(notebook, state, label);

        // Reapply the tab group the session carried when the snapshot was taken,
        // so a grouped local shell restores grouped instead of ungrouped. The id
        // is known synchronously here because a local shell starts immediately.
        if let Some(group) = group.filter(|g| !g.is_empty()) {
            notebook.set_tab_group(session_id, group);
        }

        // Per spec: New connections ALWAYS create independent Root_Tabs
        // Register session for potential drag-and-drop, but don't show in split pane
        if let Some(info) = notebook.get_session_info(session_id) {
            // The display widget is resolved on demand via the content provider.
            split_view.add_session(info);
        }

        // Hide split view, show TabView content for the new tab
        split_view.widget().set_visible(false);
        split_view.widget().set_vexpand(false);
        notebook.widget().set_vexpand(true);
        notebook.show_tab_view_content();

        // Note: The switch_page signal handler will handle visibility
        // based on whether the session has a split_color assigned

        session_id
    }

    /// Creates a local shell session and starts the user's shell in it.
    ///
    /// `label` is the title of the new tab; `None` titles it with the default.
    /// The restore path passes the label the tab carried when the snapshot was
    /// taken, which is what carries a user's own name across a restart.
    ///
    /// Returns the new session id. Nothing beyond the tab and the child process
    /// is touched — no split-view visibility, no tab switching — because the
    /// caller decides where the session belongs: [`Self::open_local_shell_with_split`]
    /// leaves it in its own tab, while the empty split panel's Local Shell button
    /// moves it straight into that panel.
    pub(crate) fn spawn_local_shell(
        notebook: &SharedNotebook,
        state: Option<&SharedAppState>,
        label: Option<&str>,
    ) -> Uuid {
        let title = local_shell_label(label.unwrap_or_default());

        // Get terminal settings from state if available
        let terminal_settings = state
            .and_then(|s| s.try_borrow().ok())
            .map(|s| s.settings().terminal.clone())
            .unwrap_or_default();

        let session_id = notebook.create_terminal_tab_with_settings(
            Uuid::nil(),
            title,
            LOCAL_SHELL_PROTOCOL,
            None,
            &terminal_settings,
            None,
            &[],
        );

        Self::start_local_shell_process(notebook, session_id, &terminal_settings);

        // A shell that exits leaves a tab that takes no input. Mark it ended
        // and show the banner every other terminal session gets, so the tab
        // can be closed with Q or the shell started again with R (or the
        // Reconnect button) instead of sitting there dead. With "Close tab on
        // clean exit" on, a clean exit (`exit`, Ctrl+D) closes the tab instead,
        // exactly as it does for a connection.
        let close_on_clean_exit = terminal_settings.close_on_clean_exit;
        let notebook_weak = Rc::downgrade(notebook);
        notebook.connect_child_exited(session_id, ChildExitHook::Disconnect, move |exit_status| {
            let Some(notebook) = notebook_weak.upgrade() else {
                return;
            };
            let is_failure = session_lifecycle::child_exit_is_failure(exit_status);
            tracing::info!(%session_id, exit_status, is_failure, "local shell exited");
            // Deferred like the connection disconnect path: resetting VTE or
            // changing the widget tree inside `child-exited` can race the
            // pending GTK snapshot (#171).
            glib::idle_add_local_once(move || {
                // Closed by the user, or ended by the quit path.
                if crate::app::is_shutting_down() || notebook.get_session_info(session_id).is_none()
                {
                    return;
                }
                // `close_tab` closes a tab, a detached window or a split
                // guest (whose pane goes with it), exactly as a connection's
                // clean exit does.
                if close_on_clean_exit && !is_failure {
                    notebook.close_tab(session_id);
                    return;
                }
                notebook.mark_tab_disconnected(session_id);
                notebook.show_reconnect_overlay(session_id);
            });
        });

        session_id
    }

    /// Starts the shell process of a local shell session in its terminal.
    ///
    /// Used when the session is created and again when an ended one is
    /// restarted in place, so both run exactly the same command.
    pub(super) fn start_local_shell_process(
        notebook: &SharedNotebook,
        session_id: Uuid,
        terminal_settings: &rustconn_core::config::TerminalSettings,
    ) {
        // Get user's default shell
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/bash".to_string());

        // Check if a custom local shell command is configured
        let custom_command = if terminal_settings.local_shell_command.is_empty() {
            None
        } else {
            Some(terminal_settings.local_shell_command.clone())
        };

        // In Flatpak, spawn the shell on the host via flatpak-spawn so the
        // user gets their full system shell with all tools and dotfiles (#122).
        //
        // The session runs on a PTY of our own (see `pty_spawn::spawn_on_pty`).
        // `flatpak-spawn --host` with the Development interface forwards
        // stdin/stdout/stderr — the PTY fd included — to the host process, but
        // the host shell needs a controlling terminal of its own for job
        // control, which is what `script` provides. That second PTY is why the
        // spawn waits for the widget's allocation; see
        // `spawn_host_shell_when_allocated`.
        //
        // $SHELL inside the sandbox is /bin/sh, not the user's host shell.
        // Query the host $SHELL first, then exec into it.
        if rustconn_core::flatpak::is_flatpak() {
            let host_shell = std::process::Command::new("flatpak-spawn")
                .args(["--host", "sh", "-c", "echo $SHELL"])
                .output()
                .ok()
                .and_then(|out| {
                    if out.status.success() {
                        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
                        if s.is_empty() { None } else { Some(s) }
                    } else {
                        None
                    }
                })
                .unwrap_or_else(|| "/bin/bash".to_string());

            // If a custom command is set, run it via the host shell.
            // Otherwise use the default login shell behavior.
            let inner = if let Some(ref cmd) = custom_command {
                let escaped_cmd = cmd.replace('\'', "'\\''");
                format!("{host_shell} -c '{escaped_cmd}'")
            } else {
                format!("{host_shell} --login")
            };

            // `script` (util-linux) allocates a real PTY on the host, which is
            // what gives the shell job control — Ctrl-Z, fg, bg. It is not
            // present on every host, and calling it unconditionally is issue
            // [#306]: the Local Shell button failed outright with `Failed to
            // start command: script` on a Fedora 44 host. Fedora moved the
            // binary out of `util-linux` into its own `util-linux-script`
            // package in F42, so a host with `util-linux-core` installed — which
            // is what a minimal install has — does not have it, and the advice
            // to install `util-linux-core` does not help.
            //
            // So probe the host and fall back to running the shell directly. Job
            // control is lost in the fallback, which is worth saying out loud
            // because it is a real downgrade; a shell that opens without Ctrl-Z
            // still beats a button that does nothing.
            //
            // The same probe already existed one module away, in
            // `protocols.rs`'s Generic-command path, and was not applied here —
            // the two copies had drifted, which is why the Generic path survived
            // a missing `script` and this one did not.
            //
            // The command travels as a positional parameter (`$1`) and *not*
            // interpolated into the runner string. That is the whole reason this
            // shape was chosen, and it is worth stating because the obvious
            // alternative is broken in a way that only shows up on one input.
            // Interpolating needs two different quoting depths — `script -c`
            // takes the command as a single argument, the fallback `exec` takes
            // it bare — and an implementation that escapes once and substitutes
            // twice looks right until a custom command contains a single quote.
            // Measured on `printf 'it works\n'` as the custom command: the
            // interpolating version fails with `script: unexpected number of
            // arguments` when `script` is present, and produces nothing at all
            // when it is absent. With `$1` there is one quoting layer, applied
            // in one place, and both branches run the command unchanged.
            //
            // [#306]: https://github.com/totoshko88/RustConn/issues/306
            let inner_escaped = inner.replace('\'', "'\\''");
            let host_runner = "if command -v script >/dev/null 2>&1; then exec script -qfc \"$1\" /dev/null; else exec sh -c \"$1\"; fi";
            let spawn_cmd = format!(
                "flatpak-spawn --host --env=TERM=xterm-256color -- sh -c '{host_runner}' rustconn '{inner_escaped}'"
            );
            Self::spawn_host_shell_when_allocated(notebook, session_id, spawn_cmd);
        } else if let Some(ref cmd) = custom_command {
            // Custom command: run via user's shell with -c
            notebook.spawn_command(session_id, &[&shell, "-c", cmd], None, None, None);
        } else {
            // On macOS, launch as login shell so .zprofile/.zshrc are sourced
            // and the shell gets a proper controlling terminal.
            #[cfg(target_os = "macos")]
            notebook.spawn_command(session_id, &[&shell, "--login"], None, None, None);
            #[cfg(not(target_os = "macos"))]
            notebook.spawn_command(session_id, &[&shell], None, None, None);
        }
    }

    /// Starts the Flatpak host shell once its terminal widget has a size.
    ///
    /// `flatpak-spawn --host` hands this session's PTY to the host, where
    /// `script` creates a *second* PTY and copies the window size from its own
    /// stdin — once, at startup, and never again. Spawning before the widget has
    /// been allocated therefore freezes the host PTY at VTE's 24×80 default: the
    /// later `TIOCSWINSZ` from `TerminalNotebook::watch_grid_size` reaches the
    /// sandbox PTY only, and the `SIGWINCH` it raises goes to the sandbox
    /// foreground process group — `flatpak-spawn`, which forwards `SIGHUP`,
    /// `SIGINT`, `SIGQUIT`, `SIGTERM`, `SIGCONT`, `SIGTSTP`, `SIGUSR1` and
    /// `SIGUSR2` to the host, and not `SIGWINCH`. Programs that read their
    /// geometry once at startup (`mc`, `htop`, shells) then draw at the wrong
    /// size (issue #294).
    ///
    /// Waiting for the widget's grid to *settle* is the whole fix: the spawn
    /// path's `grid_size` reports the real grid from then on and `script`
    /// inherits it. "Settled" means a non-zero pixel allocation plus two
    /// consecutive ticks reporting the same row/column count — a plain non-zero
    /// allocation is not enough, because it arrives a frame before VTE
    /// recomputes the grid for it, so a tab opened while the window is a
    /// different size than the last one would otherwise spawn on the first tick
    /// with the previous grid still in place (issue #294). The predicate is the
    /// allocation rather than "is the grid still 24×80", because a window
    /// genuinely that size is a valid state and not a symptom.
    ///
    /// There is deliberately no resize forwarding beside this. The
    /// `flatpak-spawn --host -- stty rows R cols C` that used to sit here ran
    /// `stty` against *its own* stdin, which is the stdin `RustConn` itself was
    /// started with — `/dev/null` under a desktop launcher, the user's own
    /// terminal when started from a shell, never the session's PTY. It could
    /// only fail silently or resize the wrong terminal, and it did both without
    /// anyone noticing because the exit status was discarded. A mid-session
    /// resize consequently does not reach the Flatpak host shell.
    pub(super) fn spawn_host_shell_when_allocated(
        notebook: &SharedNotebook,
        session_id: Uuid,
        spawn_cmd: String,
    ) {
        /// Re-check interval. GTK lays out on the frame clock, so the first tick
        /// normally already sees an allocation; 20 ms is short enough that the
        /// later shell start is imperceptible.
        const POLL: std::time::Duration = std::time::Duration::from_millis(20);
        /// Ceiling on the wait, so ~1 s. A terminal that never gets an
        /// allocation — a tab in a window the compositor never maps — must still
        /// get its shell, at whatever size VTE reports.
        const POLL_LIMIT: u32 = 50;

        let notebook = Rc::downgrade(notebook);
        let mut ticks = 0_u32;
        // The grid from the previous tick. `script` copies the window size once
        // and never again, so it must inherit the *settled* grid, not a
        // transitional one. Issue #294: a non-zero pixel allocation
        // (`width()/height() > 0`) arrives a frame before VTE recomputes its
        // row/column count for that allocation, so a new tab opened while the
        // window is at a different size than the last one fired the spawn on the
        // first tick (`ticks=1`) with the *previous* grid still in place — the
        // 18×77 the user saw. Waiting until two consecutive ticks report the
        // same non-zero grid means the layout has come to rest before the host
        // shell reads it. VTE reports its own 24×80 default while unallocated,
        // which is itself a stable pair, so the pixel-allocation guard below is
        // still required to tell "settled at 24×80" from "not laid out yet".
        let mut last_grid: Option<(u16, u16)> = None;
        glib::timeout_add_local(POLL, move || {
            let Some(notebook) = notebook.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let Some(terminal) = notebook.get_terminal(session_id) else {
                // The tab was closed before the shell started.
                return glib::ControlFlow::Break;
            };
            ticks += 1;
            let allocated = terminal.width() > 0 && terminal.height() > 0;
            let grid = (
                u16::try_from(terminal.row_count().max(0)).unwrap_or(u16::MAX),
                u16::try_from(terminal.column_count().max(0)).unwrap_or(u16::MAX),
            );
            let settled = allocated && last_grid == Some(grid);
            last_grid = Some(grid);
            if !settled && ticks < POLL_LIMIT {
                return glib::ControlFlow::Continue;
            }
            if !allocated {
                tracing::warn!(
                    %session_id,
                    "Terminal never received an allocation; starting the host shell at VTE's default size"
                );
            } else if !settled {
                tracing::warn!(
                    %session_id,
                    "Terminal grid never settled; starting the host shell at the last reported size"
                );
            }
            tracing::debug!(
                %session_id,
                rows = grid.0,
                cols = grid.1,
                ticks,
                "Starting the Flatpak host shell"
            );
            notebook.spawn_command(session_id, &["/bin/sh", "-c", &spawn_cmd], None, None, None);
            glib::ControlFlow::Break
        });
    }
}
