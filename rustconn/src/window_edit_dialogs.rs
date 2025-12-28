//! Edit dialogs for main window
//!
//! This module contains functions for editing connections and groups,
//! showing connection details, and quick connect dialog.

use crate::dialogs::ConnectionDialog;
use crate::sidebar::ConnectionSidebar;
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

/// Shows connection details in a popover
pub fn show_connection_details(
    window: &ApplicationWindow,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
) {
    // Get selected item
    let Some(conn_item) = sidebar.get_selected_item() else {
        return;
    };

    // Only show details for connections, not groups
    if conn_item.is_group() {
        return;
    }

    let id_str = conn_item.id();
    let Ok(id) = Uuid::parse_str(&id_str) else {
        return;
    };

    let state_ref = state.borrow();
    let Some(conn) = state_ref.get_connection(id).cloned() else {
        return;
    };
    drop(state_ref);

    // Create details popover
    let popover = gtk4::Popover::new();
    popover.set_autohide(true);

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.set_width_request(300);

    // Connection name header
    let name_label = Label::builder()
        .label(&conn.name)
        .css_classes(["title-2"])
        .halign(gtk4::Align::Start)
        .build();
    content.append(&name_label);

    // Basic info
    let info_grid = gtk4::Grid::builder()
        .row_spacing(4)
        .column_spacing(8)
        .build();

    let mut row = 0;

    // Protocol
    let protocol_label = Label::builder()
        .label("Protocol:")
        .halign(gtk4::Align::End)
        .css_classes(["dim-label"])
        .build();
    let protocol_value = Label::builder()
        .label(&format!("{:?}", conn.protocol))
        .halign(gtk4::Align::Start)
        .build();
    info_grid.attach(&protocol_label, 0, row, 1, 1);
    info_grid.attach(&protocol_value, 1, row, 1, 1);
    row += 1;

    // Host
    let host_label = Label::builder()
        .label("Host:")
        .halign(gtk4::Align::End)
        .css_classes(["dim-label"])
        .build();
    let host_value = Label::builder()
        .label(&format!("{}:{}", conn.host, conn.port))
        .halign(gtk4::Align::Start)
        .selectable(true)
        .build();
    info_grid.attach(&host_label, 0, row, 1, 1);
    info_grid.attach(&host_value, 1, row, 1, 1);
    row += 1;

    // Username if present
    if let Some(ref username) = conn.username {
        let user_label = Label::builder()
            .label("Username:")
            .halign(gtk4::Align::End)
            .css_classes(["dim-label"])
            .build();
        let user_value = Label::builder()
            .label(username)
            .halign(gtk4::Align::Start)
            .selectable(true)
            .build();
        info_grid.attach(&user_label, 0, row, 1, 1);
        info_grid.attach(&user_value, 1, row, 1, 1);
        row += 1;
    }

    // Tags if present
    if !conn.tags.is_empty() {
        let tags_label = Label::builder()
            .label("Tags:")
            .halign(gtk4::Align::End)
            .css_classes(["dim-label"])
            .build();
        let tags_value = Label::builder()
            .label(&conn.tags.join(", "))
            .halign(gtk4::Align::Start)
            .wrap(true)
            .build();
        info_grid.attach(&tags_label, 0, row, 1, 1);
        info_grid.attach(&tags_value, 1, row, 1, 1);
    }

    content.append(&info_grid);

    // Custom properties section
    if !conn.custom_properties.is_empty() {
        let separator = gtk4::Separator::new(gtk4::Orientation::Horizontal);
        separator.set_margin_top(8);
        separator.set_margin_bottom(8);
        content.append(&separator);

        let props_header = Label::builder()
            .label("Custom Properties")
            .css_classes(["title-4"])
            .halign(gtk4::Align::Start)
            .build();
        content.append(&props_header);

        let props_grid = gtk4::Grid::builder()
            .row_spacing(4)
            .column_spacing(8)
            .build();

        for (idx, prop) in conn.custom_properties.iter().enumerate() {
            let prop_name = Label::builder()
                .label(&format!("{}:", prop.name))
                .halign(gtk4::Align::End)
                .css_classes(["dim-label"])
                .build();

            #[allow(clippy::cast_possible_truncation)]
            let row_idx = idx as i32;

            if prop.is_protected() {
                // Show masked value for protected properties
                let prop_value = Label::builder()
                    .label("••••••••")
                    .halign(gtk4::Align::Start)
                    .build();
                props_grid.attach(&prop_name, 0, row_idx, 1, 1);
                props_grid.attach(&prop_value, 1, row_idx, 1, 1);
            } else if prop.is_url() {
                // Create clickable link for URL properties
                let link_button = gtk4::LinkButton::builder()
                    .uri(&prop.value)
                    .label(&prop.value)
                    .halign(gtk4::Align::Start)
                    .build();
                props_grid.attach(&prop_name, 0, row_idx, 1, 1);
                props_grid.attach(&link_button, 1, row_idx, 1, 1);
            } else {
                // Regular text property
                let prop_value = Label::builder()
                    .label(&prop.value)
                    .halign(gtk4::Align::Start)
                    .selectable(true)
                    .wrap(true)
                    .build();
                props_grid.attach(&prop_name, 0, row_idx, 1, 1);
                props_grid.attach(&prop_value, 1, row_idx, 1, 1);
            }
        }

        content.append(&props_grid);
    }

    popover.set_child(Some(&content));
    popover.set_parent(window);

    // Position the popover near the sidebar
    popover.set_position(gtk4::PositionType::Right);

    // Connect to closed signal to unparent
    popover.connect_closed(|p| {
        p.unparent();
    });

    popover.popup();
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
        .default_width(300)
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

/// Shows the quick connect dialog with protocol selection
#[allow(clippy::too_many_lines)]
pub fn show_quick_connect_dialog(window: &ApplicationWindow, notebook: SharedNotebook) {
    // Create a quick connect window
    let quick_window = gtk4::Window::builder()
        .title("Quick Connect")
        .transient_for(window)
        .modal(true)
        .default_width(550)
        .default_height(550)
        .build();

    // Create header bar with Cancel/Quick Connect buttons
    let header = gtk4::HeaderBar::new();
    let cancel_btn = Button::builder().label("Cancel").build();
    let connect_btn = Button::builder()
        .label("Quick Connect")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&connect_btn);
    quick_window.set_titlebar(Some(&header));

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

    // Protocol dropdown (SSH, RDP, VNC)
    let protocol_label = Label::builder()
        .label("Protocol:")
        .halign(gtk4::Align::End)
        .build();
    let protocol_list = gtk4::StringList::new(&["SSH", "RDP", "VNC"]);
    let protocol_dropdown = gtk4::DropDown::builder().model(&protocol_list).build();
    protocol_dropdown.set_selected(0); // Default to SSH
    grid.attach(&protocol_label, 0, 0, 1, 1);
    grid.attach(&protocol_dropdown, 1, 0, 2, 1);

    let host_label = Label::builder()
        .label("Host:")
        .halign(gtk4::Align::End)
        .build();
    let host_entry = gtk4::Entry::builder()
        .hexpand(true)
        .placeholder_text("hostname or IP")
        .build();
    grid.attach(&host_label, 0, 1, 1, 1);
    grid.attach(&host_entry, 1, 1, 2, 1);

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
    grid.attach(&port_label, 0, 2, 1, 1);
    grid.attach(&port_spin, 1, 2, 1, 1);

    let user_label = Label::builder()
        .label("Username:")
        .halign(gtk4::Align::End)
        .build();
    let user_entry = gtk4::Entry::builder()
        .hexpand(true)
        .placeholder_text("(optional)")
        .build();
    grid.attach(&user_label, 0, 3, 1, 1);
    grid.attach(&user_entry, 1, 3, 2, 1);

    content.append(&grid);
    quick_window.set_child(Some(&content));

    // Track if port was manually changed
    let port_manually_changed = Rc::new(RefCell::new(false));

    // Connect port spin value-changed to track manual changes
    let port_manually_changed_clone = port_manually_changed.clone();
    port_spin.connect_value_changed(move |_| {
        *port_manually_changed_clone.borrow_mut() = true;
    });

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

    // Connect cancel
    let window_clone = quick_window.clone();
    cancel_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    // Connect quick connect button
    let window_clone = quick_window.clone();
    let host_clone = host_entry;
    let port_clone = port_spin;
    let user_clone = user_entry;
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
                // RDP - native embedding not yet implemented, show placeholder
                let _session_id = notebook.create_terminal_tab(
                    Uuid::nil(),
                    &format!("Quick: {host}"),
                    "rdp",
                    None,
                );

                eprintln!(
                    "Note: Native RDP embedding not yet implemented. \
                     Quick connect to '{host}' shown as placeholder."
                );
            }
            2 => {
                // VNC - native embedding not yet implemented, show placeholder
                let _session_id = notebook.create_terminal_tab(
                    Uuid::nil(),
                    &format!("Quick: {host}"),
                    "vnc",
                    None,
                );

                eprintln!(
                    "Note: Native VNC embedding not yet implemented. \
                     Quick connect to '{host}' shown as placeholder."
                );
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
