//! Session lifecycle management: logging, activity monitoring, and child-exit handling.
//!
//! Extracted from `window/mod.rs` — sets up session logging, activity
//! monitoring with notifications, and child-exited cleanup/reconnect logic.

use zeroize::Zeroizing;

use super::*;

/// Longest a trailing partial line is held before it is written anyway.
///
/// Output is written a line at a time, so a prompt — which carries no line
/// break, because the shell is waiting on the same line — would otherwise sit in
/// the buffer unrecorded. Half a second is short enough that a transcript
/// followed live shows the prompt, and long enough that a line arriving in
/// several chunks is not split across records (issue
/// [#247](https://github.com/totoshko88/RustConn/issues/247)).
const PARTIAL_LINE_GRACE: std::time::Duration = std::time::Duration::from_millis(500);

/// Writes a session's transcript from the bytes its child actually produced.
///
/// This is fed by [`crate::terminal::pty_relay`] rather than read back from the
/// widget. Scraping the widget cannot be made correct: VTE rewraps its buffer
/// when the window is resized and renumbers the rows underneath the reader, so
/// a scraped transcript repeats some lines and skips others — the contract tests
/// in `terminal::vte_contract_tests` pin that behaviour down. Reading the PTY
/// instead means the log is a copy of the session, in order, with nothing
/// inferred (issue #247).
///
/// Bytes are buffered until a line is complete, because a log file is read by
/// lines and because redaction matches patterns against them. Everything held
/// here is zeroizing: this is the byte stream a password prompt travels on.
struct TranscriptWriter {
    logger: Rc<RefCell<rustconn_core::session::SessionLogger>>,
    /// Bytes of the line currently being written by the child.
    pending: Zeroizing<Vec<u8>>,
    /// When the pending line was last added to, for [`PARTIAL_LINE_GRACE`].
    pending_since: Option<std::time::Instant>,
}

impl TranscriptWriter {
    fn new(logger: Rc<RefCell<rustconn_core::session::SessionLogger>>) -> Self {
        Self {
            logger,
            pending: Zeroizing::new(Vec::new()),
            pending_since: None,
        }
    }

    /// Accepts a chunk of PTY output and writes every complete line in it.
    fn accept(&mut self, chunk: &[u8]) {
        self.pending.extend_from_slice(chunk);

        // `\n` is the only separator worth splitting on. A bare `\r` returns the
        // cursor to the start of the same line — progress bars and spinners use
        // it — and treating it as a line break would turn one download into
        // hundreds of log lines.
        let last_break = self.pending.iter().rposition(|byte| *byte == b'\n');
        if let Some(index) = last_break {
            let complete = Zeroizing::new(self.pending[..=index].to_vec());
            self.pending = Zeroizing::new(self.pending[index + 1..].to_vec());
            self.write(&complete);
        }
        self.pending_since = (!self.pending.is_empty()).then(std::time::Instant::now);
    }

    /// Writes a partial line that has stopped growing, such as a prompt.
    fn flush_stale_partial(&mut self) {
        let Some(since) = self.pending_since else {
            return;
        };
        if since.elapsed() < PARTIAL_LINE_GRACE || self.pending.is_empty() {
            return;
        }
        let partial = std::mem::replace(&mut self.pending, Zeroizing::new(Vec::new()));
        self.pending_since = None;
        self.write(&partial);
    }

    /// Writes whatever is left, for a session whose process has exited.
    fn finish(&mut self) {
        if self.pending.is_empty() {
            return;
        }
        let partial = std::mem::replace(&mut self.pending, Zeroizing::new(Vec::new()));
        self.pending_since = None;
        self.write(&partial);
    }

    fn write(&self, data: &[u8]) {
        // A VTE signal can fire while another handler holds the logger; losing a
        // chunk is better than panicking a session, and the next one follows in
        // milliseconds.
        let Ok(mut logger) = self.logger.try_borrow_mut() else {
            return;
        };
        // `write` strips escape sequences, redacts credentials, and prefixes each
        // line with a timestamp only when the connection asks for one. Deciding
        // that here as well is what made "Add Timestamps: off" stamp every
        // record anyway: the branch meant to be the plain one wrapped the output
        // in `[timestamp] OUTPUT:`.
        if let Err(e) = logger.write(data).and_then(|()| logger.flush()) {
            tracing::warn!(%e, "Session transcript write failed");
        }
    }
}

/// The connection-derived values a session log needs besides its configuration.
struct LogContextParts {
    /// Connection name, expanded as `${connection_name}`
    connection_name: String,
    /// Protocol id, expanded as `${protocol}`
    protocol: String,
    /// Log directory `RustConn` manages itself (pruning is confined to it)
    managed_dir: std::path::PathBuf,
}

impl MainWindow {
    /// Resolves the effective session-logging configuration for a connection.
    ///
    /// Returns the configuration together with the protocol name needed for
    /// `${protocol}` expansion and the directory `RustConn` manages itself (the
    /// only place age-based pruning is allowed to run). `None` means logging is
    /// off for this session.
    fn resolve_log_config(
        state: &SharedAppState,
        connection_id: Uuid,
    ) -> Option<(rustconn_core::session::LogConfig, LogContextParts)> {
        let state_ref = state.try_borrow().ok()?;
        let settings = state_ref.settings();
        let managed_dir = Self::managed_log_dir_from(&state_ref);
        let conn = state_ref.get_connection(connection_id)?;
        let config = rustconn_core::session::LogConfig::resolve(
            &settings.logging,
            settings.terminal.log_timestamps,
            &managed_dir,
            conn.log_config.as_ref(),
        )?;
        let parts = LogContextParts {
            connection_name: conn.name.clone(),
            protocol: conn.protocol_config.protocol_type().as_str().to_string(),
            managed_dir,
        };
        Some((config, parts))
    }

