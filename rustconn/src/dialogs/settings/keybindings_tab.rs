//! Keybindings settings tab
//!
//! Provides a preferences page for viewing and customizing keyboard shortcuts.
//! Each shortcut is displayed in a row grouped by category, with the ability
//! to record a new accelerator or reset to the default.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Button, EventControllerKey, Label};
use libadwaita as adw;
use rustconn_core::config::keybindings::{
    KeybindingCategory, KeybindingSettings, default_keybindings, is_valid_accelerator,
};
use std::cell::RefCell;
use std::rc::Rc;

use crate::i18n::{i18n, i18n_f};

/// Creates the keybindings preferences page.
///
/// Returns `(page, overrides_cell)` where `overrides_cell` holds the current
/// user overrides and is updated live as the user records new shortcuts.
#[allow(clippy::too_many_lines)]
pub fn create_keybindings_page() -> (adw::PreferencesPage, Rc<RefCell<KeybindingSettings>>) {
    let page = adw::PreferencesPage::builder()
        .title(&i18n("Keybindings"))
        .icon_name("preferences-desktop-keyboard-symbolic")
        .build();

    let overrides_cell: Rc<RefCell<KeybindingSettings>> =
        Rc::new(RefCell::new(KeybindingSettings::default()));

    let defaults = default_keybindings();

    // Build one PreferencesGroup per category
    for category in KeybindingCategory::all() {
        let cat_defs: Vec<_> = defaults
            .iter()
            .filter(|d| d.category == *category)
            .collect();
        if cat_defs.is_empty() {
            continue;
        }

        let group = adw::PreferencesGroup::builder()
            .title(&i18n(category.label()))
            .build();

        for def in &cat_defs {
            let row = adw::ActionRow::builder()
                .title(&i18n(&def.label))
                .subtitle(&def.action)
                .build();

            // Current accelerator label
            let accel_label = Label::builder()
                .label(&def.default_accels)
                .css_classes(["dim-label"])
                .valign(gtk4::Align::Center)
                .build();

            // Record button
            let record_btn = Button::builder()
                .label(&i18n("Record"))
                .valign(gtk4::Align::Center)
                .tooltip_text(&i18n("Press a key combination to set a new shortcut"))
                .build();

            // Reset button
            let reset_btn = Button::builder()
                .icon_name("edit-undo-symbolic")
                .valign(gtk4::Align::Center)
                .tooltip_text(&i18n("Reset to default"))
                .css_classes(["flat"])
                .build();

            row.add_suffix(&accel_label);
            row.add_suffix(&record_btn);
            row.add_suffix(&reset_btn);

            // --- Record button handler ---
            let action_name = def.action.clone();
            let default_accels = def.default_accels.clone();
            let accel_label_clone = accel_label.clone();
            let overrides_clone = overrides_cell.clone();
            let record_btn_clone = record_btn.clone();

            record_btn.connect_clicked(move |btn| {
                // Switch to "recording" mode
                btn.set_label(&i18n("Press keys..."));
                btn.set_sensitive(false);

                // Create a temporary key controller on the button's toplevel
                let key_ctrl = EventControllerKey::new();
                let action = action_name.clone();
                let defaults_str = default_accels.clone();
                let label = accel_label_clone.clone();
                let overrides = overrides_clone.clone();
                let record = record_btn_clone.clone();

                key_ctrl.connect_key_pressed(move |ctrl, keyval, _keycode, modifier| {
                    // Ignore lone modifier presses
                    if is_modifier_key(keyval) {
                        return gtk4::glib::Propagation::Proceed;
                    }

                    // Escape cancels recording
                    if keyval == gtk4::gdk::Key::Escape {
                        label.set_label(
                            &overrides
                                .borrow()
                                .overrides
                                .get(&action)
                                .cloned()
                                .unwrap_or_else(|| defaults_str.clone()),
                        );
                        record.set_label(&i18n("Record"));
                        record.set_sensitive(true);
                        if let Some(widget) = ctrl.widget() {
                            widget.remove_controller(ctrl);
                        }
                        return gtk4::glib::Propagation::Stop;
                    }

                    // Build accelerator string
                    let accel = gtk4::accelerator_name(keyval, modifier);
                    if is_valid_accelerator(&accel) {
                        // Check for conflicts with other actions
                        let conflict = find_accel_conflict(&accel, &action, &overrides.borrow());
                        if let Some(conflict_label) = &conflict {
                            // Show conflict warning but still allow the assignment
                            let warning = i18n_f("Conflicts with: {}", &[conflict_label]);
                            label.set_label(&format!("{accel}  \u{26A0}"));
                            label.set_tooltip_text(Some(&warning));
                            label.remove_css_class("dim-label");
                            label.add_css_class("warning");
                        } else {
                            label.set_label(&accel);
                            label.set_tooltip_text(None);
                            label.remove_css_class("warning");
                            label.add_css_class("dim-label");
                        }
                        overrides
                            .borrow_mut()
                            .overrides
                            .insert(action.clone(), accel.to_string());
                    }

                    record.set_label(&i18n("Record"));
                    record.set_sensitive(true);
                    if let Some(widget) = ctrl.widget() {
                        widget.remove_controller(ctrl);
                    }
                    gtk4::glib::Propagation::Stop
                });

                // Attach key controller to the toplevel window
                if let Some(toplevel) = btn.root() {
                    toplevel.add_controller(key_ctrl);
                }
            });

            // --- Reset button handler ---
            let action_name = def.action.clone();
            let default_accels = def.default_accels.clone();
            let overrides_clone = overrides_cell.clone();

            reset_btn.connect_clicked(move |_| {
                overrides_clone.borrow_mut().reset(&action_name);
                accel_label.set_label(&default_accels);
            });

            group.add(&row);
        }

        page.add(&group);
    }

    // Reset All button at the bottom
    let reset_all_group = adw::PreferencesGroup::new();
    let reset_all_btn = Button::builder()
        .label(&i18n("Reset All to Defaults"))
        .css_classes(["destructive-action"])
        .halign(gtk4::Align::Center)
        .build();

    let overrides_clone = overrides_cell.clone();
    let page_clone = page.clone();
    reset_all_btn.connect_clicked(move |_| {
        overrides_clone.borrow_mut().reset_all();
        // Refresh all labels by removing and re-adding the page content
        // Simpler: just update all dim-label Labels
        refresh_accel_labels(&page_clone);
    });

    reset_all_group.add(&reset_all_btn);
    page.add(&reset_all_group);

    (page, overrides_cell)
}

