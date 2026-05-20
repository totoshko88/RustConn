//! Edit dialogs for main window
//!
//! This module contains functions for editing connections and groups,
//! showing connection details, and quick connect dialog.

use super::MainWindow;
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
use rustconn_core::models::{Credentials, PasswordSource, SshAuthMethod};
use rustconn_core::sync::SyncMode;
use secrecy::ExposeSecret;
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
                                    pwd.expose_secret(),
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
    let rename_window = adw::Window::builder()
        .title(if is_group {
            i18n("Rename Group")
        } else {
            i18n("Rename Connection")
        })
        .modal(true)
        .default_width(450)
        .resizable(false)
        .build();
    rename_window.set_transient_for(Some(window));

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
    rename_window.set_content(Some(&toolbar_view));

    // Save button
    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let window_clone = rename_window.clone();
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
        .title(i18n("Edit Group"))
        .modal(true)
        .default_width(600)
        .default_height(750)
        .resizable(true)
        .build();
    // Fix minimum width to prevent the window from resizing when content
    // changes (e.g., adding/removing expect rules). The minimum matches
    // the Clamp maximum_size so the layout is always stable.
    group_window.set_size_request(600, -1);
    group_window.set_transient_for(Some(window));

    let header = adw::HeaderBar::new();
    let save_btn = gtk4::Button::from_icon_name("media-floppy-symbolic");
    save_btn.set_tooltip_text(Some(&i18n("Save")));
    save_btn.update_property(&[gtk4::accessible::Property::Label(&i18n("Save"))]);
    save_btn.add_css_class("suggested-action");
    header.pack_start(&save_btn);

    // Scrollable content with clamp
    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .tightening_threshold(600)
        .build();

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);

    clamp.set_child(Some(&content));

    let scrolled = gtk4::ScrolledWindow::builder()
        .hscrollbar_policy(gtk4::PolicyType::Never)
        .vscrollbar_policy(gtk4::PolicyType::Automatic)
        .vexpand(true)
        .child(&clamp)
        .build();
    // Overlay scrolling draws the scrollbar on top of content instead of
    // beside it, preventing layout width shifts when the scrollbar appears
    // or disappears as content height changes.
    scrolled.set_overlay_scrolling(true);

    // Use ToolbarView for proper adw::Window layout
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&scrolled));
    group_window.set_content(Some(&toolbar_view));

    // === Group Details ===
    let details_group = adw::PreferencesGroup::builder()
        .title(i18n("Group Details"))
        .build();

    // Name entry using PreferencesGroup with EntryRow
    let name_row = adw::EntryRow::builder()
        .title(i18n("Name"))
        .text(&group.name)
        .build();
    details_group.add(&name_row);

    // Group icon using EntryRow
    let icon_row = adw::EntryRow::builder()
        .title(i18n("Icon"))
        .text(group.icon.as_deref().unwrap_or(""))
        .build();
    icon_row.set_tooltip_text(Some(&i18n(
        "Enter an emoji (e.g. 🇺🇦) or GTK icon name (e.g. starred-symbolic)",
    )));
    details_group.add(&icon_row);

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
    available_groups.sort_by_key(|a| a.1.to_lowercase());

    let mut group_ids: Vec<Option<Uuid>> = vec![None];
    let mut strings: Vec<String> = vec![i18n("(None - Root Level)")];
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
        .title(i18n("Parent"))
        .subtitle(i18n("Moving a group moves all its content"))
        .model(&string_list)
        .selected(preselected_index)
        .build();
    details_group.add(&parent_row);

    content.append(&details_group);

    // === Inheritable Credentials (collapsible with enable switch) ===
    let credentials_group = adw::PreferencesGroup::new();

    let credentials_expander = adw::ExpanderRow::builder()
        .title(i18n("Default Credentials"))
        .subtitle(i18n("Credentials inherited by connections in this group"))
        .show_enable_switch(true)
        .build();
    // Enable and expand if any credential field is set
    let has_credentials =
        group.username.is_some() || group.domain.is_some() || group.password_source.is_some();
    credentials_expander.set_enable_expansion(has_credentials);
    credentials_expander.set_expanded(has_credentials);
    credentials_group.add(&credentials_expander);

    let username_row = adw::EntryRow::builder()
        .title(i18n("Username"))
        .text(group.username.as_deref().unwrap_or_default())
        .build();
    credentials_expander.add_row(&username_row);

    // Password Source dropdown
    let password_source_list = gtk4::StringList::new(&[
        &i18n("Prompt"),
        &i18n("Vault"),
        &i18n("Variable"),
        &i18n("Inherit"),
        &i18n("None"),
    ]);
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
        Some(PasswordSource::Script(_)) => 5,
        Some(PasswordSource::None) | None => 4,
    };
    password_source_dropdown.set_selected(initial_source_idx);

    let password_source_row = adw::ActionRow::builder().title(i18n("Password")).build();
    password_source_row.add_suffix(&password_source_dropdown);
    credentials_expander.add_row(&password_source_row);

    // Password Value entry with visibility toggle and load button
    let password_entry = gtk4::Entry::builder()
        .placeholder_text(i18n("Password value"))
        .visibility(false)
        .hexpand(true)
        .build();
    let password_visibility_btn = gtk4::Button::builder()
        .icon_name("view-reveal-symbolic")
        .tooltip_text(i18n("Show/hide password"))
        .valign(gtk4::Align::Center)
        .build();
    password_visibility_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Toggle password visibility",
    ))]);
    let password_load_btn = gtk4::Button::builder()
        .icon_name("folder-symbolic")
        .tooltip_text(i18n("Load password from vault"))
        .valign(gtk4::Align::Center)
        .build();
    password_load_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Load password from vault",
    ))]);

    let password_value_row = adw::ActionRow::builder().title(i18n("Value")).build();
    password_value_row.add_suffix(&password_entry);
    password_value_row.add_suffix(&password_visibility_btn);
    password_value_row.add_suffix(&password_load_btn);
    credentials_expander.add_row(&password_value_row);

    // Variable dropdown row (visible for Variable source)
    let variable_name_list = gtk4::StringList::new(&[]);
    let variable_dropdown = gtk4::DropDown::builder()
        .model(&variable_name_list)
        .valign(gtk4::Align::Center)
        .build();
    let variable_action_row = adw::ActionRow::builder().title(i18n("Variable")).build();
    variable_action_row.add_suffix(&variable_dropdown);
    credentials_expander.add_row(&variable_action_row);

    // Accessible label relations for screen readers
    password_source_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        password_source_row.upcast_ref(),
    ])]);
    password_entry.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        password_value_row.upcast_ref(),
    ])]);
    variable_dropdown.update_relation(&[gtk4::accessible::Relation::LabelledBy(&[
        variable_action_row.upcast_ref(),
    ])]);

    // Populate variable dropdown with secret global variables
    {
        let state_ref = state.borrow();
        let global_vars = state_ref.settings().global_variables.clone();
        if let Some(sl) = variable_name_list.downcast_ref::<gtk4::StringList>() {
            for var in &global_vars {
                if var.is_secret {
                    sl.append(&var.name);
                }
            }
        }
        drop(state_ref);

        // Pre-select the matching variable if group uses Variable source
        if let Some(PasswordSource::Variable(ref var_name)) = group.password_source {
            for i in 0..variable_name_list.n_items() {
                if variable_name_list.string(i).is_some_and(|s| s == *var_name) {
                    variable_dropdown.set_selected(i);
                    break;
                }
            }
        }
    }

    // Show/hide rows based on source selection
    // Vault(1) → password value row; Variable(2) → variable dropdown row
    let show_password = initial_source_idx == 1;
    let show_variable = initial_source_idx == 2;
    password_value_row.set_visible(show_password);
    variable_action_row.set_visible(show_variable);

    // Connect password source dropdown to show/hide value and variable rows
    let value_row_clone = password_value_row.clone();
    let variable_row_clone = variable_action_row.clone();
    password_source_dropdown.connect_selected_notify(move |dropdown| {
        let selected = dropdown.selected();
        value_row_clone.set_visible(selected == 1);
        variable_row_clone.set_visible(selected == 2);
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

        // Index 1 = "Vault" — dispatch to the configured default backend
        if password_source_idx != 1 {
            btn.set_sensitive(true);
            btn.set_icon_name("folder-symbolic");
            alert::show_validation_error(&window_clone, &i18n("Select Vault to load password"));
            return;
        }

        let backend_type = crate::state::select_backend_for_load(&settings.secrets);

        // KeePass/KDBX uses direct file access with group_path
        if matches!(
            backend_type,
            rustconn_core::config::SecretBackendType::KdbxFile
        ) {
            let Some(kdbx_path) = settings.secrets.kdbx_path.clone() else {
                alert::show_validation_error(
                    &window_clone,
                    &i18n("KeePass database not configured"),
                );
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
                move |result: rustconn_core::error::SecretResult<Option<secrecy::SecretString>>| {
                    btn_clone.set_sensitive(true);
                    btn_clone.set_icon_name("folder-symbolic");
                    match result {
                        Ok(Some(pwd)) => {
                            use secrecy::ExposeSecret;
                            password_entry_clone.set_text(pwd.expose_secret());
                        }
                        Ok(None) => {
                            alert::show_validation_error(
                                &window_clone,
                                &i18n("No password found for this group"),
                            );
                        }
                        Err(e) => {
                            let msg = e.to_string();
                            tracing::error!("Failed to load group password: {}", msg);
                            alert::show_error(&window_clone, &i18n("Load Error"), &msg);
                        }
                    }
                },
            );
        } else {
            // All other backends — dispatch via dispatch_vault_op
            let secret_settings = settings.secrets.clone();
            crate::utils::spawn_blocking_with_callback(
                move || {
                    crate::state::dispatch_vault_op(
                        &secret_settings,
                        &lookup_key,
                        crate::state::VaultOp::Retrieve,
                    )
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
                                    &i18n("No password found for this group"),
                                );
                            }
                        }
                        Ok(None) => {
                            alert::show_validation_error(
                                &window_clone,
                                &i18n("No password found for this group"),
                            );
                        }
                        Err(e) => {
                            tracing::error!("Failed to load group password: {}", e);
                            alert::show_error(&window_clone, &i18n("Load Error"), &e);
                        }
                    }
                },
            );
        }
    });

    let domain_row = adw::EntryRow::builder()
        .title(i18n("Domain"))
        .text(group.domain.as_deref().unwrap_or_default())
        .build();
    credentials_expander.add_row(&domain_row);

    content.append(&credentials_group);

    // === SSH Settings Section (progressive disclosure per GNOME HIG) ===
    let ssh_settings_group = adw::PreferencesGroup::new();

    let ssh_expander = adw::ExpanderRow::builder()
        .title(i18n("SSH Settings"))
        .subtitle(i18n("SSH settings inherited by connections in this group"))
        .show_enable_switch(true)
        .build();
    let has_ssh_settings = group.ssh_auth_method.is_some()
        || group.ssh_key_path.is_some()
        || group.ssh_proxy_jump.is_some()
        || group.ssh_agent_socket.is_some();
    ssh_expander.set_enable_expansion(has_ssh_settings);
    ssh_expander.set_expanded(has_ssh_settings);
    ssh_settings_group.add(&ssh_expander);

    // Confirm before clearing SSH settings when the enable switch is toggled off.
    // Per GNOME HIG, destructive actions should require confirmation.
    {
        let expander = ssh_expander.clone();
        let window_for_confirm = group_window.clone();
        // Guard flag to prevent recursive signal triggering when we
        // programmatically set_enable_expansion(false) from the response handler.
        let clearing_in_progress = Rc::new(std::cell::Cell::new(false));
        let clearing_flag = clearing_in_progress.clone();
        ssh_expander.connect_enable_expansion_notify(move |row| {
            if row.enables_expansion() {
                return; // Enabling — no confirmation needed
            }
            // Skip if we're programmatically clearing from the confirmation handler
            if clearing_in_progress.get() {
                return;
            }
            // Check if any SSH field has a value
            let has_data = row.first_child().is_some(); // rows exist
            if !has_data {
                return;
            }
            // Re-enable immediately to prevent data loss; show confirmation
            row.set_enable_expansion(true);
            let confirm = adw::AlertDialog::builder()
                .heading(i18n("Clear SSH Settings?"))
                .body(i18n(
                    "Disabling will clear all SSH settings for this group. This cannot be undone.",
                ))
                .close_response("cancel")
                .default_response("cancel")
                .build();
            confirm.add_response("cancel", &i18n("Keep"));
            confirm.add_response("clear", &i18n("Clear"));
            confirm.set_response_appearance("clear", adw::ResponseAppearance::Destructive);

            let expander_c = expander.clone();
            let flag = clearing_flag.clone();
            confirm.connect_response(None, move |_, response| {
                if response == "clear" {
                    flag.set(true);
                    expander_c.set_enable_expansion(false);
                    expander_c.set_expanded(false);
                    flag.set(false);
                }
            });
            confirm.present(Some(&window_for_confirm));
        });
    }

    // SSH Auth Method dropdown (None / Password / PublicKey / Agent / KeyboardInteractive / SecurityKey)
    let auth_method_list = gtk4::StringList::new(&[
        &i18n("None"),
        &i18n("Password"),
        &i18n("Public Key"),
        &i18n("Agent"),
        &i18n("Keyboard Interactive"),
        &i18n("Security Key"),
    ]);
    let auth_method_row = adw::ComboRow::builder()
        .title(i18n("SSH Authentication Method"))
        .model(&auth_method_list)
        .build();
    let initial_auth_idx: u32 = match group.ssh_auth_method {
        None => 0,
        Some(SshAuthMethod::Password) => 1,
        Some(SshAuthMethod::PublicKey) => 2,
        Some(SshAuthMethod::Agent) => 3,
        Some(SshAuthMethod::KeyboardInteractive) => 4,
        Some(SshAuthMethod::SecurityKey) => 5,
    };
    auth_method_row.set_selected(initial_auth_idx);
    ssh_expander.add_row(&auth_method_row);

    // SSH Key Path with file chooser suffix button
    let ssh_key_path_row = adw::EntryRow::builder()
        .title(i18n("SSH Key Path"))
        .text(
            group
                .ssh_key_path
                .as_ref()
                .map_or("", |p| p.to_str().unwrap_or("")),
        )
        .build();
    let ssh_key_browse_btn = gtk4::Button::from_icon_name("document-open-symbolic");
    ssh_key_browse_btn.set_valign(gtk4::Align::Center);
    ssh_key_browse_btn.set_tooltip_text(Some(&i18n("Select SSH key file")));
    ssh_key_browse_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
        "Select SSH key file",
    ))]);
    ssh_key_path_row.add_suffix(&ssh_key_browse_btn);
    ssh_expander.add_row(&ssh_key_path_row);

    // Connect file chooser button
    let ssh_key_path_row_clone = ssh_key_path_row.clone();
    let window_for_chooser = group_window.clone();
    ssh_key_browse_btn.connect_clicked(move |_| {
        let file_dialog = gtk4::FileDialog::builder()
            .title(i18n("Select SSH Key"))
            .modal(true)
            .build();

        // Set initial folder to ~/.ssh if it exists
        if let Some(home) = std::env::var_os("HOME") {
            let ssh_dir = std::path::PathBuf::from(home).join(".ssh");
            if ssh_dir.exists() {
                let gio_file = gtk4::gio::File::for_path(&ssh_dir);
                file_dialog.set_initial_folder(Some(&gio_file));
            }
        }

        let entry = ssh_key_path_row_clone.clone();
        let parent = window_for_chooser.clone();
        file_dialog.open(Some(&parent), gtk4::gio::Cancellable::NONE, move |result| {
            if let Ok(file) = result
                && let Some(path) = file.path()
            {
                let stable_path = if rustconn_core::is_flatpak()
                    && rustconn_core::is_portal_path(&path)
                {
                    rustconn_core::copy_key_to_flatpak_ssh(&path).unwrap_or_else(|| path.clone())
                } else {
                    path
                };
                entry.set_text(&stable_path.to_string_lossy());
            }
        });
    });

    // SSH Jump Host dropdown — select from existing SSH connections
    let state_ref = state.borrow();
    let mut jump_host_data: Vec<(Option<Uuid>, String)> = vec![(None, i18n("(None)"))];
    let mut ssh_connections: Vec<&rustconn_core::Connection> = state_ref
        .list_connections()
        .into_iter()
        .filter(|c| c.protocol == rustconn_core::models::ProtocolType::Ssh)
        .collect();
    ssh_connections.sort_by_key(|c| c.name.to_lowercase());
    for conn in &ssh_connections {
        let label = if conn.name == conn.host {
            conn.name.clone()
        } else {
            format!("{} ({})", conn.name, conn.host)
        };
        let label = if label.chars().count() > 50 {
            let truncated: String = label.chars().take(49).collect();
            format!("{truncated}…")
        } else {
            label
        };
        jump_host_data.push((Some(conn.id), label));
    }
    drop(state_ref);

    let jump_host_strings: Vec<&str> = jump_host_data.iter().map(|(_, s)| s.as_str()).collect();
    let jump_host_model = gtk4::StringList::new(&jump_host_strings);
    let ssh_jump_host_dropdown = gtk4::DropDown::builder()
        .model(&jump_host_model)
        .valign(gtk4::Align::Center)
        .enable_search(true)
        .build();
    ssh_jump_host_dropdown.set_size_request(200, -1);
    ssh_jump_host_dropdown.set_hexpand(false);

    // Pre-select the current jump host
    let mut preselected_jump_idx = 0u32;
    if let Some(jump_id) = group.ssh_jump_host_id {
        for (i, (id, _)) in jump_host_data.iter().enumerate() {
            if *id == Some(jump_id) {
                preselected_jump_idx = i as u32;
                break;
            }
        }
    }
    ssh_jump_host_dropdown.set_selected(preselected_jump_idx);

    let ssh_jump_host_row = adw::ActionRow::builder()
        .title(i18n("Jump Host"))
        .subtitle(i18n("Connect via another SSH connection"))
        .build();
    ssh_jump_host_row.add_suffix(&ssh_jump_host_dropdown);
    ssh_expander.add_row(&ssh_jump_host_row);

    // SSH Proxy Jump text field (manual entry, fallback when no saved connection)
    let ssh_proxy_jump_row = adw::EntryRow::builder()
        .title(i18n("SSH Proxy Jump"))
        .text(group.ssh_proxy_jump.as_deref().unwrap_or_default())
        .build();
    ssh_proxy_jump_row.set_tooltip_text(Some(&i18n(
        "Manual ProxyJump (-J) — used when Jump Host is (None)",
    )));
    ssh_expander.add_row(&ssh_proxy_jump_row);

    // SSH Agent Socket text field
    let ssh_agent_socket_row = adw::EntryRow::builder()
        .title(i18n("SSH Agent Socket"))
        .text(group.ssh_agent_socket.as_deref().unwrap_or_default())
        .build();
    ssh_expander.add_row(&ssh_agent_socket_row);

    content.append(&ssh_settings_group);

    // --- Dynamic visibility of SSH detail rows based on auth method ---
    // Helper: update visibility of SSH fields based on selected auth method index
    let update_ssh_fields_visibility = {
        let key_path = ssh_key_path_row.clone();
        let proxy_jump = ssh_proxy_jump_row.clone();
        let jump_host_row = ssh_jump_host_row.clone();
        let agent_socket = ssh_agent_socket_row.clone();
        move |selected: u32| {
            // 0=None, 1=Password, 2=PublicKey, 3=Agent, 4=KeyboardInteractive, 5=SecurityKey
            let method_selected = selected != 0;
            let needs_key = matches!(selected, 2 | 5); // PublicKey or SecurityKey
            let needs_agent = selected == 3; // Agent

            key_path.set_visible(needs_key);
            jump_host_row.set_visible(method_selected);
            proxy_jump.set_visible(method_selected);
            agent_socket.set_visible(needs_agent);
        }
    };

    // Apply initial visibility
    update_ssh_fields_visibility(initial_auth_idx);

    // React to auth method changes
    let update_fn = update_ssh_fields_visibility.clone();
    auth_method_row.connect_selected_notify(move |row| {
        update_fn(row.selected());
    });

    // === Cloud Sync Section (root groups only) ===
    let sync_mode_list =
        gtk4::StringList::new(&[&i18n("Not synced"), &i18n("Master"), &i18n("Import")]);
    let sync_mode_row = adw::ComboRow::builder()
        .title(i18n("Cloud Sync"))
        .model(&sync_mode_list)
        .build();
    let initial_sync_idx: u32 = match group.sync_mode {
        SyncMode::None => 0,
        SyncMode::Master => 1,
        SyncMode::Import => 2,
    };
    sync_mode_row.set_selected(initial_sync_idx);

    // Show confirmation dialog when switching to Master mode
    let previous_sync_idx: Rc<std::cell::Cell<u32>> =
        Rc::new(std::cell::Cell::new(initial_sync_idx));
    let prev_idx_for_signal = previous_sync_idx.clone();
    let state_for_sync = state.clone();
    let group_window_for_sync = group_window.clone();
    sync_mode_row.connect_selected_notify(move |row| {
        let selected = row.selected();
        let prev = prev_idx_for_signal.get();

        // Only show confirmation when changing TO Master from non-Master
        if selected == 1 && prev != 1 {
            let state_ref = state_for_sync.borrow();
            let sync_dir = state_ref.settings().sync.sync_dir.clone();
            drop(state_ref);

            if let Some(dir) = sync_dir {
                // sync_dir configured — show confirmation dialog
                let sync_dir_display = dir.display().to_string();
                show_enable_master_confirmation(
                    &group_window_for_sync,
                    &sync_dir_display,
                    row,
                    &prev_idx_for_signal,
                );
            } else {
                // sync_dir not configured — show setup dialog with folder chooser
                show_sync_setup_dialog(
                    &group_window_for_sync,
                    &state_for_sync,
                    row,
                    &prev_idx_for_signal,
                );
            }
        } else {
            // For non-Master selections, just track the new index
            prev_idx_for_signal.set(selected);
        }
    });

    let sync_group_widget = adw::PreferencesGroup::builder()
        .title(i18n("Cloud Sync"))
        .build();
    sync_group_widget.add(&sync_mode_row);

    // Show sync file path and last synced time for synced groups
    if group.sync_mode != SyncMode::None {
        if let Some(ref sync_file) = group.sync_file {
            let sync_dir_display = state
                .borrow()
                .settings()
                .sync
                .sync_dir
                .as_ref()
                .map(|d| d.join(sync_file).display().to_string())
                .unwrap_or_else(|| sync_file.clone());
            let path_row = adw::ActionRow::builder()
                .title(i18n("Sync File"))
                .subtitle(&sync_dir_display)
                .build();
            sync_group_widget.add(&path_row);
        }
        if let Some(last_synced) = group.last_synced_at {
            let time_str = last_synced.format("%Y-%m-%d %H:%M:%S").to_string();
            let synced_row = adw::ActionRow::builder()
                .title(i18n("Last Synced"))
                .subtitle(&time_str)
                .build();
            sync_group_widget.add(&synced_row);
        }
    }

    // Only show for root groups — subgroups inherit sync mode from their root
    let is_root_group = group.parent_id.is_none();
    sync_group_widget.set_visible(is_root_group);

    content.append(&sync_group_widget);

    // Hide Cloud Sync section when parent changes to non-root
    let sync_group_for_parent = sync_group_widget.clone();
    parent_row.connect_selected_notify(move |row| {
        // Index 0 = "(None - Root Level)" means this group becomes/stays root
        sync_group_for_parent.set_visible(row.selected() == 0);
    });

    // === Dynamic Folder Section ===
    let dynamic_expander = adw::ExpanderRow::builder()
        .title(i18n("Dynamic Folder"))
        .subtitle(i18n("Generate connections from a script"))
        .show_enable_switch(true)
        .expanded(group.dynamic_folder.is_some())
        .enable_expansion(group.dynamic_folder.is_some())
        .build();

    let dynamic_script_row = adw::EntryRow::builder()
        .title(i18n("Script"))
        .text(
            group
                .dynamic_folder
                .as_ref()
                .map_or("", |df| df.script.as_str()),
        )
        .build();
    dynamic_script_row.set_tooltip_text(Some(&i18n("Shell command executed via sh -c")));
    dynamic_expander.add_row(&dynamic_script_row);

    let dynamic_workdir_row = adw::EntryRow::builder()
        .title(i18n("Working Directory"))
        .text(
            group
                .dynamic_folder
                .as_ref()
                .and_then(|df| df.working_directory.as_ref())
                .map_or("", |p| p.to_str().unwrap_or("")),
        )
        .build();
    dynamic_expander.add_row(&dynamic_workdir_row);

    let dynamic_timeout_row = adw::SpinRow::builder()
        .title(i18n("Timeout (seconds)"))
        .adjustment(&gtk4::Adjustment::new(
            group
                .dynamic_folder
                .as_ref()
                .map_or(30.0, |df| df.timeout_secs as f64),
            1.0,
            300.0,
            1.0,
            10.0,
            0.0,
        ))
        .build();
    dynamic_expander.add_row(&dynamic_timeout_row);

    let dynamic_refresh_row = adw::SpinRow::builder()
        .title(i18n("Refresh Interval (seconds)"))
        .subtitle(i18n("0 = manual only"))
        .adjustment(&gtk4::Adjustment::new(
            group
                .dynamic_folder
                .as_ref()
                .and_then(|df| df.refresh_interval_secs)
                .map_or(0.0, |s| s as f64),
            0.0,
            86400.0,
            10.0,
            60.0,
            0.0,
        ))
        .build();
    dynamic_expander.add_row(&dynamic_refresh_row);

    let dynamic_group = adw::PreferencesGroup::new();
    dynamic_group.add(&dynamic_expander);
    content.append(&dynamic_group);

    // === Automation Section (Expect Rules + Post-login Scripts) ===
    let has_automation = !group.expect_rules.is_empty() || !group.post_login_scripts.is_empty();

    let automation_expander = adw::ExpanderRow::builder()
        .title(i18n("Automation"))
        .subtitle(i18n(
            "Expect rules and post-login scripts inherited by connections",
        ))
        .show_enable_switch(true)
        .expanded(has_automation)
        .enable_expansion(has_automation)
        .build();

    let automation_group = adw::PreferencesGroup::new();
    automation_group.add(&automation_expander);
    content.append(&automation_group);

    // Confirm before clearing automation when the enable switch is toggled off
    {
        let expander = automation_expander.clone();
        let window_for_confirm = group_window.clone();
        let clearing_in_progress = Rc::new(std::cell::Cell::new(false));
        let clearing_flag = clearing_in_progress.clone();
        automation_expander.connect_enable_expansion_notify(move |row| {
            if row.enables_expansion() {
                return;
            }
            if clearing_in_progress.get() {
                return;
            }
            row.set_enable_expansion(true);
            let confirm = adw::AlertDialog::builder()
                .heading(i18n("Clear Automation?"))
                .body(i18n(
                    "Disabling will clear all expect rules and post-login scripts for this group. This cannot be undone.",
                ))
                .close_response("cancel")
                .default_response("cancel")
                .build();
            confirm.add_response("cancel", &i18n("Keep"));
            confirm.add_response("clear", &i18n("Clear"));
            confirm.set_response_appearance("clear", adw::ResponseAppearance::Destructive);

            let expander_c = expander.clone();
            let flag = clearing_flag.clone();
            confirm.connect_response(None, move |_, response| {
                if response == "clear" {
                    flag.set(true);
                    expander_c.set_enable_expansion(false);
                    expander_c.set_expanded(false);
                    flag.set(false);
                }
            });
            confirm.present(Some(&window_for_confirm));
        });
    }

    // --- Expect Rules section (outside ExpanderRow to avoid click-swallowing) ---
    let expect_rules_group = adw::PreferencesGroup::builder()
        .title(i18n("Expect Rules"))
        .description(i18n("Auto-respond to terminal patterns (priority order)"))
        .build();

    // Info banner about variable substitution
    let variables_info = Label::builder()
        .label(&i18n(
            "Responses support ${password}, ${username}, and ${VARIABLE_NAME} placeholders resolved at connection time",
        ))
        .wrap(true)
        .halign(gtk4::Align::Start)
        .css_classes(["dim-label", "caption"])
        .build();
    variables_info.set_margin_bottom(4);
    expect_rules_group.add(&variables_info);

    let expect_rules_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    expect_rules_list.set_placeholder(Some(&Label::new(Some(&i18n("No expect rules")))));

    // No inner ScrolledWindow — the dialog's own scrolled window handles scrolling.
    // This avoids the scroll-in-scroll anti-pattern (GNOME HIG).
    expect_rules_group.add(&expect_rules_list);

    // Shared state for expect rules (start empty — populated below)
    let group_expect_rules: Rc<RefCell<Vec<rustconn_core::automation::ExpectRule>>> =
        Rc::new(RefCell::new(Vec::new()));

    // Button row outside PreferencesGroup to avoid ListBoxRow click-swallowing
    let expect_button_box = gtk4::Box::new(Orientation::Horizontal, 8);
    expect_button_box.set_halign(gtk4::Align::End);
    expect_button_box.set_margin_top(8);

    let template_menu_button = gtk4::MenuButton::builder()
        .label(&i18n("From Template"))
        .tooltip_text(i18n("Add rules from a built-in template"))
        .build();

    let template_popover = gtk4::Popover::new();
    // Fixed width prevents the dialog from resizing when different templates are selected
    template_popover.set_size_request(280, -1);
    let template_list_box = gtk4::Box::new(Orientation::Vertical, 4);
    template_list_box.set_margin_top(8);
    template_list_box.set_margin_bottom(8);
    template_list_box.set_margin_start(8);
    template_list_box.set_margin_end(8);

    for template in rustconn_core::automation::builtin_templates() {
        // Add protocol hint to SSH-specific templates
        let label = if template.protocol_hint.is_empty() {
            template.name.to_string()
        } else {
            format!(
                "{} ({})",
                template.name,
                template.protocol_hint.to_uppercase()
            )
        };
        let btn = Button::builder()
            .label(&label)
            .css_classes(["flat"])
            .tooltip_text(template.description)
            .build();
        template_list_box.append(&btn);
    }
    template_popover.set_child(Some(&template_list_box));
    template_menu_button.set_popover(Some(&template_popover));

    // Wire template buttons after popover is fully built
    {
        let templates = rustconn_core::automation::builtin_templates();
        let mut child = template_list_box.first_child();
        let mut idx = 0;
        while let Some(widget) = child {
            let next = widget.next_sibling();
            if let Some(btn) = widget.downcast_ref::<Button>()
                && idx < templates.len()
            {
                let rules_clone = group_expect_rules.clone();
                let list_clone = expect_rules_list.clone();
                let template_idx = idx;
                btn.connect_clicked(move |btn| {
                    let templates = rustconn_core::automation::builtin_templates();
                    if template_idx >= templates.len() {
                        return;
                    }
                    let new_rules = templates[template_idx].rules();
                    for rule in &new_rules {
                        add_group_expect_rule_row(&list_clone, &rules_clone, Some(rule));
                    }
                    if let Some(popover) = btn
                        .ancestor(gtk4::Popover::static_type())
                        .and_then(|w| w.downcast::<gtk4::Popover>().ok())
                    {
                        popover.popdown();
                    }
                });
                idx += 1;
            }
            child = next;
        }
    }

    let add_rule_button = Button::builder()
        .label(&i18n("Add Rule"))
        .css_classes(["suggested-action"])
        .build();
    {
        let rules_clone = group_expect_rules.clone();
        let list_clone = expect_rules_list.clone();
        add_rule_button.connect_clicked(move |_| {
            add_group_expect_rule_row(&list_clone, &rules_clone, None);
        });
    }

    let clear_rules_button = Button::builder()
        .label(&i18n("Clear All"))
        .css_classes(["flat"])
        .tooltip_text(i18n("Remove all expect rules"))
        .build();
    {
        let rules_clone = group_expect_rules.clone();
        let list_clone = expect_rules_list.clone();
        clear_rules_button.connect_clicked(move |_| {
            rules_clone.borrow_mut().clear();
            while let Some(row) = list_clone.row_at_index(0) {
                list_clone.remove(&row);
            }
        });
    }

    expect_button_box.append(&clear_rules_button);
    expect_button_box.append(&template_menu_button);
    expect_button_box.append(&add_rule_button);

    // Populate existing expect rules
    for rule in &group.expect_rules {
        add_group_expect_rule_row(&expect_rules_list, &group_expect_rules, Some(rule));
    }

    expect_rules_group.set_visible(has_automation);
    content.append(&expect_rules_group);
    // Button box is a plain gtk4::Box appended directly to content (not inside
    // PreferencesGroup) so that MenuButton popover and Button clicks work
    // without being swallowed by ListBoxRow selection handling.
    expect_button_box.set_visible(has_automation);
    content.append(&expect_button_box);

    // --- Pattern Tester (collapsible) ---
    let tester_expander = adw::ExpanderRow::builder()
        .title(i18n("Pattern Tester"))
        .subtitle(i18n("Test text against your expect rules"))
        .show_enable_switch(false)
        .expanded(false)
        .build();

    let test_entry = gtk4::Entry::builder()
        .hexpand(true)
        .placeholder_text(i18n("Enter text to test against patterns"))
        .build();
    let test_result_label = Label::builder()
        .label(&i18n("Enter text to test"))
        .halign(gtk4::Align::Start)
        .wrap(true)
        .css_classes(["dim-label"])
        .build();

    let test_input_row = adw::ActionRow::builder().title(i18n("Test Input")).build();
    test_input_row.add_suffix(&test_entry);
    tester_expander.add_row(&test_input_row);

    let test_result_row = adw::ActionRow::builder().title(i18n("Result")).build();
    test_result_row.add_suffix(&test_result_label);
    tester_expander.add_row(&test_result_row);

    // Wire pattern tester
    {
        let rules_for_test = group_expect_rules.clone();
        let result_label = test_result_label;
        test_entry.connect_changed(move |entry| {
            let text = entry.text().to_string();
            if text.is_empty() {
                result_label.set_label(&i18n("Enter text to test"));
                result_label.remove_css_class("success");
                result_label.remove_css_class("error");
                result_label.add_css_class("dim-label");
                return;
            }
            let rules = rules_for_test.borrow();
            let mut matched = false;
            for rule in rules.iter().filter(|r| r.enabled && !r.pattern.is_empty()) {
                if let Ok(re) = regex::Regex::new(&rule.pattern)
                    && re.is_match(&text)
                {
                    let msg = i18n_f(
                        "Match: pattern '{}' → response '{}'",
                        &[&rule.pattern, &rule.response],
                    );
                    result_label.set_label(&msg);
                    result_label.remove_css_class("dim-label");
                    result_label.remove_css_class("error");
                    result_label.add_css_class("success");
                    matched = true;
                    break;
                }
            }
            if !matched {
                result_label.set_label(&i18n("No match"));
                result_label.remove_css_class("dim-label");
                result_label.remove_css_class("success");
                result_label.add_css_class("error");
            }
        });
    }

    let tester_group = adw::PreferencesGroup::new();
    tester_group.add(&tester_expander);
    tester_group.set_visible(has_automation);
    content.append(&tester_group);

    // --- Post-login Scripts (list with Add/Delete) ---
    let scripts_group = adw::PreferencesGroup::builder()
        .title(i18n("Post-login Scripts"))
        .description(i18n(
            "Commands executed after login (inherited by connections)",
        ))
        .build();

    let scripts_list = gtk4::ListBox::builder()
        .selection_mode(gtk4::SelectionMode::None)
        .css_classes(["boxed-list"])
        .build();
    scripts_list.set_placeholder(Some(&Label::new(Some(&i18n("No post-login scripts")))));

    // No inner ScrolledWindow — the dialog's own scrolled window handles scrolling.
    scripts_group.add(&scripts_list);

    // Shared state for post-login scripts
    let group_post_login_scripts: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(Vec::new()));

    let add_script_button = Button::builder()
        .label(&i18n("Add Script"))
        .css_classes(["suggested-action"])
        .build();
    {
        let scripts_clone = group_post_login_scripts.clone();
        let list_clone = scripts_list.clone();
        add_script_button.connect_clicked(move |_| {
            add_post_login_script_row(&list_clone, &scripts_clone, None);
        });
    }

    let scripts_button_box = gtk4::Box::new(Orientation::Horizontal, 8);
    scripts_button_box.set_halign(gtk4::Align::End);
    scripts_button_box.set_margin_top(8);
    scripts_button_box.append(&add_script_button);

    // Populate existing post-login scripts
    for script in &group.post_login_scripts {
        add_post_login_script_row(&scripts_list, &group_post_login_scripts, Some(script));
    }

    scripts_group.set_visible(has_automation);
    content.append(&scripts_group);
    scripts_button_box.set_visible(has_automation);
    content.append(&scripts_button_box);

    // Show/hide all automation sub-sections based on automation expander state
    {
        let expect_group = expect_rules_group.clone();
        let expect_buttons = expect_button_box.clone();
        let tester = tester_group.clone();
        let scripts = scripts_group.clone();
        let scripts_buttons = scripts_button_box.clone();
        automation_expander.connect_enable_expansion_notify(move |row| {
            let visible = row.enables_expansion();
            expect_group.set_visible(visible);
            expect_buttons.set_visible(visible);
            tester.set_visible(visible);
            scripts.set_visible(visible);
            scripts_buttons.set_visible(visible);
        });
    }

    // === Description Section ===
    let description_group = adw::PreferencesGroup::builder()
        .title(i18n("Description"))
        .description(i18n("Notes, contacts, project info"))
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
    let state_clone = state.clone();
    let sidebar_clone = sidebar;
    let window_clone = group_window.clone();
    let name_row_clone = name_row;
    let username_row_clone = username_row;
    let password_entry_clone = password_entry.clone();
    let password_source_clone = password_source_dropdown.clone();
    let domain_row_clone = domain_row;
    let icon_row_clone = icon_row;
    let parent_row_clone = parent_row;
    let description_buffer = description_view.buffer();
    let auth_method_row_clone = auth_method_row;
    let ssh_key_path_row_clone2 = ssh_key_path_row;
    let ssh_proxy_jump_row_clone = ssh_proxy_jump_row;
    let ssh_agent_socket_row_clone = ssh_agent_socket_row;
    let sync_mode_row_clone = sync_mode_row;
    let credentials_expander_clone = credentials_expander;
    let ssh_expander_clone = ssh_expander;
    let ssh_jump_host_dropdown_clone = ssh_jump_host_dropdown;
    let variable_dropdown_clone = variable_dropdown;
    let dynamic_expander_clone = dynamic_expander;
    let dynamic_script_row_clone = dynamic_script_row;
    let dynamic_workdir_row_clone = dynamic_workdir_row;
    let dynamic_timeout_row_clone = dynamic_timeout_row;
    let dynamic_refresh_row_clone = dynamic_refresh_row;
    let automation_expander_clone = automation_expander;
    let group_expect_rules_clone = group_expect_rules;
    let group_post_login_scripts_clone = group_post_login_scripts;
    let old_name = group.name;

    save_btn.connect_clicked(move |_| {
        let new_name = name_row_clone.text().to_string();
        if new_name.trim().is_empty() {
            alert::show_validation_error(&window_clone, &i18n("Group name cannot be empty"));
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
            2 => {
                // Variable — get selected variable name from dropdown
                let selected = variable_dropdown_clone.selected();
                let var_name = variable_dropdown_clone
                    .model()
                    .and_then(|m| {
                        m.downcast_ref::<gtk4::StringList>()
                            .and_then(|sl| sl.string(selected))
                    })
                    .map_or_else(String::new, |s| s.to_string());
                if var_name.is_empty() {
                    alert::show_validation_error(&window_clone, &i18n("Please select a variable"));
                    return;
                }
                PasswordSource::Variable(var_name)
            }
            3 => PasswordSource::Inherit,
            _ => PasswordSource::None,
        };

        // Password is relevant for Vault only, and only when credentials are enabled
        let has_new_password = credentials_expander_clone.enables_expansion()
            && !password.is_empty()
            && password_source_idx == 1;

        // Check for duplicate name (but allow keeping same name)
        if new_name != old_name {
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
        }

        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
            if let Some(existing) = state_mut.get_group(group_id).cloned() {
                let mut updated = existing.clone();
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

                // When credentials switch is disabled, clear all credential fields
                if credentials_expander_clone.enables_expansion() {
                    updated.password_source = Some(new_password_source.clone());
                } else {
                    updated.username = None;
                    updated.domain = None;
                    updated.password_source = None;
                }

                // Update icon
                let icon_text = icon_row_clone.text().trim().to_string();
                if !icon_text.is_empty()
                    && let Err(e) = rustconn_core::dialog_utils::validate_icon(&icon_text)
                {
                    alert::show_validation_error(&window_clone, &i18n(&e));
                    return;
                }
                updated.icon = if icon_text.is_empty() {
                    None
                } else {
                    Some(icon_text)
                };

                // Update SSH settings — only when SSH expander is enabled
                if ssh_expander_clone.enables_expansion() {
                    updated.ssh_auth_method = match auth_method_row_clone.selected() {
                        1 => Some(SshAuthMethod::Password),
                        2 => Some(SshAuthMethod::PublicKey),
                        3 => Some(SshAuthMethod::Agent),
                        4 => Some(SshAuthMethod::KeyboardInteractive),
                        5 => Some(SshAuthMethod::SecurityKey),
                        _ => None,
                    };

                    let key_path_text = ssh_key_path_row_clone2.text().trim().to_string();
                    updated.ssh_key_path = if key_path_text.is_empty() {
                        None
                    } else {
                        Some(std::path::PathBuf::from(key_path_text))
                    };

                    let proxy_jump_text = ssh_proxy_jump_row_clone.text().trim().to_string();
                    updated.ssh_proxy_jump = if proxy_jump_text.is_empty() {
                        None
                    } else {
                        Some(proxy_jump_text)
                    };

                    // Jump Host dropdown — resolve selected connection ID
                    let jump_idx = ssh_jump_host_dropdown_clone.selected() as usize;
                    updated.ssh_jump_host_id = if jump_idx < jump_host_data.len() {
                        jump_host_data[jump_idx].0
                    } else {
                        None
                    };

                    let agent_socket_text = ssh_agent_socket_row_clone.text().trim().to_string();
                    updated.ssh_agent_socket = if agent_socket_text.is_empty() {
                        None
                    } else {
                        Some(agent_socket_text)
                    };
                } else {
                    updated.ssh_auth_method = None;
                    updated.ssh_key_path = None;
                    updated.ssh_proxy_jump = None;
                    updated.ssh_jump_host_id = None;
                    updated.ssh_agent_socket = None;
                }

                // Update Cloud Sync mode (only meaningful for root groups)
                updated.sync_mode = match sync_mode_row_clone.selected() {
                    1 => SyncMode::Master,
                    2 => SyncMode::Import,
                    _ => SyncMode::None,
                };

                // Generate sync_file when switching to Master for the first time
                if updated.sync_mode == SyncMode::Master && updated.sync_file.is_none() {
                    updated.sync_file = Some(
                        rustconn_core::sync::group_export::group_name_to_filename(&updated.name),
                    );
                }

                // Update Dynamic Folder configuration
                if dynamic_expander_clone.enables_expansion() {
                    let script = dynamic_script_row_clone.text().trim().to_string();
                    if script.is_empty() {
                        alert::show_validation_error(
                            &window_clone,
                            &i18n("Dynamic Folder script cannot be empty"),
                        );
                        return;
                    }
                    let workdir = dynamic_workdir_row_clone.text().trim().to_string();
                    let timeout_secs = dynamic_timeout_row_clone.value() as u64;
                    let refresh_secs = dynamic_refresh_row_clone.value() as u64;

                    let mut config = rustconn_core::DynamicFolderConfig::new(script);
                    if !workdir.is_empty() {
                        config.working_directory = Some(std::path::PathBuf::from(workdir));
                    }
                    config.timeout_secs = timeout_secs;
                    config.refresh_interval_secs = if refresh_secs > 0 {
                        Some(refresh_secs)
                    } else {
                        None
                    };
                    // Preserve last_refreshed_at and last_error from existing config
                    if let Some(ref existing) = existing.dynamic_folder {
                        config.last_refreshed_at = existing.last_refreshed_at;
                        config.last_error = existing.last_error.clone();
                    }
                    updated.dynamic_folder = Some(config);
                } else {
                    updated.dynamic_folder = None;
                }

                // Update Automation (Expect Rules + Post-login Scripts)
                if automation_expander_clone.enables_expansion() {
                    // Collect expect rules, filtering out empty patterns
                    updated.expect_rules = group_expect_rules_clone
                        .borrow()
                        .iter()
                        .filter(|r| !r.pattern.is_empty())
                        .cloned()
                        .collect();

                    // Collect post-login scripts from shared state, filtering empty
                    updated.post_login_scripts = group_post_login_scripts_clone
                        .borrow()
                        .iter()
                        .filter(|s| !s.trim().is_empty())
                        .cloned()
                        .collect();
                } else {
                    updated.expect_rules = Vec::new();
                    updated.post_login_scripts = Vec::new();
                }

                // Capture old groups snapshot before update for vault migration
                let name_changed = existing.name != updated.name;
                let parent_changed = existing.parent_id != updated.parent_id;
                let old_groups_snapshot: Vec<rustconn_core::models::ConnectionGroup> =
                    if name_changed || parent_changed {
                        state_mut.list_groups().into_iter().cloned().collect()
                    } else {
                        Vec::new()
                    };

                if let Err(e) = state_mut
                    .connection_manager()
                    .update_group(group_id, updated)
                {
                    alert::show_error(&window_clone, &i18n("Error"), &format!("{e}"));
                    return;
                }

                // Migrate vault entries if group name or parent changed (KeePass paths affected)
                if name_changed || parent_changed {
                    let new_groups: Vec<_> = state_mut.list_groups().into_iter().cloned().collect();
                    let connections: Vec<_> =
                        state_mut.list_connections().into_iter().cloned().collect();
                    let settings = state_mut.settings().clone();
                    crate::state::migrate_vault_entries_on_group_change(
                        &settings,
                        &old_groups_snapshot,
                        &new_groups,
                        &connections,
                        group_id,
                    );
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

/// Shows the Enable Master confirmation dialog when sync_dir is already configured.
fn show_enable_master_confirmation(
    parent: &adw::Window,
    sync_dir_display: &str,
    row: &adw::ComboRow,
    prev_idx: &Rc<std::cell::Cell<u32>>,
) {
    let body = i18n_f(
        "This group will be exported to {}. Other team members with read access can import it.",
        &[sync_dir_display],
    );

    let dialog = adw::AlertDialog::new(Some(&i18n("Enable Cloud Sync?")), Some(&body));
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.add_response("enable", &i18n("Enable"));
    dialog.set_close_response("cancel");
    dialog.set_response_appearance("enable", adw::ResponseAppearance::Suggested);

    let row_clone = row.clone();
    let prev_idx_inner = prev_idx.clone();
    dialog.connect_response(None, move |_, response| {
        if response == "enable" {
            prev_idx_inner.set(1);
        } else {
            row_clone.set_selected(prev_idx_inner.get());
        }
    });

    dialog.present(Some(parent));
}

/// Shows the Cloud Sync setup dialog when sync_dir is not configured.
///
/// Displays an `AdwAlertDialog` with an `AdwStatusPage` empty state
/// (icon: `folder-remote-symbolic`, title: "Set Up Cloud Sync") and a
/// "Choose Directory" button. After the user selects a directory, saves
/// it to `SyncSettings.sync_dir` and proceeds with the Enable Master
/// confirmation flow.
fn show_sync_setup_dialog(
    parent: &adw::Window,
    state: &SharedAppState,
    row: &adw::ComboRow,
    prev_idx: &Rc<std::cell::Cell<u32>>,
) {
    let choose_btn = Button::builder()
        .label(i18n("Choose Directory"))
        .halign(gtk4::Align::Center)
        .css_classes(["suggested-action", "pill"])
        .build();

    let status_page = adw::StatusPage::builder()
        .icon_name("folder-remote-symbolic")
        .title(i18n("Set Up Cloud Sync"))
        .description(i18n(
            "Choose a directory synced with your cloud service (Google Drive, Nextcloud, Syncthing, etc.)",
        ))
        .child(&choose_btn)
        .build();

    let dialog = adw::AlertDialog::new(None, None);
    dialog.set_extra_child(Some(&status_page));
    dialog.add_response("cancel", &i18n("Cancel"));
    dialog.set_close_response("cancel");

    // Revert combo row on cancel
    let row_for_cancel = row.clone();
    let prev_idx_for_cancel = prev_idx.clone();
    dialog.connect_response(None, move |_, _response| {
        // Any response (only "cancel" exists) reverts the combo row
        row_for_cancel.set_selected(prev_idx_for_cancel.get());
    });

    // "Choose Directory" button opens a folder chooser
    let state_clone = state.clone();
    let parent_clone = parent.clone();
    let row_clone = row.clone();
    let prev_idx_clone = prev_idx.clone();
    let dialog_clone = dialog.clone();
    choose_btn.connect_clicked(move |_| {
        let file_dialog = gtk4::FileDialog::builder()
            .title(i18n("Select Sync Directory"))
            .modal(true)
            .build();

        let state_inner = state_clone.clone();
        let parent_inner = parent_clone.clone();
        let row_inner = row_clone.clone();
        let prev_idx_inner = prev_idx_clone.clone();
        let dialog_inner = dialog_clone.clone();
        file_dialog.select_folder(
            Some(&parent_clone),
            gtk4::gio::Cancellable::NONE,
            move |result| {
                let Ok(folder) = result else {
                    return; // User cancelled folder chooser — stay on setup dialog
                };
                let Some(path) = folder.path() else {
                    return;
                };

                // Detect Flatpak document portal path — inotify won't work there
                if rustconn_core::flatpak::is_flatpak()
                    && rustconn_core::flatpak::is_portal_path(&path)
                {
                    dialog_inner.force_close();
                    crate::dialogs::settings::cloud_sync_tab::show_flatpak_sync_dir_portal_warning(
                        parent_inner.upcast_ref(),
                        &path,
                    );
                    return;
                }

                // Save sync_dir to settings
                if let Ok(mut state_mut) = state_inner.try_borrow_mut() {
                    state_mut.settings_mut().sync.sync_dir = Some(path.clone());
                    if let Err(e) = state_mut.save_settings() {
                        tracing::warn!(?e, "Failed to save sync settings");
                    }
                }

                // Close the setup dialog
                dialog_inner.force_close();

                // Now show the Enable Master confirmation with the new sync_dir
                let sync_dir_display = path.display().to_string();
                show_enable_master_confirmation(
                    &parent_inner,
                    &sync_dir_display,
                    &row_inner,
                    &prev_idx_inner,
                );
            },
        );
    });

    dialog.present(Some(parent));
}

/// Shows the quick connect dialog with protocol selection and template support
pub fn show_quick_connect_dialog(
    window: &gtk4::Window,
    notebook: SharedNotebook,
    split_view: SharedSplitView,
    sidebar: SharedSidebar,
    history: super::types::SharedQuickConnectHistory,
) {
    show_quick_connect_dialog_with_state(window, notebook, split_view, sidebar, None, history);
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
    let quick_window = adw::Window::builder()
        .title(i18n("Quick Connect"))
        .modal(true)
        .default_width(450)
        .build();

    if let Some(gtk_win) = window.downcast_ref::<gtk4::Window>() {
        quick_window.set_transient_for(Some(gtk_win));
    }

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
    let window_clone = quick_window.clone();
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

    quick_window.present();
}

/// Adds an expect rule row to the group edit dialog's expect rules list.
///
/// Creates a compact rule editor with pattern, response, priority, timeout,
/// enabled/one-shot checkboxes, and delete/move buttons. Layout uses vertical
/// stacking to ensure all controls are visible without horizontal overflow.
fn add_group_expect_rule_row(
    list: &gtk4::ListBox,
    rules: &Rc<RefCell<Vec<rustconn_core::automation::ExpectRule>>>,
    rule: Option<&rustconn_core::automation::ExpectRule>,
) {
    use rustconn_core::automation::ExpectRule;

    let main_box = gtk4::Box::new(Orientation::Vertical, 6);
    main_box.set_margin_top(8);
    main_box.set_margin_bottom(8);
    main_box.set_margin_start(8);
    main_box.set_margin_end(8);

    // Row 0: Action buttons (delete, move up/down) — top-right for visibility
    let action_box = gtk4::Box::new(Orientation::Horizontal, 4);
    action_box.set_halign(gtk4::Align::End);

    let move_up_button = Button::builder()
        .icon_name("go-up-symbolic")
        .css_classes(["flat"])
        .tooltip_text(i18n("Move up (higher priority)"))
        .build();
    move_up_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Move rule up"))]);
    let move_down_button = Button::builder()
        .icon_name("go-down-symbolic")
        .css_classes(["flat"])
        .tooltip_text(i18n("Move down (lower priority)"))
        .build();
    move_down_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Move rule down"))]);
    let delete_button = Button::builder()
        .icon_name("user-trash-symbolic")
        .css_classes(["flat"])
        .tooltip_text(i18n("Delete rule"))
        .build();
    delete_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Delete rule"))]);
    action_box.append(&move_up_button);
    action_box.append(&move_down_button);
    action_box.append(&delete_button);
    main_box.append(&action_box);

    // Row 1: Pattern entry (full width)
    let pattern_box = gtk4::Box::new(Orientation::Horizontal, 6);
    let pattern_label = Label::builder()
        .label(&i18n("Pattern:"))
        .halign(gtk4::Align::End)
        .width_chars(10)
        .build();
    let pattern_entry = gtk4::Entry::builder()
        .hexpand(true)
        .placeholder_text(i18n("Regex pattern (e.g., password:\\s*$)"))
        .tooltip_text(i18n("Regular expression to match against terminal output"))
        .build();
    pattern_box.append(&pattern_label);
    pattern_box.append(&pattern_entry);
    main_box.append(&pattern_box);

    // Row 2: Response entry + "Insert Variable" button
    let response_box = gtk4::Box::new(Orientation::Horizontal, 6);
    let response_label = Label::builder()
        .label(&i18n("Response:"))
        .halign(gtk4::Align::End)
        .width_chars(10)
        .build();
    let response_entry = gtk4::Entry::builder()
        .hexpand(true)
        .placeholder_text(i18n("Text to send (e.g., ${password}\\n)"))
        .tooltip_text(i18n(
            "Response to send when pattern matches. Use ${password}, ${username}, or ${VAR_NAME} for variables.",
        ))
        .build();

    // "Insert Variable" button with popover listing available variables
    let var_menu_button = gtk4::MenuButton::builder()
        .icon_name("list-add-symbolic")
        .css_classes(["flat"])
        .tooltip_text(i18n("Insert a variable placeholder"))
        .build();
    var_menu_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Insert variable"))]);

    let var_popover = gtk4::Popover::new();
    var_popover.set_size_request(220, -1);
    let var_list = gtk4::Box::new(Orientation::Vertical, 2);
    var_list.set_margin_top(6);
    var_list.set_margin_bottom(6);
    var_list.set_margin_start(6);
    var_list.set_margin_end(6);

    // Built-in variables
    let builtin_header = Label::builder()
        .label(&i18n("Built-in"))
        .halign(gtk4::Align::Start)
        .css_classes(["dim-label", "caption"])
        .build();
    var_list.append(&builtin_header);

    for (var_name, var_desc) in [
        ("${password}", i18n("Connection password")),
        ("${username}", i18n("Connection username")),
        ("${host}", i18n("Connection host")),
        ("${port}", i18n("Connection port")),
    ] {
        let btn = Button::builder()
            .label(var_name)
            .css_classes(["flat"])
            .tooltip_text(&var_desc)
            .build();
        let entry_clone = response_entry.clone();
        let var = var_name.to_string();
        btn.connect_clicked(move |btn| {
            // Insert variable at cursor position
            let pos = entry_clone.position();
            entry_clone.insert_text(&var, &mut pos.clone());
            #[allow(clippy::cast_possible_wrap)]
            entry_clone.set_position(pos + var.len() as i32);
            if let Some(popover) = btn
                .ancestor(gtk4::Popover::static_type())
                .and_then(|w| w.downcast::<gtk4::Popover>().ok())
            {
                popover.popdown();
            }
        });
        var_list.append(&btn);
    }

    // Newline helper
    let special_header = Label::builder()
        .label(&i18n("Special"))
        .halign(gtk4::Align::Start)
        .css_classes(["dim-label", "caption"])
        .margin_top(4)
        .build();
    var_list.append(&special_header);

    let newline_btn = Button::builder()
        .label("\\n")
        .css_classes(["flat"])
        .tooltip_text(i18n("Newline (Enter key)"))
        .build();
    {
        let entry_clone = response_entry.clone();
        newline_btn.connect_clicked(move |btn| {
            let pos = entry_clone.position();
            entry_clone.insert_text("\\n", &mut pos.clone());
            #[allow(clippy::cast_possible_wrap)]
            entry_clone.set_position(pos + 2);
            if let Some(popover) = btn
                .ancestor(gtk4::Popover::static_type())
                .and_then(|w| w.downcast::<gtk4::Popover>().ok())
            {
                popover.popdown();
            }
        });
    }
    var_list.append(&newline_btn);

    var_popover.set_child(Some(&var_list));
    var_menu_button.set_popover(Some(&var_popover));

    response_box.append(&response_label);
    response_box.append(&response_entry);
    response_box.append(&var_menu_button);
    main_box.append(&response_box);

    // Row 3: Priority, Timeout, Enabled, One-shot — compact horizontal row
    let settings_box = gtk4::Box::new(Orientation::Horizontal, 8);
    settings_box.set_halign(gtk4::Align::Start);

    let priority_label = Label::builder()
        .label(&i18n("Priority:"))
        .css_classes(["dim-label", "caption"])
        .build();
    let priority_adj = gtk4::Adjustment::new(0.0, -1000.0, 1000.0, 1.0, 10.0, 0.0);
    let priority_spin = gtk4::SpinButton::builder()
        .adjustment(&priority_adj)
        .climb_rate(1.0)
        .digits(0)
        .width_chars(5)
        .tooltip_text(i18n("Higher priority rules are checked first"))
        .build();

    let timeout_label = Label::builder()
        .label(&i18n("Timeout:"))
        .css_classes(["dim-label", "caption"])
        .build();
    let timeout_adj = gtk4::Adjustment::new(0.0, 0.0, 60000.0, 100.0, 1000.0, 0.0);
    let timeout_spin = gtk4::SpinButton::builder()
        .adjustment(&timeout_adj)
        .climb_rate(1.0)
        .digits(0)
        .width_chars(6)
        .tooltip_text(i18n("Timeout in milliseconds (0 = no timeout)"))
        .build();

    let enabled_check = gtk4::CheckButton::builder()
        .label(i18n("Enabled"))
        .active(true)
        .build();
    let one_shot_check = gtk4::CheckButton::builder()
        .label(i18n("One-shot"))
        .active(true)
        .tooltip_text(i18n("Fire only once, then remove the rule"))
        .build();

    settings_box.append(&priority_label);
    settings_box.append(&priority_spin);
    settings_box.append(&timeout_label);
    settings_box.append(&timeout_spin);
    settings_box.append(&enabled_check);
    settings_box.append(&one_shot_check);
    main_box.append(&settings_box);

    // Row 4: Regex validation label
    let validation_label = Label::builder()
        .halign(gtk4::Align::Start)
        .css_classes(["error"])
        .visible(false)
        .build();
    main_box.append(&validation_label);

    // Wire regex validation on pattern entry
    let validation_clone = validation_label;
    pattern_entry.connect_changed(move |entry| {
        let text = entry.text().to_string();
        if text.is_empty() {
            validation_clone.set_visible(false);
            entry.remove_css_class("error");
        } else {
            match regex::Regex::new(&text) {
                Ok(_) => {
                    validation_clone.set_visible(false);
                    entry.remove_css_class("error");
                }
                Err(e) => {
                    validation_clone.set_text(&e.to_string());
                    validation_clone.set_visible(true);
                    entry.add_css_class("error");
                }
            }
        }
    });

    // Populate from existing rule if provided
    let rule_id = rule.map_or_else(Uuid::new_v4, |r| {
        pattern_entry.set_text(&r.pattern);
        response_entry.set_text(&r.response);
        priority_spin.set_value(f64::from(r.priority));
        timeout_spin.set_value(f64::from(r.timeout_ms.unwrap_or(0)));
        enabled_check.set_active(r.enabled);
        one_shot_check.set_active(r.one_shot);
        r.id
    });

    let row = gtk4::ListBoxRow::builder().child(&main_box).build();

    // Add rule to shared state
    let new_rule = rule
        .cloned()
        .unwrap_or_else(|| ExpectRule::with_id(rule_id, "", ""));
    rules.borrow_mut().push(new_rule);

    // Connect delete button
    let list_for_delete = list.clone();
    let rules_for_delete = rules.clone();
    let row_for_delete = row.clone();
    delete_button.connect_clicked(move |_| {
        list_for_delete.remove(&row_for_delete);
        rules_for_delete.borrow_mut().retain(|r| r.id != rule_id);
    });

    // Connect move up button
    let list_for_up = list.clone();
    let rules_for_up = rules.clone();
    let row_for_up = row.clone();
    move_up_button.connect_clicked(move |_| {
        let index = row_for_up.index();
        if index <= 0 {
            return;
        }
        list_for_up.remove(&row_for_up);
        list_for_up.insert(&row_for_up, index - 1);
        #[allow(clippy::cast_sign_loss)]
        let idx = index as usize;
        let mut rules_vec = rules_for_up.borrow_mut();
        if idx < rules_vec.len() {
            rules_vec.swap(idx, idx - 1);
        }
    });

    // Connect move down button
    let list_for_down = list.clone();
    let rules_for_down = rules.clone();
    let row_for_down = row.clone();
    move_down_button.connect_clicked(move |_| {
        let index = row_for_down.index();
        let rules_len = rules_for_down.borrow().len();
        #[allow(clippy::cast_possible_wrap, clippy::cast_possible_truncation)]
        if index < 0 || index >= (rules_len as i32 - 1) {
            return;
        }
        list_for_down.remove(&row_for_down);
        list_for_down.insert(&row_for_down, index + 1);
        #[allow(clippy::cast_sign_loss)]
        let idx = index as usize;
        let mut rules_vec = rules_for_down.borrow_mut();
        if idx + 1 < rules_vec.len() {
            rules_vec.swap(idx, idx + 1);
        }
    });

    // Connect entry changes to update the rule in shared state
    let rules_for_pattern = rules.clone();
    pattern_entry.connect_changed(move |entry| {
        let text = entry.text().to_string();
        if let Some(r) = rules_for_pattern
            .borrow_mut()
            .iter_mut()
            .find(|r| r.id == rule_id)
        {
            r.pattern = text;
        }
    });

    let rules_for_response = rules.clone();
    response_entry.connect_changed(move |entry| {
        let text = entry.text().to_string();
        if let Some(r) = rules_for_response
            .borrow_mut()
            .iter_mut()
            .find(|r| r.id == rule_id)
        {
            r.response = text;
        }
    });

    let rules_for_priority = rules.clone();
    priority_spin.connect_value_changed(move |spin| {
        #[allow(clippy::cast_possible_truncation)]
        let value = spin.value() as i32;
        if let Some(r) = rules_for_priority
            .borrow_mut()
            .iter_mut()
            .find(|r| r.id == rule_id)
        {
            r.priority = value;
        }
    });

    let rules_for_timeout = rules.clone();
    timeout_spin.connect_value_changed(move |spin| {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let value = spin.value() as u32;
        if let Some(r) = rules_for_timeout
            .borrow_mut()
            .iter_mut()
            .find(|r| r.id == rule_id)
        {
            r.timeout_ms = if value == 0 { None } else { Some(value) };
        }
    });

    let rules_for_enabled = rules.clone();
    enabled_check.connect_toggled(move |check| {
        if let Some(r) = rules_for_enabled
            .borrow_mut()
            .iter_mut()
            .find(|r| r.id == rule_id)
        {
            r.enabled = check.is_active();
        }
    });

    let rules_for_one_shot = rules.clone();
    one_shot_check.connect_toggled(move |check| {
        if let Some(r) = rules_for_one_shot
            .borrow_mut()
            .iter_mut()
            .find(|r| r.id == rule_id)
        {
            r.one_shot = check.is_active();
        }
    });

    list.append(&row);
}

/// Adds a post-login script row to the group edit dialog's scripts list.
///
/// Creates a simple row with a command entry and delete button.
fn add_post_login_script_row(
    list: &gtk4::ListBox,
    scripts: &Rc<RefCell<Vec<String>>>,
    script: Option<&str>,
) {
    use crate::i18n::i18n;

    let row_box = gtk4::Box::new(Orientation::Horizontal, 8);
    row_box.set_margin_top(8);
    row_box.set_margin_bottom(8);
    row_box.set_margin_start(8);
    row_box.set_margin_end(8);

    let command_entry = gtk4::Entry::builder()
        .hexpand(true)
        .placeholder_text(i18n("Shell command (e.g., export TERM=xterm-256color)"))
        .tooltip_text(i18n("Command to execute after login"))
        .build();

    if let Some(s) = script {
        command_entry.set_text(s);
    }

    let delete_button = Button::builder()
        .icon_name("user-trash-symbolic")
        .css_classes(["flat"])
        .tooltip_text(i18n("Delete script"))
        .valign(gtk4::Align::Center)
        .build();
    delete_button.update_property(&[gtk4::accessible::Property::Label(&i18n("Delete script"))]);
    delete_button.set_hexpand(false);

    row_box.append(&command_entry);
    row_box.append(&delete_button);

    let row = gtk4::ListBoxRow::builder().child(&row_box).build();

    // Track index for this script in shared state
    let script_idx = scripts.borrow().len();
    scripts.borrow_mut().push(script.unwrap_or("").to_string());

    // Connect delete button
    let list_for_delete = list.clone();
    let scripts_for_delete = scripts.clone();
    let row_for_delete = row.clone();
    delete_button.connect_clicked(move |_| {
        let idx = row_for_delete.index();
        list_for_delete.remove(&row_for_delete);
        #[allow(clippy::cast_sign_loss)]
        if idx >= 0 {
            let idx = idx as usize;
            let mut vec = scripts_for_delete.borrow_mut();
            if idx < vec.len() {
                vec.remove(idx);
            }
        }
    });

    // Connect entry changes to update the script in shared state
    let scripts_for_change = scripts.clone();
    command_entry.connect_changed(move |entry| {
        let text = entry.text().to_string();
        let mut vec = scripts_for_change.borrow_mut();
        if script_idx < vec.len() {
            vec[script_idx] = text;
        }
    });

    list.append(&row);
}