    /// The log directory `RustConn` manages itself.
    ///
    /// This is where the global logging switch writes and the only directory
    /// age-based pruning is allowed to touch. A relative setting is resolved
    /// against the configuration directory.
    pub(super) fn managed_log_dir(state: &SharedAppState) -> Option<std::path::PathBuf> {
        let state_ref = state.try_borrow().ok()?;
        Some(Self::managed_log_dir_from(&state_ref))
    }

    fn managed_log_dir_from(state_ref: &crate::state::AppState) -> std::path::PathBuf {
        let configured = &state_ref.settings().logging.log_directory;
        if configured.is_absolute() {
            configured.clone()
        } else {
            state_ref.config_manager().config_dir().join(configured)
        }
    }

    /// The directory a connection's session logs are written to.
    ///
    /// A per-connection path template can put them anywhere, so this expands the
    /// template the same way a session start would and returns its parent. Falls
    /// back to the managed directory when the connection logs nowhere of its own.
    pub(super) fn session_log_dir(
        state: &SharedAppState,
        connection_id: Uuid,
    ) -> Option<std::path::PathBuf> {
        let Some((config, parts)) = Self::resolve_log_config(state, connection_id) else {
            return Self::managed_log_dir(state);
        };
        let context =
            rustconn_core::session::LogContext::new(&parts.connection_name, &parts.protocol);
        rustconn_core::session::SessionLogger::expand_path_template(
            &config.path_template,
            &context,
            None,
        )
        .ok()
        .and_then(|path| path.parent().map(std::path::Path::to_path_buf))
        .or(Some(parts.managed_dir))
    }

    /// Sets up session logging for a terminal session
    ///
    /// The per-connection configuration (connection editor → Logs) takes
    /// precedence over the global switch (Settings → Terminal → Logging); when
    /// both are off nothing is written and this returns immediately.
    ///
    /// Directory creation and file opening are performed asynchronously
    /// to avoid blocking the GTK main thread on slow storage.
    pub fn setup_session_logging(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        session_id: Uuid,
        connection_id: Uuid,
        connection_name: &str,
    ) {
        let Some((config, parts)) = Self::resolve_log_config(state, connection_id) else {
            return;
        };

        let retention_days = config.retention_days;
        let path_template = config.path_template.clone();
        let managed_dir = parts.managed_dir;

        let context = rustconn_core::session::LogContext::new(connection_name, parts.protocol);
        let connection_name_for_header = connection_name.to_string();
        let connection_name_for_callback = connection_name.to_string();
        let notebook_clone = notebook.clone();

        // Template expansion, directory creation and the first write all touch
        // the filesystem, so they happen off the GTK main thread.
        crate::utils::spawn_blocking_with_callback(
            move || {
                // Age-based pruning is destructive, so it is confined to the
                // directory RustConn owns. A user-supplied absolute template
                // may point anywhere — those files are never touched.
                rustconn_core::session::prune_logs(&managed_dir, retention_days);

                let mut logger = rustconn_core::session::SessionLogger::new(config, &context, None)
                    .map_err(|e| e.to_string())?;
                let header = format!(
                    "=== Session Log ===\nConnection: {}\nConnection ID: {}\nSession ID: {}\nStarted: {}\n",
                    connection_name_for_header,
                    connection_id,
                    session_id,
                    chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
                );
                logger.write_record(&header).map_err(|e| e.to_string())?;
                logger.flush().map_err(|e| e.to_string())?;
                Ok(logger)
            },
            move |result: Result<rustconn_core::session::SessionLogger, String>| match result {
                Ok(logger) => {
                    let log_path = logger.log_path().to_path_buf();
                    tracing::info!(
                        connection_name = %connection_name_for_callback,
                        log_path = %log_path.display(),
                        "Session logging enabled"
                    );

                    // Store log file path in session info
                    notebook_clone.set_log_file(session_id, log_path);

                    Self::setup_logging_handlers(&notebook_clone, session_id, logger);
                }
                Err(e) => {
                    tracing::error!(%e, template = %path_template, "Session logging setup failed");
                    // A silent failure is what made issue #247 so hard to
                    // diagnose: the log simply never appeared anywhere.
                    if let Some(root) = notebook_clone.widget().root()
                        && let Some(window) = root.downcast_ref::<gtk4::Window>()
                    {
                        crate::toast::show_toast_on_window(
                            window,
                            &crate::i18n::i18n_f("Session log could not be created: {}", &[&e]),
                            crate::toast::ToastType::Error,
                        );
                    }
                }
            },
        );
    }

    /// Resolves the effective activity-monitor settings for a connection.
    ///
    /// Returns `(mode, quiet_period_secs, silence_timeout_secs, connection_name)`
    /// from the per-connection override layered over the global defaults.
    ///
    /// Returns `None` if the application state is currently borrowed or the
    /// connection no longer exists.
    fn resolve_activity_config(
        state: &SharedAppState,
        connection_id: Uuid,
    ) -> Option<(
        rustconn_core::activity_monitor::MonitorMode,
        u32,
        u32,
        String,
    )> {
        let state_ref = state.try_borrow().ok()?;
        let defaults = &state_ref.settings().activity_monitor;
        let conn = state_ref.get_connection(connection_id)?;
        let name = conn.name.clone();
        Some(if let Some(ref config) = conn.activity_monitor_config {
            (
                config.effective_mode(defaults),
                config.effective_quiet_period(defaults),
                config.effective_silence_timeout(defaults),
                name,
            )
        } else {
            (
                defaults.mode,
                defaults.effective_quiet_period(),
                defaults.effective_silence_timeout(),
                name,
            )
        })
    }

