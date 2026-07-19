//! Batch edit dialog for multi-selected connections
//!
//! Lets the user change group, tags, and icon for all connections selected
//! in group-operations mode in one pass. Each field has an "apply" check —
//! only checked fields are written, so unrelated settings stay untouched.

use adw::prelude::*;
use gtk4::glib;
use gtk4::prelude::*;
use libadwaita as adw;
use uuid::Uuid;

use super::MainWindow;
use crate::alert;
use crate::i18n::{i18n, i18n_f};
use crate::state::SharedAppState;
use crate::window::operations::SharedSidebar;

/// Shows the batch edit dialog for the current sidebar selection.
///
/// Only connection items are edited; selected groups are ignored. With
/// fewer than two connections selected an informational alert is shown
/// instead (single items are edited via the regular connection dialog).
pub fn show_batch_edit_dialog(
    window: &gtk4::Window,
    state: &SharedAppState,
    sidebar: &SharedSidebar,
    toast_overlay: &adw::ToastOverlay,
) {
    let selected_ids = sidebar.get_selected_ids();

    // Keep only connections (selection may include groups)
    let connection_ids: Vec<Uuid> = if let Ok(state_ref) = state.try_borrow() {
        selected_ids
            .iter()
            .filter(|id| state_ref.get_connection(**id).is_some())
            .copied()
            .collect()
    } else {
        return;
    };

    if connection_ids.len() < 2 {
        alert::show_alert(
            window,
            &i18n("Batch Edit"),
            &i18n("Select two or more connections to edit them together."),
        );
        return;
    }

    let dialog = adw::Window::builder()
        .title(i18n_f(
            "Edit {} Connections",
            &[&connection_ids.len().to_string()],
        ))
        .transient_for(window)
        .modal(true)
        .default_width(480)
        .build();

    let header = adw::HeaderBar::new();
    let cancel_btn = gtk4::Button::builder().label(i18n("Cancel")).build();
    let apply_btn = gtk4::Button::builder()
        .label(i18n("Apply"))
        .css_classes(["suggested-action"])
        .sensitive(false)
        .build();
    header.pack_start(&cancel_btn);
    header.pack_end(&apply_btn);

    let prefs_group = adw::PreferencesGroup::builder()
        .title(i18n("Fields to change"))
        .description(i18n("Only checked fields are applied to the selection"))
        .build();

    // --- Group row -------------------------------------------------------
    let group_check = gtk4::CheckButton::new();
    group_check.set_valign(gtk4::Align::Center);
    group_check.set_tooltip_text(Some(&i18n("Apply group change")));

    // Build hierarchical group list: index 0 = "(No Group)"
    let mut group_ids: Vec<Option<Uuid>> = vec![None];
    let mut group_names: Vec<String> = vec![i18n("(No Group)")];
    if let Ok(state_ref) = state.try_borrow() {
        let mut paths: Vec<(Uuid, String)> = state_ref
            .list_groups()
            .iter()
            .map(|g| {
                let path = state_ref
                    .get_group_path(g.id)
                    .unwrap_or_else(|| g.name.clone());
                (g.id, path)
            })
            .collect();
        paths.sort_by_key(|(_, p)| p.to_lowercase());
        for (id, path) in paths {
            group_ids.push(Some(id));
            group_names.push(path);
        }
    }
    let group_model = gtk4::StringList::new(
        &group_names
            .iter()
            .map(std::string::String::as_str)
            .collect::<Vec<_>>(),
    );
    let group_row = adw::ComboRow::builder()
        .title(i18n("Group"))
        .model(&group_model)
        .build();
    group_row.add_prefix(&group_check);
    prefs_group.add(&group_row);

    // --- Tags row ---------------------------------------------------------
    let tags_check = gtk4::CheckButton::new();
    tags_check.set_valign(gtk4::Align::Center);
    tags_check.set_tooltip_text(Some(&i18n("Apply tags change")));
    let tags_row = adw::EntryRow::builder()
        .title(i18n("Tags (comma-separated, replaces existing)"))
        .build();
    tags_row.add_prefix(&tags_check);
    prefs_group.add(&tags_row);

    // --- Icon row ---------------------------------------------------------
    let icon_check = gtk4::CheckButton::new();
    icon_check.set_valign(gtk4::Align::Center);
    icon_check.set_tooltip_text(Some(&i18n("Apply icon change")));
    let icon_row = adw::EntryRow::builder()
        .title(i18n("Icon (emoji or GTK icon name, empty clears)"))
        .build();
    icon_row.add_prefix(&icon_check);
    prefs_group.add(&icon_row);

    // Apply is enabled only when at least one field is checked
    let update_apply_sensitivity = {
        let apply_btn = apply_btn.clone();
        let group_check = group_check.clone();
        let tags_check = tags_check.clone();
        let icon_check = icon_check.clone();
        move || {
            apply_btn.set_sensitive(
                group_check.is_active() || tags_check.is_active() || icon_check.is_active(),
            );
        }
    };
    for check in [&group_check, &tags_check, &icon_check] {
        let update = update_apply_sensitivity.clone();
        check.connect_toggled(move |_| update());
    }

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.append(&prefs_group);

    let clamp = adw::Clamp::builder()
        .maximum_size(600)
        .child(&content)
        .build();

    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&clamp));
    dialog.set_content(Some(&toolbar_view));

    // Allow Enter to activate Apply (GNOME HIG: primary action accessible via keyboard)
    dialog.set_default_widget(Some(&apply_btn));

    let dialog_clone = dialog.clone();
    cancel_btn.connect_clicked(move |_| dialog_clone.close());

    let state_clone = state.clone();
    let sidebar_clone = sidebar.clone();
    let dialog_clone = dialog.clone();
    let parent_window = window.clone();
    let toast_overlay = toast_overlay.clone();
    apply_btn.connect_clicked(move |_| {
        // Validate icon before touching any connection
        let icon_text = icon_row.text().trim().to_string();
        if icon_check.is_active()
            && !icon_text.is_empty()
            && let Err(e) = rustconn_core::dialog_utils::validate_icon(&icon_text)
        {
            alert::show_validation_error(&dialog_clone, &i18n(&e.to_string()));
            return;
        }

        let target_group = group_ids
            .get(group_row.selected() as usize)
            .copied()
            .flatten();
        let tags: Vec<String> = tags_row
            .text()
            .split(',')
            .map(str::trim)
            .filter(|t| !t.is_empty())
            .map(std::string::ToString::to_string)
            .collect();

        // Snapshot for Undo, then apply checked fields in one pass
        let mut snapshot = Vec::new();
        let mut updated = 0usize;
        if let Ok(mut state_mut) = state_clone.try_borrow_mut() {
            for conn_id in &connection_ids {
                let Some(original) = state_mut.get_connection(*conn_id).cloned() else {
                    continue;
                };
                let mut modified = original.clone();
                if group_check.is_active() {
                    modified.group_id = target_group;
                }
                if tags_check.is_active() {
                    modified.tags.clone_from(&tags);
                }
                if icon_check.is_active() {
                    modified.icon = if icon_text.is_empty() {
                        None
                    } else {
                        Some(icon_text.clone())
                    };
                }
                if state_mut.update_connection(*conn_id, modified).is_ok() {
                    snapshot.push(original);
                    updated += 1;
                }
            }
        }

        let toast = adw::Toast::new(&i18n_f("Updated {} connections", &[&updated.to_string()]));
        toast.set_button_label(Some(&i18n("Undo")));
        // Longer timeout for bulk operations — give the user more time to notice
        toast.set_timeout(10);
        let undo_state = state_clone.clone();
        let undo_sidebar = sidebar_clone.clone();
        toast.connect_button_clicked(move |_| {
            if let Ok(mut state_mut) = undo_state.try_borrow_mut() {
                for original in &snapshot {
                    let _ = state_mut.update_connection(original.id, original.clone());
                }
            }
            MainWindow::reload_sidebar_preserving_state(&undo_state, &undo_sidebar);
        });
        toast_overlay.add_toast(toast);

        // Defer sidebar reload so the dialog closes without jank
        let state = state_clone.clone();
        let sidebar = sidebar_clone.clone();
        let dialog = dialog_clone.clone();
        glib::idle_add_local_once(move || {
            MainWindow::reload_sidebar_preserving_state(&state, &sidebar);
            dialog.close();
        });
        let _ = &parent_window;
    });

    dialog.present();
}
