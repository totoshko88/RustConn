//! Edit dialogs for main window
//!
//! This module contains functions for editing connections and groups,
//! showing connection details, and quick connect dialog.

use crate::dialogs::ConnectionDialog;
use crate::embedded_rdp::{EmbeddedRdpWidget, RdpConfig as EmbeddedRdpConfig};
use crate::sidebar::ConnectionSidebar;
use crate::split_view::SplitTerminalView;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use crate::window::MainWindow;
use gtk4::prelude::*;
use gtk4::{ApplicationWindow, Button, Label, Orientation};
use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Type alias for shared notebook reference
pub type SharedNotebook = Rc<TerminalNotebook>;

/// Type alias for shared split view reference
pub type SharedSplitView = Rc<SplitTerminalView>;

/// Edits the selected connection or group
pub fn edit_selected_connection(
    window: &ApplicationWindow,
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

        let dialog = ConnectionDialog::new(Some(&window.clone().upcast()));
        dialog.setup_key_file_chooser(Some(&window.clone().upcast()));

        // Set available groups
        {
            let state_ref = state.borrow();
            let groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
            dialog.set_groups(&groups);
        }

        dialog.set_connection(&conn);

        // Set KeePass enabled state from settings
        {
            let state_ref = state.borrow();
            let keepass_enabled = state_ref.settings().secrets.kdbx_enabled;
            dialog.set_keepass_enabled(keepass_enabled);
        }

        // Connect save to KeePass callback
        let window_for_keepass = window.clone();
        let state_for_save = state.clone();
        let conn_name = conn.name.clone();
        let conn_host = conn.host.clone();
        dialog.connect_save_to_keepass(move |name, host, username, password, protocol| {
            handle_save_to_keepass(
                &window_for_keepass,
                &state_for_save,
                &conn_name,
                &conn_host,
                name,
                host,
                username,
                password,
                protocol,
            );
        });

        // Connect load from KeePass callback
        let state_for_load = state.clone();
        dialog.connect_load_from_keepass(move |name, host, _protocol| {
            handle_load_from_keepass(&state_for_load, name, host)
        });

        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        let window_clone = window.clone();
        dialog.run(move |result| {
            if let Some(updated_conn) = result {
                if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                    match state_mut.update_connection(id, updated_conn) {
                        Ok(()) => {
                            drop(state_mut);
                            // Preserve tree state when editing connections
                            MainWindow::reload_sidebar_preserving_state(
                                &state_clone,
                                &sidebar_clone,
                            );
                        }
                        Err(e) => {
                            let alert = gtk4::AlertDialog::builder()
                                .message("Error Updating Connection")
                                .detail(&e)
                                .modal(true)
                                .build();
                            alert.show(Some(&window_clone));
                        }
                    }
                }
            }
        });
    }
}

/// Handles saving password to KeePass
#[allow(clippy::too_many_arguments)]
fn handle_save_to_keepass(
    window: &ApplicationWindow,
    state: &SharedAppState,
    conn_name: &str,
    conn_host: &str,
    name: &str,
    host: &str,
    username: &str,
    password: &str,
    protocol: &str,
) {
    use secrecy::ExposeSecret;

    let state_ref = state.borrow();
    let settings = state_ref.settings();

    if !settings.secrets.kdbx_enabled {
        let alert = gtk4::AlertDialog::builder()
            .message("KeePass Not Enabled")
            .detail("Please enable KeePass integration in Settings first.")
            .modal(true)
            .build();
        alert.show(Some(window));
        return;
    }

    let Some(kdbx_path) = settings.secrets.kdbx_path.as_ref() else {
        let alert = gtk4::AlertDialog::builder()
            .message("KeePass Database Not Configured")
            .detail("Please select a KeePass database file in Settings.")
            .modal(true)
            .build();
        alert.show(Some(window));
        return;
    };

    // Use connection name/host for lookup key
    let lookup_key = if !name.trim().is_empty() {
        name.to_string()
    } else if !host.trim().is_empty() {
        host.to_string()
    } else if !conn_name.is_empty() {
        conn_name.to_string()
    } else {
        conn_host.to_string()
    };

    // Get credentials - password and key file can be used together
    let db_password = settings
        .secrets
        .kdbx_password
        .as_ref()
        .map(|p| p.expose_secret());

    // Key file is optional additional authentication
    let key_file = settings.secrets.kdbx_key_file.as_deref();

    // Check if we have at least one credential
    if db_password.is_none() && key_file.is_none() {
        let alert = gtk4::AlertDialog::builder()
            .message("KeePass Credentials Required")
            .detail("Please enter the database password or select a key file in Settings.")
            .modal(true)
            .build();
        alert.show(Some(window));
        return;
    }

    // Use protocol from callback parameter
    let url = format!(
        "{}://{}",
        protocol,
        if host.is_empty() { conn_host } else { host }
    );

    match rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
        kdbx_path,
        db_password,
        key_file,
        &lookup_key,
        username,
        password,
        Some(&url),
    ) {
        Ok(()) => {
            let alert = gtk4::AlertDialog::builder()
                .message("Password Saved")
                .detail(format!("Password for '{lookup_key}' saved to KeePass."))
                .modal(true)
                .build();
            alert.show(Some(window));
        }
        Err(e) => {
            let alert = gtk4::AlertDialog::builder()
                .message("Failed to Save Password")
                .detail(format!("Error: {e}"))
                .modal(true)
                .build();
            alert.show(Some(window));
        }
    }
}