    /// Re-registers activity monitoring for a session after an in-place reconnect.
    ///
    /// In-place reconnect reuses the existing VTE terminal widget, so the
    /// `contents_changed` / `child_exited` handlers wired in
    /// [`Self::setup_activity_monitoring`] persist. Only the coordinator's
    /// session entry needs recreating — it was removed when the previous child
    /// process exited. Re-wiring the handlers here would double them.
    pub(crate) fn reactivate_activity_monitoring(
        state: &SharedAppState,
        activity: &types::SharedActivityCoordinator,
        connection_id: Uuid,
        session_id: Uuid,
    ) {
        let Some((mode, quiet, silence, _name)) =
            Self::resolve_activity_config(state, connection_id)
        else {
            return;
        };
        activity.start(session_id, mode, quiet, silence);
        tracing::debug!(
            %session_id,
            %connection_id,
            ?mode,
            "Activity monitoring re-armed after in-place reconnect"
        );
    }

    /// Sets up activity monitoring for a terminal session.
    ///
    /// Called once per session from the notebook's `on_session_created` hook,
    /// so it covers every terminal protocol (SSH, Telnet, serial, Kubernetes,
    /// Mosh, Zero Trust) and both synchronous and async connection paths.
    ///
    /// Resolves per-connection or global defaults, registers the session with
    /// the coordinator (even when the mode is Off, so the per-tab menu can
    /// enable it later), connects VTE `contents_changed` to `on_output()`, and
    /// delivers notifications (tab indicator, toast, desktop notification).
    /// Also wires `connect_child_exited` for cleanup.
    pub(crate) fn setup_activity_monitoring(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        activity: &types::SharedActivityCoordinator,
        session_id: Uuid,
        connection_id: Uuid,
    ) {
        // Resolve effective config from per-connection override + global defaults
        let Some((mode, quiet, silence, conn_name)) =
            Self::resolve_activity_config(state, connection_id)
        else {
            return;
        };

        // Always register the session with the coordinator, even when the mode
        // is Off. This lets the per-tab "Monitor" menu cycle the mode on a live
        // session (Off → Activity → Silence) without reconnecting (issue #180).
        // `start` only arms the silence timer for Silence mode, and `on_output`
        // is a no-op for Off, so Off mode carries no meaningful runtime cost.
        activity.start(session_id, mode, quiet, silence);

        // Set up silence callback for timer-based silence notifications.
        // The callback is global to the coordinator, so it resolves the session
        // name dynamically rather than capturing a single connection's name
        // (otherwise every session would report the most recently wired name).
        let notebook_for_silence = notebook.clone();
        let sessions_for_silence = notebook.sessions_map();
        activity.set_silence_callback(move |sid, ntype| {
            let name = notebook_for_silence
                .get_session_info(sid)
                .map_or_else(String::new, |info| info.name);
            Self::deliver_activity_notification(
                &notebook_for_silence,
                &sessions_for_silence,
                sid,
                ntype,
                &name,
            );
        });

        // 4.2: Connect contents_changed to on_output() for activity detection
        let activity_for_output = Rc::clone(activity);
        let notebook_for_output = notebook.clone();
        let sessions_for_output = notebook.sessions_map();
        let conn_name_for_output = conn_name.clone();
        notebook.connect_contents_changed(session_id, move || {
            if let Some(ntype) = activity_for_output.on_output(session_id) {
                Self::deliver_activity_notification(
                    &notebook_for_output,
                    &sessions_for_output,
                    session_id,
                    ntype,
                    &conn_name_for_output,
                );
            }
        });

        // 4.7: On child exit, stop the coordinator to clean up timers
        let activity_for_exit = Rc::clone(activity);
        notebook.connect_child_exited(session_id, move |_exit_status| {
            activity_for_exit.stop(session_id);
        });

        tracing::debug!(
            %session_id,
            %connection_id,
            ?mode,
            quiet_period = quiet,
            silence_timeout = silence,
            "Activity monitoring started"
        );
    }

    /// Delivers an activity/silence notification through all channels:
    /// tab indicator icon, toast, and desktop notification (when unfocused).
    fn deliver_activity_notification(
        notebook: &SharedNotebook,
        sessions: &Rc<RefCell<std::collections::HashMap<Uuid, adw::TabPage>>>,
        session_id: Uuid,
        ntype: crate::activity_coordinator::NotificationType,
        session_name: &str,
    ) {
        use crate::activity_coordinator::NotificationType;
        use crate::i18n::i18n_f;

        let (icon_name, toast_msg) = match ntype {
            NotificationType::Activity => (
                "dialog-information-symbolic",
                i18n_f("Activity detected: {}", &[session_name]),
            ),
            NotificationType::Silence => (
                "dialog-warning-symbolic",
                i18n_f("Silence detected: {}", &[session_name]),
            ),
        };

        // 4.3: Set tab indicator icon
        if let Some(page) = sessions.borrow().get(&session_id) {
            page.set_indicator_icon(Some(&gio::ThemedIcon::new(icon_name)));
        }

        // 4.4: Show toast via existing ToastOverlay — on the window the session
        // actually lives in, which is its own one while it is detached
        // (Requirement 5.4, issue #236).
        let host_window: Option<gtk4::Window> =
            super::detach_actions::detached_host_window(session_id)
                .map(gtk4::prelude::Cast::upcast)
                .or_else(|| notebook.widget().root().and_downcast::<gtk4::Window>());
        if let Some(ref window) = host_window {
            let toast_type = match ntype {
                NotificationType::Activity => crate::toast::ToastType::Info,
                NotificationType::Silence => crate::toast::ToastType::Warning,
            };
            crate::toast::show_toast_on_window(window, &toast_msg, toast_type);

            // 4.5: Send desktop notification when window is unfocused
            if !window.is_active()
                && let Some(app) = window.application()
            {
                let notification = gio::Notification::new(&toast_msg);
                notification.set_icon(&gio::ThemedIcon::new(icon_name));
                app.send_notification(Some(&format!("activity-{session_id}")), &notification);
            }
        }
    }

