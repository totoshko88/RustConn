//! Keybindings settings tab
//!
//! Provides a preferences page for viewing and customizing keyboard shortcuts.
//! Each shortcut is displayed in a row grouped by category, with the ability
//! to record a new accelerator or reset to the default.

use adw::prelude::*;
use gtk4::prelude::*;
use gtk4::{Button, EventControllerKey, Label, gio};
use libadwaita as adw;
use rustconn_core::config::keybindings::{
    KeybindingCategory, KeybindingSettings, accelerators_equivalent, default_keybindings,
    is_valid_accelerator,
};
use std::cell::RefCell;
use std::rc::Rc;

use crate::i18n::{i18n, i18n_f};

/// Creates the keybindings preferences page.
///
/// Returns `(page, overrides_cell)` where `overrides_cell` holds the current
/// user overrides and is updated live as the user records new shortcuts.
///
/// Each category is rendered as a collapsible `ExpanderRow` inside a single
/// `PreferencesGroup`, keeping the Interface page compact.
pub fn create_keybindings_page() -> (adw::PreferencesPage, Rc<RefCell<KeybindingSettings>>) {
    let page = adw::PreferencesPage::builder()
        .title(&i18n("Keybindings"))
        .icon_name("preferences-desktop-keyboard-symbolic")
        .build();

    let overrides_cell: Rc<RefCell<KeybindingSettings>> =
        Rc::new(RefCell::new(KeybindingSettings::default()));

    let defaults = default_keybindings();

    // Single group for all keybinding categories (collapsible expanders)
    let group = adw::PreferencesGroup::builder()
        .title(&i18n("Keyboard Shortcuts"))
        .build();

    // Build one ExpanderRow per category
    for category in KeybindingCategory::all() {
        let cat_defs: Vec<_> = defaults
            .iter()
            .filter(|d| d.category == *category)
            .collect();
        if cat_defs.is_empty() {
            continue;
        }

        let expander = adw::ExpanderRow::builder()
            .title(&i18n(category.label()))
            .show_enable_switch(false)
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
            reset_btn.update_property(&[gtk4::accessible::Property::Label(&i18n(
                "Reset keybinding to default",
            ))]);

            row.add_suffix(&accel_label);
            row.add_suffix(&record_btn);
            row.add_suffix(&reset_btn);

            // --- Record button handler ---
            let action_name = def.action.clone();
            let default_accels = def.default_accels.clone();
            let accel_label_clone = accel_label.clone();
            let overrides_clone = overrides_cell.clone();

            record_btn.connect_clicked(move |btn| {
                show_shortcut_recorder(
                    btn,
                    action_name.clone(),
                    default_accels.clone(),
                    accel_label_clone.clone(),
                    overrides_clone.clone(),
                );
            });

            // --- Reset button handler ---
            let action_name = def.action.clone();
            let default_accels = def.default_accels.clone();
            let overrides_clone = overrides_cell.clone();

            reset_btn.connect_clicked(move |_| {
                overrides_clone.borrow_mut().reset(&action_name);
                accel_label.set_label(&default_accels);
            });

            expander.add_row(&row);
        }

        group.add(&expander);
    }

    page.add(&group);

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

