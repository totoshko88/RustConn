//! Variables and history window actions
//!
//! Extracted from `window/mod.rs` to reduce module complexity.

use super::*;

impl MainWindow {
    pub(crate) fn setup_variables_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
    ) {
        use crate::dialogs::VariablesDialog;

        // Manage variables action
        let manage_variables_action = gio::SimpleAction::new("manage-variables", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let toast = self.toast_overlay.clone();
        manage_variables_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                // Get current global variables and settings snapshot
                let state_ref = state_clone.borrow();
                let settings_snapshot = state_ref.settings().clone();
                drop(state_ref);

                // Get current global variables without loading secrets from vault.
                // Secret values are loaded on-demand via the "Load from vault" button.
                let current_vars = settings_snapshot.global_variables.clone();

                let dialog = VariablesDialog::new(Some(win.upcast_ref()));
                dialog.set_variables(&current_vars);
                dialog.set_settings(&settings_snapshot);

                let state_for_save = state_clone.clone();
                let toast_for_save = toast.clone();
                let win_weak_for_save = win.downgrade();
                dialog.run(move |result| {
                    if let Some(variables) = result {
                        // Store secret variable values in vault,
                        // then clear their value in settings
                        let mut vars_to_save = variables.clone();
                        let settings = state_for_save.borrow().settings().clone();
                        for var in &vars_to_save {
                            if var.is_secret && !var.value.is_empty() {
                                // Skip saving to vault if variable uses a custom KeePass
                                // entry path — the entry already exists in the database
                                if var
                                    .kdbx_entry_path
                                    .as_ref()
                                    .is_some_and(|p| !p.trim().is_empty())
                                {
                                    continue;
                                }
                                // Skip saving to vault if variable references an existing
                                // vault entry by name — the entry is managed externally
                                // and should not be duplicated under rustconn/var/ (#166)
                                if var
                                    .vault_entry_name
                                    .as_ref()
                                    .is_some_and(|n| !n.trim().is_empty())
                                {
                                    tracing::debug!(
                                        var_name = %var.name,
                                        "Skipping vault write — variable uses external vault entry"
                                    );
                                    continue;
                                }
                                // Wrap into SecretString immediately so plaintext
                                // does not live in the closure as a plain String.
                                let pwd = secrecy::SecretString::from(var.value.clone());
                                let var_name = var.name.clone();
                                let var_name_log = var_name.clone();
                                let secrets_c = settings.secrets.clone();
                                let toast_c = toast_for_save.clone();
                                let win_weak_c = win_weak_for_save.clone();
                                crate::utils::spawn_blocking_with_callback(
                                    move || {
                                        crate::state::save_variable_to_vault(
                                            &secrets_c, &var_name, &pwd,
                                        )
                                    },
                                    move |result: Result<(), String>| {
                                        if let Err(e) = result {
                                            tracing::error!(
                                                "Failed to save secret \
                                                 variable '{var_name_log}' \
                                                 to vault: {e}"
                                            );
                                            // Data-loss risk: the plaintext value is cleared
                                            // from settings after this spawn, so a failed
                                            // vault write means the secret is gone. Use a
                                            // blocking AlertDialog (GNOME HIG) so the user
                                            // must acknowledge and can re-enter it — a
                                            // transient toast could be missed.
                                            if let Some(win) = win_weak_c.upgrade() {
                                                crate::alert::show_error(
                                                    &win,
                                                    &crate::i18n::i18n("Secret Not Saved"),
                                                    &crate::i18n::i18n_f(
                                                        "The secret for variable “{}” could not be saved to the vault and was not stored. Re-enter it and check your secret backend in Settings.",
                                                        &[&var_name_log],
                                                    ),
                                                );
                                            } else {
                                                toast_c.show_error(&crate::i18n::i18n(
                                                    "Failed to save secret to vault. Check secret backend in Settings.",
                                                ));
                                            }
                                        } else {
                                            tracing::info!(
                                                "Secret variable \
                                                 '{var_name_log}' saved \
                                                 to vault"
                                            );
                                        }
                                    },
                                );
                            }
                        }

                        // Clear secret variable values before persisting
                        // to disk — the actual values live in the vault
                        for var in &mut vars_to_save {
                            if var.is_secret {
                                var.value.clear();
                            }
                        }

                        // Save variables to settings
                        let mut state_ref = state_for_save.borrow_mut();
                        state_ref.settings_mut().global_variables = vars_to_save.clone();

                        // A saved secret may differ from a cached copy, so drop
                        // the resolved-secret cache — the next connect re-reads
                        // from the vault (P1).
                        crate::vault_ops::invalidate_global_secret_cache();

                        // Persist to disk
                        let save_error = match state_ref
                            .config_manager()
                            .save_variables(&vars_to_save)
                        {
                            Ok(()) => {
                                tracing::info!("Saved {} global variables", vars_to_save.len());
                                None
                            }
                            Err(e) => {
                                tracing::error!("Failed to save variables: {e}");
                                Some(e.to_string())
                            }
                        };
                        // Release the borrow before presenting anything: the
                        // dialog's response runs as a GTK callback and must not
                        // find this still held (window-guide.md).
                        drop(state_ref);

                        // This is a save failure on user data, so it gets a
                        // dialog, not a toast. Settings in memory were already
                        // updated, so without this the edits look saved and are
                        // gone on the next start — the same data-loss reasoning
                        // the vault write above already follows.
                        if let Some(reason) = save_error {
                            if let Some(win) = win_weak_for_save.upgrade() {
                                crate::alert::show_error(
                                    &win,
                                    &crate::i18n::i18n("Variables Not Saved"),
                                    &crate::i18n::i18n_f(
                                        "Your changes could not be written to disk and will be lost when RustConn closes: {}",
                                        &[&reason],
                                    ),
                                );
                            } else {
                                toast_for_save.show_error(&crate::i18n::i18n(
                                    "Global variables could not be saved to disk.",
                                ));
                            }
                        }
                    }
                });
            }
        });
        window.add_action(&manage_variables_action);
    }

    /// Sets up history and statistics actions
    pub(crate) fn setup_history_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
    ) {
        use crate::dialogs::{
            HistoryDialog, LogViewerDialog, StatisticsDialog, show_password_generator_dialog,
        };

        // Session logs action — issue #247: without an entry point the log files
        // were effectively invisible, whatever the settings said.
        let show_logs_action = gio::SimpleAction::new("show-logs", None);
        let window_weak = window.downgrade();
        let state_for_logs = state.clone();
        show_logs_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade()
                && let Some(log_dir) = Self::managed_log_dir(&state_for_logs)
            {
                LogViewerDialog::new(Some(win.upcast_ref::<gtk4::Window>()), log_dir).show();
            }
        });
        window.add_action(&show_logs_action);

        // Session log of the selected connection — a per-connection path template
        // can point anywhere, so this opens the viewer where that connection
        // actually writes (issue #247).
        let show_connection_log_action = gio::SimpleAction::new("show-connection-log", None);
        let window_weak = window.downgrade();
        let state_for_conn_log = state.clone();
        let sidebar_for_conn_log = self.sidebar.clone();
        show_connection_log_action.connect_activate(move |_, _| {
            let Some(win) = window_weak.upgrade() else {
                return;
            };
            let log_dir = sidebar_for_conn_log
                .get_selected_item()
                .filter(|item| !item.is_group())
                .and_then(|item| Uuid::parse_str(&item.id()).ok())
                .and_then(|id| Self::session_log_dir(&state_for_conn_log, id))
                .or_else(|| Self::managed_log_dir(&state_for_conn_log));
            if let Some(log_dir) = log_dir {
                LogViewerDialog::new(Some(win.upcast_ref::<gtk4::Window>()), log_dir).show();
            }
        });
        window.add_action(&show_connection_log_action);

        // Show history action
        let show_history_action = gio::SimpleAction::new("show-history", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = self.terminal_notebook.clone();
        let sidebar_clone = self.sidebar.clone();
        let split_view_clone = self.split_view.clone();
        let monitoring_clone = self.monitoring.clone();
        let activity_clone_hist = self.activity_coordinator.clone();
        show_history_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let state_ref = state_clone.borrow();
                let entries = state_ref.history_entries().to_vec();
                drop(state_ref);

                let dialog = HistoryDialog::new(Some(&win));
                dialog.set_entries(entries);

                // Connect callback for reconnecting from history
                let state_for_connect = state_clone.clone();
                let notebook_for_connect = notebook_clone.clone();
                let sidebar_for_connect = sidebar_clone.clone();
                let split_view_for_connect = split_view_clone.clone();
                let monitoring_for_connect = monitoring_clone.clone();
                let activity_for_connect = activity_clone_hist.clone();
                dialog.connect_on_connect(move |entry| {
                    if entry.is_quick_connect() {
                        tracing::warn!("Cannot reconnect to quick connect from history");
                    } else {
                        tracing::info!(
                            "Reconnecting to {} (id: {}) from history",
                            entry.connection_name,
                            entry.connection_id
                        );
                        Self::start_connection_with_credential_resolution(
                            state_for_connect.clone(),
                            notebook_for_connect.clone(),
                            split_view_for_connect.clone(),
                            sidebar_for_connect.clone(),
                            monitoring_for_connect.clone(),
                            entry.connection_id,
                            Some(activity_for_connect.clone()),
                        );
                    }
                });

                dialog.present();
            }
        });
        window.add_action(&show_history_action);

        // Show statistics action
        let show_statistics_action = gio::SimpleAction::new("show-statistics", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        show_statistics_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let state_ref = state_clone.borrow();
                let all_stats = state_ref.get_all_statistics();
                drop(state_ref);

                let dialog = StatisticsDialog::new(Some(&win));
                dialog.set_overview_statistics(&all_stats);

                // Connect clear statistics callback
                let state_for_clear = state_clone.clone();
                dialog.connect_on_clear(move || {
                    if let Ok(mut state_mut) = state_for_clear.try_borrow_mut() {
                        state_mut.clear_all_statistics();
                        tracing::info!("All connection statistics cleared");
                    }
                });

                dialog.present();
            }
        });
        window.add_action(&show_statistics_action);

        // Password generator action
        let password_generator_action = gio::SimpleAction::new("password-generator", None);
        let window_weak = window.downgrade();
        password_generator_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                show_password_generator_dialog(Some(&win));
            }
        });
        window.add_action(&password_generator_action);

        // Wake On LAN dialog action
        let wol_dialog_action = gio::SimpleAction::new("wake-on-lan-dialog", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        wol_dialog_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let state_ref = state_clone.borrow();
                let connections: Vec<rustconn_core::models::Connection> =
                    state_ref.list_connections_owned();
                drop(state_ref);

                let dialog = crate::dialogs::WolDialog::new();
                dialog.set_connections(&connections);
                dialog.present(&win);
            }
        });
        window.add_action(&wol_dialog_action);

        // Network scan dialog action. A host it finds becomes a connection
        // named after its address, at the top level of the connection tree.
        let network_scan_action = gio::SimpleAction::new("network-scan", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = self.sidebar.clone();
        network_scan_action.connect_activate(move |_, _| {
            let Some(win) = window_weak.upgrade() else {
                return;
            };
            let state = state_clone.clone();
            let sidebar = sidebar_clone.clone();
            let window_for_toast = win.downgrade();
            let dialog = crate::dialogs::NetworkScanDialog::new(move |address, port| {
                let host = address.to_string();
                let Some(protocol) = port.protocol else {
                    return false;
                };
                let connection = match protocol {
                    "ssh" => rustconn_core::Connection::new_ssh(host.clone(), host, port.port),
                    "rdp" => rustconn_core::Connection::new_rdp(host.clone(), host, port.port),
                    "vnc" => rustconn_core::Connection::new_vnc(host.clone(), host, port.port),
                    "telnet" => {
                        rustconn_core::Connection::new_telnet(host.clone(), host, port.port)
                    }
                    other => {
                        tracing::warn!(protocol = other, "network scan: no connection type");
                        return false;
                    }
                };
                let name = connection.name.clone();
                let created = match state.try_borrow_mut() {
                    Ok(mut state_mut) => state_mut.create_connection(connection),
                    Err(_) => Err("state busy".to_string()),
                };
                let window = window_for_toast.upgrade();
                match created {
                    Ok(connection_id) => {
                        tracing::info!(%connection_id, protocol, "connection added from network scan");
                        Self::reload_sidebar_preserving_state(&state, &sidebar);
                        if let Some(window) = window {
                            crate::toast::show_toast_on_window(
                                &window,
                                &crate::i18n::i18n_f("Connection “{}” added", &[&name]),
                                crate::toast::ToastType::Success,
                            );
                        }
                        true
                    }
                    Err(error) => {
                        tracing::error!(%error, "network scan: failed to add connection");
                        if let Some(window) = window {
                            crate::toast::show_toast_on_window(
                                &window,
                                &crate::i18n::i18n("Failed to add the connection"),
                                crate::toast::ToastType::Error,
                            );
                        }
                        false
                    }
                }
            });
            dialog.present(&win);
        });
        window.add_action(&network_scan_action);

        // Manage recordings action
        let manage_recordings_action = gio::SimpleAction::new("manage-recordings", None);
        let window_weak = window.downgrade();
        let notebook_for_playback = self.terminal_notebook.clone();
        manage_recordings_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let dialog = std::rc::Rc::new(crate::dialogs::RecordingsDialog::new(Some(
                    win.upcast_ref(),
                )));

                // on_delete: log deletion (row already removed inline by dialog)
                dialog.set_on_delete(|path| {
                    tracing::info!(?path, "Recording deleted");
                });

                // on_rename: log rename (label already updated inline by dialog)
                dialog.set_on_rename(|path, new_name| {
                    tracing::info!(?path, %new_name, "Recording renamed");
                });

                // on_import: refresh list after import
                let dialog_for_import = dialog.clone();
                dialog.set_on_import(move || {
                    tracing::info!("Recording imported, refreshing list");
                    dialog_for_import.refresh_list();
                });

                // on_play: open a Playback Tab for the selected recording
                let notebook_clone = notebook_for_playback.clone();
                dialog.set_on_play(move |entry| {
                    tracing::info!(
                        name = %entry.metadata.connection_name,
                        path = ?entry.data_path,
                        "Opening playback tab"
                    );
                    notebook_clone.open_playback_tab(&entry);
                });

                dialog.present();
            }
        });
        window.add_action(&manage_recordings_action);
    }
}