    /// Sets up the child exited handler for session cleanup
    pub fn setup_child_exited_handler(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        session_id: Uuid,
        connection_id: Uuid,
    ) {
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = notebook.clone();
        let connection_id_str = connection_id.to_string();

        // Capture post-disconnect task and connection info before entering the closure
        let post_disconnect_conn = state
            .try_borrow()
            .ok()
            .and_then(|s| s.get_connection(connection_id).cloned());
        let post_disconnect_task = post_disconnect_conn
            .as_ref()
            .and_then(|c| c.post_disconnect_task.clone());

        let post_disconnect_variables = state
            .try_borrow()
            .ok()
            .map(|s| crate::state::resolve_global_variables(s.settings()))
            .unwrap_or_default();
        let post_disconnect_folder_id = post_disconnect_conn.as_ref().and_then(|c| c.group_id);

        let post_disconnect_folder_tracker = state
            .try_borrow()
            .ok()
            .map(|s| Arc::clone(s.folder_tracker()))
            .unwrap_or_default();

        // A Custom Command (Generic Zero Trust) is a one-shot launcher, not a
        // session: RustDesk, WinBox or `xdg-open` return as soon as their own
        // window is up. Leaving a dead terminal with a reconnect banner behind
        // is pure noise (#151), so such a tab always closes on a clean exit —
        // a failure still keeps the tab so its output stays readable.
        let is_custom_command = post_disconnect_conn.as_ref().is_some_and(|c| {
            matches!(
                &c.protocol_config,
                rustconn_core::ProtocolConfig::ZeroTrust(zt)
                    if matches!(zt.provider, rustconn_core::models::ZeroTrustProvider::Generic)
            )
        });

        // Capture close-on-clean-exit setting before entering the closure
        let close_on_clean_exit = is_custom_command
            || state
                .try_borrow()
                .ok()
                .is_some_and(|s| s.settings().terminal.close_on_clean_exit);

        notebook.connect_child_exited(session_id, move |exit_status| {
            // The child can linger on a reconnect overlay, but its expect rules
            // must not: cancel polling and scrub resolved responses immediately.
            notebook_clone.clear_automation_session(session_id);

            // Execute post-disconnect task if configured
            if let Some(ref task) = post_disconnect_task {
                tracing::info!(
                    %connection_id,
                    command = %task.command,
                    "Executing post-disconnect task"
                );

                let mut var_manager = VariableManager::new();
                for var in &post_disconnect_variables {
                    var_manager.set_global(var.clone());
                }
                // Add connection-scoped synthetic variables (host, port, username, name)
                if let Some(ref conn) = post_disconnect_conn {
                    var_manager.set_connection(
                        connection_id,
                        rustconn_core::Variable::new("host", &conn.host),
                    );
                    var_manager.set_connection(
                        connection_id,
                        rustconn_core::Variable::new("port", conn.port.to_string()),
                    );
                    if let Some(ref user) = conn.username {
                        var_manager.set_connection(
                            connection_id,
                            rustconn_core::Variable::new("username", user),
                        );
                    }
                    var_manager.set_connection(
                        connection_id,
                        rustconn_core::Variable::new("name", &conn.name),
                    );
                }

                let executor =
                    TaskExecutor::with_tracker(Arc::new(var_manager), Arc::clone(&post_disconnect_folder_tracker));

                // Run it off the main thread. A post-disconnect task is an
                // arbitrary user command — a script, an ssh, a curl against a
                // host that may itself be unreachable — and blocking here
                // freezes the entire application, every other terminal, RDP and
                // VNC tab included, for as long as it takes: up to the ceiling
                // below. Nothing further down depends on the outcome; the arms
                // only log. Same shape the wake-on-LAN host check already uses.
                let task_for_run = task.clone();
                let command_for_log = task.command.clone();
                crate::utils::spawn_blocking_with_callback(
                    move || {
                        crate::async_utils::with_runtime(|rt| {
                            rt.block_on(async {
                                // ponytail: 60s ceiling bounds the worker thread;
                                // the task's own timeout (if configured) fires first.
                                let ceiling = std::time::Duration::from_mins(1);
                                tokio::time::timeout(
                                    ceiling,
                                    executor.execute_post_disconnect(
                                        &task_for_run,
                                        VariableScope::Connection(connection_id),
                                        post_disconnect_folder_id,
                                    ),
                                )
                                .await
                                .unwrap_or(Err(
                                    rustconn_core::automation::TaskError::Timeout(60_000),
                                ))
                            })
                        })
                    },
                    move |result| match result {
                        Ok(Ok(_)) => {
                            tracing::info!(
                                %connection_id,
                                "Post-disconnect task completed successfully"
                            );
                        }
                        Ok(Err(e)) => {
                            tracing::warn!(
                                %connection_id,
                                command = %command_for_log,
                                error = %e,
                                "Post-disconnect task failed"
                            );
                        }
                        Err(runtime_err) => {
                            tracing::warn!(
                                %connection_id,
                                error = %runtime_err,
                                "Failed to create async runtime for post-disconnect task"
                            );
                        }
                    },
                );
            }

            // Get history entry ID before session info is removed
            let history_entry_id = notebook_clone
                .get_session_info(session_id)
                .and_then(|info| info.history_entry_id);

            // Update session status in state manager. The session log itself is
            // owned by the handlers in `setup_logging_handlers`, which flush it
            // on the same signal.
            if let Ok(mut state_mut) = state_clone.try_borrow_mut()
                && let Err(e) = state_mut.terminate_session(session_id) { tracing::warn!(?e, %session_id, "Failed to terminate session"); }

            // Check if session still exists in notebook
            // If it doesn't, the tab was closed by user
            if notebook_clone.get_session_info(session_id).is_none() {
                // Record connection end in history (user closed tab)
                if let Some(entry_id) = history_entry_id
                    && let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                        state_mut.record_connection_end(entry_id);
                    }
                // Decrement session count - status changes only if no other sessions active
                sidebar_clone.decrement_session_count(&connection_id_str, false);
                return;
            }

            // If the app is shutting down, suppress failure handling — the session
            // exits are expected because close_all_control_sockets() kills SSH connections.
            if crate::app::is_shutting_down() {
                sidebar_clone.decrement_session_count(&connection_id_str, false);
                return;
            }

            // Parse waitpid status to determine if exit was a failure or intentional kill
            // WIFSIGNALED: (status & 0x7f) != 0
            // WTERMSIG: status & 0x7f
            // WIFEXITED: (status & 0x7f) == 0
            // WEXITSTATUS: (status >> 8) & 0xff

            let term_sig = exit_status & 0x7f;
            let is_signaled = term_sig != 0;
            let exit_code = (exit_status >> 8) & 0xff;

            // Consider it a failure if:
            // 1. Killed by a signal that isn't a standard termination signal (HUP, INT, KILL, TERM)
            // 2. Exited normally with non-zero code, UNLESS that code indicates a standard signal kill (128+N)
            let is_failure = if is_signaled {
                !matches!(term_sig, 1 | 2 | 9 | 15)
            } else {
                exit_code != 0 && !matches!(exit_code, 129 | 130 | 137 | 143)
            };

            // Record connection end/failure in history
            if let Some(entry_id) = history_entry_id
                && let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                    if is_failure {
                        let error_msg =
                            format!("Exit status: {exit_status} (Signal: {term_sig}, Code: {exit_code})");
                        state_mut.record_connection_failed(entry_id, &error_msg);
                    } else {
                        state_mut.record_connection_end(entry_id);
                    }
                }

            if is_failure {
                tracing::error!(%session_id, exit_status, term_sig, exit_code, "Session exited with failure");
            }

            // Stop recording if active before marking as disconnected
            notebook_clone.stop_recording(session_id);

            // If the session exited cleanly and close-on-clean-exit is enabled,
            // close the tab automatically instead of showing the reconnect overlay.
            if !is_failure && close_on_clean_exit {
                tracing::info!(
                    %session_id,
                    %connection_id,
                    "Session exited cleanly, auto-closing tab (close_on_clean_exit=true)"
                );
                // Defer close_tab to the next idle iteration of the main loop.
                // Closing the VTE widget synchronously from within its own
                // `child-exited` signal can race with a pending GTK snapshot,
                // causing a use-after-free SIGSEGV in libvte/pango (#171).
                let nb = notebook_clone.clone();
                let sb = sidebar_clone.clone();
                let cid = connection_id_str.clone();
                glib::idle_add_local_once(move || {
                    // Guard: if user already closed the tab before idle fires,
                    // the session no longer exists — skip to avoid double
                    // decrement_session_count.
                    if nb.get_session_info(session_id).is_none() {
                        return;
                    }
                    nb.close_tab(session_id);
                    sb.decrement_session_count(&cid, false);
                });
                return;
            }

            // Defer all remaining widget/VTE work (disconnect indicator,
            // terminal.reset(), reconnect banner, auto-reconnect setup) to
            // the next main-loop idle. `child-exited` is emitted from inside
            // VTE — resetting VTE state or mutating the widget tree during
            // the emission can race with the pending GTK snapshot of the
            // current frame and crash in libvte/pango (#171).
            let notebook_clone = notebook_clone.clone();
            let state_clone = state_clone.clone();
            let sidebar_clone = sidebar_clone.clone();
            let connection_id_str = connection_id_str.clone();
            glib::idle_add_local_once(move || {
            // Guard: the tab may have been closed before this idle ran.
            if notebook_clone.get_session_info(session_id).is_none() {
                sidebar_clone.decrement_session_count(&connection_id_str, is_failure);
                return;
            }

            // Mark tab as disconnected and show reconnect overlay
            notebook_clone.mark_tab_disconnected(session_id);
            notebook_clone.show_reconnect_overlay(session_id);

            // Auto-reconnect: poll host and reconnect when it comes back online
            // Only for non-intentional disconnects (failures, not user-initiated)
            // Skip auto-reconnect for SSH authentication failures (exit code 255)
            // because the host is reachable but credentials are wrong — polling
            // would instantly trigger reconnect creating an infinite loop.
            let is_ssh_auth_failure = exit_code == 255
                && notebook_clone
                    .get_session_info(session_id)
                    .is_some_and(|info| info.protocol == "ssh");

            // Skip auto-reconnect for rapid crashes (session lived < 5 seconds).
            // This prevents infinite reconnect loops when the terminal process
            // crashes immediately (e.g., SIGSEGV in VTE on macOS).
            let is_rapid_crash = notebook_clone
                .get_session_info(session_id)
                .is_some_and(|info| {
                    let elapsed = chrono::Utc::now()
                        .signed_duration_since(info.connected_at)
                        .num_seconds();
                    elapsed < 5
                });

            if is_rapid_crash {
                tracing::warn!(
                    %session_id,
                    %connection_id,
                    "Skipping auto-reconnect: session crashed within 5 seconds of start"
                );
            }

            if is_failure
                && !is_ssh_auth_failure
                && !is_rapid_crash
                && let Ok(state_ref) = state_clone.try_borrow()
                && let Some(conn) = state_ref.get_connection(connection_id)
            {
                // Use per-connection retry config or default
                let retry_config = conn.retry_config.clone()
                    .unwrap_or_default();

                // If retry is explicitly disabled, skip auto-reconnect
                if !retry_config.enabled {
                    drop(state_ref);
                    sidebar_clone.decrement_session_count(&connection_id_str, is_failure);
                    return;
                }

                let host = conn.host.clone();
                let port = conn.port;
                drop(state_ref);

                let cancel = std::sync::Arc::new(
                    std::sync::atomic::AtomicBool::new(false),
                );
                // Register cancel token so closing the tab cancels polling
                notebook_clone.register_poll_cancel(session_id, cancel.clone());

                tracing::info!(
                    %connection_id,
                    %host,
                    %port,
                    max_attempts = retry_config.max_attempts,
                    initial_delay_ms = retry_config.initial_delay_ms,
                    "Starting auto-reconnect with exponential backoff"
                );

                // Update the reconnect banner to show auto-reconnect status
                notebook_clone.update_reconnect_banner_status(session_id, true);

                // Channel for attempt progress updates (background → main thread)
                let (attempt_tx, attempt_rx) = std::sync::mpsc::channel::<u32>();
                let notebook_attempt = notebook_clone.clone();
                let max_attempts_for_ui = retry_config.max_attempts;
                gtk4::glib::timeout_add_local(std::time::Duration::from_millis(100), move || {
                    match attempt_rx.try_recv() {
                        Ok(attempt) => {
                            notebook_attempt.update_reconnect_banner_attempt(
                                session_id,
                                attempt,
                                max_attempts_for_ui,
                            );
                            gtk4::glib::ControlFlow::Continue
                        }
                        Err(std::sync::mpsc::TryRecvError::Empty) => {
                            gtk4::glib::ControlFlow::Continue
                        }
                        Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                            gtk4::glib::ControlFlow::Break
                        }
                    }
                });

                // Clone notebook's on_reconnect callback for use in the polling result
                let on_reconnect = notebook_clone.reconnect_callback();
                let notebook_cleanup = notebook_clone.clone();

                crate::utils::spawn_blocking_with_callback(
                    move || {
                        let rt = tokio::runtime::Runtime::new()
                            .map_err(rustconn_core::host_check::HostCheckError::Io)?;
                        rt.block_on(
                            rustconn_core::host_check::poll_until_online_with_backoff(
                                &host,
                                port,
                                &retry_config,
                                &cancel,
                                |attempt, is_online| {
                                    tracing::debug!(
                                        attempt,
                                        is_online,
                                        "Auto-reconnect probe"
                                    );
                                    let _ = attempt_tx.send(attempt);
                                },
                            ),
                        )
                    },
                    move |result| {
                        // Clean up the cancel token
                        notebook_cleanup.cancel_poll(session_id);
                        if matches!(result, Ok(true)) {
                            // Guard: if the user closed the tab while polling
                            // was active, the session no longer exists — skip
                            // reconnect to avoid creating an orphan tab. A
                            // detached session has no tab but is very much
                            // alive, so it counts as existing (issue #236).
                            let session_exists = notebook_cleanup
                                .sessions_map()
                                .borrow()
                                .contains_key(&session_id)
                                || notebook_cleanup.is_detached(session_id);
                            if !session_exists {
                                tracing::debug!(
                                    %session_id,
                                    "Tab closed during polling, skipping reconnect"
                                );
                                return;
                            }
                            tracing::info!(
                                %connection_id,
                                "Host is back online, triggering reconnect"
                            );
                            if let Some(ref cb) = *on_reconnect.borrow() {
                                cb(session_id, connection_id);
                            }
                        }
                    },
                );
            }

            // Decrement session count - status changes only if no other sessions active
            sidebar_clone.decrement_session_count(&connection_id_str, is_failure);
            });
        });
    }

    /// Sets up logging handlers for a terminal session based on settings
    ///
    /// Supports three logging modes:
    /// - Activity: logs change counts (default, lightweight)
    /// - Input: logs user commands sent to terminal
    /// - Output: logs full terminal transcript
    ///
    /// Everything goes through [`rustconn_core::session::SessionLogger`], so the
    /// configured timestamp format, size rotation, retention and credential
    /// redaction all apply — whether the configuration came from the connection
    /// editor or from the global settings.
    fn setup_logging_handlers(
        notebook: &SharedNotebook,
        session_id: Uuid,
        logger: rustconn_core::session::SessionLogger,
    ) {
        let (log_activity, log_input, log_output) = {
            let config = logger.config();
            (config.log_activity, config.log_input, config.log_output)
        };
        let logger = Rc::new(RefCell::new(logger));

        // Set up activity logging (change counts)
        if log_activity {
            let logger_clone = logger.clone();
            let change_counter: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));
            let last_log_time: Rc<RefCell<std::time::Instant>> =
                Rc::new(RefCell::new(std::time::Instant::now()));
            let flush_counter: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));

            notebook.connect_contents_changed(session_id, move || {
                let mut counter = change_counter.borrow_mut();
                *counter += 1;

                let mut flush_count = flush_counter.borrow_mut();
                *flush_count += 1;

                let now = std::time::Instant::now();
                let elapsed = now.duration_since(*last_log_time.borrow());

                if *counter >= 100 || elapsed.as_secs() >= 5 {
                    let flush = *flush_count >= 10 || elapsed.as_secs() >= 30;
                    log_event(
                        &logger_clone,
                        &format!("Terminal activity ({} changes)", *counter),
                        flush,
                    );
                    if flush {
                        *flush_count = 0;
                    }

                    *counter = 0;
                    *last_log_time.borrow_mut() = now;
                }
            });
        }

        // Set up input logging (user commands)
        if log_input {
            let logger_clone = logger.clone();
            let input_buffer: Rc<RefCell<String>> = Rc::new(RefCell::new(String::new()));
            let last_flush: Rc<RefCell<std::time::Instant>> =
                Rc::new(RefCell::new(std::time::Instant::now()));

            notebook.connect_commit(session_id, move |text| {
                let mut buffer = input_buffer.borrow_mut();

                // Handle special characters
                for ch in text.chars() {
                    match ch {
                        '\r' | '\n' => {
                            // End of command - log it
                            if !buffer.is_empty() {
                                log_event(&logger_clone, &format!("INPUT: {}", *buffer), true);
                                buffer.clear();
                            }
                        }
                        '\x7f' | '\x08' => {
                            // Backspace - remove last char
                            buffer.pop();
                        }
                        '\x03' => {
                            // Ctrl+C
                            log_event(&logger_clone, "INPUT: ^C", true);
                            buffer.clear();
                        }
                        '\x04' => {
                            // Ctrl+D
                            log_event(&logger_clone, "INPUT: ^D", true);
                        }
                        _ if ch.is_control() => {
                            // Skip other control characters
                        }
                        _ => {
                            buffer.push(ch);
                        }
                    }
                }

                // Periodic flush for long-running commands
                let now = std::time::Instant::now();
                if now.duration_since(*last_flush.borrow()).as_secs() >= 30 && !buffer.is_empty() {
                    log_event(
                        &logger_clone,
                        &format!("INPUT (partial): {}", *buffer),
                        true,
                    );
                    *last_flush.borrow_mut() = now;
                }
            });
        }

        // Set up output logging (full transcript, issue #247).
        //
        // The transcript is a copy of the session's PTY output, delivered by the
        // relay as the child produces it. There is no sampling and no diffing:
        // the previous implementations read the text back out of the widget,
        // which cannot be made correct because VTE rewraps and renumbers its
        // buffer on a resize.
        if log_output {
            let writer = Rc::new(RefCell::new(TranscriptWriter::new(logger.clone())));

            let writer_for_output = Rc::clone(&writer);
            notebook.add_output_observer(session_id, move |chunk: &[u8]| {
                // A reentrant call cannot happen — observers run one at a time on
                // the main thread — but dropping a chunk is still better than
                // panicking a live session.
                if let Ok(mut writer) = writer_for_output.try_borrow_mut() {
                    writer.accept(chunk);
                }
            });

            // On exit the shell's last line (a `logout`, an authentication
            // failure) is still partial, and no further output will complete it.
            // This handler holds the second strong reference, and it is released
            // when the terminal is destroyed — after the close path has removed
            // the observer, so a session ending normally still gets its final
            // line.
            let writer_on_exit = Rc::clone(&writer);
            notebook.connect_child_exited(session_id, move |_exit_status| {
                if let Ok(mut writer) = writer_on_exit.try_borrow_mut() {
                    writer.finish();
                }
            });

            // A prompt carries no line break, so a trailing partial line needs a
            // deadline of its own rather than waiting for output that will only
            // arrive once the user has typed something. The timer holds a weak
            // reference, which is also how it ends: once the tab is gone and both
            // strong references with it, the upgrade fails and the source stops.
            // It deliberately survives an in-place reconnect, because the same
            // writer keeps serving the reconnected session.
            let writer_for_timer = Rc::downgrade(&writer);
            glib::timeout_add_local(PARTIAL_LINE_GRACE, move || {
                let Some(writer) = writer_for_timer.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                if let Ok(mut writer) = writer.try_borrow_mut() {
                    writer.flush_stale_partial();
                }
                glib::ControlFlow::Continue
            });
        }

        // Flush when the session's process exits, so the log is readable while
        // the tab lingers on the reconnect overlay. Deliberately not `close()`:
        // an in-place reconnect reuses this terminal and keeps these handlers,
        // so closing the writer here would silently end logging for the rest of
        // the tab's life. The "Session ended" marker is written by the logger's
        // `Drop` when the tab is finally closed.
        notebook.connect_child_exited(session_id, move |_exit_status| {
            if let Ok(mut logger) = logger.try_borrow_mut()
                && let Err(e) = logger.flush()
            {
                tracing::warn!(%e, "Session log flush on exit failed");
            }
        });
    }
}

