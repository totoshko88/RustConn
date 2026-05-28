//! Edit dialogs for main window
//!
//! This module contains functions for editing connections and groups,
//! showing connection details, and quick connect dialog.

use super::MainWindow;
use super::edit_group::show_edit_group_dialog;
use crate::alert;
use crate::dialogs::ConnectionDialog;
use crate::embedded_rdp::{EmbeddedRdpWidget, RdpConfig as EmbeddedRdpConfig};
use crate::i18n::{i18n, i18n_f};
use crate::sidebar::ConnectionSidebar;
use crate::split_view::SplitViewBridge;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Button, Label, Orientation};
use libadwaita as adw;
use rustconn_core::models::PasswordSource;
use rustconn_core::sync::SyncMode;
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Type alias for shared notebook reference
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Type alias for shared split view reference
pub type SharedSplitView = Rc<SplitViewBridge>;

/// Edits the selected connection or group
pub fn edit_selected_connection(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    // Get selected item using sidebar's method (works in both single and multi-selection modes)
    let Some(conn_item) = sidebar.get_selected_item() else {
        return;
    };

    let id_str = conn_item.id();
    let Ok(id) = Uuid::parse_str(&id_str) else {
        return;
    };

    if conn_item.is_group() {
        // Edit group - show simple rename dialog
        show_edit_group_dialog(window, state.clone(), sidebar.clone(), id);
    } else {
        // Edit connection
        let state_ref = state.borrow();
        let Some(conn) = state_ref.get_connection(id).cloned() else {
            return;
        };
        drop(state_ref);

        let dialog = ConnectionDialog::new(Some(&window.clone().upcast()), state.clone());
        dialog.setup_key_file_chooser(Some(&window.clone().upcast()));

        // Set available groups
        {
            let state_ref = state.borrow();
            let mut groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
            groups.sort_by_key(|a| a.name.to_lowercase());
            dialog.set_groups(&groups);
        }

        // Set available connections for Jump Host (excluding self)
        {
            let state_ref = state.borrow();
            let connections: Vec<_> = state_ref
                .list_connections()
                .into_iter()
                .filter(|c| c.id != id)
                .cloned()
                .collect();
            dialog.set_connections(&connections);
        }

        // Populate variable dropdown with secret global variables
        // Must be before set_connection so variable selection works
        {
            let state_ref = state.borrow();
            let global_vars = state_ref.settings().global_variables.clone();
            dialog.set_global_variables(&global_vars);
        }

        dialog.set_connection(&conn);

        // Check if connection belongs to an Import group → configure read-only synced fields
        {
            let state_ref = state.borrow();
            if let Some(group_id) = conn.group_id {
                // Walk up to root group to check sync_mode
                let mut current_id = Some(group_id);
                while let Some(gid) = current_id {
                    if let Some(group) = state_ref.get_group(gid) {
                        if group.parent_id.is_none() {
                            // Root group found — check sync_mode
                            if group.sync_mode == SyncMode::Import {
                                dialog.configure_import_group_mode();
                            }
                            break;
                        }
                        current_id = group.parent_id;
                    } else {
                        break;
                    }
                }
            }
        }

        // Set up password visibility toggle and source visibility
        dialog.connect_password_visibility_toggle();
        dialog.connect_password_source_visibility();
        dialog.update_password_row_visibility();

        // Set up password load button with KeePass settings
        {
            let state_ref = state.borrow();
            let settings = state_ref.settings();
            let groups: Vec<rustconn_core::models::ConnectionGroup> =
                state_ref.list_groups().iter().cloned().cloned().collect();
            dialog.connect_password_load_button_with_groups(
                settings.secrets.kdbx_enabled,
                settings.secrets.kdbx_path.clone(),
                settings.secrets.kdbx_password.as_ref(),
                settings.secrets.kdbx_key_file.clone(),
                groups.clone(),
                settings.secrets.clone(),
            );
            dialog.connect_vault_test_button(
                settings.secrets.kdbx_enabled,
                settings.secrets.kdbx_path.clone(),
                settings.secrets.kdbx_password.as_ref(),
                settings.secrets.kdbx_key_file.clone(),
                groups,
                settings.secrets.clone(),
            );
        }

        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let window_clone = window.clone();
        dialog.run(move |result| {
            if let Some(dialog_result) = result {
                let updated_conn = dialog_result.connection;
                let password = dialog_result.password;

                if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                    // Clone values needed for password saving
                    let conn_name = updated_conn.name.clone();
                    let conn_host = updated_conn.host.clone();
                    let conn_username = updated_conn.username.clone();
                    let password_source = updated_conn.password_source.clone();
                    let protocol = updated_conn.protocol;

                    match state_mut.update_connection(id, updated_conn) {
                        Ok(()) => {
                            // Save password to vault if needed
                            if password_source == PasswordSource::Vault
                                && let Some(pwd) = password
                            {
                                let settings = state_mut.settings().clone();
                                let groups: Vec<_> =
                                    state_mut.list_groups().into_iter().cloned().collect();
                                let conn_for_path = state_mut.get_connection(id).cloned();
                                let username = conn_username.unwrap_or_default();

                                crate::state::save_password_to_vault(
                                    &settings,
                                    &groups,
                                    conn_for_path.as_ref(),
                                    &conn_name,
                                    &conn_host,
                                    protocol,
                                    &username,
                                    &pwd,
                                    id,
                                );
                            }

                            drop(state_mut);
                            // Defer sidebar reload to prevent UI freeze
                            let state = state_clone.clone();
                            let sidebar = sidebar_clone.clone();
                            glib::idle_add_local_once(move || {
                                MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                            });
                        }
                        Err(e) => {
                            alert::show_error(
                                &window_clone,
                                &i18n("Error Updating Connection"),
                                &e,
                            );
                        }
                    }
                }
            }
        });
    }
}

