//! Session recording (start, stop, playback, metadata).
//!
//! Extracted from `terminal/mod.rs` to reduce module complexity.

use super::*;

impl TerminalNotebook {
    // ========================================================================
    // Session Recording
    // ========================================================================

    /// Starts recording a terminal session.
    ///
    /// For local shells, launches `script` with local file paths.
    /// For SSH/remote sessions, launches `script` on the remote host with
    /// `/tmp` paths and retrieves the files via SCP when recording stops.
    ///
    /// # Returns
    ///
    /// `true` if recording was started successfully, `false` on error.
    pub fn start_recording(
        &self,
        session_id: Uuid,
        connection_name: &str,
        ssh_params: Option<SshRecordingParams>,
    ) -> bool {
        use rustconn_core::session::recording::{
            default_recordings_dir, ensure_recordings_dir, recording_paths,
        };

        // Duplicate check: if already recording, return true without action
        if self.is_recording(session_id) {
            return true;
        }

        let Some(dir) = default_recordings_dir() else {
            tracing::error!("Cannot determine recordings directory");
            return false;
        };

        if let Err(e) = ensure_recordings_dir(&dir) {
            tracing::error!(%e, "Recordings directory is not writable, disabling recording");
            return false;
        }

        let (data_path, timing_path) = recording_paths(&dir, connection_name);

        // Determine if this is a remote session by checking the protocol
        let is_remote = self
            .session_info
            .borrow()
            .get(&session_id)
            .map(|info| {
                matches!(
                    info.protocol.as_str(),
                    "ssh" | "sftp" | "telnet" | "mosh" | "serial"
                )
            })
            .unwrap_or(false);

        // Store recording paths and start time for metadata generation on stop
        self.recording_paths.borrow_mut().insert(
            session_id,
            (
                data_path.clone(),
                timing_path.clone(),
                connection_name.to_string(),
                Instant::now(),
            ),
        );

        // Update tab title with ●REC indicator
        self.update_recording_indicator(session_id, true);

        if let Some(terminal) = self.get_terminal(session_id) {
            if let Some(params) = ssh_params.filter(|_| is_remote) {
                // Remote session: use /tmp paths on the remote host.
                // After stop_recording we retrieve files via SCP.
                let short_id = &session_id.to_string()[..8];
                let remote_data = format!("/tmp/rustconn_rec_{short_id}.data");
                let remote_timing = format!("/tmp/rustconn_rec_{short_id}.timing");

                self.remote_recordings.borrow_mut().insert(
                    session_id,
                    RemoteRecordingInfo {
                        remote_data: remote_data.clone(),
                        remote_timing: remote_timing.clone(),
                        local_data: data_path.clone(),
                        local_timing: timing_path.clone(),
                        ssh_params: params,
                    },
                );

                let cmd = format!(
                    " script -q -f --log-out '{remote_data}' --log-timing '{remote_timing}'\n"
                );
                terminal.feed_child(cmd.as_bytes());
                // Erase the echoed command after a short delay so the PTY echo
                // has time to arrive before we clear the line.
                let term_clone = terminal.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
                    term_clone.feed(b"\x1b[1A\x1b[2K");
                });
            } else {
                // Local session: write directly to local recording paths.
                let data_str = data_path.display().to_string();
                let timing_str = timing_path.display().to_string();
                let cmd =
                    format!(" script -q -f --log-out '{data_str}' --log-timing '{timing_str}'\n");
                terminal.feed_child(cmd.as_bytes());
                // Erase the echoed command after a short delay so the PTY echo
                // has time to arrive before we clear the line.
                let term_clone = terminal.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(100), move || {
                    term_clone.feed(b"\x1b[1A\x1b[2K");
                });
            }
            self.active_recordings.borrow_mut().insert(session_id);
        }

        tracing::info!(
            %session_id,
            data = %data_path.display(),
            timing = %timing_path.display(),
            remote = is_remote,
            "Session recording started via script"
        );

        true
    }

    /// Stops recording a terminal session.
    ///
    /// Sends EOF (Ctrl+D) to terminate the `script` sub-shell, then restores
    /// the tab title and generates the metadata sidecar. For remote sessions,
    /// retrieves the recording files from the remote host via SCP in a
    /// background thread to avoid blocking the GTK main loop.
    pub fn stop_recording(&self, session_id: Uuid) {
        let Some(stopped) = self.detach_recording(session_id) else {
            return;
        };

        // For remote sessions, retrieve files via SCP in a background thread
        if let Some(remote) = stopped.remote {
            let rec_info = stopped.info;

            crate::utils::spawn_blocking_with_callback(
                move || (retrieve_remote_recording(&remote), rec_info),
                move |result: (
                    bool,
                    Option<(std::path::PathBuf, std::path::PathBuf, String, Instant)>,
                )| {
                    let (scp_ok, rec_info) = result;
                    log_retrieval(session_id, scp_ok);
                    // Generate .meta.json sidecar on the GTK thread
                    if let Some((data_path, timing_path, connection_name, start_time)) = rec_info {
                        Self::write_recording_metadata(
                            &data_path,
                            &timing_path,
                            &connection_name,
                            start_time,
                            session_id,
                        );
                    }
                },
            );
        } else if let Some((data_path, timing_path, connection_name, start_time)) = stopped.info {
            // Local session — generate metadata synchronously (fast, no I/O)
            Self::write_recording_metadata(
                &data_path,
                &timing_path,
                &connection_name,
                start_time,
                session_id,
            );
        }

        tracing::info!(%session_id, "Session recording stopped");
    }

    /// Stops a recording leaving no work behind for the main loop.
    ///
    /// [`Self::stop_recording`] finishes a remote recording from a GTK
    /// callback, which is right while the app runs and wrong while it quits:
    /// the main loop stops before that callback can be polled, so the SCP
    /// result is dropped, the `.meta.json` sidecar is never written, and the
    /// worker thread dies with the process — possibly mid-transfer. The
    /// recording then simply never appears in the list.
    ///
    /// On the shutdown path the retrieval runs inline instead, so the files and
    /// their sidecar are on disk before the window is allowed to close. The
    /// cost is a stall on quit, bounded by the SSH connect timeout in
    /// [`retrieve_remote_recording`] so an unreachable host cannot hang it.
    pub fn stop_recording_blocking(&self, session_id: Uuid) {
        let Some(stopped) = self.detach_recording(session_id) else {
            return;
        };

        if let Some(remote) = stopped.remote {
            log_retrieval(session_id, retrieve_remote_recording(&remote));
        }

        if let Some((data_path, timing_path, connection_name, start_time)) = stopped.info {
            Self::write_recording_metadata(
                &data_path,
                &timing_path,
                &connection_name,
                start_time,
                session_id,
            );
        }

        tracing::info!(%session_id, "Session recording stopped");
    }

    /// Detaches a session from the live recording bookkeeping.
    ///
    /// Shared prologue of both stop paths: drops the session from the active
    /// set, sends EOF so the `script` sub-shell flushes and exits, clears the
    /// indicator, and hands back what the caller needs to finish the job.
    /// `None` when the session was not being recorded.
    fn detach_recording(&self, session_id: Uuid) -> Option<StoppedRecording> {
        if !self.active_recordings.borrow_mut().remove(&session_id) {
            return None;
        }

        // Send Ctrl+D (EOF) to terminate the `script` sub-shell cleanly.
        // Unlike `exit\n`, EOF produces no visible echo and is safely ignored
        // if the sub-shell has already exited.
        if let Some(terminal) = self.get_terminal(session_id) {
            terminal.feed_child(b"\x04");
        }

        self.update_recording_indicator(session_id, false);

        Some(StoppedRecording {
            // Recording paths + start time, captured before moving into closures
            info: self.recording_paths.borrow_mut().remove(&session_id),
            remote: self.remote_recordings.borrow_mut().remove(&session_id),
        })
    }

    /// Writes the `.meta.json` sidecar for a finished recording.
    fn write_recording_metadata(
        data_path: &std::path::Path,
        timing_path: &std::path::Path,
        connection_name: &str,
        start_time: Instant,
        session_id: Uuid,
    ) {
        let duration = start_time.elapsed().as_secs_f64();
        let data_size = std::fs::metadata(data_path).map(|m| m.len()).unwrap_or(0);
        let timing_size = std::fs::metadata(timing_path).map(|m| m.len()).unwrap_or(0);

        let meta = RecordingMetadata {
            connection_name: connection_name.to_string(),
            display_name: None,
            created_at: chrono::Utc::now(),
            duration_secs: duration,
            total_size_bytes: data_size + timing_size,
        };
        let meta_path = metadata_path(data_path);
        if let Err(e) = write_metadata(&meta_path, &meta) {
            tracing::warn!(%e, %session_id, "Failed to write recording metadata sidecar");
        }
    }

    /// Returns whether a session is currently being recorded.
    #[must_use]
    pub fn is_recording(&self, session_id: Uuid) -> bool {
        self.active_recordings.borrow().contains(&session_id)
    }

    /// Opens a new Playback Tab for the given recording entry.
    ///
    /// Creates a tab containing a VTE terminal with a playback toolbar
    /// overlay. The recording is loaded and playback starts automatically.
    pub fn open_playback_tab(&self, entry: &rustconn_core::session::recording::RecordingEntry) {
        self.remove_welcome_page();

        let display_name = entry
            .metadata
            .display_name
            .as_deref()
            .unwrap_or(&entry.metadata.connection_name);
        let tab_title = i18n_f("Playback: {}", &[display_name]);

        let widget = playback::create_playback_tab_widget(entry);

        let tab_container = TabPageContainer::single(&widget);
        let page = self.tab_view.append(tab_container.widget());
        page.set_title(&tab_title);
        page.set_icon(Some(&gio::ThemedIcon::new("media-playback-start-symbolic")));
        page.set_tooltip(&tab_title);

        self.tab_view.set_selected_page(&page);
    }

    /// Flushes all active session recorders without removing them.
    ///
    /// Called during window close / application shutdown to ensure all
    /// buffered recording data is written to disk before exit.
    pub fn flush_active_recordings(&self) {
        // With the `script`-based approach, recording is handled by the
        // external `script` process which flushes on exit. We send `exit`
        // to each active recording session to ensure `script` terminates
        // and flushes its buffers.
        //
        // Blocking variant on purpose: this runs while the window is closing,
        // and anything deferred to the main loop from here would never run.
        let ids: Vec<Uuid> = self.active_recordings.borrow().iter().copied().collect();
        for session_id in ids {
            self.stop_recording_blocking(session_id);
        }
    }

    /// Updates the tab title to show or hide the "●REC" indicator.
    pub(crate) fn update_recording_indicator(&self, session_id: Uuid, recording: bool) {
        // Notify the window so the sidebar recording indicator follows
        // every start/stop path (manual action, auto-record, session end)
        let connection_id = self
            .session_info
            .borrow()
            .get(&session_id)
            .map(|info| info.connection_id);
        if let Some(connection_id) = connection_id
            && let Some(ref callback) = *self.on_recording_changed.borrow()
        {
            callback(connection_id, recording);
        }

        let rec_prefix = i18n("●REC");
        if let Some(page) = self.sessions.borrow().get(&session_id) {
            let current_title = page.title().to_string();
            if recording {
                if !current_title.starts_with(&rec_prefix) {
                    page.set_title(&format!("{rec_prefix} {current_title}"));
                }
            } else {
                let stripped = current_title
                    .strip_prefix(&rec_prefix)
                    .map(|s| s.trim_start())
                    .unwrap_or(&current_title);
                page.set_title(stripped);
            }
        }
    }
}