/// Writes one timestamped event record through the session logger.
///
/// A VTE signal can fire while another handler still holds the logger, so a
/// failed borrow drops the record instead of panicking — losing one activity
/// line is preferable to killing the session.
fn log_event(
    logger: &Rc<RefCell<rustconn_core::session::SessionLogger>>,
    event: &str,
    flush: bool,
) {
    let Ok(mut logger) = logger.try_borrow_mut() else {
        return;
    };
    // "Add Timestamps" governs every line in the file, not just the transcript:
    // a log the user asked to keep unstamped should not have stamps on its
    // `INPUT:` and activity records either.
    let record = if logger.config().log_timestamps {
        format!("[{}] {event}", logger.current_timestamp())
    } else {
        event.to_owned()
    };
    if let Err(e) = logger.write_record(&record) {
        tracing::warn!(%e, "Session log write failed");
        return;
    }
    if flush && let Err(e) = logger.flush() {
        tracing::warn!(%e, "Session log flush failed");
    }
}

#[cfg(test)]
mod transcript_writer_tests {
    use rustconn_core::session::{LogConfig, LogContext, SessionLogger};

    use super::{PARTIAL_LINE_GRACE, TranscriptWriter};

    /// Builds a writer over a real log file and returns both.
    ///
    /// `LogConfig::new` leaves timestamps off, which is the default and
    /// therefore the shape most session logs take.
    fn writer() -> (TranscriptWriter, tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let config = LogConfig::new(
            dir.path()
                .join("session.log")
                .to_string_lossy()
                .into_owned(),
        );
        let logger = SessionLogger::new(config, &LogContext::new("host", "ssh"), None)
            .expect("logger opens");
        let path = logger.log_path().to_owned();
        let logger = std::rc::Rc::new(std::cell::RefCell::new(logger));
        (TranscriptWriter::new(logger), dir, path)
    }