/// Renames the selected connection or group with a simple inline dialog
pub fn rename_selected_item(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    // Get selected item
    let Some(conn_item) = sidebar.get_selected_item() else {
        return;
    };

    let id_str = conn_item.id();
    let Ok(id) = Uuid::parse_str(&id_str) else {
        return;
    };

    let is_group = conn_item.is_group();
    let current_name = conn_item.name();

    // Create rename dialog with Adwaita
    let rename_dialog = adw::Dialog::builder()
        .title(if is_group {
            i18n("Rename Group")
        } else {
            i18n("Rename Connection")
        })
        .content_width(450)
        .build();

    let header = adw::HeaderBar::new();
    let save_btn = gtk4::Button::builder()
        .label(i18n("Rename"))
        .css_classes(["suggested-action"])
        .build();
    header.pack_end(&save_btn);

    let content = gtk4::Box::new(Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Name entry using PreferencesGroup with EntryRow
    let name_group = adw::PreferencesGroup::new();
    let name_row = adw::EntryRow::builder()
        .title(i18n("Name"))
        .text(&current_name)
        .build();
    name_group.add(&name_row);
    content.append(&name_group);

    // Use ToolbarView for proper adw::Window layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    rename_dialog.set_child(Some(&toolbar_view));

    // Save button
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let window_clone = rename_dialog.clone();
    let name_row_clone = name_row.clone();
    save_btn.connect_clicked(move |_| {
        let new_name = name_row_clone.text().trim().to_string();
        if new_name.is_empty() {
            alert::show_validation_error(&window_clone, &i18n("Name cannot be empty"));
            return;
        }

        if new_name == current_name {
            window_clone.close();
            return;
        }

        if is_group {
            // Rename group
            let state_ref = state_clone.borrow();
            if state_ref.group_exists_by_name(&new_name) {
                drop(state_ref);
                alert::show_validation_error(
                    &window_clone,
                    &i18n_f("Group with name '{}' already exists", &[&new_name]),
                );
                return;
            }
            drop(state_ref);

            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                if let Some(existing) = state_mut.get_group(id).cloned() {
                    let old_name_val = existing.name.clone();
                    let mut updated = existing;
                    updated.name = new_name.clone();

                    // Capture old groups snapshot before update for vault migration
                    let old_groups_snapshot: Vec<rustconn_core::models::ConnectionGroup> =
                        if old_name_val == new_name {
                            Vec::new()
                        } else {
                            state_mut.list_groups().into_iter().cloned().collect()
                        };

                    if let Err(e) = state_mut.connection_manager().update_group(id, updated) {
                        alert::show_error(&window_clone, &i18n("Error"), &format!("{e}"));
                        return;
                    }

                    // Migrate vault entries if name changed (KeePass paths affected)
                    if old_name_val != new_name {
                        let new_groups: Vec<_> =
                            state_mut.list_groups().into_iter().cloned().collect();
                        let connections: Vec<_> =
                            state_mut.list_connections().into_iter().cloned().collect();
                        let settings = state_mut.settings().clone();
                        crate::state::migrate_vault_entries_on_group_change(
                            &settings,
                            &old_groups_snapshot,
                            &new_groups,
                            &connections,
                            id,
                        );
                    }
                }
                drop(state_mut);
                // Defer sidebar reload to prevent UI freeze
                let state = state_clone.clone();
                let sidebar = sidebar_clone.clone();
                let window = window_clone.clone();
                glib::idle_add_local_once(move || {
                    MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                    window.close();
                });
            }
        } else {
            // Rename connection
            if let Ok(mut state_mut) = state_clone.try_borrow_mut()
                && let Some(existing) = state_mut.get_connection(id).cloned()
            {
                let old_name = existing.name.clone();
                let mut updated = existing.clone();
                updated.name = new_name.clone();

                // Get data needed for credential rename before updating
                let password_source = updated.password_source.clone();
                let protocol = updated.protocol_config.protocol_type();
                let groups: Vec<rustconn_core::models::ConnectionGroup> =
                    state_mut.list_groups().iter().cloned().cloned().collect();
                let settings = state_mut.settings().clone();

                match state_mut.update_connection(id, updated.clone()) {
                    Ok(()) => {
                        drop(state_mut);

                        // Rename credentials in secret backend if needed
                        match password_source {
                            rustconn_core::models::PasswordSource::Vault => {
                                // Vault — rename in configured backend
                                let updated_conn = updated;
                                let groups_clone = groups;
                                let settings_clone = settings;
                                let protocol_str = protocol.as_str().to_lowercase();

                                crate::utils::spawn_blocking_with_callback(
                                    move || {
                                        crate::state::rename_vault_credential(
                                            &settings_clone,
                                            &groups_clone,
                                            &updated_conn,
                                            &old_name,
                                            &protocol_str,
                                        )
                                    },
                                    |result: Result<(), String>| {
                                        if let Err(e) = result {
                                            tracing::warn!("Failed to rename vault entry: {}", e);
                                        }
                                    },
                                );
                            }
                            rustconn_core::models::PasswordSource::Variable(_)
                            | rustconn_core::models::PasswordSource::Script(_)
                            | rustconn_core::models::PasswordSource::Prompt
                            | rustconn_core::models::PasswordSource::Inherit
                            | rustconn_core::models::PasswordSource::None => {
                                // No credentials stored
                            }
                        }

                        // Defer sidebar reload to prevent UI freeze
                        let state = state_clone.clone();
                        let sidebar = sidebar_clone.clone();
                        let window = window_clone.clone();
                        glib::idle_add_local_once(move || {
                            MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
                            window.close();
                        });
                    }
                    Err(e) => {
                        alert::show_error(&window_clone, &i18n("Error"), &e);
                    }
                }
            }
        }
    });

    // Enter key triggers save
    let save_btn_clone = save_btn.clone();
    name_row.connect_entry_activated(move |_| {
        save_btn_clone.emit_clicked();
    });

    rename_dialog.present(Some(window));
    name_row.grab_focus();
}

