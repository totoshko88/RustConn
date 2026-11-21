//! Output filter, input forwarding, grid-size watching and connection-output reads for a tab's PTY relay.
//!
//! Moved out of `terminal/mod.rs`; behaviour is unchanged.

use super::*;

/// Returns a terminal's grid as `(rows, columns)`, clamped to `u16`.
///
/// VTE reports these as `i64`; a terminal larger than 65535 cells in either
/// direction does not exist, and `TIOCSWINSZ` could not express it anyway.
pub(super) fn grid_size(terminal: &Terminal) -> (u16, u16) {
    let clamp = |value: i64| u16::try_from(value.max(0)).unwrap_or(u16::MAX);
    (clamp(terminal.row_count()), clamp(terminal.column_count()))
}

/// The text of the active screen's rows `first_row..=last_row`, with a
/// wrapped line joined into one.
fn rows_text(terminal: &Terminal, first_row: i64, last_row: i64) -> String {
    terminal
        .text_range_format(
            vte4::Format::Text,
            first_row,
            0,
            last_row,
            terminal.column_count(),
        )
        .0
        .map_or_else(String::new, |g| g.to_string())
}

/// [`TerminalNotebook::connection_output`] for one terminal, given the
/// session's recorded start (`None`: count from row 0).
///
/// Returns the output, or `None` while the banner has not landed, and the
/// start to record in place of `start` when this read resolved it, so the
/// banner search runs once. Kept free of the notebook so an opt-in test can
/// drive it with a bare VTE terminal.
pub(super) fn read_connection_output(
    terminal: &Terminal,
    start: Option<&output_start::OutputStart>,
) -> (Option<ConnectionOutput>, Option<output_start::OutputStart>) {
    let cursor_row = terminal.cursor_position().1;
    let Some((base, banner, resolved)) = connection_start_row(terminal, start, cursor_row) else {
        return (None, None);
    };
    // At most the last BANNER_SEARCH_ROWS rows: a detector the heuristic
    // never decides (a jump-host session whose prompt it misses) reads on
    // every change for the whole session, and copying everything from the
    // start row grew with the session. The verdict reads the last line, the
    // last prompt or question and the failures after it; a failure further
    // up than this with no prompt or question below it would need a client
    // that printed a screenful after failing, and one that fails exits.
    // `row` still counts from the start row.
    let first_row = base
        .min(cursor_row)
        .max(cursor_row - output_start::BANNER_SEARCH_ROWS);
    let text = rows_text(terminal, first_row, cursor_row);
    // Only a cursor above the start row can mean another screen, so the
    // wider read runs only then.
    let screen_replaced = cursor_row < base
        && banner.is_some_and(|last_line| {
            let first_row = (cursor_row - output_start::BANNER_SEARCH_ROWS).max(0);
            !output_start::shows_banner(&rows_text(terminal, first_row, cursor_row), &last_line)
        });
    let output = ConnectionOutput {
        row: (cursor_row - base).max(0),
        text,
        screen_replaced,
    };
    (Some(output), resolved)
}

/// The absolute row the current connection output starts on, with the
/// banner line above it when it was found by that line, and the start to
/// record when this call resolved a pending one.
///
/// Resolves a pending [`output_start::OutputStart::AfterBanner`] by finding
/// the banner in the rows above the cursor. `None` while the banner has not
/// landed.
fn connection_start_row(
    terminal: &Terminal,
    start: Option<&output_start::OutputStart>,
    cursor_row: i64,
) -> Option<(i64, Option<String>, Option<output_start::OutputStart>)> {
    let (last_line, fed_at_row) = match start {
        None => return Some((0, None, None)),
        Some(output_start::OutputStart::Row(base)) => return Some((*base, None, None)),
        Some(output_start::OutputStart::BelowBanner { row, last_line }) => {
            return Some((*row, Some(last_line.clone()), None));
        }
        Some(output_start::OutputStart::AfterBanner {
            last_line,
            fed_at_row,
        }) => (last_line, *fed_at_row),
    };
    let first_row = output_start::search_first_row(fed_at_row, cursor_row);
    let col_count = terminal.column_count();
    let rows: Vec<String> = (first_row..=cursor_row)
        .map(|row| {
            terminal
                .text_range_format(vte4::Format::Text, row, 0, row, col_count)
                .0
                .map_or_else(String::new, |g| g.to_string())
        })
        .collect();
    if let Some(row) = output_start::row_below_banner(&rows, first_row, last_line) {
        let resolved = output_start::OutputStart::BelowBanner {
            row,
            last_line: last_line.clone(),
        };
        Some((row, Some(last_line.clone()), Some(resolved)))
    } else {
        // A screenful printed since the banner: count from the window top.
        let row = output_start::base_without_banner(fed_at_row, cursor_row)?;
        Some((row, None, Some(output_start::OutputStart::Row(row))))
    }
}