    fn read(path: &std::path::Path) -> String {
        std::fs::read_to_string(path).expect("read log back")
    }

    #[test]
    fn complete_lines_are_written_and_the_partial_is_held() {
        let (mut writer, _dir, path) = writer();
        writer.accept(b"first line\nsecond line\npart");

        let written = read(&path);
        assert!(written.contains("first line"), "{written}");
        assert!(written.contains("second line"), "{written}");
        assert!(
            !written.contains("part"),
            "an unfinished line must wait for its newline: {written}"
        );
    }

    #[test]
    fn a_line_split_across_chunks_is_written_once_and_whole() {
        // The relay hands over whatever a read returned, so a line arriving in
        // three pieces is the normal case, not an edge case.
        let (mut writer, _dir, path) = writer();
        writer.accept(b"total ");
        writer.accept(b"used ");
        writer.accept(b"free\n");

        let written = read(&path);
        assert_eq!(
            written.matches("total used free").count(),
            1,
            "the line must appear exactly once, reassembled: {written}"
        );
    }

    #[test]
    fn carriage_returns_do_not_split_a_line() {
        // A progress bar redraws itself with `\r`. Treating that as a line break
        // would turn one download into hundreds of log lines.
        let (mut writer, _dir, path) = writer();
        writer.accept(b"10%\r 50%\r100%\n");

        let written = read(&path);
        assert!(written.contains("100%"), "{written}");
        assert_eq!(
            written.lines().filter(|l| l.contains('%')).count(),
            1,
            "one line, not one per redraw: {written}"
        );
    }

