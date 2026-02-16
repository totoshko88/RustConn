//! Edit dialogs for main window
//!
//! This module contains functions for editing connections and groups,
//! showing connection details, and quick connect dialog.

use super::MainWindow;
use crate::alert;
use crate::dialogs::ConnectionDialog;
use crate::embedded_rdp::{EmbeddedRdpWidget, RdpConfig as EmbeddedRdpConfig};
use crate::sidebar::ConnectionSidebar;
use crate::split_view::SplitViewBridge;
use crate::state::SharedAppState;
use crate::terminal::TerminalNotebook;
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

        let dialog = ConnectionDialog::new(Some(&window.clone().upcast()), state.clone());
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

        // Populate variable dropdown with secret global variables
        // Must be before set_connection so variable selection works
        {
            let state_ref = state.borrow();
            let global_vars = state_ref.settings().global_variables.clone();
            dialog.set_global_variables(&global_vars);
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
            let groups: Vec<rustconn_core::models::ConnectionGroup> =
                state_ref.list_groups().iter().cloned().cloned().collect();
            dialog.connect_password_load_button_with_groups(
                settings.secrets.kdbx_enabled,
                settings.secrets.kdbx_path.clone(),
                settings
                    .secrets
                    .kdbx_password
                    .as_ref()
                    .map(|p| p.expose_secret().to_string()),
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
                            if password_source == PasswordSource::Vault {
                                if let Some(pwd) = password {
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
        .default_width(450)
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
                                                tracing::warn!(
                                                    "Failed to rename vault entry: {}",
                                                    e
                                                );
                                            }
                                        },
                                    );
                                }
                                rustconn_core::models::PasswordSource::Variable(_)
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
        .default_width(450)
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
    let mut available_groups: Vec<(Uuid, String, u32)> = Vec::new(); // (id, name, depth)
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

    // Helper to calculate depth of a group
    let get_depth = |gid: Uuid| -> u32 {
        let mut depth = 0u32;
        let mut current = gid;
        while let Some(g) = state_ref.get_group(current) {
            if let Some(p) = g.parent_id {
                depth += 1;
                current = p;
            } else {
                break;
            }
        }
        depth
    };

    for g in all_groups {
        if g.id == group_id {
            continue;
        }
        if is_descendant(g.id) {
            continue;
        }

        // Get full path for sorting, but store name and depth for display
        let path = state_ref
            .get_group_path(g.id)
            .unwrap_or_else(|| g.name.clone());
        let depth = get_depth(g.id);
        available_groups.push((g.id, path, depth));
    }
    drop(state_ref);

    // Sort by the full path to maintain hierarchy order
    available_groups.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));

    let mut group_ids: Vec<Option<Uuid>> = vec![None];
    let mut strings: Vec<String> = vec!["(None - Root Level)".to_string()];
    let mut preselected_index = 0u32;

    for (id, path, depth) in available_groups {
        // Extract just the group name (last segment of path)
        let name = path.rsplit('/').next().unwrap_or(&path);
        // Add indentation based on depth using Unicode box-drawing chars
        let indent = "    ".repeat(depth as usize);
        let prefix = if depth > 0 { "└ " } else { "" };
        let display = format!("{indent}{prefix}{name}");
        strings.push(display);
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

    // Use ComboRow for better handling of long group paths
    let parent_row = adw::ComboRow::builder()
        .title("Parent")
        .subtitle("Moving a group moves all its content")
        .model(&string_list)
        .selected(preselected_index)
        .build();
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
    let password_source_list =
        gtk4::StringList::new(&["Prompt", "Vault", "Variable", "Inherit", "None"]);
    let password_source_dropdown = gtk4::DropDown::builder()
        .model(&password_source_list)
        .valign(gtk4::Align::Center)
        .build();
    // Set initial selection based on group's password_source
    let initial_source_idx = match group.password_source {
        Some(PasswordSource::Prompt) => 0,
        Some(PasswordSource::Vault) => 1,
        Some(PasswordSource::Variable(_)) => 2,
        Some(PasswordSource::Inherit) => 3,
        Some(PasswordSource::None) | None => 4,
    };
    password_source_dropdown.set_selected(initial_source_idx);

    let password_source_row = adw::ActionRow::builder().title("Password").build();
    password_source_row.add_suffix(&password_source_dropdown);
    credentials_group.add(&password_source_row);

    // Password Value entry with visibility toggle and load button
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
    let password_load_btn = gtk4::Button::builder()
        .icon_name("folder-symbolic")
        .tooltip_text("Load password from vault")
        .valign(gtk4::Align::Center)
        .build();

    let password_value_row = adw::ActionRow::builder().title("Value").build();
    password_value_row.add_suffix(&password_entry);
    password_value_row.add_suffix(&password_visibility_btn);
    password_value_row.add_suffix(&password_load_btn);
    credentials_group.add(&password_value_row);

    // Show/hide password value row based on source selection
    // Show for KeePass(1), Keyring(2), Bitwarden(3), 1Password(4)
    let show_value = matches!(initial_source_idx, 1..=4);
    password_value_row.set_visible(show_value);

    // Connect password source dropdown to show/hide value row
    let value_row_clone = password_value_row.clone();
    password_source_dropdown.connect_selected_notify(move |dropdown| {
        let selected = dropdown.selected();
        let show = matches!(selected, 1..=4);
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

    // Connect password load button - loads password from configured vault
    let password_entry_for_load = password_entry.clone();
    let password_source_for_load = password_source_dropdown.clone();
    let state_for_load = state.clone();
    let window_for_load = group_window.clone();
    let group_name_for_load = group.name.clone();
    let group_id_for_load = group_id;
    password_load_btn.connect_clicked(move |btn| {
        let password_source_idx = password_source_for_load.selected();

        // Get settings and group path for vault access
        let state_ref = state_for_load.borrow();
        let settings = state_ref.settings().clone();
        let groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
        let grp = state_ref.get_group(group_id_for_load).cloned();
        drop(state_ref);

        let lookup_key = if let Some(ref g) = grp {
            g.id.to_string()
        } else {
            format!("group:{}", group_name_for_load.replace('/', "-"))
        };

        let group_path = if let Some(ref g) = grp {
            rustconn_core::secret::KeePassHierarchy::build_group_entry_path(g, &groups)
        } else {
            format!("RustConn/Groups/{}", group_name_for_load)
        };

        let password_entry_clone = password_entry_for_load.clone();
        let window_clone = window_for_load.clone();
        let btn_clone = btn.clone();

        btn.set_sensitive(false);
        btn.set_icon_name("content-loading-symbolic");

        match password_source_idx {
            1 => {
                // KeePass
                if !settings.secrets.kdbx_enabled {
                    alert::show_validation_error(&window_clone, "KeePass is not enabled");
                    btn_clone.set_sensitive(true);
                    btn_clone.set_icon_name("folder-symbolic");
                    return;
                }
                let Some(kdbx_path) = settings.secrets.kdbx_path.clone() else {
                    alert::show_validation_error(&window_clone, "KeePass database not configured");
                    btn_clone.set_sensitive(true);
                    btn_clone.set_icon_name("folder-symbolic");
                    return;
                };
                let key_file = settings.secrets.kdbx_key_file.clone();

                crate::utils::spawn_blocking_with_callback(
                    move || {
                        let key_file_path = key_file.as_ref().map(std::path::Path::new);
                        rustconn_core::secret::KeePassStatus::get_password_from_kdbx_with_key(
                            std::path::Path::new(&kdbx_path),
                            None,
                            key_file_path,
                            &group_path,
                            None, // No protocol for groups
                        )
                    },
                    move |result: Result<Option<String>, String>| {
                        btn_clone.set_sensitive(true);
                        btn_clone.set_icon_name("folder-symbolic");
                        match result {
                            Ok(Some(pwd)) => {
                                password_entry_clone.set_text(&pwd);
                            }
                            Ok(None) => {
                                alert::show_validation_error(
                                    &window_clone,
                                    "No password found for this group",
                                );
                            }
                            Err(e) => {
                                tracing::error!("Failed to load password: {}", e);
                                alert::show_error(&window_clone, "Load Error", &e);
                            }
                        }
                    },
                );
            }
            2 => {
                // Keyring
                crate::utils::spawn_blocking_with_callback(
                    move || {
                        use rustconn_core::secret::SecretBackend;
                        let backend = rustconn_core::secret::LibSecretBackend::new("rustconn");
                        crate::async_utils::with_runtime(|rt| {
                            rt.block_on(backend.retrieve(&lookup_key))
                                .map_err(|e| format!("{e}"))
                        })?
                    },
                    move |result: Result<Option<Credentials>, String>| {
                        btn_clone.set_sensitive(true);
                        btn_clone.set_icon_name("folder-symbolic");
                        match result {
                            Ok(Some(creds)) => {
                                if let Some(pwd) = creds.expose_password() {
                                    password_entry_clone.set_text(pwd);
                                } else {
                                    alert::show_validation_error(
                                        &window_clone,
                                        "No password found for this group",
                                    );
                                }
                            }
                            Ok(None) => {
                                alert::show_validation_error(
                                    &window_clone,
                                    "No password found for this group",
                                );
                            }
                            Err(e) => {
                                tracing::error!("Failed to load password: {}", e);
                                alert::show_error(&window_clone, "Load Error", &e);
                            }
                        }
                    },
                );
            }
            3 => {
                // Bitwarden
                crate::utils::spawn_blocking_with_callback(
                    move || {
                        use rustconn_core::secret::SecretBackend;
                        let backend = rustconn_core::secret::BitwardenBackend::new();
                        crate::async_utils::with_runtime(|rt| {
                            rt.block_on(backend.retrieve(&lookup_key))
                                .map_err(|e| format!("{e}"))
                        })?
                    },
                    move |result: Result<Option<Credentials>, String>| {
                        btn_clone.set_sensitive(true);
                        btn_clone.set_icon_name("folder-symbolic");
                        match result {
                            Ok(Some(creds)) => {
                                if let Some(pwd) = creds.expose_password() {
                                    password_entry_clone.set_text(pwd);
                                } else {
                                    alert::show_validation_error(
                                        &window_clone,
                                        "No password found for this group",
                                    );
                                }
                            }
                            Ok(None) => {
                                alert::show_validation_error(
                                    &window_clone,
                                    "No password found for this group",
                                );
                            }
                            Err(e) => {
                                tracing::error!("Failed to load password: {}", e);
                                alert::show_error(&window_clone, "Load Error", &e);
                            }
                        }
                    },
                );
            }
            _ => {
                btn.set_sensitive(true);
                btn.set_icon_name("folder-symbolic");
                alert::show_validation_error(
                    &window_clone,
                    "Select KeePass, Keyring, or Bitwarden to load password",
                );
            }
        }
    });

    let domain_row = adw::EntryRow::builder()
        .title("Domain")
        .text(group.domain.as_deref().unwrap_or_default())
        .build();
    credentials_group.add(&domain_row);

    content.append(&credentials_group);

    // === Description Section ===
    let description_group = adw::PreferencesGroup::builder()
        .title("Description")
        .description("Notes, contacts, project info")
        .build();

    let description_view = gtk4::TextView::builder()
        .wrap_mode(gtk4::WrapMode::Word)
        .accepts_tab(false)
        .top_margin(8)
        .bottom_margin(8)
        .left_margin(8)
        .right_margin(8)
        .build();
    description_view
        .buffer()
        .set_text(group.description.as_deref().unwrap_or_default());

    let description_scroll = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .min_content_height(144)
        .hexpand(true)
        .child(&description_view)
        .build();
    description_scroll.add_css_class("card");

    description_group.add(&description_scroll);
    content.append(&description_group);

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
    let parent_row_clone = parent_row;
    let description_buffer = description_view.buffer();
    let old_name = group.name;

    save_btn.connect_clicked(move |_| {
        let new_name = name_row_clone.text().to_string();
        if new_name.trim().is_empty() {
            alert::show_validation_error(&window_clone, "Group name cannot be empty");
            return;
        }

        let selected_idx = parent_row_clone.selected() as usize;
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
            1 => PasswordSource::Vault,
            2 => PasswordSource::Variable(String::new()),
            3 => PasswordSource::Inherit,
            _ => PasswordSource::None,
        };

        // Password is relevant for Vault only
        let has_new_password = !password.is_empty() && password_source_idx == 1;

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

                // Get description from text buffer
                let (start, end) = description_buffer.bounds();
                let desc_text = description_buffer.text(&start, &end, false).to_string();
                updated.description = if desc_text.trim().is_empty() {
                    None
                } else {
                    Some(desc_text)
                };

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

                updated.password_source = Some(new_password_source.clone());

                if let Err(e) = state_mut
                    .connection_manager()
                    .update_group(group_id, updated)
                {
                    alert::show_error(&window_clone, "Error", &format!("{e}"));
                    return;
                }

                // Save password if provided and source requires it
                if has_new_password {
                    // Get group path for hierarchical storage
                    let groups: Vec<_> = state_mut.list_groups().into_iter().cloned().collect();
                    let grp = state_mut.get_group(group_id).cloned();
                    let settings = state_mut.settings().clone();

                    if let Some(g) = grp {
                        let group_path =
                            rustconn_core::secret::KeePassHierarchy::build_group_entry_path(
                                &g, &groups,
                            );
                        let lookup_key = g.id.to_string();

                        if new_password_source == PasswordSource::Vault {
                            // Save to vault using configured backend
                            crate::state::save_group_password_to_vault(
                                &settings,
                                &group_path,
                                &lookup_key,
                                &username,
                                &password,
                            );
                        }
                    }
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
    );
    notebook.spawn_telnet(
        session_id,
        &params.host,
        params.port,
        &[],
        rustconn_core::TelnetBackspaceSends::Automatic,
        rustconn_core::TelnetDeleteSends::Automatic,
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
        .default_width(450)
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
    let protocol_list = gtk4::StringList::new(&["SSH", "RDP", "VNC", "Telnet"]);
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
                3 => 23.0,   // Telnet
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

        #[allow(clippy::cast_sign_loss)]
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
            3 => start_quick_telnet(&notebook, &params, &terminal_settings),
            _ => start_quick_ssh(&notebook, &params, &terminal_settings),
        }

        window_clone.close();
    });

    quick_window.present();
}