impl TerminalNotebook {
    /// Registers, or clears, the output filter a session's command is piped through.
    ///
    /// Call before spawning. A filter whose executable cannot be found is
    /// refused here rather than at spawn time: the session then starts unfiltered,
    /// which is the documented fallback, where a pipeline to a missing command
    /// would start and immediately lose every byte the session wrote.
    pub fn set_output_filter(
        &self,
        session_id: Uuid,
        postpend: Option<&rustconn_core::models::PostpendCommand>,
    ) {
        let Some(argv) = postpend.and_then(rustconn_core::models::PostpendCommand::filter_argv)
        else {
            self.output_filters.borrow_mut().remove(&session_id);
            return;
        };

        // `argv[0]` is guaranteed non-empty by `filter_argv`.
        let program = argv[0].clone();
        if !filter_program_is_runnable(&program) {
            tracing::warn!(
                %session_id,
                filter = %program,
                "Output filter is not installed or not executable; session output is unfiltered"
            );
            // A log line is not feedback: the session starts and looks completely
            // normal, just unfiltered, so nothing tells the user their filter did
            // not run. A background failure the user can act on is a toast, per
            // the project's error-feedback rule.
            crate::toast::show_error_toast_on_active_window(&i18n_f(
                "Output filter '{}' is not installed; session output is not filtered",
                &[&program],
            ));
            self.output_filters.borrow_mut().remove(&session_id);
            return;
        }

        tracing::info!(%session_id, filter = %program, "Session output will be piped through a filter");
        self.output_filters.borrow_mut().insert(session_id, argv);
    }

    /// Returns the `sh -c` script for a filtered session, or `None` when unfiltered.
    ///
    /// See [`filter_program_is_runnable`] for why availability is decided before
    /// this point rather than at spawn time.
    ///
    /// ponytail: the pipeline's exit status is the *filter's*, not the session's,
    /// because POSIX `sh` has no portable `PIPESTATUS`. A filtered session that
    /// fails therefore looks like a clean exit to the reconnect heuristics. Worth
    /// revisiting only if a filter turns out to be common enough that the lost
    /// status matters.
    pub(super) fn wrap_in_output_filter(&self, session_id: Uuid, argv: &[&str]) -> Option<String> {
        let filters = self.output_filters.borrow();
        let filter = filters.get(&session_id)?;
        let filter_refs: Vec<&str> = filter.iter().map(String::as_str).collect();
        Some(rustconn_core::shell_escape::pipe_argv_through(
            argv,
            &filter_refs,
        ))
    }