/// Loads keybinding settings into the page by updating accelerator labels.
pub fn load_keybinding_settings(
    page: &adw::PreferencesPage,
    overrides_cell: &Rc<RefCell<KeybindingSettings>>,
    settings: &KeybindingSettings,
) {
    *overrides_cell.borrow_mut() = settings.clone();

    // Walk the page and update labels to reflect overrides
    let defaults = default_keybindings();
    let mut def_iter = defaults.iter();

    // Iterate groups → rows → suffix labels
    let mut child = page.first_child();
    while let Some(widget) = child {
        if let Some(group_widget) = widget.first_child() {
            let mut group_child = group_widget.first_child();
            while let Some(row_widget) = group_child {
                if row_widget.is::<adw::ActionRow>()
                    && let Some(def) = def_iter.next()
                {
                    let accel = settings.get_accel(def);
                    // Find the dim-label Label in the row suffixes
                    update_row_accel_label(&row_widget, accel);
                }
                group_child = row_widget.next_sibling();
            }
        }
        child = widget.next_sibling();
    }
}

/// Collects the current keybinding overrides from the page state.
pub fn collect_keybinding_settings(
    overrides_cell: &Rc<RefCell<KeybindingSettings>>,
) -> KeybindingSettings {
    overrides_cell.borrow().clone()
}

/// Checks whether `accel` conflicts with another action's shortcut.
///
/// Returns the human-readable label of the conflicting action, or `None`.
fn find_accel_conflict(
    accel: &str,
    current_action: &str,
    overrides: &KeybindingSettings,
) -> Option<String> {
    let defaults = default_keybindings();
    for def in &defaults {
        if def.action == current_action {
            continue;
        }
        let effective = overrides.get_accel(def);
        // Check each pipe-separated accelerator
        for existing in effective.split('|') {
            if existing == accel {
                return Some(def.label.clone());
            }
        }
    }
    None
}

/// Returns `true` if the keyval is a modifier key (Shift, Control, Alt, Super).
fn is_modifier_key(keyval: gtk4::gdk::Key) -> bool {
    matches!(
        keyval,
        gtk4::gdk::Key::Shift_L
            | gtk4::gdk::Key::Shift_R
            | gtk4::gdk::Key::Control_L
            | gtk4::gdk::Key::Control_R
            | gtk4::gdk::Key::Alt_L
            | gtk4::gdk::Key::Alt_R
            | gtk4::gdk::Key::Super_L
            | gtk4::gdk::Key::Super_R
            | gtk4::gdk::Key::Meta_L
            | gtk4::gdk::Key::Meta_R
            | gtk4::gdk::Key::Hyper_L
            | gtk4::gdk::Key::Hyper_R
            | gtk4::gdk::Key::ISO_Level3_Shift
    )
}

/// Refreshes all accelerator labels in the page to show defaults.
fn refresh_accel_labels(page: &adw::PreferencesPage) {
    let defaults = default_keybindings();
    let mut def_iter = defaults.iter();

    let mut child = page.first_child();
    while let Some(widget) = child {
        if let Some(group_widget) = widget.first_child() {
            let mut group_child = group_widget.first_child();
            while let Some(row_widget) = group_child {
                if row_widget.is::<adw::ActionRow>()
                    && let Some(def) = def_iter.next()
                {
                    update_row_accel_label(&row_widget, &def.default_accels);
                }
                group_child = row_widget.next_sibling();
            }
        }
        child = widget.next_sibling();
    }
}

/// Finds and updates the accelerator label within an `ActionRow`.
fn update_row_accel_label(row_widget: &gtk4::Widget, accel: &str) {
    // The suffix box is the last child of the ActionRow's internal layout.
    // Walk children looking for a Label with the "dim-label" CSS class.
    let mut child = row_widget.first_child();
    while let Some(w) = child {
        if let Some(label) = w.downcast_ref::<Label>()
            && label.has_css_class("dim-label")
        {
            label.set_label(accel);
            return;
        }
        // Check nested children (suffix box)
        let mut inner = w.first_child();
        while let Some(inner_w) = inner {
            if let Some(label) = inner_w.downcast_ref::<Label>()
                && label.has_css_class("dim-label")
            {
                label.set_label(accel);
                return;
            }
            // One more level deep for the suffix box
            let mut deep = inner_w.first_child();
            while let Some(deep_w) = deep {
                if let Some(label) = deep_w.downcast_ref::<Label>()
                    && label.has_css_class("dim-label")
                {
                    label.set_label(accel);
                    return;
                }
                deep = deep_w.next_sibling();
            }
            inner = inner_w.next_sibling();
        }
        child = w.next_sibling();
    }
}
