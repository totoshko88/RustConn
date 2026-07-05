//! Smart Folder window actions.
//!
//! Registers actions for creating, editing, and deleting smart folders
//! from the sidebar UI.

use gtk4::gio;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;

use super::MainWindow;
use super::types::SharedSidebar;
use crate::dialogs::SmartFolderDialog;
use crate::i18n::{i18n, i18n_f};
use crate::state::SharedAppState;

impl MainWindow {
    /// Registers smart folder actions on the window.
    pub fn setup_smart_folder_actions(
        window: &adw::ApplicationWindow,
        state: &SharedAppState,
        sidebar: &SharedSidebar,
    ) {
        // --- New Smart Folder ---
        let new_action = gio::SimpleAction::new("new-smart-folder", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        new_action.connect_activate(move |_, _| {
            let Some(win) = window_weak.upgrade() else {
                return;
            };
            let dialog = SmartFolderDialog::new(Some(win.upcast_ref()), None);

            // Populate group picker
            let groups: Vec<_> = state_clone
                .borrow()
                .list_groups()
                .into_iter()
                .cloned()
                .collect();
            dialog.set_groups(&groups);

            let state_save = state_clone.clone();
            let sidebar_save = sidebar_clone.clone();
            let window_weak_save = win.downgrade();
            dialog.run(move |result| {
                if let Some(folder) = result {
                    if let Ok(mut state_mut) = state_save.try_borrow_mut() {
                        let mut settings = state_mut.settings().clone();
                        settings.smart_folders.push(folder);
                        if let Err(e) = state_mut.update_settings(settings) {
                            tracing::warn!(error = %e, "failed to save smart folder");
                            if let Some(win) = window_weak_save.upgrade() {
                                // Persistence failure is not transient — blocking dialog (GNOME HIG).
                                let msg = i18n_f("Could not save smart folder: {}", &[&e]);
                                crate::alert::show_error(
                                    &win,
                                    &i18n("Could not save smart folder"),
                                    &msg,
                                );
                            }
                        }
                    }
                    Self::reload_sidebar_preserving_state(&state_save, &sidebar_save);
                }
            });
        });
        window.add_action(&new_action);

        // --- Edit Smart Folder ---
        let edit_action = gio::SimpleAction::new("edit-smart-folder", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        edit_action.connect_activate(move |_, _| {
            let Some(win) = window_weak.upgrade() else {
                return;
            };

            // Get selected smart folder from the list box
            let smart_folders_widget = sidebar_clone.smart_folders_sidebar();
            let Some(selected_row) = smart_folders_widget.list_box().selected_row() else {
                return;
            };
            let index = selected_row.index() as usize;

            let state_ref = state_clone.borrow();
            let folders = &state_ref.settings().smart_folders;
            let Some(folder) = folders.get(index) else {
                drop(state_ref);
                return;
            };
            let folder_clone = folder.clone();
            let groups: Vec<_> = state_ref.list_groups().into_iter().cloned().collect();
            drop(state_ref);

            let dialog = SmartFolderDialog::new(Some(win.upcast_ref()), Some(&folder_clone));
            dialog.set_groups(&groups);
            if let Some(gid) = folder_clone.filter_group_id {
                dialog.set_selected_group(Some(gid));
            }

            let state_save = state_clone.clone();
            let sidebar_save = sidebar_clone.clone();
            let window_weak_save = win.downgrade();
            dialog.run(move |result| {
                if let Some(updated_folder) = result {
                    if let Ok(mut state_mut) = state_save.try_borrow_mut() {
                        let mut settings = state_mut.settings().clone();
                        if let Some(existing) = settings
                            .smart_folders
                            .iter_mut()
                            .find(|f| f.id == updated_folder.id)
                        {
                            *existing = updated_folder;
                        }
                        if let Err(e) = state_mut.update_settings(settings) {
                            tracing::warn!(error = %e, "failed to save edited smart folder");
                            if let Some(win) = window_weak_save.upgrade() {
                                // Persistence failure is not transient — blocking dialog (GNOME HIG).
                                let msg = i18n_f("Could not save smart folder: {}", &[&e]);
                                crate::alert::show_error(
                                    &win,
                                    &i18n("Could not save smart folder"),
                                    &msg,
                                );
                            }
                        }
                    }
                    Self::reload_sidebar_preserving_state(&state_save, &sidebar_save);
                }
            });
        });
        window.add_action(&edit_action);

        // --- Delete Smart Folder ---
        let delete_action = gio::SimpleAction::new("delete-smart-folder", None);
        let window_weak = window.downgrade();
        let state_clone = state.clone();
        let sidebar_clone = sidebar.clone();
        delete_action.connect_activate(move |_, _| {
            let Some(win) = window_weak.upgrade() else {
                return;
            };

            let smart_folders_widget = sidebar_clone.smart_folders_sidebar();
            let Some(selected_row) = smart_folders_widget.list_box().selected_row() else {
                return;
            };
            let index = selected_row.index() as usize;

            let state_ref = state_clone.borrow();
            let folders = &state_ref.settings().smart_folders;
            let Some(folder) = folders.get(index) else {
                drop(state_ref);
                return;
            };
            let folder_id = folder.id;
            let folder_name = folder.name.clone();
            drop(state_ref);

            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                let mut settings = state_mut.settings().clone();
                settings.smart_folders.retain(|f| f.id != folder_id);
                if let Err(e) = state_mut.update_settings(settings) {
                    tracing::warn!(error = %e, "failed to delete smart folder");
                    // Failure to persist settings is not transient — show a
                    // blocking dialog instead of a toast (GNOME HIG).
                    let msg = i18n_f("Could not delete smart folder: {}", &[&e]);
                    crate::alert::show_error(&win, &i18n("Could not delete smart folder"), &msg);
                    return;
                }
            }

            Self::reload_sidebar_preserving_state(&state_clone, &sidebar_clone);

            let msg = i18n_f("Smart folder '{}' deleted", &[&folder_name]);
            crate::toast::show_toast_on_window(&win, &msg, crate::toast::ToastType::Info);
        });
        window.add_action(&delete_action);

        // --- Wire "Add" button on SmartFoldersSidebar to the new-smart-folder action ---
        let smart_folders_widget = sidebar.smart_folders_sidebar();
        smart_folders_widget
            .add_button()
            .set_action_name(Some("win.new-smart-folder"));

        // --- Toggle Smart Folders visibility ---
        let toggle_action = gio::SimpleAction::new("toggle-smart-folders", None);
        let sidebar_clone = sidebar.clone();
        let state_clone = state.clone();
        let window_weak = window.downgrade();
        toggle_action.connect_activate(move |_, _| {
            let new_visible = !sidebar_clone.is_smart_folders_visible();
            sidebar_clone.set_smart_folders_visible(new_visible);
            // Persist the setting
            if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
                let mut settings = state_mut.settings().clone();
                settings.ui.show_smart_folders = new_visible;
                if let Err(e) = state_mut.update_settings(settings) {
                    tracing::warn!(
                        error = %e,
                        "failed to persist smart-folders visibility toggle"
                    );
                    if let Some(win) = window_weak.upgrade() {
                        // Settings persistence failure is not transient — blocking dialog (GNOME HIG).
                        let msg = i18n_f("Could not save preference: {}", &[&e]);
                        crate::alert::show_error(&win, &i18n("Could not save preference"), &msg);
                    }
                }
            }
        });
        window.add_action(&toggle_action);

        // --- Select item by ID (used by smart folder context menu) ---
        let select_by_id_action =
            gio::SimpleAction::new("select-item-by-id", Some(glib::VariantTy::STRING));
        let sidebar_clone = sidebar.clone();
        select_by_id_action.connect_activate(move |_, param| {
            if let Some(param) = param
                && let Some(id_str) = param.get::<String>()
                && let Ok(item_id) = uuid::Uuid::parse_str(&id_str)
            {
                sidebar_clone.select_item_by_id(item_id);
            }
        });
        window.add_action(&select_by_id_action);
    }
}