    /// Feeds a session's PTY output to its terminal and to its observers.
    ///
    /// Runs on the GTK main thread for as long as the relay lives; when the
    /// relay is dropped the stream ends and this loop finishes on its own.
    pub(super) fn deliver_output_to(
        &self,
        session_id: Uuid,
        terminal: &Terminal,
        output: pty_relay::OutputStream,
    ) {
        let hold = selection_hold::SelectionHold::attach(terminal);
        let terminal = terminal.downgrade();
        let observers = Rc::clone(&self.output_observers);
        glib::spawn_future_local(async move {
            // One filter per stream, because it has to remember a sequence split
            // across a chunk boundary and its state belongs to exactly one PTY.
            let mut osc52_filter = osc52::Osc52Filter::default();
            while let Ok(chunk) = output.recv().await {
                hold.wait_while_selecting(|| terminal.upgrade().is_some_and(|t| t.has_selection()))
                    .await;
                let live_terminal = terminal.upgrade();
                // The clipboard offer is written here, inside the sink, so it
                // only fires while the terminal is alive to supply a display.
                // The length is the only part of a payload worth logging: it is
                // remote-supplied output and may be a secret the user yanked.
                let mut sink = |text: &str| {
                    tracing::debug!(
                        %session_id,
                        bytes = text.len(),
                        "OSC 52 clipboard offer from the remote side"
                    );
                    if let Some(terminal) = &live_terminal {
                        osc52::offer_to_clipboard(terminal, text);
                    }
                };
                // Run the filter on every chunk, alive widget or not — it has to
                // keep its cross-chunk state consistent. The result is owned so
                // the same bytes reach VTE *and* the observers: a lifted OSC 52
                // offer must not survive in the session transcript either, which
                // is exactly where the raw chunk used to leak it.
                let filtered = osc52_filter.push(&chunk, &mut sink).to_vec();
                if let Some(terminal) = &live_terminal {
                    // A chunk can filter to nothing while the filter holds back
                    // the start of a sequence (a lone `ESC`, an OSC string that
                    // is still open). A removed offer is not empty: it leaves a
                    // `CAN`. `feed` is not called on an empty slice: VTE has been
                    // known to reach an `assert()` on degenerate input, and an
                    // assertion in a library takes the whole process down.
                    if !filtered.is_empty() {
                        terminal.feed(&filtered);
                    }
                }
                // The list is cloned so that no borrow is held while an observer
                // runs: session logging is then free to touch the notebook.
                let handlers = observers
                    .borrow()
                    .get(&session_id)
                    .cloned()
                    .unwrap_or_default();
                for handler in handlers {
                    handler(&filtered);
                }
            }
            // The stream can end inside a sequence. A lone `ESC` or `c2` is
            // handed to VTE rather than dropped with the filter. An OSC string
            // still open depends on the setting: with OSC 52 enabled `finish`
            // replaces it with a `CAN`, so a later `BEL` or `ST` cannot
            // complete it; with it disabled the fragment is released as it
            // came.
            if let Some(terminal) = terminal.upgrade() {
                hold.detach(&terminal);
                let tail = osc52_filter.finish();
                if !tail.is_empty() {
                    terminal.feed(tail);
                }
            }
        });
    }

    /// Sends whatever the terminal reports as input to the session's PTY.
    ///
    /// Keys, pasted text, mouse reports and the replies VTE makes to terminal
    /// queries all arrive through `commit`, which VTE emits whether or not it
    /// owns a PTY. Connected once per terminal, because the handler resolves the
    /// relay by session and so keeps working across a reconnect.
    pub(super) fn forward_input_from(&self, session_id: Uuid, terminal: &Terminal) {
        if !self.commit_forwarded.borrow_mut().insert(session_id) {
            return;
        }
        let relays = Rc::clone(&self.pty_relays);
        terminal.connect_commit(move |_terminal, text, size| {
            if let Some(relay) = relays.borrow().get(&session_id) {
                relay.write_input(&pty_relay::commit_bytes(text, size));
            }
        });
    }

    /// Keeps the child's idea of the window size in step with the widget.
    ///
    /// A poll rather than a signal, because VTE has none to offer: no row-count
    /// or column-count notification, `char-size-changed` covers font metrics
    /// only, and GTK4 removed `size-allocate` (see `vte_contract_tests`). The
    /// check is two integer reads, and `TIOCSWINSZ` is only sent when the grid
    /// really changed — a redundant `SIGWINCH` makes full-screen programs
    /// repaint for nothing.
    pub(super) fn watch_grid_size(&self, session_id: Uuid, terminal: &Terminal) {
        // Issue #294: push the real window size on the first main-loop idle
        // after the session is spawned. Without this, the child starts at the
        // default 24×80 and programs that read geometry only at startup (`mc`,
        // `htop`) render at the wrong size until they are restarted.
        //
        // Best effort by construction: the idle runs at
        // `G_PRIORITY_DEFAULT_IDLE`, below GTK's frame clock, so in the common
        // case layout has already happened — but if it has not, VTE reports its
        // own 24×80 defaults rather than zero, the guard below does not catch it,
        // and `sync_size` finds nothing to change. That case falls back to the
        // 250 ms poll, which is the behaviour this replaced, so there is no
        // retry: the cost of being early is a slower fix, not a wrong size.
        {
            let terminal_weak = terminal.downgrade();
            let relays_early = Rc::clone(&self.pty_relays);
            glib::idle_add_local_once(move || {
                let Some(terminal) = terminal_weak.upgrade() else {
                    return;
                };
                let (rows, cols) = grid_size(&terminal);
                if rows == 0 || cols == 0 {
                    return;
                }
                let relays = relays_early.borrow();
                if let Some(relay) = relays.get(&session_id)
                    && relay.sync_size(rows, cols)
                {
                    tracing::debug!(
                        %session_id, rows, cols,
                        "Early size push after widget allocation"
                    );
                }
            });
        }

        let terminal = terminal.downgrade();
        let relays = Rc::clone(&self.pty_relays);
        let timers = Rc::clone(&self.pty_size_timers);
        let source = glib::timeout_add_local(GRID_SIZE_POLL, move || {
            let Some(terminal) = terminal.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let relays = relays.borrow();
            let Some(relay) = relays.get(&session_id) else {
                // No relay means the session is over, and so is this poll.
                timers.borrow_mut().remove(&session_id);
                return glib::ControlFlow::Break;
            };
            let (rows, cols) = grid_size(&terminal);
            if relay.sync_size(rows, cols) {
                tracing::debug!(%session_id, rows, cols, "Pushed new window size to child");
            }
            glib::ControlFlow::Continue
        });
        if let Some(previous) = self.pty_size_timers.borrow_mut().insert(session_id, source) {
            previous.remove();
        }
    }

