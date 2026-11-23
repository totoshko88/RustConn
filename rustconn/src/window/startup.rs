//! Startup actions, session restore and `.rdp` file open

use super::*;

impl MainWindow {
    /// Reopens the sessions that were open when RustConn last closed (issue #243).
    ///
    /// Called from `build_ui` after the startup action, so an explicit startup
    /// connection lands first and the restored set follows. A no-op when
    /// "Restore sessions on startup" is off or no usable snapshot exists.
    pub fn restore_previous_sessions(&self) {
        session_restore::restore_previous_sessions(
            session_restore::RestoreContext {
                state: self.state.clone(),
                notebook: self.terminal_notebook.clone(),
                split_view: self.split_view.clone(),
                sidebar: Rc::clone(&self.sidebar),
                monitoring: Rc::clone(&self.monitoring),
                activity: self.activity_coordinator.clone(),
            },
            self.window.upcast_ref::<gtk4::Window>(),
        );
    }

    /// Executes a startup action (open local shell or connect to a saved connection)
    ///
    /// Called from `build_ui` after the window is presented. CLI args override
    /// the persisted setting.
    pub fn execute_startup_action(&self, action: &rustconn_core::config::StartupAction) {
        use rustconn_core::config::StartupAction;
        match action {
            StartupAction::None => {}
            StartupAction::LocalShell => {
                tracing::info!("Startup action: opening local shell");
                Self::open_local_shell_with_split(
                    &self.terminal_notebook,
                    &self.split_view,
                    Some(&self.state),
                    None,
                    None,
                );
            }
            StartupAction::Connection(id) => {
                // Verify the connection exists before attempting to connect
                let exists = self
                    .state
                    .try_borrow()
                    .ok()
                    .and_then(|s| s.get_connection(*id).cloned())
                    .is_some();
                if exists {
                    tracing::info!(%id, "Startup action: connecting to saved connection");
                    Self::start_connection_with_split(
                        &self.state,
                        &self.terminal_notebook,
                        &self.split_view,
                        &self.sidebar,
                        &self.monitoring,
                        *id,
                        Some(&self.activity_coordinator),
                    );
                } else {
                    tracing::warn!(%id, "Startup action: connection not found, skipping");
                    self.toast_overlay
                        .show_warning(&crate::i18n::i18n("Startup connection not found"));
                }
            }
            StartupAction::RdpFile(path) => {
                tracing::info!(path = %path.display(), "Startup action: opening .rdp file");
                match rustconn_core::import::RdpFileImporter::parse_rdp_file(path) {
                    Ok(connection) => {
                        let devices = rdp_file_requested_devices(&connection);
                        let host = connection.host.clone();
                        // Add the imported connection to state and connect
                        let state = self.state.clone();
                        let notebook = self.terminal_notebook.clone();
                        let split_view = self.split_view.clone();
                        let sidebar = Rc::clone(&self.sidebar);
                        let monitoring = Rc::clone(&self.monitoring);
                        let activity = self.activity_coordinator.clone();
                        let connect = move |connection: rustconn_core::models::Connection| {
                            let conn_id = connection.id;
                            if let Ok(mut state_mut) = state.try_borrow_mut()
                                && let Err(e) = state_mut.create_connection(connection)
                            {
                                tracing::error!(%e, "Failed to add imported .rdp connection");
                            }
                            Self::start_connection_with_split(
                                &state,
                                &notebook,
                                &split_view,
                                &sidebar,
                                &monitoring,
                                conn_id,
                                Some(&activity),
                            );
                            let state_clone = state.clone();
                            let sidebar_clone = Rc::clone(&sidebar);
                            glib::idle_add_local_once(move || {
                                Self::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                            });
                        };
                        if devices.is_empty() {
                            connect(connection);
                        } else {
                            // A double-clicked .rdp file is untrusted input: one
                            // that turns on device redirection is the shape of
                            // rogue-RDP phishing, so ask first, as mstsc does for
                            // unsigned files. The import dialog needs no prompt —
                            // the user reviews the result there.
                            let file_name = path.file_name().map_or_else(
                                || path.display().to_string(),
                                |name| name.to_string_lossy().into_owned(),
                            );
                            let dialog = rdp_file_devices_dialog(&file_name, &host, &devices);
                            let pending = std::cell::Cell::new(Some(connection));
                            dialog.connect_response(None, move |_, response| {
                                if response != "connect" {
                                    tracing::info!(
                                        "Startup .rdp file declined: device redirection not confirmed"
                                    );
                                    return;
                                }
                                if let Some(connection) = pending.take() {
                                    connect(connection);
                                }
                            });
                            dialog.present(Some(&self.window));
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            ?e,
                            path = %path.display(),
                            "Failed to parse .rdp file"
                        );
                        self.toast_overlay
                            .show_warning(&crate::i18n::i18n("Failed to open .rdp file"));
                    }
                }
            }
            StartupAction::VvFile(path) => {
                tracing::info!(path = %path.display(), "Startup action: opening .vv file");
                match rustconn_core::import::VirtViewerImporter::parse_vv_file(path) {
                    Ok(connection) => {
                        let conn_id = connection.id;
                        if let Ok(mut state_mut) = self.state.try_borrow_mut()
                            && let Err(e) = state_mut.create_connection(connection)
                        {
                            tracing::error!(%e, "Failed to add imported .vv connection");
                        }
                        Self::start_connection_with_split(
                            &self.state,
                            &self.terminal_notebook,
                            &self.split_view,
                            &self.sidebar,
                            &self.monitoring,
                            conn_id,
                            Some(&self.activity_coordinator),
                        );
                        let state_clone = self.state.clone();
                        let sidebar_clone = Rc::clone(&self.sidebar);
                        glib::idle_add_local_once(move || {
                            Self::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                        });
                    }
                    Err(e) => {
                        tracing::error!(
                            ?e,
                            path = %path.display(),
                            "Failed to parse .vv file"
                        );
                        self.toast_overlay
                            .show_warning(&crate::i18n::i18n("Failed to open .vv file"));
                    }
                }
            }
        }

        // Auto-start standalone tunnels (runs regardless of startup action)
        Self::auto_start_tunnels(&self.state, &self.tunnel_manager);

        // Health check polling for standalone tunnels (every 5 seconds)
        {
            let tm = self.tunnel_manager.clone();
            let state_c = self.state.clone();
            let toast_c = self.toast_overlay.clone();
            glib::timeout_add_local(std::time::Duration::from_secs(5), move || {
                let failed = tm.borrow_mut().health_check();
                if !failed.is_empty() {
                    // Auto-reconnect failed tunnels
                    let tunnels = state_c.borrow().settings().standalone_tunnels.clone();
                    let connections: Vec<_> = state_c
                        .borrow()
                        .list_connections()
                        .into_iter()
                        .cloned()
                        .collect();
                    for failure in &failed {
                        let id = &failure.id;
                        let Some(tunnel) = tunnels.iter().find(|t| t.id == *id) else {
                            continue;
                        };

                        // A tunnel that dies is a background failure the user can
                        // retry, so it gets a toast (gnome-hig.md). Until now the
                        // only trace was a log line: the row simply left the
                        // Active group, which looks the same as stopping it.
                        if !tunnel.auto_reconnect || !tunnel.enabled {
                            toast_c.show_error(&crate::i18n::i18n_f(
                                "Tunnel “{}” stopped: {}",
                                &[&tunnel.name, &failure.reason],
                            ));
                            continue;
                        }

                        // Check if tunnel exceeded max reconnect attempts
                        if tm.borrow().exceeded_max_reconnects(*id) {
                            tracing::warn!(
                                tunnel = %tunnel.name,
                                tunnel_id = %id,
                                "Tunnel exceeded max reconnect attempts, giving up"
                            );
                            // Giving up is the end of the retry loop, so this is
                            // the last chance to tell the user anything.
                            toast_c.show_error(&crate::i18n::i18n_f(
                                "Gave up reconnecting tunnel “{}”: {}",
                                &[&tunnel.name, &failure.reason],
                            ));
                            continue;
                        }

                        if let Some(conn) =
                            connections.iter().find(|c| c.id == tunnel.connection_id)
                        {
                            tracing::info!(tunnel = %tunnel.name, "Auto-reconnecting failed tunnel");
                            // Resolve cached password for reconnection
                            let cached_pw: Option<secrecy::SecretString> = state_c
                                .try_borrow()
                                .ok()
                                .and_then(|s| s.get_cached_credentials(tunnel.connection_id))
                                .and_then(|c| {
                                    use secrecy::ExposeSecret;
                                    if c.password.expose_secret().is_empty() {
                                        None
                                    } else {
                                        Some(c.password.clone())
                                    }
                                });
                            // The reconnect result used to be discarded, so a
                            // reconnect that could never work — `ssh` missing, the
                            // connection no longer SSH — retried in silence until
                            // the attempt counter ran out.
                            if let Err(e) =
                                tm.borrow_mut().start(tunnel, conn, cached_pw.as_ref(), &[])
                            {
                                tracing::warn!(
                                    tunnel = %tunnel.name,
                                    tunnel_id = %id,
                                    %e,
                                    "Auto-reconnect failed"
                                );
                            }
                        }
                    }

                    // Redraw the tunnel manager if the user has it open. Without
                    // this, the row of a tunnel that just died kept saying
                    // "Running" in the Active group until something else happened
                    // to refresh the list — so the warning icon and the Last Error
                    // row this release added were invisible in the one window
                    // built to show them. A no-op when no dialog is open, which
                    // is why it is safe on a five-second timer.
                    crate::dialogs::tunnel::refresh_open_manager();
                }
                glib::ControlFlow::Continue
            });
        }
    }

