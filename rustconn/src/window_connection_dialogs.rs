//! Connection and group creation dialogs for main window
//!
//! This module contains dialog functions for creating new connections and groups,
//! including template picker and parent group selection.

use crate::dialogs::{ConnectionDialog, ImportDialog};
use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::window::MainWindow;
use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use std::rc::Rc;
use uuid::Uuid;

/// Type alias for shared sidebar reference
pub type SharedSidebar = Rc<ConnectionSidebar>;

/// Shows the new connection dialog (always creates blank connection)
pub fn show_new_connection_dialog(
    window: &gtk4::Window,
    state: SharedAppState,
    sidebar: SharedSidebar,
) {
    // Always show regular connection dialog (no template picker)
    show_new_connection_dialog_internal(window, state, sidebar, None);
}

/// Internal function to show the new connection dialog with optional template
#[allow(clippy::too_many_lines)]
pub fn show_new_connection_dialog_internal(
    window: &gtk4::Window,
    state: SharedAppState,
    sidebar: SharedSidebar,
    template: Option<rustconn_core::models::ConnectionTemplate>,
) {
    let dialog = ConnectionDialog::new(Some(&window.clone().upcast()));
    dialog.setup_key_file_chooser(Some(&window.clone().upcast()));

    // Set available groups
    {
        let state_ref = state.borrow();
        let groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
        dialog.set_groups(&groups);
    }

    // Set KeePass enabled state from settings
    {
        let state_ref = state.borrow();
        let keepass_enabled = state_ref.settings().secrets.kdbx_enabled;
        dialog.set_keepass_enabled(keepass_enabled);
    }

    // If template provided, pre-populate the dialog
    if let Some(ref tmpl) = template {
        let connection = tmpl.apply(None);
        dialog.set_connection(&connection);
        dialog
            .window()
            .set_title(Some("New Connection from Template"));
    }

    // Connect save to KeePass callback
    let window_for_keepass = window.clone();
    let state_for_save = state.clone();
    dialog.connect_save_to_keepass(move |name, host, username, password, protocol| {
        use secrecy::ExposeSecret;

        let state_ref = state_for_save.borrow();
        let settings = state_ref.settings();

        if !settings.secrets.kdbx_enabled {
            let alert = gtk4::AlertDialog::builder()
                .message("KeePass Not Enabled")
                .detail("Please enable KeePass integration in Settings first.")
                .modal(true)
                .build();
            alert.show(Some(&window_for_keepass));
            return;
        }

        let Some(kdbx_path) = settings.secrets.kdbx_path.clone() else {
            let alert = gtk4::AlertDialog::builder()
                .message("KeePass Database Not Configured")
                .detail("Please select a KeePass database file in Settings.")
                .modal(true)
                .build();
            alert.show(Some(&window_for_keepass));
            return;
        };

        // Build lookup key with protocol suffix for uniqueness
        // Format: "name (protocol)" or "host (protocol)" if name is empty
        let base_name = if name.trim().is_empty() {
            host.to_string()
        } else {
            name.to_string()
        };
        let lookup_key = format!("{base_name} ({protocol})");

        // Get credentials - password and key file can be used together
        let db_password = settings
            .secrets
            .kdbx_password
            .as_ref()
            .map(|p| p.expose_secret().to_string());

        // Key file is optional additional authentication
        let key_file = settings.secrets.kdbx_key_file.clone();

        // Check if we have at least one credential
        if db_password.is_none() && key_file.is_none() {
            let alert = gtk4::AlertDialog::builder()
                .message("KeePass Credentials Required")
                .detail("Please enter the database password or select a key file in Settings.")
                .modal(true)
                .build();
            alert.show(Some(&window_for_keepass));
            return;
        }

        // Build URL for the entry with correct protocol
        let url = format!("{}://{}", protocol, host);

        // Clone data for the background thread
        let username = username.to_string();
        let password = password.to_string();
        let lookup_key_clone = lookup_key.clone();
        let window = window_for_keepass.clone();

        // Drop state borrow before spawning
        drop(state_ref);

        // Run KeePass operation in background thread to avoid blocking UI
        // Use channel to communicate result back to main thread
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = rustconn_core::secret::KeePassStatus::save_password_to_kdbx(
                &kdbx_path,
                db_password.as_deref(),
                key_file.as_deref(),
                &lookup_key_clone,
                &username,
                &password,
                Some(&url),
            );
            let _ = tx.send(result);
        });

        // Poll for result using idle callback
        glib::idle_add_local_once(move || {
            // Try to receive result (non-blocking check, then schedule another idle if not ready)
            fn check_result(
                rx: std::sync::mpsc::Receiver<Result<(), String>>,
                window: gtk4::Window,
                lookup_key: String,
            ) {
                match rx.try_recv() {
                    Ok(Ok(())) => {
                        let alert = gtk4::AlertDialog::builder()
                            .message("Password Saved")
                            .detail(format!("Password for '{lookup_key}' saved to KeePass."))
                            .modal(true)
                            .build();
                        alert.show(Some(&window));
                    }
                    Ok(Err(e)) => {
                        let alert = gtk4::AlertDialog::builder()
                            .message("Failed to Save Password")
                            .detail(format!("Error: {e}"))
                            .modal(true)
                            .build();
                        alert.show(Some(&window));
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        // Not ready yet, schedule another check
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(50),
                            move || check_result(rx, window, lookup_key),
                        );
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        let alert = gtk4::AlertDialog::builder()
                            .message("Failed to Save Password")
                            .detail("Thread error: channel disconnected")
                            .modal(true)
                            .build();
                        alert.show(Some(&window));
                    }
                }
            }
            check_result(rx, window, lookup_key);
        });
    });

    // Connect load from KeePass callback
    let state_for_load = state.clone();
    dialog.connect_load_from_keepass(move |name, host, protocol, password_entry, window| {
        use secrecy::ExposeSecret;

        let state_ref = state_for_load.borrow();
        let settings = state_ref.settings();

        if !settings.secrets.kdbx_enabled {
            crate::toast::show_toast_on_window(
                &window,
                "KeePass integration is not enabled",
                crate::toast::ToastType::Warning,
            );
            return;
        }

        let Some(kdbx_path) = settings.secrets.kdbx_path.clone() else {
            crate::toast::show_toast_on_window(
                &window,
                "KeePass database not configured",
                crate::toast::ToastType::Warning,
            );
            return;
        };

        // Build lookup key with protocol suffix for uniqueness
        // Format: "name (protocol)" or "host (protocol)" if name is empty
        let base_name = if name.trim().is_empty() {
            host.to_string()
        } else {
            name.to_string()
        };
        let lookup_key = format!("{base_name} ({protocol})");

        // Get credentials - password and key file can be used together
        let db_password = settings
            .secrets
            .kdbx_password
            .as_ref()
            .map(|p| p.expose_secret().to_string());

        // Key file is optional additional authentication
        let key_file = settings.secrets.kdbx_key_file.clone();

        // Drop state borrow before spawning
        drop(state_ref);

        // Run KeePass operation in background thread to avoid blocking UI
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                &kdbx_path,
                db_password.as_deref(),
                key_file.as_deref(),
                &lookup_key,
                None, // Protocol already included in lookup_key
            );
            let _ = tx.send(result);
        });

        // Poll for result using idle callback
        glib::idle_add_local_once(move || {
            fn check_result(
                rx: std::sync::mpsc::Receiver<Result<Option<String>, String>>,
                password_entry: gtk4::Entry,
                window: gtk4::Window,
            ) {
                match rx.try_recv() {
                    Ok(Ok(Some(password))) => {
                        password_entry.set_text(&password);
                    }
                    Ok(Ok(None)) => {
                        crate::toast::show_toast_on_window(
                            &window,
                            "No password found in KeePass for this connection",
                            crate::toast::ToastType::Info,
                        );
                    }
                    Ok(Err(e)) => {
                        tracing::warn!("Failed to load password from KeePass: {e}");
                        crate::toast::show_toast_on_window(
                            &window,
                            "Failed to load password from KeePass",
                            crate::toast::ToastType::Error,
                        );
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                        // Not ready yet, schedule another check
                        glib::timeout_add_local_once(
                            std::time::Duration::from_millis(50),
                            move || check_result(rx, password_entry, window),
                        );
                    }
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        tracing::warn!("Thread error loading from KeePass: channel disconnected");
                        crate::toast::show_toast_on_window(
                            &window,
                            "Failed to load password from KeePass",
                            crate::toast::ToastType::Error,
                        );
                    }
                }
            }
            check_result(rx, password_entry, window);
        });
    });

    let window_clone = window.clone();
    dialog.run(move |result| {
        if let Some(conn) = result {
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                match state_mut.create_connection(conn) {
                    Ok(_) => {
                        // Release borrow before scheduling reload
                        drop(state_mut);
                        // Defer sidebar reload to next main loop iteration
                        // This prevents UI freeze during save operation
                        let state_clone = state.clone();
                        let sidebar_clone = sidebar.clone();
                        glib::idle_add_local_once(move || {
                            MainWindow::reload_sidebar_preserving_state(
                                &state_clone,
                                &sidebar_clone,
                            );
                        });
                    }
                    Err(e) => {
                        // Show error in UI dialog with proper transient parent
                        let alert = gtk4::AlertDialog::builder()
                            .message("Error Creating Connection")
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

/// Shows the new group dialog with optional parent selection
pub fn show_new_group_dialog(window: &gtk4::Window, state: SharedAppState, sidebar: SharedSidebar) {
    show_new_group_dialog_with_parent(window, state, sidebar, None);
}

/// Shows the new group dialog with parent group selection
#[allow(clippy::too_many_lines, clippy::needless_pass_by_value)]
pub fn show_new_group_dialog_with_parent(
    window: &gtk4::Window,
    state: SharedAppState,
    sidebar: SharedSidebar,
    preselected_parent: Option<Uuid>,
) {
    let group_window = adw::Window::builder()
        .title("New Group")
        .transient_for(window)
        .modal(true)
        .default_width(500)
        .default_height(300)
        .build();

    // Create header bar with Close/Create buttons (GNOME HIG)
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_show_start_title_buttons(false);
    let close_btn = gtk4::Button::builder().label("Close").build();
    let create_btn = gtk4::Button::builder()
        .label("Create")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&close_btn);
    header.pack_end(&create_btn);

    // Close button handler
    let window_clone = group_window.clone();
    close_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    // Scrollable content with clamp
    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .tightening_threshold(400)
        .build();

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    clamp.set_child(Some(&content));

    let main_box = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    main_box.append(&header);
    main_box.append(&clamp);
    group_window.set_content(Some(&main_box));

    // === Group Details ===
    let details_group = adw::PreferencesGroup::builder()
        .title("Group Details")
        .build();

    // Group name using ActionRow with Entry
    let entry = gtk4::Entry::builder()
        .hexpand(true)
        .valign(gtk4::Align::Center)
        .placeholder_text("Enter group name")
        .build();
    let name_row = adw::ActionRow::builder().title("Name").build();
    name_row.add_suffix(&entry);
    name_row.set_activatable_widget(Some(&entry));
    details_group.add(&name_row);

    // Parent group dropdown
    let state_ref = state.borrow();
    let groups: Vec<_> = state_ref
        .list_groups()
        .iter()
        .map(|g| (*g).clone())
        .collect();
    drop(state_ref);

    let mut group_ids: Vec<Option<Uuid>> = vec![None];
    let mut strings: Vec<String> = vec!["(None - Root Level)".to_string()];
    let mut preselected_index = 0u32;

    for group in &groups {
        let state_ref = state.borrow();
        let path = state_ref
            .get_group_path(group.id)
            .unwrap_or_else(|| group.name.clone());
        drop(state_ref);

        strings.push(path);
        group_ids.push(Some(group.id));

        if preselected_parent == Some(group.id) {
            #[allow(clippy::cast_possible_truncation)]
            {
                preselected_index = (group_ids.len() - 1) as u32;
            }
        }
    }

    let string_list = gtk4::StringList::new(
        &strings
            .iter()
            .map(std::string::String::as_str)
            .collect::<Vec<_>>(),
    );
    let parent_dropdown = gtk4::DropDown::builder()
        .model(&string_list)
        .selected(preselected_index)
        .valign(gtk4::Align::Center)
        .build();

    let parent_row = adw::ActionRow::builder()
        .title("Parent")
        .subtitle("Optional - leave empty for root level")
        .build();
    parent_row.add_suffix(&parent_dropdown);
    details_group.add(&parent_row);

    content.append(&details_group);

    // Connect create button
    let state_clone = state.clone();
    let sidebar_clone = sidebar;
    let window_clone = group_window.clone();
    let entry_clone = entry;
    let dropdown_clone = parent_dropdown;
    create_btn.connect_clicked(move |_| {
        let name = entry_clone.text().to_string();
        if name.trim().is_empty() {
            let alert = gtk4::AlertDialog::builder()
                .message("Validation Error")
                .detail("Group name cannot be empty")
                .modal(true)
                .build();
            alert.show(Some(&window_clone));
            return;
        }

        let selected_idx = dropdown_clone.selected() as usize;
        let parent_id = if selected_idx < group_ids.len() {
            group_ids[selected_idx]
        } else {
            None
        };

        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
            let result = if let Some(pid) = parent_id {
                state_mut.create_group_with_parent(name, pid)
            } else {
                state_mut.create_group(name)
            };

            match result {
                Ok(_) => {
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
    });

    group_window.present();
}

/// Shows the import dialog
pub fn show_import_dialog(window: &gtk4::Window, state: SharedAppState, sidebar: SharedSidebar) {
    let dialog = ImportDialog::new(Some(&window.clone().upcast()));

    let window_clone = window.clone();
    dialog.run_with_source(move |result, source_name| {
        if let Some(import_result) = result {
            if let Ok(mut state_mut) = state.try_borrow_mut() {
                match state_mut.import_connections_with_source(&import_result, &source_name) {
                    Ok(count) => {
                        drop(state_mut);
                        // Defer sidebar reload to prevent UI freeze
                        let state_clone = state.clone();
                        let sidebar_clone = sidebar.clone();
                        let window = window_clone.clone();
                        let source = source_name.clone();
                        glib::idle_add_local_once(move || {
                            MainWindow::reload_sidebar_preserving_state(
                                &state_clone,
                                &sidebar_clone,
                            );
                            let alert = gtk4::AlertDialog::builder()
                                .message("Import Successful")
                                .detail(format!("Imported {count} connections to '{source}' group"))
                                .modal(true)
                                .build();
                            alert.show(Some(&window));
                        });
                    }
                    Err(e) => {
                        let alert = gtk4::AlertDialog::builder()
                            .message("Import Failed")
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