/// Handles loading password from KeePass
fn handle_load_from_keepass(state: &SharedAppState, name: &str, host: &str) -> Option<String> {
    use secrecy::ExposeSecret;

    let state_ref = state.borrow();
    let settings = state_ref.settings();

    if !settings.secrets.kdbx_enabled {
        return None;
    }

    let kdbx_path = settings.secrets.kdbx_path.as_ref()?;

    let lookup_key = if name.trim().is_empty() {
        host.to_string()
    } else {
        name.to_string()
    };

    // Get credentials - password and key file can be used together
    let db_password = settings
        .secrets
        .kdbx_password
        .as_ref()
        .map(|p| p.expose_secret());

    // Key file is optional additional authentication
    let key_file = settings.secrets.kdbx_key_file.as_deref();

    match rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
        kdbx_path,
        db_password,
        key_file,
        &lookup_key,
    ) {
        Ok(password) => password,
        Err(e) => {
            eprintln!("Failed to load password from KeePass: {e}");
            None
        }
    }
}

/// Renames the selected connection or group with a simple inline dialog
pub fn rename_selected_item(
    window: &ApplicationWindow,
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

    // Create rename dialog
    let rename_window = gtk4::Window::builder()
        .title(if is_group {
            "Rename Group"
        } else {
            "Rename Connection"
        })
        .transient_for(window)
        .modal(true)
        .default_width(750)
        .build();

    let header = gtk4::HeaderBar::new();
    let cancel_btn = gtk4::Button::builder().label("Cancel").build();
    let save_btn = gtk4::Button::builder()
        .label("Rename")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&save_btn);
    rename_window.set_titlebar(Some(&header));

    let content = gtk4::Box::new(Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let label = Label::new(Some("New name:"));
    label.set_halign(gtk4::Align::Start);
    content.append(&label);

    let entry = gtk4::Entry::new();
    entry.set_text(&current_name);
    entry.select_region(0, -1); // Select all text
    content.append(&entry);

    rename_window.set_child(Some(&content));

    // Cancel button
    let window_clone = rename_window.clone();
    cancel_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    // Save button
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let window_clone = rename_window.clone();
    let entry_clone = entry.clone();
    save_btn.connect_clicked(move |_| {
        let new_name = entry_clone.text().trim().to_string();
        if new_name.is_empty() {
            let alert = gtk4::AlertDialog::builder()
                .message("Validation Error")
                .detail("Name cannot be empty")
                .modal(true)
                .build();
            alert.show(Some(&window_clone));
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
                let alert = gtk4::AlertDialog::builder()
                    .message("Validation Error")
                    .detail(format!("Group with name '{new_name}' already exists"))
                    .modal(true)
                    .build();
                alert.show(Some(&window_clone));
                return;
            }
            drop(state_ref);

            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                if let Some(existing) = state_mut.get_group(id).cloned() {
                    let mut updated = existing;
                    updated.name = new_name;
                    if let Err(e) = state_mut.connection_manager().update_group(id, updated) {
                        let alert = gtk4::AlertDialog::builder()
                            .message("Error")
                            .detail(format!("{e}"))
                            .modal(true)
                            .build();
                        alert.show(Some(&window_clone));
                        return;
                    }
                }
                drop(state_mut);
                MainWindow::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
                window_clone.close();
            }
        } else {
            // Rename connection
            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                if let Some(existing) = state_mut.get_connection(id).cloned() {
                    let mut updated = existing;
                    updated.name = new_name;
                    match state_mut.update_connection(id, updated) {
                        Ok(()) => {
                            drop(state_mut);
                            MainWindow::reload_sidebar_preserving_state(
                                &state_clone,
                                &sidebar_clone,
                            );
                            window_clone.close();
                        }
                        Err(e) => {
                            let alert = gtk4::AlertDialog::builder()
                                .message("Error")
                                .detail(&e)
                                .modal(true)
                                .build();
                            alert.show(Some(&window_clone));
                        }
                    }
                }
            }
        }
    });

    // Enter key triggers save
    let save_btn_clone = save_btn.clone();
    entry.connect_activate(move |_| {
        save_btn_clone.emit_clicked();
    });

    rename_window.present();
    entry.grab_focus();
}