/// Opens a modal dialog that captures a single key combination.
///
/// The previous implementation attached an `EventControllerKey` to the toplevel
/// window and relied on `grab_focus()` on the parent row to establish a key
/// event target. That was fragile: inside `AdwPreferencesDialog` the search
/// `key_capture_widget` and the row's focusability differ across libadwaita
/// versions and Wayland/Flatpak, so the recorder often never received any keys.
///
/// A dedicated modal `AdwDialog` owns its own keyboard focus scope. The capture
/// target (`status`) is explicitly focusable and grabs focus on present, so the
/// `EventControllerKey` reliably receives every key press regardless of the
/// launching row or the parent dialog's search state.
///
/// Global application accelerators are still suspended during capture (they are
/// application-scoped and fire even while a modal dialog is open) and restored
/// when the recorder closes for any reason.
///
/// See: <https://github.com/totoshko88/RustConn/issues/167>
/// and <https://github.com/totoshko88/RustConn/issues/170>
fn show_shortcut_recorder(
    anchor: &Button,
    action: String,
    default_accels: String,
    accel_label: Label,
    overrides: Rc<RefCell<KeybindingSettings>>,
) {
    let app = gio::Application::default().and_then(|a| a.downcast::<gtk4::Application>().ok());

    // Suspend global accelerators so combinations like Ctrl+W or Ctrl+Shift+W
    // are captured here instead of triggering their currently bound actions.
    if let Some(ref app) = app {
        suspend_accels(app);
    }

    let dialog = adw::Dialog::builder()
        .title(&i18n("Set Shortcut"))
        .content_width(420)
        .build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&adw::HeaderBar::new());

    let status = adw::StatusPage::builder()
        .icon_name("preferences-desktop-keyboard-symbolic")
        .title(&i18n("Press the new shortcut"))
        .description(&i18n("Press Backspace to reset, or Escape to cancel"))
        .build();
    // The status page is the explicit focus target for the key controller, so
    // Capture-phase key events are always delivered to it.
    status.set_focusable(true);
    toolbar.set_content(Some(&status));
    dialog.set_child(Some(&toolbar));

    let key_ctrl = EventControllerKey::new();
    key_ctrl.set_propagation_phase(gtk4::PropagationPhase::Capture);

    // Clone for the close handler before the key handler takes ownership.
    let overrides_for_close = overrides.clone();

    {
        let dialog = dialog.clone();
        let action = action.clone();
        key_ctrl.connect_key_pressed(move |_ctrl, keyval, _keycode, state| {
            // Ignore lone modifier presses; wait for a real key.
            if is_modifier_key(keyval) {
                return gtk4::glib::Propagation::Proceed;
            }

            // Escape cancels without changing the binding.
            if keyval == gtk4::gdk::Key::Escape {
                dialog.close();
                return gtk4::glib::Propagation::Stop;
            }

            // Backspace resets the binding to its default.
            if keyval == gtk4::gdk::Key::BackSpace {
                overrides.borrow_mut().reset(&action);
                accel_label.set_label(&default_accels);
                accel_label.set_tooltip_text(None);
                accel_label.remove_css_class("warning");
                accel_label.add_css_class("dim-label");
                dialog.close();
                return gtk4::glib::Propagation::Stop;
            }

            // Strip lock modifiers (Caps/Num) so they do not pollute the accel.
            let mods = state & gtk4::accelerator_get_default_mod_mask();
            let accel = gtk4::accelerator_name(keyval, mods);
            if !is_valid_accelerator(&accel) {
                // E.g. a bare letter without modifiers: keep waiting.
                return gtk4::glib::Propagation::Stop;
            }

            if let Some(conflict_label) = find_accel_conflict(&accel, &action, &overrides.borrow())
            {
                // Show a conflict warning but still allow the assignment.
                let warning = i18n_f("Conflicts with: {}", &[&conflict_label]);
                accel_label.set_label(&format!("{accel}  \u{26A0}"));
                accel_label.set_tooltip_text(Some(&warning));
                accel_label.remove_css_class("dim-label");
                accel_label.add_css_class("warning");
            } else {
                accel_label.set_label(&accel);
                accel_label.set_tooltip_text(None);
                accel_label.remove_css_class("warning");
                accel_label.add_css_class("dim-label");
            }
            overrides
                .borrow_mut()
                .overrides
                .insert(action.clone(), accel.to_string());
            dialog.close();
            gtk4::glib::Propagation::Stop
        });
    }
    status.add_controller(key_ctrl);

    // Restore global accelerators whenever the recorder closes, regardless of
    // whether a shortcut was set, reset, or cancelled.
    {
        dialog.connect_closed(move |_| {
            if let Some(ref app) = app {
                restore_accels_with_overrides(app, &overrides_for_close.borrow());
            }
        });
    }

    dialog.present(Some(anchor));
    // Ensure the controller's widget holds focus so Capture-phase key events
    // are delivered to it immediately.
    status.grab_focus();
}

