//! Connection and group creation dialogs for main window
//!
//! This module contains dialog functions for creating new connections and groups,
//! including template picker and parent group selection.

use crate::alert;
use crate::dialogs::{ConnectionDialog, ImportDialog};
use crate::sidebar::ConnectionSidebar;
use crate::state::SharedAppState;
use crate::window::MainWindow;
use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use rustconn_core::models::{Credentials, PasswordSource};
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
        let mut groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
        groups.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
        dialog.set_groups(&groups);
        let connections: Vec<_> = state_ref.list_connections().into_iter().cloned().collect();
        dialog.set_connections(&connections);
    }

    // Set preferred backend based on settings (filters password source dropdown)
    {
        let state_ref = state.borrow();
        let preferred_backend = state_ref.settings().secrets.preferred_backend;
        dialog.set_preferred_backend(preferred_backend);
    }

    // Set up password visibility toggle and source visibility
    dialog.connect_password_visibility_toggle();
    dialog.connect_password_source_visibility();
    dialog.update_password_row_visibility();

    // Set up password load button with KeePass settings
    {
        use secrecy::ExposeSecret;
        let state_ref = state.borrow();
        let settings = state_ref.settings();
        dialog.connect_password_load_button(
            settings.secrets.kdbx_enabled,
            settings.secrets.kdbx_path.clone(),
            settings
                .secrets
                .kdbx_password
                .as_ref()
                .map(|p| p.expose_secret().to_string()),
            settings.secrets.kdbx_key_file.clone(),
        );
    }

    // If template provided, pre-populate the dialog
    if let Some(ref tmpl) = template {
        let connection = tmpl.apply(None);
        dialog.set_connection(&connection);
        dialog
            .window()
            .set_title(Some("New Connection from Template"));
    }

    let window_clone = window.clone();
    dialog.run(move |result| {
        if let Some(dialog_result) = result {
            let conn = dialog_result.connection;
            let password = dialog_result.password;

            if let Ok(mut state_mut) = state.try_borrow_mut() {
                // Clone values needed for password saving before creating connection
                let conn_name = conn.name.clone();
                let conn_host = conn.host.clone();
                let conn_username = conn.username.clone();
                let password_source = conn.password_source;
                let protocol = conn.protocol;

                match state_mut.create_connection(conn) {
                    Ok(conn_id) => {
                        // Save password to KeePass if password source is KeePass and password
                        // was provided
                        if password_source == PasswordSource::KeePass {
                            if let Some(pwd) = password.clone() {
                                // Get KeePass settings
                                let settings = state_mut.settings().clone();
                                if settings.secrets.kdbx_enabled {
                                    if let Some(kdbx_path) = settings.secrets.kdbx_path.clone() {
                                        let key_file = settings.secrets.kdbx_key_file.clone();
                                        let entry_name = format!(
                                            "{} ({})",
                                            conn_name,
                                            protocol.as_str().to_lowercase()
                                        );
                                        let username = conn_username.clone().unwrap_or_default();
                                        let url = format!(
                                            "{}://{}",
                                            protocol.as_str().to_lowercase(),
                                            conn_host
                                        );

                                        // Save password in background
                                        crate::utils::spawn_blocking_with_callback(
                                            move || {
                                                let kdbx = std::path::Path::new(&kdbx_path);
                                                let key = key_file
                                                    .as_ref()
                                                    .map(|p| std::path::Path::new(p));
                                                rustconn_core::secret::KeePassStatus
                                                    ::save_password_to_kdbx(
                                                        kdbx,
                                                        None, // No db password (using key file)
                                                        key,
                                                        &entry_name,
                                                        &username,
                                                        &pwd,
                                                        Some(&url),
                                                    )
                                            },
                                            move |result| {
                                                if let Err(e) = result {
                                                    tracing::error!(
                                                        "Failed to save password to KeePass: {}",
                                                        e
                                                    );
                                                } else {
                                                    tracing::info!(
                                                        "Password saved to KeePass for \
                                                         connection {}",
                                                        conn_id
                                                    );
                                                }
                                            },
                                        );
                                    }
                                }
                            }
                        }

                        // Save password to Keyring if password source is Keyring
                        if password_source == PasswordSource::Keyring {
                            if let Some(pwd) = password.clone() {
                                let lookup_key = format!(
                                    "{} ({})",
                                    conn_name.replace('/', "-"),
                                    protocol.as_str().to_lowercase()
                                );
                                let username = conn_username.clone().unwrap_or_default();

                                // Save password in background
                                crate::utils::spawn_blocking_with_callback(
                                    move || {
                                        use rustconn_core::secret::SecretBackend;
                                        let backend = rustconn_core::secret::LibSecretBackend::new(
                                            "rustconn",
                                        );
                                        let creds = Credentials {
                                            username: Some(username),
                                            password: Some(secrecy::SecretString::from(pwd)),
                                            key_passphrase: None,
                                            domain: None,
                                        };
                                        let rt = tokio::runtime::Runtime::new()
                                            .map_err(|e| format!("Runtime error: {e}"))?;
                                        rt.block_on(backend.store(&lookup_key, &creds))
                                            .map_err(|e| format!("{e}"))
                                    },
                                    move |result: Result<(), String>| {
                                        if let Err(e) = result {
                                            tracing::error!(
                                                "Failed to save password to Keyring: {}",
                                                e
                                            );
                                        } else {
                                            tracing::info!(
                                                "Password saved to Keyring for connection {}",
                                                conn_id
                                            );
                                        }
                                    },
                                );
                            }
                        }

                        // Save password to Bitwarden if password source is Bitwarden
                        if password_source == PasswordSource::Bitwarden {
                            if let Some(pwd) = password {
                                let lookup_key = format!(
                                    "{} ({})",
                                    conn_name.replace('/', "-"),
                                    protocol.as_str().to_lowercase()
                                );
                                let username = conn_username.unwrap_or_default();

                                // Save password in background
                                crate::utils::spawn_blocking_with_callback(
                                    move || {
                                        use rustconn_core::secret::SecretBackend;
                                        let backend =
                                            rustconn_core::secret::BitwardenBackend::new();
                                        let creds = Credentials {
                                            username: Some(username),
                                            password: Some(secrecy::SecretString::from(pwd)),
                                            key_passphrase: None,
                                            domain: None,
                                        };
                                        let rt = tokio::runtime::Runtime::new()
                                            .map_err(|e| format!("Runtime error: {e}"))?;
                                        rt.block_on(backend.store(&lookup_key, &creds))
                                            .map_err(|e| format!("{e}"))
                                    },
                                    move |result: Result<(), String>| {
                                        if let Err(e) = result {
                                            tracing::error!(
                                                "Failed to save password to Bitwarden: {}",
                                                e
                                            );
                                        } else {
                                            tracing::info!(
                                                "Password saved to Bitwarden for connection {}",
                                                conn_id
                                            );
                                        }
                                    },
                                );
                            }
                        }

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
                        alert::show_error(&window_clone, "Error Creating Connection", &e);
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

    // Use ToolbarView for proper adw::Window layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&clamp));
    group_window.set_content(Some(&toolbar_view));

    // === Group Details ===
    let details_group = adw::PreferencesGroup::builder()
        .title("Group Details")
        .build();

    // Group name using EntryRow
    let name_row = adw::EntryRow::builder().title("Name").build();
    details_group.add(&name_row);

    // Parent group dropdown
    let state_ref = state.borrow();

    // Sort by full path (displayed string)
    let mut groups: Vec<(Uuid, String)> = state_ref
        .list_groups()
        .iter()
        .map(|g| {
            let path = state_ref
                .get_group_path(g.id)
                .unwrap_or_else(|| g.name.clone());
            (g.id, path)
        })
        .collect();
    groups.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
    drop(state_ref);

    let mut group_ids: Vec<Option<Uuid>> = vec![None];
    let mut strings: Vec<String> = vec!["(None - Root Level)".to_string()];
    let mut preselected_index = 0u32;

    for (id, path) in groups {
        strings.push(path);
        group_ids.push(Some(id));

        if preselected_parent == Some(id) {
            preselected_index = (group_ids.len() - 1) as u32;
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

    // === Inheritable Credentials ===
    let credentials_group = adw::PreferencesGroup::builder()
        .title("Default Credentials")
        .description("Credentials inherited by connections in this group")
        .build();

    let username_row = adw::EntryRow::builder().title("Username").build();
    credentials_group.add(&username_row);

    // Password Source dropdown
    let password_source_list = gtk4::StringList::new(&[
        "Prompt",
        "KeePass",
        "Keyring",
        "Bitwarden",
        "Inherit",
        "None",
    ]);
    let password_source_dropdown = gtk4::DropDown::builder()
        .model(&password_source_list)
        .selected(5) // Default to None
        .valign(gtk4::Align::Center)
        .build();

    let password_source_row = adw::ActionRow::builder().title("Password").build();
    password_source_row.add_suffix(&password_source_dropdown);
    credentials_group.add(&password_source_row);

    // Password Value entry with visibility toggle
    let password_entry = gtk4::Entry::builder()
        .placeholder_text("Password value")
        .visibility(false)
        .hexpand(true)
        .build();
    let password_visibility_btn = gtk4::Button::builder()
        .icon_name("view-reveal-symbolic")
        .tooltip_text("Show/hide password")
        .valign(gtk4::Align::Center)
        .build();

    let password_value_row = adw::ActionRow::builder().title("Value").build();
    password_value_row.add_suffix(&password_entry);
    password_value_row.add_suffix(&password_visibility_btn);
    credentials_group.add(&password_value_row);

    // Initially hidden (None selected)
    password_value_row.set_visible(false);

    // Connect password source dropdown to show/hide value row
    let value_row_clone = password_value_row.clone();
    password_source_dropdown.connect_selected_notify(move |dropdown| {
        let selected = dropdown.selected();
        let show = matches!(selected, 1..=3);
        value_row_clone.set_visible(show);
    });

    // Connect password visibility toggle
    let password_entry_clone = password_entry.clone();
    let is_visible = std::rc::Rc::new(std::cell::Cell::new(false));
    password_visibility_btn.connect_clicked(move |btn| {
        let currently_visible = is_visible.get();
        let new_visible = !currently_visible;
        is_visible.set(new_visible);
        password_entry_clone.set_visibility(new_visible);
        if new_visible {
            btn.set_icon_name("view-conceal-symbolic");
        } else {
            btn.set_icon_name("view-reveal-symbolic");
        }
    });

    let domain_row = adw::EntryRow::builder().title("Domain").build();
    credentials_group.add(&domain_row);

    content.append(&credentials_group);

    // Connect create button
    let state_clone = state.clone();
    let sidebar_clone = sidebar;
    let window_clone = group_window.clone();
    let name_row_clone = name_row;
    let dropdown_clone = parent_dropdown;
    let username_row_clone = username_row;
    let password_entry_clone2 = password_entry.clone();
    let password_source_clone = password_source_dropdown.clone();
    let domain_row_clone = domain_row;

    create_btn.connect_clicked(move |_| {
        let name = name_row_clone.text().to_string();
        if name.trim().is_empty() {
            alert::show_validation_error(&window_clone, "Group name cannot be empty");
            return;
        }

        let selected_idx = dropdown_clone.selected() as usize;
        let parent_id = if selected_idx < group_ids.len() {
            group_ids[selected_idx]
        } else {
            None
        };

        // Capture credential values
        let username = username_row_clone.text().to_string();
        let password = password_entry_clone2.text().to_string();
        let domain = domain_row_clone.text().to_string();

        // Get selected password source
        let password_source_idx = password_source_clone.selected();
        let new_password_source = match password_source_idx {
            0 => PasswordSource::Prompt,
            1 => PasswordSource::KeePass,
            2 => PasswordSource::Keyring,
            3 => PasswordSource::Bitwarden,
            4 => PasswordSource::Inherit,
            _ => PasswordSource::None,
        };

        let has_username = !username.trim().is_empty();
        // Password is relevant only for KeePass, Keyring, Bitwarden
        let has_password = !password.is_empty() && matches!(password_source_idx, 1..=3);
        let has_domain = !domain.trim().is_empty();

        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
            let result = if let Some(pid) = parent_id {
                state_mut.create_group_with_parent(name, pid)
            } else {
                state_mut.create_group(name)
            };

            match result {
                Ok(group_id) => {
                    // Update group with credentials if provided
                    if has_username
                        || has_domain
                        || has_password
                        || !matches!(new_password_source, PasswordSource::None)
                    {
                        if let Some(existing) = state_mut.get_group(group_id).cloned() {
                            let mut updated = existing;
                            if has_username {
                                updated.username = Some(username.clone());
                            }
                            if has_domain {
                                updated.domain = Some(domain.clone());
                            }
                            // Set the selected password source
                            updated.password_source = Some(new_password_source);

                            if let Err(e) = state_mut
                                .connection_manager()
                                .update_group(group_id, updated)
                            {
                                alert::show_error(
                                    &window_clone,
                                    "Error Updating Group",
                                    &e.to_string(),
                                );
                                // Don't return, allow closing window since group was created
                            }
                        }
                    }

                    // Save password if provided
                    if has_password {
                        let secret_manager = state_mut.secret_manager().clone();
                        let creds = Credentials::with_password(
                            if has_username { &username } else { "" },
                            password,
                        );
                        let gid_str = group_id.to_string();

                        crate::utils::spawn_blocking_with_callback(
                            move || {
                                let rt =
                                    tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
                                rt.block_on(async {
                                    secret_manager
                                        .store(&gid_str, &creds)
                                        .await
                                        .map_err(|e| e.to_string())
                                })
                            },
                            move |result| {
                                if let Err(e) = result {
                                    tracing::error!("Failed to save group password: {}", e);
                                }
                            },
                        );
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
                Err(e) => {
                    alert::show_error(&window_clone, "Error", &e);
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
                            alert::show_success(
                                &window,
                                "Import Successful",
                                &format!("Imported {count} connections to '{source}' group"),
                            );
                        });
                    }
                    Err(e) => {
                        alert::show_error(&window_clone, "Import Failed", &e);
                    }
                }
            }
        }
    });
}