/// Shows dialog to edit a group name
// SharedAppState is Rc<RefCell<...>> - cheap to clone and needed for closure ownership
#[allow(clippy::needless_pass_by_value)]
pub fn show_edit_group_dialog(
    window: &ApplicationWindow,
    state: SharedAppState,
    sidebar: SharedSidebar,
    group_id: Uuid,
) {
    let state_ref = state.borrow();
    let Some(group) = state_ref.get_group(group_id).cloned() else {
        return;
    };
    drop(state_ref);

    let entry = gtk4::Entry::new();
    entry.set_text(&group.name);

    let group_window = gtk4::Window::builder()
        .title("Edit Group")
        .transient_for(window)
        .modal(true)
        .default_width(750)
        .build();

    let header = gtk4::HeaderBar::new();
    let cancel_btn = gtk4::Button::builder().label("Cancel").build();
    let save_btn = gtk4::Button::builder()
        .label("Save")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&save_btn);
    group_window.set_titlebar(Some(&header));

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    let label = Label::new(Some("Group name:"));
    content.append(&label);
    content.append(&entry);
    group_window.set_child(Some(&content));

    let window_clone = group_window.clone();
    cancel_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    let state_clone = state.clone();
    let sidebar_clone = sidebar;
    let window_clone = group_window.clone();
    let entry_clone = entry;
    let old_name = group.name;
    save_btn.connect_clicked(move |_| {
        let new_name = entry_clone.text().to_string();
        if new_name.trim().is_empty() {
            let alert = gtk4::AlertDialog::builder()
                .message("Validation Error")
                .detail("Group name cannot be empty")
                .modal(true)
                .build();
            alert.show(Some(&window_clone));
            return;
        }

        // Check for duplicate name (but allow keeping same name)
        if new_name != old_name {
            let state_ref = state_clone.borrow();
            if state_ref.group_exists_by_name(&new_name) {
                drop(state_ref);
                let alert = gtk4::AlertDialog::builder()
                    .message("Validation Error")
                    .detail(format!("Group with name '{new_name}' already exists"))
                    .modal(true)
                    .build();
                alert.show(Some(&window_clone));
                return;
            }
            drop(state_ref);
        }

        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
            if let Some(existing) = state_mut.get_group(group_id).cloned() {
                let mut updated = existing;
                updated.name = new_name;
                if let Err(e) = state_mut
                    .connection_manager()
                    .update_group(group_id, updated)
                {
                    let alert = gtk4::AlertDialog::builder()
                        .message("Error")
                        .detail(format!("{e}"))
                        .modal(true)
                        .build();
                    alert.show(Some(&window_clone));
                    return;
                }
            }
            drop(state_mut);
            // Preserve tree state when editing groups
            MainWindow::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);
            window_clone.close();
        }
    });

    group_window.present();
}

/// Shows the quick connect dialog with protocol selection and template support
#[allow(clippy::too_many_lines)]
pub fn show_quick_connect_dialog(
    window: &ApplicationWindow,
    notebook: SharedNotebook,
    split_view: SharedSplitView,
    sidebar: SharedSidebar,
) {
    show_quick_connect_dialog_with_state(window, notebook, split_view, sidebar, None);
}