/// Loads keybinding settings into the page by updating accelerator labels.
pub fn load_keybinding_settings(
    page: &adw::PreferencesPage,
    overrides_cell: &Rc<RefCell<KeybindingSettings>>,
    settings: &KeybindingSettings,
) {
    *overrides_cell.borrow_mut() = settings.clone();

    // Match each row to its definition by the action name stored in the row's
    // subtitle, rather than by DOM position. This is robust to any divergence
    // between the UI layout order and the `default_keybindings()` vector order.
    let defaults = default_keybindings();
    let mut action_rows: Vec<gtk4::Widget> = Vec::new();
    collect_action_rows(&page.clone().upcast::<gtk4::Widget>(), &mut action_rows);

    for row_widget in &action_rows {
        let Some(action) = action_row_action(row_widget) else {
            continue;
        };
        if let Some(def) = defaults.iter().find(|d| d.action == action) {
            update_row_accel_label(row_widget, settings.get_accel(def));
        }
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
            if accelerators_equivalent(existing, accel) {
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
    let mut action_rows: Vec<gtk4::Widget> = Vec::new();
    collect_action_rows(&page.clone().upcast::<gtk4::Widget>(), &mut action_rows);

    for row_widget in &action_rows {
        let Some(action) = action_row_action(row_widget) else {
            continue;
        };
        if let Some(def) = defaults.iter().find(|d| d.action == action) {
            update_row_accel_label(row_widget, &def.default_accels);
        }
    }
}

/// Returns the action name stored as the subtitle of a keybinding `ActionRow`.
///
/// `create_keybindings_page` sets each row's subtitle to its `def.action`,
/// which we use as a stable identifier instead of relying on row position.
fn action_row_action(row_widget: &gtk4::Widget) -> Option<String> {
    row_widget
        .downcast_ref::<adw::ActionRow>()
        .and_then(|row| row.subtitle())
        .map(|subtitle| subtitle.to_string())
}

/// Recursively collects all `ActionRow` widgets from a widget tree.
///
/// Skips `ExpanderRow` itself (which is also an `ActionRow` subclass) and
/// only collects leaf `ActionRow` widgets that represent keybinding entries.
fn collect_action_rows(widget: &gtk4::Widget, rows: &mut Vec<gtk4::Widget>) {
    // ExpanderRow is a subclass of PreferencesRow, not ActionRow, so
    // checking `is::<adw::ActionRow>()` won't match it. But to be safe,
    // skip any ExpanderRow explicitly.
    if widget.is::<adw::ExpanderRow>() {
        // Still recurse into its children to find nested ActionRows
        let mut child = widget.first_child();
        while let Some(w) = child {
            collect_action_rows(&w, rows);
            child = w.next_sibling();
        }
        return;
    }

    if widget.is::<adw::ActionRow>() {
        rows.push(widget.clone());
        return;
    }

    let mut child = widget.first_child();
    while let Some(w) = child {
        collect_action_rows(&w, rows);
        child = w.next_sibling();
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

/// Temporarily removes all application accelerators.
///
/// This prevents global shortcuts (e.g. `Ctrl+W` for close-tab) from
/// intercepting key events while the user is recording a new shortcut.
/// Call [`restore_accels_with_overrides`] after recording completes or is cancelled.
///
/// See: <https://github.com/totoshko88/RustConn/issues/167>
fn suspend_accels(app: &gtk4::Application) {
    let defaults = default_keybindings();
    for def in &defaults {
        app.set_accels_for_action(&def.action, &[]);
    }
}

/// Restores all application accelerators respecting user overrides.
///
/// This re-applies the currently effective accelerators (user overrides
/// where present, defaults otherwise) after a recording session has ended.
/// Also called on dialog close to guarantee accelerators are never left empty.
pub fn restore_accels_with_overrides(app: &gtk4::Application, overrides: &KeybindingSettings) {
    let defaults = default_keybindings();
    for def in &defaults {
        let effective = overrides.get_accel(def);
        let accels: Vec<&str> = effective.split('|').collect();
        app.set_accels_for_action(&def.action, &accels);
    }
}
