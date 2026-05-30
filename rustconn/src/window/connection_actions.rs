//! Connection-related window actions (new, import, export, settings, quick connect)
//!
//! Extracted from `window/mod.rs` to reduce module complexity.

use super::*;

impl MainWindow {
    pub(crate) fn setup_connection_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
        notebook: &SharedNotebook,
    ) {
        // New connection action — opens Connection Wizard (Ctrl+N)
        let new_conn_action = gio::SimpleAction::new("new-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let toast_clone = self.toast_overlay.clone();
        new_conn_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_connection_wizard(
                    win.upcast_ref(),
                    state_clone.clone(),
                    sidebar_clone.clone(),
                    toast_clone.clone(),
                );
            }
        });
        window.add_action(&new_conn_action);

        // New connection advanced — opens full ConnectionDialog directly
        let new_conn_advanced_action = gio::SimpleAction::new("new-connection-advanced", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        new_conn_advanced_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_new_connection_dialog(&win, state_clone.clone(), sidebar_clone.clone());
            }
        });
        window.add_action(&new_conn_advanced_action);

        // New connection in group action (pre-selects the currently selected group)
        let new_conn_in_group_action = gio::SimpleAction::new("new-connection-in-group", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        new_conn_in_group_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                if let Some(item) = sidebar_clone.get_selected_item()
                    && let Ok(group_id) = uuid::Uuid::parse_str(&item.id())
                {
                    connection_dialogs::show_new_connection_dialog_in_group(
                        win.upcast_ref(),
                        state_clone.clone(),
                        sidebar_clone.clone(),
                        group_id,
                    );
                    return;
                }
                // Fallback: open without group pre-selection
                connection_dialogs::show_new_connection_dialog(
                    win.upcast_ref(),
                    state_clone.clone(),
                    sidebar_clone.clone(),
                );
            }
        });
        window.add_action(&new_conn_in_group_action);

        // Connect all connections in a group (including nested subgroups)
        let connect_all_action = gio::SimpleAction::new("connect-all-in-group", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = self.terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        let monitoring_clone = self.monitoring.clone();
        let activity_clone_all = self.activity_coordinator.clone();
        connect_all_action.connect_activate(move |_, _| {
            let Some(item) = sidebar_clone.get_selected_item() else {
                return;
            };
            if !item.is_group() {
                return;
            }
            let Ok(group_id) = uuid::Uuid::parse_str(&item.id()) else {
                return;
            };
            // Collect all descendant group IDs (including the selected group itself)
            let conn_ids: Vec<uuid::Uuid> = {
                let Ok(state_ref) = state_clone.try_borrow() else {
                    return;
                };
                let groups = state_ref.list_groups();
                let mut descendant_ids = std::collections::HashSet::new();
                descendant_ids.insert(group_id);
                let mut to_process = vec![group_id];
                while let Some(current) = to_process.pop() {
                    for g in &groups {
                        if g.parent_id == Some(current) && descendant_ids.insert(g.id) {
                            to_process.push(g.id);
                        }
                    }
                }
                state_ref
                    .list_connections()
                    .into_iter()
                    .filter(|c| c.group_id.is_some_and(|gid| descendant_ids.contains(&gid)))
                    .map(|c| c.id)
                    .collect()
            };
            for conn_id in conn_ids {
                Self::start_connection_with_credential_resolution(
                    state_clone.clone(),
                    notebook_clone.clone(),
                    split_view_clone.clone(),
                    sidebar_clone.clone(),
                    monitoring_clone.clone(),
                    conn_id,
                    Some(activity_clone_all.clone()),
                );
            }
        });
        window.add_action(&connect_all_action);

        // Sync Now action — exports Master groups, imports Import groups
        let sync_now_action = gio::SimpleAction::new("sync-now", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let toast_clone = self.toast_overlay.clone();
        sync_now_action.connect_activate(move |_, _| {
            let Some(item) = sidebar_clone.get_selected_item() else {
                return;
            };
            if !item.is_group() {
                return;
            }
            let Ok(group_id) = uuid::Uuid::parse_str(&item.id()) else {
                return;
            };

            // Walk up to root group to find the sync-enabled ancestor
            let root_group_id = {
                let Ok(state_ref) = state_clone.try_borrow() else {
                    return;
                };
                let groups = state_ref.list_groups();
                let mut current_id = group_id;
                loop {
                    if let Some(group) = groups.iter().find(|g| g.id == current_id) {
                        if group.parent_id.is_none() {
                            break current_id;
                        }
                        if let Some(pid) = group.parent_id {
                            current_id = pid;
                        } else {
                            break current_id;
                        }
                    } else {
                        break group_id;
                    }
                }
            };

            match state_clone.try_borrow_mut() {
                Ok(mut state_mut) => {
                    match state_mut.sync_now_group(root_group_id) {
                        Ok(report) => {
                            let msg = crate::i18n::i18n_f(
                                "Synced '{}': +{} connections, ~{} updated, -{} removed",
                                &[
                                    &report.group_name,
                                    &report.connections_added.to_string(),
                                    &report.connections_updated.to_string(),
                                    &report.connections_removed.to_string(),
                                ],
                            );
                            toast_clone.show_success(&msg);
                            // Reload sidebar to reflect changes
                            drop(state_mut);
                            Self::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                        }
                        Err(e) => {
                            let error_str = e.clone();
                            let msg = if error_str.contains("not configured")
                                && rustconn_core::flatpak::is_flatpak()
                            {
                                crate::i18n::i18n(
                                    "Sync directory is not configured. In Flatpak, grant filesystem access first: flatpak override --user --filesystem=/path/to/sync io.github.totoshko88.RustConn",
                                )
                            } else if error_str.contains("not writable")
                                && rustconn_core::flatpak::is_flatpak()
                            {
                                crate::i18n::i18n_f(
                                    "Sync directory is not accessible from Flatpak sandbox. Run: flatpak override --user --filesystem={} io.github.totoshko88.RustConn",
                                    &[&error_str],
                                )
                            } else {
                                crate::i18n::i18n_f("Sync failed: {}", &[&error_str])
                            };
                            toast_clone.show_error(&msg);
                        }
                    }
                }
                Err(_) => {
                    toast_clone.show_error(&crate::i18n::i18n("Sync failed: state is busy"));
                }
            }
        });
        window.add_action(&sync_now_action);

        // Refresh Dynamic Folder action — executes the script and updates connections
        let refresh_dynamic_action = gio::SimpleAction::new("refresh-dynamic-folder", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let toast_clone = self.toast_overlay.clone();
        refresh_dynamic_action.connect_activate(move |_, _| {
            let Some(item) = sidebar_clone.get_selected_item() else {
                return;
            };
            if !item.is_group() {
                return;
            }
            let Ok(group_id) = uuid::Uuid::parse_str(&item.id()) else {
                return;
            };

            let config = {
                let Ok(state_ref) = state_clone.try_borrow() else {
                    return;
                };
                let Some(group) = state_ref.get_group(group_id) else {
                    return;
                };
                group.dynamic_folder.clone()
            };

            let Some(config) = config else {
                return;
            };

            let state_for_task = state_clone.clone();
            let sidebar_for_task = sidebar_clone.clone();
            let toast_for_task = toast_clone.clone();
            let group_name = item.name();

            // Run the script asynchronously
            crate::utils::spawn_blocking_with_callback(
                move || {
                    crate::async_utils::with_runtime(|rt| {
                        rt.block_on(rustconn_core::dynamic_folder::execute_script(&config))
                    })
                },
                move |result| {
                    let result = match result {
                        Ok(inner) => inner,
                        Err(rt_err) => {
                            let msg = crate::i18n::i18n_f(
                                "Dynamic folder '{}' failed: {}",
                                &[&group_name, &rt_err],
                            );
                            toast_for_task.show_error(&msg);
                            return;
                        }
                    };
                    match result {
                        Ok(folder_result) => {
                            let count = folder_result.entries.len();
                            let warnings = folder_result.warnings.clone();

                            // Convert entries to connections and update state
                            if let Ok(mut state_mut) = state_for_task.try_borrow_mut() {
                                // Remove old dynamic connections for this group
                                let old_dynamic: Vec<uuid::Uuid> = state_mut
                                    .get_connections_by_group(group_id)
                                    .iter()
                                    .filter(|c| c.is_dynamic)
                                    .map(|c| c.id)
                                    .collect();
                                for conn_id in old_dynamic {
                                    // best-effort: failure to remove a stale dynamic
                                    // connection should not block the refresh; the
                                    // overall result is still surfaced via toast.
                                    if let Err(e) = state_mut
                                        .connection_manager()
                                        .delete_connection(conn_id)
                                    {
                                        tracing::warn!(
                                            connection = %conn_id,
                                            error = %e,
                                            "failed to remove stale dynamic connection"
                                        );
                                    }
                                }

                                // Add new dynamic connections
                                for entry in &folder_result.entries {
                                    let conn = rustconn_core::dynamic_folder::entry_to_connection(
                                        entry, group_id,
                                    );
                                    if let Err(e) = state_mut.create_connection(conn) {
                                        tracing::warn!(
                                            error = %e,
                                            "failed to create dynamic connection during refresh"
                                        );
                                    }
                                }

                                // Update group's last_refreshed_at
                                if let Some(mut group) = state_mut.get_group(group_id).cloned()
                                    && let Some(ref mut df) = group.dynamic_folder
                                {
                                    df.last_refreshed_at = Some(chrono::Utc::now());
                                    df.last_error = None;
                                    if let Err(e) = state_mut
                                        .connection_manager()
                                        .update_group(group_id, group)
                                    {
                                        tracing::warn!(
                                            group = %group_id,
                                            error = %e,
                                            "failed to record dynamic folder refresh timestamp"
                                        );
                                    }
                                }

                                drop(state_mut);
                                Self::reload_sidebar_preserving_state(
                                    &state_for_task,
                                    &sidebar_for_task,
                                );
                            }

                            // Show warnings if any
                            for warning in &warnings {
                                tracing::warn!(group = %group_name, %warning, "Dynamic folder warning");
                            }

                            let msg = crate::i18n::i18n_f(
                                "Refreshed '{}': {} connections generated",
                                &[&group_name, &count.to_string()],
                            );
                            toast_for_task.show_success(&msg);
                        }
                        Err(e) => {
                            let error_msg = e.to_string();
                            tracing::error!(group = %group_name, error = %error_msg, "Dynamic folder refresh failed");

                            // Update group's last_error
                            if let Ok(mut state_mut) = state_for_task.try_borrow_mut()
                                && let Some(mut group) = state_mut.get_group(group_id).cloned()
                                && let Some(ref mut df) = group.dynamic_folder
                            {
                                df.last_error = Some(error_msg.clone());
                                if let Err(e) = state_mut
                                    .connection_manager()
                                    .update_group(group_id, group)
                                {
                                    tracing::warn!(
                                        group = %group_id,
                                        error = %e,
                                        "failed to record dynamic folder error state"
                                    );
                                }
                            }

                            let msg = crate::i18n::i18n_f(
                                "Dynamic folder '{}' failed: {}",
                                &[&group_name, &error_msg],
                            );
                            toast_for_task.show_error(&msg);
                        }
                    }
                },
            );
        });
        window.add_action(&refresh_dynamic_action);

        // New connection from connection context (pre-selects the group of the selected connection)
        let new_conn_from_ctx_action = gio::SimpleAction::new("new-connection-from-context", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        new_conn_from_ctx_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                // Try to get group_id from the selected connection
                let selected = sidebar_clone.get_selected_item();
                let group_id = selected.as_ref().and_then(|item| {
                    let id_str = item.id();
                    let is_group = item.is_group();
                    tracing::debug!(
                        id = %id_str,
                        is_group,
                        "new-connection-from-context: selected item"
                    );
                    let conn_id = uuid::Uuid::parse_str(&id_str).ok()?;
                    if is_group {
                        // If user right-clicked a group, use the group ID directly
                        Some(conn_id)
                    } else {
                        // If user right-clicked a connection, get its group_id
                        state_clone.try_borrow().ok().and_then(|s| {
                            let conn = s.get_connection(conn_id);
                            tracing::debug!(
                                found = conn.is_some(),
                                group_id = ?conn.and_then(|c| c.group_id),
                                "new-connection-from-context: connection lookup"
                            );
                            conn.and_then(|c| c.group_id)
                        })
                    }
                });
                if let Some(gid) = group_id {
                    connection_dialogs::show_new_connection_dialog_in_group(
                        win.upcast_ref(),
                        state_clone.clone(),
                        sidebar_clone.clone(),
                        gid,
                    );
                } else {
                    connection_dialogs::show_new_connection_dialog(
                        win.upcast_ref(),
                        state_clone.clone(),
                        sidebar_clone.clone(),
                    );
                }
            }
        });
        window.add_action(&new_conn_from_ctx_action);

        // New group action
        let new_group_action = gio::SimpleAction::new("new-group", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        new_group_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_new_group_dialog(
                    win.upcast_ref(),
                    state_clone.clone(),
                    sidebar_clone.clone(),
                );
            }
        });
        window.add_action(&new_group_action);

        // Import action
        let import_action = gio::SimpleAction::new("import", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        import_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_import_dialog(
                    win.upcast_ref(),
                    state_clone.clone(),
                    sidebar_clone.clone(),
                );
            }
        });
        window.add_action(&import_action);

        // Settings action
        let settings_action = gio::SimpleAction::new("settings", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let notebook_clone = notebook.clone();
        let monitoring_clone = self.monitoring.clone();
        let sidebar_clone = sidebar.clone();
        let overlay_split_view_clone = self.overlay_split_view.clone();
        settings_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_settings_dialog(
                    win.upcast_ref(),
                    state_clone.clone(),
                    notebook_clone.clone(),
                    monitoring_clone.clone(),
                    sidebar_clone.clone(),
                    overlay_split_view_clone.clone(),
                );
            }
        });
        window.add_action(&settings_action);

        // Flatpak Components action - only functional in Flatpak environment
        let flatpak_components_action = gio::SimpleAction::new("flatpak-components", None);
        let window_weak = window.downgrade();
        flatpak_components_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade()
                && let Some(dialog) = crate::dialogs::FlatpakComponentsDialog::new(Some(&win))
            {
                dialog.present();
            }
        });
        // Only enable inside a sandbox (snap or Flatpak), where CLI tools are
        // downloaded into the app's writable data dir.
        flatpak_components_action.set_enabled(rustconn_core::is_sandboxed());
        window.add_action(&flatpak_components_action);

        // SSH Tunnels action — opens the standalone tunnel manager window
        let ssh_tunnels_action = gio::SimpleAction::new("ssh-tunnels", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let tunnel_manager_clone = self.tunnel_manager.clone();
        ssh_tunnels_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                let manager = crate::dialogs::TunnelManagerWindow::new(
                    Some(win.upcast_ref()),
                    state_clone.clone(),
                    tunnel_manager_clone.clone(),
                );
                manager.present();
            }
        });
        window.add_action(&ssh_tunnels_action);

        // Open password vault action - opens the configured password manager
        let open_keepass_action = gio::SimpleAction::new("open-keepass", None);
        let state_clone = state.clone();
        open_keepass_action.connect_activate(move |_, _| {
            let state_ref = state_clone.borrow();
            let settings = state_ref.settings();
            let backend = settings.secrets.preferred_backend;
            let passbolt_url = settings.secrets.passbolt_server_url.clone();
            drop(state_ref);

            // Open the password manager for the configured backend
            if let Err(e) =
                rustconn_core::secret::open_password_manager(&backend, passbolt_url.as_deref())
            {
                tracing::error!(%e, "Failed to open password manager");
            }
        });
        // Enable based on backend type - always enabled for libsecret/bitwarden/1password,
        // for KeePassXC/KdbxFile requires kdbx_enabled and valid path
        let settings = state.borrow().settings().clone();
        let action_enabled = match settings.secrets.preferred_backend {
            rustconn_core::config::SecretBackendType::LibSecret
            | rustconn_core::config::SecretBackendType::MacOsKeychain
            | rustconn_core::config::SecretBackendType::Bitwarden
            | rustconn_core::config::SecretBackendType::OnePassword
            | rustconn_core::config::SecretBackendType::Passbolt
            | rustconn_core::config::SecretBackendType::Pass => true,
            rustconn_core::config::SecretBackendType::KeePassXc
            | rustconn_core::config::SecretBackendType::KdbxFile => {
                settings.secrets.kdbx_enabled
                    && settings
                        .secrets
                        .kdbx_path
                        .as_ref()
                        .is_some_and(|p| p.exists())
            }
        };
        open_keepass_action.set_enabled(action_enabled);
        window.add_action(&open_keepass_action);

        // Export action
        let export_action = gio::SimpleAction::new("export", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        export_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::show_export_dialog(win.upcast_ref(), state_clone.clone());
            }
        });
        window.add_action(&export_action);
    }
}