/// Shows the quick connect dialog with protocol selection and template support
pub fn show_quick_connect_dialog(
    window: &gtk4::Window,
    notebook: SharedNotebook,
    split_view: SharedSplitView,
    sidebar: SharedSidebar,
    state: &SharedAppState,
    history: super::types::SharedQuickConnectHistory,
) {
    show_quick_connect_dialog_with_state(
        window,
        notebook,
        split_view,
        sidebar,
        Some(state),
        history,
    );
}

/// Parameters for a quick connect session
struct QuickConnectParams {
    host: String,
    port: u16,
    username: Option<String>,
    password: Option<secrecy::SecretString>,
}

/// Starts a quick Telnet connection
fn start_quick_telnet(
    notebook: &SharedNotebook,
    params: &QuickConnectParams,
    terminal_settings: &rustconn_core::config::TerminalSettings,
) {
    let session_id = notebook.create_terminal_tab_with_settings(
        Uuid::nil(),
        &format!("Quick: {}", params.host),
        "telnet",
        None,
        terminal_settings,
        None,
        &[],
    );
    notebook.spawn_telnet(
        session_id,
        &params.host,
        params.port,
        &[],
        rustconn_core::models::TelnetBackspaceSends::Automatic,
        rustconn_core::models::TelnetDeleteSends::Automatic,
    );
}