    #[test]
    fn timestamps_are_absent_unless_the_connection_asks_for_them() {
        // "Add Timestamps" governs the whole file. The transcript used to wrap
        // output in `[timestamp] OUTPUT:` on exactly the path that is supposed to
        // have no timestamps, so a log with the switch off was stamped anyway.
        let (mut writer, _dir, path) = writer();
        writer.accept(b"plain output\n");

        let written = read(&path);
        assert!(written.contains("plain output"), "{written}");
        assert!(
            !written.contains("] OUTPUT:"),
            "no record marker when timestamps are off: {written}"
        );
        assert!(
            written
                .lines()
                .filter(|line| line.contains("plain output"))
                .all(|line| !line.trim_start().starts_with('[')),
            "the output line must carry no timestamp: {written}"
        );
    }

    #[test]
    fn timestamps_are_written_when_the_connection_asks_for_them() {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let config = LogConfig::new(
            dir.path()
                .join("stamped.log")
                .to_string_lossy()
                .into_owned(),
        )
        .with_log_timestamps(true);
        let logger = SessionLogger::new(config, &LogContext::new("host", "ssh"), None)
            .expect("logger opens");
        let path = logger.log_path().to_owned();
        let mut writer = TranscriptWriter::new(std::rc::Rc::new(std::cell::RefCell::new(logger)));

        writer.accept(b"stamped output\n");

        let written = read(&path);
        assert!(
            written
                .lines()
                .any(|line| line.contains("stamped output") && line.trim_start().starts_with('[')),
            "with the switch on, the line must be prefixed: {written}"
        );
    }

