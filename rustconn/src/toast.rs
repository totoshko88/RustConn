//! Toast notification system using libadwaita
//!
//! Wraps `adw::ToastOverlay` to provide a simple interface for showing notifications.
//! Supports standard toast types (info, success, warning, error) and actions.
//!
//! # Accessibility
//!
//! Toast notifications are automatically announced by screen readers via
//! libadwaita's built-in accessibility support.

use adw::prelude::*;
use gtk4 as gui;
use gui::glib;
use libadwaita as adw;

/// Toast message types for styling and semantic meaning
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastType {
    /// Informational message (default)
    Info,
    /// Success message
    Success,
    /// Warning message
    Warning,
    /// Error message
    Error,
}

impl ToastType {
    /// Returns the CSS class for this toast type
    #[must_use]
    pub const fn css_class(&self) -> &'static str {
        match self {
            Self::Info => "toast-info",
            Self::Success => "toast-success",
            Self::Warning => "toast-warning",
            Self::Error => "toast-error",
        }
    }

    /// Returns the icon name for this toast type
    #[must_use]
    pub const fn icon_name(&self) -> &'static str {
        match self {
            Self::Info => "dialog-information-symbolic",
            Self::Success => "object-select-symbolic",
            Self::Warning => "dialog-warning-symbolic",
            Self::Error => "dialog-error-symbolic",
        }
    }

    /// Returns the priority for this toast type
    /// Higher priority toasts are shown first
    #[must_use]
    pub const fn priority(&self) -> adw::ToastPriority {
        match self {
            Self::Info => adw::ToastPriority::Normal,
            Self::Success => adw::ToastPriority::Normal,
            Self::Warning => adw::ToastPriority::High,
            Self::Error => adw::ToastPriority::High,
        }
    }

    /// Returns an optional custom title for the toast
    ///
    /// Error and warning toasts get a short prefix so users can quickly
    /// distinguish severity at a glance.
    #[must_use]
    pub const fn custom_title(&self) -> Option<&'static str> {
        match self {
            Self::Info | Self::Success => None,
            Self::Warning => Some("Warning"),
            Self::Error => Some("Error"),
        }
    }
}

/// Toast overlay widget that wraps `adw::ToastOverlay`
pub struct ToastOverlay {
    /// The underlying libadwaita toast overlay
    overlay: adw::ToastOverlay,
}

impl ToastOverlay {
    /// Creates a new toast overlay
    #[must_use]
    pub fn new() -> Self {
        Self {
            overlay: adw::ToastOverlay::new(),
        }
    }

    /// Returns the overlay widget to add to the UI
    #[must_use]
    pub fn widget(&self) -> &adw::ToastOverlay {
        &self.overlay
    }

    /// Sets the main content of the overlay
    pub fn set_child(&self, child: Option<&impl IsA<gui::Widget>>) {
        self.overlay.set_child(child);
    }

    /// Shows a toast message with default options
    pub fn show_toast(&self, message: &str) {
        let toast = adw::Toast::new(message);
        self.overlay.add_toast(toast);
    }

    /// Shows a typed toast with appropriate priority and custom title
    ///
    /// Uses `adw::ToastPriority` to ensure important messages (warnings, errors)
    /// are shown before less important ones. Applies a custom title prefix
    /// for error/warning toasts to improve scannability.
    pub fn show_toast_with_type(&self, message: &str, toast_type: ToastType) {
        let toast = adw::Toast::new(message);
        toast.set_priority(toast_type.priority());
        if let Some(title) = toast_type.custom_title() {
            toast.set_custom_title(Some(&Self::build_toast_title_widget(title, toast_type)));
        }
        self.overlay.add_toast(toast);
    }

    /// Builds a small icon + label widget for custom toast titles
    fn build_toast_title_widget(label_text: &str, toast_type: ToastType) -> gui::Widget {
        let hbox = gui::Box::new(gui::Orientation::Horizontal, 6);
        hbox.set_halign(gui::Align::Center);
        let icon = gui::Image::from_icon_name(toast_type.icon_name());
        icon.set_pixel_size(16);
        let label = gui::Label::new(Some(label_text));
        hbox.append(&icon);
        hbox.append(&label);
        hbox.upcast()
    }

    /// Shows a success toast message
    pub fn show_success(&self, message: &str) {
        self.show_toast_with_type(message, ToastType::Success);
    }

    /// Shows a warning toast message (high priority)
    pub fn show_warning(&self, message: &str) {
        self.show_toast_with_type(message, ToastType::Warning);
    }

    /// Shows an error toast message (high priority)
    pub fn show_error(&self, message: &str) {
        self.show_toast_with_type(message, ToastType::Error);
    }

    /// Shows a toast with an action (e.g. "Undo")
    pub fn show_toast_with_action(
        &self,
        message: &str,
        action_label: &str,
        action_name: &str,
        action_target: Option<&glib::Variant>,
    ) {
        let toast = adw::Toast::new(message);
        toast.set_button_label(Some(action_label));
        toast.set_action_name(Some(action_name));
        if let Some(target) = action_target {
            toast.set_action_target_value(Some(target));
        }
        self.overlay.add_toast(toast);
    }
}

impl Default for ToastOverlay {
    fn default() -> Self {
        Self::new()
    }
}

/// Helper function to show a toast on a window
///
/// Tries to find an `adw::ToastOverlay` in the window structure. If no overlay
/// is found, falls back to an `adw::AlertDialog` so the message is never lost.
pub fn show_toast_on_window(window: &impl IsA<gui::Window>, message: &str, toast_type: ToastType) {
    if let Some(child) = window.child()
        && let Some(overlay) = find_toast_overlay(&child)
    {
        let toast = adw::Toast::new(message);
        toast.set_priority(toast_type.priority());
        overlay.add_toast(toast);
        return;
    }

    // Fallback: show an AlertDialog so the user still sees the message
    tracing::warn!(toast_message = %message, "ToastOverlay not found, falling back to AlertDialog");
    let heading = toast_type.custom_title().unwrap_or("Info");
    let dialog = adw::AlertDialog::new(Some(heading), Some(message));
    dialog.add_response("ok", "OK");
    dialog.set_default_response(Some("ok"));
    let widget = window.as_ref().upcast_ref::<gui::Widget>();
    dialog.present(Some(widget));
}

/// Helper to recursively find a `ToastOverlay` in the widget tree
///
/// Walks the tree using `first_child()` / `next_sibling()` which works
/// regardless of internal `adw::ApplicationWindow` wrapper widgets.
fn find_toast_overlay(widget: &gui::Widget) -> Option<adw::ToastOverlay> {
    if let Some(overlay) = widget.downcast_ref::<adw::ToastOverlay>() {
        return Some(overlay.clone());
    }

    // Walk children: GTK4 uses first_child / next_sibling linked list
    let mut child = widget.first_child();
    while let Some(c) = child {
        if let Some(found) = find_toast_overlay(&c) {
            return Some(found);
        }
        child = c.next_sibling();
    }

    None
}

/// Helper to show an Undo toast on a window
pub fn show_undo_toast_on_window(
    window: &impl IsA<gui::Window>,
    message: &str,
    action_target: &str,
) {
    if let Some(child) = window.child()
        && let Some(overlay) = find_toast_overlay(&child)
    {
        let toast = adw::Toast::new(message);
        toast.set_button_label(Some("Undo"));
        toast.set_action_name(Some("win.undo-delete"));
        toast.set_action_target_value(Some(&glib::Variant::from(action_target)));
        overlay.add_toast(toast);
    }
}