/// Starts a quick SSH connection
fn start_quick_ssh(
    notebook: &SharedNotebook,
    params: &QuickConnectParams,
    terminal_settings: &rustconn_core::config::TerminalSettings,
) {
    let session_id = notebook.create_terminal_tab_with_settings(
        Uuid::nil(),
        &format!("Quick: {}", params.host),
        "ssh",
        None,
        terminal_settings,
        None,
        &[],
    );
    notebook.spawn_ssh(
        session_id,
        &params.host,
        params.port,
        params.username.as_deref(),
        None,
        &[],
        false,
        None,
        None,
    );
}

/// Starts a quick RDP connection
fn start_quick_rdp(
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    params: &QuickConnectParams,
) {
    let embedded_widget = EmbeddedRdpWidget::new();

    let mut embedded_config = EmbeddedRdpConfig::new(&params.host)
        .with_port(params.port)
        .with_resolution(1920, 1080)
        .with_clipboard(true);

    if let Some(ref user) = params.username {
        embedded_config = embedded_config.with_username(user);
    }

    if let Some(ref pass) = params.password {
        use secrecy::ExposeSecret;
        embedded_config = embedded_config.with_password(pass.expose_secret());
    }

    let embedded_widget = Rc::new(embedded_widget);
    let session_id = Uuid::new_v4();

    // Connect state change callback
    let notebook_for_state = notebook.clone();
    let sidebar_for_state = sidebar.clone();
    let connection_id = Uuid::nil();
    embedded_widget.connect_state_changed(move |rdp_state| match rdp_state {
        crate::embedded_rdp::RdpConnectionState::Disconnected => {
            notebook_for_state.stop_recording(session_id);
            notebook_for_state.mark_tab_disconnected(session_id);
            sidebar_for_state.decrement_session_count(&connection_id.to_string(), false);
        }
        crate::embedded_rdp::RdpConnectionState::Connected => {
            notebook_for_state.mark_tab_connected(session_id);
        }
        _ => {}
    });

    // Connect reconnect callback
    let widget_for_reconnect = embedded_widget.clone();
    embedded_widget.connect_reconnect(move || {
        if let Err(e) = widget_for_reconnect.reconnect() {
            tracing::error!("RDP reconnect failed: {}", e);
        }
    });

    // Start connection
    if let Err(e) = embedded_widget.connect(&embedded_config) {
        tracing::error!("RDP connection failed for '{}': {}", params.host, e);
    }

    notebook.add_embedded_rdp_tab(
        session_id,
        Uuid::nil(),
        &format!("Quick: {}", params.host),
        embedded_widget,
    );

    // Show notebook for RDP session
    split_view.widget().set_visible(false);
    split_view.widget().set_vexpand(false);
    notebook.widget().set_vexpand(true);
    notebook.show_tab_view_content();
}

/// Starts a quick VNC connection
fn start_quick_vnc(
    notebook: &SharedNotebook,
    split_view: &SharedSplitView,
    sidebar: &SharedSidebar,
    params: &QuickConnectParams,
) {
    let session_id = notebook.create_vnc_session_tab_with_host(
        Uuid::nil(),
        &format!("Quick: {}", params.host),
        &params.host,
    );

    // Get the VNC widget and initiate connection
    if let Some(vnc_widget) = notebook.get_vnc_widget(session_id) {
        let vnc_config = rustconn_core::models::VncConfig::default();

        // Connect state change callback
        let notebook_for_state = notebook.clone();
        let sidebar_for_state = sidebar.clone();
        let connection_id = Uuid::nil();
        vnc_widget.connect_state_changed(move |vnc_state| {
            if vnc_state == crate::session::SessionState::Disconnected {
                notebook_for_state.stop_recording(session_id);
                notebook_for_state.mark_tab_disconnected(session_id);
                sidebar_for_state.decrement_session_count(&connection_id.to_string(), false);
            } else if vnc_state == crate::session::SessionState::Connected {
                notebook_for_state.mark_tab_connected(session_id);
            }
        });

        // Connect reconnect callback
        let widget_for_reconnect = vnc_widget.clone();
        vnc_widget.connect_reconnect(move || {
            if let Err(e) = widget_for_reconnect.reconnect() {
                tracing::error!("VNC reconnect failed: {}", e);
            }
        });

        // Initiate connection with password if provided
        let pw_exposed = params.password.as_ref().map(|s| {
            use secrecy::ExposeSecret;
            zeroize::Zeroizing::new(s.expose_secret().to_string())
        });
        if let Err(e) = vnc_widget.connect_with_config(
            &params.host,
            params.port,
            pw_exposed.as_ref().map(|z| z.as_str()),
            &vnc_config,
        ) {
            tracing::error!("Failed to connect VNC session '{}': {}", params.host, e);
        }
    }

    // Show notebook for VNC session
    split_view.widget().set_visible(false);
    split_view.widget().set_vexpand(false);
    notebook.widget().set_vexpand(true);
    notebook.show_tab_view_content();
}

