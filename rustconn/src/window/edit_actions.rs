//! Edit-related window actions (connect, edit, delete, rename, duplicate, SFTP)
//!
//! Extracted from `window/mod.rs` to reduce module complexity.

use super::*;

impl MainWindow {
    pub(crate) fn setup_edit_actions(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        // Connect action
        let connect_action = gio::SimpleAction::new("connect", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_clone = self.terminal_notebook.clone();
        let monitoring_clone = self.monitoring.clone();
        connect_action.connect_activate(move |_, _| {
            Self::connect_selected(
                &state_clone,
                &sidebar_clone,
                &notebook_clone,
                &monitoring_clone,
            );
        });
        window.add_action(&connect_action);

        // Edit connection action
        let edit_action = gio::SimpleAction::new("edit-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        edit_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::edit_selected_connection(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&edit_action);

        // Delete connection action
        let delete_action = gio::SimpleAction::new("delete-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        delete_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::delete_selected_connection(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&delete_action);

        // Duplicate connection action
        let duplicate_action = gio::SimpleAction::new("duplicate-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        duplicate_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::duplicate_selected_connection(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&duplicate_action);

        // Toggle pin action
        let toggle_pin_action = gio::SimpleAction::new("toggle-pin", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        toggle_pin_action.connect_activate(move |_, _| {
            Self::toggle_pin_selected(&state_clone, &sidebar_clone);
        });
        window.add_action(&toggle_pin_action);

        // Move to group action
        let move_to_group_action = gio::SimpleAction::new("move-to-group", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        move_to_group_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                groups::show_move_to_group_dialog(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&move_to_group_action);

        // Copy username to clipboard
        let copy_username_action = gio::SimpleAction::new("copy-username", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let window_weak = window.downgrade();
        let toast_clone = self.toast_overlay.clone();
        copy_username_action.connect_activate(move |_, _| {
            let Some(item) = sidebar_clone.get_selected_item() else {
                return;
            };
            if item.is_group() {
                return;
            }
            let Ok(conn_id) = uuid::Uuid::parse_str(&item.id()) else {
                return;
            };
            let Ok(state_ref) = state_clone.try_borrow() else {
                return;
            };
            if let Some(conn) = state_ref.get_connection(conn_id) {
                // Try cached credentials first (resolved from vault during connection),
                // fall back to the username stored directly on the connection model
                let username = state_ref
                    .get_cached_credentials(conn_id)
                    .map(|creds| creds.username.clone())
                    .filter(|u| !u.is_empty())
                    .or_else(|| conn.username.clone())
                    .unwrap_or_default();
                if username.is_empty() {
                    toast_clone.show_warning(&crate::i18n::i18n("No username configured"));
                } else if let Some(win) = window_weak.upgrade() {
                    gtk4::prelude::WidgetExt::display(&win)
                        .clipboard()
                        .set_text(&username);
                    toast_clone.show_success(&crate::i18n::i18n("Username copied"));
                }
            }
        });
        window.add_action(&copy_username_action);

        // Copy password to clipboard (with auto-clear after 30 seconds)
        let copy_password_action = gio::SimpleAction::new("copy-password", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let window_weak = window.downgrade();
        let toast_clone = self.toast_overlay.clone();
        copy_password_action.connect_activate(move |_, _| {
            let Some(item) = sidebar_clone.get_selected_item() else {
                return;
            };
            if item.is_group() {
                return;
            }
            let Ok(conn_id) = uuid::Uuid::parse_str(&item.id()) else {
                return;
            };
            let Ok(state_ref) = state_clone.try_borrow() else {
                return;
            };
            if state_ref.get_connection(conn_id).is_some() {
                // Try cached credentials (resolved from vault during connection)
                use secrecy::ExposeSecret;
                if let Some(creds) = state_ref.get_cached_credentials(conn_id) {
                    let pw = creds.password.expose_secret();
                    if pw.is_empty() {
                        toast_clone.show_warning(&crate::i18n::i18n("Cached password is empty"));
                    } else {
                        let pw_owned = pw.to_string();
                        if let Some(win) = window_weak.upgrade() {
                            let clipboard = gtk4::prelude::WidgetExt::display(&win).clipboard();
                            clipboard.set_text(&pw_owned);
                            toast_clone.show_success(&crate::i18n::i18n(
                                "Password copied (auto-clears in 30s)",
                            ));
                            // Auto-clear clipboard after 30 seconds only if it still
                            // contains the password we set (don't clobber user data)
                            let clipboard_weak = clipboard.downgrade();
                            glib::timeout_add_seconds_local_once(30, move || {
                                if let Some(cb) = clipboard_weak.upgrade() {
                                    cb.read_text_async(gio::Cancellable::NONE, move |result| {
                                        if let Ok(Some(current)) = result
                                            && current.as_str() == pw_owned
                                            && let Some(cb2) = clipboard_weak.upgrade()
                                        {
                                            cb2.set_text("");
                                        }
                                    });
                                }
                            });
                        }
                    }
                } else {
                    toast_clone
                        .show_warning(&crate::i18n::i18n("Connect first to cache credentials"));
                }
            }
        });
        window.add_action(&copy_password_action);

        // Rename item action (works for both connections and groups)
        let rename_action = gio::SimpleAction::new("rename-item", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        rename_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::rename_selected_item(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&rename_action);

        // Copy connection action
        let copy_connection_action = gio::SimpleAction::new("copy-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        copy_connection_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::copy_selected_connection(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&copy_connection_action);

        // Paste connection action
        let paste_connection_action = gio::SimpleAction::new("paste-connection", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        paste_connection_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::paste_connection(win.upcast_ref(), &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&paste_connection_action);

        // Undo delete action
        let undo_delete_action =
            gio::SimpleAction::new("undo-delete", Some(glib::VariantTy::STRING));
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let window_weak = window.downgrade();
        undo_delete_action.connect_activate(move |_, param| {
            if let Some(param) = param
                && let Some(param_str) = param.get::<String>()
            {
                // Format: "type:uuid"
                let parts: Vec<&str> = param_str.split(':').collect();
                if parts.len() != 2 {
                    return;
                }

                let item_type = parts[0];
                let Ok(id) = Uuid::parse_str(parts[1]) else {
                    return;
                };

                if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                    let result = match item_type {
                        "connection" => state_mut.restore_connection(id),
                        "group" => state_mut.restore_group(id),
                        _ => return,
                    };

                    if result.is_ok() {
                        drop(state_mut);
                        let state = state_clone.clone();
                        let sidebar = sidebar_clone.clone();

                        // Reload sidebar
                        glib::idle_add_local_once(move || {
                            Self::reload_sidebar_preserving_state(&state, &sidebar);
                        });

                        // Show confirmation
                        if let Some(win) = window_weak.upgrade() {
                            crate::toast::show_toast_on_window(
                                &win,
                                &crate::i18n::i18n("Item restored"),
                                crate::toast::ToastType::Success,
                            );
                        }
                    }
                }
            }
        });
        window.add_action(&undo_delete_action);

        // Retry connect action — retries a failed connection from toast button
        let retry_action = gio::SimpleAction::new("retry-connect", Some(glib::VariantTy::STRING));
        let state_clone = state.clone();
        let notebook_clone = self.terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        let sidebar_clone = sidebar.clone();
        let monitoring_clone = self.monitoring.clone();
        let activity_clone_retry = self.activity_coordinator.clone();
        retry_action.connect_activate(move |_, param| {
            if let Some(param) = param
                && let Some(id_str) = param.get::<String>()
                && let Ok(conn_id) = Uuid::parse_str(&id_str)
            {
                tracing::info!(%conn_id, "Retrying connection from toast");
                Self::start_connection_with_credential_resolution(
                    state_clone.clone(),
                    notebook_clone.clone(),
                    split_view_clone.clone(),
                    sidebar_clone.clone(),
                    monitoring_clone.clone(),
                    conn_id,
                    Some(activity_clone_retry.clone()),
                );
            }
        });
        window.add_action(&retry_action);

        // Wake On LAN action — sends WoL packet for selected connection
        let wol_action = gio::SimpleAction::new("wake-on-lan", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let toast_clone = self.toast_overlay.clone();
        let notebook_clone = self.terminal_notebook.clone();
        let monitoring_clone = self.monitoring.clone();
        let split_view_clone = self.split_view.clone();
        wol_action.connect_activate(move |_, _| {
            let Some(item) = sidebar_clone.get_selected_item() else {
                return;
            };
            if item.is_group() {
                return;
            }
            let id_str = item.id();
            let Ok(conn_id) = Uuid::parse_str(&id_str) else {
                return;
            };
            let state_ref = state_clone.borrow();
            let Some(conn) = state_ref.get_connection(conn_id) else {
                return;
            };
            let Some(wol_config) = conn.get_wol_config() else {
                toast_clone.show_warning(
                    "No Wake On LAN configured. Edit the connection \
                     to set a MAC address.",
                );
                return;
            };
            let wol_config = wol_config.clone();
            let mac_display = wol_config.mac_address.to_string();
            let host_for_check = conn.host.clone();
            let port_for_check = conn.port;
            drop(state_ref);

            let mac_for_cb = mac_display.clone();
            let id_for_cb = id_str.clone();
            let toast_for_cb = toast_clone.clone();
            let state_for_wol = state_clone.clone();
            let sidebar_for_wol = sidebar_clone.clone();
            let notebook_for_wol = notebook_clone.clone();
            let monitoring_for_wol = monitoring_clone.clone();
            let split_view_for_wol = split_view_clone.clone();
            crate::utils::spawn_blocking_with_callback(
                move || rustconn_core::wol::send_wol_with_retry(&wol_config, 3, 500),
                move |result| match result {
                    Ok(()) => {
                        tracing::info!(
                            mac = %mac_for_cb,
                            "WoL packet sent for connection {}",
                            id_for_cb,
                        );
                        toast_for_cb.show_success(&crate::i18n::i18n_f(
                            "Wake On LAN sent to {} — waiting for host to come online...",
                            &[&mac_for_cb],
                        ));

                        // Start polling for host to come online, then auto-connect
                        let toast = toast_for_cb.clone();
                        let host = host_for_check.clone();
                        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
                        // Register cancel token so it can be cancelled if needed
                        notebook_for_wol.register_poll_cancel(conn_id, cancel.clone());

                        let config =
                            rustconn_core::host_check::HostCheckConfig::new(&host, port_for_check)
                                .with_timeout_secs(3)
                                .with_poll_interval_secs(5)
                                .with_max_poll_duration_secs(300);

                        let state_wol = state_for_wol.clone();
                        let notebook_wol = notebook_for_wol.clone();
                        let sidebar_wol = sidebar_for_wol.clone();
                        let monitoring_wol = monitoring_for_wol.clone();
                        let split_view_wol = split_view_for_wol.clone();
                        let host_display = host.clone();
                        let notebook_cleanup = notebook_for_wol.clone();

                        crate::utils::spawn_blocking_with_callback(
                            move || {
                                let rt = tokio::runtime::Runtime::new()
                                    .expect("Failed to create tokio runtime");
                                rt.block_on(rustconn_core::host_check::poll_until_online(
                                    &config,
                                    &cancel,
                                    |_online, _elapsed| {},
                                ))
                            },
                            move |result| {
                                notebook_cleanup.cancel_poll(conn_id);
                                match result {
                                    Ok(true) => {
                                        toast.show_success(&crate::i18n::i18n_f(
                                            "{} is online — connecting...",
                                            &[&host_display],
                                        ));
                                        Self::start_connection_with_credential_resolution(
                                            state_wol,
                                            notebook_wol,
                                            split_view_wol,
                                            sidebar_wol,
                                            monitoring_wol,
                                            conn_id,
                                            None,
                                        );
                                    }
                                    Ok(false) => {
                                        toast.show_warning(&crate::i18n::i18n_f(
                                            "{} did not come online after WoL",
                                            &[&host_display],
                                        ));
                                    }
                                    Err(_) => {}
                                }
                            },
                        );
                    }
                    Err(e) => {
                        tracing::error!(?e, "Failed to send WoL for connection {}", id_for_cb,);
                        toast_for_cb.show_error(&crate::i18n::i18n(
                            "Failed to send Wake On LAN packet. Check network permissions.",
                        ));
                    }
                },
            );
        });
        window.add_action(&wol_action);

        // Check if host is online — TCP probe with polling and optional auto-connect
        let check_online_action = gio::SimpleAction::new("check-host-online", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let toast_clone = self.toast_overlay.clone();
        let notebook_clone_online = self.terminal_notebook.clone();
        let monitoring_clone_online = self.monitoring.clone();
        let split_view_clone_online = self.split_view.clone();
        check_online_action.connect_activate(move |_, _| {
            let Some(item) = sidebar_clone.get_selected_item() else {
                return;
            };
            if item.is_group() {
                return;
            }
            let id_str = item.id();
            let Ok(conn_id) = Uuid::parse_str(&id_str) else {
                return;
            };
            let (host, port) = {
                let Ok(state_ref) = state_clone.try_borrow() else {
                    return;
                };
                let Some(conn) = state_ref.get_connection(conn_id) else {
                    return;
                };
                (conn.host.clone(), conn.port)
            };

            let toast = toast_clone.clone();
            let state_for_connect = state_clone.clone();
            let sidebar_for_connect = sidebar_clone.clone();
            let notebook_for_connect = notebook_clone_online.clone();
            let monitoring_for_connect = monitoring_clone_online.clone();
            let split_view_for_connect = split_view_clone_online.clone();
            let host_display = host.clone();

            toast.show_toast(&crate::i18n::i18n_f(
                "Checking if {} is online...",
                &[&host_display],
            ));

            // Run async TCP probe in background
            let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            // Register cancel token so it can be cancelled if needed
            notebook_for_connect.register_poll_cancel(conn_id, cancel.clone());

            let config = rustconn_core::host_check::HostCheckConfig::new(&host, port)
                .with_timeout_secs(3)
                .with_poll_interval_secs(5)
                .with_max_poll_duration_secs(120);
            let host_display2 = host.clone();
            let notebook_cleanup = notebook_for_connect.clone();

            crate::utils::spawn_blocking_with_callback(
                move || {
                    let rt =
                        tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
                    rt.block_on(rustconn_core::host_check::poll_until_online(
                        &config,
                        &cancel,
                        |_online, _elapsed| {},
                    ))
                },
                move |result| {
                    notebook_cleanup.cancel_poll(conn_id);
                    match result {
                        Ok(true) => {
                            toast.show_success(&crate::i18n::i18n_f(
                                "{} is online",
                                &[&host_display2],
                            ));
                            Self::start_connection_with_credential_resolution(
                                state_for_connect,
                                notebook_for_connect,
                                split_view_for_connect,
                                sidebar_for_connect,
                                monitoring_for_connect,
                                conn_id,
                                None,
                            );
                        }
                        Ok(false) => {
                            toast.show_warning(&crate::i18n::i18n_f(
                                "{} did not come online within timeout",
                                &[&host_display2],
                            ));
                        }
                        Err(e) => {
                            tracing::warn!(%e, "Host check failed");
                            toast.show_error(&crate::i18n::i18n_f(
                                "Host check failed: {}",
                                &[&e.to_string()],
                            ));
                        }
                    }
                },
            );
        });
        window.add_action(&check_online_action);

        // Open SFTP action — opens file manager or mc in local shell
        let sftp_action = gio::SimpleAction::new("open-sftp", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let toast_clone = self.toast_overlay.clone();
        let notebook_clone = self.terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        sftp_action.connect_activate(move |_, _| {
            let Some(item) = sidebar_clone.get_selected_item() else {
                return;
            };
            if item.is_group() {
                return;
            }
            let id_str = item.id();
            let Ok(conn_id) = Uuid::parse_str(&id_str) else {
                return;
            };
            let state_ref = state_clone.borrow();
            let Some(conn) = state_ref.get_connection(conn_id) else {
                return;
            };
            let use_mc = state_ref.settings().terminal.sftp_use_mc;

            // Ensure SSH key is in agent before SFTP (mc and
            // file managers cannot pass identity files directly).
            let key_path = rustconn_core::sftp::get_ssh_key_path(conn)
                .and_then(|p| rustconn_core::resolve_key_path(&p));

            // Check if password auth — mc FISH doesn't support it
            let uses_password = matches!(
                &conn.protocol_config,
                rustconn_core::models::ProtocolConfig::Ssh(cfg)
                | rustconn_core::models::ProtocolConfig::Sftp(cfg)
                    if matches!(
                        cfg.auth_method,
                        rustconn_core::models::SshAuthMethod::Password
                            | rustconn_core::models::SshAuthMethod::KeyboardInteractive
                    )
            );

            if use_mc {
                // Open mc in a local shell tab with SFTP panel
                let mc_cmd = rustconn_core::sftp::build_mc_sftp_command(conn);
                let conn_name = conn.name.clone();
                let terminal_settings = state_ref.settings().terminal.clone();
                drop(state_ref);

                let Some(mc_args) = mc_cmd else {
                    toast_clone.show_warning(&crate::i18n::i18n(
                        "SFTP is only available for SSH connections.",
                    ));
                    return;
                };

                tracing::info!(?mc_args, "Opening SFTP via mc");

                // Warn about password auth — mc FISH can't prompt
                if uses_password && key_path.is_none() {
                    toast_clone.show_warning(
                        "mc requires SSH key in agent. \
                         Password auth is not supported.",
                    );
                }

                // Add SSH key to agent if configured
                if let Some(ref kp) = key_path {
                    if !rustconn_core::sftp::is_ssh_agent_available() {
                        toast_clone.show_warning(
                            "SSH agent not running. Run \
                             'eval $(ssh-agent)' and retry.",
                        );
                    }
                    tracing::info!(?kp, "Adding SSH key to agent for mc");
                    let mut ssh_add = std::process::Command::new("ssh-add");
                    ssh_add
                        .arg(kp)
                        .stdin(std::process::Stdio::null())
                        .stdout(std::process::Stdio::null())
                        .stderr(std::process::Stdio::piped());
                    rustconn_core::sftp::apply_agent_env(&mut ssh_add);
                    match ssh_add.output() {
                        Ok(output) if output.status.success() => {
                            tracing::info!("SSH key added to agent for mc");
                        }
                        Ok(output) => {
                            let stderr = String::from_utf8_lossy(&output.stderr);
                            tracing::warn!(
                                %stderr,
                                "ssh-add failed — mc FISH may not authenticate"
                            );
                            toast_clone.show_error(&format!(
                                "{}: {}",
                                crate::i18n::i18n("SSH key not available"),
                                stderr.trim()
                            ));
                            return;
                        }
                        Err(e) => {
                            tracing::error!(?e, "Failed to run ssh-add");
                            toast_clone
                                .show_error(&crate::i18n::i18n("Failed to add SSH key to agent."));
                            return;
                        }
                    }
                }

                // Check mc availability
                if std::process::Command::new("which")
                    .arg("mc")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map_or(true, |s| !s.success())
                {
                    toast_clone.show_error(&crate::i18n::i18n(
                        "Midnight Commander (mc) is not installed.",
                    ));
                    return;
                }

                toast_clone.show_toast(&crate::i18n::i18n("Opening mc SFTP..."));

                let tab_name = format!("mc: {conn_name}");
                let session_id = notebook_clone.create_terminal_tab_with_settings(
                    conn_id,
                    &tab_name,
                    "sftp",
                    None,
                    &terminal_settings,
                    None,
                );

                let downloads = rustconn_core::sftp::get_downloads_dir();

                // Delay mc spawn slightly so GTK allocates the VTE widget's
                // final size before mc reads terminal dimensions at startup.
                let nb = notebook_clone.clone();
                let mc_clone = mc_args.clone();
                let dl = downloads.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                    let argv: Vec<&str> = mc_clone.iter().map(String::as_str).collect();
                    nb.spawn_command(session_id, &argv, None, Some(&dl), None);
                });

                if let Some(info) = notebook_clone.get_session_info(session_id) {
                    split_view_clone.add_session(info, None);
                }
                split_view_clone.widget().set_visible(false);
                split_view_clone.widget().set_vexpand(false);
                notebook_clone.widget().set_vexpand(true);
                notebook_clone.show_tab_view_content();
            } else {
                // Open file manager with sftp:// URI
                let Some(uri) = rustconn_core::sftp::build_sftp_uri_from_connection(conn) else {
                    toast_clone.show_warning(&crate::i18n::i18n(
                        "SFTP is only available for SSH connections.",
                    ));
                    drop(state_ref);
                    return;
                };
                drop(state_ref);

                tracing::info!(%uri, "Opening SFTP file browser");
                toast_clone.show_toast(&crate::i18n::i18n("Opening SFTP..."));

                // Add SSH key to agent in background, then open URI
                let toast_cb = toast_clone.clone();
                let key_for_add = key_path.clone();

                // ssh-add in background thread, then launch file
                // manager as a direct subprocess (not via
                // UriLauncher/D-Bus) so it inherits SSH_AUTH_SOCK.
                let uri_clone = uri.clone();
                crate::utils::spawn_blocking_with_callback(
                    move || {
                        // Add SSH key to agent if configured
                        if let Some(ref kp) = key_for_add {
                            if !rustconn_core::sftp::is_ssh_agent_available() {
                                tracing::warn!(
                                    "SSH agent not available; \
                                     file manager may fail to authenticate"
                                );
                            }
                            tracing::info!(?kp, "Adding SSH key to agent for SFTP");
                            let mut ssh_add = std::process::Command::new("ssh-add");
                            ssh_add
                                .arg(kp)
                                .stdin(std::process::Stdio::null())
                                .stdout(std::process::Stdio::null())
                                .stderr(std::process::Stdio::piped());
                            rustconn_core::sftp::apply_agent_env(&mut ssh_add);
                            match ssh_add.output() {
                                Ok(output) if output.status.success() => {
                                    tracing::info!("SSH key added to agent");
                                }
                                Ok(output) => {
                                    let stderr = String::from_utf8_lossy(&output.stderr);
                                    tracing::warn!(
                                        %stderr,
                                        "ssh-add failed — file manager \
                                         may not authenticate"
                                    );
                                }
                                Err(e) => {
                                    tracing::error!(?e, "Failed to run ssh-add");
                                }
                            }
                        }
                        rustconn_core::sftp::is_ssh_agent_available()
                    },
                    move |agent_ok| {
                        // Launch file manager as a direct subprocess
                        // so it inherits SSH_AUTH_SOCK. UriLauncher
                        // goes through D-Bus/portal which may not
                        // pass our env to an already-running Dolphin.
                        Self::sftp_launch_file_manager(&uri_clone);
                        if !agent_ok {
                            toast_cb.show_warning(
                                "SSH agent not running — file manager \
                                 may not authenticate.",
                            );
                        }
                    },
                );
            }
        });
        window.add_action(&sftp_action);
    }

    /// Launches a file manager for an `sftp://` URI as a direct
    /// subprocess so it inherits the ssh-agent socket.
    ///
    /// On KDE, `xdg-open` may route through D-Bus to an
    /// already-running Dolphin that doesn't have our env.
    /// Launching `dolphin` directly as a child process ensures
    /// the KIO sftp worker sees the ssh-agent socket.
    ///
    /// Tries: `dolphin` → `nautilus` → `xdg-open` (last resort).
    pub(crate) fn sftp_launch_file_manager(uri: &str) {
        // Try dolphin first (KDE)
        let mut cmd = std::process::Command::new("dolphin");
        cmd.arg("--new-window").arg(uri);
        rustconn_core::sftp::apply_agent_env(&mut cmd);
        if cmd.spawn().is_ok() {
            tracing::info!(%uri, "Launched dolphin for SFTP");
            return;
        }

        // Try nautilus (GNOME)
        let mut cmd = std::process::Command::new("nautilus");
        cmd.args(["--new-window", uri]);
        rustconn_core::sftp::apply_agent_env(&mut cmd);
        if cmd.spawn().is_ok() {
            tracing::info!(%uri, "Launched nautilus for SFTP");
            return;
        }

        // Last resort — xdg-open (may go through D-Bus)
        let mut cmd = std::process::Command::new("xdg-open");
        cmd.arg(uri);
        rustconn_core::sftp::apply_agent_env(&mut cmd);
        if cmd.spawn().is_ok() {
            tracing::info!(%uri, "Launched xdg-open for SFTP");
            return;
        }

        tracing::warn!(
            %uri,
            "No file manager found for SFTP URI"
        );
    }

    /// Handles SFTP connection — opens file manager or mc
    ///
    /// Called when user clicks "Connect" on an SFTP-type connection.
    /// Reuses the same logic as the `open-sftp` sidebar action.
    /// Performs SSH port check before opening the file manager or mc.
    pub(crate) fn handle_sftp_connect(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: Option<&SharedSidebar>,
        split_view: Option<&SharedSplitView>,
        connection_id: Uuid,
    ) {
        // Pre-connect SSH port check before opening SFTP
        let (should_check, host, port, timeout) = {
            let state_ref = state.borrow();
            let Some(conn) = state_ref.get_connection(connection_id) else {
                return;
            };
            let settings = state_ref.settings();
            let should = settings.connection.pre_connect_port_check && !conn.skip_port_check;
            (
                should,
                conn.host.clone(),
                conn.port,
                settings.connection.port_check_timeout_secs,
            )
        };

        if should_check {
            let state_clone = state.clone();
            let notebook_clone = notebook.clone();
            let sidebar_clone = sidebar.cloned();
            let split_view_clone = split_view.cloned();

            crate::utils::spawn_blocking_with_callback(
                move || rustconn_core::check_port(&host, port, timeout),
                move |result| match result {
                    Ok(_) => {
                        Self::handle_sftp_connect_internal(
                            &state_clone,
                            &notebook_clone,
                            sidebar_clone.as_ref(),
                            split_view_clone.as_ref(),
                            connection_id,
                        );
                    }
                    Err(e) => {
                        tracing::warn!("Port check failed for SFTP connection: {e}");
                        if let Some(sb) = &sidebar_clone {
                            sb.update_connection_status(&connection_id.to_string(), "failed");
                        }
                        if let Some(root) = notebook_clone.widget().root()
                            && let Some(window) = root.downcast_ref::<gtk4::Window>()
                        {
                            crate::toast::show_retry_toast_on_window(
                                window,
                                &crate::i18n::i18n("Connection failed. Host unreachable."),
                                &connection_id.to_string(),
                            );
                        }
                    }
                },
            );
        } else {
            Self::handle_sftp_connect_internal(state, notebook, sidebar, split_view, connection_id);
        }
    }

    /// Internal SFTP connect logic after port check passes
    fn handle_sftp_connect_internal(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: Option<&SharedSidebar>,
        split_view: Option<&SharedSplitView>,
        connection_id: Uuid,
    ) {
        let state_ref = state.borrow();
        let Some(conn) = state_ref.get_connection(connection_id) else {
            return;
        };
        let use_mc = state_ref.settings().terminal.sftp_use_mc;
        let key_path = rustconn_core::sftp::get_ssh_key_path(conn)
            .and_then(|p| rustconn_core::resolve_key_path(&p));

        if use_mc {
            let mc_cmd = rustconn_core::sftp::build_mc_sftp_command(conn);
            let conn_name = conn.name.clone();
            let terminal_settings = state_ref.settings().terminal.clone();
            drop(state_ref);

            let Some(mc_args) = mc_cmd else {
                return;
            };

            tracing::info!(?mc_args, "SFTP connect: opening mc");

            if let Some(ref kp) = key_path {
                let mut ssh_add = std::process::Command::new("ssh-add");
                ssh_add
                    .arg(kp)
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::piped());
                rustconn_core::sftp::apply_agent_env(&mut ssh_add);
                match ssh_add.output() {
                    Ok(output) if output.status.success() => {
                        tracing::info!(?kp, "SSH key added to agent");
                    }
                    Ok(output) => {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        tracing::warn!(?kp, %stderr, "ssh-add failed");
                        if let Some(sb) = sidebar {
                            sb.update_connection_status(&connection_id.to_string(), "failed");
                        }
                        return;
                    }
                    Err(e) => {
                        tracing::error!(?e, "Failed to run ssh-add");
                        if let Some(sb) = sidebar {
                            sb.update_connection_status(&connection_id.to_string(), "failed");
                        }
                        return;
                    }
                }
            }

            // Update sidebar status for SFTP connection
            if let Some(sb) = sidebar {
                sb.update_connection_status(&connection_id.to_string(), "connecting");
            }

            let tab_name = format!("mc: {conn_name}");
            let session_id = notebook.create_terminal_tab_with_settings(
                connection_id,
                &tab_name,
                "sftp",
                None,
                &terminal_settings,
                None,
            );

            let downloads = rustconn_core::sftp::get_downloads_dir();

            // Delay mc spawn slightly so GTK allocates the VTE widget's
            // final size before mc reads terminal dimensions at startup.
            let notebook_clone = notebook.clone();
            let mc_args_clone = mc_args.clone();
            let downloads_clone = downloads.clone();
            glib::timeout_add_local_once(std::time::Duration::from_millis(150), move || {
                let argv: Vec<&str> = mc_args_clone.iter().map(String::as_str).collect();
                notebook_clone.spawn_command(session_id, &argv, None, Some(&downloads_clone), None);
            });

            // Mark as connected and increment session count
            if let Some(sb) = sidebar {
                sb.update_connection_status(&connection_id.to_string(), "connected");
                sb.increment_session_count(&connection_id.to_string());
            }

            if let Some(sv) = split_view {
                if let Some(info) = notebook.get_session_info(session_id) {
                    sv.add_session(info, None);
                }
                sv.widget().set_visible(false);
                sv.widget().set_vexpand(false);
            }
            notebook.widget().set_vexpand(true);
            notebook.show_tab_view_content();
        } else {
            let Some(uri) = rustconn_core::sftp::build_sftp_uri_from_connection(conn) else {
                drop(state_ref);
                return;
            };
            drop(state_ref);

            tracing::info!(%uri, "SFTP connect: opening file browser");

            let key_for_add = key_path.clone();
            let uri_clone = uri.clone();
            crate::utils::spawn_blocking_with_callback(
                move || {
                    if let Some(ref kp) = key_for_add {
                        let mut ssh_add = std::process::Command::new("ssh-add");
                        ssh_add
                            .arg(kp)
                            .stdin(std::process::Stdio::null())
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::piped());
                        rustconn_core::sftp::apply_agent_env(&mut ssh_add);
                        match ssh_add.output() {
                            Ok(output) if output.status.success() => {
                                tracing::info!(?kp, "SSH key added to agent");
                            }
                            Ok(output) => {
                                let stderr = String::from_utf8_lossy(&output.stderr);
                                tracing::warn!(
                                    ?kp, %stderr, "ssh-add failed"
                                );
                            }
                            Err(e) => {
                                tracing::error!(?e, "Failed to run ssh-add");
                            }
                        }
                    }
                    true
                },
                move |_| {
                    // Launch file manager as a direct subprocess
                    // so it inherits SSH_AUTH_SOCK. UriLauncher
                    // goes through D-Bus which may not pass env.
                    Self::sftp_launch_file_manager(&uri_clone);
                },
            );
        }
    }
}
