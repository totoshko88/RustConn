//! Edit-related window actions (connect, edit, delete, rename, duplicate, SFTP)
//!
//! Extracted from `window/mod.rs` to reduce module complexity.

use super::*;

/// The id of the connection selected in the sidebar, or `None` for a group or
/// no selection.
fn selected_connection_id(sidebar: &SharedSidebar) -> Option<Uuid> {
    let item = sidebar.get_selected_item()?;
    if item.is_group() {
        return None;
    }
    Uuid::parse_str(&item.id()).ok()
}

/// Registers `win.<name>`, acting on the connection selected in the sidebar,
/// and `win.<name>-by-id`, acting on the connection whose id is the string
/// target.
///
/// A menu that is not about the sidebar selection — a smart folder — needs the
/// second form: selecting the row first and then running the plain action does
/// not work, because the selection is applied on idle, after the action has
/// already run against whatever was selected before.
fn register_selected_and_by_id(
    window: &adw::ApplicationWindow,
    sidebar: &SharedSidebar,
    name: &str,
    handler: impl Fn(Uuid) + 'static,
) {
    let handler = Rc::new(handler);

    let selected_action = gio::SimpleAction::new(name, None);
    let sidebar_clone = sidebar.clone();
    let handler_clone = Rc::clone(&handler);
    selected_action.connect_activate(move |_, _| {
        if let Some(conn_id) = selected_connection_id(&sidebar_clone) {
            handler_clone(conn_id);
        }
    });
    window.add_action(&selected_action);

    let by_id_action =
        gio::SimpleAction::new(&format!("{name}-by-id"), Some(glib::VariantTy::STRING));
    by_id_action.connect_activate(move |_, param| {
        if let Some(conn_id) = param
            .and_then(glib::Variant::get::<String>)
            .and_then(|s| Uuid::parse_str(&s).ok())
        {
            handler(conn_id);
        }
    });
    window.add_action(&by_id_action);
}

/// Why the external tunnel browser could not be resolved.
enum TunnelBrowserError {
    /// A command is configured but does not name a Chromium-family browser.
    NotChromium(String),
    /// A configured command names a browser that is not present on `PATH`.
    NotFound(String),
    /// No command is configured and auto-detection found nothing.
    NoneFound,
}

/// Which way `open_browser_via_tunnel` will open the tunnelled browser, decided
/// before the tunnel is raised.
enum TunnelBrowserPath {
    /// Use the embedded browser (either preferred, or as a fallback).
    Embedded,
    /// Launch this external Chromium binary with these extra args.
    External(String, Vec<String>),
    /// Nothing can tunnel; the reason toast is already shown, so stop.
    Fail,
}

thread_local! {
    /// Connections whose "Open Browser via Tunnel" tunnel is still starting.
    ///
    /// Only touched on the GTK thread. Every borrow lives inside one `with` call
    /// in [`BrowserTunnelStart`], never across a callback.
    static BROWSER_TUNNELS_STARTING: std::cell::RefCell<std::collections::BTreeSet<Uuid>> =
        const { std::cell::RefCell::new(std::collections::BTreeSet::new()) };
}

/// Marks a connection's browser tunnel as starting; unmarks on drop.
///
/// The guard is moved into the callback that runs when the tunnel is ready (or
/// has failed), so the mark is cleared on success, on failure, and when the
/// worker dies and the callback is dropped unrun.
struct BrowserTunnelStart(Uuid);

impl BrowserTunnelStart {
    /// Marks `id` as starting, or returns `None` when a tunnel for it is
    /// already starting.
    fn begin(id: Uuid) -> Option<Self> {
        let inserted = BROWSER_TUNNELS_STARTING
            .with(|set| set.try_borrow_mut().is_ok_and(|mut set| set.insert(id)));
        inserted.then_some(Self(id))
    }
}