/// What a stopped recording still needs in order to be finished off.
///
/// Produced by [`TerminalNotebook::detach_recording`] once the session is out
/// of the live bookkeeping, so the two stop paths — deferred and blocking —
/// share the prologue instead of each keeping their own copy of it.
struct StoppedRecording {
    /// Local data path, timing path, connection name and start instant.
    info: Option<(PathBuf, PathBuf, String, Instant)>,
    /// Set when the `script` process ran on a remote host and the files have
    /// to be fetched back.
    remote: Option<RemoteRecordingInfo>,
}

/// Copies a finished remote recording back over SCP and removes the remote
/// temporaries. Returns whether both files arrived.
///
/// Deliberately free of `self` and of GTK: the ordinary stop runs it on a
/// worker thread, application shutdown runs it inline on the main thread.
fn retrieve_remote_recording(remote: &RemoteRecordingInfo) -> bool {
    let params = &remote.ssh_params;

    // Bounded connect: this also runs on the quit path, where an unreachable
    // host would otherwise hold the window open for the TCP default (~2min).
    // Only the connect is bounded — a slow transfer still runs to completion.
    let common_opts = |args: &mut Vec<String>| {
        if let Some(ref key) = params.identity_file {
            args.push("-i".to_string());
            args.push(key.clone());
        }
        if let Some(kh) = rustconn_core::get_flatpak_known_hosts_path() {
            args.push("-o".to_string());
            args.push(format!("UserKnownHostsFile={}", kh.display()));
        }
        args.push("-o".to_string());
        args.push("StrictHostKeyChecking=accept-new".to_string());
        args.push("-o".to_string());
        args.push("ConnectTimeout=5".to_string());
    };

    // scp spells the port -P, ssh spells it -p.
    let mut scp_args = vec!["-P".to_string(), params.port.to_string()];
    common_opts(&mut scp_args);
    let mut ssh_args = vec!["-p".to_string(), params.port.to_string()];
    common_opts(&mut ssh_args);

    let user_host = params.username.as_ref().map_or_else(
        || params.host.clone(),
        |user| format!("{user}@{}", params.host),
    );

    let fetch = |remote_path: &str, local_path: &std::path::Path| {
        std::process::Command::new("scp")
            .args(&scp_args)
            .arg(format!("{user_host}:{remote_path}"))
            .arg(local_path.as_os_str())
            .output()
            .is_ok_and(|o| o.status.success())
    };

    let data_ok = fetch(&remote.remote_data, &remote.local_data);
    let timing_ok = fetch(&remote.remote_timing, &remote.local_timing);

    // Clean up remote temp files (best-effort)
    let _ = std::process::Command::new("ssh")
        .args(&ssh_args)
        .arg(&user_host)
        .arg(format!(
            "rm -f '{}' '{}'",
            remote.remote_data, remote.remote_timing
        ))
        .output();

    data_ok && timing_ok
}

/// Reports the outcome of a remote retrieval, identically on both stop paths.
fn log_retrieval(session_id: Uuid, scp_ok: bool) {
    if scp_ok {
        tracing::info!(%session_id, "Remote recording files retrieved via SCP");
    } else {
        tracing::warn!(%session_id, "Failed to retrieve remote recording files via SCP");
    }
}