    #[test]
    fn a_stale_partial_line_is_written_so_a_prompt_is_recorded() {
        // A shell prompt carries no newline: the shell is waiting on the same
        // line. Diagnosing a connection needs to see where it stopped (issue
        // #247), so a partial that stops growing is written anyway.
        let (mut writer, _dir, path) = writer();
        writer.accept(b"[ec2-user@ip-10-0-0-1 ~]$ ");
        assert!(
            !read(&path).contains("ec2-user"),
            "held at first, in case more of the line follows"
        );

        std::thread::sleep(PARTIAL_LINE_GRACE + std::time::Duration::from_millis(100));
        writer.flush_stale_partial();

        assert!(
            read(&path).contains("[ec2-user@ip-10-0-0-1 ~]$"),
            "a prompt must reach the log without waiting for the next command"
        );
    }

    #[test]
    fn a_growing_partial_line_is_not_flushed_early() {
        let (mut writer, _dir, path) = writer();
        writer.accept(b"still ");
        std::thread::sleep(PARTIAL_LINE_GRACE + std::time::Duration::from_millis(100));
        // More output resets the clock, so the line is still being written.
        writer.accept(b"arriving");
        writer.flush_stale_partial();

        assert!(
            !read(&path).contains("still arriving"),
            "a line that is still growing must not be cut in half"
        );
    }

    #[test]
    fn finish_writes_the_last_line_of_a_session() {
        // `logout` and an authentication failure both land on an unterminated
        // line that no later output will complete.
        let (mut writer, _dir, path) = writer();
        writer.accept(b"Permission denied (publickey).");
        writer.finish();

        assert!(
            read(&path).contains("Permission denied (publickey)."),
            "the last thing a session says must be recorded"
        );
    }

    #[test]
    fn finish_on_an_empty_buffer_writes_nothing() {
        let (mut writer, _dir, path) = writer();
        writer.accept(b"line\n");
        let before = read(&path);
        writer.finish();
        assert_eq!(before, read(&path), "no empty record may be appended");
    }

    #[test]
    fn escape_sequences_are_stripped_but_the_text_survives() {
        // Raw PTY output is full of colour codes; the log is read by people.
        let (mut writer, _dir, path) = writer();
        writer.accept(b"\x1b[32mconnected\x1b[0m to host\r\n");

        let written = read(&path);
        assert!(!written.contains('\x1b'), "escapes must be gone: {written}");
        assert!(written.contains("connected"), "{written}");
        assert!(written.contains("to host"), "{written}");
    }

    #[test]
    fn a_credential_in_the_stream_is_redacted() {
        // The transcript now carries what the child wrote, byte for byte, so
        // redaction is the only thing between an echoed secret and the log file.
        // It still runs on this path: the writer goes through the same
        // `write_record` the scraped transcript used.
        let (mut writer, _dir, path) = writer();
        writer.accept(b"password: hunter2\n");

        let written = read(&path);
        assert!(
            !written.contains("hunter2"),
            "the secret must not reach the log: {written}"
        );
        assert!(
            written.contains("[REDACTED]"),
            "the line must be marked, not silently dropped: {written}"
        );
    }
}