    /// Renders text in the terminal widget without sending it to the session.
    ///
    /// `feed` is what VTE uses for output coming from the child; feeding bytes
    /// here draws them locally and never reaches the remote shell. That is the
    /// difference between a script printing a status line and a script typing
    /// that line at a prompt.
    pub fn feed_local_output(&self, session_id: Uuid, bytes: &[u8]) {
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.feed(bytes);
        }
    }

    /// Registers a callback for a session's raw PTY output.
    ///
    /// The callback runs on the GTK main thread with each chunk exactly as the
    /// child wrote it — no viewport, no rewrapping, no de-duplication — which is
    /// what a session transcript needs. Several observers can coexist.
    pub fn add_output_observer<F>(&self, session_id: Uuid, observer: F)
    where
        F: Fn(&[u8]) + 'static,
    {
        self.output_observers
            .borrow_mut()
            .entry(session_id)
            .or_default()
            .push(Rc::new(observer));
    }

    /// Stops a session's relay and its window-size poll.
    ///
    /// Output observers deliberately survive: an in-place reconnect keeps the
    /// same session log, and re-registering them would double every line.
    pub(super) fn teardown_relay(&self, session_id: Uuid) {
        if let Some(timer) = self.pty_size_timers.borrow_mut().remove(&session_id) {
            timer.remove();
        }
        drop(self.pty_relays.borrow_mut().remove(&session_id));
    }

    /// Ends every live session's child process, for the application-exit paths.
    ///
    /// The `close-page` handler already does this per tab (issue #172), but
    /// quitting never closes the pages: the window's `close_request` and the
    /// `app.quit` action both went straight to `Proceed`, so an open `telnet`
    /// tab left its process running after RustConn was gone
    /// (issue [#304](https://github.com/totoshko88/RustConn/issues/304)). The
    /// leak was not visible for VTE-spawned children, but RustConn owns the PTY
    /// itself now: dropping the notebook closes the master and nothing else, and
    /// `telnet` is precisely the client that ignores the resulting `SIGHUP`.
    ///
    /// Deliberately does not go through `close_page`. That path fires the
    /// session-ended callbacks, rebuilds the Welcome tab and touches the sidebar,
    /// all pointless on the way out and all of it re-entering state the caller is
    /// in the middle of persisting. This only releases what would outlive the
    /// process.
    ///
    /// Idempotent, so it is safe for more than one exit path to run — and they
    /// do: the three of them funnel through
    /// [`crate::window::shutdown_sessions_for_exit`], and the window button and
    /// Ctrl+Q can both fire for one quit.
    ///
    /// Must run *after* `flush_active_recordings`, which is why the caller owns
    /// the ordering: this clears the PTY relays, and a recorder still being
    /// flushed through one would lose its tail.
    pub fn shutdown_sessions(&self) {
        // External viewers hosted in a tab (`xfreerdp`, `vncviewer`): the same
        // omission, and the same fix as the registry's tabless ones (issue #209).
        //
        // Only their `Child` is taken, and the widget map is left alone. An
        // embedded RDP/VNC/Web widget needs `disconnect()`, not a kill, and that
        // is what the `close-page` path does — which still runs, for every
        // detached session, immediately after this. Draining the map here would
        // silently take that away. Taking the child leaves `None` behind, so the
        // `take()` in `close-page` is a no-op rather than a double kill.
        let external: Vec<_> = self
            .session_widgets
            .borrow()
            .values()
            .filter_map(|storage| match storage {
                SessionWidgetStorage::ExternalProcess(process) => Some(Rc::clone(process)),
                _ => None,
            })
            .collect();
        for process in external {
            if let Some(mut child) = process.borrow_mut().take() {
                let _ = child.kill();
                let _ = child.wait();
            }
        }

        // Stop reading before signalling, so a relay thread does not sit in a
        // read on a PTY whose child is going away.
        for (_, timer) in self.pty_size_timers.borrow_mut().drain() {
            timer.remove();
        }
        self.pty_relays.borrow_mut().clear();

        let pids: Vec<i32> = self
            .vte_child_pids
            .borrow_mut()
            .drain()
            .map(|(_, pid)| pid)
            .collect();
        if !pids.is_empty() {
            tracing::info!(
                count = pids.len(),
                "terminating session children on shutdown"
            );
        }
        child_teardown::terminate_child_groups_blocking(&pids);

        // `SshTunnel::drop` kills its ssh process. Tabless external sessions park
        // their tunnel here too (see `spawn_and_register_external_viewer`), which
        // is the only thing that ever collected them.
        self.ssh_tunnels.borrow_mut().clear();
    }

    /// What the session's current process has printed below its connect
    /// banner, for the connection status detectors.
    ///
    /// VTE's `cursor_position()` returns `(column, row)` with the row in
    /// absolute buffer coordinates — scrollback included — so the row and the
    /// text are counted from the row the current connection's output starts
    /// on: the row just below RustConn's connect banner, pinned by
    /// [`Self::display_connect_banner`]. Neither the banner nor, after an
    /// in-place reconnect, the kept scrollback counts (issue #253). See
    /// [`ConnectionOutput::screen_replaced`] for a client on the alternate
    /// screen, whose rows cannot be compared with that start row.
    ///
    /// Returns `None` when the session has no terminal or the banner has not
    /// been processed yet; callers wait for the next output.
    #[must_use]
    pub fn connection_output(&self, session_id: Uuid) -> Option<ConnectionOutput> {
        let terminal = self.get_terminal(session_id)?;
        let start = self.cursor_row_base.borrow().get(&session_id).cloned();
        let (output, resolved) = read_connection_output(&terminal, start.as_ref());
        if let Some(resolved) = resolved {
            self.cursor_row_base
                .borrow_mut()
                .insert(session_id, resolved);
        }
        output
    }

    /// Runs `decide` on [`Self::connection_output`] after every change in the
    /// session's terminal, until it returns `true`.
    ///
    /// The handler disconnects itself once `decide` returns `true`, and also,
    /// without calling it again, once an in-place reconnect has started
    /// another process in the session: a detector left armed by a failed
    /// attempt never counts a later attempt's output, and detectors do not
    /// pile up across reconnects. Closing the tab drops the terminal, and the
    /// handler with it.
    pub fn watch_connection_output<F>(self: &Rc<Self>, session_id: Uuid, decide: F)
    where
        F: Fn(Option<&ConnectionOutput>) -> bool + 'static,
    {
        let Some(terminal) = self.get_terminal(session_id) else {
            return;
        };
        let started_at = self.session_started_at(session_id);
        let notebook = Rc::downgrade(self);
        let handler: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
        let handler_in_callback = Rc::clone(&handler);
        let id = terminal.connect_contents_changed(move |terminal| {
            let done = notebook.upgrade().is_none_or(|notebook| {
                notebook.session_started_at(session_id) != started_at
                    || decide(notebook.connection_output(session_id).as_ref())
            });
            if done && let Some(id) = handler_in_callback.borrow_mut().take() {
                terminal.disconnect(id);
            }
        });
        *handler.borrow_mut() = Some(id);
    }

    /// Feeds a terminal connect's banner and pins the connection start below
    /// it.
    ///
    /// Call this, not [`Self::display_output`], for the banner every terminal
    /// connect and in-place reconnect prints before it spawns its client: from
    /// then on [`Self::connection_output`] counts only what the client prints.
    pub fn display_connect_banner(&self, session_id: Uuid, banner: &str) {
        // The row the banner starts on: VTE has not processed it yet, so the
        // cursor is still where the buffer (or the kept history) ended.
        let Some(fed_at_row) = self
            .get_terminal(session_id)
            .map(|terminal| terminal.cursor_position().1)
        else {
            return;
        };
        self.display_output(session_id, banner);
        self.cursor_row_base.borrow_mut().insert(
            session_id,
            output_start::OutputStart::AfterBanner {
                last_line: output_start::banner_last_line(banner),
                fed_at_row,
            },
        );
    }
}