    /// Auto-starts standalone tunnels that have `auto_start` and `enabled` set
    pub(super) fn auto_start_tunnels(state: &SharedAppState, tunnel_manager: &SharedTunnelManager) {
        let tunnels = state.borrow().settings().standalone_tunnels.clone();
        let auto_tunnels: Vec<_> = tunnels
            .iter()
            .filter(|t| t.auto_start && t.enabled)
            .collect();

        if auto_tunnels.is_empty() {
            return;
        }

        tracing::info!(
            count = auto_tunnels.len(),
            "Auto-starting standalone tunnels"
        );

        let connections: Vec<_> = state
            .borrow()
            .list_connections()
            .into_iter()
            .cloned()
            .collect();

        for tunnel in auto_tunnels {
            let conn = connections.iter().find(|c| c.id == tunnel.connection_id);
            if let Some(conn) = conn {
                // Resolve cached password for the connection
                let cached_pw: Option<secrecy::SecretString> = state
                    .try_borrow()
                    .ok()
                    .and_then(|s| s.get_cached_credentials(tunnel.connection_id))
                    .and_then(|c| {
                        use secrecy::ExposeSecret;
                        if c.password.expose_secret().is_empty() {
                            None
                        } else {
                            Some(c.password.clone())
                        }
                    });
                if let Err(e) =
                    tunnel_manager
                        .borrow_mut()
                        .start(tunnel, conn, cached_pw.as_ref(), &[])
                {
                    tracing::warn!(tunnel = %tunnel.name, %e, "Failed to auto-start tunnel");
                }
            } else {
                tracing::warn!(
                    tunnel = %tunnel.name,
                    connection_id = %tunnel.connection_id,
                    "SSH connection not found for auto-start tunnel"
                );
            }
        }
    }

