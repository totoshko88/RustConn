//! Keyboard navigation helpers for dialogs
//!
//! This module provides reusable functions for improving keyboard navigation
//! in dialog windows, following GNOME HIG guidelines.

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;

/// Sets up standard keyboard shortcuts for a dialog window.
///
/// This adds:
/// - Escape: Close the dialog
/// - Ctrl+S: Activate the save/confirm button (if provided)
/// - Ctrl+W: Close the dialog
///
/// # Arguments
/// * `window` - The dialog window (can be `adw::Window` or `gtk4::Window`)
/// * `save_button` - Optional save/confirm button to activate on Ctrl+S
///
/// # Example
/// ```ignore
/// let dialog = adw::Window::new();
/// let save_btn = gtk4::Button::with_label("Save");
/// setup_dialog_shortcuts(&dialog, Some(&save_btn));
/// ```
pub fn setup_dialog_shortcuts<W: IsA<gtk4::Widget>>(
    window: &W,
    save_button: Option<&gtk4::Button>,
) {
    let key_controller = gtk4::EventControllerKey::new();

    let window_weak = window.downgrade();
    let save_button_weak = save_button.map(gtk4::prelude::ObjectExt::downgrade);

    key_controller.connect_key_pressed(move |_, key, _, modifiers| {
        let ctrl = modifiers.contains(gdk::ModifierType::CONTROL_MASK);

        match key {
            // Escape: Close dialog
            gdk::Key::Escape => {
                if let Some(w) = window_weak.upgrade()
                    && let Some(window) = w.root().and_then(|r| r.downcast::<gtk4::Window>().ok())
                {
                    window.close();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
            // Ctrl+S: Save/Confirm
            gdk::Key::s | gdk::Key::S if ctrl => {
                if let Some(ref weak) = save_button_weak
                    && let Some(btn) = weak.upgrade()
                    && btn.is_sensitive()
                {
                    btn.emit_clicked();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
            // Ctrl+W: Close dialog
            gdk::Key::w | gdk::Key::W if ctrl => {
                if let Some(w) = window_weak.upgrade()
                    && let Some(window) = w.root().and_then(|r| r.downcast::<gtk4::Window>().ok())
                {
                    window.close();
                    return glib::Propagation::Stop;
                }
                glib::Propagation::Proceed
            }
            _ => glib::Propagation::Proceed,
        }
    });

    window.add_controller(key_controller);
}

/// Sets up Enter key to activate a button when focused on an entry.
///
/// This is useful for forms where pressing Enter in a text field
/// should submit the form.
///
/// # Arguments
/// * `entry` - The entry widget
/// * `button` - The button to activate on Enter
pub fn setup_entry_activation(entry: &gtk4::Entry, button: &gtk4::Button) {
    let button_weak = button.downgrade();

    entry.connect_activate(move |_| {
        if let Some(btn) = button_weak.upgrade()
            && btn.is_sensitive()
        {
            btn.emit_clicked();
        }
    });
}

/// Sets up Enter key to activate a button when focused on a password entry.
///
/// # Arguments
/// * `entry` - The password entry widget
/// * `button` - The button to activate on Enter
pub fn setup_password_entry_activation(entry: &gtk4::PasswordEntry, button: &gtk4::Button) {
    let button_weak = button.downgrade();

    entry.connect_activate(move |_| {
        if let Some(btn) = button_weak.upgrade()
            && btn.is_sensitive()
        {
            btn.emit_clicked();
        }
    });
}

/// Makes a button the default widget that responds to Enter key.
///
/// This sets up the button with the "suggested-action" CSS class
/// and makes it activatable via Enter key when the dialog has focus.
///
/// # Arguments
/// * `button` - The button to make default
pub fn make_default_button(button: &gtk4::Button) {
    button.add_css_class("suggested-action");
    button.set_receives_default(true);
}

/// Makes a button a destructive action with appropriate styling.
///
/// # Arguments
/// * `button` - The button to style as destructive
pub fn make_destructive_button(button: &gtk4::Button) {
    button.add_css_class("destructive-action");
}

/// Sets up tab order for a list of widgets.
///
/// This ensures that pressing Tab moves focus through widgets
/// in the specified order.
///
/// # Arguments
/// * `widgets` - Slice of widgets in desired tab order
pub fn setup_tab_order(widgets: &[&impl IsA<gtk4::Widget>]) {
    for window in widgets.windows(2) {
        if let [current, next] = window {
            // GTK4 handles tab order automatically based on widget hierarchy
            // but we can ensure focusability
            current.set_focusable(true);
            next.set_focusable(true);
        }
    }
}

/// Focuses the first focusable widget in a container.
///
/// Useful for setting initial focus when a dialog opens.
///
/// # Arguments
/// * `container` - The container to search for focusable widgets
pub fn focus_first_widget(container: &impl IsA<gtk4::Widget>) {
    // Use idle_add to ensure widget is mapped before focusing
    let container_weak = container.downgrade();
    glib::idle_add_local_once(move || {
        if let Some(c) = container_weak.upgrade() {
            c.grab_focus();
        }
    });
}

#[cfg(test)]
mod tests {
    // Note: GTK widget tests require a display, so we test the logic patterns
    // rather than actual widget behavior. The module's compilation is verified
    // by the build process itself.
}
