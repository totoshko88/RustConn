//! Edit dialogs for main window
//!
//! This module contains functions for editing connections and groups,
//! showing connection details, and quick connect dialog.

use crate::alert;
use crate::dialogs::ConnectionDialog;
use crate::embedded_rdp::{EmbeddedRdpWidget, RdpConfig as EmbeddedRdpConfig};
use crate::sidebar::ConnectionSidebar;
use crate::split_view::SplitViewBridge;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
use crate::window::MainWindow;
use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4::{Button, Label, Orientation};
use libadwaita as adw;
use rustconn_core::models::{Credentials, PasswordSource};
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

        let dialog = ConnectionDialog::new(Some(&window.clone().upcast()));
        dialog.setup_key_file_chooser(Some(&window.clone().upcast()));

        // Set available groups
        {
            let state_ref = state.borrow();
            let mut groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
            groups.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
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

        dialog.set_connection(&conn);

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
                    let password_source = updated_conn.password_source;
                    let protocol = updated_conn.protocol;

                    match state_mut.update_connection(id, updated_conn) {
                        Ok(()) => {
                            // Save password to KeePass if needed
                            if password_source == PasswordSource::KeePass {
                                if let Some(pwd) = password.clone() {
                                    let settings = state_mut.settings().clone();
                                    if settings.secrets.kdbx_enabled {
                                        if let Some(kdbx_path) = settings.secrets.kdbx_path.clone()
                                        {
                                            let key_file = settings.secrets.kdbx_key_file.clone();
                                            let entry_name = format!(
                                                "{} ({})",
                                                conn_name,
                                                protocol.as_str().to_lowercase()
                                            );
                                            let username =
                                                conn_username.clone().unwrap_or_default();
                                            let url = format!(
                                                "{}://{}",
                                                protocol.as_str().to_lowercase(),
                                                conn_host
                                            );

                                            crate::utils::spawn_blocking_with_callback(
                                                move || {
                                                    let kdbx = std::path::Path::new(&kdbx_path);
                                                    let key = key_file
                                                        .as_ref()
                                                        .map(|p| std::path::Path::new(p));
                                                    rustconn_core::secret::KeePassStatus
                                                        ::save_password_to_kdbx(
                                                            kdbx,
                                                            None,
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
                                                            "Failed to save password to \
                                                             KeePass: {}",
                                                            e
                                                        );
                                                    } else {
                                                        tracing::info!(
                                                            "Password saved to KeePass for \
                                                             connection {}",
                                                            id
                                                        );
                                                    }
                                                },
                                            );
                                        }
                                    }
                                }
                            }

                            // Save password to Keyring if needed
                            if password_source == PasswordSource::Keyring {
                                if let Some(pwd) = password.clone() {
                                    let lookup_key = format!(
                                        "{} ({})",
                                        conn_name.replace('/', "-"),
                                        protocol.as_str().to_lowercase()
                                    );
                                    let username = conn_username.clone().unwrap_or_default();

                                    crate::utils::spawn_blocking_with_callback(
                                        move || {
                                            use rustconn_core::secret::SecretBackend;
                                            let backend =
                                                rustconn_core::secret::LibSecretBackend::new(
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
                                                    id
                                                );
                                            }
                                        },
                                    );
                                }
                            }

                            // Save password to Bitwarden if needed
                            if password_source == PasswordSource::Bitwarden {
                                if let Some(pwd) = password {
                                    let lookup_key = format!(
                                        "{} ({})",
                                        conn_name.replace('/', "-"),
                                        protocol.as_str().to_lowercase()
                                    );
                                    let username = conn_username.unwrap_or_default();

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
                                                    id
                                                );
                                            }
                                        },
                                    );
                                }
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
                            alert::show_error(&window_clone, "Error Updating Connection", &e);
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
    let rename_window = adw::Window::builder()
        .title(if is_group {
            "Rename Group"
        } else {
            "Rename Connection"
        })
        .modal(true)
        .default_width(400)
        .resizable(false)
        .build();
    rename_window.set_transient_for(Some(window));

    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_show_start_title_buttons(false);
    let cancel_btn = gtk4::Button::builder().label("Cancel").build();
    let save_btn = gtk4::Button::builder()
        .label("Rename")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&save_btn);

    let content = gtk4::Box::new(Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Name entry using PreferencesGroup with EntryRow
    let name_group = adw::PreferencesGroup::new();
    let name_row = adw::EntryRow::builder()
        .title("Name")
        .text(&current_name)
        .build();
    name_group.add(&name_row);
    content.append(&name_group);

    // Use ToolbarView for proper adw::Window layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    rename_window.set_content(Some(&toolbar_view));

    // Cancel button
    let window_clone = rename_window.clone();
    cancel_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    // Save button
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let window_clone = rename_window.clone();
    let name_row_clone = name_row.clone();
    save_btn.connect_clicked(move |_| {
        let new_name = name_row_clone.text().trim().to_string();
        if new_name.is_empty() {
            alert::show_validation_error(&window_clone, "Name cannot be empty");
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
                    &format!("Group with name '{new_name}' already exists"),
                );
                return;
            }
            drop(state_ref);

            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                if let Some(existing) = state_mut.get_group(id).cloned() {
                    let mut updated = existing;
                    updated.name = new_name.clone();
                    if let Err(e) = state_mut.connection_manager().update_group(id, updated) {
                        alert::show_error(&window_clone, "Error", &format!("{e}"));
                        return;
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
            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                if let Some(existing) = state_mut.get_connection(id).cloned() {
                    let mut updated = existing;
                    updated.name = new_name;
                    match state_mut.update_connection(id, updated) {
                        Ok(()) => {
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
            }
        }
    });

    // Enter key triggers save
    let save_btn_clone = save_btn.clone();
    name_row.connect_entry_activated(move |_| {
        save_btn_clone.emit_clicked();
    });

    rename_window.present();
    name_row.grab_focus();
}

/// Shows dialog to edit a group name
// SharedAppState is Rc<RefCell<...>> - cheap to clone and needed for closure ownership
#[allow(clippy::needless_pass_by_value)]
pub fn show_edit_group_dialog(
    window: &gtk4::Window,
    state: SharedAppState,
    sidebar: SharedSidebar,
    group_id: Uuid,
) {
    let state_ref = state.borrow();
    let Some(group) = state_ref.get_group(group_id).cloned() else {
        return;
    };
    drop(state_ref);

    // Create group window with Adwaita
    let group_window = adw::Window::builder()
        .title("Edit Group")
        .modal(true)
        .default_width(500)
        .resizable(false)
        .build();
    group_window.set_transient_for(Some(window));

    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_show_start_title_buttons(false);
    let cancel_btn = gtk4::Button::builder().label("Cancel").build();
    let save_btn = gtk4::Button::builder()
        .label("Save")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&save_btn);

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

    // Name entry using PreferencesGroup with EntryRow
    let name_row = adw::EntryRow::builder()
        .title("Name")
        .text(&group.name)
        .build();
    details_group.add(&name_row);

    // Parent group dropdown
    let state_ref = state.borrow();

    // Get all groups and filter out self and descendants to avoid cycles
    let mut available_groups: Vec<(Uuid, String)> = Vec::new();
    let all_groups = state_ref.list_groups();

    // Helper to check if a group is a descendant of the current group
    let is_descendant = |possible_descendant: Uuid| -> bool {
        let mut current = possible_descendant;
        let mut visited = std::collections::HashSet::new();

        while let Some(g) = state_ref.get_group(current) {
            if !visited.insert(current) {
                break;
            }
            if current == group_id {
                return true;
            }
            match g.parent_id {
                Some(p) => current = p,
                None => break,
            }
        }
        false
    };

    for g in all_groups {
        if g.id == group_id {
            continue;
        }
        if is_descendant(g.id) {
            continue;
        }

        // Use full path for display and sorting
        let path = state_ref
            .get_group_path(g.id)
            .unwrap_or_else(|| g.name.clone());
        available_groups.push((g.id, path));
    }
    drop(state_ref);

    // Sort by the displayed path string
    available_groups.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

    let mut group_ids: Vec<Option<Uuid>> = vec![None];
    let mut strings: Vec<String> = vec!["(None - Root Level)".to_string()];
    let mut preselected_index = 0u32;

    for (id, path) in available_groups {
        strings.push(path);
        group_ids.push(Some(id));

        if group.parent_id == Some(id) {
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
        .subtitle("Moving a group moves all its content")
        .build();
    parent_row.add_suffix(&parent_dropdown);
    details_group.add(&parent_row);

    content.append(&details_group);

    // === Inheritable Credentials ===
    let credentials_group = adw::PreferencesGroup::builder()
        .title("Default Credentials")
        .description("Credentials inherited by connections in this group")
        .build();

    let username_row = adw::EntryRow::builder()
        .title("Username")
        .text(group.username.as_deref().unwrap_or_default())
        .build();
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
        .valign(gtk4::Align::Center)
        .build();
    // Set initial selection based on group's password_source
    let initial_source_idx = match group.password_source {
        Some(PasswordSource::Prompt) => 0,
        Some(PasswordSource::KeePass) => 1,
        Some(PasswordSource::Keyring) => 2,
        Some(PasswordSource::Bitwarden) => 3,
        Some(PasswordSource::Inherit) => 4,
        Some(PasswordSource::None) | None => 5,
    };
    password_source_dropdown.set_selected(initial_source_idx);

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

    // Show/hide password value row based on source selection
    // Show for Stored(1), KeePass(2), Keyring(3)
    let show_value = matches!(initial_source_idx, 1..=3);
    password_value_row.set_visible(show_value);

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

    let domain_row = adw::EntryRow::builder()
        .title("Domain")
        .text(group.domain.as_deref().unwrap_or_default())
        .build();
    credentials_group.add(&domain_row);

    content.append(&credentials_group);

    // Connect handlers
    let window_clone = group_window.clone();
    cancel_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    let state_clone = state.clone();
    let sidebar_clone = sidebar;
    let window_clone = group_window.clone();
    let name_row_clone = name_row;
    let username_row_clone = username_row;
    let password_entry_clone = password_entry.clone();
    let password_source_clone = password_source_dropdown.clone();
    let domain_row_clone = domain_row;
    let dropdown_clone = parent_dropdown;
    let old_name = group.name;

    save_btn.connect_clicked(move |_| {
        let new_name = name_row_clone.text().to_string();
        if new_name.trim().is_empty() {
            alert::show_validation_error(&window_clone, "Group name cannot be empty");
            return;
        }

        let selected_idx = dropdown_clone.selected() as usize;
        let new_parent_id = if selected_idx < group_ids.len() {
            group_ids[selected_idx]
        } else {
            None
        };

        let username = username_row_clone.text().to_string();
        let password = password_entry_clone.text().to_string();
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

        // Password is relevant only for KeePass, Keyring, Bitwarden
        let has_new_password = !password.is_empty() && matches!(password_source_idx, 1..=3);

        // Check for duplicate name (but allow keeping same name)
        if new_name != old_name {
            let state_ref = state_clone.borrow();
            if state_ref.group_exists_by_name(&new_name) {
                drop(state_ref);
                alert::show_validation_error(
                    &window_clone,
                    &format!("Group with name '{new_name}' already exists"),
                );
                return;
            }
            drop(state_ref);
        }

        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
            if let Some(existing) = state_mut.get_group(group_id).cloned() {
                let mut updated = existing;
                updated.name = new_name;
                updated.parent_id = new_parent_id;

                updated.username = if username.trim().is_empty() {
                    None
                } else {
                    Some(username.clone())
                };

                updated.domain = if domain.trim().is_empty() {
                    None
                } else {
                    Some(domain)
                };

                updated.password_source = Some(new_password_source);

                if let Err(e) = state_mut
                    .connection_manager()
                    .update_group(group_id, updated)
                {
                    alert::show_error(&window_clone, "Error", &format!("{e}"));
                    return;
                }

                // Save password if provided and source requires it
                if has_new_password {
                    let secret_manager = state_mut.secret_manager().clone();
                    let creds = Credentials::with_password(
                        if username.trim().is_empty() {
                            ""
                        } else {
                            &username
                        },
                        password,
                    );
                    let gid_str = group_id.to_string();

                    crate::utils::spawn_blocking_with_callback(
                        move || {
                            let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
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
    });

    group_window.present();
}

/// Shows the quick connect dialog with protocol selection and template support
pub fn show_quick_connect_dialog(
    window: &gtk4::Window,
    notebook: SharedNotebook,
    split_view: SharedSplitView,
    sidebar: SharedSidebar,
) {
    show_quick_connect_dialog_with_state(window, notebook, split_view, sidebar, None);
}

/// Parameters for a quick connect session
struct QuickConnectParams {
    host: String,
    port: u16,
    username: Option<String>,
    password: Option<String>,
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
    );
    notebook.spawn_ssh(
        session_id,
        &params.host,
        params.port,
        params.username.as_deref(),
        None,
        &[],
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
        if let Err(e) = vnc_widget.connect_with_config(
            &params.host,
            params.port,
            params.password.as_deref(),
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
) {
    // Collect templates if state is available
    let templates: Vec<rustconn_core::models::ConnectionTemplate> = state
        .map(|s| {
            let state_ref = s.borrow();
            state_ref.load_templates().unwrap_or_default()
        })
        .unwrap_or_default();

    // Create a quick connect window with Adwaita
    let quick_window = adw::Window::builder()
        .title("Quick Connect")
        .modal(true)
        .default_width(500)
        .default_height(400)
        .build();

    if let Some(gtk_win) = window.downcast_ref::<gtk4::Window>() {
        quick_window.set_transient_for(Some(gtk_win));
    }

    // Create header bar with Close/Connect buttons (GNOME HIG)
    let header = adw::HeaderBar::new();
    header.set_show_end_title_buttons(false);
    header.set_show_start_title_buttons(false);
    let close_btn = Button::builder().label("Close").build();
    let connect_btn = Button::builder()
        .label("Connect")
        .css_classes(["suggested-action"])
        .build();
    header.pack_start(&close_btn);
    header.pack_end(&connect_btn);

    // Close button handler
    let window_clone = quick_window.clone();
    close_btn.connect_clicked(move |_| {
        window_clone.close();
    });

    // Main content
    let content = gtk4::Box::new(Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    // Info label
    let info_label = Label::new(Some("⚠ This connection will not be saved"));
    info_label.add_css_class("dim-label");
    content.append(&info_label);

    // Connection settings group
    let settings_group = adw::PreferencesGroup::new();

    // Template row (if templates available)
    let template_dropdown: Option<gtk4::DropDown> = if templates.is_empty() {
        None
    } else {
        let mut template_names: Vec<String> = vec!["(None)".to_string()];
        template_names.extend(templates.iter().map(|t| t.name.clone()));
        let template_strings: Vec<&str> = template_names.iter().map(String::as_str).collect();
        let template_list = gtk4::StringList::new(&template_strings);

        let dropdown = gtk4::DropDown::builder()
            .model(&template_list)
            .valign(gtk4::Align::Center)
            .build();
        dropdown.set_selected(0);

        let template_row = adw::ActionRow::builder().title("Template").build();
        template_row.add_suffix(&dropdown);
        settings_group.add(&template_row);

        Some(dropdown)
    };

    // Protocol dropdown
    let protocol_list = gtk4::StringList::new(&["SSH", "RDP", "VNC"]);
    let protocol_dropdown = gtk4::DropDown::builder()
        .model(&protocol_list)
        .valign(gtk4::Align::Center)
        .build();
    protocol_dropdown.set_selected(0);
    let protocol_row = adw::ActionRow::builder().title("Protocol").build();
    protocol_row.add_suffix(&protocol_dropdown);
    settings_group.add(&protocol_row);

    // Host entry
    let host_entry = gtk4::Entry::builder()
        .placeholder_text("hostname or IP")
        .valign(gtk4::Align::Center)
        .hexpand(true)
        .build();
    let host_row = adw::ActionRow::builder().title("Host").build();
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
    let port_row = adw::ActionRow::builder().title("Port").build();
    port_row.add_suffix(&port_spin);
    settings_group.add(&port_row);

    // Username entry
    let user_entry = gtk4::Entry::builder()
        .placeholder_text("(optional)")
        .valign(gtk4::Align::Center)
        .hexpand(true)
        .build();
    let user_row = adw::ActionRow::builder().title("Username").build();
    user_row.add_suffix(&user_entry);
    settings_group.add(&user_row);

    // Password entry
    let password_entry = gtk4::PasswordEntry::builder()
        .show_peek_icon(true)
        .placeholder_text("(optional)")
        .valign(gtk4::Align::Center)
        .hexpand(true)
        .build();
    let password_row = adw::ActionRow::builder().title("Password").build();
    password_row.add_suffix(&password_entry);
    settings_group.add(&password_row);

    content.append(&settings_group);

    // Use ToolbarView for proper adw::Window layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&content));
    quick_window.set_content(Some(&toolbar_view));

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
    // Clone state for use in closure
    let state_for_connect = state.cloned();
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

        let params = QuickConnectParams {
            host,
            port,
            username,
            password,
        };

        match protocol_clone.selected() {
            0 => start_quick_ssh(&notebook, &params, &terminal_settings),
            1 => start_quick_rdp(&notebook, &split_view, &sidebar, &params),
            2 => start_quick_vnc(&notebook, &split_view, &sidebar, &params),
            _ => start_quick_ssh(&notebook, &params, &terminal_settings),
        }

        window_clone.close();
    });

    quick_window.present();
}