/// Shows the quick connect dialog with optional state for template access
#[allow(clippy::too_many_lines)]
pub fn show_quick_connect_dialog_with_state(
    window: &ApplicationWindow,
    notebook: SharedNotebook,
    split_view: SharedSplitView,
    sidebar: SharedSidebar,
    state: Option<&SharedAppState>,
) {
    // Collect templates if state is available
    let templates: Vec<rustconn_core::models::ConnectionTemplate> = state
        .map(|s| {
            let state_ref = s.borrow();
            state_ref.load_templates().unwrap_or_default()
        })
        .unwrap_or_default();

    // Create a quick connect window
    let quick_window = gtk4::Window::builder()
        .title("Quick Connect")
        .transient_for(window)
        .modal(true)
        .default_width(750)
        .default_height(550)
        .build();

    // Create header bar with Close/Connect buttons (GNOME HIG)
    let header = gtk4::HeaderBar::new();
    header.set_show_title_buttons(false);
    let close_btn = Button::builder().label("Close").build();
    let connect_btn = Button::builder()
        .label("Connect")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&close_btn);
    header.pack_end(&connect_btn);
    quick_window.set_titlebar(Some(&header));

    // Close button handler
    let window_clone = quick_window.clone();
    close_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    // Create content
    let content = gtk4::Box::new(Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Info label
    let info_label = Label::new(Some("⚠ This connection will not be saved"));
    info_label.add_css_class("dim-label");
    content.append(&info_label);

    // Basic fields
    let grid = gtk4::Grid::builder()
        .row_spacing(8)
        .column_spacing(12)
        .build();

    // Template dropdown (if templates available)
    let template_dropdown: Option<gtk4::DropDown> = if templates.is_empty() {
        None
    } else {
        let template_label = Label::builder()
            .label("Template:")
            .halign(gtk4::Align::End)
            .build();

        // Build template names list with "None" option
        let mut template_names: Vec<String> = vec!["(None)".to_string()];
        template_names.extend(templates.iter().map(|t| t.name.clone()));
        let template_strings: Vec<&str> = template_names.iter().map(String::as_str).collect();
        let template_list = gtk4::StringList::new(&template_strings);

        let dropdown = gtk4::DropDown::builder().model(&template_list).build();
        dropdown.set_selected(0); // Default to "None"

        grid.attach(&template_label, 0, 0, 1, 1);
        grid.attach(&dropdown, 1, 0, 2, 1);

        Some(dropdown)
    };

    // Protocol dropdown (SSH, RDP, VNC) - row depends on whether template dropdown exists
    let protocol_row = i32::from(template_dropdown.is_some());
    let protocol_label = Label::builder()
        .label("Protocol:")
        .halign(gtk4::Align::End)
        .build();
    let protocol_list = gtk4::StringList::new(&["SSH", "RDP", "VNC"]);
    let protocol_dropdown = gtk4::DropDown::builder().model(&protocol_list).build();
    protocol_dropdown.set_selected(0); // Default to SSH
    grid.attach(&protocol_label, 0, protocol_row, 1, 1);
    grid.attach(&protocol_dropdown, 1, protocol_row, 2, 1);

    let host_label = Label::builder()
        .label("Host:")
        .halign(gtk4::Align::End)
        .build();
    let host_entry = gtk4::Entry::builder()
        .hexpand(true)
        .placeholder_text("hostname or IP")
        .build();
    grid.attach(&host_label, 0, protocol_row + 1, 1, 1);
    grid.attach(&host_entry, 1, protocol_row + 1, 2, 1);

    let port_label = Label::builder()
        .label("Port:")
        .halign(gtk4::Align::End)
        .build();
    let port_adj = gtk4::Adjustment::new(22.0, 1.0, 65535.0, 1.0, 10.0, 0.0);
    let port_spin = gtk4::SpinButton::builder()
        .adjustment(&port_adj)
        .climb_rate(1.0)
        .digits(0)
        .build();
    grid.attach(&port_label, 0, protocol_row + 2, 1, 1);
    grid.attach(&port_spin, 1, protocol_row + 2, 1, 1);

    let user_label = Label::builder()
        .label("Username:")
        .halign(gtk4::Align::End)
        .build();
    let user_entry = gtk4::Entry::builder()
        .hexpand(true)
        .placeholder_text("(optional)")
        .build();
    grid.attach(&user_label, 0, protocol_row + 3, 1, 1);
    grid.attach(&user_entry, 1, protocol_row + 3, 2, 1);

    // Password field
    let password_label = Label::builder()
        .label("Password:")
        .halign(gtk4::Align::End)
        .build();
    let password_entry = gtk4::PasswordEntry::builder()
        .hexpand(true)
        .show_peek_icon(true)
        .placeholder_text("(optional)")
        .build();
    grid.attach(&password_label, 0, protocol_row + 4, 1, 1);
    grid.attach(&password_entry, 1, protocol_row + 4, 2, 1);

    content.append(&grid);
    quick_window.set_child(Some(&content));

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
                if let Some(ref username) = &template.username {
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
                _ => 22.0,   // SSH (0) and any other value
            };
            port_spin_clone.set_value(default_port);
        }
        // Reset the flag after protocol change so next protocol change updates port
        *port_manually_changed_clone.borrow_mut() = false;
    });

    // Connect quick connect button
    let window_clone = quick_window.clone();
    let host_clone = host_entry;
    let port_clone = port_spin;
    let user_clone = user_entry;
    let password_clone = password_entry;
    let protocol_clone = protocol_dropdown;
    connect_btn.connect_clicked(move |_| {
        let host = host_clone.text().to_string();
        if host.trim().is_empty() {
            return;
        }

        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
                Some(text.to_string())
            }
        };

        let protocol_idx = protocol_clone.selected();

        match protocol_idx {
            0 => {
                // SSH - use terminal tab
                let session_id = notebook.create_terminal_tab(
                    Uuid::nil(),
                    &format!("Quick: {host}"),
                    "ssh",
                    None,
                );

                notebook.spawn_ssh(session_id, &host, port, username.as_deref(), None, &[]);
            }
            1 => {
                // RDP - use embedded RDP widget
                let embedded_widget = EmbeddedRdpWidget::new();

                let mut embedded_config = EmbeddedRdpConfig::new(&host)
                    .with_port(port)
                    .with_resolution(1920, 1080)
                    .with_clipboard(true);

                if let Some(ref user) = username {
                    embedded_config = embedded_config.with_username(user);
                }

                if let Some(ref pass) = password {
                    embedded_config = embedded_config.with_password(pass);
                }

                let embedded_widget = Rc::new(embedded_widget);
                let session_id = Uuid::new_v4();

                // Connect state change callback
                let notebook_for_state = notebook.clone();
                let sidebar_for_state = sidebar.clone();
                let connection_id = Uuid::nil();
                embedded_widget.connect_state_changed(move |rdp_state| match rdp_state {
                    crate::embedded_rdp::RdpConnectionState::Disconnected => {
                        notebook_for_state.mark_tab_disconnected(session_id);
                        sidebar_for_state
                            .decrement_session_count(&connection_id.to_string(), false);
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
                    tracing::error!("RDP connection failed for '{}': {}", host, e);
                }

                notebook.add_embedded_rdp_tab(
                    session_id,
                    Uuid::nil(),
                    &format!("Quick: {host}"),
                    embedded_widget,
                );

                // Show notebook for RDP session
                split_view.widget().set_visible(false);
                split_view.widget().set_vexpand(false);
                notebook.widget().set_vexpand(true);
                notebook.notebook().set_vexpand(true);
            }
            2 => {
                // VNC - use VNC session widget
                let session_id = notebook.create_vnc_session_tab_with_host(
                    Uuid::nil(),
                    &format!("Quick: {host}"),
                    &host,
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
                            notebook_for_state.mark_tab_disconnected(session_id);
                            sidebar_for_state
                                .decrement_session_count(&connection_id.to_string(), false);
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
                    if let Err(e) = vnc_widget.connect_with_config(
                        &host,
                        port,
                        password.as_deref(),
                        &vnc_config,
                    ) {
                        tracing::error!("Failed to connect VNC session '{}': {}", host, e);
                    }
                }

                // Show notebook for VNC session
                split_view.widget().set_visible(false);
                split_view.widget().set_vexpand(false);
                notebook.widget().set_vexpand(true);
                notebook.notebook().set_vexpand(true);
            }
            _ => {
                // Default to SSH
                let session_id = notebook.create_terminal_tab(
                    Uuid::nil(),
                    &format!("Quick: {host}"),
                    "ssh",
                    None,
                );

                notebook.spawn_ssh(session_id, &host, port, username.as_deref(), None, &[]);
            }
        }

        window_clone.close();
    });

    quick_window.present();
}