impl Drop for BrowserTunnelStart {
    fn drop(&mut self) {
        // `try_borrow_mut`: a failure here means a nested borrow, which cannot
        // happen (borrows never span a callback); skipping beats panicking in drop.
        BROWSER_TUNNELS_STARTING.with(|set| {
            if let Ok(mut set) = set.try_borrow_mut() {
                set.remove(&self.0);
            }
        });
    }
}

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
        let key_setup_ctx_for_edit = self.key_setup_context(window, state, sidebar);
        edit_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::edit_selected_connection(
                    win.upcast_ref(),
                    &state_clone,
                    &sidebar_clone,
                    &key_setup_ctx_for_edit,
                );
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

        // The same, addressed by connection id (smart folders).
        let delete_by_id_action =
            gio::SimpleAction::new("delete-connection-by-id", Some(glib::VariantTy::STRING));
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        delete_by_id_action.connect_activate(move |_, param| {
            if let Some(conn_id) = param
                .and_then(glib::Variant::get::<String>)
                .and_then(|s| Uuid::parse_str(&s).ok())
                && let Some(win) = window_weak.upgrade()
            {
                operations::delete_connection_by_id(
                    win.upcast_ref(),
                    &state_clone,
                    &sidebar_clone,
                    conn_id,
                );
            }
        });
        window.add_action(&delete_by_id_action);

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

        // Duplicate a group with its subgroups and connections
        let duplicate_group_action = gio::SimpleAction::new("duplicate-group", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        duplicate_group_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                super::operations::duplicate_selected_group(
                    win.upcast_ref(),
                    &state_clone,
                    &sidebar_clone,
                );
            }
        });
        window.add_action(&duplicate_group_action);

        // Run a local bash script on the selected SSH hosts
        let run_script_action = gio::SimpleAction::new("run-local-script", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        run_script_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                super::remote_script::open_dialog(&win, &state_clone, &sidebar_clone);
            }
        });
        window.add_action(&run_script_action);

        // Duplicate as a numbered range of connections
        let duplicate_range_action = gio::SimpleAction::new("duplicate-as-range", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        duplicate_range_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                super::operations::duplicate_selected_as_range(
                    win.upcast_ref(),
                    &state_clone,
                    &sidebar_clone,
                );
            }
        });
        window.add_action(&duplicate_range_action);

        // Duplicate via Wizard action — opens wizard pre-filled with existing connection data
        let duplicate_wizard_action = gio::SimpleAction::new("duplicate-via-wizard", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let toast_clone = self.toast_overlay.clone();
        duplicate_wizard_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::duplicate_via_wizard(&win, &state_clone, &sidebar_clone, &toast_clone);
            }
        });
        window.add_action(&duplicate_wizard_action);

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

        self.setup_copy_field_actions(window, state, sidebar);

        // Rename item action (works for both connections and groups)
        let rename_action = gio::SimpleAction::new("rename-item", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let notebook_for_rename = Rc::clone(&self.terminal_notebook);
        rename_action.connect_activate(move |_, _| {
            if let Some(win) = window_weak.upgrade() {
                Self::rename_selected_item(
                    win.upcast_ref(),
                    &state_clone,
                    &sidebar_clone,
                    &notebook_for_rename,
                );
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

        // Empty-trash action — permanently removes every entry in the trash
        // (and its vault credential) after a confirmation. A deleted item stays
        // in the trash until this runs, so the Trash row can restore it, and the
        // vault credential survives the deletion for the same reason (issue #263).
        let empty_trash_action = gio::SimpleAction::new("empty-trash", None);
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let window_weak = window.downgrade();
        empty_trash_action.connect_activate(move |_, _| {
            let Some(win) = window_weak.upgrade() else {
                return;
            };
            let state = state_clone.clone();
            let sidebar = sidebar_clone.clone();
            let win_for_result = win.clone();
            crate::alert::show_confirm(
                &win,
                &crate::i18n::i18n("Empty trash?"),
                &crate::i18n::i18n(
                    "Deleted items will be removed permanently. This cannot be undone.",
                ),
                &crate::i18n::i18n("Empty Trash"),
                true,
                move |confirmed| {
                    if !confirmed {
                        return;
                    }
                    let Ok(mut state_mut) = state.try_borrow_mut() else {
                        tracing::warn!("Empty trash skipped: state busy");
                        return;
                    };
                    // The vault credentials of the deleted items are removed on a
                    // worker thread (about 0.7 s each with a KeePass file). The
                    // header-bar spinner shows until it ends, while the Trash row
                    // already reflects the emptied trash.
                    let busy_guard = super::acquire_busy_guard();
                    let result = state_mut.empty_trash(move || drop(busy_guard));
                    drop(state_mut);
                    // Whatever the outcome, the row must show what is left.
                    sorting::refresh_trash_row(&state, &sidebar);
                    match result {
                        Ok(()) => crate::toast::show_toast_on_window(
                            &win_for_result,
                            &crate::i18n::i18n("Trash emptied"),
                            crate::toast::ToastType::Success,
                        ),
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to empty trash");
                            crate::toast::show_toast_on_window(
                                &win_for_result,
                                &crate::i18n::i18n_f(
                                    "Failed to empty trash: {}",
                                    &[&e.to_string()],
                                ),
                                crate::toast::ToastType::Error,
                            );
                        }
                    }
                },
            );
        });
        window.add_action(&empty_trash_action);

        // Purge-trash-item action — the single-entry Empty Trash, behind
        // "Delete Permanently…" on a trashed row. Target: "type:uuid", the
        // format `win.undo-delete` takes. Asks first, then removes that entry
        // and its vault credential, sparing a credential another connection
        // still resolves to (`AppState::purge_trash_item`).
        let purge_trash_item_action =
            gio::SimpleAction::new("purge-trash-item", Some(glib::VariantTy::STRING));
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let window_weak = window.downgrade();
        purge_trash_item_action.connect_activate(move |_, param| {
            let Some(win) = window_weak.upgrade() else {
                return;
            };
            let Some((kind, id)) = param
                .and_then(glib::Variant::get::<String>)
                .and_then(|target| crate::sidebar::trash::parse_item_target(&target))
            else {
                return;
            };
            let Some(name) = state_clone
                .borrow()
                .trash_entries()
                .into_iter()
                .find(|entry| entry.kind == kind && entry.id == id)
                .map(|entry| entry.name)
            else {
                return;
            };
            let body = match kind {
                crate::sidebar::trash::TrashKind::Connection => crate::i18n::i18n_f(
                    "“{}” will be removed together with its saved password, if any. This cannot be undone.",
                    &[&name],
                ),
                crate::sidebar::trash::TrashKind::Group => crate::i18n::i18n_f(
                    "The group “{}” will be removed together with its saved password, if any. Its connections stay in the trash. This cannot be undone.",
                    &[&name],
                ),
            };
            let state = state_clone.clone();
            let sidebar = sidebar_clone.clone();
            let win_for_result = win.clone();
            crate::alert::show_confirm(
                &win,
                &crate::i18n::i18n("Delete permanently?"),
                &body,
                &crate::i18n::i18n("Delete"),
                true,
                move |confirmed| {
                    if !confirmed {
                        return;
                    }
                    let Ok(mut state_mut) = state.try_borrow_mut() else {
                        tracing::warn!("Permanent delete skipped: state busy");
                        return;
                    };
                    // The vault credential is removed on a worker thread; the
                    // header-bar spinner shows until it ends.
                    let busy_guard = super::acquire_busy_guard();
                    let result = state_mut.purge_trash_item(kind, id, move || drop(busy_guard));
                    drop(state_mut);
                    // Whatever the outcome, the row must show what is left.
                    sorting::refresh_trash_row(&state, &sidebar);
                    match result {
                        Ok(true) => crate::toast::show_toast_on_window(
                            &win_for_result,
                            &crate::i18n::i18n("Deleted permanently"),
                            crate::toast::ToastType::Success,
                        ),
                        // Restored or purged while the dialog was open.
                        Ok(false) => {}
                        Err(e) => {
                            tracing::warn!(error = %e, "Failed to delete a trash item permanently");
                            crate::toast::show_toast_on_window(
                                &win_for_result,
                                &crate::i18n::i18n_f(
                                    "Failed to delete permanently: {}",
                                    &[&e.to_string()],
                                ),
                                crate::toast::ToastType::Error,
                            );
                        }
                    }
                },
            );
        });
        window.add_action(&purge_trash_item_action);

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

        // Connect-to action — connects to a specific connection by ID (used by smart folders)
        let connect_to_action = gio::SimpleAction::new("connect-to", Some(glib::VariantTy::STRING));
        let state_clone = state.clone();
        let notebook_clone = self.terminal_notebook.clone();
        let split_view_clone = self.split_view.clone();
        let sidebar_clone = sidebar.clone();
        let monitoring_clone = self.monitoring.clone();
        let activity_clone_ct = self.activity_coordinator.clone();
        connect_to_action.connect_activate(move |_, param| {
            if let Some(param) = param
                && let Some(id_str) = param.get::<String>()
                && let Ok(conn_id) = Uuid::parse_str(&id_str)
            {
                tracing::info!(%conn_id, "Connecting to connection by ID");
                sidebar_clone.update_connection_status(&conn_id.to_string(), "connecting");
                Self::start_connection_with_credential_resolution(
                    state_clone.clone(),
                    notebook_clone.clone(),
                    split_view_clone.clone(),
                    sidebar_clone.clone(),
                    monitoring_clone.clone(),
                    conn_id,
                    Some(activity_clone_ct.clone()),
                );
            }
        });
        window.add_action(&connect_to_action);

        // Wake On LAN action — sends WoL packet for selected connection.
        //
        // Deliberately reachable from the sidebar context menu only, and not from
        // the app menu. It acts on `sidebar.get_selected_item()`, so the target is
        // whatever the user right-clicked — an explicit choice made in the same
        // gesture. An app-menu entry would fire against whatever happened to be
        // selected, which the user is not looking at when the menu is open.
        //
        // Tools ▸ "Wake On LAN..." is not a second copy of this and should not be
        // merged into it: that dialog wakes a host the user types a MAC for, which
        // needs no selection and may not correspond to any saved connection. This
        // action wakes the selected connection and then polls it to connect. Two
        // different jobs that happen to send the same packet.
        //
        // `wake-on-lan-by-id` is the same job addressed by connection id, for
        // menus that are not about the sidebar selection (a smart folder).
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let toast_clone = self.toast_overlay.clone();
        let notebook_clone = self.terminal_notebook.clone();
        let monitoring_clone = self.monitoring.clone();
        let split_view_clone = self.split_view.clone();
        let wake_connection = move |conn_id: Uuid| {
            let id_str = conn_id.to_string();
            let Ok(state_ref) = state_clone.try_borrow() else {
                return;
            };
            let Some(conn) = state_ref.get_connection(conn_id) else {
                return;
            };
            let Some(wol_config) = conn.get_wol_config() else {
                toast_clone.show_warning(&crate::i18n::i18n(
                    "No Wake On LAN configured. Edit the connection to set a MAC address.",
                ));
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
                            "Wake On LAN sent to {} — waiting for host to come online…",
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
                                let rt = tokio::runtime::Runtime::new().map_err(|e| {
                                    rustconn_core::host_check::HostCheckError::Io(e)
                                })?;
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
                                            "{} is online — connecting…",
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
        };
        register_selected_and_by_id(window, sidebar, "wake-on-lan", wake_connection);

        // Check if host is online — TCP probe with polling and optional auto-connect
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let toast_clone = self.toast_overlay.clone();
        let notebook_clone_online = self.terminal_notebook.clone();
        let monitoring_clone_online = self.monitoring.clone();
        let split_view_clone_online = self.split_view.clone();
        let check_connection_online = move |conn_id: Uuid| {
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
                "Checking if {} is online…",
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
                    let rt = tokio::runtime::Runtime::new()
                        .map_err(rustconn_core::host_check::HostCheckError::Io)?;
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
        };
        register_selected_and_by_id(
            window,
            sidebar,
            "check-host-online",
            check_connection_online,
        );

        // Set Up Key Authentication — generate, install and verify an SSH key,
        // then switch the connection to it. The window is held weakly: it owns
        // this action, which owns the context.
        let key_setup_context = self.key_setup_context(window, state, sidebar);
        register_selected_and_by_id(window, sidebar, "set-up-key-auth", move |id| {
            super::key_setup::open_key_setup_dialog(Rc::clone(&key_setup_context), id);
        });

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
            let targets = {
                let state_ref = state_clone.borrow();
                let Some(conn) = state_ref.get_connection(conn_id) else {
                    return;
                };
                super::key_unlock::sftp_unlock_targets(conn, &state_ref.list_groups_owned())
            };
            let state = state_clone.clone();
            let notebook = notebook_clone.clone();
            let split_view = split_view_clone.clone();
            let toast = toast_clone.clone();
            super::key_unlock::unlock_then(notebook_clone.widget(), targets, move || {
                Self::open_sftp_for_connection(&state, &notebook, &split_view, &toast, conn_id);
            });
        });
        window.add_action(&sftp_action);

        // Open Browser via Tunnel — raises a SOCKS proxy to the selected SSH
        // connection and opens a fresh browser through it (Ásbrú's incognito
        // browser). Embedded or external per the Settings toggle.
        let obt_action = gio::SimpleAction::new("open-browser-via-tunnel", None);
        let state_obt = state.clone();
        let sidebar_obt = sidebar.clone();
        let notebook_obt = self.terminal_notebook.clone();
        obt_action.connect_activate(move |_, _| {
            let Some(item) = sidebar_obt.get_selected_item() else {
                return;
            };
            if item.is_group() {
                return;
            }
            let Ok(conn_id) = Uuid::parse_str(&item.id()) else {
                return;
            };
            Self::open_browser_via_tunnel(&state_obt, &notebook_obt, &sidebar_obt, conn_id);
        });
        window.add_action(&obt_action);
    }

    /// Builds the context `window::key_setup` needs to open "Set Up Key
    /// Authentication…", shared by the sidebar menu's action and the
    /// connection editor's own row (K4) — one function, so the two entry
    /// points cannot end up wired to a different window, state or notebook.
    pub(super) fn key_setup_context(
        &self,
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) -> Rc<super::key_setup::KeySetupContext> {
        Rc::new(super::key_setup::KeySetupContext {
            window: window.downgrade(),
            state: state.clone(),
            sidebar: sidebar.clone(),
            notebook: self.terminal_notebook.clone(),
            split_view: self.split_view.clone(),
            monitoring: self.monitoring.clone(),
            toast_overlay: self.toast_overlay.clone(),
        })
    }

    /// Raises a dynamic SOCKS proxy (`ssh -N -D`) to the given SSH or Tailscale SSH connection
    /// **off the GTK thread** and waits for it to accept connections, delivering the result to
    /// `on_ready` back on the GTK thread.
    ///
    /// The tunnel reuses that connection's key (resolved through the group
    /// chain), its own jump-host chain and any session-cached password. It is
    /// the shared primitive behind both a Web connection's `tunnel_via` and the
    /// SSH sidebar's "Open Browser via Tunnel" action. The params are read from
    /// `AppState` here (cheap, on the GTK thread, borrow released before the
    /// worker starts); only the blocking spawn+wait moves to a worker.
    ///
    /// `on_ready` receives `Err(message)` when the connection is gone, the state
    /// is momentarily busy, or `ssh` fails to establish the proxy.
    ///
    /// Known limitation (shared with the Web `tunnel_via` path):
    /// - Quitting while the worker waits for the port leaves its `ssh -N` running,
    ///   because the tunnel is not registered anywhere until `on_ready` runs and
    ///   `spawn_tunnel` sets neither a process group nor a parent-death signal.
    ///
    /// The sidebar action guards against repeat clicks itself, see
    /// [`BrowserTunnelStart`]; this primitive does not.
    // ponytail: orphan-on-quit needs PDEATHSIG in rustconn-pty-sys (deferred to 0.24.x).
    fn raise_socks_tunnel_async<C>(state: &SharedAppState, jump_id: Uuid, on_ready: C)
    where
        C: FnOnce(Result<rustconn_core::ssh_tunnel::SshTunnel, String>) + 'static,
    {
        let params = match Self::socks_tunnel_params_for(state, jump_id) {
            Ok(params) => params,
            Err(msg) => {
                on_ready(Err(msg));
                return;
            }
        };
        crate::utils::spawn_blocking_with_callback(
            move || Self::spawn_socks_tunnel(&params),
            on_ready,
        );
    }

    /// The translated reason shown when a connection cannot host a SOCKS tunnel
    /// (mirrors `SshTunnelError::UnsupportedConnection`, whose text is English
    /// because `rustconn-core` has no locale).
    fn unsupported_tunnel_reason() -> String {
        crate::i18n::i18n("This connection type cannot host a SOCKS tunnel")
    }

    /// Builds the [`rustconn_core::ssh_tunnel::SshTunnelParams`] for a SOCKS tunnel to `jump_id`, reading
    /// everything the spawn needs from `AppState`.
    ///
    /// Split from [`Self::spawn_socks_tunnel`] so the spawn — which blocks while
    /// `ssh` connects and the port comes up — can run off the GTK thread while
    /// this cheap, borrow-only half stays on it. See
    /// [`Self::raise_socks_tunnel_async`].
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when the connection is gone or the state
    /// is momentarily busy.
    fn socks_tunnel_params_for(
        state: &SharedAppState,
        jump_id: Uuid,
    ) -> Result<rustconn_core::ssh_tunnel::SshTunnelParams, String> {
        let state_ref = state
            .try_borrow()
            .map_err(|_| crate::i18n::i18n("application is busy"))?;
        let jump_conn = state_ref
            .get_connection(jump_id)
            .ok_or_else(|| crate::i18n::i18n("the SSH connection it tunnels through is gone"))?;

        // The same gate the sidebar uses: a connection that cannot host a tunnel
        // is refused here rather than run through `ssh` as if it were plain SSH.
        let Some(route) = rustconn_core::ssh_tunnel::socks_tunnel_route(jump_conn) else {
            return Err(Self::unsupported_tunnel_reason());
        };

        if let rustconn_core::ssh_tunnel::SocksTunnelRoute::TailscaleSsh { destination } = route {
            // Same `tailscale ssh <dest>` the session runs: tailnet identity, no key,
            // no cached password, no port (issue #397).
            return Ok(rustconn_core::ssh_tunnel::SshTunnelParams {
                jump_host: destination,
                jump_port: 22,
                remote_host: String::new(),
                remote_port: 0,
                identity_file: None,
                password: None,
                extra_args: Vec::new(),
                launcher: rustconn_core::ssh_tunnel::TunnelLauncher::TailscaleSsh,
            });
        }

        let mut jump_dest = jump_conn.host.clone();
        if let Some(user) = &jump_conn.username {
            jump_dest = format!("{user}@{jump_dest}");
        }
        let groups = state_ref.list_groups_owned();
        let identity_file =
            rustconn_core::connection::ssh_inheritance::resolve_ssh_key_path(jump_conn, &groups)
                .and_then(|p| rustconn_core::resolve_key_path(&p))
                .map(|p| p.to_string_lossy().to_string());
        let password = state_ref
            .get_cached_credentials(jump_id)
            .filter(|c| {
                use secrecy::ExposeSecret;
                !c.password.expose_secret().is_empty()
            })
            .map(|c| c.password.clone());
        let extra_args = super::protocols::resolve_jump_chain_for_tunnel(
            &state_ref,
            jump_conn,
            password.is_some(),
            rustconn_core::ssh_tunnel::HopHostKeys::UserConfig,
        );

        Ok(rustconn_core::ssh_tunnel::SshTunnelParams {
            jump_host: jump_dest,
            jump_port: jump_conn.port,
            // Unused for a SOCKS (`-D`) tunnel, but the struct requires them.
            remote_host: String::new(),
            remote_port: 0,
            identity_file,
            password,
            extra_args,
            launcher: rustconn_core::ssh_tunnel::TunnelLauncher::OpenSsh,
        })
    }

    /// Spawns the `ssh -N -D` SOCKS proxy from prebuilt `params` and waits for
    /// its local port to accept connections.
    ///
    /// This blocks (the wait polls the port with `std::thread::sleep`), so it is
    /// **not** to be called on the GTK thread — a dead jump host would otherwise
    /// freeze the window until `ConnectTimeout` lapses. Callers on the UI thread
    /// go through [`Self::raise_socks_tunnel_async`], which runs this on a worker.
    ///
    /// # Errors
    ///
    /// Returns a human-readable message when `ssh` fails to establish the proxy.
    fn spawn_socks_tunnel(
        params: &rustconn_core::ssh_tunnel::SshTunnelParams,
    ) -> Result<rustconn_core::ssh_tunnel::SshTunnel, String> {
        let mut tunnel =
            rustconn_core::ssh_tunnel::create_socks_tunnel(params).map_err(|e| e.to_string())?;
        rustconn_core::ssh_tunnel::wait_for_tunnel_ready(
            &mut tunnel,
            40,
            std::time::Duration::from_millis(250),
        )
        .map_err(|e| e.to_string())?;
        tracing::info!(socks_port = tunnel.local_port(), "SOCKS tunnel ready");
        Ok(tunnel)
    }

    /// Raises a Web connection's SOCKS tunnel **off the GTK thread**, delivering
    /// the result to `on_ready` back on the GTK thread.
    ///
    /// `on_ready` receives `Ok(None)` when the connection browses directly (no
    /// `tunnel_via`), `Ok(Some(tunnel))` once the proxy is up, or `Err(message)`
    /// when it could not be built — the same shape `Self::raise_web_socks_tunnel`
    /// returned synchronously, but without blocking the window while `ssh`
    /// connects. The params are read from `AppState` here (cheap, on the GTK
    /// thread); only the blocking spawn+wait moves to a worker.
    fn raise_web_socks_tunnel_async<C>(
        state: &SharedAppState,
        web_config: &rustconn_core::models::WebConfig,
        on_ready: C,
    ) where
        C: FnOnce(Result<Option<rustconn_core::ssh_tunnel::SshTunnel>, String>) + 'static,
    {
        let Some(jump_id) = web_config.tunnel_via else {
            on_ready(Ok(None));
            return;
        };
        Self::raise_socks_tunnel_async(state, jump_id, move |result| on_ready(result.map(Some)));
    }

    /// Starts the relay for a proxy that needs a login, and points `web_config`
    /// at it. `Ok(None)` for every other connection.
    fn start_web_socks_relay(
        state: &SharedAppState,
        connection_id: Uuid,
        web_config: &mut rustconn_core::models::WebConfig,
    ) -> Result<Option<rustconn_core::socks_relay::SocksRelay>, String> {
        let (Some(username), Some(proxy)) = (
            web_config.socks_proxy_username.as_deref(),
            web_config.socks_proxy.as_deref(),
        ) else {
            return Ok(None);
        };
        let variable = web_config
            .socks_proxy_password_variable
            .as_deref()
            .unwrap_or_default();
        let globals = super::protocols::resolve_globals_with_ask(state, connection_id);
        // Only a secret variable: a plain one keeps the proxy password in the
        // configuration file as plain text.
        let password = match rustconn_core::protocol::socks_password_variable(&globals, variable) {
            Ok(found) if !found.value.is_empty() => {
                secrecy::SecretString::from(found.value.clone())
            }
            Err(rustconn_core::protocol::SocksPasswordVariableError::NotSecret(_)) => {
                return Err(crate::i18n::i18n_f(
                    "The SOCKS proxy password variable ‘{}’ is not secret, so it was not used. Mark it as secret in Global Variables.",
                    &[variable],
                ));
            }
            _ => {
                return Err(crate::i18n::i18n_f(
                    "The SOCKS proxy password variable ‘{}’ has no value",
                    &[variable],
                ));
            }
        };
        let relay = rustconn_core::socks_relay::SocksRelay::start(proxy, username, password)
            .map_err(|e| {
                crate::i18n::i18n_f("Could not start the SOCKS relay: {}", &[&e.to_string()])
            })?;
        web_config.socks_proxy = Some(relay.proxy_uri());
        Ok(Some(relay))
    }

    /// Registers a tunnelled external browser as an external session that keeps
    /// `held` (its profile) until the browser exits. Without a registry the
    /// browser is not tracked, and `held` still waits for it to exit.
    fn track_proxied_browser(
        session_id: Uuid,
        connection_id: Uuid,
        child: std::process::Child,
        held: impl std::any::Any,
    ) {
        match super::external_session_registry() {
            Some(registry) => {
                registry.register(session_id, connection_id, Some(child), None);
                registry.hold(session_id, held);
            }
            None => crate::external_session::drop_when_exits(child, held),
        }
    }

    /// Normalises the configured tunnel-browser start URL into something both
    /// the embedded WebKit view and an external browser accept.
    ///
    /// An empty value falls back to `https://www.google.com`. A value with no
    /// `http(s)://` (or `file://`) scheme is prefixed with `https://`, matching
    /// what the embedded browser's own address bar does, so a user who typed a
    /// bare `ifconfig.me` still gets a valid URL.
    fn normalize_start_url(configured: &str) -> String {
        let trimmed = configured.trim();
        if trimmed.is_empty() {
            return "https://www.google.com".to_string();
        }
        let lower = trimmed.to_lowercase();
        if lower.starts_with("http://")
            || lower.starts_with("https://")
            || lower.starts_with("file://")
        {
            trimmed.to_string()
        } else {
            format!("https://{trimmed}")
        }
    }

    /// True if `name` looks like a Chromium-family binary — the only kind that
    /// takes `--proxy-server` on the command line.
    fn is_chromium_browser(name: &str) -> bool {
        rustconn_core::protocol::browser_family(name)
            == Some(rustconn_core::protocol::BrowserFamily::Chromium)
    }

    /// Resolves the external tunnel browser: the configured command when set,
    /// otherwise auto-detection.
    ///
    /// Returns the binary and any extra arguments to pass before the proxy
    /// flags. A configured command must name a Chromium-family binary that is
    /// present (a bare name resolved on `PATH`, a path checked as a file);
    /// anything else is an error so the caller can explain it rather than browse
    /// un-tunnelled.
    fn resolve_tunnel_browser(command: &str) -> Result<(String, Vec<String>), TunnelBrowserError> {
        let command = command.trim();
        if command.is_empty() {
            return Self::find_chromium_browser()
                .or_else(rustconn_core::protocol::first_installed_proxy_browser)
                .map(|bin| (bin, Vec::new()))
                .ok_or(TunnelBrowserError::NoneFound);
        }

        let mut parts = command.split_whitespace();
        let bin = parts.next().unwrap_or(command).to_string();
        let extra: Vec<String> = parts.map(str::to_string).collect();

        if !rustconn_core::protocol::can_use_socks_proxy(&bin) {
            return Err(TunnelBrowserError::NotChromium(bin));
        }
        let available = if bin.contains('/') {
            std::path::Path::new(&bin).is_file()
        } else {
            rustconn_core::which::is_available(&bin)
        };
        if !available {
            return Err(TunnelBrowserError::NotFound(bin));
        }
        Ok((bin, extra))
    }

    /// The system default web browser's binary, if it is Chromium-family and
    /// present.
    ///
    /// Reads `xdg-settings get default-web-browser` (a `.desktop` file name),
    /// resolves its `Exec=` line to a binary, and returns it only when that
    /// binary looks Chromium-based. Returns `None` on any failure or when the
    /// default is Firefox or another non-Chromium browser — the caller then
    /// falls back to probing `PATH`. Honouring the user's actual default is the
    /// point (issue: tunnelled browser ignored the chosen default).
    fn system_default_chromium_browser() -> Option<String> {
        let output = std::process::Command::new("xdg-settings")
            .args(["get", "default-web-browser"])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let desktop = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if desktop.is_empty() {
            return None;
        }

        // Find the .desktop file across the usual application directories.
        let mut dirs: Vec<std::path::PathBuf> = Vec::new();
        if let Some(data_home) = std::env::var_os("XDG_DATA_HOME") {
            dirs.push(std::path::PathBuf::from(data_home).join("applications"));
        } else if let Some(home) = std::env::var_os("HOME") {
            dirs.push(std::path::PathBuf::from(home).join(".local/share/applications"));
        }
        dirs.push(std::path::PathBuf::from("/usr/share/applications"));
        dirs.push(std::path::PathBuf::from("/usr/local/share/applications"));

        let contents = dirs
            .iter()
            .map(|dir| dir.join(&desktop))
            .find_map(|path| std::fs::read_to_string(path).ok())?;

        // First Exec= line; take the binary, drop field codes (%u, %U, …).
        let exec = contents
            .lines()
            .find_map(|line| line.strip_prefix("Exec="))?;
        let bin = exec.split_whitespace().next()?;
        let name = std::path::Path::new(bin)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(bin);

        if Self::is_chromium_browser(name) && rustconn_core::which::is_available(bin) {
            Some(bin.to_string())
        } else {
            None
        }
    }

    /// probes a small candidate list on `PATH`. Returns `None` when none is
    /// found — Firefox and the portal browser cannot take a command-line SOCKS
    /// proxy, so there is nothing to launch through the tunnel.
    fn find_chromium_browser() -> Option<String> {
        // Respect the user's system default browser first, when it is Chromium.
        if let Some(bin) = Self::system_default_chromium_browser() {
            return Some(bin);
        }

        // $BROWSER may be a colon-separated list; take the first Chromium entry.
        if let Ok(browser_env) = std::env::var("BROWSER") {
            for entry in browser_env.split(':').filter(|s| !s.is_empty()) {
                let bin = entry.split_whitespace().next().unwrap_or(entry);
                if Self::is_chromium_browser(bin) && rustconn_core::which::is_available(bin) {
                    return Some(bin.to_string());
                }
            }
        }

        // The candidates below are Linux binary names resolved on PATH. On macOS
        // a Chromium-family browser is an `.app` bundle under /Applications, not
        // a binary on PATH — a GUI app's PATH does not even list it — so the
        // list never matches and detection silently fails. Probe the bundles
        // directly and return the executable inside, which still accepts
        // `--proxy-server`/`--user-data-dir` when launched by its full path.
        #[cfg(target_os = "macos")]
        if let Some(path) = Self::find_macos_chromium_app() {
            return Some(path);
        }

        rustconn_core::protocol::chromium_proxy_browsers()
            .find(|bin| rustconn_core::which::is_available(bin))
            .map(str::to_string)
    }

    /// Locates a Chromium-family browser installed as a macOS `.app` bundle.
    ///
    /// Returns the full path to the executable inside the bundle
    /// (`…/Contents/MacOS/<Name>`), which — unlike the Linux binary names in
    /// `find_chromium_browser`'s candidate list — is what actually exists on
    /// macOS. Both the system `/Applications` and the per-user
    /// `~/Applications` are checked. Launched by this path, the browser still
    /// honours `--proxy-server` and `--user-data-dir`.
    #[cfg(target_os = "macos")]
    fn find_macos_chromium_app() -> Option<String> {
        // (bundle name, executable name inside Contents/MacOS). The executable
        // name is not always the bundle stem — Brave's differs. The bundle probe
        // itself lives in `rustconn_core::which::find_macos_app`, shared with the
        // RDP/VNC/SPICE viewer detection.
        const BUNDLES: &[(&str, &str)] = &[
            ("Google Chrome.app", "Google Chrome"),
            ("Chromium.app", "Chromium"),
            ("Brave Browser.app", "Brave Browser"),
            ("Microsoft Edge.app", "Microsoft Edge"),
            ("Vivaldi.app", "Vivaldi"),
        ];
        rustconn_core::which::find_macos_app(BUNDLES)
            .and_then(|p| p.into_os_string().into_string().ok())
    }

    /// Opens a browser routed through a fresh SOCKS tunnel to the selected SSH
    /// connection — RustConn's take on Ásbrú's incognito browser.
    ///
    /// Uses the embedded browser when this build has it and the
    /// `open_tunnelled_browser_in_embedded` setting is on; otherwise launches an
    /// external Chromium-family browser in an isolated throwaway profile with
    /// `--proxy-server=socks5://…`. The start page comes from the
    /// `tunnel_browser_start_url` setting (default `https://www.google.com`), so
    /// it is visible the tunnel works.
    ///
    /// When the external browser cannot be used — a configured command that is
    /// not Chromium-family or not found, or no Chromium binary at all — RustConn
    /// explains why in a toast and falls back to the embedded browser if this
    /// build has it (it can always tunnel), otherwise stops with an error toast
    /// asking for a Chromium-based browser. The system browser is not an option:
    /// it cannot take a proxy. A tunnel failure is reported and nothing is
    /// opened. It never browses un-tunnelled.
    fn open_browser_via_tunnel(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        connection_id: Uuid,
    ) {
        // Only connections `supports_browser_tunnel` accepts (SSH/SFTP, Tailscale
        // SSH with a matching provider and config) can host a SOCKS tunnel.
        let (can_tunnel, prefer_embedded, start_url, browser_command) = {
            let Ok(state_ref) = state.try_borrow() else {
                return;
            };
            // `None` = the connection is gone; that case falls through so
            // `socks_tunnel_params_for` reports it as such.
            let can_tunnel = state_ref
                .get_connection(connection_id)
                .map(rustconn_core::ssh_tunnel::supports_browser_tunnel);
            let ui = &state_ref.settings().ui;
            (
                can_tunnel,
                ui.open_tunnelled_browser_in_embedded,
                Self::normalize_start_url(&ui.tunnel_browser_start_url),
                ui.tunnel_browser_command.trim().to_string(),
            )
        };
        if can_tunnel == Some(false) {
            // The menu only offers the item for connections the same predicate
            // accepts, so this is a stale row or a direct action call.
            tracing::debug!(
                %connection_id,
                "open-browser-via-tunnel on a connection that cannot host a tunnel"
            );
            crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                "Could not open the SSH tunnel: {}",
                &[&Self::unsupported_tunnel_reason()],
            ));
            return;
        }

        // A repeat click while the tunnel is starting is ignored: the first click
        // already raises one tunnel and one browser. Checked before anything else
        // runs (browser detection spawns a process on this thread). The guard
        // moves into the tunnel callback, so the mark is cleared on success and
        // on failure alike, and by `Drop` on every early return below.
        let Some(start_guard) = BrowserTunnelStart::begin(connection_id) else {
            crate::toast::show_info_toast_on_active_window(&crate::i18n::i18n(
                "Starting the tunnel…",
            ));
            return;
        };

        // Only consulted by the embedded path; a build without it browses external.
        // Decide the external browser up front (does not need the tunnel yet).
        // A configured command wins; empty auto-detects, preferring the system
        // default browser when it is Chromium, then $BROWSER, then PATH.
        let external = Self::resolve_tunnel_browser(&browser_command);

        // Embedded first when the setting asks for it. On a build without the
        // embedded browser this is forced false below, so the external path runs.
        let use_embedded_first = prefer_embedded;

        // Resolve which path to take *before* raising the tunnel, so a launch
        // that cannot proceed never opens a proxy it will not use.
        //
        // The external-browser choice matters only when we are not using the
        // embedded browser first. When it is unusable — a configured command
        // that is not Chromium or not found, or no Chromium at all — the outcome
        // is the same shape: explain the reason, then fall back to the embedded
        // browser if this build has it (it can always tunnel), otherwise stop
        // with an "install a Chromium-based browser" error. The only difference
        // per case is the wording, so it is computed once here.
        //
        // `embedded_available` is a runtime flag rather than raw `cfg!` so the
        // match arms below read the same on both builds; the compiler prunes the
        // dead branch. It is simply whether this build has the feature — no
        // snap-specific gating, for parity with Flatpak.
        #[cfg(feature = "web-embedded")]
        let embedded_available = true;
        #[cfg(not(feature = "web-embedded"))]
        let embedded_available = false;

        // Emits the reason a chosen/auto external browser is unusable. When the
        // embedded browser will pick up the slack the message is informational
        // ("… opening the embedded browser instead"); when nothing can proceed
        // (no embedded browser in this build either) it is an error. Returns the
        // `Path` to take.
        let external_unusable =
            |reason_with_embedded: String, reason_no_embedded: &str| -> TunnelBrowserPath {
                if embedded_available {
                    crate::toast::show_info_toast_on_active_window(&reason_with_embedded);
                    TunnelBrowserPath::Embedded
                } else {
                    crate::toast::show_error_toast_on_active_window(reason_no_embedded);
                    TunnelBrowserPath::Fail
                }
            };

        let path = if use_embedded_first && embedded_available {
            // The command is external-only; an invalid one must not block the
            // embedded path the user actually asked for.
            TunnelBrowserPath::Embedded
        } else {
            match external {
                Ok((browser, extra_args)) => TunnelBrowserPath::External(browser, extra_args),
                Err(TunnelBrowserError::NotChromium(bin)) => external_unusable(
                    crate::i18n::i18n_f(
                        "The tunnelled browser command ‘{}’ can't use a proxy; opening the embedded browser instead. Set a Chromium-based browser or Firefox in Settings to use an external browser.",
                        &[&bin],
                    ),
                    &crate::i18n::i18n_f(
                        "The tunnelled browser command ‘{}’ is not a Chromium-based browser. Only Chromium-family browsers can be proxied through the tunnel; set a chromium/chrome/brave/vivaldi/edge command in Settings.",
                        &[&bin],
                    ),
                ),
                Err(TunnelBrowserError::NotFound(bin)) => external_unusable(
                    crate::i18n::i18n_f(
                        "The tunnelled browser command ‘{}’ was not found on PATH; opening the embedded browser instead. Check the command in Settings, or clear it to auto-detect a browser.",
                        &[&bin],
                    ),
                    &crate::i18n::i18n_f(
                        "The tunnelled browser command ‘{}’ was not found on PATH. Check the command in Settings, or clear it to auto-detect a browser.",
                        &[&bin],
                    ),
                ),
                Err(TunnelBrowserError::NoneFound) => external_unusable(
                    crate::i18n::i18n(
                        "No Chromium-based browser found; opening the embedded browser instead. Install a Chromium-based browser to tunnel an external one.",
                    ),
                    &crate::i18n::i18n(
                        "No Chromium-based browser found. Install a Chromium-based browser, or use a build with the embedded browser, to open a tunnelled browser.",
                    ),
                ),
            }
        };

        // Nothing viable — the toast is already shown, so stop before opening a
        // proxy that would go unused.
        if matches!(path, TunnelBrowserPath::Fail) {
            return;
        }

        // The tunnel comes up on a worker (a dead host would otherwise freeze the
        // window until `ConnectTimeout`); the browser opens from the callback back
        // on the GTK thread. No `Ref` is held across the worker: the params are
        // read and released before it starts. The one-shot closure owns plain `Rc`
        // clones and is stored nowhere, so it forms no cycle.
        crate::toast::show_info_toast_on_active_window(&crate::i18n::i18n("Starting the tunnel…"));
        let state_cb = std::rc::Rc::clone(state);
        let notebook_cb = notebook.clone();
        let sidebar_cb = sidebar.clone();
        Self::raise_socks_tunnel_async(state, connection_id, move |result| {
            // Cleared as soon as the outcome is known, whichever it is.
            drop(start_guard);
            match result {
                Ok(tunnel) => {
                    Self::launch_tunnel_browser(
                        &state_cb,
                        &notebook_cb,
                        &sidebar_cb,
                        connection_id,
                        path,
                        &start_url,
                        tunnel,
                    );
                }
                Err(msg) => {
                    crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                        "Could not open the SSH tunnel: {}",
                        &[&msg],
                    ));
                }
            }
        });
    }

    /// Opens the browser chosen by [`Self::open_browser_via_tunnel`] through a
    /// SOCKS tunnel that is already up, and settles who owns the tunnel.
    ///
    /// The embedded browser consumes the tunnel (its lifetime is the tab's). An
    /// external browser leaves it with the notebook and the external-session
    /// registry; if the browser fails to start the tunnel is dropped here, which
    /// kills `ssh`.
    fn launch_tunnel_browser(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        connection_id: Uuid,
        path: TunnelBrowserPath,
        start_url: &str,
        tunnel: rustconn_core::ssh_tunnel::SshTunnel,
    ) {
        let port = tunnel.local_port();

        let (browser, extra_args) = match path {
            TunnelBrowserPath::External(browser, extra_args) => (browser, extra_args),
            // Embedded (preferred or fallback). Compiled in only when available;
            // `Embedded` cannot be produced on a build without it.
            #[cfg(feature = "web-embedded")]
            TunnelBrowserPath::Embedded => {
                Self::open_embedded_tunnel_browser(
                    state,
                    notebook,
                    connection_id,
                    start_url,
                    tunnel,
                );
                return;
            }
            #[cfg(not(feature = "web-embedded"))]
            TunnelBrowserPath::Embedded => {
                // `state` is only read by the embedded path.
                let _ = state;
                unreachable!("embedded path is unreachable without the feature")
            }
            TunnelBrowserPath::Fail => return,
        };

        let mut cmd = std::process::Command::new(&browser);
        for arg in &extra_args {
            cmd.arg(arg);
        }
        // A profile of its own forces a separate instance: a running browser
        // would otherwise take the URL and load it un-tunnelled. An SSH
        // connection has no private-mode setting; the fresh profile is the
        // isolation here.
        let profile = match rustconn_core::protocol::proxied_browser_args(
            &browser,
            &format!("socks5://127.0.0.1:{port}"),
            start_url,
            false,
        ) {
            Ok((args, profile)) => {
                cmd.args(args);
                profile
            }
            Err(e) => {
                crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                    "Failed to open browser: {}",
                    &[&e.to_string()],
                ));
                sidebar.update_connection_status(&connection_id.to_string(), "failed");
                return;
            }
        };

        match cmd.spawn() {
            Ok(child) => {
                // Keep the tunnel alive behind the detached browser (reclaimed at
                // app exit), the same way the Custom web-browser path does. The
                // profile goes when the browser exits.
                let session_id = uuid::Uuid::new_v4();
                notebook.store_ssh_tunnel(session_id, tunnel);
                Self::track_proxied_browser(session_id, connection_id, child, profile);
                sidebar.update_connection_status(&connection_id.to_string(), "");
                crate::toast::show_info_toast_on_active_window(&crate::i18n::i18n(
                    "Opened a tunnelled browser",
                ));
            }
            Err(e) => {
                crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                    "Failed to open browser: {}",
                    &[&e.to_string()],
                ));
            }
        }
    }

    /// Opens the tunnelled start page in the embedded WebKit browser, consuming
    /// the tunnel so its lifetime is tied to the tab.
    ///
    /// On a load failure the tab is closed and a toast explains why — the tunnel
    /// has already been consumed by the widget at that point, so there is
    /// nothing to reclaim.
    #[cfg(feature = "web-embedded")]
    fn open_embedded_tunnel_browser(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        connection_id: Uuid,
        start_url: &str,
        tunnel: rustconn_core::ssh_tunnel::SshTunnel,
    ) {
        use std::rc::Rc;

        use crate::embedded_web::EmbeddedWebWidget;

        let conn_name = state
            .try_borrow()
            .ok()
            .and_then(|s| s.get_connection(connection_id).map(|c| c.name.clone()))
            .unwrap_or_else(|| "SSH".to_string());
        let title = crate::i18n::i18n_f("{} (tunnel)", &[&conn_name]);

        let cfg = rustconn_core::models::WebConfig {
            browser_mode: rustconn_core::models::WebBrowserMode::Embedded,
            ..Default::default()
        };
        match EmbeddedWebWidget::new(connection_id, start_url, &cfg, None, Some(tunnel), None) {
            Ok(widget) => {
                let widget = Rc::new(widget);
                let session_id = uuid::Uuid::new_v4();
                let notebook_fail = notebook.clone();
                widget.connect_initial_load_failed(move |message| {
                    notebook_fail.close_session(session_id);
                    crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                        "Could not open the page: {}",
                        &[&message],
                    ));
                });
                notebook.add_embedded_web_tab(session_id, connection_id, &title, widget);
            }
            Err(e) => {
                tracing::error!(error = %e, "Embedded tunnel browser failed");
                crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                    "Failed to open browser: {}",
                    &[&e.to_string()],
                ));
            }
        }
    }

    /// Opens a Web bookmark and observes an embedded session it creates.
    pub(crate) fn handle_web_connect_observed(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: &SharedSidebar,
        connection_id: Uuid,
        observer: Option<types::SessionStartObserver>,
    ) {
        let (url, mut web_config, conn_name) = {
            let Ok(state_ref) = state.try_borrow() else {
                return;
            };
            let Some(conn) = state_ref.get_connection(connection_id) else {
                return;
            };
            let url = conn.host.clone();
            let name = conn.name.clone();
            let config = match &conn.protocol_config {
                rustconn_core::models::ProtocolConfig::Web(cfg) => cfg.clone(),
                _ => rustconn_core::models::WebConfig::default(),
            };
            drop(state_ref);
            // Update last_connected timestamp
            if let Ok(mut state_mut) = state.try_borrow_mut()
                && let Err(e) = state_mut.update_last_connected(connection_id)
            {
                tracing::warn!(?e, "Failed to update last_connected for web connection");
            }
            (url, config, name)
        };

        if url.is_empty() {
            tracing::warn!(connection_id = %connection_id, "Web connection has empty URL");
            sidebar.update_connection_status(&connection_id.to_string(), "");
            return;
        }

        // A proxy with a login goes through a local relay; from here on the
        // browser sees only the relay's address.
        let socks_relay = match Self::start_web_socks_relay(state, connection_id, &mut web_config) {
            Ok(relay) => relay,
            Err(message) => {
                sidebar.update_connection_status(&connection_id.to_string(), "failed");
                crate::toast::show_error_toast_on_active_window(&message);
                return;
            }
        };

        // Silence unused-variable warnings when web-embedded feature is disabled.
        // These are used in the #[cfg(feature = "web-embedded")] block below.
        #[cfg(not(feature = "web-embedded"))]
        {
            // `notebook` and `conn_name` are now used by the Custom-browser tunnel
            // path regardless of the feature; only `observer` is embedded-only.
            let _ = &observer;

            // The stored mode is kept as-is even though this build cannot honour
            // it — see `rustconn_core::models::WebBrowserMode`. Say so out loud:
            // the user asked for an in-tab browser and is getting the system one,
            // and at debug level nobody would ever find out why.
            if web_config.browser_mode == rustconn_core::models::WebBrowserMode::Embedded {
                tracing::info!(
                    connection = %conn_name,
                    "Connection asks for the embedded browser, which this build was \
                     compiled without; opening in the system browser instead"
                );
            }
        }

        // Embedded mode: create an in-tab WebKitGTK widget
        #[cfg(feature = "web-embedded")]
        if web_config.browser_mode == rustconn_core::models::WebBrowserMode::Embedded {
            use std::rc::Rc;

            use crate::embedded_web::EmbeddedWebWidget;

            // Retrieve cached credentials for autofill (username + SecretString password)
            let credentials = {
                let Ok(state_ref) = state.try_borrow() else {
                    return;
                };
                state_ref
                    .get_cached_credentials(connection_id)
                    // Clone the `SecretString` directly rather than exposing it
                    // into a plaintext `String`: the clone copies the protected
                    // value without ever materializing an un-zeroized allocation.
                    .map(|creds| (creds.username.clone(), creds.password.clone()))
            };

            // Raise the SSH SOCKS tunnel this Web connection browses through, if
            // one is configured. A failure to bring it up aborts the launch
            // rather than silently browsing directly (which would defeat the
            // point and could leak traffic the user meant to tunnel). The spawn
            // runs off the GTK thread so a dead jump host cannot freeze the
            // window; the widget is built in the callback once the tunnel is up.
            sidebar.update_connection_status(&connection_id.to_string(), "connecting");
            let notebook_cb = notebook.clone();
            let sidebar_cb = sidebar.clone();
            let state_cb = Rc::clone(state);
            Self::raise_web_socks_tunnel_async(state, &web_config.clone(), move |result| {
                let socks_tunnel = match result {
                    Ok(tunnel) => tunnel,
                    Err(msg) => {
                        tracing::error!(connection = %conn_name, error = %msg, "Web SOCKS tunnel failed");
                        sidebar_cb.update_connection_status(&connection_id.to_string(), "failed");
                        crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                            "Could not open the SSH tunnel: {}",
                            &[&msg],
                        ));
                        return;
                    }
                };

                let session_id = uuid::Uuid::new_v4();
                match EmbeddedWebWidget::new(
                    connection_id,
                    &url,
                    &web_config,
                    credentials,
                    socks_tunnel,
                    socks_relay,
                ) {
                    Ok(widget) => {
                        let widget = Rc::new(widget);

                        // Wire zoom persistence: save zoom level to connection config on change
                        let state_for_zoom = Rc::clone(&state_cb);
                        let conn_id_for_zoom = connection_id;
                        widget.connect_zoom_changed(move |new_zoom| {
                            if let Ok(mut state_mut) = state_for_zoom.try_borrow_mut() {
                                state_mut.update_web_zoom_level(conn_id_for_zoom, new_zoom);
                            }
                        });

                        // If the very first load never paints (bad host, DNS/SOCKS
                        // `Name or service not known`), close the blank tab and toast
                        // the reason instead of leaving an empty browser behind.
                        {
                            let notebook_for_fail = notebook_cb.clone();
                            let sidebar_for_fail = sidebar_cb.clone();
                            let conn_id_str = connection_id.to_string();
                            widget.connect_initial_load_failed(move |message| {
                                notebook_for_fail.close_session(session_id);
                                sidebar_for_fail.update_connection_status(&conn_id_str, "");
                                crate::toast::show_error_toast_on_active_window(
                                    &crate::i18n::i18n_f(
                                        "Could not open the page: {}",
                                        &[&message],
                                    ),
                                );
                            });
                        }

                        notebook_cb.add_embedded_web_tab(
                            session_id,
                            connection_id,
                            &conn_name,
                            widget,
                        );
                        if let Some(observer) = observer {
                            observer.complete(session_id);
                        }
                        sidebar_cb
                            .update_connection_status(&connection_id.to_string(), "connected");
                        tracing::info!(
                            connection = %conn_name,
                            %connection_id,
                            "Embedded web session started"
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            connection = %conn_name,
                            error = %e,
                            "Failed to create embedded web widget"
                        );
                        sidebar_cb.update_connection_status(&connection_id.to_string(), "failed");
                        crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                            "Failed to open URL: {}",
                            &[&e.to_string()],
                        ));
                    }
                }
            });
            return;
        }

        // Custom mode: launch user-specified browser as a subprocess
        let proxied = web_config.tunnel_via.is_some() || web_config.socks_proxy.is_some();
        let external_browser =
            if web_config.browser_mode == rustconn_core::models::WebBrowserMode::Custom {
                web_config.browser.clone()
            } else if proxied {
                // The portal picks the system browser and cannot be told about a
                // proxy, so a proxied page goes to a browser that can be.
                Self::find_chromium_browser()
                    .or_else(rustconn_core::protocol::first_installed_proxy_browser)
            } else {
                None
            };
        if let Some(browser) = external_browser {
            // Raise the SOCKS tunnel first, if configured, off the GTK thread so
            // a dead jump host cannot freeze the window; the browser is launched
            // in the callback once the tunnel is up. A failure aborts the launch
            // rather than browsing directly, matching the embedded path.
            sidebar.update_connection_status(&connection_id.to_string(), "connecting");
            let notebook_cb = notebook.clone();
            let sidebar_cb = sidebar.clone();
            Self::raise_web_socks_tunnel_async(state, &web_config.clone(), move |result| {
                let socks_tunnel = match result {
                    Ok(tunnel) => tunnel,
                    Err(msg) => {
                        tracing::error!(connection = %conn_name, error = %msg, "Web SOCKS tunnel failed");
                        sidebar_cb.update_connection_status(&connection_id.to_string(), "failed");
                        crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                            "Could not open the SSH tunnel: {}",
                            &[&msg],
                        ));
                        return;
                    }
                };

                let mut cmd = std::process::Command::new(&browser);
                let family = rustconn_core::protocol::browser_family(&browser);

                // Route the browser through the proxy. A browser that cannot be
                // pointed at one is not opened, so the page never loads directly.
                let proxy_uri = socks_tunnel
                    .as_ref()
                    .map(|tunnel| format!("socks5://127.0.0.1:{}", tunnel.local_port()))
                    .or_else(|| web_config.socks_proxy.clone());
                let proxy_profile = if let Some(proxy_uri) = proxy_uri {
                    let Ok((args, profile)) = rustconn_core::protocol::proxied_browser_args(
                        &browser,
                        &proxy_uri,
                        &url,
                        web_config.private_mode,
                    ) else {
                        sidebar_cb.update_connection_status(&connection_id.to_string(), "failed");
                        crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n(
                            "This browser can't use a proxy, so the page was not opened. Use a Chromium-based browser, Firefox or the embedded browser.",
                        ));
                        return;
                    };
                    cmd.args(args);
                    Some(profile)
                } else {
                    None
                };
                let tunnelled_isolated = proxy_profile.is_some();

                // Add private mode flag for known browsers. A proxied launch got
                // its flag from `proxied_browser_args` above: Firefox keeps
                // `--private-window`, and a Chromium instance in its own
                // throwaway profile is clean, where `--incognito` would rejoin the
                // running process.
                if web_config.private_mode {
                    use rustconn_core::protocol::BrowserFamily;
                    if family == Some(BrowserFamily::Firefox) && !tunnelled_isolated {
                        cmd.arg("--private-window");
                    } else if family == Some(BrowserFamily::Chromium) && !tunnelled_isolated {
                        cmd.arg("--incognito");
                    }
                }

                if !tunnelled_isolated {
                    cmd.arg(&url);
                }

                match cmd.spawn() {
                    Ok(child) => {
                        tracing::info!(browser = %browser, "Web bookmark opened in custom browser");
                        sidebar_cb.update_connection_status(&connection_id.to_string(), "");
                        // Keep the tunnel alive behind the detached browser. There is
                        // no tab to own it, so it is parked in the notebook tunnel map
                        // under a synthetic session id and registered with the external
                        // session registry; both are reclaimed at app exit. The child
                        // is handed over so the registry reaps it — its exit is when
                        // the tunnel could be dropped, though today that waits for exit
                        // of RustConn (documented tradeoff, as for RDP/VNC viewers).
                        // The browser profile, and the relay of a proxy with a
                        // login, go as soon as the browser exits: the relay accepts
                        // any local client and holds the proxy password.
                        if let Some(tunnel) = socks_tunnel {
                            let session_id = uuid::Uuid::new_v4();
                            notebook_cb.store_ssh_tunnel(session_id, tunnel);
                            Self::track_proxied_browser(
                                session_id,
                                connection_id,
                                child,
                                proxy_profile,
                            );
                        } else if proxy_profile.is_some() || socks_relay.is_some() {
                            // A SOCKS proxy without a tunnel is not an external
                            // session; what it needs just waits for the browser.
                            crate::external_session::drop_when_exits(
                                child,
                                (proxy_profile, socks_relay),
                            );
                        }
                        if let Some(app) = gtk4::gio::Application::default()
                            && let Some(gtk_app) = app.downcast_ref::<gtk4::Application>()
                            && let Some(window) = gtk_app.active_window()
                        {
                            crate::toast::show_toast_on_window(
                                &window,
                                &crate::i18n::i18n("Opened in browser"),
                                crate::toast::ToastType::Success,
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!(browser = %browser, error = %e, "Failed to launch custom browser");
                        sidebar_cb.update_connection_status(&connection_id.to_string(), "failed");
                        // The tunnel (if any) drops here with `socks_tunnel`, killing
                        // its ssh process — nothing is browsing through it.
                        crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                            "Failed to open URL: {}",
                            &[&e.to_string()],
                        ));
                    }
                }
            });
            return;
        }

        // System mode (default): use gtk4::UriLauncher for portal-aware URL opening.
        // Reaching this with a proxy means no browser that can use one was found.
        if proxied {
            sidebar.update_connection_status(&connection_id.to_string(), "failed");
            crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n(
                "No browser that can use a proxy was found, so the page was not opened. Install Chromium or Firefox, or use the embedded browser.",
            ));
            return;
        }
        let launcher = gtk4::UriLauncher::new(&url);
        let sidebar_clone = sidebar.clone();
        launcher.launch(
            gtk4::Window::NONE,
            gtk4::gio::Cancellable::NONE,
            move |result| {
                if let Err(e) = result {
                    tracing::error!(url = %url, error = %e, "Failed to open URL in browser");
                    sidebar_clone.update_connection_status(&connection_id.to_string(), "failed");
                    crate::toast::show_error_toast_on_active_window(&crate::i18n::i18n_f(
                        "Failed to open URL: {}",
                        &[&e.to_string()],
                    ));
                } else {
                    tracing::info!("Web bookmark opened in browser");
                    // Clear "connecting" status — Web has no persistent session
                    sidebar_clone.update_connection_status(&connection_id.to_string(), "");
                    // Show success toast on active window
                    if let Some(app) = gtk4::gio::Application::default()
                        && let Some(gtk_app) = app.downcast_ref::<gtk4::Application>()
                        && let Some(window) = gtk_app.active_window()
                    {
                        crate::toast::show_toast_on_window(
                            &window,
                            &crate::i18n::i18n("Opened in browser"),
                            crate::toast::ToastType::Success,
                        );
                    }
                }
            },
        );
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
        // On macOS, `open` handles sftp:// URIs natively (delegates to Finder or
        // a registered SFTP client like Cyberduck/Transmit).
        #[cfg(target_os = "macos")]
        {
            let mut cmd = std::process::Command::new("open");
            cmd.arg(uri);
            rustconn_core::sftp::apply_agent_env(&mut cmd);
            if cmd.spawn().is_ok() {
                tracing::info!(%uri, "Launched macOS open for SFTP");
                return;
            }
            tracing::warn!(%uri, "macOS open failed for SFTP URI");
        }

        // Linux: Try dolphin first (KDE)
        #[cfg(not(target_os = "macos"))]
        {
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

            // Last resort — xdg-open
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
    }

    /// Warning shown when the external file-manager path is asked to reach a
    /// host behind a jump host, which its SFTP backend cannot do.
    ///
    /// Shows a warning toast on the window hosting `notebook`.
    ///
    /// `handle_sftp_connect_internal` has no `ToastOverlay` of its own — it is
    /// reached from a double-click as well as from the sidebar action — so it
    /// reaches the toast through the notebook's root window.
    fn sftp_warn_on_window(notebook: &SharedNotebook, message: &str) {
        if let Some(root) = notebook.widget().root()
            && let Some(window) = root.downcast_ref::<gtk4::Window>()
        {
            crate::toast::show_toast_on_window(window, message, crate::toast::ToastType::Warning);
        }
    }

    /// The `open-sftp` action for the connection `conn_id`, once any key
    /// unlock is done: opens mc in a local shell tab or the file manager.
    fn open_sftp_for_connection(
        state_clone: &SharedAppState,
        notebook_clone: &SharedNotebook,
        split_view_clone: &SharedSplitView,
        toast_clone: &SharedToastOverlay,
        conn_id: Uuid,
    ) {
        let retry = {
            let state = state_clone.clone();
            let notebook = notebook_clone.clone();
            let split_view = split_view_clone.clone();
            let toast = toast_clone.clone();
            move || {
                Self::open_sftp_for_connection(&state, &notebook, &split_view, &toast, conn_id);
            }
        };
        if super::protocols::start_gateway_probe(state_clone, conn_id, retry) {
            return;
        }

        let state_ref = state_clone.borrow();
        let Some(stored_conn) = state_ref.get_connection(conn_id) else {
            return;
        };
        // The SFTP builders resolve the route from the connection they are given.
        let mut launch_conn = stored_conn.clone();
        if let Some(gateway) = state_ref.chosen_gateway(stored_conn, &state_ref.list_groups_owned())
        {
            rustconn_core::connection::gateway_failover::use_gateway(&mut launch_conn, gateway);
        }
        let conn = &launch_conn;
        let use_mc = state_ref.settings().terminal.sftp_use_mc;

        // Collect groups for SSH inheritance resolution, and connections so a
        // jump host stored as a reference (`jump_host_id`) can be resolved.
        let groups: Vec<rustconn_core::models::ConnectionGroup> = state_ref.list_groups_owned();
        let connections: Vec<rustconn_core::Connection> = state_ref.list_connections_owned();
        // The outermost bastion tier, for connections that inherit.
        let network = state_ref.settings().network.clone();

        // Ensure SSH key is in agent before SFTP (mc and
        // file managers cannot pass identity files directly).
        let key_path = rustconn_core::sftp::get_ssh_key_path(conn, &groups)
            .and_then(|p| rustconn_core::resolve_key_path(&p));

        if use_mc {
            // Open mc in a local shell tab with SFTP panel
            let tab = super::mc_launch::McTab {
                connection_id: conn_id,
                conn: conn.clone(),
                connections,
                groups,
                network,
                terminal_settings: state_ref.settings().terminal.clone(),
                announce: true,
            };
            drop(state_ref);

            // Add SSH key to agent if configured
            if let Some(ref kp) = key_path {
                if !rustconn_core::sftp::is_ssh_agent_available() {
                    toast_clone.show_warning(&crate::i18n::i18n(
                        "SSH agent not running. Run 'eval $(ssh-agent)' and retry.",
                    ));
                }
                tracing::info!(?kp, "Adding SSH key to agent for mc");
                match rustconn_core::sftp::ensure_identity_loaded(kp) {
                    Ok(()) => {}
                    // The agent may hold the key anyway (the user loaded
                    // it); mc finds out when it logs in.
                    Err(rustconn_core::IdentityLoadError::Unverified(detail)) => {
                        tracing::warn!(?kp, %detail, "opening mc without adding the SSH key");
                        toast_clone.show_warning(&crate::i18n::i18n(
                            "Could not check whether the SSH agent holds the key. Opening mc anyway.",
                        ));
                    }
                    Err(rustconn_core::IdentityLoadError::Failed(stderr)) => {
                        toast_clone.show_error(&format!(
                            "{}: {}",
                            crate::i18n::i18n("SSH key not available"),
                            stderr
                        ));
                        return;
                    }
                }
            }

            super::mc_launch::open_mc_tab(
                state_clone,
                notebook_clone,
                Some(split_view_clone),
                None,
                tab,
            );
        } else {
            // Open file manager with sftp:// URI. The home directory is
            // resolved later off the main thread; base_uri (server root)
            // is the fallback (issue #212).
            let Some(base_uri) = rustconn_core::sftp::build_sftp_uri_from_connection(conn) else {
                toast_clone.show_warning(&crate::i18n::i18n(
                    "SFTP is only available for SSH connections.",
                ));
                drop(state_ref);
                return;
            };
            let conn_for_uri = conn.clone();
            let groups_for_uri = groups.clone();
            let connections_for_uri = connections.clone();
            let network_for_uri = network.clone();
            let has_jump_host = !rustconn_core::connection::resolve_jump_chain(
                conn,
                &connections,
                &groups,
                &network,
            )
            .is_empty();
            drop(state_ref);

            // Warn in Flatpak: external file managers cannot access
            // the sandbox's SSH agent, so authentication will likely fail.
            if rustconn_core::flatpak::is_flatpak() {
                tracing::warn!(
                    "Flatpak: external file manager may not authenticate \
                     (no access to sandbox SSH_AUTH_SOCK)"
                );
                toast_clone.show_warning(&crate::i18n::i18n(
                    // xgettext keeps the spaces after the `\` (Rust drops
                    // them), so this line keeps the column of the copy in
                    // `handle_sftp_connect_internal`: one msgid for both.
                    "External file managers cannot access SSH agent in Flatpak. \
                         Enable \"SFTP via mc\" in Settings for reliable access.",
                ));
            }

            // The GVFS sftp backend spawns its own ssh with a fixed argument
            // list and reads no configuration we can point it at, so a
            // bastion cannot be applied on this path at all (issue #255).
            // Instead of only warning, fall back to the mc path which does
            // support jump hosts via a generated ssh_config wrapper.
            if has_jump_host {
                tracing::info!(
                    "Jump host configured — falling back to mc for SFTP \
                     (the file manager's GVFS backend cannot route through a bastion)"
                );
                toast_clone.show_toast(&crate::i18n::i18n(
                    "Jump host detected. Opening SFTP via mc instead of the file manager.",
                ));

                let terminal_settings = state_clone
                    .try_borrow()
                    .ok()
                    .map(|s| s.settings().terminal.clone())
                    .unwrap_or_default();
                super::mc_launch::open_mc_tab(
                    state_clone,
                    notebook_clone,
                    Some(split_view_clone),
                    None,
                    super::mc_launch::McTab {
                        connection_id: conn_id,
                        conn: conn_for_uri,
                        connections: connections_for_uri,
                        groups: groups_for_uri,
                        network: network_for_uri,
                        terminal_settings,
                        announce: false,
                    },
                );
                return;
            }

            tracing::info!(%base_uri, "Opening SFTP file browser");
            toast_clone.show_toast(&crate::i18n::i18n("Opening SFTP…"));

            // Add SSH key to agent in background, then open URI
            let toast_cb = toast_clone.clone();
            let key_for_add = key_path.clone();

            // ssh-add in background thread, then launch file
            // manager as a direct subprocess (not via
            // UriLauncher/D-Bus) so it inherits SSH_AUTH_SOCK.
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
                        if let Err(stderr) = rustconn_core::sftp::ensure_identity_loaded(kp) {
                            tracing::warn!(
                                %stderr,
                                "ssh-add failed — file manager may not authenticate"
                            );
                        }
                    }
                    // Resolve the home-directory URI after the key is in
                    // the agent, off the GTK main thread; fall back to the
                    // server-root URI if resolution fails.
                    let uri = rustconn_core::sftp::build_sftp_browser_uri(
                        &conn_for_uri,
                        &connections_for_uri,
                        &groups_for_uri,
                        &network_for_uri,
                    )
                    .unwrap_or(base_uri);
                    (uri, rustconn_core::sftp::is_ssh_agent_available())
                },
                move |(uri, agent_ok): (String, bool)| {
                    // Launch file manager as a direct subprocess
                    // so it inherits SSH_AUTH_SOCK. UriLauncher
                    // goes through D-Bus/portal which may not
                    // pass our env to an already-running Dolphin.
                    Self::sftp_launch_file_manager(&uri);
                    if !agent_ok {
                        toast_cb.show_warning(&crate::i18n::i18n(
                            "SSH agent not running — file manager may not authenticate.",
                        ));
                    }
                },
            );
        }
    }

    /// Handles SFTP connection — opens file manager or mc
    ///
    /// Called when user clicks "Connect" on an SFTP-type connection.
    /// Reuses the same logic as the `open-sftp` sidebar action.
    /// Performs SSH port check before opening the file manager or mc. An
    /// encrypted key the agent does not hold is unlocked first in RustConn's
    /// dialog (`key_unlock::unlock_then`).
    pub(crate) fn handle_sftp_connect(
        state: &SharedAppState,
        notebook: &SharedNotebook,
        sidebar: Option<&SharedSidebar>,
        split_view: Option<&SharedSplitView>,
        connection_id: Uuid,
    ) {
        let targets = {
            let state_ref = state.borrow();
            let Some(conn) = state_ref.get_connection(connection_id) else {
                return;
            };
            super::key_unlock::sftp_unlock_targets(conn, &state_ref.list_groups_owned())
        };
        let state_rc = state.clone();
        let notebook_rc = notebook.clone();
        let sidebar = sidebar.cloned();
        let split_view = split_view.cloned();
        super::key_unlock::unlock_then(notebook.widget(), targets, move || {
            Self::handle_sftp_connect_checked(
                &state_rc,
                &notebook_rc,
                sidebar.as_ref(),
                split_view.as_ref(),
                connection_id,
            );
        });
    }

    /// [`Self::handle_sftp_connect`] once any key unlock is done.
    fn handle_sftp_connect_checked(
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
            let should = conn.should_pre_connect_check(&settings.connection);
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
                        if let Ok(mut state_mut) = state_clone.try_borrow_mut()
                            && let Some(conn) = state_mut.get_connection(connection_id).cloned()
                        {
                            let username = conn.username.clone();
                            state_mut.record_connection_attempt_failed(
                                &conn,
                                username.as_deref(),
                                &e.to_string(),
                            );
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
        let Some(stored_conn) = state_ref.get_connection(connection_id) else {
            return;
        };
        // The SFTP builders resolve the route from the connection they are given.
        let mut launch_conn = stored_conn.clone();
        if let Some(gateway) = state_ref.chosen_gateway(stored_conn, &state_ref.list_groups_owned())
        {
            rustconn_core::connection::gateway_failover::use_gateway(&mut launch_conn, gateway);
        }
        let conn = &launch_conn;
        let use_mc = state_ref.settings().terminal.sftp_use_mc;
        let groups: Vec<rustconn_core::models::ConnectionGroup> = state_ref.list_groups_owned();
        // Needed to resolve a jump host stored as a connection reference.
        let connections: Vec<rustconn_core::Connection> = state_ref.list_connections_owned();
        // The outermost bastion tier, for connections that inherit.
        let network = state_ref.settings().network.clone();
        let key_path = rustconn_core::sftp::get_ssh_key_path(conn, &groups)
            .and_then(|p| rustconn_core::resolve_key_path(&p));

        if use_mc {
            let tab = super::mc_launch::McTab {
                connection_id,
                conn: conn.clone(),
                connections,
                groups,
                network,
                terminal_settings: state_ref.settings().terminal.clone(),
                announce: false,
            };
            drop(state_ref);

            if let Some(ref kp) = key_path {
                match rustconn_core::sftp::ensure_identity_loaded(kp) {
                    Ok(()) => {}
                    // The agent may hold the key anyway; mc finds out when it
                    // logs in.
                    Err(rustconn_core::IdentityLoadError::Unverified(detail)) => {
                        tracing::warn!(?kp, %detail, "opening mc without adding the SSH key");
                    }
                    Err(rustconn_core::IdentityLoadError::Failed(stderr)) => {
                        tracing::warn!(?kp, %stderr, "ssh-add failed");
                        if let Some(sb) = sidebar {
                            sb.update_connection_status(&connection_id.to_string(), "failed");
                        }
                        return;
                    }
                }
            }

            super::mc_launch::open_mc_tab(state, notebook, split_view, sidebar, tab);
        } else {
            let Some(base_uri) = rustconn_core::sftp::build_sftp_uri_from_connection(conn) else {
                drop(state_ref);
                return;
            };
            // Clone for off-thread home-directory resolution (issue #212);
            // base_uri (server root) stays as the fallback.
            let conn_for_uri = conn.clone();
            let groups_for_uri = groups.clone();
            let connections_for_uri = connections.clone();
            let network_for_uri = network.clone();
            let has_jump_host = !rustconn_core::connection::resolve_jump_chain(
                conn,
                &connections,
                &groups,
                &network,
            )
            .is_empty();
            drop(state_ref);

            // Warn in Flatpak: external file managers cannot access
            // the sandbox's SSH agent, so authentication will likely fail.
            if rustconn_core::flatpak::is_flatpak() {
                tracing::warn!(
                    "Flatpak: external file manager may not authenticate \
                     (no access to sandbox SSH_AUTH_SOCK)"
                );
                Self::sftp_warn_on_window(
                    notebook,
                    &crate::i18n::i18n(
                        "External file managers cannot access SSH agent in Flatpak. \
                         Enable \"SFTP via mc\" in Settings for reliable access.",
                    ),
                );
            }

            // The GVFS sftp backend spawns its own ssh with a fixed argument list
            // and reads no configuration we can point it at, so a bastion cannot
            // be applied on this path at all (issue #255).
            // Fall back to mc which supports jump hosts via a generated ssh_config.
            if has_jump_host {
                tracing::info!(
                    "Jump host configured — falling back to mc for SFTP \
                     (the file manager's GVFS backend cannot route through a bastion)"
                );
                Self::sftp_warn_on_window(
                    notebook,
                    &crate::i18n::i18n(
                        "Jump host detected. Opening SFTP via mc instead of the file manager.",
                    ),
                );

                let terminal_settings = state
                    .try_borrow()
                    .ok()
                    .map(|s| s.settings().terminal.clone())
                    .unwrap_or_default();
                super::mc_launch::open_mc_tab(
                    state,
                    notebook,
                    split_view,
                    sidebar,
                    super::mc_launch::McTab {
                        connection_id,
                        conn: conn_for_uri,
                        connections: connections_for_uri,
                        groups: groups_for_uri,
                        network: network_for_uri,
                        terminal_settings,
                        announce: false,
                    },
                );
                return;
            }

            tracing::info!(%base_uri, "SFTP connect: opening file browser");

            let key_for_add = key_path.clone();
            crate::utils::spawn_blocking_with_callback(
                move || {
                    if let Some(ref kp) = key_for_add
                        && let Err(stderr) = rustconn_core::sftp::ensure_identity_loaded(kp)
                    {
                        tracing::warn!(?kp, %stderr, "ssh-add failed");
                    }
                    // Resolve the home-directory URI after the key is in the
                    // agent, off the GTK main thread; fall back to server root.
                    rustconn_core::sftp::build_sftp_browser_uri(
                        &conn_for_uri,
                        &connections_for_uri,
                        &groups_for_uri,
                        &network_for_uri,
                    )
                    .unwrap_or(base_uri)
                },
                move |uri: String| {
                    // Launch file manager as a direct subprocess
                    // so it inherits SSH_AUTH_SOCK. UriLauncher
                    // goes through D-Bus which may not pass env.
                    Self::sftp_launch_file_manager(&uri);
                },
            );
        }
    }
}