    /// Saves the current expanded groups state to settings
    ///
    /// Note: Part of tree state persistence API.
    #[expect(
        dead_code,
        reason = "kept alive for GTK widget lifecycle / future API exposure"
    )]
    pub fn save_expanded_groups(&self) {
        let expanded = self.sidebar.get_expanded_groups();
        if let Some(Err(e)) =
            try_with_state_mut(&self.state, |state| state.update_expanded_groups(expanded))
        {
            tracing::warn!(?e, "Failed to update expanded groups");
        }
    }
}

/// The local devices an imported `.rdp` file asks to redirect into the
/// session, as translated labels for the confirmation dialog. Empty when the
/// file requests none of them.
pub(super) fn rdp_file_requested_devices(
    connection: &rustconn_core::models::Connection,
) -> Vec<String> {
    let rustconn_core::ProtocolConfig::Rdp(rdp) = &connection.protocol_config else {
        return Vec::new();
    };
    // Literal `i18n()` calls, so `po/update-pot.sh` extracts each label.
    let mut devices = Vec::new();
    if rdp.microphone_enabled {
        devices.push(crate::i18n::i18n("Microphone"));
    }
    if rdp.smartcard_enabled {
        devices.push(crate::i18n::i18n("Smart Cards"));
    }
    if rdp.fido2_enabled {
        devices.push(crate::i18n::i18n("Security Key (FIDO2)"));
    }
    devices
}

/// Asks before connecting with an opened `.rdp` file that redirects local
/// devices. Cancel is both the default and the Escape response.
pub(super) fn rdp_file_devices_dialog(
    file_name: &str,
    host: &str,
    devices: &[String],
) -> adw::AlertDialog {
    let list = devices
        .iter()
        .map(|device| format!("• {device}"))
        .collect::<Vec<_>>()
        .join("\n");
    let dialog = adw::AlertDialog::new(
        Some(&crate::i18n::i18n("Share Local Devices?")),
        Some(&crate::i18n::i18n_f(
            "The file {} asks to share these devices with {}:\n\n{}\n\nConnect only if you trust where the file came from.",
            &[file_name, host, &list],
        )),
    );
    dialog.add_response("cancel", &crate::i18n::i18n("Cancel"));
    dialog.add_response("connect", &crate::i18n::i18n("Connect"));
    dialog.set_default_response(Some("cancel"));
    dialog.set_close_response("cancel");
    dialog
}