/// Shows the quick connect dialog with optional state for template access
pub fn show_quick_connect_dialog_with_state(
    window: &gtk4::Window,
    notebook: SharedNotebook,
    split_view: SharedSplitView,
    sidebar: SharedSidebar,
    state: Option<&SharedAppState>,
    history: super::types::SharedQuickConnectHistory,
) {
    // Collect templates if state is available
    let templates: Vec<rustconn_core::models::ConnectionTemplate> = state
        .map(|s| {
            let state_ref = s.borrow();
            state_ref.get_all_templates()
        })
        .unwrap_or_default();

    // Create a quick connect window with Adwaita
    let quick_dialog = adw::Dialog::builder()
        .title(i18n("Quick Connect"))
        .content_width(450)
        .build();

    // Header bar with Connect icon and standard window buttons (GNOME HIG)
    let header = adw::HeaderBar::new();
    let connect_btn = Button::from_icon_name("go-next-symbolic");
    connect_btn.set_tooltip_text(Some(&i18n("Connect")));
    connect_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Connect"))]);
    connect_btn.add_css_class("suggested-action");
    header.pack_start(&connect_btn);

    // Main content
    let content = gtk4::Box::new(Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Info label
    let info_label = Label::new(Some(&i18n("⚠ This connection will not be saved")));
    info_label.add_css_class("dim-label");
    content.append(&info_label);

    // Connection settings group
    let settings_group = adw::PreferencesGroup::new();

    // Template row (if templates available)
    let template_dropdown: Option<gtk4::DropDown> = if templates.is_empty() {
        None
    } else {
        let mut template_names: Vec<String> = vec![i18n("(None)")];
        template_names.extend(templates.iter().map(|t| t.name.clone()));
        let template_strings: Vec<&str> = template_names.iter().map(String::as_str).collect();
        let template_list = gtk4::StringList::new(&template_strings);

        let dropdown = gtk4::DropDown::builder()
            .model(&template_list)
            .valign(gtk4::Align::Center)
            .build();
        dropdown.set_selected(0);

        let template_row = adw::ActionRow::builder().title(i18n("Template")).build();
        template_row.add_suffix(&dropdown);
        settings_group.add(&template_row);

        Some(dropdown)
    };

    // Protocol dropdown
    let protocol_list = gtk4::StringList::new(&["SSH", "RDP", "VNC", "Telnet"]);
    let protocol_dropdown = gtk4::DropDown::builder()
        .model(&protocol_list)
        .valign(gtk4::Align::Center)
        .build();
    protocol_dropdown.set_selected(0);
    let protocol_row = adw::ActionRow::builder().title(i18n("Protocol")).build();
    protocol_row.add_suffix(&protocol_dropdown);
    settings_group.add(&protocol_row);

    // Host entry
    let host_entry = gtk4::Entry::builder()
        .placeholder_text(i18n("hostname or IP"))
        .valign(gtk4::Align::Center)
        .hexpand(true)
        .build();
    let host_row = adw::ActionRow::builder().title(i18n("Host")).build();
    host_row.add_suffix(&host_entry);
    settings_group.add(&host_row);

    // Port spin
    let port_adj = gtk4::Adjustment::new(22.0, 1.0, 65535.0, 1.0, 10.0, 0.0);
    let port_spin = gtk4::SpinButton::builder()
        .adjustment(&port_adj)
        .climb_rate(1.0)
        .digits(0)
        .valign(gtk4::Align::Center)
        .build();
    let port_row = adw::ActionRow::builder().title(i18n("Port")).build();
    port_row.add_suffix(&port_spin);
    settings_group.add(&port_row);

    // Username entry
    let user_entry = gtk4::Entry::builder()
        .placeholder_text(i18n("(optional)"))
        .valign(gtk4::Align::Center)
        .hexpand(true)
        .build();
    let user_row = adw::ActionRow::builder().title(i18n("Username")).build();
    user_row.add_suffix(&user_entry);
    settings_group.add(&user_row);

    // Password entry
    let password_entry = gtk4::PasswordEntry::builder()
        .show_peek_icon(true)
        .placeholder_text(i18n("(optional)"))
        .valign(gtk4::Align::Center)
        .hexpand(true)
        .build();
    let password_row = adw::ActionRow::builder().title(i18n("Password")).build();
    password_row.add_suffix(&password_entry);
    settings_group.add(&password_row);

    content.append(&settings_group);

    // --- History section (runtime only, shown if history is non-empty) ---
    let history_group = adw::PreferencesGroup::builder()
        .title(i18n("Recent"))
        .build();
    let history_listbox = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::Single)
        .css_classes(vec!["boxed-list".to_string()])
        .build();
    history_group.add(&history_listbox);
    content.append(&history_group);

    // Populate history list
    {
        let hist = history.borrow();
        if hist.is_empty() {
            history_group.set_visible(false);
        } else {
            for entry in hist.iter() {
                let row = adw::ActionRow::builder()
                    .title(glib::markup_escape_text(&entry.display_string()))
                    .activatable(true)
                    .build();
                row.add_prefix(&gtk4::Image::from_icon_name("go-jump-symbolic"));
                history_listbox.append(&row);
            }
        }
    }

    // Connect history row activation to fill fields
    let history_for_activate = history.clone();
    let host_entry_for_hist = host_entry.clone();
    let port_spin_for_hist = port_spin.clone();
    let user_entry_for_hist = user_entry.clone();
    let protocol_dd_for_hist = protocol_dropdown.clone();
    history_listbox.connect_row_activated(move |_, row| {
        let index = row.index();
        if index < 0 {
            return;
        }
        let hist = history_for_activate.borrow();
        if let Some(entry) = hist.get(index as usize) {
            protocol_dd_for_hist.set_selected(entry.protocol_index);
            host_entry_for_hist.set_text(&entry.host);
            port_spin_for_hist.set_value(f64::from(entry.port));
            if let Some(ref user) = entry.username {
                user_entry_for_hist.set_text(user);
            } else {
                user_entry_for_hist.set_text("");
            }
        }
    });

    // Filter history when host entry text changes
    let history_for_filter = history.clone();
    let history_listbox_for_filter = history_listbox.clone();
    let history_group_for_filter = history_group.clone();
    host_entry.connect_changed(move |entry| {
        let filter_text = entry.text().to_string().to_lowercase();
        let hist = history_for_filter.borrow();
        if hist.is_empty() {
            history_group_for_filter.set_visible(false);
            return;
        }
        let mut any_visible = false;
        let mut idx = 0i32;
        while let Some(row) = history_listbox_for_filter.row_at_index(idx) {
            if let Some(entry) = hist.get(idx as usize) {
                let visible = filter_text.is_empty()
                    || entry.host.to_lowercase().contains(&filter_text)
                    || entry
                        .username
                        .as_ref()
                        .is_some_and(|u| u.to_lowercase().contains(&filter_text));
                row.set_visible(visible);
                if visible {
                    any_visible = true;
                }
            }
            idx += 1;
        }
        history_group_for_filter.set_visible(any_visible);
    });

    // Use ToolbarView for proper adw::Window layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    quick_dialog.set_child(Some(&toolbar_view));

    // Track if port was manually changed
    let port_manually_changed = Rc::new(RefCell::new(false));

    // Connect port spin value-changed to track manual changes
    let port_manually_changed_clone = port_manually_changed.clone();
    port_spin.connect_value_changed(move |_| {
        *port_manually_changed_clone.borrow_mut() = true;
    });

    // Connect template selection to fill fields
    if let Some(ref template_dd) = template_dropdown {
        let templates_clone = templates.clone();
        let protocol_dd = protocol_dropdown.clone();
        let host_entry_clone = host_entry.clone();
        let port_spin_clone = port_spin.clone();
        let user_entry_clone = user_entry.clone();
        let port_manually_changed_for_template = Rc::new(RefCell::new(false));

        template_dd.connect_selected_notify(move |dropdown| {
            let selected = dropdown.selected();
            if selected == 0 {
                // "None" selected - clear fields
                return;
            }

            // Get template (index - 1 because of "None" option)
            if let Some(template) = templates_clone.get(selected as usize - 1) {
                // Set protocol
                let protocol_idx = match template.protocol {
                    rustconn_core::models::ProtocolType::Ssh => 0,
                    rustconn_core::models::ProtocolType::Rdp => 1,
                    rustconn_core::models::ProtocolType::Vnc => 2,
                    rustconn_core::models::ProtocolType::Telnet => 3,
                    _ => 0,
                };
                protocol_dd.set_selected(protocol_idx);

                // Set host if not empty
                if !template.host.is_empty() {
                    host_entry_clone.set_text(&template.host);
                }

                // Set port
                *port_manually_changed_for_template.borrow_mut() = false;
                port_spin_clone.set_value(f64::from(template.port));

                // Set username if present
                if let Some(username) = &template.username {
                    user_entry_clone.set_text(username);
                }
            }
        });
    }

    // Connect protocol change to port update
    let port_spin_clone = port_spin.clone();
    let port_manually_changed_clone = port_manually_changed;
    protocol_dropdown.connect_selected_notify(move |dropdown| {
        // Only update port if it wasn't manually changed
        if !*port_manually_changed_clone.borrow() {
            let default_port = match dropdown.selected() {
                1 => 3389.0, // RDP
                2 => 5900.0, // VNC
                3 => 23.0,   // Telnet
                _ => 22.0,   // SSH (0) and any other value
            };
            port_spin_clone.set_value(default_port);
        }
        // Reset the flag after protocol change so next protocol change updates port
        *port_manually_changed_clone.borrow_mut() = false;
    });

    // Connect quick connect button
    let window_clone = quick_dialog.clone();
    let host_clone = host_entry;
    let port_clone = port_spin;
    let user_clone = user_entry;
    let password_clone = password_entry;
    let protocol_clone = protocol_dropdown;
    // Clone state for use in closure
    let state_for_connect = state.cloned();
    let history_for_connect = history;
    connect_btn.connect_clicked(move |_| {
        let host = host_clone.text().to_string();
        if host.trim().is_empty() {
            return;
        }

        // Get terminal settings from state if available
        let terminal_settings = state_for_connect
            .as_ref()
            .and_then(|s| s.try_borrow().ok())
            .map(|s| s.settings().terminal.clone())
            .unwrap_or_default();

        #[expect(
            clippy::cast_sign_loss,
            reason = "value is non-negative by construction in this code path"
        )]
        let port = port_clone.value() as u16;
        let username = {
            let text = user_clone.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(text.to_string())
            }
        };
        let password = {
            let text = password_clone.text();
            if text.trim().is_empty() {
                None
            } else {
                Some(secrecy::SecretString::from(text.to_string()))
            }
        };

        // Save to runtime history (and persist to settings if state is available)
        let protocol_idx = protocol_clone.selected();
        let history_entry = super::types::QuickConnectHistoryEntry::new(
            protocol_idx,
            host.clone(),
            port,
            username.clone(),
        );
        super::types::add_to_quick_connect_history(&history_for_connect, history_entry);
        if let Some(state) = state_for_connect.as_ref() {
            super::types::persist_quick_connect_history(&history_for_connect, state);
        }

        let params = QuickConnectParams {
            host,
            port,
            username,
            password,
        };

        match protocol_idx {
            0 => start_quick_ssh(&notebook, &params, &terminal_settings),
            1 => start_quick_rdp(&notebook, &split_view, &sidebar, &params),
            2 => start_quick_vnc(&notebook, &split_view, &sidebar, &params),
            3 => start_quick_telnet(&notebook, &params, &terminal_settings),
            _ => start_quick_ssh(&notebook, &params, &terminal_settings),
        }

        window_clone.close();
    });

    quick_dialog.present(Some(window));
}