#[cfg(test)]
mod browser_tunnel_tests {
    use super::*;

    #[test]
    fn second_begin_for_same_connection_is_refused() {
        let id = Uuid::new_v4();
        let first = BrowserTunnelStart::begin(id);
        assert!(first.is_some());
        assert!(BrowserTunnelStart::begin(id).is_none());
    }

    #[test]
    fn other_connection_is_not_blocked() {
        let a = BrowserTunnelStart::begin(Uuid::new_v4());
        let b = BrowserTunnelStart::begin(Uuid::new_v4());
        assert!(a.is_some() && b.is_some());
    }

    #[test]
    fn drop_clears_the_mark() {
        let id = Uuid::new_v4();
        drop(BrowserTunnelStart::begin(id));
        assert!(BrowserTunnelStart::begin(id).is_some());
    }

    /// The success and failure paths consume the callback; a dead worker drops it
    /// unrun. Either way the guard it owns must release the mark.
    #[test]
    fn guard_moved_into_a_dropped_closure_clears_the_mark() {
        let id = Uuid::new_v4();
        let guard = BrowserTunnelStart::begin(id);
        let callback = move |_: Result<(), String>| {
            let _guard = &guard;
        };
        assert!(BrowserTunnelStart::begin(id).is_none());
        drop(callback);
        assert!(BrowserTunnelStart::begin(id).is_some());
    }
}
